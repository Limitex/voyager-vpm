use crate::config::{Manifest, Package, validation};
use crate::domain::Release;
use crate::error::{Error, Result};
use crate::infra::GitHubApi;
use crate::lock::{LockedPackage, LockedVersion, Lockfile, PackageManifest};
use futures::stream::{self, StreamExt};
use semver::Version;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::{info, instrument, warn};

pub struct PackageFetcher<G: GitHubApi> {
    github: Arc<G>,
    config: FetcherConfig,
}

pub struct FetcherConfig {
    pub max_concurrent: usize,
    pub max_retries: u32,
    pub asset_name: String,
}

pub trait FetchProgressReporter: Send + Sync {
    fn on_fetching_releases(&self, package_id: &str);
    fn on_downloading(&self, package_id: &str, version_count: usize);
    fn on_done(&self, package_id: &str, existing: usize, new: usize);
}

struct PackageFetchResult {
    package_id: String,
    versions: Vec<LockedVersion>,
    existing_count: usize,
    new_count: usize,
    failed_count: usize,
}

impl<G: GitHubApi> PackageFetcher<G> {
    fn is_valid_sha256_hex(value: &str) -> bool {
        value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
    }

    pub fn new(github: Arc<G>, config: FetcherConfig) -> Self {
        Self { github, config }
    }

    fn parse_package_manifest(
        &self,
        content: &str,
        source: Option<&str>,
    ) -> Result<PackageManifest> {
        serde_json::from_str(content).map_err(|e| Error::JsonParse {
            source: source.unwrap_or("unknown").to_string(),
            error: e,
        })
    }

    fn validate_package_manifest(
        &self,
        package: &Package,
        release: &Release,
        manifest: &PackageManifest,
    ) -> Result<()> {
        if manifest.name != package.id {
            return Err(Error::ConfigValidation(format!(
                "package.json name '{}' does not match package id '{}' (release '{}')",
                manifest.name,
                package.id,
                release.tag()
            )));
        }

        let expected_version = release.version();
        if manifest.version != expected_version {
            return Err(Error::ConfigValidation(format!(
                "package.json version '{}' does not match release tag '{}' (expected '{}') for package '{}'",
                manifest.version,
                release.tag(),
                expected_version,
                package.id
            )));
        }

        if Version::parse(&manifest.version).is_err() {
            return Err(Error::ConfigValidation(format!(
                "package.json version '{}' is not valid SemVer for package '{}' (release '{}')",
                manifest.version,
                package.id,
                release.tag()
            )));
        }

        if manifest.display_name.trim().is_empty() {
            return Err(Error::ConfigValidation(format!(
                "package.json is missing required field 'displayName' for package '{}' (release '{}')",
                package.id,
                release.tag()
            )));
        }

        if manifest.author.name.trim().is_empty() {
            return Err(Error::ConfigValidation(format!(
                "package.json is missing required field 'author.name' for package '{}' (release '{}')",
                package.id,
                release.tag()
            )));
        }

        if manifest.author.email.trim().is_empty() {
            return Err(Error::ConfigValidation(format!(
                "package.json is missing required field 'author.email' for package '{}' (release '{}')",
                package.id,
                release.tag()
            )));
        }

        if manifest.unity.trim().is_empty() {
            if !manifest.unity_release.trim().is_empty() {
                return Err(Error::ConfigValidation(format!(
                    "package.json field 'unityRelease' requires field 'unity' for package '{}' (release '{}')",
                    package.id,
                    release.tag()
                )));
            }
            warn!(
                package_id = %package.id,
                release = %release.tag(),
                "package.json is missing recommended field 'unity'"
            );
        } else if let Err(e) = validation::validate_unity_version(&manifest.unity) {
            return Err(Error::ConfigValidation(format!(
                "package.json field 'unity' is invalid for package '{}' (release '{}'): {}",
                package.id,
                release.tag(),
                e
            )));
        }

        if !manifest.unity_release.trim().is_empty()
            && let Err(e) = validation::validate_unity_release(&manifest.unity_release)
        {
            return Err(Error::ConfigValidation(format!(
                "package.json field 'unityRelease' is invalid for package '{}' (release '{}'): {}",
                package.id,
                release.tag(),
                e
            )));
        }

        if manifest.url.trim().is_empty() {
            return Err(Error::ConfigValidation(format!(
                "package.json is missing required field 'url' for package '{}' (release '{}')",
                package.id,
                release.tag()
            )));
        }

        if let Err(e) = validation::validate_zip_url(&manifest.url) {
            return Err(Error::ConfigValidation(format!(
                "package.json field 'url' is invalid for package '{}' (release '{}'): {}",
                package.id,
                release.tag(),
                e
            )));
        }

        for (dependency_name, dependency_version) in &manifest.dependencies {
            if let Err(e) = validation::validate_reverse_domain(dependency_name) {
                return Err(Error::ConfigValidation(format!(
                    "package.json field 'dependencies' has invalid package name '{}' for package '{}' (release '{}'): {}",
                    dependency_name,
                    package.id,
                    release.tag(),
                    e
                )));
            }

            if let Err(e) = validation::validate_unity_dependency_version(dependency_version) {
                return Err(Error::ConfigValidation(format!(
                    "package.json field 'dependencies' has invalid version '{}' for dependency '{}' in package '{}' (release '{}'): {}",
                    dependency_version,
                    dependency_name,
                    package.id,
                    release.tag(),
                    e
                )));
            }
        }

        for (dependency_name, dependency_range) in &manifest.vpm_dependencies {
            if let Err(e) = validation::validate_reverse_domain(dependency_name) {
                return Err(Error::ConfigValidation(format!(
                    "package.json field 'vpmDependencies' has invalid package name '{}' for package '{}' (release '{}'): {}",
                    dependency_name,
                    package.id,
                    release.tag(),
                    e
                )));
            }

            if let Err(e) = validation::validate_vpm_dependency_range(dependency_range) {
                return Err(Error::ConfigValidation(format!(
                    "package.json field 'vpmDependencies' has invalid range '{}' for dependency '{}' in package '{}' (release '{}'): {}",
                    dependency_range,
                    dependency_name,
                    package.id,
                    release.tag(),
                    e
                )));
            }
        }

        if !manifest.zip_sha256.is_empty() && !Self::is_valid_sha256_hex(&manifest.zip_sha256) {
            return Err(Error::ConfigValidation(format!(
                "package.json field 'zipSHA256' must be a 64-character hex string for package '{}' (release '{}')",
                package.id,
                release.tag()
            )));
        }

        Ok(())
    }

