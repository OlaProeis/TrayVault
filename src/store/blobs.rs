//! Content-addressed image blob storage (`blobs/<hex-sha256>.dib`).
//!
//! New blobs use a `TVB1` header + WIC-encoded PNG/JPEG payload. Legacy installs
//! store raw top-down BGRA without a header; [`read_blob`] decodes both formats
//! and always returns decoded BGRA bytes.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::config::{BlobWriteConfig, ImageBlobCodec};
use crate::error::{ClipError, Result};
use crate::log;
use crate::win32::wic;

const BLOBS_DIR: &str = "blobs";
const TVB1_MAGIC: &[u8; 4] = b"TVB1";
const TVB1_VERSION: u8 = 1;
const TVB1_HEADER_LEN: usize = 8;

pub const CODEC_ID_RAW: u8 = 0;
pub const CODEC_ID_PNG: u8 = 1;
pub const CODEC_ID_JPEG: u8 = 2;

pub fn blobs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(BLOBS_DIR)
}

fn blob_path(data_dir: &Path, hash: &str) -> PathBuf {
    blobs_dir(data_dir).join(format!("{hash}.dib"))
}

fn is_tvb1_header(data: &[u8]) -> bool {
    data.len() >= TVB1_HEADER_LEN && data.starts_with(TVB1_MAGIC)
}

fn codec_id_for(config: &BlobWriteConfig) -> u8 {
    match config.codec {
        ImageBlobCodec::Png => CODEC_ID_PNG,
        ImageBlobCodec::Jpeg => CODEC_ID_JPEG,
    }
}

fn build_tvb1_header(codec_id: u8) -> [u8; TVB1_HEADER_LEN] {
    [
        TVB1_MAGIC[0],
        TVB1_MAGIC[1],
        TVB1_MAGIC[2],
        TVB1_MAGIC[3],
        TVB1_VERSION,
        codec_id,
        0,
        0,
    ]
}

fn validate_bgra_buffer(width: u32, height: u32, pixels: &[u8]) -> Result<()> {
    let expected = (width as u64)
        .checked_mul(height as u64)
        .and_then(|n| n.checked_mul(4));
    let Some(expected) = expected else {
        return Err(ClipError::Other("image dimensions overflow".into()));
    };
    if pixels.len() as u64 != expected {
        return Err(ClipError::Other(format!(
            "pixel buffer length {} does not match {width}x{height} BGRA",
            pixels.len()
        )));
    }
    Ok(())
}

fn write_blob_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp_path = path.with_extension("dib.tmp");
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(tmp_path, path)?;
    Ok(())
}

/// Write WIC-compressed pixels if the blob does not already exist.
///
/// Falls back to legacy raw BGRA when WIC encode is unavailable on this OS install.
pub fn write_blob(
    data_dir: &Path,
    hash: &str,
    width: u32,
    height: u32,
    pixels: &[u8],
    config: &BlobWriteConfig,
) -> Result<()> {
    if hash.is_empty() {
        return Err(ClipError::Other("empty blob hash".into()));
    }

    fs::create_dir_all(blobs_dir(data_dir))?;
    let path = blob_path(data_dir, hash);
    if path.exists() {
        return Ok(());
    }

    validate_bgra_buffer(width, height, pixels)?;

    let bytes = match wic::encode_bgra(width, height, pixels, config.codec, config.jpeg_quality) {
        Ok(payload) => {
            let codec_id = codec_id_for(config);
            let header = build_tvb1_header(codec_id);
            let mut out = Vec::with_capacity(TVB1_HEADER_LEN + payload.len());
            out.extend_from_slice(&header);
            out.extend_from_slice(&payload);
            out
        }
        Err(err) => {
            log::warn(&format!(
                "WIC encode failed for blob `{hash}`: {err}; writing raw BGRA fallback"
            ));
            pixels.to_vec()
        }
    };

    write_blob_bytes(&path, &bytes)
}

