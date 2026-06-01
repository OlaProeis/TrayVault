//! Clipboard history data types (`ClipEntry`, `EntryKind`, `ImageRef`).
//!
//! See `trayvault-prd.md` § Data Model for the authoritative schema.

/// A single captured clipboard item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipEntry {
    pub id: u64,
    /// Unix epoch milliseconds.
    pub created_at: u64,
    pub kind: EntryKind,
    /// Plain text, or a plain-text preview for rich-text entries.
    pub text: Option<String>,
    /// Raw HTML for [`EntryKind::RichText`] entries.
    pub html: Option<String>,
    /// Image metadata; pixel bytes live in [`Self::image_pixels`] briefly after capture, then on disk under `blobs/`.
    pub image: Option<ImageRef>,
    /// Decoded BGRA pixels until the blob persist job is enqueued; cleared afterward (see `App::on_clipboard_captured`).
    pub image_pixels: Option<Vec<u8>>,
    /// Best-effort foreground process executable name at capture time.
    pub source_app: Option<String>,
    pub is_pinned: bool,
    /// SHA-256 of normalized content (see `hash::hash_clip_entry`).
    pub hash: [u8; 32],
}

/// Kind of clipboard content captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Text,
    RichText,
    Image,
}

/// Metadata for an image entry; blob filename equals `hash` (Task 5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRef {
    /// Hex SHA-256; also the blob filename under `blobs/`.
    pub hash: String,
    pub width: u32,
    pub height: u32,
}

impl ClipEntry {
    /// Current Unix epoch milliseconds.
    pub fn now_millis() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}
