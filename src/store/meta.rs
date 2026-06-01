//! Line-oriented `entries.dat` metadata: serialize, parse, atomic replace via `MoveFileExW`.
//!
//! Schema (tab-separated):
//! - Header: `version\t1`
//! - Entry: `id`, `created_at`, `kind`, `text`, `html`, `image_hash`, `image_w`,
//!   `image_h`, `source_app`, `is_pinned`, `hash_hex`

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{ClipError, Result};
use crate::hash::{hash_to_hex, hex_to_hash};
use crate::log;
use crate::models::{ClipEntry, EntryKind, ImageRef};
use crate::win32::ffi::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
use crate::win32::{last_error, wide};

pub const METADATA_VERSION: u32 = 1;
const ENTRIES_FILE: &str = "entries.dat";
const ENTRIES_TMP: &str = "entries.dat.tmp";
const ENTRIES_BAK: &str = "entries.dat.bak";

/// Result of loading metadata from disk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadResult {
    pub entries: Vec<ClipEntry>,
    pub next_id: u64,
}

/// Serialize entries to line-oriented text (including version header).
pub fn serialize_entries(entries: &[ClipEntry]) -> String {
    let mut lines = Vec::with_capacity(entries.len() + 1);
    lines.push(format!("version\t{METADATA_VERSION}"));
    for entry in entries {
        lines.push(serialize_entry_line(entry));
    }
    lines.join("\n")
}

fn serialize_entry_line(entry: &ClipEntry) -> String {
    let kind = kind_to_str(entry.kind);
    let text = escape_field(entry.text.as_deref().unwrap_or(""));
    let html = escape_field(entry.html.as_deref().unwrap_or(""));
    let (image_hash, image_w, image_h) = match &entry.image {
        Some(img) => (
            escape_field(&img.hash),
            img.width.to_string(),
            img.height.to_string(),
        ),
        None => (String::new(), String::new(), String::new()),
    };
    let source_app = escape_field(entry.source_app.as_deref().unwrap_or(""));
    let is_pinned = if entry.is_pinned { "1" } else { "0" };
    let hash_hex = hash_to_hex(entry.hash);

    [
        entry.id.to_string(),
        entry.created_at.to_string(),
        kind.to_string(),
        text,
        html,
        image_hash,
        image_w,
        image_h,
        source_app,
        is_pinned.to_string(),
        hash_hex,
    ]
    .join("\t")
}

/// Parse metadata text into entries. Returns `Err` on unknown version or malformed lines.
pub fn parse_entries(text: &str) -> Result<Vec<ClipEntry>> {
    let mut lines = text.lines();
    let header = lines
        .next()
        .ok_or_else(|| ClipError::Other("metadata file is empty".into()))?;

    let version = parse_version_header(header)?;
    if version != METADATA_VERSION {
        return Err(ClipError::Other(format!(
            "unsupported metadata version {version}"
        )));
    }

    let mut entries = Vec::new();
    for (line_no, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        entries.push(
            parse_entry_line(line)
                .map_err(|e| ClipError::Other(format!("metadata line {}: {e}", line_no + 2)))?,
        );
    }
    Ok(entries)
}

fn parse_version_header(header: &str) -> Result<u32> {
    let mut parts = header.split('\t');
    let key = parts
        .next()
        .ok_or_else(|| ClipError::Other("missing version header".into()))?;
    if key != "version" {
        return Err(ClipError::Other("expected version header line".into()));
    }
    let value = parts
        .next()
        .ok_or_else(|| ClipError::Other("missing version number".into()))?;
    value
        .parse::<u32>()
        .map_err(|_| ClipError::Other("invalid version number".into()))
}

