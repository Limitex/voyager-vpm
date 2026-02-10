use crate::cli::ColorChoice;
use console::{Emoji, style};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::fmt::Display;
use std::sync::OnceLock;
use std::time::Duration;

static EMOJI_SUCCESS: Emoji<'_, '_> = Emoji("✔ ", "+ ");
static EMOJI_WARNING: Emoji<'_, '_> = Emoji("⚠ ", "! ");
static EMOJI_ERROR: Emoji<'_, '_> = Emoji("✖ ", "x ");
static EMOJI_DONE: Emoji<'_, '_> = Emoji("✓", "+");
static EMOJI_WORKING: Emoji<'_, '_> = Emoji("⟳", ">");
static EMOJI_WAITING: Emoji<'_, '_> = Emoji("·", ".");

static QUIET_MODE: OnceLock<bool> = OnceLock::new();
static COLOR_ENABLED: OnceLock<bool> = OnceLock::new();

/// Initializes the terminal output settings.
/// Should be called once at startup with CLI args.
pub fn init(quiet: bool, color: ColorChoice) {
    QUIET_MODE.set(quiet).ok();

    let no_color = std::env::var("NO_COLOR").is_ok();
    let color_enabled = if no_color {
        // NO_COLOR standard: https://no-color.org/
        false
    } else {
        match color {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => console::colors_enabled(),
        }
    };
    COLOR_ENABLED.set(color_enabled).ok();

    if !color_enabled {
        console::set_colors_enabled(false);
        console::set_colors_enabled_stderr(false);
    }
}

fn is_quiet() -> bool {
    *QUIET_MODE.get().unwrap_or(&false)
}

/// Creates a spinner with the given message.
/// Returns a hidden spinner in quiet mode.
pub fn spinner(message: impl Into<String>) -> ProgressBar {
    if is_quiet() {
        return ProgressBar::hidden();
    }
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    spinner.set_message(message.into());
    spinner.enable_steady_tick(Duration::from_millis(80));
    spinner
}

/// Creates a progress bar with the given total count.
/// Returns a hidden progress bar in quiet mode.
pub fn progress_bar(total: u64, message: impl Into<String>) -> ProgressBar {
    if is_quiet() {
        return ProgressBar::hidden();
    }
    let bar = ProgressBar::new(total);
    bar.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.cyan} {msg} [{bar:30.cyan/dim}] {pos}/{len}")
            .unwrap()
            .progress_chars("━━─"),
    );
    bar.set_message(message.into());
    bar.enable_steady_tick(Duration::from_millis(80));
    bar
}

/// Progress display for fetch command with per-package status lines.
pub struct FetchProgress {
    multi: MultiProgress,
    main_bar: ProgressBar,
    package_bars: Vec<ProgressBar>,
}

