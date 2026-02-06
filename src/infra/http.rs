use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use indicatif::ProgressBar;
use reqwest::{Client, StatusCode};
use std::time::Duration;
use tracing::{debug, instrument};

#[cfg(test)]
use mockall::automock;

use super::retry::retry_backoff_delay;
use crate::error::{Error, Result};

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;

pub(crate) fn build_http_client(timeout_secs: u64, context: &str) -> Result<Client> {
    Client::builder()
        .user_agent("voyager")
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(Duration::from_secs(timeout_secs))
        .connect_timeout(Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECS))
        .build()
        .map_err(|e| Error::Http {
            url: context.to_string(),
            source: e,
        })
}

/// Trait for HTTP operations, enabling dependency injection and testing.
#[cfg_attr(test, automock)]
#[async_trait]
pub trait HttpApi: Send + Sync {
    /// Check if a URL exists using HEAD request with retry logic.
    async fn check_url_exists(&self, url: &str, max_retries: u32) -> bool;

    /// Validate multiple URLs concurrently, returning invalid ones.
    /// Note: This version does not support progress tracking.
    async fn validate_urls(
        &self,
        urls: Vec<(String, String, String)>,
        max_concurrent: usize,
        max_retries: u32,
    ) -> Vec<(String, String, String)>;
}

pub struct HttpClient {
    client: Client,
}

impl HttpClient {
    pub fn new() -> Result<Self> {
        let client = build_http_client(DEFAULT_TIMEOUT_SECS, "client initialization")?;

        Ok(Self { client })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    fn should_fallback_to_get(status: StatusCode) -> bool {
        matches!(
            status,
            StatusCode::FORBIDDEN | StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_IMPLEMENTED
        )
    }

    async fn check_url_exists_with_get(&self, url: &str) -> Option<bool> {
        match self
            .client
            .get(url)
            .header(reqwest::header::RANGE, "bytes=0-0")
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    Some(true)
                } else if status == StatusCode::TOO_MANY_REQUESTS {
                    debug!(url = %url, status = %status, "GET fallback hit rate limit; retrying");
                    None
                } else if status.is_client_error() {
                    Some(false)
                } else {
                    debug!(url = %url, status = %status, "GET fallback returned retryable status");
                    None
                }
            }
            Err(e) => {
                debug!(url = %url, error = %e, "GET fallback URL check failed with error");
                None
            }
        }
    }

    pub async fn check_url_exists(&self, url: &str, max_retries: u32) -> bool {
        // Use HEAD to avoid incrementing GitHub release download counts.
        // Retries handle transient failures. Some hosts block HEAD, so we
        // selectively fallback to a range-limited GET check.
        for attempt in 0..=max_retries {
            if attempt > 0 {
                let delay = retry_backoff_delay(attempt);
                debug!(url = %url, attempt, ?delay, "Retrying URL check");
                tokio::time::sleep(delay).await;
            }

            match self.client.head(url).send().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return true;
                    }
                    debug!(url = %url, status = %status, "URL check failed with status");
                    if Self::should_fallback_to_get(status) {
                        debug!(url = %url, status = %status, "Retrying URL check with GET fallback");
                        match self.check_url_exists_with_get(url).await {
                            Some(true) => return true,
                            Some(false) => return false,
                            None => continue,
                        }
                    }
                    if status == StatusCode::TOO_MANY_REQUESTS {
                        debug!(url = %url, status = %status, "URL check hit rate limit; retrying");
                        continue;
                    }
                    // Don't retry on 4xx errors (client errors like 404)
                    if status.is_client_error() {
                        return false;
                    }
                }
                Err(e) => {
                    debug!(url = %url, attempt, error = %e, "URL check failed with error");
                }
            }
        }
        false
    }

    #[instrument(skip(self, urls, progress), fields(url_count = urls.len(), max_concurrent, max_retries))]
    pub async fn validate_urls_with_progress(
        &self,
        urls: Vec<(String, String, String)>,
        max_concurrent: usize,
        max_retries: u32,
        progress: Option<&ProgressBar>,
    ) -> Vec<(String, String, String)> {
        let results: Vec<_> = stream::iter(urls)
            .map(|(package_id, version, url)| async move {
                let exists = self.check_url_exists(&url, max_retries).await;
                debug!(url = %url, exists, "URL check completed");
                (package_id, version, url, exists)
            })
            .buffer_unordered(max_concurrent)
            .inspect(|_| {
                if let Some(bar) = progress {
                    bar.inc(1);
                }
            })
            .collect()
            .await;

        results
            .into_iter()
            .filter_map(|(package_id, version, url, exists)| {
                if exists {
                    None
                } else {
                    Some((package_id, version, url))
                }
            })
            .collect()
    }
}