fn parse_entry_line(line: &str) -> Result<ClipEntry> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != 11 {
        return Err(ClipError::Other(format!(
            "expected 11 tab-separated fields, got {}",
            fields.len()
        )));
    }

    let id = fields[0]
        .parse::<u64>()
        .map_err(|_| ClipError::Other("invalid id".into()))?;
    let created_at = fields[1]
        .parse::<u64>()
        .map_err(|_| ClipError::Other("invalid created_at".into()))?;
    let kind = parse_kind(fields[2])?;

    let text = optional_text_field(fields[3])?;
    let html = optional_text_field(fields[4])?;

    let image_hash_raw = unescape_field(fields[5])?;
    let image = if image_hash_raw.is_empty() {
        None
    } else {
        let width = fields[6]
            .parse::<u32>()
            .map_err(|_| ClipError::Other("invalid image width".into()))?;
        let height = fields[7]
            .parse::<u32>()
            .map_err(|_| ClipError::Other("invalid image height".into()))?;
        Some(ImageRef {
            hash: image_hash_raw,
            width,
            height,
        })
    };

    let source_app = optional_text_field(fields[8])?;
    let is_pinned = match fields[9] {
        "0" => false,
        "1" => true,
        other => {
            return Err(ClipError::Other(format!(
                "invalid is_pinned value `{other}`"
            )));
        }
    };

    let hash =
        hex_to_hash(fields[10]).ok_or_else(|| ClipError::Other("invalid hash_hex".into()))?;

    Ok(ClipEntry {
        id,
        created_at,
        kind,
        text,
        html,
        image,
        image_pixels: None,
        source_app,
        is_pinned,
        hash,
    })
}

fn optional_text_field(raw: &str) -> Result<Option<String>> {
    let value = unescape_field(raw)?;
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

fn parse_kind(raw: &str) -> Result<EntryKind> {
    match raw {
        "Text" => Ok(EntryKind::Text),
        "RichText" => Ok(EntryKind::RichText),
        "Image" => Ok(EntryKind::Image),
        other => Err(ClipError::Other(format!("unknown entry kind `{other}`"))),
    }
}

fn kind_to_str(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Text => "Text",
        EntryKind::RichText => "RichText",
        EntryKind::Image => "Image",
    }
}

/// Escape tabs, newlines, and backslashes for tab-separated text fields.
pub fn escape_field(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\\n");
            }
            other => out.push(other),
        }
    }
    out
}

/// Reverse [`escape_field`].
pub fn unescape_field(value: &str) -> Result<String> {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    return Err(ClipError::Other(format!(
                        "invalid escape sequence `\\{other}`"
                    )));
                }
                None => return Err(ClipError::Other("dangling backslash".into())),
            }
        } else {
            out.push(ch);
        }
    }
    Ok(out)
}

pub fn entries_path(data_dir: &Path) -> PathBuf {
    data_dir.join(ENTRIES_FILE)
}

fn entries_tmp_path(data_dir: &Path) -> PathBuf {
    data_dir.join(ENTRIES_TMP)
}

fn entries_bak_path(data_dir: &Path) -> PathBuf {
    data_dir.join(ENTRIES_BAK)
}

/// Load metadata from `entries.dat`. On failure, back up the file and return empty history.
pub fn load_entries(data_dir: &Path) -> LoadResult {
    let path = entries_path(data_dir);
    if !path.exists() {
        return LoadResult::default();
    }

    match fs::read_to_string(&path) {
        Ok(text) => match parse_entries(&text) {
            Ok(entries) => LoadResult {
                next_id: compute_next_id(&entries),
                entries,
            },
            Err(err) => {
                log::warn(&format!(
                    "metadata load failed ({err}); backing up and starting fresh"
                ));
                let _ = backup_entries_file(data_dir);
                LoadResult::default()
            }
        },
        Err(err) => {
            log::warn(&format!(
                "metadata read failed ({err}); backing up and starting fresh"
            ));
            let _ = backup_entries_file(data_dir);
            LoadResult::default()
        }
    }
}

fn compute_next_id(entries: &[ClipEntry]) -> u64 {
    entries
        .iter()
        .map(|e| e.id)
        .max()
        .map(|max_id| max_id.saturating_add(1))
        .unwrap_or(0)
}

fn backup_entries_file(data_dir: &Path) -> Result<()> {
    let src = entries_path(data_dir);
    if !src.exists() {
        return Ok(());
    }
    let dst = entries_bak_path(data_dir);
    if dst.exists() {
        let _ = fs::remove_file(&dst);
    }
    fs::rename(&src, &dst)?;
    Ok(())
}

/// Atomically rewrite `entries.dat` via a temp file + `sync_all` + `MoveFileExW`.
pub fn write_entries_atomic(data_dir: &Path, entries: &[ClipEntry]) -> Result<()> {
    fs::create_dir_all(data_dir)?;

    let tmp_path = entries_tmp_path(data_dir);
    let final_path = entries_path(data_dir);
    let payload = serialize_entries(entries);

    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)?;
        file.write_all(payload.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }

    let tmp_w = wide(&tmp_path.to_string_lossy());
    let final_w = wide(&final_path.to_string_lossy());
    let flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH;
    // SAFETY: `tmp_w` and `final_w` are NUL-terminated UTF-16 paths alive for the call.
    let ok = unsafe { MoveFileExW(tmp_w.as_ptr(), final_w.as_ptr(), flags) };
    if ok == 0 {
        return Err(last_error("MoveFileExW"));
    }
    Ok(())
}

