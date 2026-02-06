use super::validation;
use crate::domain::Repository;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub vpm: Vpm,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<Package>,
}

impl Manifest {
    pub fn new(vpm: Vpm) -> Self {
        Self {
            vpm,
            packages: Vec::new(),
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let path_str = path.display().to_string();

        let content = std::fs::read_to_string(path).map_err(|e| Error::FileRead {
            path: path_str.clone(),
            source: e,
        })?;

        let manifest: Manifest = toml::from_str(&content).map_err(|e| Error::TomlParse {
            path: path_str,
            source: e,
        })?;

        manifest.validate()?;
        Ok(manifest)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path = path.as_ref();
        let content = toml::to_string_pretty(self).map_err(|e| Error::TomlSerialize {
            path: path.display().to_string(),
            source: e,
        })?;

        std::fs::write(path, content).map_err(|e| Error::FileWrite {
            path: path.display().to_string(),
            source: e,
        })?;

        Ok(())
    }

    fn validate(&self) -> Result<()> {
        self.vpm.validate()?;

        let mut seen_ids = HashSet::new();
        for package in &self.packages {
            package.validate()?;
            validation::validate_package_id_prefix(&package.id, &self.vpm.id)?;

            if !seen_ids.insert(&package.id) {
                return Err(Error::ConfigValidation(format!(
                    "Duplicate package ID: {}",
                    package.id
                )));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Vpm {
    pub id: String,
    pub name: String,
    pub author: String,
    pub url: String,
}

impl Vpm {
    fn validate(&self) -> Result<()> {
        if self.id.is_empty() {
            return Err(Error::ConfigValidation("VPM id is empty".to_string()));
        }

        validation::validate_reverse_domain(&self.id)?;

        if self.name.is_empty() {
            return Err(Error::ConfigValidation("VPM name is empty".to_string()));
        }

        if self.author.is_empty() {
            return Err(Error::ConfigValidation("VPM author is empty".to_string()));
        }

        validation::validate_url(&self.url)?;

        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Package {
    pub id: String,
    pub repository: Repository,
}

impl Package {
    fn validate(&self) -> Result<()> {
        if self.id.is_empty() {
            return Err(Error::ConfigValidation("Package id is empty".to_string()));
        }

        validation::validate_reverse_domain(&self.id)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_temp_manifest(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file
    }

    mod manifest_load {
        use super::*;

        #[test]
        fn loads_valid_manifest() {
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

            assert_eq!(manifest.vpm.id, "com.example.vpm");
            assert_eq!(manifest.vpm.name, "Example VPM");
            assert_eq!(manifest.vpm.author, "Test Author");
            assert_eq!(manifest.vpm.url, "https://example.com/vpm.json");
            assert_eq!(manifest.packages.len(), 1);
            assert_eq!(manifest.packages[0].id, "com.example.vpm.package");
        }

        #[test]
        fn loads_manifest_with_multiple_packages() {
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
            let manifest = Manifest::load(file.path()).unwrap();

            assert_eq!(manifest.packages.len(), 2);
        }

        #[test]
        fn allows_empty_packages() {
            let content = r#"
[vpm]
id = "com.example.vpm"
name = "Example VPM"
author = "Test Author"
url = "https://example.com/vpm.json"

packages = []
"#;
            let file = create_temp_manifest(content);
            let manifest = Manifest::load(file.path()).unwrap();

            assert!(manifest.packages.is_empty());
        }

        #[test]
        fn allows_missing_packages() {
            let content = r#"
[vpm]
id = "com.example.vpm"
name = "Example VPM"
author = "Test Author"
url = "https://example.com/vpm.json"
"#;
            let file = create_temp_manifest(content);
            let manifest = Manifest::load(file.path()).unwrap();

            assert!(manifest.packages.is_empty());
        }

        #[test]
        fn fails_on_invalid_vpm_id() {
            let content = r#"
[vpm]
id = "invalid"
name = "Example VPM"
author = "Test Author"
url = "https://example.com/vpm.json"

[[packages]]
id = "com.example.vpm.package"
repository = "owner/repo"
"#;
            let file = create_temp_manifest(content);
            let result = Manifest::load(file.path());

            assert!(matches!(result, Err(Error::InvalidPackageId(_))));
        }

        #[test]
        fn fails_on_invalid_package_id() {
            let content = r#"
[vpm]
id = "com.example.vpm"
name = "Example VPM"
author = "Test Author"
url = "https://example.com/vpm.json"

[[packages]]
id = "invalid"
repository = "owner/repo"
"#;
            let file = create_temp_manifest(content);
            let result = Manifest::load(file.path());

            assert!(matches!(result, Err(Error::InvalidPackageId(_))));
        }

        #[test]
        fn fails_when_package_id_prefix_does_not_match_vpm_id() {
            let content = r#"
[vpm]
id = "com.example.vpm"
name = "Example VPM"
author = "Test Author"
url = "https://example.com/vpm.json"

[[packages]]
id = "org.other.package"
repository = "owner/repo"
"#;
            let file = create_temp_manifest(content);
            let result = Manifest::load(file.path());

            assert!(matches!(result, Err(Error::InvalidPackageId(_))));
        }

        #[test]
        fn fails_on_invalid_url() {
            let content = r#"
[vpm]
id = "com.example.vpm"
name = "Example VPM"
author = "Test Author"
url = "invalid-url"

[[packages]]
id = "com.example.vpm.package"
repository = "owner/repo"
"#;
            let file = create_temp_manifest(content);
            let result = Manifest::load(file.path());

            assert!(matches!(result, Err(Error::InvalidUrl(_, _))));
        }

        #[test]
        fn fails_on_empty_vpm_name() {
            let content = r#"
[vpm]
id = "com.example.vpm"
name = ""
author = "Test Author"
url = "https://example.com/vpm.json"

[[packages]]
id = "com.example.vpm.package"
repository = "owner/repo"
"#;
            let file = create_temp_manifest(content);
            let result = Manifest::load(file.path());

            assert!(matches!(result, Err(Error::ConfigValidation(_))));
        }

        #[test]
        fn fails_on_duplicate_package_id() {
            let content = r#"
[vpm]
id = "com.example.vpm"
name = "Example VPM"
author = "Test Author"
url = "https://example.com/vpm.json"

[[packages]]
id = "com.example.vpm.package"
repository = "owner/repo1"

[[packages]]
id = "com.example.vpm.package"
repository = "owner/repo2"
"#;
            let file = create_temp_manifest(content);
            let result = Manifest::load(file.path());

            assert!(
                matches!(result, Err(Error::ConfigValidation(msg)) if msg.contains("Duplicate"))
            );
        }

        #[test]
        fn fails_on_missing_file() {
            let result = Manifest::load("/nonexistent/path.toml");
            assert!(matches!(result, Err(Error::FileRead { .. })));
        }

        #[test]
        fn fails_on_invalid_toml() {
            let content = "invalid toml content {{{";
            let file = create_temp_manifest(content);
            let result = Manifest::load(file.path());

            assert!(matches!(result, Err(Error::TomlParse { .. })));
        }
    }
}
