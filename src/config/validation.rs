use crate::error::{Error, Result};
use reqwest::Url;
use semver::{Version, VersionReq};

/// Validates that a string is in reverse domain notation.
///
/// Valid examples: "com.example.package", "org.test-project.utils"
/// Invalid examples: "invalid", "com.", ".example"
///
/// Rules:
/// - Must have at least 2 parts separated by dots
/// - Each part must be non-empty
/// - Each part can only contain lowercase alphanumeric characters, hyphens, or underscores
pub fn validate_reverse_domain(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(Error::InvalidPackageId(id.to_string()));
    }

    let parts: Vec<&str> = id.split('.').collect();
    if parts.len() < 2 {
        return Err(Error::InvalidPackageId(id.to_string()));
    }

    for part in parts {
        if part.is_empty() {
            return Err(Error::InvalidPackageId(id.to_string()));
        }
        if !part
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(Error::InvalidPackageId(id.to_string()));
        }
    }

    Ok(())
}

/// Validates that a package ID starts with the expected VPM ID prefix.
///
/// Valid example: package_id="com.example.package", vpm_id="com.example"
/// Invalid example: package_id="org.other.package", vpm_id="com.example"
pub fn validate_package_id_prefix(package_id: &str, vpm_id: &str) -> Result<()> {
    let expected_prefix = format!("{}.", vpm_id);
    if !package_id.starts_with(&expected_prefix) {
        return Err(Error::InvalidPackageId(format!(
            "'{}' must start with VPM ID prefix '{}'",
            package_id, vpm_id
        )));
    }
    Ok(())
}

/// Validates that a URL has a valid format.
///
/// Valid examples: "http://example.com", "https://example.com/path"
/// Invalid examples: "", "example.com", "ftp://example.com"
///
/// Rules:
/// - Must not be empty
/// - Must start with http:// or https://
pub fn validate_url(url: &str) -> Result<()> {
    if url.is_empty() {
        return Err(Error::InvalidUrl(
            url.to_string(),
            "URL is empty".to_string(),
        ));
    }

    let parsed = Url::parse(url)
        .map_err(|e| Error::InvalidUrl(url.to_string(), format!("Invalid URL format: {e}")))?;

    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(Error::InvalidUrl(
            url.to_string(),
            "URL must start with http:// or https://".to_string(),
        ));
    }

    if parsed.host_str().is_none() {
        return Err(Error::InvalidUrl(
            url.to_string(),
            "URL must include a host".to_string(),
        ));
    }

    Ok(())
}

/// Validates that a URL points to a ZIP archive.
pub fn validate_zip_url(url: &str) -> Result<()> {
    validate_url(url)?;
    let parsed = Url::parse(url)
        .map_err(|e| Error::InvalidUrl(url.to_string(), format!("Invalid URL format: {e}")))?;

    let path = parsed.path();
    let file_name = path.rsplit('/').next().unwrap_or_default();
    let has_extension = file_name.contains('.');

    // Some signed download URLs are extensionless. Reject only when a non-zip
    // extension is explicitly present (e.g. `.json`).
    if has_extension && !file_name.to_ascii_lowercase().ends_with(".zip") {
        return Err(Error::InvalidUrl(
            url.to_string(),
            "URL must point to a .zip file".to_string(),
        ));
    }

    Ok(())
}

/// Validates that a Unity version string is in `MAJOR.MINOR` format.
///
/// Valid examples: "2022.3", "2019.1", "6000.0"
/// Invalid examples: "2022", "2022.3.1", "hello", "2022.x"
pub fn validate_unity_version(version: &str) -> Result<()> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 2 {
        return Err(Error::ConfigValidation(format!(
            "Unity version '{version}' must be in MAJOR.MINOR format (e.g. \"2022.3\")"
        )));
    }
    for part in parts {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return Err(Error::ConfigValidation(format!(
                "Unity version '{version}' must be in MAJOR.MINOR format (e.g. \"2022.3\")"
            )));
        }
    }
    Ok(())
}

