use crate::cli::AddArgs;
use crate::config::{Package, validation};
use crate::context::AppContext;
use crate::domain::Repository;
use crate::error::{Error, Result};
use crate::infra::GitHubApi;
use crate::lock::compute_manifest_hash_from_manifest;
use crate::services::{check_and_load, save_manifest_and_lock};
use crate::term;

pub async fn execute<G: GitHubApi>(args: AddArgs, ctx: &AppContext<G>) -> Result<()> {
    let config_path = ctx.paths.config_path();
    let lock_path = ctx.paths.lock_path();
    let repo = Repository::parse(&args.repository)
        .map_err(|e| Error::InvalidRepository(e.input().to_string()))?;

    let check_result = check_and_load(config_path, lock_path)?;
    let mut manifest = check_result.manifest;
    let mut lockfile = check_result.lockfile;

    let package_id = match args.id {
        Some(id) => {
            validation::validate_reverse_domain(&id)?;
            validation::validate_package_id_prefix(&id, &manifest.vpm.id)?;
            id
        }
        None => {
            let repo_name = repo.repo.to_lowercase().replace('-', "_");
            format!("{}.{}", manifest.vpm.id, repo_name)
        }
    };

    if manifest.packages.iter().any(|p| p.id == package_id) {
        return Err(Error::ConfigValidation(format!(
            "Package '{}' already exists in {}",
            package_id,
            config_path.display()
        )));
    }

    let spinner = term::spinner("Verifying repository...");
    let verify_result = ctx.github.verify_repository(&repo).await;
    spinner.finish_and_clear();
    verify_result?;

    manifest.packages.push(Package {
        id: package_id.clone(),
        repository: repo.clone(),
    });

    let new_hash = compute_manifest_hash_from_manifest(&manifest, config_path)?;
    lockfile.manifest_hash = Some(new_hash);
    save_manifest_and_lock(&manifest, &lockfile, config_path, lock_path)?;

    term::success(format!("Added {} ({})", package_id, repo));
    term::blank();
    term::hint("Next: voy fetch");

    Ok(())
}
