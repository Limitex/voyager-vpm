use thiserror::Error;

/// Exit codes following sysexits.h conventions where applicable.
/// See: https://man.freebsd.org/cgi/man.cgi?query=sysexits
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitCode(pub i32);

impl ExitCode {
    /// General error
    pub const FAILURE: Self = Self(1);
    /// Configuration error (invalid config file, validation failed)
    pub const CONFIG: Self = Self(78); // EX_CONFIG
    /// I/O error (file read/write failed)
    pub const IO: Self = Self(74); // EX_IOERR
    /// Network/service unavailable
    pub const UNAVAILABLE: Self = Self(69); // EX_UNAVAILABLE
    /// Data format error (JSON/TOML parse error)
    pub const DATA: Self = Self(65); // EX_DATAERR
}

impl From<ExitCode> for std::process::ExitCode {
    fn from(code: ExitCode) -> Self {
        std::process::ExitCode::from(code.0 as u8)
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to read file '{path}': {source}")]
    FileRead {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to write file '{path}': {source}")]
    FileWrite {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse TOML '{path}': {source}")]
    TomlParse {
        path: String,
        #[source]
        source: toml::de::Error,
    },

    #[error("Failed to serialize TOML '{path}': {source}")]
    TomlSerialize {
        path: String,
        #[source]
        source: toml::ser::Error,
    },

    #[error("Config validation failed: {0}")]
    ConfigValidation(String),

    #[error("Invalid repository format '{0}', expected 'owner/repo'")]
    InvalidRepository(String),

    #[error(
        "Invalid package ID '{0}': must be in reverse domain notation (e.g., 'com.example.package')"
    )]
    InvalidPackageId(String),

    #[error("Invalid URL '{0}': {1}")]
    InvalidUrl(String, String),

    #[error("GitHub API error: {message}: {source}")]
    GitHub {
        message: String,
        #[source]
        source: octocrab::Error,
    },

    #[error("HTTP request failed for '{url}': {source}")]
    Http {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("Failed to parse JSON from '{source}': {error}")]
    JsonParse {
        source: String,
        #[source]
        error: serde_json::Error,
    },

    #[error("Failed to serialize JSON: {0}")]
    JsonSerialize(#[source] serde_json::Error),

    #[error("package.json not found in release '{tag}'")]
    PackageJsonNotFound { tag: String },

    #[error("Failed to write output to '{path}': {source}")]
    OutputWrite {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("URL validation failed: {count} URL(s) are not accessible")]
    UrlValidation { count: usize },

    #[error("Repository '{0}' not found on GitHub")]
    RepositoryNotFound(String),

    #[error("Fetch completed with {count} failed release(s); lockfile was not updated")]
    FetchPartialFailure { count: usize },

    #[error("Manifest has been modified outside of voyager")]
    ManifestHashMismatch,

    #[error("Runtime initialization failed: {0}")]
    RuntimeInit(String),
}

impl Error {
    /// Returns the appropriate exit code for this error type.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            // I/O errors
            Error::Io(_)
            | Error::FileRead { .. }
            | Error::FileWrite { .. }
            | Error::OutputWrite { .. } => ExitCode::IO,
            // Data format errors
            Error::TomlParse { .. }
            | Error::TomlSerialize { .. }
            | Error::JsonParse { .. }
            | Error::JsonSerialize(_) => ExitCode::DATA,
            // Configuration/validation errors
            Error::ConfigValidation(_)
            | Error::InvalidRepository(_)
            | Error::InvalidPackageId(_)
            | Error::InvalidUrl(_, _)
            | Error::ManifestHashMismatch => ExitCode::CONFIG,
            // Network/service errors
            Error::GitHub { .. }
            | Error::Http { .. }
            | Error::RepositoryNotFound(_)
            | Error::UrlValidation { .. }
            | Error::FetchPartialFailure { .. } => ExitCode::UNAVAILABLE,
            // Other errors
            Error::PackageJsonNotFound { .. } | Error::RuntimeInit(_) => ExitCode::FAILURE,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
