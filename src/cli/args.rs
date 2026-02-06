use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use std::path::{Path, PathBuf};

/// Default configuration file name.
pub const DEFAULT_CONFIG_FILE: &str = "voyager.toml";

/// Runtime configuration paths.
#[derive(Debug, Clone)]
pub struct ConfigPaths {
    /// Path to the manifest file (voyager.toml).
    config: PathBuf,
    /// Path to the lock file (voyager.lock).
    lock: PathBuf,
}

impl ConfigPaths {
    /// Create new ConfigPaths from a config file path.
    /// Lock file path is derived by changing the extension to `.lock`.
    pub fn new(config: PathBuf) -> Self {
        let lock = config.with_extension("lock");
        Self { config, lock }
    }

    /// Get the config file path.
    pub fn config_path(&self) -> &Path {
        &self.config
    }

    /// Get the lock file path.
    pub fn lock_path(&self) -> &Path {
        &self.lock
    }
}

impl Default for ConfigPaths {
    fn default() -> Self {
        Self::new(PathBuf::from(DEFAULT_CONFIG_FILE))
    }
}

fn parse_max_concurrent(s: &str) -> Result<usize, String> {
    let value: usize = parse_number(s)?;

    if value == 0 {
        return Err("max-concurrent must be at least 1".to_string());
    }

    if value > 50 {
        return Err("max-concurrent must be at most 50".to_string());
    }

    Ok(value)
}

fn parse_max_retries(s: &str) -> Result<u32, String> {
    let value: u32 = parse_number(s)?;

    if value > 8 {
        return Err("max-retries must be at most 8".to_string());
    }

    Ok(value)
}

fn parse_number<T: std::str::FromStr>(s: &str) -> Result<T, String> {
    s.parse()
        .map_err(|_| format!("'{s}' is not a valid number"))
}

#[derive(Parser, Debug)]
#[command(name = "voy", version, about = "VPM package index generator")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Path to configuration file
    #[arg(long, global = true, default_value = DEFAULT_CONFIG_FILE)]
    pub config: PathBuf,

    /// Verbosity level (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Suppress non-essential output
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Control color output
    #[arg(long, value_enum, default_value = "auto", global = true)]
    pub color: ColorChoice,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Fetch package data from GitHub releases and update voyager.lock
    Fetch(FetchArgs),

    /// Generate VPM package index from voyager.lock
    Generate(GenerateArgs),

    /// Validate URLs in an existing index file
    Validate(ValidateArgs),

    /// Initialize a new voyager.toml configuration file
    Init(InitArgs),

    /// Add a package to voyager.toml
    Add(AddArgs),

    /// Update manifest hash in voyager.lock (accept manual changes to voyager.toml)
    Lock(LockArgs),

    /// List packages, or versions of a specific package
    List(ListArgs),

    /// Remove a package from voyager.toml
    Remove(RemoveArgs),

    /// Show detailed information about a package
    Info(InfoArgs),

    /// Generate shell completions
    Completions(CompletionsArgs),
}

#[derive(Args, Debug)]
pub struct CompletionsArgs {
    /// Shell to generate completions for
    #[arg(value_enum)]
    pub shell: Shell,
}

impl CompletionsArgs {
    /// Generates and prints shell completions to stdout.
    pub fn generate(&self) {
        let mut cmd = Cli::command();
        clap_complete::generate(self.shell, &mut cmd, "voy", &mut std::io::stdout());
    }
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Package ID to show versions for (omit to list all packages)
    pub package_id: Option<String>,
}

#[derive(Args, Debug)]
pub struct RemoveArgs {
    /// Package ID to remove
    pub package_id: String,
}

#[derive(Args, Debug)]
pub struct LockArgs {
    /// Only check if manifest hash matches (don't update)
    #[arg(long)]
    pub check: bool,

    /// GitHub personal access token (for repository verification)
    #[arg(long, env = "VOYAGER_GITHUB_TOKEN")]
    pub github_token: Option<String>,
}

#[derive(Args, Debug)]
pub struct FetchArgs {
    /// GitHub personal access token
    #[arg(long, env = "VOYAGER_GITHUB_TOKEN")]
    pub github_token: Option<String>,

    /// Maximum number of concurrent downloads (1-50)
    #[arg(long, env = "VOYAGER_MAX_CONCURRENT", default_value = "5", value_parser = parse_max_concurrent)]
    pub max_concurrent: usize,

    /// Name of the asset file to download from releases
    #[arg(long, env = "VOYAGER_ASSET_NAME", default_value = "package.json")]
    pub asset_name: String,

    /// Maximum number of retries for failed downloads (0-8)
    #[arg(long, env = "VOYAGER_MAX_RETRIES", default_value = "3", value_parser = parse_max_retries)]
    pub max_retries: u32,

    /// Clear all cached versions and re-fetch everything
    #[arg(long)]
    pub wipe: bool,
}

#[derive(Args, Debug)]
pub struct GenerateArgs {
    /// Path to the output file
    #[arg(short, long, env = "VOYAGER_OUTPUT_PATH", default_value = "index.json")]
    pub output: PathBuf,
}

#[derive(Args, Debug)]
pub struct ValidateArgs {
    /// Path to the index file to validate
    pub file: PathBuf,

    /// Maximum number of concurrent URL checks (1-50)
    #[arg(long, env = "VOYAGER_MAX_CONCURRENT", default_value = "5", value_parser = parse_max_concurrent)]
    pub max_concurrent: usize,

    /// Maximum number of retries for failed URL checks (0-8)
    #[arg(long, env = "VOYAGER_MAX_RETRIES", default_value = "3", value_parser = parse_max_retries)]
    pub max_retries: u32,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    /// VPM name
    #[arg(long)]
    pub name: Option<String>,

    /// VPM ID (reverse domain notation, e.g., com.example.vpm)
    #[arg(long)]
    pub id: Option<String>,

    /// Author name
    #[arg(long)]
    pub author: Option<String>,

    /// VPM URL (where the index.json will be hosted)
    #[arg(long)]
    pub url: Option<String>,

    /// Overwrite existing file without confirmation
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// GitHub repository (owner/repo)
    pub repository: String,

    /// Package ID (optional, inferred from repository if not specified)
    #[arg(long)]
    pub id: Option<String>,

    /// GitHub personal access token (for repository verification)
    #[arg(long, env = "VOYAGER_GITHUB_TOKEN")]
    pub github_token: Option<String>,
}

#[derive(Args, Debug)]
pub struct InfoArgs {
    /// Package ID to show information for
    pub package_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_max_concurrent_accepts_valid_range() {
        assert_eq!(parse_max_concurrent("1").unwrap(), 1);
        assert_eq!(parse_max_concurrent("50").unwrap(), 50);
    }

    #[test]
    fn parse_max_concurrent_rejects_zero() {
        assert!(parse_max_concurrent("0").is_err());
    }

    #[test]
    fn parse_max_concurrent_rejects_too_large_value() {
        assert!(parse_max_concurrent("51").is_err());
    }

    #[test]
    fn parse_max_concurrent_rejects_non_numeric() {
        assert!(parse_max_concurrent("abc").is_err());
    }

    #[test]
    fn parse_max_retries_accepts_valid_range() {
        assert_eq!(parse_max_retries("0").unwrap(), 0);
        assert_eq!(parse_max_retries("8").unwrap(), 8);
    }

    #[test]
    fn parse_max_retries_rejects_too_large_value() {
        assert!(parse_max_retries("9").is_err());
    }

    #[test]
    fn parse_max_retries_rejects_non_numeric() {
        assert!(parse_max_retries("abc").is_err());
    }
}