impl FetchProgress {
    /// Creates a new fetch progress display with package IDs.
    pub fn new(package_ids: &[String]) -> Self {
        if is_quiet() {
            return Self {
                multi: MultiProgress::new(),
                main_bar: ProgressBar::hidden(),
                package_bars: package_ids.iter().map(|_| ProgressBar::hidden()).collect(),
            };
        }

        let multi = MultiProgress::new();

        let main_bar = multi.add(ProgressBar::new(package_ids.len() as u64));
        main_bar.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.cyan} [{bar:30.cyan/dim}] {pos}/{len}")
                .unwrap()
                .progress_chars("━━─"),
        );
        main_bar.enable_steady_tick(Duration::from_millis(80));

        let package_bars: Vec<_> = package_ids
            .iter()
            .map(|id| {
                let bar = multi.add(ProgressBar::new_spinner());
                bar.set_style(
                    ProgressStyle::default_spinner()
                        .template("  {msg}")
                        .unwrap(),
                );
                bar.set_message(format!("{} {}    waiting", EMOJI_WAITING, id));
                bar
            })
            .collect();

        Self {
            multi,
            main_bar,
            package_bars,
        }
    }

    /// Updates a package to "fetching releases" state.
    pub fn set_fetching_releases(&self, index: usize, package_id: &str) {
        if let Some(bar) = self.package_bars.get(index) {
            bar.set_message(format!(
                "{} {}    fetching releases...",
                EMOJI_WORKING, package_id
            ));
        }
    }

    /// Updates a package to "downloading version" state.
    pub fn set_downloading(&self, index: usize, package_id: &str, version: &str) {
        if let Some(bar) = self.package_bars.get(index) {
            bar.set_message(format!(
                "{} {}    downloading {}...",
                EMOJI_WORKING, package_id, version
            ));
        }
    }

    /// Updates a package to completed state.
    pub fn set_done(&self, index: usize, package_id: &str, existing: usize, new: usize) {
        if let Some(bar) = self.package_bars.get(index) {
            let total = existing + new;
            let msg = if new > 0 {
                format!(
                    "{} {}    {} versions (+{} new)",
                    style(EMOJI_DONE).green(),
                    package_id,
                    total,
                    new
                )
            } else {
                format!(
                    "{} {}    {} versions",
                    style(EMOJI_DONE).green(),
                    package_id,
                    total
                )
            };
            bar.set_message(msg);
        }
        self.main_bar.inc(1);
    }

    /// Finishes and clears all progress bars.
    pub fn finish(&self) {
        self.main_bar.finish_and_clear();
        for bar in &self.package_bars {
            bar.finish_and_clear();
        }
    }

    /// Returns a reference to the MultiProgress for spawning.
    pub fn multi(&self) -> &MultiProgress {
        &self.multi
    }
}

/// Prints a success message with a green checkmark.
pub fn success(message: impl Display) {
    if is_quiet() {
        return;
    }
    println!("{} {}", EMOJI_SUCCESS, style(message).green());
}

/// Prints a warning message with a yellow warning sign to stderr.
pub fn warning(message: impl Display) {
    eprintln!("{} {}", EMOJI_WARNING, style(message).yellow());
}

/// Prints an error message with a red X to stderr.
pub fn error(message: impl Display) {
    eprintln!("  {} {}", EMOJI_ERROR, style(message).red());
}

/// Prints a blank line.
pub fn blank() {
    if is_quiet() {
        return;
    }
    println!();
}

/// Prints a hint/next step message in dim style.
pub fn hint(message: impl Display) {
    if is_quiet() {
        return;
    }
    println!("  {}", style(message).dim());
}

/// Prints a secondary info line (indented, dim).
pub fn info(message: impl Display) {
    if is_quiet() {
        return;
    }
    println!("  {}", style(message).dim());
}

/// Prints a status line (indented, no styling).
pub fn status(message: impl Display) {
    if is_quiet() {
        return;
    }
    println!("  {}", message);
}

/// Returns a green styled value for inline use.
pub fn green(value: impl Display) -> impl Display {
    style(value).green()
}

/// Returns a dim styled value for inline use.
pub fn dim(value: impl Display) -> impl Display {
    style(value).dim()
}

/// Returns a red styled value for inline use.
pub fn red(value: impl Display) -> impl Display {
    style(value).red()
}

/// Returns an underlined styled value for inline use.
pub fn underlined(value: impl Display) -> impl Display {
    style(value).underlined()
}

/// Returns a bold styled value for inline use.
pub fn bold(value: impl Display) -> impl Display {
    style(value).bold()
}

/// Prints a line with the given message.
pub fn line(message: impl Display) {
    if is_quiet() {
        return;
    }
    println!("{}", message);
}

/// Prints an indented line (level * 2 spaces).
pub fn indent(level: usize, message: impl Display) {
    if is_quiet() {
        return;
    }
    let spaces = "  ".repeat(level);
    println!("{}{}", spaces, message);
}

/// Warns if GitHub token is not configured.
/// Should be called before making GitHub API requests.
pub fn warn_if_no_github_token(token: Option<&str>) {
    if token.is_none() && !is_quiet() {
        warning("VOYAGER_GITHUB_TOKEN is not set. API rate limits may apply.");
        hint("Set VOYAGER_GITHUB_TOKEN or use --github-token option.");
        blank();
    }
}
