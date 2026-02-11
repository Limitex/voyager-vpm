use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// Sample entry in a Unity package manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub display_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub path: String,
}

/// Normalized package manifest data persisted in voyager.lock.
///
/// This mirrors the fields voyager needs from package.json while remaining
/// independent from output serialization types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unity: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unity_release: String,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub dependencies: IndexMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub author: PackageAuthor,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub vpm_dependencies: IndexMap<String, String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub legacy_folders: IndexMap<String, String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub legacy_files: IndexMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legacy_packages: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub documentation_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub changelog_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub licenses_url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<Sample>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide_in_editor: Option<bool>,
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "type")]
    pub package_type: String,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        rename = "zipSHA256"
    )]
    pub zip_sha256: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub license: String,
    #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
    pub extra: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct PackageAuthor {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
}

impl PackageAuthor {
    fn parse_author_string(raw: &str) -> Self {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Self::default();
        }

        let mut remainder = trimmed.to_string();
        let email = extract_bracketed_segment(&mut remainder, '<', '>');
        let url = extract_bracketed_segment(&mut remainder, '(', ')');
        let name = collapse_whitespace(&remainder);

        Self {
            name: if name.is_empty() {
                trimmed.to_string()
            } else {
                name
            },
            email,
            url,
        }
    }
}

fn extract_bracketed_segment(input: &mut String, open: char, close: char) -> String {
    let Some(start) = input.find(open) else {
        return String::new();
    };
    let value_start = start + open.len_utf8();
    let Some(end_rel) = input[value_start..].find(close) else {
        return String::new();
    };
    let value_end = value_start + end_rel;
    let value = input[value_start..value_end].trim().to_string();
    input.replace_range(start..value_end + close.len_utf8(), " ");
    value
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl<'de> Deserialize<'de> for PackageAuthor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum AuthorRepr {
            Name(String),
            Object {
                #[serde(default)]
                name: String,
                #[serde(default)]
                email: String,
                #[serde(default)]
                url: String,
            },
        }

        match AuthorRepr::deserialize(deserializer)? {
            AuthorRepr::Name(name) => Ok(Self::parse_author_string(&name)),
            AuthorRepr::Object { name, email, url } => Ok(Self { name, email, url }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_author_as_string() {
        let json = r#"{
            "name": "com.example.pkg",
            "version": "1.2.3",
            "url": "https://example.com/pkg.zip",
            "author": "Example Author <author@example.com> (https://example.com)"
        }"#;

        let manifest: PackageManifest = serde_json::from_str(json).unwrap();

        assert_eq!(manifest.author.name, "Example Author");
        assert_eq!(manifest.author.email, "author@example.com");
        assert_eq!(manifest.author.url, "https://example.com");
    }

    #[test]
    fn deserializes_author_as_plain_name_string() {
        let json = r#"{
            "name": "com.example.pkg",
            "version": "1.2.3",
            "url": "https://example.com/pkg.zip",
            "author": "Example Author"
        }"#;

        let manifest: PackageManifest = serde_json::from_str(json).unwrap();

        assert_eq!(manifest.author.name, "Example Author");
        assert_eq!(manifest.author.email, "");
        assert_eq!(manifest.author.url, "");
    }

    #[test]
    fn deserializes_author_as_object() {
        let json = r#"{
            "name": "com.example.pkg",
            "version": "1.2.3",
            "url": "https://example.com/pkg.zip",
            "author": {
                "name": "Example Author",
                "email": "author@example.com",
                "url": "https://example.com"
            }
        }"#;

        let manifest: PackageManifest = serde_json::from_str(json).unwrap();

        assert_eq!(
            manifest.author,
            PackageAuthor {
                name: "Example Author".to_string(),
                email: "author@example.com".to_string(),
                url: "https://example.com".to_string(),
            }
        );
    }

    #[test]
    fn defaults_recommended_fields_when_missing() {
        let json = r#"{
            "name": "com.example.pkg",
            "version": "1.2.3",
            "url": "https://example.com/pkg.zip"
        }"#;

        let manifest: PackageManifest = serde_json::from_str(json).unwrap();

        assert_eq!(manifest.display_name, "");
        assert_eq!(manifest.description, "");
        assert_eq!(manifest.unity, "");
        assert_eq!(manifest.author, PackageAuthor::default());
    }

    #[test]
    fn deserializes_vpm_extension_fields() {
        let json = r#"{
            "name": "com.example.pkg",
            "version": "1.2.3",
            "displayName": "Example",
            "author": {
                "name": "Example Author",
                "email": "author@example.com"
            },
            "legacyFolders": {
                "Assets/OldFolder": "Assets/NewFolder"
            },
            "legacyFiles": {
                "Assets/Old.prefab": "Assets/New.prefab"
            },
            "legacyPackages": ["com.example.oldpkg"],
            "changelogUrl": "https://example.com/changelog",
            "zipSHA256": "abcdef",
            "url": "https://example.com/pkg.zip",
            "documentationUrl": "https://example.com/docs",
            "samples": [
                {
                    "displayName": "Example Sample",
                    "description": "A sample",
                    "path": "Samples~/Example"
                }
            ]
        }"#;

        let manifest: PackageManifest = serde_json::from_str(json).unwrap();

        assert_eq!(
            manifest
                .legacy_folders
                .get("Assets/OldFolder")
                .map(String::as_str),
            Some("Assets/NewFolder")
        );
        assert_eq!(
            manifest
                .legacy_files
                .get("Assets/Old.prefab")
                .map(String::as_str),
            Some("Assets/New.prefab")
        );
        assert_eq!(manifest.legacy_packages, vec!["com.example.oldpkg"]);
        assert_eq!(manifest.changelog_url, "https://example.com/changelog");
        assert_eq!(manifest.zip_sha256, "abcdef");
        assert_eq!(manifest.documentation_url, "https://example.com/docs");
        assert_eq!(manifest.samples.len(), 1);
        assert_eq!(manifest.samples[0].display_name, "Example Sample");
        assert_eq!(manifest.samples[0].description, "A sample");
        assert_eq!(manifest.samples[0].path, "Samples~/Example");
    }
}