    #[instrument(skip(self, manifest, lockfile, progress), fields(packages = manifest.packages.len()))]
    pub async fn fetch<P: FetchProgressReporter>(
        &self,
        manifest: &Manifest,
        lockfile: &mut Lockfile,
        progress: Option<&P>,
    ) -> Result<()> {
        self.reconcile_lockfile(manifest, lockfile);

        if manifest.packages.is_empty() {
            info!("No packages configured; skipping fetch");
            return Ok(());
        }

        let package_concurrency = self.config.max_concurrent.clamp(1, manifest.packages.len());
        let per_package_download_concurrency =
            (self.config.max_concurrent / package_concurrency).max(1);

        let existing_packages: HashMap<String, LockedPackage> = lockfile
            .packages
            .iter()
            .map(|pkg| (pkg.id.clone(), pkg.clone()))
            .collect();

        let mut outcomes: Vec<(usize, Result<PackageFetchResult>)> =
            stream::iter(manifest.packages.iter().enumerate())
                .map(|(index, package)| {
                    let existing_package =
                        existing_packages
                            .get(&package.id)
                            .cloned()
                            .unwrap_or(LockedPackage {
                                id: package.id.clone(),
                                repository: package.repository.clone(),
                                versions: Vec::new(),
                            });

                    async move {
                        (
                            index,
                            self.fetch_package(
                                package,
                                existing_package,
                                per_package_download_concurrency,
                                progress,
                            )
                            .await,
                        )
                    }
                })
                .buffer_unordered(package_concurrency)
                .collect()
                .await;

        outcomes.sort_by_key(|(index, _)| *index);

        let mut total_failed = 0usize;

        for (_, outcome) in outcomes {
            let outcome = outcome?;
            let locked_pkg = lockfile
                .get_package_mut(&outcome.package_id)
                .ok_or_else(|| {
                    Error::ConfigValidation(format!(
                        "Lockfile missing package '{}' after reconciliation",
                        outcome.package_id
                    ))
                })?;

            locked_pkg.versions = outcome.versions;
            if let Some(progress) = progress {
                progress.on_done(&locked_pkg.id, outcome.existing_count, outcome.new_count);
            }
            total_failed += outcome.failed_count;
            info!(
                package_id = %locked_pkg.id,
                total_versions = locked_pkg.versions.len(),
                new_versions = outcome.new_count,
                failed_versions = outcome.failed_count,
                "Package fetch completed"
            );
        }

        if total_failed > 0 {
            return Err(Error::FetchPartialFailure {
                count: total_failed,
            });
        }

        info!(
            package_concurrency,
            per_package_download_concurrency, "Fetch completed"
        );
        Ok(())
    }

