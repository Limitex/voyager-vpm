use crate::cli::{ConfigPaths, GenerateArgs};
use crate::error::{Error, Result};
use crate::infra::write_json;
use crate::services::{check_and_load, generate_from_lockfile};
use crate::term;
use tracing::info;

pub fn execute(args: GenerateArgs, paths: &ConfigPaths) -> Result<()> {
    let config_path = paths.config_path();
    let lock_path = paths.lock_path();

    let check_result = check_and_load(config_path, lock_path)?;
    let manifest = check_result.manifest;
    let lockfile = check_result.lockfile;

    if !lock_path.exists() {
        return Err(Error::ConfigValidation(format!(
            "Lock file '{}' not found. Run 'voy fetch' first.",
            lock_path.display()
        )));
    }

    if lockfile.packages.is_empty() && !manifest.packages.is_empty() {
        return Err(Error::ConfigValidation(
            "Lock file has no packages. Run 'voy fetch' first.".to_string(),
        ));
    }

    info!(
        config = %config_path.display(),
        lock = %lock_path.display(),
        output = %args.output.display(),
        packages = manifest.packages.len(),
        "Starting index generation"
    );

    let spinner = term::spinner("Generating index...");

    let output = generate_from_lockfile(&manifest, &lockfile)?;

    write_json(&args.output, &output)?;
    info!(path = %args.output.display(), "Output written successfully");

    spinner.finish_and_clear();

    term::success(format!("Generated {}", args.output.display()));

    Ok(())
}
