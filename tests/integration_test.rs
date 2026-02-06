mod common;

use async_trait::async_trait;
use common::{SAMPLE_CONFIG, SAMPLE_LOCKFILE, SAMPLE_LOCKFILE_NO_HASH, TestEnv};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use voyager::cli::{AddArgs, ConfigPaths, LockArgs, RemoveArgs};
use voyager::commands;
use voyager::config::{Manifest, Package, Vpm};
use voyager::context::AppContext;
use voyager::domain::{Release, Repository};
use voyager::error::{Error, Result};
use voyager::infra::GitHubApi;
use voyager::lock::{LockedPackage, Lockfile, compute_manifest_hash_from_manifest};
use voyager::services::{check_and_load, generate_from_lockfile};

struct TestGitHub;

#[async_trait]
impl GitHubApi for TestGitHub {
    async fn get_releases(&self, _repo: &Repository, _asset_name: &str) -> Result<Vec<Release>> {
        Ok(Vec::new())
    }

    async fn download_assets(
        &self,
        _releases: Vec<Release>,
        _max_concurrent: usize,
        _max_retries: u32,
    ) -> Vec<(Release, Result<String>)> {
        Vec::new()
    }

    async fn verify_repository(&self, _repo: &Repository) -> Result<()> {
        Ok(())
    }
}

struct MutatingGitHub {
    config_path: PathBuf,
}

#[async_trait]
impl GitHubApi for MutatingGitHub {
    async fn get_releases(&self, _repo: &Repository, _asset_name: &str) -> Result<Vec<Release>> {
        Ok(Vec::new())
    }

    async fn download_assets(
        &self,
        _releases: Vec<Release>,
        _max_concurrent: usize,
        _max_retries: u32,
    ) -> Vec<(Release, Result<String>)> {
        Vec::new()
    }

    async fn verify_repository(&self, _repo: &Repository) -> Result<()> {
        let changed = sample_manifest(
            "Changed During Verify",
            &[("com.test.vpm.pkg", "owner/repo")],
        );
        changed.save(&self.config_path)?;
        Ok(())
    }
}

fn sample_manifest(name: &str, packages: &[(&str, &str)]) -> Manifest {
    Manifest {
        vpm: Vpm {
            id: "com.test.vpm".to_string(),
            name: name.to_string(),
            author: "Test".to_string(),
            url: "https://example.com/index.json".to_string(),
        },
        packages: packages
            .iter()
            .map(|(id, repo)| Package {
                id: (*id).to_string(),
                repository: Repository::parse(repo).unwrap(),
            })
            .collect(),
    }
}

fn lockfile_with_packages(manifest_hash: &str, packages: &[(&str, &str)]) -> Lockfile {
    Lockfile {
        version: 1,
        manifest_hash: Some(manifest_hash.to_string()),
        packages: packages
            .iter()
            .map(|(id, repo)| LockedPackage {
                id: (*id).to_string(),
                repository: Repository::parse(repo).unwrap(),
                versions: Vec::new(),
            })
            .collect(),
    }
}

fn txn_path(config_path: &Path) -> PathBuf {
    config_path.with_extension("txn")
}

