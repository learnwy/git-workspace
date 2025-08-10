use crate::config::Config;
use crate::lockfile::Lockfile;
use crate::repository::Repository;
use anyhow::Context;
use indicatif::ParallelProgressIterator;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::path::Path;
use log::{debug, info, warn};

/// Update our lockfile
pub fn lock(workspace: &Path, limit: usize, retry_delay: u64, retry_times: usize) -> anyhow::Result<()> {
    info!("Starting lock operation for workspace: {}", workspace.display());
    let config = Config::from_workspace(workspace)?;

    // Read the configuration sources
    let sources = config
        .read()
        .with_context(|| "Error reading config files")?;
    
    info!("Processing {} provider sources", sources.len());
    for (i, source) in sources.iter().enumerate() {
        info!("Provider {}: {}", i + 1, source);
    }

    let total_bar = ProgressBar::new(sources.len() as u64);
    total_bar.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {percent}% [{wide_bar:.cyan/blue}] {pos}/{len} (ETA: {eta_precise})").expect("Invalid template")
            .progress_chars("#>-"),
    );

    println!("Fetching repositories...");

    // For each source, in sequence, fetch the repositories
    let results = sources
        .par_iter()
        .map(|provider| {
            info!("Fetching repositories from provider: {}", provider);
            let mut repositories = vec![];
            
            // Try initial fetch
            match provider.fetch_repositories(limit) {
                Ok(repos) => {
                    info!("Successfully fetched {} repositories from {}", repos.len(), provider);
                    debug!("Repositories from {}: {:?}", provider, repos.iter().map(|r| r.name()).collect::<Vec<_>>());
                    // Log first few repositories for verification
                    for (i, repo) in repos.iter().take(3).enumerate() {
                        debug!("  Repository {}: {}", i + 1, repo.name());
                    }
                    repositories.extend(repos);
                    return repositories;
                }
                Err(e) => {
                    warn!("Initial fetch failed for {}: {}", provider, e);
                }
            }
            
            // Retry logic
            for attempt in 1..=retry_times {
                warn!("Retrying fetch for {} (attempt {} of {})", provider, attempt, retry_times);
                std::thread::sleep(std::time::Duration::from_secs(retry_delay));
                
                match provider.fetch_repositories(limit) {
                    Ok(repos) => {
                        info!("Successfully fetched {} repositories from {} on retry attempt {}", repos.len(), provider, attempt);
                        debug!("Repositories from {}: {:?}", provider, repos.iter().map(|r| r.name()).collect::<Vec<_>>());
                        // Log first few repositories for verification
                        for (i, repo) in repos.iter().take(3).enumerate() {
                            debug!("  Repository {}: {}", i + 1, repo.name());
                        }
                        repositories.extend(repos);
                        return repositories;
                    }
                    Err(e) => {
                        warn!("Retry attempt {} failed for {}: {}", attempt, provider, e);
                    }
                }
            }
            
            // All attempts failed, skip this provider
            warn!("Skipping provider {} after {} failed attempts", provider, retry_times + 1);
            repositories
        })
        .progress_with(total_bar)
        .collect::<Vec<_>>();
    let mut all_repositories: Vec<Repository> = vec![];
    for (i, result) in results.into_iter().enumerate() {
        debug!("Adding {} repositories from provider {} to final list", result.len(), i + 1);
        // Log first few repositories from each provider
        for (j, repo) in result.iter().take(3).enumerate() {
            debug!("  Provider {} Repository {}: {}", i + 1, j + 1, repo.name());
        }
        all_repositories.extend(result);
        debug!("Total repositories so far: {}", all_repositories.len());
    }
    
    info!("Total repositories collected: {}", all_repositories.len());
    
    // Log repository summary by organization
    let mut org_counts = std::collections::HashMap::new();
    for repo in &all_repositories {
        let parts: Vec<&str> = repo.name().split('/').collect();
        if parts.len() >= 2 {
            let org = parts[0];
            *org_counts.entry(org).or_insert(0) += 1;
        }
    }
    
    info!("Repositories by organization:");
    for (org, count) in org_counts.iter() {
        info!("  {}: {} repositories", org, count);
    }
    
    // We may have duplicated repositories here. Make sure they are unique based on the full path.
    all_repositories.sort();
    all_repositories.dedup();
    
    info!("After deduplication: {} repositories", all_repositories.len());
    
    // Write the lockfile out
    let lockfile_path = workspace.join("workspace-lock.toml");
    info!("Writing lockfile to: {}", lockfile_path.display());
    let lockfile = Lockfile::new(lockfile_path);
    lockfile.write(&all_repositories)?;
    
    info!("Lock operation completed successfully");
    Ok(())
}
