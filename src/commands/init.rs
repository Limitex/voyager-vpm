use crate::cli::{ConfigPaths, InitArgs};
use crate::config::{Manifest, Vpm, validation};
use crate::error::{Error, Result};
use crate::lock::{Lockfile, compute_manifest_hash_from_manifest};
use crate::services::save_manifest_and_lock;
use crate::term;

pub fn execute(args: InitArgs, paths: &ConfigPaths) -> Result<()> {
    let output_path = paths.config_path();
    let lock_path = paths.lock_path();

    cliclack::intro("voy init")?;

    if output_path.exists() && !args.force {
        let overwrite = cliclack::confirm(format!(
            "{} already exists. Overwrite?",
            output_path.display()
        ))
        .initial_value(false)
        .interact()?;

        if !overwrite {
            cliclack::outro_cancel("Aborted.")?;
            return Ok(());
        }
    }

    let name: String = match args.name {
        Some(n) => n,
        None => cliclack::input("VPM name")
            .placeholder("My Awesome VPM")
            .interact()?,
    };

    let id: String = match args.id {
        Some(i) => {
            validation::validate_reverse_domain(&i)?;
            i
        }
        None => cliclack::input("VPM ID")
            .placeholder("com.example.vpm")
            .validate(|input: &String| {
                validation::validate_reverse_domain(input)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            })
            .interact()?,
    };

    let author: String = match args.author {
        Some(a) => a,
        None => cliclack::input("Author name").interact()?,
    };

    let url: String = match args.url {
        Some(u) => {
            validation::validate_url(&u)?;
            u
        }
        None => cliclack::input("VPM URL")
            .placeholder("https://example.github.io/repo/index.json")
            .validate(|input: &String| {
                validation::validate_url(input)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            })
            .interact()?,
    };

    let manifest = Manifest::new(Vpm {
        id,
        name,
        author,
        url,
    });

    if args.force {
        let tx_path = output_path.with_extension("txn");
        if tx_path.exists() {
            std::fs::remove_file(&tx_path).map_err(|e| Error::FileWrite {
                path: tx_path.display().to_string(),
                source: e,
            })?;
            term::status(format!(
                "Removed stale transaction log {}",
                tx_path.display()
            ));
        }
    }

    let hash = compute_manifest_hash_from_manifest(&manifest, output_path)?;
    let mut lockfile = Lockfile::new();
    lockfile.manifest_hash = Some(hash);
    save_manifest_and_lock(&manifest, &lockfile, output_path, lock_path)?;

    cliclack::outro(format!("Created {}", output_path.display()))?;

    term::blank();
    term::hint("Next: voy add <owner/repo>");

    Ok(())
}