#[async_trait]
impl HttpApi for HttpClient {
    async fn check_url_exists(&self, url: &str, max_retries: u32) -> bool {
        HttpClient::check_url_exists(self, url, max_retries).await
    }

    async fn validate_urls(
        &self,
        urls: Vec<(String, String, String)>,
        max_concurrent: usize,
        max_retries: u32,
    ) -> Vec<(String, String, String)> {
        self.validate_urls_with_progress(urls, max_concurrent, max_retries, None)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn can_bind_localhost() -> bool {
        std::net::TcpListener::bind("127.0.0.1:0").is_ok()
    }

    mod check_url_exists {
        use super::*;

        #[tokio::test]
        async fn returns_true_for_200_response() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .and(path("/exists"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&mock_server)
                .await;

            let client = HttpClient::new().unwrap();
            let url = format!("{}/exists", mock_server.uri());

            assert!(client.check_url_exists(&url, 0).await);
        }

        #[tokio::test]
        async fn returns_true_for_204_response() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .and(path("/no-content"))
                .respond_with(ResponseTemplate::new(204))
                .mount(&mock_server)
                .await;

            let client = HttpClient::new().unwrap();
            let url = format!("{}/no-content", mock_server.uri());

            assert!(client.check_url_exists(&url, 0).await);
        }

        #[tokio::test]
        async fn returns_false_for_404_response() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .and(path("/missing"))
                .respond_with(ResponseTemplate::new(404))
                .mount(&mock_server)
                .await;

            let client = HttpClient::new().unwrap();
            let url = format!("{}/missing", mock_server.uri());

            assert!(!client.check_url_exists(&url, 0).await);
        }

        #[tokio::test]
        async fn returns_false_for_403_response() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .and(path("/forbidden"))
                .respond_with(ResponseTemplate::new(403))
                .mount(&mock_server)
                .await;

            let client = HttpClient::new().unwrap();
            let url = format!("{}/forbidden", mock_server.uri());

