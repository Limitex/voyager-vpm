use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Barrier};
use std::thread;
use tempfile::TempDir;
use voyager::config::Manifest;
use voyager::lock::{Lockfile, compute_manifest_hash};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn voy_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_voy"))
}

fn run_voy(args: &[&str], cwd: &Path) -> Output {
    Command::new(voy_bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to run voy")
}

fn can_bind_localhost() -> bool {
    std::net::TcpListener::bind("127.0.0.1:0").is_ok()
}

fn write(path: &Path, content: &str) {
    std::fs::write(path, content).expect("failed to write test file");
}

fn make_lock_content(manifest_hash: &str) -> String {
    format!(
        r#"version = 1
manifest_hash = "{manifest_hash}"
packages = []
"#
    )
}

fn make_manifest_empty(name: &str) -> String {
    format!(
        r#"[vpm]
id = "com.test.vpm"
name = "{name}"
author = "Author"
url = "https://example.com/index.json"
packages = []
"#
    )
}

fn make_manifest_single_package(name: &str) -> String {
    format!(
        r#"[vpm]
id = "com.test.vpm"
name = "{name}"
author = "Author"
url = "https://example.com/index.json"

[[packages]]
id = "com.test.vpm.package1"
repository = "testowner/testrepo"
"#
    )
}

fn make_lock_with_single_package(manifest_hash: &str) -> String {
    format!(
        r#"version = 1
manifest_hash = "{manifest_hash}"

[[packages]]
id = "com.test.vpm.package1"
repository = "testowner/testrepo"
versions = []
"#
    )
}

fn make_lock_with_two_versions(manifest_hash: &str) -> String {
    format!(
        r#"version = 1
manifest_hash = "{manifest_hash}"

[[packages]]
id = "com.test.vpm.package1"
repository = "testowner/testrepo"

[[packages.versions]]
tag = "v2.0.0"
version = "2.0.0"
url = "https://example.com/package-2.0.0.zip"
hash = "sha256:222"

[packages.versions.manifest]
name = "com.test.vpm.package1"
displayName = "Test Package"
version = "2.0.0"
unity = "2022.3"
description = "A test package"
license = "MIT"
url = "https://example.com/package-2.0.0.zip"

[packages.versions.manifest.author]
name = "Test Author"

[[packages.versions]]
tag = "v1.0.0"
version = "1.0.0"
url = "https://example.com/package-1.0.0.zip"
hash = "sha256:111"

[packages.versions.manifest]
name = "com.test.vpm.package1"
displayName = "Test Package"
version = "1.0.0"
unity = "2022.3"
description = "A test package"
license = "MIT"
url = "https://example.com/package-1.0.0.zip"

[packages.versions.manifest.author]
name = "Test Author"
"#
    )
}

fn make_lock_with_stale_package_versions(manifest_hash: &str) -> String {
    format!(
        r#"version = 1
manifest_hash = "{manifest_hash}"

[[packages]]
id = "com.test.vpm.stale"
repository = "stale-owner/stale-repo"

[[packages.versions]]
tag = "v1.0.0"
version = "1.0.0"
url = "https://example.com/stale-1.0.0.zip"
hash = "sha256:stale111"

[packages.versions.manifest]
name = "com.test.vpm.stale"
displayName = "Stale Package"
version = "1.0.0"
unity = "2022.3"
description = "Stale package"
license = "MIT"
url = "https://example.com/stale-1.0.0.zip"

[packages.versions.manifest.author]
name = "Test Author"
"#
    )
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
    write(
        &txn_path(config_path),
        &serde_json::to_string_pretty(&tx_json).unwrap(),
    );
}

#[test]
fn fetch_fails_on_manifest_hash_mismatch() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(
        &config_path,
        r#"[vpm]
id = "com.test.vpm"
name = "Test"
author = "Author"
url = "https://example.com/index.json"

packages = []
"#,
    );

    write(
        &lock_path,
        r#"version = 1
manifest_hash = "definitely-wrong-hash"
packages = []
"#,
    );

    let output = run_voy(
        &[
            "fetch",
            "--config",
            config_path.to_str().unwrap(),
            "--max-retries",
            "0",
        ],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(78));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Manifest has been modified outside of voyager"));
}

