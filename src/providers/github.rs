use crate::providers::{
    create_exclude_regex_set, create_include_regex_set, Provider, APP_USER_AGENT,
};
use crate::repository::Repository;
use anyhow::{bail, Context};
use console::style;
use graphql_client::{GraphQLQuery, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::env;
use std::fmt;

// See https://github.com/graphql-rust/graphql-client/blob/master/graphql_client/tests/custom_scalars.rs#L6
type GitSSHRemote = String;
#[allow(clippy::upper_case_acronyms)]
type URI = String;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/providers/graphql/github/schema.graphql",
    query_path = "src/providers/graphql/github/projects.graphql",
    response_derives = "Debug"
)]
pub struct Repositories;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/providers/graphql/github/schema.graphql",
    query_path = "src/providers/graphql/github/projects_by_stars.graphql",
    response_derives = "Debug"
)]
pub struct RepositoriesByStars;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/providers/graphql/github/schema.graphql",
    query_path = "src/providers/graphql/github/projects_by_updated.graphql",
    response_derives = "Debug"
)]
pub struct RepositoriesByUpdated;

fn default_env_var() -> String {
    String::from("GITHUB_TOKEN")
}

static DEFAULT_GITHUB_URL: &str = "https://api.github.com/graphql";

fn public_github_url() -> String {
    DEFAULT_GITHUB_URL.to_string()
}

#[derive(Deserialize, Serialize, Default, Debug, Eq, Ord, PartialEq, PartialOrd, clap::Parser)]
#[serde(rename_all = "lowercase")]
#[command(about = "Add a Github user or organization by name")]
pub struct GithubProvider {
    /// The name of the user or organisation to add.
    pub name: String,
    #[arg(long = "path", default_value = "github")]
    /// Clone repositories to a specific base path
    path: String,
    #[arg(long = "env-name", short = 'e', default_value = "GITHUB_TOKEN")]
    #[serde(default = "default_env_var")]
    /// Environment variable containing the auth token
    env_var: String,

    #[arg(long = "skip-forks")]
    #[serde(default)]
    /// Don't clone forked repositories
    skip_forks: bool,

    #[arg(long = "include")]
    #[serde(default)]
    /// Only clone repositories that match these regular expressions. The repository name
    /// includes the user or organisation name.
    include: Vec<String>,

    #[arg(long = "auth-http")]
    #[serde(default)]
    /// Use HTTP authentication instead of SSH
    auth_http: bool,

    #[arg(long = "exclude")]
    #[serde(default)]
    /// Don't clone repositories that match these regular expressions. The repository name
    /// includes the user or organisation name.
    exclude: Vec<String>,

    #[serde(default = "public_github_url")]
    #[arg(long = "url", default_value = DEFAULT_GITHUB_URL)]
    /// Github instance URL, if using Github Enterprise this should be
    /// http(s)://HOSTNAME/api/graphql
    pub url: String,
}

impl fmt::Display for GithubProvider {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Github user/org {} in directory {}, using the token stored in {}",
            style(&self.name.to_lowercase()).green(),
            style(&self.path.to_lowercase()).green(),
            style(&self.env_var).green(),
        )
    }
}

impl GithubProvider {
    fn parse_repo(
        &self,
        path: &str,
        repo: &repositories::RepositoriesRepositoryOwnerRepositoriesNodes,
    ) -> Repository {
        let default_branch = repo
            .default_branch_ref
            .as_ref()
            .map(|branch| branch.name.clone());
        let upstream = repo.parent.as_ref().map(|parent| parent.ssh_url.clone());

        Repository::new(
            format!("{}/{}", path, repo.name_with_owner.clone()),
            if self.auth_http {
                repo.url.clone()
            } else {
                repo.ssh_url.clone()
            },
            default_branch,
            upstream,
        )
    }

