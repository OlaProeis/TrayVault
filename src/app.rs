//! Central application state: history orchestration, cap pruning, and copy-back.
//!
//! Owns the in-memory entry list, coordinates persistence jobs, and applies config
//! (max entries, dedup, image size limits). Clipboard capture and write helpers
//! live in [`crate::win32::clipboard`].

#![allow(dead_code)] // public API for Tasks 8–12 (UI, settings, hotkey)

use std::collections::HashMap;

use crate::config::{BlobWriteConfig, Config};
use crate::error::{ClipError, Result};
use crate::hash::is_duplicate_entry;
use crate::log;
use crate::models::{ClipEntry, EntryKind};
use crate::store::{LoadResult, Store};
use crate::ui::thumb_loader::ThumbLoader;
use crate::win32::clipboard::{write_entry_to_clipboard, ClipboardMonitor};
use crate::win32::ffi::HWND;

/// Application orchestration layer wired into the Win32 message loop.
pub struct App {
    pub config: Config,
    pub entries: Vec<ClipEntry>,
    /// Bumped on entry-set / pin mutations; invalidates cached `display_indices`.
    pub entries_version: u64,
    pub selected_index: usize,
    pub filter_query: String,
    pub pause_capture: bool,
    store: Store,
    thumb_loader: ThumbLoader,
    hash_index: HashMap<[u8; 32], u32>,
    next_id: u64,
    window_visible: bool,
    needs_repaint: bool,
}

impl App {
    /// Bootstrap in-memory state from persisted metadata and config.
    pub fn new(config: Config, loaded: LoadResult, store: Store) -> Self {
        let mut entries = loaded.entries;
        ensure_most_recent_first(&mut entries);
        let hash_index = build_hash_index(&entries);
        let data_dir = store.data_dir_path().to_path_buf();
        let thumb_loader = ThumbLoader::new(data_dir);
        Self {
            pause_capture: config.pause_capture,
            config,
            entries,
            entries_version: 0,
            selected_index: 0,
            filter_query: String::new(),
            store,
            thumb_loader,
            hash_index,
            next_id: loaded.next_id,
            window_visible: false,
            needs_repaint: false,
        }
    }