#[test]
fn fetch_prunes_stale_packages_when_manifest_no_longer_contains_them() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(&config_path, &make_manifest_empty("Test"));
    let hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_with_stale_package_versions(&hash));

    let output = run_voy(
        &[
            "fetch",
            "--config",
            config_path.to_str().unwrap(),
            "--max-retries",
            "0",
            "--max-concurrent",
            "1",
        ],
        dir.path(),
    );
    assert_eq!(output.status.code(), Some(0));

    let lock = Lockfile::load(&lock_path).unwrap();
    assert_eq!(lock.manifest_hash.as_deref(), Some(hash.as_str()));
    assert!(lock.packages.is_empty());
}

#[test]
fn validate_succeeds_for_empty_index() {
    let dir = TempDir::new().unwrap();
    let index_path = dir.path().join("index.json");

    write(
        &index_path,
        r#"{
  "name": "Test VPM",
  "id": "com.test.vpm",
  "url": "https://example.com/index.json",
  "author": "Author",
  "packages": {}
}"#,
    );

    let output = run_voy(&["validate", index_path.to_str().unwrap()], dir.path());
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn validate_succeeds_when_head_is_blocked_but_get_fallback_works() {
    if !can_bind_localhost() {
        return;
    }

    let dir = TempDir::new().unwrap();
    let index_path = dir.path().join("index.json");

    let rt = tokio::runtime::Runtime::new().unwrap();
    let mock_server = rt.block_on(async { MockServer::start().await });
    rt.block_on(async {
        Mock::given(method("HEAD"))
            .and(path("/package.zip"))
            .respond_with(ResponseTemplate::new(405))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/package.zip"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&mock_server)
            .await;
    });

    write(
        &index_path,
        &format!(
            r#"{{
  "name": "Test VPM",
  "id": "com.test.vpm",
  "url": "https://example.com/index.json",
  "author": "Author",
  "packages": {{
    "com.test.vpm.pkg": {{
      "versions": {{
        "1.0.0": {{
          "name": "com.test.vpm.pkg",
          "version": "1.0.0",
          "displayName": "Test Package",
          "description": "desc",
          "unity": "2022.3",
          "author": {{ "name": "Author" }},
          "url": "{}/package.zip"
        }}
      }}
    }}
  }}
}}"#,
            mock_server.uri()
        ),
    );

    let output = run_voy(
        &[
            "validate",
            index_path.to_str().unwrap(),
            "--max-retries",
            "0",
            "--max-concurrent",
            "1",
        ],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("all valid"));
}

#[test]
fn validate_fails_when_url_is_unreachable() {
    let dir = TempDir::new().unwrap();
    let index_path = dir.path().join("index.json");

    write(
        &index_path,
        r#"{
  "name": "Test VPM",
  "id": "com.test.vpm",
  "url": "https://example.com/index.json",
  "author": "Author",
  "packages": {
    "com.test.vpm.pkg": {
      "versions": {
        "1.0.0": {
          "name": "com.test.vpm.pkg",
          "version": "1.0.0",
          "displayName": "Test Package",
          "description": "desc",
          "unity": "2022.3",
          "author": { "name": "Author" },
          "url": "http://127.0.0.1:9/package.zip"
        }
      }
    }
  }
}"#,
    );

    let output = run_voy(
        &[
            "validate",
            index_path.to_str().unwrap(),
            "--max-retries",
            "0",
            "--max-concurrent",
            "1",
        ],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(69));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("URL validation failed"));
}

#[test]
fn validate_fails_on_malformed_json() {
    let dir = TempDir::new().unwrap();
    let index_path = dir.path().join("index.json");
    write(&index_path, "{ not-valid-json }");

    let output = run_voy(&["validate", index_path.to_str().unwrap()], dir.path());

    assert_eq!(output.status.code(), Some(65));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Failed to parse JSON"));
}

#[test]
fn completions_succeeds_when_transaction_log_is_corrupted() {
    let dir = TempDir::new().unwrap();
    write(
        &txn_path(&dir.path().join("voyager.toml")),
        "{ this is not valid json }",
    );

    let output = run_voy(&["completions", "bash"], dir.path());

    assert_eq!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("Failed to parse JSON"));
}

#[test]
fn init_force_removes_stale_transaction_log() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");

    write(
        &txn_path(&config_path),
        "{ this is stale and should be removed by init --force }",
    );

    let init_output = run_voy(
        &[
            "init",
            "--force",
            "--config",
            config_path.to_str().unwrap(),
            "--name",
            "Fresh",
            "--id",
            "com.fresh.vpm",
            "--author",
            "Author",
            "--url",
            "https://example.com/index.json",
        ],
        dir.path(),
    );

    assert_eq!(init_output.status.code(), Some(0));
    assert!(!txn_path(&config_path).exists());

    let list_output = run_voy(
        &["list", "--config", config_path.to_str().unwrap()],
        dir.path(),
    );
    assert_eq!(list_output.status.code(), Some(0));
}