    fn fetch_repositories_by_stars(&self, limit: usize) -> anyhow::Result<Vec<repositories_by_stars::RepositoriesByStarsRepositoryOwnerRepositoriesNodes>> {
        log::info!("Fetching repositories sorted by stars for GitHub provider: {}", self.name);
        let github_token = env::var(&self.env_var)
            .with_context(|| format!("Environment variable {} is not set", &self.env_var))?;

        let auth_header = if github_token.starts_with("ghp_") || github_token.starts_with("github_pat_") {
            format!("token {}", github_token)
        } else {
            format!("Bearer {}", github_token)
        };

        let include_forks = if self.skip_forks {
            Some(false)
        } else {
            None
        };

        let agent = ureq::AgentBuilder::new()
            .user_agent(APP_USER_AGENT)
            .build();

        let mut all_repositories = Vec::new();
        let mut after = None;
        
        // Fetch repositories with pagination
        loop {
            let q = RepositoriesByStars::build_query(repositories_by_stars::Variables {
                login: self.name.clone(),
                include_forks,
                after,
                first: std::cmp::min(100, limit) as i64,
            });
            
            let res = agent
                .post(&self.url)
                .set("Authorization", &auth_header)
                .send_json(json!(&q));

            let res = match res {
                Ok(response) => response,
                Err(ureq::Error::Status(status, response)) => match response.into_string() {
                    Ok(resp) => {
                        bail!("Got status code {status}. Body: {resp}")
                    }
                    Err(e) => {
                        bail!("Got status code {status}. Error reading body: {e}")
                    }
                },
                Err(e) => return Err(e.into()),
            };

            let body = res.into_string()?;
            let response_data: Response<repositories_by_stars::ResponseData> = serde_json::from_str(&body)?;

            if let Some(errors) = response_data.errors {
                let total_errors = errors.len();
                let combined_errors: Vec<_> = errors.into_iter().map(|e| e.message).collect();
                let combined_message = combined_errors.join("\n");
                bail!(
                    "Received {} errors. Errors:\n{}",
                    total_errors,
                    combined_message
                );
            }

            let response_repositories = response_data
                .data
                .with_context(|| format!("Invalid response from GitHub: {}", body))?
                .repository_owner
                .with_context(|| format!("Invalid response from GitHub: {}", body))?
                .repositories;

            let nodes = response_repositories.nodes.unwrap();
            log::debug!("Received {} repositories from GitHub API for provider '{}' (sorted by stars)", 
                nodes.len(), self.name);
            
            for node in nodes {
                if let Some(repo) = node {
                    all_repositories.push(repo);
                }
            }

            if !response_repositories.page_info.has_next_page || all_repositories.len() >= limit {
                break;
            }
            after = response_repositories.page_info.end_cursor;
        }
        
        Ok(all_repositories)
    }

    fn fetch_repositories_by_updated(&self, limit: usize) -> anyhow::Result<Vec<repositories_by_updated::RepositoriesByUpdatedRepositoryOwnerRepositoriesNodes>> {
        log::info!("Fetching repositories sorted by updated for GitHub provider: {}", self.name);
        let github_token = env::var(&self.env_var)
            .with_context(|| format!("Environment variable {} is not set", &self.env_var))?;

        let auth_header = if github_token.starts_with("ghp_") || github_token.starts_with("github_pat_") {
            format!("token {}", github_token)
        } else {
            format!("Bearer {}", github_token)
        };

        let include_forks = if self.skip_forks {
            Some(false)
        } else {
            None
        };

        let agent = ureq::AgentBuilder::new()
            .user_agent(APP_USER_AGENT)
            .build();

        let mut all_repositories = Vec::new();
        let mut after = None;
        
        // Fetch repositories with pagination
        loop {
            let q = RepositoriesByUpdated::build_query(repositories_by_updated::Variables {
                login: self.name.clone(),
                include_forks,
                after,
                first: std::cmp::min(100, limit) as i64,
            });
            
            let res = agent
                .post(&self.url)
                .set("Authorization", &auth_header)
                .send_json(json!(&q));

            let res = match res {
                Ok(response) => response,
                Err(ureq::Error::Status(status, response)) => match response.into_string() {
                    Ok(resp) => {
                        bail!("Got status code {status}. Body: {resp}")
                    }
                    Err(e) => {
                        bail!("Got status code {status}. Error reading body: {e}")
                    }
                },
                Err(e) => return Err(e.into()),
            };

            let body = res.into_string()?;
            let response_data: Response<repositories_by_updated::ResponseData> = serde_json::from_str(&body)?;

            if let Some(errors) = response_data.errors {
                let total_errors = errors.len();
                let combined_errors: Vec<_> = errors.into_iter().map(|e| e.message).collect();
                let combined_message = combined_errors.join("\n");
                bail!(
                    "Received {} errors. Errors:\n{}",
                    total_errors,
                    combined_message
                );
            }

            let response_repositories = response_data
                .data
                .with_context(|| format!("Invalid response from GitHub: {}", body))?
                .repository_owner
                .with_context(|| format!("Invalid response from GitHub: {}", body))?
                .repositories;

            let nodes = response_repositories.nodes.unwrap();
            log::debug!("Received {} repositories from GitHub API for provider '{}' (sorted by updated)", 
                nodes.len(), self.name);
            
            for node in nodes {
                if let Some(repo) = node {
                    all_repositories.push(repo);
                }
            }

            if !response_repositories.page_info.has_next_page || all_repositories.len() >= limit {
                break;
            }
            after = response_repositories.page_info.end_cursor;
        }
        
        Ok(all_repositories)
    }
}

impl Provider for GithubProvider {
    fn correctly_configured(&self) -> bool {
        let token = env::var(&self.env_var);
        if token.is_err() {
            println!(
                "{}",
                style(format!(
                    "Error: {} environment variable is not defined",
                    self.env_var
                ))
                .red()
            );
            if self.url == public_github_url() {
                println!(
                    "Create a personal access token here: {}",
                    style("https://github.com/settings/tokens").green()
                );
            } else {
                println!(
                    "Create a personal access token in your {}.",
                    style("Github Enterprise server").green()
                );
            }

            println!(
                "Then set a {} environment variable with the value",
                style(&self.env_var).green()
            );
            return false;
        }
        if self.name.ends_with('/') {
            println!(
                "{}",
                style("Error: Ensure that names do not end in forward slashes").red()
            );
            println!("You specified: {}", self.name);
            return false;
        }
        true
    }

