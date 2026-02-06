use super::http::build_http_client;
use super::retry::retry_backoff_delay;
use crate::domain::{Release, Repository};
use crate::error::{Error, Result};
use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use octocrab::Octocrab;
use reqwest::Client;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tracing::{debug, info, instrument, warn};

#[cfg(test)]
use mockall::automock;

/// Minimum remaining API calls before waiting for rate limit reset.
const RATE_LIMIT_BUFFER: u64 = 10;
const DOWNLOAD_TIMEOUT_SECS: u64 = 30;

fn should_retry_download_error(error: &Error) -> bool {
    match error {
        Error::Http { source, .. } => {
            if let Some(status) = source.status() {
                return status.is_server_error() || status.as_u16() == 429;
            }
            source.is_timeout() || source.is_connect() || source.is_request()
        }
        _ => false,
    }
}

/// Trait defining GitHub API operations for package fetching.
///
/// This trait abstracts the GitHub client operations, allowing for:
/// - Easier unit testing with mock implementations
/// - Potential support for other git hosting providers (GitLab, etc.)
#[cfg_attr(test, automock)]
#[async_trait]
pub trait GitHubApi: Send + Sync {
    /// Fetches all releases for a repository that contain the specified asset.
    async fn get_releases(&self, repo: &Repository, asset_name: &str) -> Result<Vec<Release>>;

    /// Downloads asset files for the given releases and returns raw content.
    ///
    /// Returns a vector of tuples containing:
    /// - The release information
    /// - Result containing raw content string
    async fn download_assets(
        &self,
        releases: Vec<Release>,
        max_concurrent: usize,
        max_retries: u32,
    ) -> Vec<(Release, Result<String>)>;

    /// Verifies that a repository exists and is accessible on GitHub.
    async fn verify_repository(&self, repo: &Repository) -> Result<()>;
}

pub struct GitHubClient {
    octocrab: Octocrab,
    http: Client,
    rate_limit_remaining: AtomicU64,
    rate_limit_reset: AtomicU64,
}

impl GitHubClient {
    pub fn new(token: Option<&str>) -> Result<Self> {
        let builder = Octocrab::builder();
        let octocrab = match token {
            Some(t) => builder.personal_token(t.to_string()).build(),
            None => builder.build(),
        }
        .map_err(|e| Error::GitHub {
            message: "Failed to initialize GitHub client".to_string(),
            source: e,
        })?;

        let http = build_http_client(
            DOWNLOAD_TIMEOUT_SECS,
            "github download client initialization",
        )?;

        Ok(Self {
            octocrab,
            http,
            // u64::MAX signals "not yet fetched" so the first API call triggers a rate limit check
            rate_limit_remaining: AtomicU64::new(u64::MAX),
            rate_limit_reset: AtomicU64::new(0),
        })
    }

    async fn wait_for_rate_limit(&self) {
        let remaining = self.rate_limit_remaining.load(Ordering::Relaxed);
        let reset = self.rate_limit_reset.load(Ordering::Relaxed);

        if remaining <= RATE_LIMIT_BUFFER && reset > 0 {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            if reset > now {
                let wait_secs = reset - now + 1;
                info!(
                    remaining,
                    reset_in_secs = wait_secs,
                    "Rate limit low, waiting for reset"
                );
                tokio::time::sleep(Duration::from_secs(wait_secs)).await;
            }
        }
    }

    fn update_rate_limit(&self, remaining: Option<u64>, reset: Option<u64>) {
        if let Some(r) = remaining {
            self.rate_limit_remaining.store(r, Ordering::Relaxed);
        }
        if let Some(r) = reset {
            self.rate_limit_reset.store(r, Ordering::Relaxed);
        }
    }

    #[instrument(skip(self), fields(%repo, %asset_name))]
    pub async fn get_releases(&self, repo: &Repository, asset_name: &str) -> Result<Vec<Release>> {
        let mut result = Vec::new();
        let mut page = 1u32;
        let repo_str = repo.to_string();

        loop {
            self.check_and_update_rate_limit().await?;
            self.wait_for_rate_limit().await;

            debug!(page, "Fetching releases page");

            let releases = self
                .octocrab
                .repos(&repo.owner, &repo.repo)
                .releases()
                .list()
                .per_page(100)
                .page(page)
                .send()
                .await
                .map_err(|e| Error::GitHub {
                    message: format!("Failed to fetch releases for '{}'", repo_str),
                    source: e,
                })?;

            if releases.items.is_empty() {
                break;
            }

            for release in &releases.items {
                let asset_url = release
                    .assets
                    .iter()
                    .find(|a| a.name == asset_name)
                    .map(|a| a.browser_download_url.to_string());

                result.push(Release::new(release.tag_name.clone(), asset_url));
            }

            if releases.items.len() < 100 {
                break;
            }
            page += 1;
        }

        debug!(count = result.len(), "Found releases");
        Ok(result)
    }