/// Validates a Unity release suffix in `<UPDATE><RELEASE>` format.
///
/// Valid examples: "0b4", "22f1"
/// Invalid examples: "b4", "0beta4", "0b", "0B4"
pub fn validate_unity_release(release: &str) -> Result<()> {
    let err = || {
        Error::ConfigValidation(format!(
            "Unity release '{release}' must be in <UPDATE><RELEASE> format (e.g. \"0b4\", \"22f1\")"
        ))
    };

    let chars: Vec<char> = release.chars().collect();
    if chars.is_empty() {
        return Err(err());
    }

    let mut idx = 0usize;
    while idx < chars.len() && chars[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == 0 || idx >= chars.len() {
        return Err(err());
    }

    let channel = chars[idx];
    if !channel.is_ascii_lowercase() {
        return Err(err());
    }
    idx += 1;

    if idx >= chars.len() || !chars[idx..].iter().all(|c| c.is_ascii_digit()) {
        return Err(err());
    }

    Ok(())
}

/// Unity `dependencies` versions must be exact SemVer versions (no ranges).
pub fn validate_unity_dependency_version(version: &str) -> Result<()> {
    if Version::parse(version).is_err() {
        return Err(Error::ConfigValidation(format!(
            "Unity dependency version '{version}' must be a valid SemVer version"
        )));
    }
    Ok(())
}

/// VPM `vpmDependencies` values support semver range expressions.
pub fn validate_vpm_dependency_range(range: &str) -> Result<()> {
    let trimmed = range.trim();
    if trimmed.is_empty() {
        return Err(Error::ConfigValidation(
            "VPM dependency range must not be empty".to_string(),
        ));
    }

    for clause in trimmed.split("||").map(str::trim) {
        if clause.is_empty() {
            return Err(Error::ConfigValidation(format!(
                "VPM dependency range '{range}' contains an empty OR clause"
            )));
        }

        if is_valid_hyphen_range(clause) {
            continue;
        }

        let normalized = normalize_vpm_clause(clause);
        if VersionReq::parse(&normalized).is_ok() {
            continue;
        }

        // `semver::VersionReq` does not accept space-separated AND clauses.
        // Convert `>=1.0.0 <2.0.0` to `>=1.0.0, <2.0.0`.
        let comma_joined = normalized.split_whitespace().collect::<Vec<_>>().join(", ");
        if !comma_joined.is_empty() && VersionReq::parse(&comma_joined).is_ok() {
            continue;
        }

        return Err(Error::ConfigValidation(format!(
            "VPM dependency range '{range}' is invalid"
        )));
    }

    Ok(())
}

fn is_valid_hyphen_range(clause: &str) -> bool {
    let Some((left, right)) = clause.split_once(" - ") else {
        return false;
    };

    let left = normalize_vpm_version_token(left.trim());
    let right = normalize_vpm_version_token(right.trim());
    if left.is_empty() || right.is_empty() {
        return false;
    }

    VersionReq::parse(&format!(">={left}, <={right}")).is_ok()
}

fn normalize_vpm_clause(clause: &str) -> String {
    clause
        .split(',')
        .map(|segment| {
            segment
                .split_whitespace()
                .map(normalize_comparator_token)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn normalize_comparator_token(token: &str) -> String {
    let split_index = token
        .char_indices()
        .find_map(|(index, ch)| {
            if matches!(ch, '<' | '>' | '=' | '~' | '^') {
                None
            } else {
                Some(index)
            }
        })
        .unwrap_or(token.len());
    let (operator, version) = token.split_at(split_index);
    if version.is_empty() {
        return token.to_string();
    }

    format!("{operator}{}", normalize_vpm_version_token(version))
}

fn normalize_vpm_version_token(token: &str) -> String {
    token
        .split('.')
        .map(|part| {
            if part.eq_ignore_ascii_case("x") {
                "*".to_string()
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    mod reverse_domain {
        use super::*;

        #[test]
        fn valid_two_parts() {
            assert!(validate_reverse_domain("com.example").is_ok());
        }

        #[test]
        fn valid_three_parts() {
            assert!(validate_reverse_domain("com.example.package").is_ok());
        }

        #[test]
        fn valid_with_hyphen() {
            assert!(validate_reverse_domain("com.my-org.my-package").is_ok());
        }

        #[test]
        fn valid_with_underscore() {
            assert!(validate_reverse_domain("com.my_org.my_package").is_ok());
        }

        #[test]
        fn valid_with_numbers() {
            assert!(validate_reverse_domain("com.example123.pkg456").is_ok());
        }

        #[test]
        fn invalid_empty() {
            assert!(validate_reverse_domain("").is_err());
        }

        #[test]
        fn invalid_single_part() {
            assert!(validate_reverse_domain("example").is_err());
        }

        #[test]
        fn invalid_empty_part() {
            assert!(validate_reverse_domain("com..example").is_err());
        }

        #[test]
        fn invalid_leading_dot() {
            assert!(validate_reverse_domain(".com.example").is_err());
        }

        #[test]
        fn invalid_trailing_dot() {
            assert!(validate_reverse_domain("com.example.").is_err());
        }

        #[test]
        fn invalid_special_chars() {
            assert!(validate_reverse_domain("com.example@test").is_err());
        }

        #[test]
        fn invalid_uppercase() {
            assert!(validate_reverse_domain("com.Example.package").is_err());
        }
    }

    mod package_id_prefix {
        use super::*;

        #[test]
        fn valid_prefix() {
            assert!(validate_package_id_prefix("com.example.package", "com.example").is_ok());
        }

        #[test]
        fn valid_nested_prefix() {
            assert!(validate_package_id_prefix("com.example.sub.package", "com.example").is_ok());
        }

        #[test]
        fn invalid_different_prefix() {
            assert!(validate_package_id_prefix("org.other.package", "com.example").is_err());
        }

        #[test]
        fn invalid_partial_match() {
            assert!(validate_package_id_prefix("com.exampleother.package", "com.example").is_err());
        }
    }

    mod url_validation {
        use super::*;

        #[test]
        fn valid_https() {
            assert!(validate_url("https://example.com").is_ok());
        }

        #[test]
        fn valid_http() {
            assert!(validate_url("http://example.com").is_ok());
        }

        #[test]
        fn valid_with_path() {
            assert!(validate_url("https://example.com/path/to/file").is_ok());
        }

        #[test]
        fn valid_with_query() {
            assert!(validate_url("https://example.com?foo=bar").is_ok());
        }

        #[test]
        fn invalid_empty() {
            assert!(validate_url("").is_err());
        }

        #[test]
        fn invalid_no_scheme() {
            assert!(validate_url("example.com").is_err());
        }

        #[test]
        fn invalid_ftp_scheme() {
            assert!(validate_url("ftp://example.com").is_err());
        }

        #[test]
        fn invalid_missing_host() {
            assert!(validate_url("http://").is_err());
        }

        #[test]
        fn invalid_malformed_url() {
            assert!(validate_url("https://").is_err());
        }
    }

    mod zip_url_validation {
        use super::*;

        #[test]
        fn valid_zip_url() {
            assert!(validate_zip_url("https://example.com/package.zip").is_ok());
        }

        #[test]
        fn valid_zip_url_with_query() {
            assert!(validate_zip_url("https://example.com/package.zip?token=abc").is_ok());
        }

        #[test]
        fn invalid_non_zip_url() {
            assert!(validate_zip_url("https://example.com/package.json").is_err());
        }

        #[test]
        fn valid_extensionless_download_url() {
            assert!(validate_zip_url("https://example.com/download/12345").is_ok());
        }
    }

    mod unity_dependency_version {
        use super::*;

        #[test]
        fn accepts_exact_semver() {
            assert!(validate_unity_dependency_version("1.2.3").is_ok());
        }

        #[test]
        fn accepts_prerelease_semver() {
            assert!(validate_unity_dependency_version("1.2.3-beta.1").is_ok());
        }

        #[test]
        fn rejects_version_range() {
            assert!(validate_unity_dependency_version("^1.2.3").is_err());
        }
    }

    mod unity_version {
        use super::*;

        #[test]
        fn accepts_standard_version() {
            assert!(validate_unity_version("2022.3").is_ok());
        }

        #[test]
        fn accepts_older_version() {
            assert!(validate_unity_version("2019.1").is_ok());
        }

        #[test]
        fn accepts_unity6_version() {
            assert!(validate_unity_version("6000.0").is_ok());
        }

        #[test]
        fn rejects_major_only() {
            assert!(validate_unity_version("2022").is_err());
        }

        #[test]
        fn rejects_three_parts() {
            assert!(validate_unity_version("2022.3.1").is_err());
        }

        #[test]
        fn rejects_non_numeric() {
            assert!(validate_unity_version("hello.world").is_err());
        }

        #[test]
        fn rejects_empty_part() {
            assert!(validate_unity_version("2022.").is_err());
        }

        #[test]
        fn rejects_x_wildcard() {
            assert!(validate_unity_version("2022.x").is_err());
        }
    }

    mod unity_release {
        use super::*;

        #[test]
        fn accepts_standard_beta_release() {
            assert!(validate_unity_release("0b4").is_ok());
        }

        #[test]
        fn accepts_multi_digit_release() {
            assert!(validate_unity_release("22f1").is_ok());
        }

        #[test]
        fn rejects_missing_update_number() {
            assert!(validate_unity_release("b4").is_err());
        }

        #[test]
        fn rejects_long_channel_name() {
            assert!(validate_unity_release("0beta4").is_err());
        }

        #[test]
        fn rejects_missing_release_number() {
            assert!(validate_unity_release("0b").is_err());
        }

        #[test]
        fn rejects_uppercase_channel_letter() {
            assert!(validate_unity_release("0B4").is_err());
        }
    }

    mod vpm_dependency_range {
        use super::*;

        #[test]
        fn accepts_comparator_range() {
            assert!(validate_vpm_dependency_range(">=3.4.0").is_ok());
        }

        #[test]
        fn accepts_space_separated_range() {
            assert!(validate_vpm_dependency_range(">=3.4.0 <3.5.0").is_ok());
        }

        #[test]
        fn accepts_x_range() {
            assert!(validate_vpm_dependency_range("3.5.x").is_ok());
        }

        #[test]
        fn accepts_or_range() {
            assert!(validate_vpm_dependency_range("^1.2.3 || 2.x").is_ok());
        }

        #[test]
        fn accepts_hyphen_range() {
            assert!(validate_vpm_dependency_range("1.2.3 - 2.0.0").is_ok());
        }

        #[test]
        fn rejects_empty() {
            assert!(validate_vpm_dependency_range("").is_err());
        }

        #[test]
        fn rejects_invalid() {
            assert!(validate_vpm_dependency_range("definitely-not-a-range").is_err());
        }
    }
}
