use std::path::PathBuf;
use tempfile::TempDir;

/// Test environment for integration tests.
pub struct TestEnv {
    pub temp_dir: TempDir,
    pub config_path: PathBuf,
    pub lock_path: PathBuf,
}

impl TestEnv {
    /// Creates a new test environment with temporary directory.
    pub fn new() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_path = temp_dir.path().join("voyager.toml");
        let lock_path = temp_dir.path().join("voyager.lock");

        Self {
            temp_dir,
            config_path,
            lock_path,
        }
    }

    /// Writes content to the config file.
    pub fn write_config(&self, content: &str) {
        std::fs::write(&self.config_path, content).expect("Failed to write config");
    }

    /// Writes content to the lock file.
    pub fn write_lockfile(&self, content: &str) {
        std::fs::write(&self.lock_path, content).expect("Failed to write lockfile");
    }

    /// Checks if lock file exists.
    pub fn lock_exists(&self) -> bool {
        self.lock_path.exists()
    }
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

/// Sample valid TOML configuration.
pub const SAMPLE_CONFIG: &str = r#"[vpm]
id = "com.test.vpm"
name = "Test VPM"
author = "Test Author"
url = "https://test.example.com/index.json"

[[packages]]
id = "com.test.vpm.package1"
repository = "testowner/testrepo"
"#;

/// Sample lock file content.
pub const SAMPLE_LOCKFILE: &str = r#"version = 1
manifest_hash = "abc123"

[[packages]]
id = "com.test.vpm.package1"
repository = "testowner/testrepo"

[[packages.versions]]
tag = "v1.0.0"
version = "1.0.0"
url = "https://example.com/package.zip"
hash = "abc123def456"

[packages.versions.manifest]
name = "com.test.vpm.package1"
displayName = "Test Package"
version = "1.0.0"
unity = "2022.3"
description = "A test package"
license = "MIT"
url = "https://example.com/package.zip"

[packages.versions.manifest.author]
name = "Test Author"
"#;

/// Sample lock file without manifest_hash (for tests that need fresh loading).
pub const SAMPLE_LOCKFILE_NO_HASH: &str = r#"version = 1

[[packages]]
id = "com.test.vpm.package1"
repository = "testowner/testrepo"

[[packages.versions]]
tag = "v1.0.0"
version = "1.0.0"
url = "https://example.com/package.zip"
hash = "abc123def456"

[packages.versions.manifest]
name = "com.test.vpm.package1"
displayName = "Test Package"
version = "1.0.0"
unity = "2022.3"
description = "A test package"
license = "MIT"
url = "https://example.com/package.zip"

[packages.versions.manifest.author]
name = "Test Author"
"#;
