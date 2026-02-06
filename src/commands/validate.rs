use crate::cli::ValidateArgs;
use crate::error::{Error, Result};
use crate::infra::{HttpApi, read_json};
use crate::output::VpmOutput;
use crate::services::UrlValidator;
use crate::term;
use std::sync::Arc;
use tracing::info;

pub async fn execute<H: HttpApi>(args: ValidateArgs, http: Arc<H>) -> Result<()> {
    info!(
        file = %args.file.display(),
        max_concurrent = args.max_concurrent,
        "Starting URL validation"
    );

    let output: VpmOutput = read_json(&args.file)?;

    info!(packages = output.packages.len(), "Loaded index file");

    let spinner = term::spinner("Validating URLs...");

    let validator = UrlValidator::new(http, args.max_concurrent, args.max_retries);
    let result = validator.validate(&output).await?;
    spinner.finish_and_clear();

    if result.invalid.is_empty() {
        term::success(format!("Checked {} URL(s): all valid", result.total));
    } else {
        term::status(format!(
            "Checked {} URL(s): {} valid, {} invalid",
            result.total,
            term::green(result.valid),
            term::red(result.invalid.len())
        ));
    }

    if !result.invalid.is_empty() {
        term::blank();
        for invalid in &result.invalid {
            term::error(format!(
                "{} {}: {}",
                term::red(&invalid.package_id),
                term::dim(format!("v{}", invalid.version)),
                term::underlined(&invalid.url)
            ));
        }
        return Err(Error::UrlValidation {
            count: result.invalid.len(),
        });
    }

    info!("Validation completed successfully");

    Ok(())
}
