use crate::cli::LockArgs;
use crate::config::Manifest;
use crate::context::AppContext;
use crate::error::{Error, Result};
use crate::infra::GitHubApi;
use crate::lock::{Lockfile, compute_manifest_hash};
use crate::services::recover_manifest_lock_transaction;
use crate::term;
use tracing::info;

pub async fn execute<G: GitHubApi>(args: LockArgs, ctx: &AppContext<G>) -> Result<()> {
    let config_path = ctx.paths.config_path();
    let lock_path = ctx.paths.lock_path();
    recover_manifest_lock_transaction(config_path, lock_path)?;

    if !config_path.exists() {
        return Err(Error::ConfigValidation(format!(
            "Configuration file '{}' not found. Run 'voy init' first.",
            config_path.display()
        )));
    }

    if !lock_path.exists() {
        return Err(Error::ConfigValidation(format!(
            "Lock file '{}' not found. Run 'voy fetch' first.",
            lock_path.display()
        )));
    }

    let initial_hash = compute_manifest_hash(config_path)?;
    let mut lockfile = Lockfile::load(lock_path)?;

    let is_match = lockfile
        .manifest_hash
        .as_ref()
        .is_some_and(|h| h == &initial_hash);

    if args.check {
        if is_match {
            term::success("Manifest hash matches lock file");
            Ok(())
        } else {
            term::error("Manifest hash does not match lock file");
            Err(Error::ManifestHashMismatch)
        }
    } else {
        if is_match {
            term::success("Lock file is already up to date");
            return Ok(());
        }

        let manifest = Manifest::load(config_path)?;
        verify_repositories(&manifest, ctx.github.as_ref()).await?;

        let final_hash = compute_manifest_hash(config_path)?;
        if final_hash != initial_hash {
            return Err(Error::ManifestHashMismatch);
        }

        lockfile.manifest_hash = Some(final_hash);
        lockfile.save(lock_path)?;
        info!(path = %lock_path.display(), "Lock file updated");
        term::success("Updated manifest hash in lock file");

        Ok(())
    }
}

async fn verify_repositories<G: GitHubApi>(manifest: &Manifest, github: &G) -> Result<()> {
    if manifest.packages.is_empty() {
        return Ok(());
    }

    let spinner = term::spinner("Verifying repositories...");
    let verify_result = async {
        for package in &manifest.packages {
            github.verify_repository(&package.repository).await?;
        }
        Ok(())
    }
    .await;
    spinner.finish_and_clear();
    verify_result
}
