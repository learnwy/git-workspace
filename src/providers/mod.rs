mod github;
mod gitlab;
mod selfgitlab;

use crate::repository::Repository;
pub use github::GithubProvider;
pub use gitlab::GitlabProvider;
pub use selfgitlab::SelfGitlabProvider;
use std::fmt;

pub trait Provider: fmt::Display {
    /// Returns true if the provider should work, otherwise prints an error and return false
    fn correctly_configured(&self) -> bool;
    fn fetch_repositories(&self) -> anyhow::Result<Vec<Repository>>;
}
