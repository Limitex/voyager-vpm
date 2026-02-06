use clap::Parser;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;
use voyager::cli::{Cli, Commands, ConfigPaths};
use voyager::commands;
use voyager::context::AppContext;
use voyager::error::Error;
use voyager::infra::HttpClient;
use voyager::term;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();
    let paths = ConfigPaths::new(cli.config.clone());

    term::init(cli.quiet, cli.color);
    init_tracing(cli.verbose);

    if let Err(e) = install_rustls_provider() {
        term::error(&e);
        return e.exit_code().into();
    }

    if let Err(e) = run(cli.command, paths).await {
        term::error(&e);
        if matches!(e, Error::ManifestHashMismatch) {
            term::hint("Run 'voy lock' to validate and accept changes.");
        }
        return e.exit_code().into();
    }

    std::process::ExitCode::SUCCESS
}

async fn run(command: Commands, paths: ConfigPaths) -> Result<(), Error> {
    match command {
        Commands::Fetch(args) => {
            term::warn_if_no_github_token(args.github_token.as_deref());
            let ctx = AppContext::new(paths, args.github_token.as_deref())?;
            commands::fetch::execute(args, &ctx).await
        }
        Commands::Generate(args) => commands::generate::execute(args, &paths),
        Commands::Validate(args) => {
            let http = Arc::new(HttpClient::new()?);
            commands::validate::execute(args, http).await
        }
        Commands::Init(args) => commands::init::execute(args, &paths),
        Commands::Add(args) => {
            term::warn_if_no_github_token(args.github_token.as_deref());
            let ctx = AppContext::new(paths, args.github_token.as_deref())?;
            commands::add::execute(args, &ctx).await
        }
        Commands::Lock(args) => {
            let ctx = AppContext::new(paths, args.github_token.as_deref())?;
            commands::lock::execute(args, &ctx).await
        }
        Commands::List(args) => commands::list::execute(args, &paths),
        Commands::Remove(args) => commands::remove::execute(args, &paths),
        Commands::Info(args) => commands::info::execute(args, &paths),
        Commands::Completions(args) => {
            args.generate();
            Ok(())
        }
    }
}

fn install_rustls_provider() -> Result<(), Error> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|e| Error::RuntimeInit(format!("failed to install rustls provider: {e:?}")))
}

fn init_tracing(verbose: u8) {
    let filter = match verbose {
        0 => "voyager=warn",
        1 => "voyager=info",
        2 => "voyager=debug",
        _ => "voyager=trace",
    };

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .init();
}
