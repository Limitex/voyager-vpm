use crate::config::Manifest;
use crate::error::{Error, Result};
use crate::lock::{Lockfile, PackageManifest};
use crate::output::{Author, VersionOutput, VpmOutput};
use indexmap::IndexMap;
use tracing::info;

/// Generates VPM index output from a manifest and lockfile.
///
/// This function transforms the locked package data into the VPM index format
/// that can be published for VCC (VRChat Creator Companion) to consume.
pub fn generate_from_lockfile(manifest: &Manifest, lockfile: &Lockfile) -> Result<VpmOutput> {
    let mut output = VpmOutput::from_manifest(manifest);

    for package in &manifest.packages {
        let locked_pkg = lockfile.get_package(&package.id).ok_or_else(|| {
            Error::ConfigValidation(format!(
                "Lock file missing package '{}'. Run 'voy fetch' first.",
                package.id
            ))
        })?;
        let mut versions = IndexMap::new();

        for locked_version in &locked_pkg.versions {
            versions.insert(
                locked_version.version.clone(),
                to_output_version(&locked_version.manifest),
            );
        }

        // VpmOutput::from_manifest() already creates entries for all packages,
        // so this lookup should always succeed
        output
            .packages
            .get_mut(&package.id)
            .expect("Package was created by from_manifest")
            .versions = versions;
    }

    info!(
        packages = output.packages.len(),
        "Index generation completed"
    );

    Ok(output)
}

