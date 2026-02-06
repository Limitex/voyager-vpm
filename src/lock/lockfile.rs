use super::package_manifest::PackageManifest;
use crate::config::Manifest;
use crate::domain::Repository;
use crate::error::{Error, Result};
use crate::infra::write_atomic_file;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Current lockfile version that will be written.
const LOCKFILE_VERSION: u32 = 1;

/// Minimum supported lockfile version for reading.
const MIN_SUPPORTED_VERSION: u32 = 1;

/// Maximum supported lockfile version for reading.
const MAX_SUPPORTED_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lockfile {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_hash: Option<String>,
    #[serde(default)]
    pub packages: Vec<LockedPackage>,
}

impl Default for Lockfile {
    fn default() -> Self {
        Self::new()
    }
}

impl Lockfile {
    pub fn new() -> Self {
        Self {
            version: LOCKFILE_VERSION,
            manifest_hash: None,
            packages: Vec::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path).map_err(|e| Error::FileRead {
            path: path.display().to_string(),
            source: e,
        })?;

        let mut lockfile: Self = toml::from_str(&content).map_err(|e| Error::TomlParse {
            path: path.display().to_string(),
            source: e,
        })?;

        if lockfile.version < MIN_SUPPORTED_VERSION {
            return Err(Error::ConfigValidation(format!(
                "Lockfile version {} is too old (minimum supported: {}). \
                 Please delete the lockfile and run 'voy fetch' again.",
                lockfile.version, MIN_SUPPORTED_VERSION
            )));
        }

        if lockfile.version > MAX_SUPPORTED_VERSION {
            return Err(Error::ConfigValidation(format!(
                "Lockfile version {} is newer than supported (maximum: {}). \
                 Please upgrade voyager to read this lockfile.",
                lockfile.version, MAX_SUPPORTED_VERSION
            )));
        }

        lockfile = Self::migrate(lockfile)?;

        Ok(lockfile)
    }

    /// Migrates a lockfile from an older version to the current version.
    fn migrate(mut lockfile: Self) -> Result<Self> {
        lockfile.version = LOCKFILE_VERSION;
        Ok(lockfile)
    }

