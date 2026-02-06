use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct Release {
    tag: String,
    asset_url: Option<String>,
}

impl Release {
    pub fn new(tag: String, asset_url: Option<String>) -> Self {
        Self { tag, asset_url }
    }

    pub fn tag(&self) -> &str {
        &self.tag
    }

    pub fn version(&self) -> &str {
        self.tag.strip_prefix('v').unwrap_or(&self.tag)
    }

    pub fn asset_url(&self) -> Option<&str> {
        self.asset_url.as_deref()
    }

    pub fn filter_new<'a>(
        releases: &'a [Release],
        existing_versions: &HashSet<String>,
    ) -> Vec<&'a Release> {
        releases
            .iter()
            .filter(|r| r.asset_url.is_some() && !existing_versions.contains(r.version()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod version {
        use super::*;

        #[test]
        fn strips_v_prefix() {
            let release = Release::new("v1.0.0".to_string(), None);
            assert_eq!(release.version(), "1.0.0");
        }

        #[test]
        fn returns_tag_when_no_v_prefix() {
            let release = Release::new("1.0.0".to_string(), None);
            assert_eq!(release.version(), "1.0.0");
        }

        #[test]
        fn handles_v_only_tag() {
            let release = Release::new("v".to_string(), None);
            assert_eq!(release.version(), "");
        }

        #[test]
        fn preserves_complex_version() {
            let release = Release::new("v1.2.3-beta.1+build.123".to_string(), None);
            assert_eq!(release.version(), "1.2.3-beta.1+build.123");
        }

        #[test]
        fn handles_uppercase_v() {
            let release = Release::new("V1.0.0".to_string(), None);
            assert_eq!(release.version(), "V1.0.0");
        }
    }

    mod tag {
        use super::*;

        #[test]
        fn returns_original_tag() {
            let release = Release::new("v1.0.0".to_string(), None);
            assert_eq!(release.tag(), "v1.0.0");
        }
    }

    mod asset_url {
        use super::*;

        #[test]
        fn returns_some_when_present() {
            let release =
                Release::new("v1.0.0".to_string(), Some("http://example.com".to_string()));
            assert_eq!(release.asset_url(), Some("http://example.com"));
        }

        #[test]
        fn returns_none_when_absent() {
            let release = Release::new("v1.0.0".to_string(), None);
            assert_eq!(release.asset_url(), None);
        }
    }

    mod filter_new {
        use super::*;

        #[test]
        fn filters_existing_versions() {
            let releases = vec![
                Release::new("v1.0.0".to_string(), Some("url1".to_string())),
                Release::new("v2.0.0".to_string(), Some("url2".to_string())),
                Release::new("v3.0.0".to_string(), Some("url3".to_string())),
            ];
            let existing: HashSet<String> = ["1.0.0".to_string(), "2.0.0".to_string()]
                .into_iter()
                .collect();

            let new = Release::filter_new(&releases, &existing);

            assert_eq!(new.len(), 1);
            assert_eq!(new[0].version(), "3.0.0");
        }

        #[test]
        fn excludes_releases_without_asset_url() {
            let releases = vec![
                Release::new("v1.0.0".to_string(), None),
                Release::new("v2.0.0".to_string(), Some("url".to_string())),
            ];
            let existing: HashSet<String> = HashSet::new();

            let new = Release::filter_new(&releases, &existing);

            assert_eq!(new.len(), 1);
            assert_eq!(new[0].version(), "2.0.0");
        }

        #[test]
        fn returns_empty_when_all_existing() {
            let releases = vec![Release::new("v1.0.0".to_string(), Some("url".to_string()))];
            let existing: HashSet<String> = ["1.0.0".to_string()].into_iter().collect();

            let new = Release::filter_new(&releases, &existing);

            assert!(new.is_empty());
        }

        #[test]
        fn returns_all_when_none_existing() {
            let releases = vec![
                Release::new("v1.0.0".to_string(), Some("url1".to_string())),
                Release::new("v2.0.0".to_string(), Some("url2".to_string())),
            ];
            let existing: HashSet<String> = HashSet::new();

            let new = Release::filter_new(&releases, &existing);

            assert_eq!(new.len(), 2);
        }

        #[test]
        fn handles_empty_releases() {
            let releases: Vec<Release> = vec![];
            let existing: HashSet<String> = ["1.0.0".to_string()].into_iter().collect();

            let new = Release::filter_new(&releases, &existing);

            assert!(new.is_empty());
        }

        #[test]
        fn handles_empty_existing() {
            let releases = vec![Release::new("v1.0.0".to_string(), Some("url".to_string()))];
            let existing: HashSet<String> = HashSet::new();

            let new = Release::filter_new(&releases, &existing);

            assert_eq!(new.len(), 1);
        }
    }
}
