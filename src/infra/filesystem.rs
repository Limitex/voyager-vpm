use crate::error::{Error, Result};
use serde::{Serialize, de::DeserializeOwned};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{debug, instrument};

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_path_for(path: &Path) -> std::path::PathBuf {
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut temp_name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("voyager"));
    temp_name.push(format!(".{}.{}.tmp", std::process::id(), counter));
    path.with_file_name(temp_name)
}

fn parent_dir_for_fs_ops(path: &Path) -> Option<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
}

#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = parent_dir_for_fs_ops(path) {
        let dir = fs::File::open(parent)?;
        dir.sync_all()?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

pub(crate) fn write_atomic_file(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = parent_dir_for_fs_ops(path) {
        fs::create_dir_all(parent)?;
    }

    let temp_path = temp_path_for(path);

    let mut file = fs::File::create(&temp_path)?;
    file.write_all(content.as_bytes())?;
    file.sync_all()?;
    drop(file);

    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }

    fs::rename(&temp_path, path)?;
    sync_parent_dir(path)?;

    Ok(())
}

pub(crate) fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
        sync_parent_dir(path)?;
    }
    Ok(())
}

pub(crate) fn read_to_string_if_exists(path: &Path) -> std::io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[instrument(skip_all, fields(path = %path.as_ref().display()))]
pub fn read_json<T, P>(path: P) -> Result<T>
where
    T: DeserializeOwned,
    P: AsRef<Path>,
{
    let path = path.as_ref();
    let path_str = path.display().to_string();

    let content = fs::read_to_string(path).map_err(|e| Error::FileRead {
        path: path_str.clone(),
        source: e,
    })?;

    let data: T = serde_json::from_str(&content).map_err(|e| Error::JsonParse {
        source: path_str,
        error: e,
    })?;

    debug!("Successfully read JSON file");
    Ok(data)
}

#[instrument(skip(data), fields(path = %path.as_ref().display()))]
pub fn write_json<T, P>(path: P, data: &T) -> Result<()>
where
    T: Serialize,
    P: AsRef<Path>,
{
    let path = path.as_ref();
    let path_str = path.display().to_string();

    let json = serde_json::to_string_pretty(data).map_err(Error::JsonSerialize)?;

    write_atomic_file(path, &json).map_err(|e| Error::OutputWrite {
        path: path_str,
        source: e,
    })?;

    debug!("Successfully wrote JSON file");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::io::Write;
    use tempfile::{NamedTempFile, tempdir};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestData {
        name: String,
        value: i32,
    }

    mod read_json_tests {
        use super::*;

        #[test]
        fn reads_valid_json_file() {
            let mut file = NamedTempFile::new().unwrap();
            writeln!(file, r#"{{"name": "test", "value": 42}}"#).unwrap();

            let data: TestData = read_json(file.path()).unwrap();

            assert_eq!(data.name, "test");
            assert_eq!(data.value, 42);
        }

        #[test]
        fn reads_json_with_whitespace() {
            let mut file = NamedTempFile::new().unwrap();
            writeln!(
                file,
                r#"{{
                    "name": "test",
                    "value": 42
                }}"#
            )
            .unwrap();

            let data: TestData = read_json(file.path()).unwrap();

            assert_eq!(data.name, "test");
            assert_eq!(data.value, 42);
        }

        #[test]
        fn fails_on_missing_file() {
            let result: Result<TestData> = read_json("/nonexistent/file.json");
            assert!(matches!(result, Err(Error::FileRead { .. })));
        }

        #[test]
        fn fails_on_invalid_json() {
            let mut file = NamedTempFile::new().unwrap();
            writeln!(file, "not valid json").unwrap();

            let result: Result<TestData> = read_json(file.path());
            assert!(matches!(result, Err(Error::JsonParse { .. })));
        }

        #[test]
        fn fails_on_wrong_schema() {
            let mut file = NamedTempFile::new().unwrap();
            writeln!(file, r#"{{"wrong": "schema"}}"#).unwrap();

            let result: Result<TestData> = read_json(file.path());
            assert!(matches!(result, Err(Error::JsonParse { .. })));
        }

        #[test]
        fn fails_on_empty_file() {
            let file = NamedTempFile::new().unwrap();

            let result: Result<TestData> = read_json(file.path());
            assert!(matches!(result, Err(Error::JsonParse { .. })));
        }

        #[test]
        fn fails_on_partial_json() {
            let mut file = NamedTempFile::new().unwrap();
            writeln!(file, r#"{{"name": "test""#).unwrap();

            let result: Result<TestData> = read_json(file.path());
            assert!(matches!(result, Err(Error::JsonParse { .. })));
        }
    }

    mod write_json_tests {
        use super::*;

        #[test]
        fn writes_json_to_file() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("output.json");

            let data = TestData {
                name: "test".to_string(),
                value: 42,
            };
            write_json(&path, &data).unwrap();

            let content = std::fs::read_to_string(&path).unwrap();
            assert!(content.contains("\"name\": \"test\""));
            assert!(content.contains("\"value\": 42"));
        }

        #[test]
        fn creates_parent_directories() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("nested/dirs/output.json");

            let data = TestData {
                name: "test".to_string(),
                value: 42,
            };
            write_json(&path, &data).unwrap();

            assert!(path.exists());
        }

        #[test]
        fn writes_pretty_formatted_json() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("output.json");

            let data = TestData {
                name: "test".to_string(),
                value: 42,
            };
            write_json(&path, &data).unwrap();

            let content = std::fs::read_to_string(&path).unwrap();
            assert!(content.contains('\n'));
        }

        #[test]
        fn overwrites_existing_file() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("output.json");

            let data1 = TestData {
                name: "first".to_string(),
                value: 1,
            };
            write_json(&path, &data1).unwrap();

            let data2 = TestData {
                name: "second".to_string(),
                value: 2,
            };
            write_json(&path, &data2).unwrap();

            let content = std::fs::read_to_string(&path).unwrap();
            assert!(content.contains("\"name\": \"second\""));
            assert!(!content.contains("\"name\": \"first\""));
        }

        #[test]
        fn roundtrip_preserves_data() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("output.json");

            let original = TestData {
                name: "test".to_string(),
                value: 42,
            };
            write_json(&path, &original).unwrap();

            let loaded: TestData = read_json(&path).unwrap();
            assert_eq!(original, loaded);
        }
    }
}