    fn fetch_repositories(&self, limit: usize) -> anyhow::Result<Vec<Repository>> {
        use std::collections::HashSet;
        
        log::info!("Fetching repositories for GitHub provider: {} with 1/4 stars + 3/4 updated distribution", self.name);
        
        let include_regex_set = create_include_regex_set(&self.include)?;
        let exclude_regex_set = create_exclude_regex_set(&self.exclude)?;
        
        // Calculate limits for each type
        let stars_limit = (limit as f64 * 0.25).ceil() as usize;
        let updated_limit = (limit as f64 * 0.75).ceil() as usize;
        
        // Fetch repositories by stars
        let starred_repos = self.fetch_repositories_by_stars(stars_limit)?;
        
        // Fetch repositories by updated
        let updated_repos = self.fetch_repositories_by_updated(updated_limit)?;
        
        let mut final_repositories = Vec::new();
        let mut seen_names = HashSet::new();
        
        // Helper function to check if repo should be included
        let should_include_repo = |repo_name: &str, is_archived: bool| -> bool {
            // Filter out archived repositories
            if is_archived {
                log::debug!("Filtering out archived repo: {}", repo_name);
                return false;
            }
            
            // Ensure the repository belongs to the specified organization/user
            let owner = repo_name.split('/').next().unwrap_or("");
            if !owner.eq_ignore_ascii_case(&self.name) {
                log::warn!("Repository {} does not belong to organization '{}', skipping", 
                    repo_name, self.name);
                return false;
            }
            
            // Check include patterns
            if !include_regex_set.is_match(repo_name) {
                log::debug!("Repository {} does not match include patterns, skipping", repo_name);
                return false;
            }
            
            // Check exclude patterns
            if exclude_regex_set.is_match(repo_name) {
                log::debug!("Repository {} matches exclude patterns, skipping", repo_name);
                return false;
            }
            
            true
        };
        
        // Add starred repositories (1/4 of limit)
        for repo in starred_repos.iter().take(stars_limit) {
            if should_include_repo(&repo.name_with_owner, repo.is_archived) {
                if seen_names.insert(repo.name_with_owner.clone()) {
                    final_repositories.push(Repository::new(
                        format!("{}/{}", self.path, repo.name_with_owner.clone()),
                        if self.auth_http {
                            repo.url.clone()
                        } else {
                            format!("git@github.com:{}.git", repo.name_with_owner)
                        },
                        repo.default_branch_ref.as_ref().map(|branch| branch.name.clone()),
                        None, // No upstream for starred repos from this query
                    ));
                }
            }
        }
        
        // Add updated repositories (3/4 of limit, excluding duplicates)
        for repo in updated_repos.iter().take(updated_limit) {
            if final_repositories.len() >= limit {
                break;
            }
            if should_include_repo(&repo.name_with_owner, repo.is_archived) {
                if seen_names.insert(repo.name_with_owner.clone()) {
                    final_repositories.push(Repository::new(
                        format!("{}/{}", self.path, repo.name_with_owner.clone()),
                        if self.auth_http {
                            repo.url.clone()
                        } else {
                            format!("git@github.com:{}.git", repo.name_with_owner)
                        },
                        repo.default_branch_ref.as_ref().map(|branch| branch.name.clone()),
                        None, // No upstream for updated repos from this query
                    ));
                }
            }
        }
        
        // If we still need more repositories, loop through starred repos again
        if final_repositories.len() < limit {
            for repo in starred_repos.iter().skip(stars_limit) {
                if final_repositories.len() >= limit {
                    break;
                }
                if should_include_repo(&repo.name_with_owner, repo.is_archived) {
                    if seen_names.insert(repo.name_with_owner.clone()) {
                        final_repositories.push(Repository::new(
                            format!("{}/{}", self.path, repo.name_with_owner.clone()),
                            if self.auth_http {
                                repo.url.clone()
                            } else {
                                format!("git@github.com:{}.git", repo.name_with_owner)
                            },
                            repo.default_branch_ref.as_ref().map(|branch| branch.name.clone()),
                            None,
                        ));
                    }
                }
            }
        }
        
        // Ensure we don't exceed the limit
        final_repositories.truncate(limit);
        log::info!("Added {} repositories from provider '{}' (1/4 stars: {}, 3/4 updated: {}, total: {})", 
            final_repositories.len(), self.name, 
            final_repositories.iter().take(stars_limit).count(),
            final_repositories.iter().skip(stars_limit).count(),
            final_repositories.len());
        
        Ok(final_repositories)
    }
    

}