    pub fn thumb_loader(&self) -> &ThumbLoader {
        &self.thumb_loader
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn is_window_visible(&self) -> bool {
        self.window_visible
    }

    pub fn set_needs_repaint(&mut self) {
        self.needs_repaint = true;
    }

    pub fn take_needs_repaint(&mut self) -> bool {
        let needed = self.needs_repaint;
        self.needs_repaint = false;
        needed
    }

    /// Snapshot of blob codec settings for the storage worker.
    fn blob_write_config(&self) -> BlobWriteConfig {
        BlobWriteConfig::from_config(&self.config)
    }

    /// Apply current config to the clipboard monitor's capture toggles.
    pub fn apply_capture_config(&self, monitor: &mut ClipboardMonitor) {
        monitor.set_config(self.config.capture_config());
    }

    /// Handle a captured clipboard entry: dedup, assign id, prune, persist.
    pub fn on_clipboard_captured(&mut self, entry: Option<ClipEntry>) {
        let Some(entry) = entry else {
            return;
        };

        if self.pause_capture || self.config.pause_capture {
            return;
        }

        if self.is_duplicate(&entry) {
            log::info("clipboard capture skipped (duplicate content)");
            return;
        }

        let mut entry = entry;
        entry.id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        add_hash(&mut self.hash_index, entry.hash);

        let kind = entry.kind;
        let id = entry.id;
        self.entries.insert(0, entry);
        if self.selected_index < self.entries.len().saturating_sub(1) {
            self.selected_index = self.selected_index.saturating_add(1);
        }

        let removed = prune_to_cap(&mut self.entries, self.config.max_entries);
        for removed_entry in &removed {
            remove_hash(&mut self.hash_index, removed_entry.hash);
            self.enqueue_removed_entry_delete(removed_entry);
        }

        self.store
            .enqueue_persist(&self.entries, self.blob_write_config());
        if kind == EntryKind::Image {
            Self::release_captured_image_pixels(&mut self.entries, id);
        }
        if !removed.is_empty() {
            self.store.enqueue_prune_orphans(&self.entries);
            log::info(&format!(
                "pruned {} entries to cap (max_entries={})",
                removed.len(),
                self.config.max_entries
            ));
        }

        self.bump_entries_version();
        self.needs_repaint = true;
        log::info(&format!("history entry added id={id} kind={kind:?}"));
    }

    /// Refresh relative-time labels; returns whether a repaint is warranted.
    pub fn on_timer_tick(&mut self) -> bool {
        if !self.window_visible || self.entries.is_empty() {
            return false;
        }
        self.needs_repaint = true;
        true
    }

    pub fn on_show_window(&mut self) {
        self.window_visible = true;
        self.needs_repaint = true;
    }

    pub fn on_hide_window(&mut self) {
        self.window_visible = false;
    }

    /// Set clipboard capture pause (tray menu / settings) and persist.
    pub fn set_pause_capture(&mut self, paused: bool) -> Result<()> {
        self.pause_capture = paused;
        self.config.pause_capture = paused;
        self.persist_config()?;
        log::info(&format!(
            "clipboard capture {}",
            if self.pause_capture {
                "paused"
            } else {
                "resumed"
            }
        ));
        Ok(())
    }

    /// Toggle clipboard capture pause (tray menu / settings).
    pub fn toggle_pause_capture(&mut self) -> Result<bool> {
        let paused = !self.pause_capture;
        self.set_pause_capture(paused)?;
        Ok(self.pause_capture)
    }

    /// Write the current config to the default path.
    pub fn persist_config(&self) -> Result<()> {
        self.config.save(&Config::default_path())
    }

    /// Update `max_entries`, prune history if needed, and persist.
    pub fn set_max_entries(&mut self, max_entries: u32) -> Result<()> {
        if max_entries == 0 {
            return Err(ClipError::Config(
                "max_entries must be greater than 0".into(),
            ));
        }
        self.config.max_entries = max_entries;
        let removed = prune_to_cap(&mut self.entries, max_entries);
        for removed_entry in &removed {
            remove_hash(&mut self.hash_index, removed_entry.hash);
            self.enqueue_removed_entry_delete(removed_entry);
        }
        if !removed.is_empty() {
            self.store
                .enqueue_persist(&self.entries, self.blob_write_config());
            self.store.enqueue_prune_orphans(&self.entries);
        }
        if self.selected_index >= self.entries.len() {
            self.selected_index = self.entries.len().saturating_sub(1);
        }
        if !removed.is_empty() {
            self.bump_entries_version();
        }
        self.persist_config()?;
        self.needs_repaint = true;
        Ok(())
    }

    /// Update a boolean config field and persist.
    pub fn set_config_bool<F>(&mut self, setter: F, value: bool) -> Result<()>
    where
        F: FnOnce(&mut Config) -> &mut bool,
    {
        *setter(&mut self.config) = value;
        self.persist_config()
    }

    /// Update theme and request a repaint.
    pub fn set_theme(&mut self, theme: crate::config::ThemeMode) -> Result<()> {
        self.config.theme = theme;
        self.persist_config()?;
        self.needs_repaint = true;
        Ok(())
    }

    /// Update hotkey string in config (caller registers with Win32).
    pub fn set_hotkey_string(&mut self, hotkey: String) -> Result<()> {
        if hotkey.trim().is_empty() {
            return Err(ClipError::Config("hotkey must not be empty".into()));
        }
        self.config.hotkey = hotkey;
        self.persist_config()
    }

    /// Update max image size (MB), persist, and return whether capture config changed.
    pub fn set_max_image_size_mb(&mut self, mb: f32) -> Result<()> {
        if !mb.is_finite() || mb <= 0.0 {
            return Err(ClipError::Config(
                "max_image_size_mb must be a positive finite number".into(),
            ));
        }
        self.config.max_image_size_mb = mb;
        self.persist_config()
    }

    /// Update image blob codec (new writes only) and persist.
    pub fn set_image_blob_codec(&mut self, codec: crate::config::ImageBlobCodec) -> Result<()> {
        self.config.image_blob_codec = codec;
        self.persist_config()
    }

    /// Update JPEG blob quality (1–100) and persist.
    pub fn set_jpeg_quality(&mut self, quality: u8) -> Result<()> {
        if !(1..=100).contains(&quality) {
            return Err(ClipError::Config(
                "jpeg_quality must be between 1 and 100".into(),
            ));
        }
        self.config.jpeg_quality = quality;
        self.persist_config()
    }

    /// Persist in-memory pause flag into config before exit.
    pub fn sync_config(&mut self) {
        self.config.pause_capture = self.pause_capture;
    }

    /// Toggle autostart in config and the Windows Run key.
    pub fn set_autostart(&mut self, enabled: bool, exe_path: &std::path::Path) -> Result<()> {
        crate::win32::autostart::set_autostart(enabled, exe_path)?;
        self.config.autostart = enabled;
        self.config.save(&Config::default_path())?;
        log::info(&format!("autostart setting saved (enabled={enabled})"));
        Ok(())
    }

    /// Capture window placement, save settings, and flush outstanding storage jobs.
    pub fn shutdown(
        &mut self,
        hwnd: crate::win32::ffi::HWND,
        config_path: &std::path::Path,
    ) -> Result<()> {
        self.sync_config();
        crate::win32::window::capture_geometry_into_config(hwnd, &mut self.config);
        self.config.save(config_path)?;
        self.store.flush(&self.entries, self.blob_write_config());
        Ok(())
    }

    /// Persist the current window position/size (after move/resize).
    pub fn persist_window_geometry(
        &mut self,
        hwnd: crate::win32::ffi::HWND,
        config_path: &std::path::Path,
    ) -> Result<()> {
        if crate::win32::window::capture_geometry_into_config(hwnd, &mut self.config) {
            self.config.save(config_path)?;
        }
        Ok(())
    }

    /// Join background worker threads after the message loop exits.
    pub fn join_storage(&mut self) {
        self.store.join();
        self.thumb_loader.join();
    }

    /// Toggle pin state for an entry and persist.
    pub fn toggle_pin(&mut self, entry_id: u64) {
        let pinned = {
            let Some(entry) = self.entries.iter_mut().find(|e| e.id == entry_id) else {
                return;
            };
            entry.is_pinned = !entry.is_pinned;
            entry.is_pinned
        };
        self.store
            .enqueue_persist(&self.entries, self.blob_write_config());
        self.bump_entries_version();
        self.needs_repaint = true;
        log::info(&format!("entry id={entry_id} pin={pinned}"));
    }

    /// Remove an entry from history and enqueue blob delete when applicable.
    pub fn delete_entry(&mut self, entry_id: u64) {
        let Some(pos) = self.entries.iter().position(|e| e.id == entry_id) else {
            return;
        };
        let removed = self.entries.remove(pos);
        remove_hash(&mut self.hash_index, removed.hash);
        self.enqueue_removed_entry_delete(&removed);
        self.store
            .enqueue_persist(&self.entries, self.blob_write_config());
        if self.selected_index >= self.entries.len() {
            self.selected_index = self.entries.len().saturating_sub(1);
        }
        self.bump_entries_version();
        self.needs_repaint = true;
        log::info(&format!("deleted entry id={entry_id}"));
    }

    /// Id of the currently selected entry, if any.
    pub fn selected_entry_id(&self) -> Option<u64> {
        self.entries.get(self.selected_index).map(|e| e.id)
    }

    /// Copy a history entry back to the system clipboard without self-capture.
    ///
    /// Returns `Ok(true)` when the caller should hide the window (`close_on_copy`).
    pub fn copy_entry_to_clipboard(
        &mut self,
        entry_id: u64,
        hwnd: HWND,
        monitor: &mut ClipboardMonitor,
    ) -> Result<bool> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.id == entry_id)
            .cloned()
            .ok_or_else(|| ClipError::Other(format!("entry id {entry_id} not found")))?;

