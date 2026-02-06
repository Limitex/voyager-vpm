use crate::config::Manifest;
use crate::error::{Error, Result};
use crate::infra::{
    read_to_string_if_exists, remove_file_if_exists as fs_remove_file_if_exists, write_atomic_file,
};
use crate::lock::Lockfile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::fs;

#[derive(Debug, Serialize, Deserialize)]
struct ManifestLockTransaction {
    old_manifest: Option<String>,
    old_lock: Option<String>,
    new_manifest: String,
    new_lock: String,
}

fn transaction_path(config_path: &Path) -> PathBuf {
    config_path.with_extension("txn")
}

fn write_atomic(path: &Path, content: &str) -> Result<()> {
    write_atomic_file(path, content).map_err(|e| Error::FileWrite {
        path: path.display().to_string(),
        source: e,
    })
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    fs_remove_file_if_exists(path).map_err(|e| Error::FileWrite {
        path: path.display().to_string(),
        source: e,
    })
}

fn read_optional_file(path: &Path) -> Result<Option<String>> {
    read_to_string_if_exists(path).map_err(|e| Error::FileRead {
        path: path.display().to_string(),
        source: e,
    })
}

fn serialize_manifest(manifest: &Manifest, config_path: &Path) -> Result<String> {
    toml::to_string_pretty(manifest).map_err(|e| Error::TomlSerialize {
        path: config_path.display().to_string(),
        source: e,
    })
}

fn serialize_lock(lockfile: &Lockfile, lock_path: &Path) -> Result<String> {
    toml::to_string_pretty(lockfile).map_err(|e| Error::TomlSerialize {
        path: lock_path.display().to_string(),
        source: e,
    })
}

fn write_transaction_log(config_path: &Path, tx: &ManifestLockTransaction) -> Result<()> {
    let tx_path = transaction_path(config_path);
    let content = serde_json::to_string_pretty(tx).map_err(Error::JsonSerialize)?;
    write_atomic(&tx_path, &content)
}

fn load_transaction_log(config_path: &Path) -> Result<Option<ManifestLockTransaction>> {
    let tx_path = transaction_path(config_path);
    let Some(content) = read_optional_file(&tx_path)? else {
        return Ok(None);
    };

    let tx = serde_json::from_str(&content).map_err(|e| Error::JsonParse {
        source: tx_path.display().to_string(),
        error: e,
    })?;
    Ok(Some(tx))
}

/// Recovers an interrupted manifest+lock transaction if a transaction log exists.
///
/// - If both files already contain the new contents, the transaction is finalized
///   by deleting the log.
/// - Otherwise, files are rolled back to their previous state and the log is removed.
pub fn recover_manifest_lock_transaction(config_path: &Path, lock_path: &Path) -> Result<()> {
    let Some(tx) = load_transaction_log(config_path)? else {
        return Ok(());
    };

    let current_manifest = read_optional_file(config_path)?;
    let current_lock = read_optional_file(lock_path)?;

    let manifest_is_old = match (&current_manifest, &tx.old_manifest) {
        (None, None) => true,
        (Some(current), Some(old)) => current == old,
        _ => false,
    };
    let manifest_is_new = current_manifest.as_deref() == Some(tx.new_manifest.as_str());

    let lock_is_old = match (&current_lock, &tx.old_lock) {
        (None, None) => true,
        (Some(current), Some(old)) => current == old,
        _ => false,
    };
    let lock_is_new = current_lock.as_deref() == Some(tx.new_lock.as_str());

    if manifest_is_new && lock_is_new {
        remove_file_if_exists(&transaction_path(config_path))?;
        return Ok(());
    }

    if manifest_is_old && lock_is_old {
        remove_file_if_exists(&transaction_path(config_path))?;
        return Ok(());
    }

    // Known partial state for this write order: manifest has new content,
    // lockfile is still old (or absent if old_lock was None).
    if manifest_is_new && lock_is_old {
        match tx.old_manifest {
            Some(old_manifest) => write_atomic(config_path, &old_manifest)?,
            None => remove_file_if_exists(config_path)?,
        }
        match tx.old_lock {
            Some(old_lock) => write_atomic(lock_path, &old_lock)?,
            None => remove_file_if_exists(lock_path)?,
        }
        remove_file_if_exists(&transaction_path(config_path))?;
        return Ok(());
    }

    Err(Error::ConfigValidation(format!(
        "Found unresolved manifest/lock transaction '{}', but current files do not match \
         a recoverable state. Please inspect files and remove the transaction file manually.",
        transaction_path(config_path).display()
    )))
}