#[test]
fn init_with_default_config_path_succeeds() {
    let dir = TempDir::new().unwrap();

    let output = run_voy(
        &[
            "init",
            "--force",
            "--name",
            "Default",
            "--id",
            "com.default.vpm",
            "--author",
            "Author",
            "--url",
            "https://example.com/index.json",
        ],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(dir.path().join("voyager.toml").exists());
    assert!(dir.path().join("voyager.lock").exists());
    assert!(!dir.path().join("voyager.txn").exists());
}

#[test]
fn list_rolls_back_partial_transaction_before_running() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    let old_manifest = r#"[vpm]
id = "com.test.vpm"
name = "Old"
author = "Author"
url = "https://example.com/index.json"
packages = []
"#;
    let new_manifest = r#"[vpm]
id = "com.test.vpm"
name = "New"
author = "Author"
url = "https://example.com/index.json"
packages = []
"#;

    write(&config_path, old_manifest);
    let old_hash = compute_manifest_hash(&config_path).unwrap();
    let old_lock = make_lock_content(&old_hash);
    write(&lock_path, &old_lock);

    write(&config_path, new_manifest);
    let new_hash = compute_manifest_hash(&config_path).unwrap();
    let new_lock = make_lock_content(&new_hash);

    write_txn(
        &config_path,
        old_manifest,
        Some(&old_lock),
        new_manifest,
        &new_lock,
    );

    let output = run_voy(
        &["list", "--config", config_path.to_str().unwrap()],
        dir.path(),
    );
    assert_eq!(output.status.code(), Some(0));

    let persisted_manifest = std::fs::read_to_string(&config_path).unwrap();
    let persisted_lock = std::fs::read_to_string(&lock_path).unwrap();
    assert_eq!(persisted_manifest, old_manifest);
    assert_eq!(persisted_lock, old_lock);
    assert!(!txn_path(&config_path).exists());
}

#[test]
fn list_finalizes_committed_transaction_when_txn_file_remains() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    let old_manifest = r#"[vpm]
id = "com.test.vpm"
name = "Old"
author = "Author"
url = "https://example.com/index.json"
packages = []
"#;
    let new_manifest = r#"[vpm]
id = "com.test.vpm"
name = "New"
author = "Author"
url = "https://example.com/index.json"
packages = []
"#;

    write(&config_path, old_manifest);
    let old_hash = compute_manifest_hash(&config_path).unwrap();
    let old_lock = make_lock_content(&old_hash);

    write(&config_path, new_manifest);
    let new_hash = compute_manifest_hash(&config_path).unwrap();
    let new_lock = make_lock_content(&new_hash);
    write(&lock_path, &new_lock);

    write_txn(
        &config_path,
        old_manifest,
        Some(&old_lock),
        new_manifest,
        &new_lock,
    );

    let output = run_voy(
        &["list", "--config", config_path.to_str().unwrap()],
        dir.path(),
    );
    assert_eq!(output.status.code(), Some(0));

    let persisted_manifest = std::fs::read_to_string(&config_path).unwrap();
    let persisted_lock = std::fs::read_to_string(&lock_path).unwrap();
    assert_eq!(persisted_manifest, new_manifest);
    assert_eq!(persisted_lock, new_lock);
    assert!(!txn_path(&config_path).exists());
}

#[test]
fn list_fails_when_transaction_log_is_corrupted() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    let manifest = r#"[vpm]
id = "com.test.vpm"
name = "Test"
author = "Author"
url = "https://example.com/index.json"
packages = []
"#;
    write(&config_path, manifest);
    let hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_content(&hash));

    write(&txn_path(&config_path), "{ this is not valid json }");

    let output = run_voy(
        &["list", "--config", config_path.to_str().unwrap()],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(65));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Failed to parse JSON"));
}

#[test]
fn lock_check_fails_when_manifest_hash_mismatch() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(&config_path, &make_manifest_empty("Test"));
    write(&lock_path, &make_lock_content("definitely-wrong-hash"));

    let output = run_voy(
        &["lock", "--check", "--config", config_path.to_str().unwrap()],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(78));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Manifest has been modified outside of voyager"));
}