        let image_pixels = Self::resolve_image_pixels(&entry, &self.store);

        monitor.mark_own_write();
        write_entry_to_clipboard(hwnd, &entry, image_pixels.as_deref())?;

        let hide = self.config.close_on_copy;
        if hide {
            self.on_hide_window();
        }

        log::info(&format!("copied entry id={entry_id} back to clipboard"));
        Ok(hide)
    }

    fn is_duplicate(&self, entry: &ClipEntry) -> bool {
        is_duplicate_entry(&self.entries, entry, false)
            || (self.config.deduplicate_global && self.hash_index.contains_key(&entry.hash))
    }

    /// True when any remaining in-memory entry still references this blob hash.
    fn blob_still_referenced(&self, hash_hex: &str) -> bool {
        self.entries
            .iter()
            .any(|e| e.image.as_ref().is_some_and(|i| i.hash == hash_hex))
    }

    /// Enqueue blob delete only when no surviving entry references the same hash.
    fn enqueue_removed_entry_delete(&self, removed: &ClipEntry) {
        if let Some(image) = &removed.image {
            let blob_hash = if self.blob_still_referenced(&image.hash) {
                None
            } else {
                Some(image.hash.clone())
            };
            self.store.enqueue_delete(removed.id, blob_hash);
        }
    }

    fn bump_entries_version(&mut self) {
        self.entries_version = self.entries_version.wrapping_add(1);
    }

    /// Drop in-memory image bytes after the persist job has been queued (worker owns a clone).
    fn release_captured_image_pixels(entries: &mut [ClipEntry], entry_id: u64) {
        let Some(entry) = entries.iter_mut().find(|e| e.id == entry_id) else {
            return;
        };
        if entry.kind == EntryKind::Image && entry.image.is_some() {
            entry.image_pixels = None;
        }
    }

    fn resolve_image_pixels(entry: &ClipEntry, store: &Store) -> Option<Vec<u8>> {
        if let Some(pixels) = entry.image_pixels.as_ref() {
            return Some(pixels.clone());
        }
        entry
            .image
            .as_ref()
            .and_then(|img| store.read_blob(&img.hash, img.width, img.height))
    }
}

