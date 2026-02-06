use crate::cli::{ConfigPaths, ListArgs};
use crate::commands::{package_not_found_error, print_no_versions_fetched_hint};
use crate::config::Manifest;
use crate::error::Result;
use crate::lock::Lockfile;
use crate::services::check_and_load;
use crate::term;

pub fn execute(args: ListArgs, paths: &ConfigPaths) -> Result<()> {
    let config_path = paths.config_path();
    let lock_path = paths.lock_path();

    let check_result = check_and_load(config_path, lock_path)?;
    let manifest = check_result.manifest;
    let lockfile = check_result.lockfile;

    match args.package_id {
        Some(package_id) => list_versions(&manifest, &lockfile, &package_id, paths),
        None => list_packages(&manifest, &lockfile),
    }
}

fn list_packages(manifest: &Manifest, lockfile: &Lockfile) -> Result<()> {
    if manifest.packages.is_empty() {
        term::status("No packages configured.");
        term::hint("Run 'voy add <owner/repo>' to add a package.");
        return Ok(());
    }

    let max_id_len = manifest
        .packages
        .iter()
        .map(|p| p.id.len())
        .max()
        .unwrap_or(7)
        .max(7);
    let max_repo_len = manifest
        .packages
        .iter()
        .map(|p| p.repository.to_string().len())
        .max()
        .unwrap_or(10)
        .max(10);

    term::line(format!(
        "  {:max_id_len$}  {:max_repo_len$}  Versions",
        "Package", "Repository",
    ));

    for package in &manifest.packages {
        let version_count = lockfile
            .get_package(&package.id)
            .map(|p| p.versions.len())
            .unwrap_or(0);

        let repo_str = package.repository.to_string();
        let id_padded = format!("{:max_id_len$}", package.id);
        let repo_padded = format!("{:max_repo_len$}", repo_str);

        if version_count > 0 {
            term::line(format!(
                "  {}  {}  {}",
                term::green(&id_padded),
                term::dim(&repo_padded),
                version_count,
            ));
        } else {
            term::line(format!(
                "  {}  {}  {}",
                id_padded,
                term::dim(&repo_padded),
                term::dim("-"),
            ));
        }
    }

    Ok(())
}

fn list_versions(
    manifest: &Manifest,
    lockfile: &Lockfile,
    package_id: &str,
    paths: &ConfigPaths,
) -> Result<()> {
    let package = manifest
        .packages
        .iter()
        .find(|p| p.id == package_id)
        .ok_or_else(|| package_not_found_error(package_id, paths.config_path()))?;

    let locked_package = lockfile.get_package(package_id);

    term::line(format!(
        "  {} {}",
        term::bold(&package.id),
        term::dim(format!("({})", package.repository))
    ));

    match locked_package {
        Some(pkg) if !pkg.versions.is_empty() => {
            term::blank();
            for version in &pkg.versions {
                term::line(format!(
                    "    {}  {}",
                    term::green(&version.version),
                    term::dim(&version.tag)
                ));
            }
        }
        _ => {
            term::blank();
            print_no_versions_fetched_hint();
        }
    }

    Ok(())
}