#[test]
fn lock_updates_manifest_hash_when_manifest_changes() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(&config_path, &make_manifest_empty("Old"));
    let old_hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_content(&old_hash));

    write(&config_path, &make_manifest_empty("New"));
    let expected_hash = compute_manifest_hash(&config_path).unwrap();

    let output = run_voy(
        &["lock", "--config", config_path.to_str().unwrap()],
        dir.path(),
    );
    assert_eq!(output.status.code(), Some(0));

    let lock = Lockfile::load(&lock_path).unwrap();
    assert_eq!(lock.manifest_hash.as_deref(), Some(expected_hash.as_str()));
}

#[test]
fn lock_check_fails_when_lock_missing_manifest_hash() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(&config_path, &make_manifest_empty("Test"));
    write(
        &lock_path,
        r#"version = 1
packages = []
"#,
    );

    let output = run_voy(
        &["lock", "--check", "--config", config_path.to_str().unwrap()],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(78));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Manifest hash does not match lock file"));
}

#[test]
fn fetch_succeeds_with_empty_packages_and_matching_hash() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(&config_path, &make_manifest_empty("Test"));
    let hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_content(&hash));

    let output = run_voy(
        &[
            "fetch",
            "--config",
            config_path.to_str().unwrap(),
            "--max-retries",
            "0",
            "--max-concurrent",
            "1",
        ],
        dir.path(),
    );
    assert_eq!(output.status.code(), Some(0));

    let lock = Lockfile::load(&lock_path).unwrap();
    assert_eq!(lock.manifest_hash.as_deref(), Some(hash.as_str()));
    assert!(lock.packages.is_empty());
}

#[test]
fn init_then_generate_produces_valid_empty_index() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let index_path = dir.path().join("index.json");

    let init = run_voy(
        &[
            "init",
            "--force",
            "--config",
            config_path.to_str().unwrap(),
            "--name",
            "Fresh",
            "--id",
            "com.fresh.vpm",
            "--author",
            "Author",
            "--url",
            "https://example.com/index.json",
        ],
        dir.path(),
    );
    assert_eq!(init.status.code(), Some(0));

    let generate = run_voy(
        &[
            "generate",
            "--config",
            config_path.to_str().unwrap(),
            "--output",
            index_path.to_str().unwrap(),
        ],
        dir.path(),
    );
    assert_eq!(generate.status.code(), Some(0));

    let output: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(index_path).unwrap()).unwrap();
    assert_eq!(output["id"], "com.fresh.vpm");
    assert_eq!(output["name"], "Fresh");
    assert_eq!(output["author"], "Author");
    assert_eq!(output["packages"], serde_json::json!({}));
}

#[test]
fn generate_fails_when_manifest_has_package_but_lock_is_empty() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");
    let index_path = dir.path().join("index.json");

    write(&config_path, &make_manifest_single_package("Test"));
    let hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_content(&hash));

    let output = run_voy(
        &[
            "generate",
            "--config",
            config_path.to_str().unwrap(),
            "--output",
            index_path.to_str().unwrap(),
        ],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(78));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Lock file has no packages. Run 'voy fetch' first."));
}

#[test]
fn remove_updates_manifest_and_lockfile() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(&config_path, &make_manifest_single_package("Test"));
    let hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_with_single_package(&hash));

    let output = run_voy(
        &[
            "remove",
            "com.test.vpm.package1",
            "--config",
            config_path.to_str().unwrap(),
        ],
        dir.path(),
    );
    assert_eq!(output.status.code(), Some(0));

    let manifest = Manifest::load(&config_path).unwrap();
    assert!(manifest.packages.is_empty());

    let expected_hash = compute_manifest_hash(&config_path).unwrap();
    let lock = Lockfile::load(&lock_path).unwrap();
    assert!(lock.packages.is_empty());
    assert_eq!(lock.manifest_hash.as_deref(), Some(expected_hash.as_str()));
}

#[test]
fn add_fails_fast_for_invalid_repository_format() {
    let dir = TempDir::new().unwrap();

    let output = run_voy(&["add", "invalid_repo"], dir.path());

    assert_eq!(output.status.code(), Some(78));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid repository format"));
}

#[test]
fn add_rejects_package_id_outside_vpm_namespace() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(&config_path, &make_manifest_empty("Test"));
    let hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_content(&hash));

    let output = run_voy(
        &[
            "add",
            "owner/repo",
            "--id",
            "org.other.pkg",
            "--config",
            config_path.to_str().unwrap(),
        ],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(78));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid package ID"));

    let manifest = Manifest::load(&config_path).unwrap();
    assert!(manifest.packages.is_empty());
    let lock = Lockfile::load(&lock_path).unwrap();
    assert_eq!(lock.manifest_hash.as_deref(), Some(hash.as_str()));
}

