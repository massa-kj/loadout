//! Filesystem and serialization I/O primitives.
//!
//! This crate provides platform-agnostic helpers for reading YAML/JSON configuration files
//! and writing state atomically. It has no knowledge of domain types — callers supply them
//! via generics.
//!
//! See: `docs/architecture/layers.md` (io layer)

use std::path::Path;

use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

/// Errors returned by I/O operations.
#[derive(Debug, Error)]
pub enum IoError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    ParseYaml {
        path: std::path::PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("failed to parse {path}: {source}")]
    ParseJson {
        path: std::path::PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize: {source}")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to rename {from} to {to}: {source}")]
    Rename {
        from: std::path::PathBuf,
        to: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Load and deserialize a YAML file.
pub fn load_yaml<T: DeserializeOwned>(path: &Path) -> Result<T, IoError> {
    let content = std::fs::read_to_string(path).map_err(|e| IoError::Read {
        path: path.to_path_buf(),
        source: e,
    })?;
    serde_yaml::from_str(&content).map_err(|e| IoError::ParseYaml {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Load and deserialize a JSON file.
pub fn load_json<T: DeserializeOwned>(path: &Path) -> Result<T, IoError> {
    let content = std::fs::read_to_string(path).map_err(|e| IoError::Read {
        path: path.to_path_buf(),
        source: e,
    })?;
    serde_json::from_str(&content).map_err(|e| IoError::ParseJson {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Write a value as pretty-printed JSON atomically.
///
/// Writes to `<path>.tmp`, then renames to `<path>`, ensuring readers never observe
/// a partially written file. Parent directories are created if they do not exist.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), IoError> {
    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        make_dirs(parent)?;
    }

    let serialized = serde_json::to_string_pretty(value).map_err(|e| IoError::Serialize { source: e })?;

    let tmp_path = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));

    std::fs::write(&tmp_path, &serialized).map_err(|e| IoError::Write {
        path: tmp_path.clone(),
        source: e,
    })?;

    std::fs::rename(&tmp_path, path).map_err(|e| IoError::Rename {
        from: tmp_path,
        to: path.to_path_buf(),
        source: e,
    })?;

    Ok(())
}

/// Copy a file from `src` to `dst`, creating parent directories for `dst` as needed.
pub fn copy_file(src: &Path, dst: &Path) -> Result<(), IoError> {
    if let Some(parent) = dst.parent() {
        make_dirs(parent)?;
    }
    std::fs::copy(src, dst).map_err(|e| IoError::Read {
        path: src.to_path_buf(),
        source: e,
    })?;
    Ok(())
}

/// Create a directory and all its parents, equivalent to `mkdir -p`.
pub fn make_dirs(path: &Path) -> Result<(), IoError> {
    std::fs::create_dir_all(path).map_err(|e| IoError::Write {
        path: path.to_path_buf(),
        source: e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // --- load_yaml ---

    #[test]
    fn load_yaml_simple_map() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.yaml");
        std::fs::write(&path, "key: value\n").unwrap();

        let m: HashMap<String, String> = load_yaml(&path).unwrap();
        assert_eq!(m["key"], "value");
    }

    #[test]
    fn load_yaml_missing_file() {
        let err = load_yaml::<HashMap<String, String>>(Path::new("/nonexistent/path.yaml"));
        assert!(matches!(err, Err(IoError::Read { .. })));
    }

    #[test]
    fn load_yaml_invalid_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.yaml");
        // Invalid YAML: unclosed brace
        std::fs::write(&path, "key: {\n").unwrap();
        let err = load_yaml::<HashMap<String, String>>(&path);
        assert!(matches!(err, Err(IoError::ParseYaml { .. })));
    }

    // --- load_json ---

    #[test]
    fn load_json_simple_object() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.json");
        std::fs::write(&path, r#"{"key": "value"}"#).unwrap();

        let m: HashMap<String, String> = load_json(&path).unwrap();
        assert_eq!(m["key"], "value");
    }

    #[test]
    fn load_json_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "{not valid json}").unwrap();
        let err = load_json::<HashMap<String, String>>(&path);
        assert!(matches!(err, Err(IoError::ParseJson { .. })));
    }

    // --- write_json_atomic ---

    #[test]
    fn write_json_atomic_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");

        let value: HashMap<String, i32> = [("x".into(), 42)].into_iter().collect();
        write_json_atomic(&path, &value).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("\"x\""));
        assert!(content.contains("42"));
    }

    #[test]
    fn write_json_atomic_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("dir").join("out.json");

        let value: Vec<i32> = vec![1, 2, 3];
        write_json_atomic(&path, &value).unwrap();

        assert!(path.exists());
    }

    #[test]
    fn write_json_atomic_no_tmp_file_left() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");

        let value: Vec<String> = vec!["a".into()];
        write_json_atomic(&path, &value).unwrap();

        // Tmp file must be cleaned up after success.
        let tmp = path.with_extension("json.tmp");
        assert!(!tmp.exists(), "tmp file should not remain after atomic write");
    }

    // --- copy_file ---

    #[test]
    fn copy_file_basic() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.txt");
        let dst = dir.path().join("dst.txt");
        std::fs::write(&src, "hello").unwrap();

        copy_file(&src, &dst).unwrap();

        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "hello");
    }

    #[test]
    fn copy_file_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.txt");
        let dst = dir.path().join("sub").join("dir").join("dst.txt");
        std::fs::write(&src, "data").unwrap();

        copy_file(&src, &dst).unwrap();

        assert!(dst.exists());
    }

    // --- make_dirs ---

    #[test]
    fn make_dirs_creates_nested() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a").join("b").join("c");

        make_dirs(&target).unwrap();

        assert!(target.is_dir());
    }

    #[test]
    fn make_dirs_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("existing");
        std::fs::create_dir_all(&target).unwrap();

        // Should not error if directory already exists.
        make_dirs(&target).unwrap();
    }
}