fn write_txn(
    config_path: &Path,
    old_manifest: &str,
    old_lock: Option<&str>,
    new_manifest: &str,
    new_lock: &str,
) {
    let tx_json = serde_json::json!({
        "old_manifest": old_manifest,
        "old_lock": old_lock,
        "new_manifest": new_manifest,
        "new_lock": new_lock,
    });
    std::fs::write(
        txn_path(config_path),
        serde_json::to_string_pretty(&tx_json).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn add_recovers_partial_transaction_before_writing() -> Result<()> {
    let env = TestEnv::new();

    let old_manifest = sample_manifest("Old", &[]);
    old_manifest.save(&env.config_path)?;
    let old_manifest_content = std::fs::read_to_string(&env.config_path).unwrap();
    let old_hash = compute_manifest_hash_from_manifest(&old_manifest, &env.config_path)?;

    let old_lock = lockfile_with_packages(&old_hash, &[]);
    old_lock.save(&env.lock_path)?;
    let old_lock_content = std::fs::read_to_string(&env.lock_path).unwrap();

    let new_manifest = sample_manifest("New", &[("com.test.vpm.temp", "owner/temp")]);
    let new_manifest_content = toml::to_string_pretty(&new_manifest).unwrap();
    let new_hash = compute_manifest_hash_from_manifest(&new_manifest, &env.config_path)?;
    let new_lock = lockfile_with_packages(&new_hash, &[("com.test.vpm.temp", "owner/temp")]);
    let new_lock_content = toml::to_string_pretty(&new_lock).unwrap();

    write_txn(
        &env.config_path,
        &old_manifest_content,
        Some(&old_lock_content),
        &new_manifest_content,
        &new_lock_content,
    );
    std::fs::write(&env.config_path, &new_manifest_content).unwrap();

    let paths = ConfigPaths::new(env.config_path.clone());
    let ctx = AppContext::with_github(paths, Arc::new(TestGitHub));
    commands::add::execute(
        AddArgs {
            repository: "owner/repo".to_string(),
            id: Some("com.test.vpm.added".to_string()),
            github_token: None,
        },
        &ctx,
    )
    .await?;

    assert!(!txn_path(&env.config_path).exists());

    let manifest = Manifest::load(&env.config_path)?;
    assert_eq!(manifest.vpm.name, "Old");
    assert_eq!(manifest.packages.len(), 1);
    assert_eq!(manifest.packages[0].id, "com.test.vpm.added");

    let lock = Lockfile::load(&env.lock_path)?;
    let expected_hash = compute_manifest_hash_from_manifest(&manifest, &env.config_path)?;
    assert_eq!(lock.manifest_hash.as_deref(), Some(expected_hash.as_str()));

    Ok(())
}

#[test]
fn remove_recovers_partial_transaction_before_writing() -> Result<()> {
    let env = TestEnv::new();

    let old_manifest = sample_manifest(
        "Old",
        &[
            ("com.test.vpm.target", "owner/target"),
            ("com.test.vpm.keep", "owner/keep"),
        ],
    );
    old_manifest.save(&env.config_path)?;
    let old_manifest_content = std::fs::read_to_string(&env.config_path).unwrap();
    let old_hash = compute_manifest_hash_from_manifest(&old_manifest, &env.config_path)?;

    let old_lock = lockfile_with_packages(
        &old_hash,
        &[
            ("com.test.vpm.target", "owner/target"),
            ("com.test.vpm.keep", "owner/keep"),
        ],
    );
    old_lock.save(&env.lock_path)?;
    let old_lock_content = std::fs::read_to_string(&env.lock_path).unwrap();

    let new_manifest = sample_manifest("New", &[("com.test.vpm.keep", "owner/keep")]);
    let new_manifest_content = toml::to_string_pretty(&new_manifest).unwrap();
    let new_hash = compute_manifest_hash_from_manifest(&new_manifest, &env.config_path)?;
    let new_lock = lockfile_with_packages(&new_hash, &[("com.test.vpm.keep", "owner/keep")]);
    let new_lock_content = toml::to_string_pretty(&new_lock).unwrap();

    write_txn(
        &env.config_path,
        &old_manifest_content,
        Some(&old_lock_content),
        &new_manifest_content,
        &new_lock_content,
    );
    std::fs::write(&env.config_path, &new_manifest_content).unwrap();

    let paths = ConfigPaths::new(env.config_path.clone());
    commands::remove::execute(
        RemoveArgs {
            package_id: "com.test.vpm.target".to_string(),
        },
        &paths,
    )?;

    assert!(!txn_path(&env.config_path).exists());

    let manifest = Manifest::load(&env.config_path)?;
    assert_eq!(manifest.vpm.name, "Old");
    assert_eq!(manifest.packages.len(), 1);
    assert_eq!(manifest.packages[0].id, "com.test.vpm.keep");

    let lock = Lockfile::load(&env.lock_path)?;
    assert_eq!(lock.packages.len(), 1);
    assert_eq!(lock.packages[0].id, "com.test.vpm.keep");
    let expected_hash = compute_manifest_hash_from_manifest(&manifest, &env.config_path)?;
    assert_eq!(lock.manifest_hash.as_deref(), Some(expected_hash.as_str()));

    Ok(())
}

#[test]
fn check_and_load_works_with_valid_files() -> Result<()> {
    let env = TestEnv::new();
    env.write_config(SAMPLE_CONFIG);
    // Use lockfile without manifest_hash to skip hash validation
    env.write_lockfile(SAMPLE_LOCKFILE_NO_HASH);

    let result = check_and_load(&env.config_path, &env.lock_path)?;

    assert_eq!(result.manifest.vpm.id, "com.test.vpm");
    assert_eq!(result.manifest.vpm.name, "Test VPM");
    assert_eq!(result.manifest.packages.len(), 1);
    assert_eq!(result.lockfile.packages.len(), 1);

    Ok(())
}

#[test]
fn check_and_load_creates_lockfile_if_missing() -> Result<()> {
    let env = TestEnv::new();
    env.write_config(SAMPLE_CONFIG);

    assert!(!env.lock_exists());

    let result = check_and_load(&env.config_path, &env.lock_path)?;

    assert_eq!(result.manifest.vpm.id, "com.test.vpm");
    assert!(result.lockfile.packages.is_empty());

    Ok(())
}

#[test]
fn manifest_load_and_save_roundtrip() -> Result<()> {
    let env = TestEnv::new();
    env.write_config(SAMPLE_CONFIG);

    let manifest = Manifest::load(&env.config_path)?;
    assert_eq!(manifest.vpm.id, "com.test.vpm");
    assert_eq!(manifest.packages.len(), 1);

    let new_path = env.temp_dir.path().join("new_config.toml");
    manifest.save(&new_path)?;

    let reloaded = Manifest::load(&new_path)?;
    assert_eq!(reloaded.vpm.id, manifest.vpm.id);
    assert_eq!(reloaded.packages.len(), manifest.packages.len());

    Ok(())
}

#[test]
fn lockfile_load_and_save_roundtrip() -> Result<()> {
    let env = TestEnv::new();
    env.write_lockfile(SAMPLE_LOCKFILE);

    let lockfile = Lockfile::load(&env.lock_path)?;
    assert_eq!(lockfile.packages.len(), 1);
    assert_eq!(lockfile.packages[0].versions.len(), 1);

    let new_path = env.temp_dir.path().join("new_lock.lock");
    lockfile.save(&new_path)?;

    let reloaded = Lockfile::load(&new_path)?;
    assert_eq!(reloaded.packages.len(), lockfile.packages.len());

    Ok(())
}

#[test]
fn generate_from_lockfile_creates_valid_output() -> Result<()> {
    let env = TestEnv::new();
    env.write_config(SAMPLE_CONFIG);
    env.write_lockfile(SAMPLE_LOCKFILE);

    let manifest = Manifest::load(&env.config_path)?;
    let lockfile = Lockfile::load(&env.lock_path)?;

    let output = generate_from_lockfile(&manifest, &lockfile)?;

    assert_eq!(output.id, "com.test.vpm");
    assert_eq!(output.name, "Test VPM");
    assert_eq!(output.packages.len(), 1);

    let pkg = output.packages.get("com.test.vpm.package1").unwrap();
    assert_eq!(pkg.versions.len(), 1);

    let version = pkg.versions.get("1.0.0").unwrap();
    assert_eq!(version.name, "com.test.vpm.package1");
    assert_eq!(version.version, "1.0.0");

    Ok(())
}

#[test]
fn config_paths_derives_lock_from_config() {
    let paths = ConfigPaths::new("custom/path/my-config.toml".into());
    assert_eq!(
        paths.config_path().to_str().unwrap(),
        "custom/path/my-config.toml"
    );
    assert_eq!(
        paths.lock_path().to_str().unwrap(),
        "custom/path/my-config.lock"
    );
}

#[test]
fn config_paths_default_uses_voyager_toml() {
    let paths = ConfigPaths::default();
    assert_eq!(paths.config_path().to_str().unwrap(), "voyager.toml");
    assert_eq!(paths.lock_path().to_str().unwrap(), "voyager.lock");
}

#[test]
fn manifest_validation_rejects_invalid_vpm_id() {
    let env = TestEnv::new();
    env.write_config(
        r#"[vpm]
id = "invalid"
name = "Test"
author = "Test"
url = "https://test.com"
"#,
    );

    let result = Manifest::load(&env.config_path);
    assert!(result.is_err());
}

#[test]
fn manifest_validation_rejects_invalid_url() {
    let env = TestEnv::new();
    env.write_config(
        r#"[vpm]
id = "com.test.vpm"
name = "Test"
author = "Test"
url = "not-a-valid-url"
"#,
    );

    let result = Manifest::load(&env.config_path);
    assert!(result.is_err());
}

#[test]
fn manifest_validation_rejects_invalid_package_id() {
    let env = TestEnv::new();
    // Package ID must be a valid reverse domain (at least 2 parts)
    env.write_config(
        r#"[vpm]
id = "com.test.vpm"
name = "Test"
author = "Test"
url = "https://test.com"

[[packages]]
id = "invalid"
repository = "owner/repo"
"#,
    );

    let result = Manifest::load(&env.config_path);
    assert!(result.is_err());
}

#[test]
fn manifest_rejects_duplicate_package_ids() {
    let env = TestEnv::new();
    env.write_config(
        r#"[vpm]
id = "com.test.vpm"
name = "Test"
author = "Test"
url = "https://test.com"

[[packages]]
id = "com.test.vpm.pkg"
repository = "owner/repo1"

[[packages]]
id = "com.test.vpm.pkg"
repository = "owner/repo2"
"#,
    );

    let result = Manifest::load(&env.config_path);
    assert!(result.is_err());
}

#[tokio::test]
async fn lock_rejects_manifest_changes_during_repository_verification() -> Result<()> {
    let env = TestEnv::new();

    let manifest = sample_manifest("Original", &[("com.test.vpm.pkg", "owner/repo")]);
    manifest.save(&env.config_path)?;

    let lockfile = lockfile_with_packages("stale-hash", &[("com.test.vpm.pkg", "owner/repo")]);
    lockfile.save(&env.lock_path)?;

    let paths = ConfigPaths::new(env.config_path.clone());
    let ctx = AppContext::with_github(
        paths,
        Arc::new(MutatingGitHub {
            config_path: env.config_path.clone(),
        }),
    );

    let result = commands::lock::execute(
        LockArgs {
            check: false,
            github_token: None,
        },
        &ctx,
    )
    .await;

    assert!(matches!(result, Err(Error::ManifestHashMismatch)));

    let persisted = Lockfile::load(&env.lock_path)?;
    assert_eq!(persisted.manifest_hash.as_deref(), Some("stale-hash"));

    Ok(())
}