#[test]
fn add_rejects_duplicate_package_id_before_repository_verification() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(&config_path, &make_manifest_single_package("Test"));
    let hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_with_single_package(&hash));

    let output = run_voy(
        &[
            "add",
            "another/repo",
            "--id",
            "com.test.vpm.package1",
            "--config",
            config_path.to_str().unwrap(),
        ],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(78));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already exists"));
}

#[test]
fn info_prints_versions_from_lockfile() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(&config_path, &make_manifest_single_package("Test"));
    let hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_with_two_versions(&hash));

    let output = run_voy(
        &[
            "info",
            "com.test.vpm.package1",
            "--config",
            config_path.to_str().unwrap(),
        ],
        dir.path(),
    );
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("com.test.vpm.package1"));
    assert!(stdout.contains("2.0.0"));
    assert!(stdout.contains("1.0.0"));
}

#[test]
fn info_shows_fetch_hint_when_versions_are_not_fetched_yet() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(&config_path, &make_manifest_single_package("Test"));
    let hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_with_single_package(&hash));

    let output = run_voy(
        &[
            "info",
            "com.test.vpm.package1",
            "--config",
            config_path.to_str().unwrap(),
        ],
        dir.path(),
    );
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("No versions fetched yet. Run 'voy fetch' first."));
}

#[test]
fn validate_rejects_too_high_max_retries() {
    let dir = TempDir::new().unwrap();
    let index_path = dir.path().join("index.json");

    write(
        &index_path,
        r#"{
  "name": "Test VPM",
  "id": "com.test.vpm",
  "url": "https://example.com/index.json",
  "author": "Author",
  "packages": {}
}"#,
    );

    let output = run_voy(
        &[
            "validate",
            index_path.to_str().unwrap(),
            "--max-retries",
            "9",
        ],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("max-retries must be at most 8"));
}

#[test]
fn lock_check_succeeds_when_manifest_hash_matches() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(&config_path, &make_manifest_empty("Test"));
    let hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_content(&hash));

    let output = run_voy(
        &["lock", "--check", "--config", config_path.to_str().unwrap()],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Manifest hash matches lock file"));
}

#[test]
fn lock_fails_when_config_file_is_missing() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("missing.toml");

    let output = run_voy(
        &["lock", "--config", config_path.to_str().unwrap()],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(78));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Configuration file"));
    assert!(stderr.contains("not found"));
}

#[test]
fn lock_fails_when_lock_file_is_missing() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    write(&config_path, &make_manifest_empty("Test"));

    let output = run_voy(
        &["lock", "--config", config_path.to_str().unwrap()],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(78));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Lock file"));
    assert!(stderr.contains("Run 'voy fetch' first."));
}

#[test]
fn generate_fails_when_lock_file_is_missing() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let output_path = dir.path().join("index.json");

    write(&config_path, &make_manifest_empty("Test"));

    let output = run_voy(
        &[
            "generate",
            "--config",
            config_path.to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
        ],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(78));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Lock file"));
    assert!(stderr.contains("Run 'voy fetch' first."));
}

#[test]
fn generate_rolls_back_partial_transaction_before_missing_lock_error() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");
    let output_path = dir.path().join("index.json");

    let old_manifest = make_manifest_empty("Old");
    let new_manifest = make_manifest_empty("New");

    write(&config_path, &new_manifest);
    let new_hash = compute_manifest_hash(&config_path).unwrap();
    let new_lock = make_lock_content(&new_hash);

    write_txn(&config_path, &old_manifest, None, &new_manifest, &new_lock);

    if lock_path.exists() {
        std::fs::remove_file(&lock_path).unwrap();
    }

    let output = run_voy(
        &[
            "generate",
            "--config",
            config_path.to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
        ],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(78));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Lock file"));
    assert!(stderr.contains("Run 'voy fetch' first."));

    let persisted_manifest = std::fs::read_to_string(&config_path).unwrap();
    assert_eq!(persisted_manifest, old_manifest);
    assert!(!txn_path(&config_path).exists());
}

#[test]
fn generate_outputs_versions_from_lockfile() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");
    let output_path = dir.path().join("index.json");

    write(&config_path, &make_manifest_single_package("Test"));
    let hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_with_two_versions(&hash));

    let output = run_voy(
        &[
            "generate",
            "--config",
            config_path.to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
        ],
        dir.path(),
    );
    assert_eq!(output.status.code(), Some(0));

    let output_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(output_path).unwrap()).unwrap();

    let versions = &output_json["packages"]["com.test.vpm.package1"]["versions"];
    assert!(versions.get("2.0.0").is_some());
    assert!(versions.get("1.0.0").is_some());
}