fn to_output_version(manifest: &PackageManifest) -> VersionOutput {
    VersionOutput {
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        display_name: manifest.display_name.clone(),
        description: manifest.description.clone(),
        unity: manifest.unity.clone(),
        unity_release: manifest.unity_release.clone(),
        dependencies: manifest.dependencies.clone(),
        keywords: manifest.keywords.clone(),
        author: Author {
            name: manifest.author.name.clone(),
            email: manifest.author.email.clone(),
            url: manifest.author.url.clone(),
        },
        vpm_dependencies: manifest.vpm_dependencies.clone(),
        legacy_folders: manifest.legacy_folders.clone(),
        legacy_files: manifest.legacy_files.clone(),
        legacy_packages: manifest.legacy_packages.clone(),
        documentation_url: manifest.documentation_url.clone(),
        changelog_url: manifest.changelog_url.clone(),
        licenses_url: manifest.licenses_url.clone(),
        samples: manifest.samples.clone(),
        hide_in_editor: manifest.hide_in_editor,
        package_type: manifest.package_type.clone(),
        zip_sha256: manifest.zip_sha256.clone(),
        url: manifest.url.clone(),
        license: manifest.license.clone(),
        extra: manifest.extra.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Package, Vpm};
    use crate::domain::Repository;
    use crate::lock::{LockedPackage, LockedVersion, Lockfile, PackageAuthor, PackageManifest};

    fn repo(s: &str) -> Repository {
        Repository::parse(s).unwrap()
    }

    fn create_manifest() -> Manifest {
        Manifest {
            vpm: Vpm {
                id: "com.example.vpm".to_string(),
                name: "Example VPM".to_string(),
                author: "Example Author".to_string(),
                url: "https://example.com/vpm.json".to_string(),
            },
            packages: vec![
                Package {
                    id: "com.example.pkg1".to_string(),
                    repository: Repository::parse("owner/repo1").unwrap(),
                },
                Package {
                    id: "com.example.pkg2".to_string(),
                    repository: Repository::parse("owner/repo2").unwrap(),
                },
            ],
        }
    }

    fn create_version_output(name: &str, version: &str) -> PackageManifest {
        PackageManifest {
            name: name.to_string(),
            version: version.to_string(),
            display_name: "Test Package".to_string(),
            description: "Test description".to_string(),
            unity: "2022.3".to_string(),
            unity_release: String::new(),
            dependencies: IndexMap::new(),
            keywords: vec![],
            author: PackageAuthor {
                name: "Test".to_string(),
                email: String::new(),
                url: String::new(),
            },
            vpm_dependencies: IndexMap::new(),
            legacy_folders: IndexMap::new(),
            legacy_files: IndexMap::new(),
            legacy_packages: vec![],
            documentation_url: String::new(),
            changelog_url: String::new(),
            licenses_url: String::new(),
            samples: vec![],
            hide_in_editor: None,
            package_type: String::new(),
            zip_sha256: String::new(),
            url: "https://example.com/test.zip".to_string(),
            license: String::new(),
            extra: IndexMap::new(),
        }
    }

    #[test]
    fn generate_errors_when_lockfile_missing_package() {
        let manifest = create_manifest();

        let mut lockfile = Lockfile::new();
        let pkg1 = LockedPackage {
            id: "com.example.pkg1".to_string(),
            repository: repo("owner/repo1"),
            versions: vec![LockedVersion::new(
                "v1.0.0".to_string(),
                "https://example.com/pkg1/package.json".to_string(),
                r#"{"name": "pkg1"}"#,
                create_version_output("pkg1", "1.0.0"),
            )],
        };
        lockfile.packages.push(pkg1);

        let result = generate_from_lockfile(&manifest, &lockfile);
        assert!(matches!(result, Err(Error::ConfigValidation(_))));
    }

    #[test]
    fn generate_preserves_manifest_order() {
        let manifest = create_manifest();

        let mut lockfile = Lockfile::new();
        let pkg2 = LockedPackage {
            id: "com.example.pkg2".to_string(),
            repository: repo("owner/repo2"),
            versions: vec![LockedVersion::new(
                "v2.0.0".to_string(),
                "https://example.com/pkg2/package.json".to_string(),
                r#"{"name": "pkg2"}"#,
                create_version_output("pkg2", "2.0.0"),
            )],
        };
        let pkg1 = LockedPackage {
            id: "com.example.pkg1".to_string(),
            repository: repo("owner/repo1"),
            versions: vec![LockedVersion::new(
                "v1.0.0".to_string(),
                "https://example.com/pkg1/package.json".to_string(),
                r#"{"name": "pkg1"}"#,
                create_version_output("pkg1", "1.0.0"),
            )],
        };
        lockfile.packages.push(pkg2);
        lockfile.packages.push(pkg1);

        let output = generate_from_lockfile(&manifest, &lockfile).unwrap();
        let keys: Vec<_> = output.packages.keys().cloned().collect();
        assert_eq!(
            keys,
            vec![
                "com.example.pkg1".to_string(),
                "com.example.pkg2".to_string()
            ]
        );
    }

    #[test]
    fn generate_includes_all_versions() {
        let manifest = Manifest {
            vpm: Vpm {
                id: "com.example.vpm".to_string(),
                name: "Example VPM".to_string(),
                author: "Example Author".to_string(),
                url: "https://example.com/vpm.json".to_string(),
            },
            packages: vec![Package {
                id: "com.example.pkg".to_string(),
                repository: repo("owner/repo"),
            }],
        };

        let mut lockfile = Lockfile::new();
        let pkg = LockedPackage {
            id: "com.example.pkg".to_string(),
            repository: repo("owner/repo"),
            versions: vec![
                LockedVersion::new(
                    "v1.0.0".to_string(),
                    "https://example.com/v1.zip".to_string(),
                    r#"{"name": "pkg"}"#,
                    create_version_output("pkg", "1.0.0"),
                ),
                LockedVersion::new(
                    "v2.0.0".to_string(),
                    "https://example.com/v2.zip".to_string(),
                    r#"{"name": "pkg"}"#,
                    create_version_output("pkg", "2.0.0"),
                ),
            ],
        };
        lockfile.packages.push(pkg);

        let output = generate_from_lockfile(&manifest, &lockfile).unwrap();
        let pkg_output = output.packages.get("com.example.pkg").unwrap();
        assert_eq!(pkg_output.versions.len(), 2);
        assert!(pkg_output.versions.contains_key("1.0.0"));
        assert!(pkg_output.versions.contains_key("2.0.0"));
    }

    #[test]
    fn generate_preserves_vpm_extension_fields() {
        let manifest = Manifest {
            vpm: Vpm {
                id: "com.example.vpm".to_string(),
                name: "Example VPM".to_string(),
                author: "Example Author".to_string(),
                url: "https://example.com/vpm.json".to_string(),
            },
            packages: vec![Package {
                id: "com.example.pkg".to_string(),
                repository: repo("owner/repo"),
            }],
        };

        let mut lockfile = Lockfile::new();
        let mut pkg_manifest = create_version_output("com.example.pkg", "1.0.0");
        pkg_manifest
            .legacy_folders
            .insert("Assets/Old".to_string(), "Assets/New".to_string());
        pkg_manifest.legacy_files.insert(
            "Assets/Old.prefab".to_string(),
            "Assets/New.prefab".to_string(),
        );
        pkg_manifest
            .legacy_packages
            .push("com.example.legacy".to_string());
        pkg_manifest.changelog_url = "https://example.com/changelog".to_string();
        pkg_manifest.zip_sha256 = "deadbeef".to_string();
        pkg_manifest.extra.insert(
            "documentationUrl".to_string(),
            serde_json::Value::String("https://example.com/docs".to_string()),
        );

        let pkg = LockedPackage {
            id: "com.example.pkg".to_string(),
            repository: repo("owner/repo"),
            versions: vec![LockedVersion::new(
                "v1.0.0".to_string(),
                "https://example.com/v1.zip".to_string(),
                r#"{"name":"pkg"}"#,
                pkg_manifest,
            )],
        };
        lockfile.packages.push(pkg);

        let output = generate_from_lockfile(&manifest, &lockfile).unwrap();
        let version = output.packages["com.example.pkg"].versions["1.0.0"].clone();

        assert_eq!(
            version.legacy_folders.get("Assets/Old").map(String::as_str),
            Some("Assets/New")
        );
        assert_eq!(
            version
                .legacy_files
                .get("Assets/Old.prefab")
                .map(String::as_str),
            Some("Assets/New.prefab")
        );
        assert_eq!(version.legacy_packages, vec!["com.example.legacy"]);
        assert_eq!(version.changelog_url, "https://example.com/changelog");
        assert_eq!(version.zip_sha256, "deadbeef");
        assert_eq!(
            version.extra.get("documentationUrl"),
            Some(&serde_json::Value::String(
                "https://example.com/docs".to_string()
            ))
        );
    }
}
