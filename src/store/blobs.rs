//! Content-addressed image blob storage (`blobs/<hex-sha256>.dib`).

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::error::{ClipError, Result};
use crate::log;

const BLOBS_DIR: &str = "blobs";

pub fn blobs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(BLOBS_DIR)
}

fn blob_path(data_dir: &Path, hash: &str) -> PathBuf {
    blobs_dir(data_dir).join(format!("{hash}.dib"))
}

/// Write raw BGRA pixels if the blob does not already exist.
pub fn write_blob(data_dir: &Path, hash: &str, pixels: &[u8]) -> Result<()> {
    if hash.is_empty() {
        return Err(ClipError::Other("empty blob hash".into()));
    }

    fs::create_dir_all(blobs_dir(data_dir))?;
    let path = blob_path(data_dir, hash);
    if path.exists() {
        return Ok(());
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(pixels)?;
    file.sync_all()?;
    Ok(())
}

/// Load blob pixels on demand. Returns `None` when the file is missing.
pub fn read_blob(data_dir: &Path, hash: &str) -> Option<Vec<u8>> {
    if hash.is_empty() {
        return None;
    }
    let path = blob_path(data_dir, hash);
    let mut file = File::open(path).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Delete a blob file if present. Failures are logged by the caller.
pub fn delete_blob(data_dir: &Path, hash: &str) -> Result<()> {
    if hash.is_empty() {
        return Ok(());
    }
    let path = blob_path(data_dir, hash);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// Remove blob files not referenced by any entry metadata hash.
pub fn prune_orphaned_blobs(data_dir: &Path, referenced: &[String]) -> Result<()> {
    let dir = blobs_dir(data_dir);
    if !dir.exists() {
        return Ok(());
    }

    let referenced_set: std::collections::HashSet<&str> =
        referenced.iter().map(String::as_str).collect();

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(hash) = name.strip_suffix(".dib") else {
            continue;
        };
        if !referenced_set.contains(hash) {
            if let Err(err) = fs::remove_file(&path) {
                log::warn(&format!("failed to delete orphaned blob `{name}`: {err}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("trayvault-blobs-{prefix}-{}", std::process::id()))
    }

    #[test]
    fn write_read_byte_equality() {
        let dir = temp_dir("rw");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let hash = "abc123";
        let pixels = vec![0u8, 1, 2, 3, 255];
        write_blob(&dir, hash, &pixels).expect("write");
        let loaded = read_blob(&dir, hash).expect("read");
        assert_eq!(loaded, pixels);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_skips_existing_blob() {
        let dir = temp_dir("skip");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let hash = "deadbeef";
        write_blob(&dir, hash, &[1, 2, 3]).expect("first write");
        write_blob(&dir, hash, &[9, 9, 9]).expect("second write");
        let loaded = read_blob(&dir, hash).expect("read");
        assert_eq!(loaded, vec![1, 2, 3]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_orphaned_removes_unreferenced() {
        let dir = temp_dir("prune");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        write_blob(&dir, "keep", &[1]).expect("keep");
        write_blob(&dir, "drop", &[2]).expect("drop");

        prune_orphaned_blobs(&dir, &[String::from("keep")]).expect("prune");
        assert!(read_blob(&dir, "keep").is_some());
        assert!(read_blob(&dir, "drop").is_none());

        let _ = fs::remove_dir_all(&dir);
    }
}
