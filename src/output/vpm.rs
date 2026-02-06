use crate::config::Manifest;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct VpmOutput {
    pub name: String,
    pub id: String,
    pub url: String,
    pub author: String,
    pub packages: IndexMap<String, PackageOutput>,
}

impl VpmOutput {
    pub fn from_manifest(manifest: &Manifest) -> Self {
        let packages = manifest
            .packages
            .iter()
            .map(|p| {
                (
                    p.id.clone(),
                    PackageOutput {
                        versions: IndexMap::new(),
                    },
                )
            })
            .collect();

        Self {
            name: manifest.vpm.name.clone(),
            id: manifest.vpm.id.clone(),
            url: manifest.vpm.url.clone(),
            author: manifest.vpm.author.clone(),
            packages,
        }
    }

    pub fn collect_urls(&self) -> Vec<(String, String, String)> {
        self.packages
            .iter()
            .flat_map(|(package_id, pkg)| {
                pkg.versions
                    .iter()
                    .map(move |(version, v)| (package_id.clone(), version.clone(), v.url.clone()))
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageOutput {
    pub versions: IndexMap<String, VersionOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionOutput {
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
    pub author: Author,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub vpm_dependencies: IndexMap<String, String>,
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub license: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Manifest;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_temp_manifest(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file
    }

    fn load_test_manifest() -> Manifest {
        let content = r#"
[vpm]
id = "com.example.vpm"
name = "Example VPM"
author = "Test Author"
url = "https://example.com/vpm.json"

[[packages]]
id = "com.example.vpm.package1"
repository = "owner/repo1"

[[packages]]
id = "com.example.vpm.package2"
repository = "owner/repo2"
"#;
        let file = create_temp_manifest(content);
        Manifest::load(file.path()).unwrap()
    }

    fn create_version_output(name: &str, version: &str, url: &str) -> VersionOutput {
        VersionOutput {
            name: name.to_string(),
            version: version.to_string(),
            display_name: "Test Package".to_string(),
            description: "Test description".to_string(),
            unity: "2022.3".to_string(),
            unity_release: String::new(),
            dependencies: IndexMap::new(),
            keywords: vec![],
            author: Author {
                name: "Test".to_string(),
                email: String::new(),
                url: String::new(),
            },
            vpm_dependencies: IndexMap::new(),
            url: url.to_string(),
            license: String::new(),
        }
    }

    mod from_manifest {
        use super::*;

        #[test]
        fn copies_vpm_metadata() {
            let manifest = load_test_manifest();
            let output = VpmOutput::from_manifest(&manifest);

            assert_eq!(output.name, "Example VPM");
            assert_eq!(output.id, "com.example.vpm");
            assert_eq!(output.url, "https://example.com/vpm.json");
            assert_eq!(output.author, "Test Author");
        }

        #[test]
        fn creates_entry_for_each_package() {
            let manifest = load_test_manifest();
            let output = VpmOutput::from_manifest(&manifest);

            assert_eq!(output.packages.len(), 2);
            assert!(output.packages.contains_key("com.example.vpm.package1"));
            assert!(output.packages.contains_key("com.example.vpm.package2"));
        }

        #[test]
        fn packages_have_empty_versions() {
            let manifest = load_test_manifest();
            let output = VpmOutput::from_manifest(&manifest);

            for (_, pkg) in &output.packages {
                assert!(pkg.versions.is_empty());
            }
        }

        #[test]
        fn preserves_package_order() {
            let manifest = load_test_manifest();
            let output = VpmOutput::from_manifest(&manifest);

            let keys: Vec<_> = output.packages.keys().collect();
            assert_eq!(keys[0], "com.example.vpm.package1");
            assert_eq!(keys[1], "com.example.vpm.package2");
        }

        #[test]
        fn handles_single_package() {
            let content = r#"
[vpm]
id = "com.example.vpm"
name = "Example VPM"
author = "Test Author"
url = "https://example.com/vpm.json"

[[packages]]
id = "com.example.vpm.package"
repository = "owner/repo"
"#;
            let file = create_temp_manifest(content);
            let manifest = Manifest::load(file.path()).unwrap();

            let output = VpmOutput::from_manifest(&manifest);
            assert_eq!(output.packages.len(), 1);
        }
    }

    mod collect_urls {
        use super::*;

        #[test]
        fn collects_all_urls_with_package_and_version() {
            let mut output = VpmOutput::from_manifest(&load_test_manifest());

            let pkg = output.packages.get_mut("com.example.vpm.package1").unwrap();
            pkg.versions.insert(
                "1.0.0".to_string(),
                create_version_output("pkg1", "1.0.0", "https://example.com/pkg1-1.0.0.zip"),
            );
            pkg.versions.insert(
                "2.0.0".to_string(),
                create_version_output("pkg1", "2.0.0", "https://example.com/pkg1-2.0.0.zip"),
            );

            let urls = output.collect_urls();

            assert_eq!(urls.len(), 2);
        }

        #[test]
        fn url_tuple_contains_correct_data() {
            let mut output = VpmOutput::from_manifest(&load_test_manifest());

            let pkg = output.packages.get_mut("com.example.vpm.package1").unwrap();
            pkg.versions.insert(
                "1.0.0".to_string(),
                create_version_output("pkg1", "1.0.0", "https://example.com/pkg1-1.0.0.zip"),
            );

            let urls = output.collect_urls();

            assert_eq!(urls.len(), 1);
            let (pkg_id, version, url) = &urls[0];
            assert_eq!(pkg_id, "com.example.vpm.package1");
            assert_eq!(version, "1.0.0");
            assert_eq!(url, "https://example.com/pkg1-1.0.0.zip");
        }

        #[test]
        fn returns_empty_for_no_packages() {
            let output = VpmOutput {
                name: "Test".to_string(),
                id: "com.test".to_string(),
                url: "https://test.com".to_string(),
                author: "Author".to_string(),
                packages: IndexMap::new(),
            };

            let urls = output.collect_urls();
            assert!(urls.is_empty());
        }

        #[test]
        fn returns_empty_for_packages_with_no_versions() {
            let output = VpmOutput::from_manifest(&load_test_manifest());
            let urls = output.collect_urls();
            assert!(urls.is_empty());
        }

        #[test]
        fn collects_from_multiple_packages() {
            let mut output = VpmOutput::from_manifest(&load_test_manifest());

            let pkg1 = output.packages.get_mut("com.example.vpm.package1").unwrap();
            pkg1.versions.insert(
                "1.0.0".to_string(),
                create_version_output("pkg1", "1.0.0", "https://example.com/pkg1.zip"),
            );

            let pkg2 = output.packages.get_mut("com.example.vpm.package2").unwrap();
            pkg2.versions.insert(
                "1.0.0".to_string(),
                create_version_output("pkg2", "1.0.0", "https://example.com/pkg2.zip"),
            );

            let urls = output.collect_urls();

            assert_eq!(urls.len(), 2);
        }
    }

    mod serialization {
        use super::*;

        #[test]
        fn version_output_uses_camel_case() {
            let version = create_version_output("test", "1.0.0", "https://example.com/test.zip");
            let json = serde_json::to_string(&version).unwrap();

            assert!(json.contains("\"displayName\""));
            assert!(json.contains("\"unityRelease\"") || !json.contains("unity_release"));
            assert!(!json.contains("\"display_name\""));
        }

        #[test]
        fn empty_fields_are_skipped() {
            let version = create_version_output("test", "1.0.0", "https://example.com/test.zip");
            let json = serde_json::to_string(&version).unwrap();

            assert!(!json.contains("\"unityRelease\""));
            assert!(!json.contains("\"dependencies\""));
            assert!(!json.contains("\"keywords\""));
            assert!(!json.contains("\"license\""));
        }

        #[test]
        fn roundtrip_preserves_data() {
            let mut version =
                create_version_output("test", "1.0.0", "https://example.com/test.zip");
            version.keywords = vec!["tag1".to_string(), "tag2".to_string()];
            version.license = "MIT".to_string();

            let json = serde_json::to_string(&version).unwrap();
            let parsed: VersionOutput = serde_json::from_str(&json).unwrap();

            assert_eq!(parsed.name, version.name);
            assert_eq!(parsed.version, version.version);
            assert_eq!(parsed.url, version.url);
            assert_eq!(parsed.keywords, version.keywords);
            assert_eq!(parsed.license, version.license);
        }
    }
}
