use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Normalized package manifest data persisted in voyager.lock.
///
/// This mirrors the fields voyager needs from package.json while remaining
/// independent from output serialization types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageManifest {
    pub name: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    pub unity: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unity_release: String,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub dependencies: IndexMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    pub author: PackageAuthor,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub vpm_dependencies: IndexMap<String, String>,
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub license: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageAuthor {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
}
