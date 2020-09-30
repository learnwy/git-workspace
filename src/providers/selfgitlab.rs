use crate::providers::{resp_to_json, Provider};
use crate::repository::Repository;
use anyhow::{anyhow, Context};
use console::style;
use graphql_client::{GraphQLQuery, Response};
use serde::{Deserialize, Serialize};
use std::env;
use std::fmt;
use structopt::StructOpt;
// GraphQL queries we use to fetch user and group repositories.
// Right now, annoyingly, Gitlab has a bug around GraphQL pagination:
// https://gitlab.com/gitlab-org/gitlab/issues/33419
// So, we don't paginate at all in these queries. I'll fix this once
// the issue is closed.

struct ProjectNode {
    archived: bool,
    full_path: String,
    ssh_url: String,
    root_ref: Option<String>,
}

static DEFAULT_GITLAB_URL: &str = "https://gitlab.com";

fn public_gitlab_url() -> String {
    DEFAULT_GITLAB_URL.to_string()
}

fn default_env_var() -> String {
    String::from("SELF_GITHUB_TOKEN")
}

fn default_max() -> usize {
    20
}

fn default_use_ssh() -> bool {
    true
}

#[derive(Deserialize, Serialize, Debug, Eq, Ord, PartialEq, PartialOrd, StructOpt)]
#[serde(rename_all = "lowercase")]
#[structopt(about = "Add a Gitlab user or group by name")]
pub struct SelfGitlabProvider {
    /// The name of the gitlab group or namespace to add. Can include slashes.
    pub name: String,
    #[structopt(long = "url")]
    /// Gitlab instance URL
    pub url: String,
    #[structopt(long = "path", short = "p")]
    /// Clone repos to a specific path
    path: String,
    #[structopt(long = "env-name", short = "e", default_value = "SELF_GITLAB_TOKEN")]
    #[serde(default = "default_env_var")]
    /// Environment variable containing the auth token
    env_var: String,
    // Currently does not work.
    // https://gitlab.com/gitlab-org/gitlab/issues/121595
    // #[structopt(long = "skip-forks")]
    // #[structopt(about = "Don't clone forked repositories")]
    // #[serde(default = "default_forks")]
    // skip_forks: bool,
    #[structopt(long = "max", default_value = "20")]
    #[serde(default = "default_max")]
    max: usize,
    #[structopt(long = "use_ssh")]
    #[serde(default = "default_use_ssh")]
    use_ssh: bool,
}

impl fmt::Display for SelfGitlabProvider {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "SelfGitlab user/group {} at {} in directory {}, using the token stored in {}",
            style(&self.name.to_lowercase()).green(),
            style(&self.url).green(),
            style(&self.path).green(),
            style(&self.env_var).green(),
        )
    }
}

impl Provider for SelfGitlabProvider {
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
            println!("Create a personal access token here:");
            println!("{}/profile/personal_access_tokens", self.url);
            println!(
                "Set an environment variable called {} with the value",
                self.env_var
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
    fn fetch_repositories(&self) -> anyhow::Result<Vec<Repository>> {
        let gitlab_token = env::var(&self.env_var)
            .with_context(|| format!("Missing {} environment variable", self.env_var))?;
        let name = self.name.to_string().to_lowercase();
        let mut repositories = vec![];
        let mut temp_repositories: Vec<ProjectNode> = vec![];
        #[derive(Serialize, Deserialize, Clone)]
        struct SelfGitLab {
            path_with_namespace: Option<String>,
            ssh_url_to_repo: Option<String>,
            http_url_to_repo: Option<String>,
            archived: bool,
            default_branch: Option<String>,
        }
        let mut page = 1;
        loop {
            let res = ureq::get(
                format!(
                    "{}/api/v4/groups/{}/projects?page={}",
                    self.url,
                    name.clone(),
                    page
                )
                .as_str(),
            )
            .set("PRIVATE-TOKEN", format!("{}", gitlab_token).as_str())
            .send_form(&[]);
            let json = resp_to_json(res)?;
            let data: Vec<SelfGitLab> = serde_json::from_value(json).unwrap();
            if data.is_empty() {
                break;
            }
            // This is annoying but I'm still not sure how to unify it.
            for d in data {
                if d.ssh_url_to_repo.is_some() && d.path_with_namespace.is_some() {
                    if !d.archived {
                        temp_repositories.push(ProjectNode {
                            archived: d.archived,
                            ssh_url: if self.use_ssh {
                                d.ssh_url_to_repo.unwrap()
                            } else {
                                d.http_url_to_repo.unwrap()
                            },
                            root_ref: d.default_branch,
                            full_path: d.path_with_namespace.unwrap(),
                        })
                    }
                }
            }
            if temp_repositories.len() >= self.max {
                break;
            }
            page += 1;
        }

        temp_repositories.truncate(self.max);
        repositories.extend(
            temp_repositories
                .into_iter()
                .filter(|r| !r.archived)
                .map(|r| {
                    Repository::new(
                        format!("{}/{}", self.path, r.full_path),
                        r.ssh_url,
                        r.root_ref,
                        None,
                    )
                }),
        );
        Ok(repositories)
    }
}