    /// Syncs lockfile with manifest: removes stale packages, inserts new ones,
    /// clears versions when a repository changes, and reorders to match manifest.
    fn reconcile_lockfile(&self, manifest: &Manifest, lockfile: &mut Lockfile) {
        let manifest_order: HashMap<String, usize> = manifest
            .packages
            .iter()
            .enumerate()
            .map(|(index, pkg)| (pkg.id.clone(), index))
            .collect();

        lockfile
            .packages
            .retain(|pkg| manifest_order.contains_key(&pkg.id));

        for package in &manifest.packages {
            let locked_pkg = lockfile.get_or_insert_package(&package.id, &package.repository);
            if locked_pkg.repository != package.repository {
                locked_pkg.repository = package.repository.clone();
                locked_pkg.versions.clear();
            }
        }

        lockfile
            .packages
            .sort_by_key(|pkg| manifest_order.get(&pkg.id).copied().unwrap_or(usize::MAX));
    }

    #[instrument(skip(self, existing_package, progress), fields(package_id = %package.id, repo = %package.repository))]
    async fn fetch_package<P: FetchProgressReporter>(
        &self,
        package: &Package,
        existing_package: LockedPackage,
        download_concurrency: usize,
        progress: Option<&P>,
    ) -> Result<PackageFetchResult> {
        info!("Fetching package");
        if let Some(progress) = progress {
            progress.on_fetching_releases(&package.id);
        }

        let existing_versions = existing_package.existing_versions();
        let existing_count = existing_versions.len();

        let releases = self
            .github
            .get_releases(&package.repository, &self.config.asset_name)
            .await?;
        info!(releases = releases.len(), "Found releases");

        let new_releases: Vec<Release> = Release::filter_new(&releases, &existing_versions)
            .into_iter()
            .cloned()
            .collect();
        info!(new_versions = new_releases.len(), "New versions to fetch");

        let mut fetched_versions = Vec::new();
        let planned_count = new_releases.len();
        let mut failed_count = 0usize;

        if !new_releases.is_empty() {
            let version_list: Vec<_> = new_releases.iter().map(|r| r.version()).collect();
            info!(versions = ?version_list, "Downloading package.json files");
            if let Some(progress) = progress {
                progress.on_downloading(&package.id, planned_count);
            }

            let results = self
                .github
                .download_assets(new_releases, download_concurrency, self.config.max_retries)
                .await;

            for (release, result) in results {
                match result {
                    Ok(raw_content) => {
                        let asset_url = release.asset_url().unwrap_or_default().to_string();
                        match self.parse_package_manifest(&raw_content, release.asset_url()) {
                            Ok(version_output) => {
                                match self.validate_package_manifest(
                                    package,
                                    &release,
                                    &version_output,
                                ) {
                                    Ok(()) => {
                                        let locked_version = LockedVersion::new(
                                            release.tag().to_string(),
                                            asset_url,
                                            &raw_content,
                                            version_output,
                                        );
                                        fetched_versions.push(locked_version);
                                    }
                                    Err(e) => {
                                        failed_count += 1;
                                        warn!(
                                            version = %release.version(),
                                            error = %e,
                                            "Rejected package.json with invalid metadata"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                failed_count += 1;
                                warn!(
                                    version = %release.version(),
                                    error = %e,
                                    "Failed to parse package.json"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        failed_count += 1;
                        warn!(
                            version = %release.version(),
                            error = %e,
                            "Failed to fetch package.json"
                        );
                    }
                }
            }
        }

        // Maintain release order (newest first) for consistent output
        let release_order: Vec<String> = releases
            .iter()
            .filter(|r| r.asset_url().is_some())
            .map(|r| r.version().to_string())
            .collect();

        let all_versions: Vec<LockedVersion> = if release_order.is_empty() {
            if !existing_package.versions.is_empty() {
                warn!(
                    package_id = %package.id,
                    "No releases with matching assets found; keeping existing locked versions"
                );
            }
            existing_package.versions.clone()
        } else {
            let mut all_versions: Vec<LockedVersion> = Vec::new();
            for version_str in &release_order {
                if let Some(pos) = fetched_versions
                    .iter()
                    .position(|v| &v.version == version_str)
                {
                    all_versions.push(fetched_versions.remove(pos));
                } else if let Some(existing) = existing_package.get_version(version_str) {
                    all_versions.push(existing.clone());
                }
            }

            // Keep previously fetched versions that are no longer returned by
            // GitHub (e.g. temporarily hidden/deleted releases) to avoid
            // destructive lockfile churn.
            let mut seen_versions: HashSet<String> =
                all_versions.iter().map(|v| v.version.clone()).collect();
            for existing in &existing_package.versions {
                if seen_versions.insert(existing.version.clone()) {
                    all_versions.push(existing.clone());
                }
            }
            all_versions
        };
        let new_count = all_versions
            .iter()
            .filter(|v| !existing_versions.contains(&v.version))
            .count();

        Ok(PackageFetchResult {
            package_id: package.id.clone(),
            versions: all_versions,
            existing_count,
            new_count,
            failed_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Manifest, Package, Vpm};
    use crate::domain::Repository;
    use crate::error::Error;
    use crate::lock::{PackageAuthor, PackageManifest};
    use async_trait::async_trait;
    use indexmap::IndexMap;
    use std::collections::HashSet;
    use std::sync::Mutex;
    use std::time::Duration;

    struct FakeGitHub {
        releases: HashMap<String, Vec<Release>>,
        assets: HashMap<String, String>,
        delays_ms: HashMap<String, u64>,
    }

    #[async_trait]
    impl GitHubApi for FakeGitHub {
        async fn get_releases(&self, repo: &Repository, _asset_name: &str) -> Result<Vec<Release>> {
            if let Some(ms) = self.delays_ms.get(&repo.to_string()) {
                tokio::time::sleep(Duration::from_millis(*ms)).await;
            }
            Ok(self
                .releases
                .get(&repo.to_string())
                .cloned()
                .unwrap_or_default())
        }

        async fn download_assets(
            &self,
            releases: Vec<Release>,
            _max_concurrent: usize,
            _max_retries: u32,
        ) -> Vec<(Release, Result<String>)> {
            releases
                .into_iter()
                .map(|release| {
                    let result = match release.asset_url() {
                        Some(url) => self.assets.get(url).cloned().ok_or_else(|| {
                            Error::ConfigValidation(format!("missing test asset: {url}"))
                        }),
                        None => Err(Error::PackageJsonNotFound {
                            tag: release.tag().to_string(),
                        }),
                    };
                    (release, result)
                })
                .collect()
        }

        async fn verify_repository(&self, _repo: &Repository) -> Result<()> {
            Ok(())
        }
    }

    enum Event {
        Fetching(String),
        Downloading(String, usize),
        Done(String, usize, usize),
    }

    #[derive(Default)]
    struct TestProgress {
        events: Mutex<Vec<Event>>,
    }

    impl TestProgress {
        fn done_events(&self) -> Vec<(String, usize, usize)> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter_map(|event| match event {
                    Event::Done(pkg, existing, new) => Some((pkg.clone(), *existing, *new)),
                    _ => None,
                })
                .collect()
        }

        fn seen_fetching_packages(&self) -> HashSet<String> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter_map(|event| match event {
                    Event::Fetching(pkg) => Some(pkg.clone()),
                    _ => None,
                })
                .collect()
        }

        fn seen_downloading_packages(&self) -> HashSet<String> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter_map(|event| match event {
                    Event::Downloading(pkg, count) if *count > 0 => Some(pkg.clone()),
                    _ => None,
                })
                .collect()
        }
    }

    impl FetchProgressReporter for TestProgress {
        fn on_fetching_releases(&self, package_id: &str) {
            self.events
                .lock()
                .unwrap()
                .push(Event::Fetching(package_id.to_string()));
        }

        fn on_downloading(&self, package_id: &str, version_count: usize) {
            self.events
                .lock()
                .unwrap()
                .push(Event::Downloading(package_id.to_string(), version_count));
        }

        fn on_done(&self, package_id: &str, existing: usize, new: usize) {
            self.events
                .lock()
                .unwrap()
                .push(Event::Done(package_id.to_string(), existing, new));
        }
    }

    fn repo(s: &str) -> Repository {
        Repository::parse(s).unwrap()
    }

    fn version_json(name: &str, version: &str, url: &str) -> String {
        format!(
            r#"{{
  "name": "{name}",
  "version": "{version}",
  "displayName": "{name}",
  "description": "desc",
  "unity": "2022.3",
  "author": {{ "name": "Author", "email": "author@example.com" }},
  "url": "{url}"
}}"#
        )
    }

    fn version_output(name: &str, version: &str, url: &str) -> PackageManifest {
        PackageManifest {
            name: name.to_string(),
            version: version.to_string(),
            display_name: name.to_string(),
            description: "desc".to_string(),
            unity: "2022.3".to_string(),
            unity_release: String::new(),
            dependencies: IndexMap::new(),
            keywords: vec![],
            author: PackageAuthor {
                name: "Author".to_string(),
                email: "author@example.com".to_string(),
                url: String::new(),
            },
            vpm_dependencies: IndexMap::new(),
            legacy_folders: IndexMap::new(),
            legacy_files: IndexMap::new(),
            legacy_packages: vec![],
            documentation_url: String::new(),
            changelog_url: String::new(),
            licenses_url: String::new(),
            samples: vec![],
            hide_in_editor: None,
            package_type: String::new(),
            zip_sha256: String::new(),
            url: url.to_string(),
            license: String::new(),
            extra: IndexMap::new(),
        }
    }

    fn manifest_two_packages() -> Manifest {
        Manifest {
            vpm: Vpm {
                id: "com.test.vpm".to_string(),
                name: "Test".to_string(),
                author: "Author".to_string(),
                url: "https://example.com/index.json".to_string(),
            },
            packages: vec![
                Package {
                    id: "com.test.vpm.pkg1".to_string(),
                    repository: repo("owner1/repo1"),
                },
                Package {
                    id: "com.test.vpm.pkg2".to_string(),
                    repository: repo("owner2/repo2"),
                },
            ],
        }
    }

    fn initial_lockfile() -> Lockfile {
        let mut lockfile = Lockfile::new();
        lockfile.packages.push(LockedPackage {
            id: "com.test.vpm.pkg1".to_string(),
            repository: repo("owner1/repo1"),
            versions: vec![LockedVersion::new(
                "v1.0.0".to_string(),
                "https://assets.example/pkg1-v1.json".to_string(),
                &version_json(
                    "com.test.vpm.pkg1",
                    "1.0.0",
                    "https://download.example/pkg1-v1.zip",
                ),
                version_output(
                    "com.test.vpm.pkg1",
                    "1.0.0",
                    "https://download.example/pkg1-v1.zip",
                ),
            )],
        });
        lockfile.packages.push(LockedPackage {
            id: "com.test.vpm.pkg2".to_string(),
            repository: repo("owner2/repo2"),
            versions: vec![],
        });
        lockfile
    }

    #[tokio::test]
    async fn fetch_reports_progress_and_counts() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();
        let progress = TestProgress::default();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![
                        Release::new(
                            "v2.0.0".to_string(),
                            Some("https://assets.example/pkg1-v2.json".to_string()),
                        ),
                        Release::new(
                            "v1.0.0".to_string(),
                            Some("https://assets.example/pkg1-v1.json".to_string()),
                        ),
                    ],
                ),
                (
                    "owner2/repo2".to_string(),
                    vec![Release::new(
                        "v1.0.0".to_string(),
                        Some("https://assets.example/pkg2-v1.json".to_string()),
                    )],
                ),
            ]),
            assets: HashMap::from([
                (
                    "https://assets.example/pkg1-v2.json".to_string(),
                    version_json(
                        "com.test.vpm.pkg1",
                        "2.0.0",
                        "https://download.example/pkg1-v2.zip",
                    ),
                ),
                (
                    "https://assets.example/pkg1-v1.json".to_string(),
                    version_json(
                        "com.test.vpm.pkg1",
                        "1.0.0",
                        "https://download.example/pkg1-v1.zip",
                    ),
                ),
                (
                    "https://assets.example/pkg2-v1.json".to_string(),
                    version_json(
                        "com.test.vpm.pkg2",
                        "1.0.0",
                        "https://download.example/pkg2-v1.zip",
                    ),
                ),
            ]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        fetcher
            .fetch(&manifest, &mut lockfile, Some(&progress))
            .await
            .unwrap();

        let done = progress.done_events();
        assert_eq!(
            done,
            vec![
                ("com.test.vpm.pkg1".to_string(), 1, 1),
                ("com.test.vpm.pkg2".to_string(), 0, 1),
            ]
        );

        let seen_fetching = progress.seen_fetching_packages();
        assert!(seen_fetching.contains("com.test.vpm.pkg1"));
        assert!(seen_fetching.contains("com.test.vpm.pkg2"));

        let seen_downloading = progress.seen_downloading_packages();
        assert!(seen_downloading.contains("com.test.vpm.pkg1"));
        assert!(seen_downloading.contains("com.test.vpm.pkg2"));

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 2);
        assert_eq!(pkg1.versions[0].version, "2.0.0");
        assert_eq!(pkg1.versions[1].version, "1.0.0");
    }

    #[tokio::test]
    async fn fetch_keeps_done_event_order_in_manifest_order() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();
        let progress = TestProgress::default();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                (
                    "owner2/repo2".to_string(),
                    vec![Release::new(
                        "v1.0.0".to_string(),
                        Some("https://assets.example/pkg2-v1.json".to_string()),
                    )],
                ),
            ]),
            assets: HashMap::from([
                (
                    "https://assets.example/pkg1-v2.json".to_string(),
                    version_json(
                        "com.test.vpm.pkg1",
                        "2.0.0",
                        "https://download.example/pkg1-v2.zip",
                    ),
                ),
                (
                    "https://assets.example/pkg2-v1.json".to_string(),
                    version_json(
                        "com.test.vpm.pkg2",
                        "1.0.0",
                        "https://download.example/pkg2-v1.zip",
                    ),
                ),
            ]),
            delays_ms: HashMap::from([
                ("owner1/repo1".to_string(), 60),
                ("owner2/repo2".to_string(), 0),
            ]),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        fetcher
            .fetch(&manifest, &mut lockfile, Some(&progress))
            .await
            .unwrap();

        let done = progress.done_events();
        assert_eq!(done[0].0, "com.test.vpm.pkg1");
        assert_eq!(done[1].0, "com.test.vpm.pkg2");
    }

    #[tokio::test]
    async fn fetch_returns_error_when_any_release_download_fails() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                (
                    "owner2/repo2".to_string(),
                    vec![Release::new(
                        "v1.0.0".to_string(),
                        Some("https://assets.example/pkg2-v1.json".to_string()),
                    )],
                ),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                version_json(
                    "com.test.vpm.pkg1",
                    "2.0.0",
                    "https://download.example/pkg1-v2.zip",
                ),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        let result = fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await;
        assert!(matches!(
            result,
            Err(Error::FetchPartialFailure { count: 1 })
        ));
    }

    #[tokio::test]
    async fn fetch_keeps_existing_versions_when_no_matching_assets_found() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new("v2.0.0".to_string(), None)],
                ),
                (
                    "owner2/repo2".to_string(),
                    vec![Release::new("v1.0.0".to_string(), None)],
                ),
            ]),
            assets: HashMap::new(),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await
            .unwrap();

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 1);
        assert_eq!(pkg1.versions[0].version, "1.0.0");

        let pkg2 = lockfile.get_package("com.test.vpm.pkg2").unwrap();
        assert!(pkg2.versions.is_empty());
    }

    #[tokio::test]
    async fn fetch_preserves_existing_versions_missing_from_latest_release_list() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                version_json(
                    "com.test.vpm.pkg1",
                    "2.0.0",
                    "https://download.example/pkg1-v2.zip",
                ),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await
            .unwrap();

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 2);
        assert_eq!(pkg1.versions[0].version, "2.0.0");
        assert_eq!(pkg1.versions[1].version, "1.0.0");
    }

    #[tokio::test]
    async fn fetch_rejects_manifest_with_mismatched_package_name() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                version_json(
                    "com.test.vpm.wrong",
                    "2.0.0",
                    "https://download.example/pkg1-v2.zip",
                ),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        let result = fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await;
        assert!(matches!(
            result,
            Err(Error::FetchPartialFailure { count: 1 })
        ));

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 1);
        assert_eq!(pkg1.versions[0].version, "1.0.0");
    }

    #[tokio::test]
    async fn fetch_rejects_manifest_with_mismatched_version() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                version_json(
                    "com.test.vpm.pkg1",
                    "9.9.9",
                    "https://download.example/pkg1-v2.zip",
                ),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        let result = fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await;
        assert!(matches!(
            result,
            Err(Error::FetchPartialFailure { count: 1 })
        ));

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 1);
        assert_eq!(pkg1.versions[0].version, "1.0.0");
    }

    #[tokio::test]
    async fn fetch_rejects_manifest_missing_author_email() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                r#"{
  "name": "com.test.vpm.pkg1",
  "version": "2.0.0",
  "displayName": "com.test.vpm.pkg1",
  "description": "desc",
  "unity": "2022.3",
  "author": {"name": "Author"},
  "url": "https://download.example/pkg1-v2.zip"
}"#
                .to_string(),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        let result = fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await;
        assert!(matches!(
            result,
            Err(Error::FetchPartialFailure { count: 1 })
        ));

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 1);
        assert_eq!(pkg1.versions[0].version, "1.0.0");
    }

    #[tokio::test]
    async fn fetch_rejects_manifest_missing_author_field() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                r#"{
  "name": "com.test.vpm.pkg1",
  "version": "2.0.0",
  "displayName": "com.test.vpm.pkg1",
  "description": "desc",
  "unity": "2022.3",
  "url": "https://download.example/pkg1-v2.zip"
}"#
                .to_string(),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        let result = fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await;
        assert!(matches!(
            result,
            Err(Error::FetchPartialFailure { count: 1 })
        ));

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 1);
        assert_eq!(pkg1.versions[0].version, "1.0.0");
    }

    #[tokio::test]
    async fn fetch_accepts_manifest_author_string_with_email() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                r#"{
  "name": "com.test.vpm.pkg1",
  "version": "2.0.0",
  "displayName": "com.test.vpm.pkg1",
  "description": "desc",
  "unity": "2022.3",
  "author": "Author <author@example.com> (https://example.com)",
  "url": "https://download.example/pkg1-v2.zip"
}"#
                .to_string(),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await
            .unwrap();

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 2);
        assert_eq!(pkg1.versions[0].version, "2.0.0");
        assert_eq!(pkg1.versions[0].manifest.author.name, "Author");
        assert_eq!(pkg1.versions[0].manifest.author.email, "author@example.com");
        assert_eq!(pkg1.versions[0].manifest.author.url, "https://example.com");
    }

    #[tokio::test]
    async fn fetch_accepts_manifest_missing_unity() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                r#"{
  "name": "com.test.vpm.pkg1",
  "version": "2.0.0",
  "displayName": "com.test.vpm.pkg1",
  "description": "desc",
  "author": {"name": "Author", "email": "author@example.com"},
  "url": "https://download.example/pkg1-v2.zip"
}"#
                .to_string(),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await
            .unwrap();

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 2);
        assert_eq!(pkg1.versions[0].version, "2.0.0");
        assert_eq!(pkg1.versions[0].manifest.unity, "");
    }

    #[tokio::test]
    async fn fetch_rejects_manifest_with_invalid_unity_version() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                r#"{
  "name": "com.test.vpm.pkg1",
  "version": "2.0.0",
  "displayName": "com.test.vpm.pkg1",
  "description": "desc",
  "unity": "invalid",
  "author": {"name": "Author", "email": "author@example.com"},
  "url": "https://download.example/pkg1-v2.zip"
}"#
                .to_string(),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        let result = fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await;
        assert!(matches!(
            result,
            Err(Error::FetchPartialFailure { count: 1 })
        ));

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 1);
    }

    #[tokio::test]
    async fn fetch_accepts_manifest_with_valid_unity_release() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                r#"{
  "name": "com.test.vpm.pkg1",
  "version": "2.0.0",
  "displayName": "com.test.vpm.pkg1",
  "description": "desc",
  "unity": "2022.3",
  "unityRelease": "22f1",
  "author": {"name": "Author", "email": "author@example.com"},
  "url": "https://download.example/pkg1-v2.zip"
}"#
                .to_string(),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        let result = fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await;
        assert!(result.is_ok());

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 2);
        assert_eq!(pkg1.versions[0].version, "2.0.0");
        assert_eq!(pkg1.versions[0].manifest.unity_release, "22f1");
    }

    #[tokio::test]
    async fn fetch_rejects_manifest_with_unity_release_without_unity() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                r#"{
  "name": "com.test.vpm.pkg1",
  "version": "2.0.0",
  "displayName": "com.test.vpm.pkg1",
  "description": "desc",
  "unityRelease": "22f1",
  "author": {"name": "Author", "email": "author@example.com"},
  "url": "https://download.example/pkg1-v2.zip"
}"#
                .to_string(),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        let result = fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await;
        assert!(matches!(
            result,
            Err(Error::FetchPartialFailure { count: 1 })
        ));

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 1);
        assert_eq!(pkg1.versions[0].version, "1.0.0");
    }

    #[tokio::test]
    async fn fetch_rejects_manifest_with_invalid_unity_release() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                r#"{
  "name": "com.test.vpm.pkg1",
  "version": "2.0.0",
  "displayName": "com.test.vpm.pkg1",
  "description": "desc",
  "unity": "2022.3",
  "unityRelease": "0beta4",
  "author": {"name": "Author", "email": "author@example.com"},
  "url": "https://download.example/pkg1-v2.zip"
}"#
                .to_string(),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        let result = fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await;
        assert!(matches!(
            result,
            Err(Error::FetchPartialFailure { count: 1 })
        ));

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 1);
        assert_eq!(pkg1.versions[0].version, "1.0.0");
    }

    #[tokio::test]
    async fn fetch_rejects_manifest_with_invalid_semver_version() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                r#"{
  "name": "com.test.vpm.pkg1",
  "version": "2",
  "displayName": "com.test.vpm.pkg1",
  "description": "desc",
  "unity": "2022.3",
  "author": {"name": "Author", "email": "author@example.com"},
  "url": "https://download.example/pkg1-v2.zip"
}"#
                .to_string(),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        let result = fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await;
        assert!(matches!(
            result,
            Err(Error::FetchPartialFailure { count: 1 })
        ));

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 1);
        assert_eq!(pkg1.versions[0].version, "1.0.0");
    }

    #[tokio::test]
    async fn fetch_rejects_manifest_with_invalid_package_url() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                r#"{
  "name": "com.test.vpm.pkg1",
  "version": "2.0.0",
  "displayName": "com.test.vpm.pkg1",
  "description": "desc",
  "unity": "2022.3",
  "author": {"name": "Author", "email": "author@example.com"},
  "url": "not-a-url"
}"#
                .to_string(),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        let result = fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await;
        assert!(matches!(
            result,
            Err(Error::FetchPartialFailure { count: 1 })
        ));

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 1);
        assert_eq!(pkg1.versions[0].version, "1.0.0");
    }

    #[tokio::test]
    async fn fetch_rejects_manifest_with_non_zip_package_url() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                r#"{
  "name": "com.test.vpm.pkg1",
  "version": "2.0.0",
  "displayName": "com.test.vpm.pkg1",
  "description": "desc",
  "unity": "2022.3",
  "author": {"name": "Author", "email": "author@example.com"},
  "url": "https://download.example/pkg1-v2.json"
}"#
                .to_string(),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        let result = fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await;
        assert!(matches!(
            result,
            Err(Error::FetchPartialFailure { count: 1 })
        ));

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 1);
        assert_eq!(pkg1.versions[0].version, "1.0.0");
    }

    #[tokio::test]
    async fn fetch_rejects_manifest_with_unity_dependency_range() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                r#"{
  "name": "com.test.vpm.pkg1",
  "version": "2.0.0",
  "displayName": "com.test.vpm.pkg1",
  "description": "desc",
  "unity": "2022.3",
  "dependencies": {
    "com.unity.modules.physics": "^1.0.0"
  },
  "author": {"name": "Author", "email": "author@example.com"},
  "url": "https://download.example/pkg1-v2.zip"
}"#
                .to_string(),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        let result = fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await;
        assert!(matches!(
            result,
            Err(Error::FetchPartialFailure { count: 1 })
        ));

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 1);
        assert_eq!(pkg1.versions[0].version, "1.0.0");
    }

    #[tokio::test]
    async fn fetch_accepts_manifest_with_vpm_dependency_x_range() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                r#"{
  "name": "com.test.vpm.pkg1",
  "version": "2.0.0",
  "displayName": "com.test.vpm.pkg1",
  "description": "desc",
  "unity": "2022.3",
  "author": {"name": "Author", "email": "author@example.com"},
  "vpmDependencies": {
    "com.vrchat.base": "3.5.x"
  },
  "url": "https://download.example/pkg1-v2.zip"
}"#
                .to_string(),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        let result = fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await;
        assert!(result.is_ok());

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 2);
        assert_eq!(pkg1.versions[0].version, "2.0.0");
        assert_eq!(pkg1.versions[1].version, "1.0.0");
    }

    #[tokio::test]
    async fn fetch_rejects_manifest_with_invalid_zip_sha256() {
        let manifest = manifest_two_packages();
        let mut lockfile = initial_lockfile();

        let github = Arc::new(FakeGitHub {
            releases: HashMap::from([
                (
                    "owner1/repo1".to_string(),
                    vec![Release::new(
                        "v2.0.0".to_string(),
                        Some("https://assets.example/pkg1-v2.json".to_string()),
                    )],
                ),
                ("owner2/repo2".to_string(), Vec::new()),
            ]),
            assets: HashMap::from([(
                "https://assets.example/pkg1-v2.json".to_string(),
                r#"{
  "name": "com.test.vpm.pkg1",
  "version": "2.0.0",
  "displayName": "com.test.vpm.pkg1",
  "description": "desc",
  "unity": "2022.3",
  "author": {"name": "Author", "email": "author@example.com"},
  "url": "https://download.example/pkg1-v2.zip",
  "zipSHA256": "abc123"
}"#
                .to_string(),
            )]),
            delays_ms: HashMap::new(),
        });

        let fetcher = PackageFetcher::new(
            github,
            FetcherConfig {
                max_concurrent: 4,
                max_retries: 0,
                asset_name: "package.json".to_string(),
            },
        );

        let result = fetcher
            .fetch(&manifest, &mut lockfile, None::<&TestProgress>)
            .await;
        assert!(matches!(
            result,
            Err(Error::FetchPartialFailure { count: 1 })
        ));

        let pkg1 = lockfile.get_package("com.test.vpm.pkg1").unwrap();
        assert_eq!(pkg1.versions.len(), 1);
        assert_eq!(pkg1.versions[0].version, "1.0.0");
    }
}