    async fn check_and_update_rate_limit(&self) -> Result<()> {
        let remaining = self.rate_limit_remaining.load(Ordering::Relaxed);

        if remaining <= RATE_LIMIT_BUFFER || remaining == u64::MAX {
            let rate_limit = self
                .octocrab
                .ratelimit()
                .get()
                .await
                .map_err(|e| Error::GitHub {
                    message: "Failed to check rate limit".to_string(),
                    source: e,
                })?;

            let core = &rate_limit.resources.core;
            self.update_rate_limit(Some(core.remaining as u64), Some(core.reset));

            debug!(
                remaining = core.remaining,
                limit = core.limit,
                reset = core.reset,
                "Updated rate limit info"
            );
        }

        Ok(())
    }

    async fn download_with_retry<T, F, Fut>(&self, url: &str, max_retries: u32, f: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let total_attempts = max_retries + 1;
        let mut last_error: Option<Error> = None;

        for attempt in 0..total_attempts {
            if attempt > 0 {
                let delay = retry_backoff_delay(attempt);
                warn!(attempt, max_retries, ?delay, "Retrying download");
                tokio::time::sleep(delay).await;
            }

            match f().await {
                Ok(output) => return Ok(output),
                Err(e) => {
                    debug!(url, attempt, error = %e, "Download attempt failed");
                    if !should_retry_download_error(&e) || attempt + 1 >= total_attempts {
                        return Err(e);
                    }
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            Error::ConfigValidation("Retry loop finished without attempts".to_string())
        }))
    }

    async fn fetch_raw(&self, url: &str) -> Result<String> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| Error::Http {
                url: url.to_string(),
                source: e,
            })?
            .error_for_status()
            .map_err(|e| Error::Http {
                url: url.to_string(),
                source: e,
            })?;

        let content = response.text().await.map_err(|e| Error::Http {
            url: url.to_string(),
            source: e,
        })?;

        Ok(content)
    }

    #[instrument(skip(self, releases), fields(release_count = releases.len(), max_concurrent, max_retries))]
    async fn download_assets_impl(
        &self,
        releases: Vec<Release>,
        max_concurrent: usize,
        max_retries: u32,
    ) -> Vec<(Release, Result<String>)> {
        stream::iter(releases.into_iter())
            .map(|release| async move {
                let result = match release.asset_url() {
                    Some(url) => self.download_asset(url, max_retries).await,
                    None => Err(Error::PackageJsonNotFound {
                        tag: release.tag().to_string(),
                    }),
                };
                (release, result)
            })
            .buffer_unordered(max_concurrent)
            .collect()
            .await
    }

    #[instrument(skip(self), fields(%url))]
    async fn download_asset(&self, url: &str, max_retries: u32) -> Result<String> {
        self.download_with_retry(url, max_retries, || self.fetch_raw(url))
            .await
    }

    #[instrument(skip(self), fields(%repo))]
    pub async fn verify_repository(&self, repo: &Repository) -> Result<()> {
        self.check_and_update_rate_limit().await?;
        self.wait_for_rate_limit().await;

        self.octocrab
            .repos(&repo.owner, &repo.repo)
            .get()
            .await
            .map_err(|e| match &e {
                octocrab::Error::GitHub { source, .. } if source.status_code.as_u16() == 404 => {
                    Error::RepositoryNotFound(repo.to_string())
                }
                _ => Error::GitHub {
                    message: format!("Failed to verify repository '{}'", repo),
                    source: e,
                },
            })?;

        debug!("Repository verified");
        Ok(())
    }
}

#[async_trait]
impl GitHubApi for GitHubClient {
    async fn get_releases(&self, repo: &Repository, asset_name: &str) -> Result<Vec<Release>> {
        GitHubClient::get_releases(self, repo, asset_name).await
    }

    async fn download_assets(
        &self,
        releases: Vec<Release>,
        max_concurrent: usize,
        max_retries: u32,
    ) -> Vec<(Release, Result<String>)> {
        self.download_assets_impl(releases, max_concurrent, max_retries)
            .await
    }

    async fn verify_repository(&self, repo: &Repository) -> Result<()> {
        GitHubClient::verify_repository(self, repo).await
    }
}