/// Load blob pixels on demand. Returns decoded top-down BGRA, or `None` when missing.
pub fn read_blob(data_dir: &Path, hash: &str, width: u32, height: u32) -> Option<Vec<u8>> {
    if hash.is_empty() {
        return None;
    }
    let path = blob_path(data_dir, hash);
    let mut file = File::open(path).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;

    if is_tvb1_header(&buf) {
        let codec_id = buf[5];
        let payload = &buf[TVB1_HEADER_LEN..];
        match decode_tvb1_payload(payload, codec_id, width, height) {
            Ok(pixels) => Some(pixels),
            Err(err) => {
                log::warn(&format!("blob decode failed for `{hash}`: {err}"));
                None
            }
        }
    } else {
        Some(buf)
    }
}

fn decode_tvb1_payload(payload: &[u8], codec_id: u8, width: u32, height: u32) -> Result<Vec<u8>> {
    match codec_id {
        CODEC_ID_PNG | CODEC_ID_JPEG => wic::decode_to_bgra(payload, width, height),
        CODEC_ID_RAW => Ok(payload.to_vec()),
        other => Err(ClipError::Other(format!(
            "unsupported TVB1 codec_id {other}"
        ))),
    }
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
    use crate::config::BlobWriteConfig;
    use crate::win32::wic;

    fn temp_dir(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("trayvault-blobs-{prefix}-{}", std::process::id()))
    }

    fn sample_pixels() -> (u32, u32, Vec<u8>) {
        let width = 2u32;
        let height = 2u32;
        let pixels = vec![
            10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255,
        ];
        (width, height, pixels)
    }

    fn require_wic_codecs() -> bool {
        if crate::win32::wic::wic_codecs_available() {
            true
        } else {
            log::warn("skip blob WIC test: PNG encoder unavailable on this system");
            false
        }
    }

    #[test]
    fn write_read_png_round_trip() {
        if !require_wic_codecs() {
            return;
        }
        let dir = temp_dir("png-rt");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let (width, height, pixels) = sample_pixels();
        let config = BlobWriteConfig {
            codec: ImageBlobCodec::Png,
            jpeg_quality: 90,
        };
        write_blob(&dir, "abc123", width, height, &pixels, &config).expect("write");
        let loaded = read_blob(&dir, "abc123", width, height).expect("read");
        assert_eq!(loaded, pixels);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_raw_dib_still_reads() {
        let dir = temp_dir("legacy");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let (width, height, pixels) = sample_pixels();
        fs::create_dir_all(blobs_dir(&dir)).expect("mkdir blobs");
        let path = blob_path(&dir, "legacy1");
        fs::write(&path, &pixels).expect("write raw");

        let loaded = read_blob(&dir, "legacy1", width, height).expect("read");
        assert_eq!(loaded, pixels);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_skips_existing_blob() {
        if !require_wic_codecs() {
            return;
        }
        let dir = temp_dir("skip");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let (width, height, pixels) = sample_pixels();
        let config = BlobWriteConfig {
            codec: ImageBlobCodec::Png,
            jpeg_quality: 90,
        };
        write_blob(&dir, "deadbeef", width, height, &pixels, &config).expect("first write");
        write_blob(&dir, "deadbeef", width, height, &[9, 9, 9, 9], &config).expect("second write");
        let loaded = read_blob(&dir, "deadbeef", width, height).expect("read");
        assert_eq!(loaded, pixels);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_orphaned_removes_unreferenced() {
        if !require_wic_codecs() {
            return;
        }
        let dir = temp_dir("prune");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let (width, height, pixels) = sample_pixels();
        let config = BlobWriteConfig {
            codec: ImageBlobCodec::Png,
            jpeg_quality: 90,
        };
        write_blob(&dir, "keep", width, height, &pixels, &config).expect("keep");
        write_blob(&dir, "drop", width, height, &pixels, &config).expect("drop");

        prune_orphaned_blobs(&dir, &[String::from("keep")]).expect("prune");
        assert!(read_blob(&dir, "keep", width, height).is_some());
        assert!(read_blob(&dir, "drop", width, height).is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wic_png_payload_round_trip_via_module() {
        if !require_wic_codecs() {
            return;
        }
        let (width, height, pixels) = sample_pixels();
        let encoded =
            wic::encode_bgra(width, height, &pixels, ImageBlobCodec::Png, 90).expect("encode");
        let decoded = wic::decode_to_bgra(&encoded, width, height).expect("decode");
        assert_eq!(decoded, pixels);
    }
}
