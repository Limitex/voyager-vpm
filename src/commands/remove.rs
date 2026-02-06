use crate::cli::{ConfigPaths, RemoveArgs};
use crate::commands::package_not_found_error;
use crate::error::Result;
use crate::lock::compute_manifest_hash_from_manifest;
use crate::services::{check_and_load, save_manifest_and_lock};
use crate::term;

pub fn execute(args: RemoveArgs, paths: &ConfigPaths) -> Result<()> {
    let config_path = paths.config_path();
    let lock_path = paths.lock_path();

    let check_result = check_and_load(config_path, lock_path)?;
    let mut manifest = check_result.manifest;
    let mut lockfile = check_result.lockfile;

    let original_len = manifest.packages.len();
    manifest.packages.retain(|p| p.id != args.package_id);

    if manifest.packages.len() == original_len {
        return Err(package_not_found_error(&args.package_id, config_path));
    }

    let new_hash = compute_manifest_hash_from_manifest(&manifest, config_path)?;
    lockfile.packages.retain(|p| p.id != args.package_id);
    lockfile.manifest_hash = Some(new_hash);
    save_manifest_and_lock(&manifest, &lockfile, config_path, lock_path)?;

    term::success(format!("Removed {}", args.package_id));

    Ok(())
}
