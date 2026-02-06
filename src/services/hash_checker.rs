use super::manifest_lock_tx::recover_manifest_lock_transaction;
use crate::config::Manifest;
use crate::error::{Error, Result};
use crate::lock::{Lockfile, compute_manifest_hash_from_manifest};
use std::path::Path;

pub struct HashCheckResult {
    pub manifest: Manifest,
    pub lockfile: Lockfile,
    pub current_hash: String,
}

/// Loads manifest and lockfile, checking for hash mismatch.
/// Returns error if manifest was modified outside of voyager.
pub fn check_and_load(config_path: &Path, lock_path: &Path) -> Result<HashCheckResult> {
    recover_manifest_lock_transaction(config_path, lock_path)?;
    let manifest = Manifest::load(config_path)?;
    let current_hash = compute_manifest_hash_from_manifest(&manifest, config_path)?;
    let lockfile = Lockfile::load_or_default(lock_path)?;

    if let Some(stored_hash) = &lockfile.manifest_hash
        && stored_hash != &current_hash
    {
        return Err(Error::ManifestHashMismatch);
    }

    Ok(HashCheckResult {
        manifest,
        lockfile,
        current_hash,
    })
}