    pub fn load_or_default(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::new())
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).map_err(|e| Error::TomlSerialize {
            path: path.display().to_string(),
            source: e,
        })?;

        write_atomic_file(path, &content).map_err(|e| Error::FileWrite {
            path: path.display().to_string(),
            source: e,
        })
    }

    pub fn get_package(&self, id: &str) -> Option<&LockedPackage> {
        self.packages.iter().find(|p| p.id == id)
    }

    pub fn get_package_mut(&mut self, id: &str) -> Option<&mut LockedPackage> {
        self.packages.iter_mut().find(|p| p.id == id)
    }

    pub fn get_or_insert_package(
        &mut self,
        id: &str,
        repository: &Repository,
    ) -> &mut LockedPackage {
        if let Some(pos) = self.packages.iter().position(|p| p.id == id) {
            &mut self.packages[pos]
        } else {
            self.packages.push(LockedPackage {
                id: id.to_string(),
                repository: repository.clone(),
                versions: Vec::new(),
            });
            self.packages.last_mut().unwrap()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedPackage {
    pub id: String,
    pub repository: Repository,
    #[serde(default)]
    pub versions: Vec<LockedVersion>,
}

impl LockedPackage {
    pub fn existing_versions(&self) -> HashSet<String> {
        self.versions.iter().map(|v| v.version.clone()).collect()
    }

    pub fn get_version(&self, version: &str) -> Option<&LockedVersion> {
        self.versions.iter().find(|v| v.version == version)
    }

    pub fn add_version(&mut self, version: LockedVersion) {
        if self.get_version(&version.version).is_none() {
            self.versions.push(version);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedVersion {
    pub version: String,
    pub tag: String,
    pub url: String,
    pub hash: String,
    pub manifest: PackageManifest,
}

impl LockedVersion {
    pub fn new(tag: String, url: String, raw_content: &str, manifest: PackageManifest) -> Self {
        let hash = compute_hash(raw_content);
        Self {
            version: manifest.version.clone(),
            tag,
            url,
            hash,
            manifest,
        }
    }
}

pub fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    format!("sha256:{:x}", result)
}

/// Computes a hash of the manifest file by normalizing it first.
/// This ensures that whitespace/comment changes don't affect the hash.
pub fn compute_manifest_hash(path: &Path) -> Result<String> {
    let content = fs::read_to_string(path).map_err(|e| Error::FileRead {
        path: path.display().to_string(),
        source: e,
    })?;

    let manifest: Manifest = toml::from_str(&content).map_err(|e| Error::TomlParse {
        path: path.display().to_string(),
        source: e,
    })?;

    compute_manifest_hash_from_manifest(&manifest, path)
}

/// Computes a hash from an in-memory Manifest.
/// Use this when you have already loaded the manifest and want to avoid re-reading the file.
pub fn compute_manifest_hash_from_manifest(manifest: &Manifest, path: &Path) -> Result<String> {
    let normalized = toml::to_string(manifest).map_err(|e| Error::TomlSerialize {
        path: path.display().to_string(),
        source: e,
    })?;

    Ok(compute_hash(&normalized))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::lock::PackageAuthor;
    use indexmap::IndexMap;
    use tempfile::TempDir;

    fn repo(s: &str) -> Repository {
        Repository::parse(s).unwrap()
    }

    fn create_test_version_output() -> PackageManifest {
        PackageManifest {
            name: "com.example.test".to_string(),
            version: "1.0.0".to_string(),
            display_name: "Test Package".to_string(),
            description: "A test package".to_string(),
            unity: "2022.3".to_string(),
            unity_release: String::new(),
            dependencies: IndexMap::new(),
            keywords: vec![],
            author: PackageAuthor {
                name: "Test Author".to_string(),
                email: String::new(),
                url: String::new(),
            },
            vpm_dependencies: IndexMap::new(),
            url: "https://example.com/test.zip".to_string(),
            license: String::new(),
        }
    }

    #[test]
    fn compute_hash_is_deterministic() {
        let content = r#"{"name": "test"}"#;
        let hash1 = compute_hash(content);
        let hash2 = compute_hash(content);
        assert_eq!(hash1, hash2);
        assert!(hash1.starts_with("sha256:"));
    }

    #[test]
    fn compute_hash_differs_for_different_content() {
        let hash1 = compute_hash("content1");
        let hash2 = compute_hash("content2");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn lockfile_new_creates_empty() {
        let lockfile = Lockfile::new();
        assert_eq!(lockfile.version, LOCKFILE_VERSION);
        assert!(lockfile.packages.is_empty());
    }

    #[test]
    fn lockfile_save_and_load_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.lock");

        let mut lockfile = Lockfile::new();
        let pkg = lockfile.get_or_insert_package("com.example.test", &repo("owner/repo"));
        pkg.add_version(LockedVersion::new(
            "v1.0.0".to_string(),
            "https://example.com/v1.0.0/package.json".to_string(),
            r#"{"name": "test"}"#,
            create_test_version_output(),
        ));

        lockfile.save(&path).unwrap();

        let loaded = Lockfile::load(&path).unwrap();
        assert_eq!(loaded.version, lockfile.version);
        assert_eq!(loaded.packages.len(), 1);
        assert_eq!(loaded.packages[0].id, "com.example.test");
        assert_eq!(loaded.packages[0].versions.len(), 1);
    }

    #[test]
    fn lockfile_load_or_default_returns_default_if_not_exists() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.lock");

        let lockfile = Lockfile::load_or_default(&path).unwrap();
        assert_eq!(lockfile.version, LOCKFILE_VERSION);
        assert!(lockfile.packages.is_empty());
    }

    #[test]
    fn lockfile_load_rejects_unknown_version() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("invalid.lock");

        let content = r#"
version = 99

[[packages]]
id = "com.example.test"
repository = "owner/repo"
"#;
        fs::write(&path, content).unwrap();

        let result = Lockfile::load(&path);
        assert!(matches!(result, Err(Error::ConfigValidation(_))));
    }

    #[test]
    fn locked_package_existing_versions() {
        let pkg = LockedPackage {
            id: "test".to_string(),
            repository: repo("owner/repo"),
            versions: vec![
                LockedVersion::new("v1.0.0".to_string(), "url1".to_string(), "content1", {
                    let mut v = create_test_version_output();
                    v.version = "1.0.0".to_string();
                    v
                }),
                LockedVersion::new("v2.0.0".to_string(), "url2".to_string(), "content2", {
                    let mut v = create_test_version_output();
                    v.version = "2.0.0".to_string();
                    v
                }),
            ],
        };

        let existing = pkg.existing_versions();
        assert!(existing.contains("1.0.0"));
        assert!(existing.contains("2.0.0"));
        assert!(!existing.contains("3.0.0"));
    }

    #[test]
    fn locked_package_add_version_prevents_duplicates() {
        let mut pkg = LockedPackage {
            id: "test".to_string(),
            repository: repo("owner/repo"),
            versions: vec![],
        };

        let version = LockedVersion::new(
            "v1.0.0".to_string(),
            "url".to_string(),
            "content",
            create_test_version_output(),
        );

        pkg.add_version(version.clone());
        pkg.add_version(version);

        assert_eq!(pkg.versions.len(), 1);
    }

    #[test]
    fn get_or_insert_package_creates_new() {
        let mut lockfile = Lockfile::new();
        let pkg = lockfile.get_or_insert_package("com.example.new", &repo("owner/new"));

        assert_eq!(pkg.id, "com.example.new");
        assert_eq!(pkg.repository, repo("owner/new"));
        assert_eq!(lockfile.packages.len(), 1);
    }

    #[test]
    fn get_or_insert_package_returns_existing() {
        let mut lockfile = Lockfile::new();
        lockfile.get_or_insert_package("com.example.test", &repo("owner/repo"));
        lockfile.get_or_insert_package("com.example.test", &repo("owner/repo"));

        assert_eq!(lockfile.packages.len(), 1);
    }

    #[test]
    fn lockfile_load_rejects_too_old_version() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("old.lock");

        let content = r#"
version = 0

[[packages]]
id = "com.example.test"
repository = "owner/repo"
"#;
        fs::write(&path, content).unwrap();

        let result = Lockfile::load(&path);
        assert!(matches!(result, Err(Error::ConfigValidation(msg)) if msg.contains("too old")));
    }

    #[test]
    fn lockfile_load_rejects_too_new_version() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("new.lock");

        let content = r#"
version = 99

[[packages]]
id = "com.example.test"
repository = "owner/repo"
"#;
        fs::write(&path, content).unwrap();

        let result = Lockfile::load(&path);
        assert!(
            matches!(result, Err(Error::ConfigValidation(msg)) if msg.contains("newer than supported"))
        );
    }

    #[test]
    fn lockfile_save_creates_parent_directories() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nested/dir/voyager.lock");
        let lockfile = Lockfile::new();

        lockfile.save(&path).unwrap();
        assert!(path.exists());
    }
}