/// Remove oldest non-pinned entries until `entries.len() <= max_entries`.
///
/// Returns removed entries (caller enqueues blob deletes). When every entry is
/// pinned and the cap is exceeded, no further removals occur.
pub fn prune_to_cap(entries: &mut Vec<ClipEntry>, max_entries: u32) -> Vec<ClipEntry> {
    let max = max_entries as usize;
    if entries.len() <= max {
        return Vec::new();
    }

    let mut removed = Vec::new();
    while entries.len() > max {
        let Some(idx) = entries.iter().rposition(|e| !e.is_pinned) else {
            break;
        };
        removed.push(entries.remove(idx));
    }
    removed
}

fn build_hash_index(entries: &[ClipEntry]) -> HashMap<[u8; 32], u32> {
    let mut index = HashMap::new();
    for entry in entries {
        add_hash(&mut index, entry.hash);
    }
    index
}

fn add_hash(index: &mut HashMap<[u8; 32], u32>, hash: [u8; 32]) {
    *index.entry(hash).or_insert(0) += 1;
}

fn remove_hash(index: &mut HashMap<[u8; 32], u32>, hash: [u8; 32]) {
    if let Some(count) = index.get_mut(&hash) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            index.remove(&hash);
        }
    }
}

fn ensure_most_recent_first(entries: &mut [ClipEntry]) {
    entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::{hash_text, hash_to_hex};
    use crate::models::{EntryKind, ImageRef};
    use std::fs;

    fn temp_data_dir(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("trayvault-app-{prefix}-{}", std::process::id()))
    }

    fn sample_entry(id: u64, pinned: bool, hash_byte: u8) -> ClipEntry {
        let mut hash = [0u8; 32];
        hash[0] = hash_byte;
        ClipEntry {
            id,
            created_at: id,
            kind: EntryKind::Text,
            text: Some(format!("entry-{id}")),
            html: None,
            image: None,
            image_pixels: None,
            source_app: None,
            is_pinned: pinned,
            hash,
        }
    }

    fn image_entry(pixels: &[u8], created_at: u64) -> ClipEntry {
        let digest = crate::hash::hash_image_pixels(pixels);
        ClipEntry {
            id: 0,
            created_at,
            kind: EntryKind::Image,
            text: None,
            html: None,
            image: Some(ImageRef {
                hash: hash_to_hex(digest),
                width: 1,
                height: 1,
            }),
            image_pixels: Some(pixels.to_vec()),
            source_app: None,
            is_pinned: false,
            hash: digest,
        }
    }

    fn text_entry(text: &str, created_at: u64) -> ClipEntry {
        ClipEntry {
            id: 0,
            created_at,
            kind: EntryKind::Text,
            text: Some(text.into()),
            html: None,
            image: None,
            image_pixels: None,
            source_app: None,
            is_pinned: false,
            hash: hash_text(text),
        }
    }

    fn clear_image_pixels(entries: &mut [ClipEntry]) {
        for entry in entries {
            if entry.kind == EntryKind::Image {
                entry.image_pixels = None;
            }
        }
    }

    #[test]
    fn prune_removes_oldest_unpinned_first() {
        let mut entries = vec![
            sample_entry(3, false, 3),
            sample_entry(2, false, 2),
            sample_entry(1, false, 1),
        ];
        let removed = prune_to_cap(&mut entries, 2);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, 3);
        assert_eq!(entries[1].id, 2);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].id, 1);
    }

    #[test]
    fn prune_skips_pinned_entries() {
        let mut entries = vec![
            sample_entry(4, false, 4),
            sample_entry(3, true, 3),
            sample_entry(2, false, 2),
            sample_entry(1, true, 1),
        ];
        let removed = prune_to_cap(&mut entries, 2);
        assert_eq!(removed.len(), 2);
        assert!(removed.iter().all(|e| !e.is_pinned));
        assert!(entries.iter().any(|e| e.id == 3 && e.is_pinned));
        assert!(entries.iter().any(|e| e.id == 1 && e.is_pinned));
    }

    #[test]
    fn prune_stops_when_all_pinned() {
        let mut entries = vec![sample_entry(2, true, 2), sample_entry(1, true, 1)];
        let removed = prune_to_cap(&mut entries, 1);
        assert!(removed.is_empty());
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn on_clipboard_captured_dedup_and_updates_hash_index() {
        let dir = temp_data_dir("dedup");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let config = Config {
            deduplicate_global: true,
            max_entries: 500,
            ..Config::default()
        };
        let store = Store::open_for_test(dir.clone());
        let mut app = App::new(config, LoadResult::default(), store);

        let hash = hash_text("hello");
        let entry = ClipEntry {
            id: 0,
            created_at: 1,
            kind: EntryKind::Text,
            text: Some("hello".into()),
            html: None,
            image: None,
            image_pixels: None,
            source_app: None,
            is_pinned: false,
            hash,
        };

        app.on_clipboard_captured(Some(entry.clone()));
        assert_eq!(app.entries.len(), 1);
        assert!(app.hash_index.contains_key(&hash));

        app.on_clipboard_captured(Some(entry));
        assert_eq!(app.entries.len(), 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn on_clipboard_captured_prunes_to_max_entries() {
        let dir = temp_data_dir("cap");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let config = Config {
            max_entries: 2,
            ..Config::default()
        };
        let store = Store::open_for_test(dir.clone());
        let mut app = App::new(config, LoadResult::default(), store);

        for i in 0..4u64 {
            let mut hash = [0u8; 32];
            hash[0] = i as u8 + 10;
            app.on_clipboard_captured(Some(ClipEntry {
                id: 0,
                created_at: i,
                kind: EntryKind::Text,
                text: Some(format!("t{i}")),
                html: None,
                image: None,
                image_pixels: None,
                source_app: None,
                is_pinned: false,
                hash,
            }));
        }

        assert_eq!(app.entries.len(), 2);
        assert_eq!(app.entries[0].text.as_deref(), Some("t3"));
        assert_eq!(app.entries[1].text.as_deref(), Some("t2"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_entry_hash_index_tracks_blob_hash() {
        let dir = temp_data_dir("image-hash");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let pixels = vec![1u8, 2, 3, 4];
        let digest = crate::hash::hash_image_pixels(&pixels);
        let store = Store::open_for_test(dir.clone());
        let mut app = App::new(Config::default(), LoadResult::default(), store);

        app.on_clipboard_captured(Some(ClipEntry {
            id: 0,
            created_at: 1,
            kind: EntryKind::Image,
            text: None,
            html: None,
            image: Some(ImageRef {
                hash: hash_to_hex(digest),
                width: 1,
                height: 1,
            }),
            image_pixels: Some(pixels),
            source_app: None,
            is_pinned: false,
            hash: digest,
        }));

        assert_eq!(app.hash_index.get(&digest), Some(&1));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_entry_preserves_blob_when_hash_still_referenced() {
        if !crate::win32::wic::wic_codecs_available() {
            log::warn("skip shared-blob delete test: WIC unavailable");
            return;
        }
        let dir = temp_data_dir("shared-blob-delete");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let pixels = vec![10u8, 20, 30, 40];
        let hash_hex = hash_to_hex(crate::hash::hash_image_pixels(&pixels));
        let config = Config {
            deduplicate_global: false,
            ..Config::default()
        };
        let store = Store::open_for_test(dir.clone());
        let mut app = App::new(config, LoadResult::default(), store);

        app.on_clipboard_captured(Some(image_entry(&pixels, 1)));
        app.on_clipboard_captured(Some(text_entry("between", 2)));
        app.on_clipboard_captured(Some(image_entry(&pixels, 3)));
        assert_eq!(app.entries.len(), 3);

        let older_id = app.entries[2].id;
        let newer_id = app.entries[0].id;

        app.store().flush(&app.entries, app.blob_write_config());
        clear_image_pixels(&mut app.entries);

        app.delete_entry(newer_id);
        app.store().flush(&app.entries, app.blob_write_config());

        assert_eq!(app.store().read_blob(&hash_hex, 1, 1), Some(pixels.clone()));
        let surviving = app
            .entries
            .iter()
            .find(|e| e.id == older_id)
            .expect("surviving entry");
        assert_eq!(
            App::resolve_image_pixels(surviving, app.store()),
            Some(pixels)
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_pixels_cleared_after_capture_and_recoverable_via_blob() {
        if !crate::win32::wic::wic_codecs_available() {
            log::warn("skip release-pixels test: WIC unavailable");
            return;
        }
        let dir = temp_data_dir("release-pixels");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let pixels = vec![10u8, 20, 30, 40];
        let hash_hex = hash_to_hex(crate::hash::hash_image_pixels(&pixels));
        let store = Store::open_for_test(dir.clone());
        let mut app = App::new(Config::default(), LoadResult::default(), store);

        app.on_clipboard_captured(Some(image_entry(&pixels, 1)));
        assert_eq!(app.entries.len(), 1);
        assert!(app.entries[0].image_pixels.is_none());

        app.store().flush(&app.entries, app.blob_write_config());
        assert_eq!(app.store().read_blob(&hash_hex, 1, 1), Some(pixels.clone()));
        assert_eq!(
            App::resolve_image_pixels(&app.entries[0], app.store()),
            Some(pixels)
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_entry_removes_blob_when_unreferenced() {
        if !crate::win32::wic::wic_codecs_available() {
            log::warn("skip single-blob delete test: WIC unavailable");
            return;
        }
        let dir = temp_data_dir("single-blob-delete");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let pixels = vec![10u8, 20, 30, 40];
        let hash_hex = hash_to_hex(crate::hash::hash_image_pixels(&pixels));
        let store = Store::open_for_test(dir.clone());
        let mut app = App::new(Config::default(), LoadResult::default(), store);

        app.on_clipboard_captured(Some(image_entry(&pixels, 1)));
        app.store().flush(&app.entries, app.blob_write_config());
        let id = app.entries[0].id;
        clear_image_pixels(&mut app.entries);

        app.delete_entry(id);
        app.store().flush(&app.entries, app.blob_write_config());

        assert!(app.store().read_blob(&hash_hex, 1, 1).is_none());

        let _ = fs::remove_dir_all(&dir);
    }
}
