use crate::cli::ConfigPaths;
use crate::error::Result;
use crate::infra::{GitHubApi, GitHubClient};
use std::sync::Arc;

/// Application context holding shared dependencies.
///
/// This struct serves as the dependency injection container for the application.
/// It's constructed once at startup and passed to command handlers.
pub struct AppContext<G: GitHubApi = GitHubClient> {
    /// Configuration file paths.
    pub paths: ConfigPaths,
    /// GitHub client for API interactions.
    pub github: Arc<G>,
}

impl AppContext<GitHubClient> {
    /// Create a new AppContext with GitHub dependency initialized.
    pub fn new(paths: ConfigPaths, github_token: Option<&str>) -> Result<Self> {
        let github = Arc::new(GitHubClient::new(github_token)?);

        Ok(Self { paths, github })
    }
}

impl<G: GitHubApi> AppContext<G> {
    /// Create an AppContext with only a custom GitHub dependency.
    pub fn with_github(paths: ConfigPaths, github: Arc<G>) -> Self {
        Self { paths, github }
    }
}