/// Collect image blob hashes referenced by the given entries.
pub fn referenced_blob_hashes(entries: &[ClipEntry]) -> Vec<String> {
    let mut hashes = Vec::new();
    for entry in entries {
        if let Some(image) = &entry.image {
            if !image.hash.is_empty() {
                hashes.push(image.hash.clone());
            }
        }
    }
    hashes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::{hash_text, hash_to_hex};

    fn sample_text_entry() -> ClipEntry {
        ClipEntry {
            id: 1,
            created_at: 1_700_000_000_000,
            kind: EntryKind::Text,
            text: Some("hello\tworld\nline\\two".into()),
            html: None,
            image: None,
            image_pixels: None,
            source_app: Some("notepad.exe".into()),
            is_pinned: true,
            hash: hash_text("hello\tworld\nline\\two"),
        }
    }

    #[test]
    fn escape_unescape_round_trip() {
        let samples = [
            "",
            "plain",
            "tab\there",
            "line\nbreak",
            "back\\slash",
            "a\r\nb",
        ];
        for sample in samples {
            let escaped = escape_field(sample);
            let restored = unescape_field(&escaped).expect("unescape");
            let normalized = sample.replace("\r\n", "\n").replace('\r', "\n");
            assert_eq!(restored, normalized, "sample={sample:?}");
        }
    }

    #[test]
    fn serialize_parse_round_trip() {
        let mut entries = vec![sample_text_entry()];
        entries.push(ClipEntry {
            id: 2,
            created_at: 1_700_000_000_001,
            kind: EntryKind::RichText,
            text: Some("preview".into()),
            html: Some("<b>bold</b>".into()),
            image: None,
            image_pixels: None,
            source_app: None,
            is_pinned: false,
            hash: hash_text("preview"),
        });
        let digest = hash_text("pixels");
        entries.push(ClipEntry {
            id: 3,
            created_at: 1_700_000_000_002,
            kind: EntryKind::Image,
            text: None,
            html: None,
            image: Some(ImageRef {
                hash: hash_to_hex(digest),
                width: 10,
                height: 20,
            }),
            image_pixels: None,
            source_app: Some("paint.exe".into()),
            is_pinned: false,
            hash: digest,
        });

        let text = serialize_entries(&entries);
        let parsed = parse_entries(&text).expect("parse");
        assert_eq!(parsed, entries);
    }

    #[test]
    fn corrupted_metadata_backs_up_and_starts_empty() {
        let dir = std::env::temp_dir().join(format!("trayvault-meta-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        fs::write(entries_path(&dir), "not-a-valid-header\n").expect("write");

        let loaded = load_entries(&dir);
        assert!(loaded.entries.is_empty());
        assert_eq!(loaded.next_id, 0);
        assert!(!entries_path(&dir).exists());
        assert!(entries_bak_path(&dir).exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_reload_equality() {
        let dir = std::env::temp_dir().join(format!("trayvault-meta-write-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let entries = vec![sample_text_entry()];
        write_entries_atomic(&dir, &entries).expect("write");
        let loaded = load_entries(&dir);
        assert_eq!(loaded.entries, entries);
        assert_eq!(loaded.next_id, 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_overwrites_existing() {
        let dir =
            std::env::temp_dir().join(format!("trayvault-meta-overwrite-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let first = vec![sample_text_entry()];
        write_entries_atomic(&dir, &first).expect("first write");
        let loaded_first = load_entries(&dir);
        assert_eq!(loaded_first.entries, first);

        let second = vec![ClipEntry {
            id: 2,
            created_at: 1_700_000_000_100,
            kind: EntryKind::Text,
            text: Some("replaced".into()),
            html: None,
            image: None,
            image_pixels: None,
            source_app: None,
            is_pinned: false,
            hash: hash_text("replaced"),
        }];
        write_entries_atomic(&dir, &second).expect("second write");
        let loaded_second = load_entries(&dir);
        assert_eq!(loaded_second.entries, second);
        assert_eq!(loaded_second.next_id, 3);

        let _ = fs::remove_dir_all(&dir);
    }
}