            assert!(!client.check_url_exists(&url, 0).await);
        }

        #[tokio::test]
        async fn falls_back_to_get_when_head_returns_405() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .and(path("/head-blocked"))
                .respond_with(ResponseTemplate::new(405))
                .expect(1)
                .mount(&mock_server)
                .await;

            Mock::given(method("GET"))
                .and(path("/head-blocked"))
                .respond_with(ResponseTemplate::new(200))
                .expect(1)
                .mount(&mock_server)
                .await;

            let client = HttpClient::new().unwrap();
            let url = format!("{}/head-blocked", mock_server.uri());

            assert!(client.check_url_exists(&url, 0).await);
        }

        #[tokio::test]
        async fn get_fallback_still_returns_false_for_missing_resource() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .and(path("/head-blocked-missing"))
                .respond_with(ResponseTemplate::new(405))
                .expect(1)
                .mount(&mock_server)
                .await;

            Mock::given(method("GET"))
                .and(path("/head-blocked-missing"))
                .respond_with(ResponseTemplate::new(404))
                .expect(1)
                .mount(&mock_server)
                .await;

            let client = HttpClient::new().unwrap();
            let url = format!("{}/head-blocked-missing", mock_server.uri());

            assert!(!client.check_url_exists(&url, 0).await);
        }

        #[tokio::test]
        async fn retries_when_get_fallback_returns_500() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .and(path("/head-blocked-retry"))
                .respond_with(ResponseTemplate::new(405))
                .expect(2)
                .mount(&mock_server)
                .await;

            Mock::given(method("GET"))
                .and(path("/head-blocked-retry"))
                .respond_with(ResponseTemplate::new(500))
                .expect(2)
                .mount(&mock_server)
                .await;

            let client = HttpClient::new().unwrap();
            let url = format!("{}/head-blocked-retry", mock_server.uri());

            assert!(!client.check_url_exists(&url, 1).await);
        }

        #[tokio::test]
        async fn succeeds_when_get_fallback_recovers_on_retry() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .and(path("/head-blocked-recovers"))
                .respond_with(ResponseTemplate::new(405))
                .expect(2)
                .mount(&mock_server)
                .await;

            Mock::given(method("GET"))
                .and(path("/head-blocked-recovers"))
                .respond_with(ResponseTemplate::new(500))
                .up_to_n_times(1)
                .mount(&mock_server)
                .await;

            Mock::given(method("GET"))
                .and(path("/head-blocked-recovers"))
                .respond_with(ResponseTemplate::new(200))
                .expect(1)
                .mount(&mock_server)
                .await;

            let client = HttpClient::new().unwrap();
            let url = format!("{}/head-blocked-recovers", mock_server.uri());

            assert!(client.check_url_exists(&url, 1).await);
        }

        #[tokio::test]
        async fn does_not_retry_on_404() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .and(path("/not-found"))
                .respond_with(ResponseTemplate::new(404))
                .expect(1)
                .mount(&mock_server)
                .await;

            let client = HttpClient::new().unwrap();
            let url = format!("{}/not-found", mock_server.uri());

            assert!(!client.check_url_exists(&url, 3).await);
        }

        #[tokio::test]
        async fn retries_on_500_error() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .and(path("/server-error"))
                .respond_with(ResponseTemplate::new(500))
                .expect(2)
                .mount(&mock_server)
                .await;

            let client = HttpClient::new().unwrap();
            let url = format!("{}/server-error", mock_server.uri());

            assert!(!client.check_url_exists(&url, 1).await);
        }

        #[tokio::test]
        async fn uses_head_method() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .respond_with(ResponseTemplate::new(200))
                .expect(1)
                .mount(&mock_server)
                .await;

            let client = HttpClient::new().unwrap();
            client.check_url_exists(&mock_server.uri(), 0).await;
        }
    }

    mod validate_urls {
        use super::*;

        #[tokio::test]
        async fn returns_only_invalid_urls() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .and(path("/valid"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&mock_server)
                .await;

            Mock::given(method("HEAD"))
                .and(path("/invalid"))
                .respond_with(ResponseTemplate::new(404))
                .mount(&mock_server)
                .await;

            let client = HttpClient::new().unwrap();
            let urls = vec![
                (
                    "pkg1".to_string(),
                    "1.0.0".to_string(),
                    format!("{}/valid", mock_server.uri()),
                ),
                (
                    "pkg2".to_string(),
                    "1.0.0".to_string(),
                    format!("{}/invalid", mock_server.uri()),
                ),
            ];

            let invalid = client.validate_urls_with_progress(urls, 4, 0, None).await;

            assert_eq!(invalid.len(), 1);
            assert_eq!(invalid[0].0, "pkg2");
        }

        #[tokio::test]
        async fn returns_empty_when_all_valid() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&mock_server)
                .await;

            let client = HttpClient::new().unwrap();
            let urls = vec![
                (
                    "pkg1".to_string(),
                    "1.0.0".to_string(),
                    format!("{}/a", mock_server.uri()),
                ),
                (
                    "pkg2".to_string(),
                    "1.0.0".to_string(),
                    format!("{}/b", mock_server.uri()),
                ),
            ];

            let invalid = client.validate_urls_with_progress(urls, 4, 0, None).await;

            assert!(invalid.is_empty());
        }

        #[tokio::test]
        async fn returns_all_when_all_invalid() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .respond_with(ResponseTemplate::new(404))
                .mount(&mock_server)
                .await;

            let client = HttpClient::new().unwrap();
            let urls = vec![
                (
                    "pkg1".to_string(),
                    "1.0.0".to_string(),
                    format!("{}/a", mock_server.uri()),
                ),
                (
                    "pkg2".to_string(),
                    "1.0.0".to_string(),
                    format!("{}/b", mock_server.uri()),
                ),
            ];

            let invalid = client.validate_urls_with_progress(urls, 4, 0, None).await;

            assert_eq!(invalid.len(), 2);
        }

        #[tokio::test]
        async fn handles_empty_urls() {
            let client = HttpClient::new().unwrap();
            let invalid = client.validate_urls_with_progress(vec![], 4, 0, None).await;
            assert!(invalid.is_empty());
        }

        #[tokio::test]
        async fn preserves_url_metadata() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .respond_with(ResponseTemplate::new(404))
                .mount(&mock_server)
                .await;

            let client = HttpClient::new().unwrap();
            let url = format!("{}/test", mock_server.uri());
            let urls = vec![(
                "com.example.pkg".to_string(),
                "2.0.0".to_string(),
                url.clone(),
            )];

            let invalid = client.validate_urls_with_progress(urls, 4, 0, None).await;

            assert_eq!(invalid.len(), 1);
            assert_eq!(invalid[0].0, "com.example.pkg");
            assert_eq!(invalid[0].1, "2.0.0");
            assert_eq!(invalid[0].2, url);
        }
    }

    mod http_client_new {
        use super::*;

        #[test]
        fn creates_client_successfully() {
            let result = HttpClient::new();
            assert!(result.is_ok());
        }

        #[test]
        fn client_accessor_returns_reference() {
            let http = HttpClient::new().unwrap();
            let _client = http.client();
        }
    }
}
