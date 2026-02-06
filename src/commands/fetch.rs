use crate::cli::FetchArgs;
use crate::context::AppContext;
use crate::error::Result;
use crate::infra::GitHubApi;
use crate::services::{FetchProgressReporter, FetcherConfig, PackageFetcher, check_and_load};
use crate::term;
use std::collections::HashMap;
use tracing::info;

struct TerminalFetchReporter {
    progress: term::FetchProgress,
    indices: HashMap<String, usize>,
}

impl TerminalFetchReporter {
    fn new(package_ids: &[String]) -> Self {
        let indices = package_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();
        Self {
            progress: term::FetchProgress::new(package_ids),
            indices,
        }
    }

    fn finish(&self) {
        self.progress.finish();
    }
}

impl FetchProgressReporter for TerminalFetchReporter {
    fn on_fetching_releases(&self, package_id: &str) {
        if let Some(&index) = self.indices.get(package_id) {
            self.progress.set_fetching_releases(index, package_id);
        }
    }

    fn on_downloading(&self, package_id: &str, version_count: usize) {
        if let Some(&index) = self.indices.get(package_id) {
            self.progress.set_downloading(
                index,
                package_id,
                &format!("{} versions", version_count),
            );
        }
    }

    fn on_done(&self, package_id: &str, existing: usize, new: usize) {
        if let Some(&index) = self.indices.get(package_id) {
            self.progress.set_done(index, package_id, existing, new);
        }
    }
}

pub async fn execute<G: GitHubApi>(args: FetchArgs, ctx: &AppContext<G>) -> Result<()> {
    let config_path = ctx.paths.config_path();
    let lock_path = ctx.paths.lock_path();

    let check_result = check_and_load(config_path, lock_path)?;
    let manifest = check_result.manifest;
    let mut lockfile = check_result.lockfile;
    let current_hash = check_result.current_hash;

    if args.wipe {
        info!("Wiping all cached versions");
        for pkg in &mut lockfile.packages {
            pkg.versions.clear();
        }
        term::status("Cleared all cached versions");
    }

    info!(
        config = %config_path.display(),
        lock = %lock_path.display(),
        packages = manifest.packages.len(),
        max_concurrent = args.max_concurrent,
        max_retries = args.max_retries,
        asset_name = %args.asset_name,
        "Starting fetch"
    );

    info!(
        existing_packages = lockfile.packages.len(),
        "Loaded existing lock file"
    );

    let package_ids: Vec<String> = manifest.packages.iter().map(|p| p.id.clone()).collect();
    let reporter = TerminalFetchReporter::new(&package_ids);

    let fetcher = PackageFetcher::new(
        ctx.github.clone(),
        FetcherConfig {
            max_concurrent: args.max_concurrent,
            max_retries: args.max_retries,
            asset_name: args.asset_name,
        },
    );

    let fetch_result = fetcher
        .fetch(&manifest, &mut lockfile, Some(&reporter))
        .await;
    reporter.finish();
    fetch_result?;

    lockfile.manifest_hash = Some(current_hash);
    lockfile.save(lock_path)?;
    info!(path = %lock_path.display(), "Lock file saved");

    let total_versions: usize = lockfile.packages.iter().map(|p| p.versions.len()).sum();
    term::success(format!(
        "Fetched {} package(s), {} version(s)",
        lockfile.packages.len(),
        total_versions
    ));
    term::info(format!("Saved {}", lock_path.display()));
    term::blank();
    term::hint("Next: voy generate");

    Ok(())
}
