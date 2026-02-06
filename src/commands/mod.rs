pub mod add;
pub mod fetch;
pub mod generate;
pub mod info;
pub mod init;
pub mod list;
pub mod lock;
pub mod remove;
pub mod validate;

use crate::error::Error;
use crate::term;
use std::path::Path;

pub(crate) fn package_not_found_error(package_id: &str, config_path: &Path) -> Error {
    Error::ConfigValidation(format!(
        "Package '{}' not found in {}",
        package_id,
        config_path.display()
    ))
}

pub(crate) fn print_no_versions_fetched_hint() {
    term::info("No versions fetched yet. Run 'voy fetch' first.");
}
