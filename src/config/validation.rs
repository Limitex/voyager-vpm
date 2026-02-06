use crate::error::{Error, Result};
use reqwest::Url;

/// Validates that a string is in reverse domain notation.
///
/// Valid examples: "com.example.package", "org.test-project.utils"
/// Invalid examples: "invalid", "com.", ".example"
///
/// Rules:
/// - Must have at least 2 parts separated by dots
/// - Each part must be non-empty
/// - Each part can only contain alphanumeric characters, hyphens, or underscores
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
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
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
            // "com.exampleother" should not match "com.example" (needs dot separator)
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
}