#[test]
fn remove_fails_when_package_does_not_exist() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(&config_path, &make_manifest_empty("Test"));
    let hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_content(&hash));

    let output = run_voy(
        &[
            "remove",
            "com.test.vpm.missing",
            "--config",
            config_path.to_str().unwrap(),
        ],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(78));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Package 'com.test.vpm.missing' not found"));
}

#[test]
fn info_fails_when_package_does_not_exist() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(&config_path, &make_manifest_empty("Test"));
    let hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_content(&hash));

    let output = run_voy(
        &[
            "info",
            "com.test.vpm.missing",
            "--config",
            config_path.to_str().unwrap(),
        ],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(78));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Package 'com.test.vpm.missing' not found"));
}

#[test]
fn list_package_shows_versions_in_descending_order() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(&config_path, &make_manifest_single_package("Test"));
    let hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_with_two_versions(&hash));

    let output = run_voy(
        &[
            "list",
            "com.test.vpm.package1",
            "--config",
            config_path.to_str().unwrap(),
        ],
        dir.path(),
    );
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pos_v2 = stdout.find("2.0.0").unwrap();
    let pos_v1 = stdout.find("1.0.0").unwrap();
    assert!(pos_v2 < pos_v1);
}

#[test]
fn list_shows_package_with_no_versions_when_lock_is_missing() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");

    write(&config_path, &make_manifest_single_package("Test"));

    let output = run_voy(
        &["list", "--config", config_path.to_str().unwrap()],
        dir.path(),
    );
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("com.test.vpm.package1"));
    assert!(stdout.contains("testowner/testrepo"));
}

#[test]
fn fetch_rejects_too_high_max_retries() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let lock_path = dir.path().join("voyager.lock");

    write(&config_path, &make_manifest_empty("Test"));
    let hash = compute_manifest_hash(&config_path).unwrap();
    write(&lock_path, &make_lock_content(&hash));

    let output = run_voy(
        &[
            "fetch",
            "--config",
            config_path.to_str().unwrap(),
            "--max-retries",
            "9",
        ],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("max-retries must be at most 8"));
}

#[test]
fn validate_rejects_zero_max_concurrent() {
    let dir = TempDir::new().unwrap();
    let index_path = dir.path().join("index.json");

    write(
        &index_path,
        r#"{
  "name": "Test VPM",
  "id": "com.test.vpm",
  "url": "https://example.com/index.json",
  "author": "Author",
  "packages": {}
}"#,
    );

    let output = run_voy(
        &[
            "validate",
            index_path.to_str().unwrap(),
            "--max-concurrent",
            "0",
        ],
        dir.path(),
    );

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("max-concurrent must be at least 1"));
}

#[test]
fn generate_is_safe_under_parallel_writes() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("voyager.toml");
    let output_path = dir.path().join("index.json");

    let init = run_voy(
        &[
            "init",
            "--force",
            "--config",
            config_path.to_str().unwrap(),
            "--name",
            "Parallel",
            "--id",
            "com.parallel.vpm",
            "--author",
            "Author",
            "--url",
            "https://example.com/index.json",
        ],
        dir.path(),
    );
    assert_eq!(init.status.code(), Some(0));

    let worker_count = 4usize;
    let barrier = Arc::new(Barrier::new(worker_count));
    let mut handles = Vec::new();

    for _ in 0..worker_count {
        let barrier = Arc::clone(&barrier);
        let cwd = dir.path().to_path_buf();
        let config = config_path.clone();
        let output = output_path.clone();

        handles.push(thread::spawn(move || {
            barrier.wait();
            run_voy(
                &[
                    "generate",
                    "--config",
                    config.to_str().unwrap(),
                    "--output",
                    output.to_str().unwrap(),
                ],
                &cwd,
            )
        }));
    }

    for handle in handles {
        let result = handle.join().expect("worker thread panicked");
        assert_eq!(result.status.code(), Some(0));
    }

    let output_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&output_path).unwrap()).unwrap();
    assert_eq!(output_json["id"], "com.parallel.vpm");
    assert_eq!(output_json["name"], "Parallel");

    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name();
            let name = name.to_str()?;
            if name.ends_with(".json.tmp") {
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "unexpected temporary files: {:?}",
        leftovers
    );
}