/// Saves `manifest` and `lockfile` as a crash-recoverable transaction.
///
/// A transaction log is written first. If a crash occurs mid-update, the next run
/// can recover by calling `recover_manifest_lock_transaction`.
pub fn save_manifest_and_lock(
    manifest: &Manifest,
    lockfile: &Lockfile,
    config_path: &Path,
    lock_path: &Path,
) -> Result<()> {
    recover_manifest_lock_transaction(config_path, lock_path)?;

    let old_manifest = read_optional_file(config_path)?;
    let old_lock = read_optional_file(lock_path)?;

    let tx = ManifestLockTransaction {
        old_manifest,
        old_lock,
        new_manifest: serialize_manifest(manifest, config_path)?,
        new_lock: serialize_lock(lockfile, lock_path)?,
    };

    write_transaction_log(config_path, &tx)?;

    let write_result = (|| -> Result<()> {
        write_atomic(config_path, &tx.new_manifest)?;
        write_atomic(lock_path, &tx.new_lock)?;
        Ok(())
    })();

    if let Err(e) = write_result {
        let _ = recover_manifest_lock_transaction(config_path, lock_path);
        return Err(e);
    }

    remove_file_if_exists(&transaction_path(config_path))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Manifest, Package, Vpm};
    use crate::domain::Repository;
    use tempfile::TempDir;

    fn sample_manifest(name: &str) -> Manifest {
        Manifest {
            vpm: Vpm {
                id: "com.example.vpm".to_string(),
                name: name.to_string(),
                author: "Author".to_string(),
                url: "https://example.com/index.json".to_string(),
            },
            packages: vec![Package {
                id: "com.example.vpm.pkg".to_string(),
                repository: Repository::parse("owner/repo").unwrap(),
            }],
        }
    }

    fn sample_lock(hash: &str) -> Lockfile {
        let mut lockfile = Lockfile::new();
        lockfile.manifest_hash = Some(hash.to_string());
        lockfile
    }

    #[test]
    fn saves_both_files_on_success() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("voyager.toml");
        let lock_path = dir.path().join("voyager.lock");

        let old_manifest = sample_manifest("Old");
        old_manifest.save(&config_path).unwrap();

        let new_manifest = sample_manifest("New");
        let new_lock = sample_lock("hash-new");
        save_manifest_and_lock(&new_manifest, &new_lock, &config_path, &lock_path).unwrap();

        let persisted = Manifest::load(&config_path).unwrap();
        assert_eq!(persisted.vpm.name, "New");
        let persisted_lock = Lockfile::load(&lock_path).unwrap();
        assert_eq!(persisted_lock.manifest_hash.as_deref(), Some("hash-new"));
        assert!(!transaction_path(&config_path).exists());
    }

    #[test]
    fn saves_when_files_do_not_exist_yet() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("voyager.toml");
        let lock_path = dir.path().join("voyager.lock");

        let manifest = sample_manifest("New");
        let lock = sample_lock("hash-new");
        save_manifest_and_lock(&manifest, &lock, &config_path, &lock_path).unwrap();

        let persisted = Manifest::load(&config_path).unwrap();
        assert_eq!(persisted.vpm.name, "New");
        let persisted_lock = Lockfile::load(&lock_path).unwrap();
        assert_eq!(persisted_lock.manifest_hash.as_deref(), Some("hash-new"));
        assert!(!transaction_path(&config_path).exists());
    }

    #[test]
    fn recovers_partial_write_when_original_files_did_not_exist() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("voyager.toml");
        let lock_path = dir.path().join("voyager.lock");

        let tx = ManifestLockTransaction {
            old_manifest: None,
            old_lock: None,
            new_manifest: serialize_manifest(&sample_manifest("New"), &config_path).unwrap(),
            new_lock: serialize_lock(&sample_lock("new"), &lock_path).unwrap(),
        };
        write_transaction_log(&config_path, &tx).unwrap();
        write_atomic(&config_path, &tx.new_manifest).unwrap();

        recover_manifest_lock_transaction(&config_path, &lock_path).unwrap();

        assert!(!config_path.exists());
        assert!(!lock_path.exists());
        assert!(!transaction_path(&config_path).exists());
    }

    #[test]
    fn rolls_back_when_lock_write_fails() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("voyager.toml");
        let blocker_path = dir.path().join("not_a_dir");
        let lock_path = blocker_path.join("voyager.lock");

        let old_manifest = sample_manifest("Old");
        old_manifest.save(&config_path).unwrap();
        fs::write(&blocker_path, "x").unwrap();

        let result = save_manifest_and_lock(
            &sample_manifest("New"),
            &sample_lock("hash"),
            &config_path,
            &lock_path,
        );
        assert!(result.is_err());

        let persisted = Manifest::load(&config_path).unwrap();
        assert_eq!(persisted.vpm.name, "Old");
        assert!(!transaction_path(&config_path).exists());
    }

    #[test]
    fn recovers_partial_write_by_rolling_back() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("voyager.toml");
        let lock_path = dir.path().join("voyager.lock");

        let old_manifest = sample_manifest("Old");
        old_manifest.save(&config_path).unwrap();
        let old_lock = sample_lock("old");
        old_lock.save(&lock_path).unwrap();

        let tx = ManifestLockTransaction {
            old_manifest: Some(serialize_manifest(&old_manifest, &config_path).unwrap()),
            old_lock: Some(serialize_lock(&old_lock, &lock_path).unwrap()),
            new_manifest: serialize_manifest(&sample_manifest("New"), &config_path).unwrap(),
            new_lock: serialize_lock(&sample_lock("new"), &lock_path).unwrap(),
        };
        write_transaction_log(&config_path, &tx).unwrap();
        write_atomic(&config_path, &tx.new_manifest).unwrap();

        recover_manifest_lock_transaction(&config_path, &lock_path).unwrap();

        let recovered_manifest = Manifest::load(&config_path).unwrap();
        assert_eq!(recovered_manifest.vpm.name, "Old");
        let recovered_lock = Lockfile::load(&lock_path).unwrap();
        assert_eq!(recovered_lock.manifest_hash.as_deref(), Some("old"));
        assert!(!transaction_path(&config_path).exists());
    }

    #[test]
    fn finalizes_committed_state_when_log_remains() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("voyager.toml");
        let lock_path = dir.path().join("voyager.lock");

        let old_manifest = sample_manifest("Old");
        old_manifest.save(&config_path).unwrap();

        let tx = ManifestLockTransaction {
            old_manifest: Some(serialize_manifest(&old_manifest, &config_path).unwrap()),
            old_lock: None,
            new_manifest: serialize_manifest(&sample_manifest("New"), &config_path).unwrap(),
            new_lock: serialize_lock(&sample_lock("new"), &lock_path).unwrap(),
        };
        write_transaction_log(&config_path, &tx).unwrap();
        write_atomic(&config_path, &tx.new_manifest).unwrap();
        write_atomic(&lock_path, &tx.new_lock).unwrap();

        recover_manifest_lock_transaction(&config_path, &lock_path).unwrap();

        let recovered_manifest = Manifest::load(&config_path).unwrap();
        assert_eq!(recovered_manifest.vpm.name, "New");
        let recovered_lock = Lockfile::load(&lock_path).unwrap();
        assert_eq!(recovered_lock.manifest_hash.as_deref(), Some("new"));
        assert!(!transaction_path(&config_path).exists());
    }

    #[test]
    fn finalizes_rolled_back_state_when_log_remains() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("voyager.toml");
        let lock_path = dir.path().join("voyager.lock");

        let old_manifest = sample_manifest("Old");
        old_manifest.save(&config_path).unwrap();
        let old_lock = sample_lock("old");
        old_lock.save(&lock_path).unwrap();

        let tx = ManifestLockTransaction {
            old_manifest: Some(serialize_manifest(&old_manifest, &config_path).unwrap()),
            old_lock: Some(serialize_lock(&old_lock, &lock_path).unwrap()),
            new_manifest: serialize_manifest(&sample_manifest("New"), &config_path).unwrap(),
            new_lock: serialize_lock(&sample_lock("new"), &lock_path).unwrap(),
        };
        write_transaction_log(&config_path, &tx).unwrap();

        recover_manifest_lock_transaction(&config_path, &lock_path).unwrap();

        let recovered_manifest = Manifest::load(&config_path).unwrap();
        assert_eq!(recovered_manifest.vpm.name, "Old");
        let recovered_lock = Lockfile::load(&lock_path).unwrap();
        assert_eq!(recovered_lock.manifest_hash.as_deref(), Some("old"));
        assert!(!transaction_path(&config_path).exists());
    }

    #[test]
    fn returns_error_for_ambiguous_state_without_overwriting_files() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("voyager.toml");
        let lock_path = dir.path().join("voyager.lock");

        let old_manifest = sample_manifest("Old");
        old_manifest.save(&config_path).unwrap();
        let old_lock = sample_lock("old");
        old_lock.save(&lock_path).unwrap();
        let old_lock_content = serialize_lock(&old_lock, &lock_path).unwrap();

        let tx = ManifestLockTransaction {
            old_manifest: Some(serialize_manifest(&old_manifest, &config_path).unwrap()),
            old_lock: Some(old_lock_content.clone()),
            new_manifest: serialize_manifest(&sample_manifest("New"), &config_path).unwrap(),
            new_lock: serialize_lock(&sample_lock("new"), &lock_path).unwrap(),
        };
        write_transaction_log(&config_path, &tx).unwrap();

        let user_manifest =
            serialize_manifest(&sample_manifest("UserEdited"), &config_path).unwrap();
        write_atomic(&config_path, &user_manifest).unwrap();

        let result = recover_manifest_lock_transaction(&config_path, &lock_path);
        assert!(matches!(result, Err(Error::ConfigValidation(_))));

        let persisted_manifest = fs::read_to_string(&config_path).unwrap();
        let persisted_lock = fs::read_to_string(&lock_path).unwrap();
        assert_eq!(persisted_manifest, user_manifest);
        assert_eq!(persisted_lock, old_lock_content);
        assert!(transaction_path(&config_path).exists());
    }

    #[test]
    fn save_manifest_and_lock_creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("nested/config/voyager.toml");
        let lock_path = dir.path().join("nested/config/voyager.lock");
        let manifest = sample_manifest("New");
        let lockfile = sample_lock("new");

        save_manifest_and_lock(&manifest, &lockfile, &config_path, &lock_path).unwrap();

        assert!(config_path.exists());
        assert!(lock_path.exists());
    }
}
