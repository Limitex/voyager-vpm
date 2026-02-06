use crate::cli::{ConfigPaths, InfoArgs};
use crate::commands::{package_not_found_error, print_no_versions_fetched_hint};
use crate::error::Result;
use crate::services::check_and_load;
use crate::term;

pub fn execute(args: InfoArgs, paths: &ConfigPaths) -> Result<()> {
    let config_path = paths.config_path();
    let lock_path = paths.lock_path();

    let check_result = check_and_load(config_path, lock_path)?;
    let manifest = check_result.manifest;
    let lockfile = check_result.lockfile;

    let package = manifest
        .packages
        .iter()
        .find(|p| p.id == args.package_id)
        .ok_or_else(|| package_not_found_error(&args.package_id, config_path))?;

    let locked_package = lockfile.get_package(&args.package_id);

    term::blank();
    term::line(format!("  {}", term::bold(&package.id)));
    term::line(format!("  {}", term::dim(&package.repository)));

    match locked_package {
        Some(pkg) if !pkg.versions.is_empty() => {
            let latest = &pkg.versions[0];

            term::blank();
            print_field("Display Name", &latest.manifest.display_name);
            print_field("Version", &format!("{} ({})", latest.version, latest.tag));
            print_field("Unity", &latest.manifest.unity);

            if !latest.manifest.description.is_empty() {
                let desc = truncate_description(&latest.manifest.description, 60);
                print_field("Description", &desc);
            }

            if !latest.manifest.author.name.is_empty() {
                print_field("Author", &latest.manifest.author.name);
            }

            if !latest.manifest.license.is_empty() {
                print_field("License", &latest.manifest.license);
            }

            if !latest.manifest.vpm_dependencies.is_empty() {
                term::blank();
                term::line(format!("  {}", term::bold("VPM Dependencies")));
                for (dep, version) in &latest.manifest.vpm_dependencies {
                    term::line(format!("    {}  {}", dep, term::dim(version)));
                }
            }

            if !latest.manifest.dependencies.is_empty() {
                term::blank();
                term::line(format!("  {}", term::bold("Unity Dependencies")));
                for (dep, version) in &latest.manifest.dependencies {
                    term::line(format!("    {}  {}", dep, term::dim(version)));
                }
            }

            if pkg.versions.len() > 1 {
                term::blank();
                term::line(format!(
                    "  {} {}",
                    term::bold("All Versions"),
                    term::dim(format!("({})", pkg.versions.len()))
                ));
                for v in &pkg.versions {
                    term::line(format!(
                        "    {}  {}",
                        term::green(&v.version),
                        term::dim(&v.tag)
                    ));
                }
            }
        }
        _ => {
            term::blank();
            print_no_versions_fetched_hint();
        }
    }

    term::blank();
    Ok(())
}

fn print_field(label: &str, value: &str) {
    term::line(format!("  {:14}  {}", term::dim(label), value));
}

fn truncate_description(desc: &str, max_len: usize) -> String {
    let first_line = desc.lines().next().unwrap_or(desc);
    if first_line.chars().count() > max_len {
        let truncated: String = first_line.chars().take(max_len).collect();
        format!("{}...", truncated)
    } else {
        first_line.to_string()
    }
}
