use crate::error::Result;
use crate::infra::HttpApi;
use crate::output::VpmOutput;
use std::sync::Arc;
use tracing::{info, instrument};

pub struct UrlValidator<H: HttpApi> {
    http: Arc<H>,
    max_concurrent: usize,
    max_retries: u32,
}

pub struct ValidationResult {
    pub total: usize,
    pub valid: usize,
    pub invalid: Vec<InvalidUrl>,
}

pub struct InvalidUrl {
    pub package_id: String,
    pub version: String,
    pub url: String,
}

impl<H: HttpApi> UrlValidator<H> {
    pub fn new(http: Arc<H>, max_concurrent: usize, max_retries: u32) -> Self {
        Self {
            http,
            max_concurrent,
            max_retries,
        }
    }

    #[instrument(skip(self, output), fields(package_count = output.packages.len()))]
    pub async fn validate(&self, output: &VpmOutput) -> Result<ValidationResult> {
        let urls = output.collect_urls();
        let total = urls.len();

        if urls.is_empty() {
            info!("No URLs to validate");
            return Ok(ValidationResult {
                total: 0,
                valid: 0,
                invalid: Vec::new(),
            });
        }

        info!(url_count = total, "Checking URL availability");

        let invalid_tuples = self
            .http
            .validate_urls(urls, self.max_concurrent, self.max_retries)
            .await;

        let invalid: Vec<InvalidUrl> = invalid_tuples
            .into_iter()
            .map(|(package_id, version, url)| InvalidUrl {
                package_id,
                version,
                url,
            })
            .collect();

        let valid = total - invalid.len();

        Ok(ValidationResult {
            total,
            valid,
            invalid,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::HttpClient;
    use crate::output::{Author, PackageOutput, VersionOutput};
    use indexmap::IndexMap;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn can_bind_localhost() -> bool {
        std::net::TcpListener::bind("127.0.0.1:0").is_ok()
    }

    fn create_version_output(url: &str) -> VersionOutput {
        VersionOutput {
            name: "com.example.package".to_string(),
            version: "1.0.0".to_string(),
            display_name: "Test Package".to_string(),
            description: "Test description".to_string(),
            unity: "2022.3".to_string(),
            unity_release: String::new(),
            dependencies: IndexMap::new(),
            keywords: vec![],
            author: Author {
                name: "Test".to_string(),
                email: String::new(),
                url: String::new(),
            },
            vpm_dependencies: IndexMap::new(),
            url: url.to_string(),
            license: String::new(),
        }
    }

    fn create_test_output(urls: Vec<(&str, &str, &str)>) -> VpmOutput {
        let mut packages = IndexMap::new();

        for (pkg_id, version, url) in urls {
            let pkg = packages.entry(pkg_id.to_string()).or_insert(PackageOutput {
                versions: IndexMap::new(),
            });

            let mut version_output = create_version_output(url);
            version_output.version = version.to_string();
            pkg.versions.insert(version.to_string(), version_output);
        }

        VpmOutput {
            name: "Test VPM".to_string(),
            id: "com.test.vpm".to_string(),
            url: "https://test.com/vpm.json".to_string(),
            author: "Test Author".to_string(),
            packages,
        }
    }

    mod validate {
        use super::*;

        #[tokio::test]
        async fn returns_all_valid_when_urls_accessible() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&mock_server)
                .await;

            let http = Arc::new(HttpClient::new().unwrap());
            let validator = UrlValidator::new(http, 4, 0);

            let url = format!("{}/package.zip", mock_server.uri());
            let output = create_test_output(vec![("com.test.pkg", "1.0.0", &url)]);

            let result = validator.validate(&output).await.unwrap();

            assert_eq!(result.total, 1);
            assert_eq!(result.valid, 1);
            assert!(result.invalid.is_empty());
        }

        #[tokio::test]
        async fn returns_invalid_for_404_urls() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .respond_with(ResponseTemplate::new(404))
                .mount(&mock_server)
                .await;

            let http = Arc::new(HttpClient::new().unwrap());
            let validator = UrlValidator::new(http, 4, 0);

            let url = format!("{}/missing.zip", mock_server.uri());
            let output = create_test_output(vec![("com.test.pkg", "1.0.0", &url)]);

            let result = validator.validate(&output).await.unwrap();

            assert_eq!(result.total, 1);
            assert_eq!(result.valid, 0);
            assert_eq!(result.invalid.len(), 1);
            assert_eq!(result.invalid[0].package_id, "com.test.pkg");
            assert_eq!(result.invalid[0].version, "1.0.0");
        }

        #[tokio::test]
        async fn handles_empty_output() {
            let http = Arc::new(HttpClient::new().unwrap());
            let validator = UrlValidator::new(http, 4, 0);

            let output = VpmOutput {
                name: "Test".to_string(),
                id: "com.test".to_string(),
                url: "https://test.com".to_string(),
                author: "Author".to_string(),
                packages: IndexMap::new(),
            };

            let result = validator.validate(&output).await.unwrap();

            assert_eq!(result.total, 0);
            assert_eq!(result.valid, 0);
            assert!(result.invalid.is_empty());
        }

        #[tokio::test]
        async fn handles_mixed_valid_and_invalid() {
            if !can_bind_localhost() {
                return;
            }
            let mock_server = MockServer::start().await;

            Mock::given(method("HEAD"))
                .and(path("/valid.zip"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&mock_server)
                .await;

            Mock::given(method("HEAD"))
                .and(path("/invalid.zip"))
                .respond_with(ResponseTemplate::new(404))
                .mount(&mock_server)
                .await;

            let http = Arc::new(HttpClient::new().unwrap());
            let validator = UrlValidator::new(http, 4, 0);

            let valid_url = format!("{}/valid.zip", mock_server.uri());
            let invalid_url = format!("{}/invalid.zip", mock_server.uri());
            let output = create_test_output(vec![
                ("com.test.pkg1", "1.0.0", &valid_url),
                ("com.test.pkg2", "1.0.0", &invalid_url),
            ]);

            let result = validator.validate(&output).await.unwrap();

            assert_eq!(result.total, 2);
            assert_eq!(result.valid, 1);
            assert_eq!(result.invalid.len(), 1);
            assert_eq!(result.invalid[0].package_id, "com.test.pkg2");
        }
    }
}
