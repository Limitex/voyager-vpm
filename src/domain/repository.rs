use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Repository {
    pub owner: String,
    pub repo: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryParseError {
    input: String,
}

impl RepositoryParseError {
    fn new(input: &str) -> Self {
        Self {
            input: input.to_string(),
        }
    }

    pub fn input(&self) -> &str {
        &self.input
    }
}

impl fmt::Display for RepositoryParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Invalid repository format '{}', expected 'owner/repo'",
            self.input
        )
    }
}

impl std::error::Error for RepositoryParseError {}

impl Repository {
    pub fn parse(s: &str) -> Result<Self, RepositoryParseError> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 2 {
            return Err(RepositoryParseError::new(s));
        }

        let owner = parts[0];
        let repo = parts[1];

        if owner.is_empty() || repo.is_empty() {
            return Err(RepositoryParseError::new(s));
        }

        if !is_valid_owner(owner) || !is_valid_repo(repo) {
            return Err(RepositoryParseError::new(s));
        }

        Ok(Self {
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    }
}

fn is_valid_owner(owner: &str) -> bool {
    if owner.len() > 39 {
        return false;
    }

    if owner.starts_with('-') || owner.ends_with('-') {
        return false;
    }

    owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn is_valid_repo(repo: &str) -> bool {
    repo.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

impl fmt::Display for Repository {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}

impl Serialize for Repository {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Repository {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Repository::parse(&s).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod parse {
        use super::*;

        #[test]
        fn parses_valid_owner_repo() {
            let repo = Repository::parse("owner/repo").unwrap();
            assert_eq!(repo.owner, "owner");
            assert_eq!(repo.repo, "repo");
        }

        #[test]
        fn parses_owner_repo_with_hyphens() {
            let repo = Repository::parse("my-owner/my-repo").unwrap();
            assert_eq!(repo.owner, "my-owner");
            assert_eq!(repo.repo, "my-repo");
        }

        #[test]
        fn parses_repo_with_dots() {
            let repo = Repository::parse("owner/repo.name").unwrap();
            assert_eq!(repo.owner, "owner");
            assert_eq!(repo.repo, "repo.name");
        }

        #[test]
        fn fails_on_multiple_slashes() {
            let result = Repository::parse("owner/repo/extra");
            assert!(result.is_err());
        }

        #[test]
        fn fails_on_missing_slash() {
            let result = Repository::parse("ownerrepo");
            assert!(result.is_err());
        }

        #[test]
        fn fails_on_empty_owner() {
            let result = Repository::parse("/repo");
            assert!(result.is_err());
        }

        #[test]
        fn fails_on_empty_repo() {
            let result = Repository::parse("owner/");
            assert!(result.is_err());
        }

        #[test]
        fn fails_on_empty_string() {
            let result = Repository::parse("");
            assert!(result.is_err());
        }

        #[test]
        fn fails_on_only_slash() {
            let result = Repository::parse("/");
            assert!(result.is_err());
        }

        #[test]
        fn fails_on_owner_with_dot() {
            let result = Repository::parse("owner.name/repo");
            assert!(result.is_err());
        }

        #[test]
        fn fails_on_owner_starting_with_hyphen() {
            let result = Repository::parse("-owner/repo");
            assert!(result.is_err());
        }

        #[test]
        fn fails_on_owner_ending_with_hyphen() {
            let result = Repository::parse("owner-/repo");
            assert!(result.is_err());
        }

        #[test]
        fn fails_on_repo_with_space() {
            let result = Repository::parse("owner/my repo");
            assert!(result.is_err());
        }
    }

    mod display {
        use super::*;

        #[test]
        fn formats_as_owner_slash_repo() {
            let repo = Repository::parse("owner/repo").unwrap();
            assert_eq!(format!("{}", repo), "owner/repo");
        }

        #[test]
        fn formats_complex_names() {
            let repo = Repository::parse("my-owner/my-repo").unwrap();
            assert_eq!(format!("{}", repo), "my-owner/my-repo");
        }
    }

    mod deserialize {
        use super::*;

        #[test]
        fn deserializes_from_string() {
            let json = r#""owner/repo""#;
            let repo: Repository = serde_json::from_str(json).unwrap();
            assert_eq!(repo.owner, "owner");
            assert_eq!(repo.repo, "repo");
        }

        #[test]
        fn fails_on_invalid_format() {
            let json = r#""invalid""#;
            let result: Result<Repository, _> = serde_json::from_str(json);
            assert!(result.is_err());
        }

        #[test]
        fn fails_on_empty_string() {
            let json = r#""""#;
            let result: Result<Repository, _> = serde_json::from_str(json);
            assert!(result.is_err());
        }
    }
}
