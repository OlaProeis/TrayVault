//! Immediate-mode UI: hand-rolled pixmap rasterization, GDI text, themes, and views.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub mod history;
pub mod input;
pub mod pixmap;
pub mod preview;
pub mod render;
pub mod scroll_bar;
pub mod search;
pub mod search_edit;
pub mod settings;
pub mod settings_input;
pub mod text;
pub mod theme;
pub mod thumb_cache;
pub mod thumb_loader;
pub mod titlebar;
pub mod widgets;

use crate::app::App;
use crate::config::Config;
use crate::models::ClipEntry;
use crate::ui::pixmap::Pixmap;
use crate::ui::preview::PreviewImageCache;
use crate::ui::text::GlyphCache;
use crate::ui::thumb_cache::ThumbCache;
use crate::ui::widgets::WidgetRect;

/// Tracks in-flight async thumbnail loads (keyed by `(entry_id, dst_w, dst_h)`).
#[derive(Debug, Default)]
pub struct ThumbLoadState {
    pub inflight: HashSet<(u64, u32, u32)>,
    pub generation: u64,
}

impl ThumbLoadState {
    /// Invalidate pending loads after window resize clears the thumb cache bucket.
    pub fn reset_on_width_change(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.inflight.clear();
    }
}

/// Filter chip selection for the history list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EntryFilter {
    #[default]
    All,
    Text,
    Images,
    Pinned,
}

impl EntryFilter {
    pub const ALL: [EntryFilter; 4] = [
        EntryFilter::All,
        EntryFilter::Text,
        EntryFilter::Images,
        EntryFilter::Pinned,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Text => "Text",
            Self::Images => "Images",
            Self::Pinned => "Pinned",
        }
    }
}

/// Which settings text field has focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsFocus {
    #[default]
    None,
    MaxEntries,
    Hotkey,
    MaxImageSizeMb,
    JpegQuality,
}

/// Per-control hit targets for the settings panel (filled each paint).
#[derive(Clone, Debug, Default)]
pub struct SettingsRects {
    pub back: Option<WidgetRect>,
    pub pause: Option<WidgetRect>,
    pub max_entries: Option<WidgetRect>,
    pub deduplicate: Option<WidgetRect>,
    pub hotkey: Option<WidgetRect>,
    pub theme_light: Option<WidgetRect>,
    pub theme_dark: Option<WidgetRect>,
    pub theme_system: Option<WidgetRect>,
    pub capture_images: Option<WidgetRect>,
    pub capture_rich_text: Option<WidgetRect>,
    pub close_on_copy: Option<WidgetRect>,
    pub show_in_taskbar: Option<WidgetRect>,
    pub max_image_mb: Option<WidgetRect>,
    pub blob_codec_png: Option<WidgetRect>,
    pub blob_codec_jpeg: Option<WidgetRect>,
    pub jpeg_quality: Option<WidgetRect>,
    pub autostart: Option<WidgetRect>,
    pub github: Option<WidgetRect>,
}

/// Compact hover identity for repaint elision on `MouseMove`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HoverKey {
    pub entry_index: Option<usize>,
    pub filter_chip: u8,
    pub settings: bool,
    pub close: bool,
}

/// Right-click context menu state.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextMenu {
    pub entry_id: u64,
    pub x: f32,
    pub y: f32,
}

/// Inputs that determine filtered display order (`App::entries_version`, filter, query).
#[derive(Clone, Debug, PartialEq, Eq)]
struct DisplayIndicesKey {
    entries_version: u64,
    filter: EntryFilter,
    query: String,
}

/// Inputs that determine per-row layout heights (`build_list_layout` cache key).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListLayoutKey {
    entries_version: u64,
    filter: EntryFilter,
    query: String,
    expanded_version: u64,
    thumb_max_w_bucket: u32,
}

/// Per-entry inputs that determine a text row's wrapped height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EntryHeightKey {
    content_hash: [u8; 32],
    expanded: bool,
    thumb_max_w_bucket: u32,
}

#[derive(Debug)]
struct CachedTextRowLayout {
    key: EntryHeightKey,
    height: f32,
    show_control: bool,
    draw_lines: Vec<String>,
}

/// Cached text-row layout keyed by entry id; cleared when `App::entries_version` changes.
#[derive(Debug, Default)]
pub(crate) struct EntryHeightCache {
    entries_version: u64,
    layouts: HashMap<u64, CachedTextRowLayout>,
    pub hit_count: u64,
    pub miss_count: u64,
}

impl EntryHeightCache {
    /// Drop all cached layouts when history metadata changes (capture, delete, pin, cap prune).
    pub fn sync_entries_version(&mut self, entries_version: u64) {
        if self.entries_version != entries_version {
            self.layouts.clear();
            self.entries_version = entries_version;
        }
    }

    pub fn get_layout(
        &mut self,
        entry_id: u64,
        key: &EntryHeightKey,
    ) -> Option<(f32, bool, Vec<String>)> {
        match self.layouts.get(&entry_id) {
            Some(cached) if &cached.key == key => {
                self.hit_count += 1;
                Some((
                    cached.height,
                    cached.show_control,
                    cached.draw_lines.clone(),
                ))
            }
            _ => {
                self.miss_count += 1;
                None
            }
        }
    }

    pub fn store_layout(
        &mut self,
        entry_id: u64,
        key: EntryHeightKey,
        height: f32,
        show_control: bool,
        draw_lines: Vec<String>,
    ) {
        self.layouts.insert(
            entry_id,
            CachedTextRowLayout {
                key,
                height,
                show_control,
                draw_lines,
            },
        );
    }
}

/// Persistent UI interaction state (separate from [`crate::app::App`] history data).
#[derive(Debug, Default)]
pub struct UiState {
    pub scroll_offset: f32,
    pub filter: EntryFilter,
    pub show_help: bool,
    pub show_settings: bool,
    pub preview_entry_id: Option<u64>,
    pub context_menu: Option<ContextMenu>,
    pub search_focused: bool,
    /// Insertion point in `App::filter_query` (UTF-8 byte index).
    pub search_caret: usize,
    /// Selection anchor; range `[min, max)` with caret when `search_sel_anchor != search_caret`.
    pub search_sel_anchor: usize,
    /// Search field hit target from the last paint (opens/focuses search on click).
    pub search_input_rect: Option<WidgetRect>,
    pub mouse_x: f32,
    pub mouse_y: f32,
    pub mouse_left_down: bool,
    pub mouse_left_pressed: bool,
    pub mouse_right_down: bool,
    pub active_widget: u32,
    pub hot_widget: u32,
    /// Inline error from the settings panel (e.g. autostart registry failure).
    pub settings_error: Option<String>,
    pub settings_scroll: f32,
    pub settings_focus: SettingsFocus,
    /// Caret byte index in the active settings text field.
    pub settings_caret: usize,
    /// Selection anchor for the active settings text field.
    pub settings_sel_anchor: usize,
    pub settings_edit_max_entries: String,
    pub settings_edit_hotkey: String,
    pub settings_edit_max_image_mb: String,
    pub settings_edit_jpeg_quality: String,
    pub settings_rects: SettingsRects,
    /// Gear button in the main header (opens settings).
    pub settings_button_rect: Option<WidgetRect>,
    /// Close button in the custom title bar (hide-to-tray).
    pub close_button_rect: Option<WidgetRect>,
    /// Indices into `App::entries` for the current filtered display order (pinned first).
    pub display_indices: Vec<usize>,
    /// Last inputs used to build `display_indices` (cache key for `refresh_display_indices`).
    display_indices_key: Option<DisplayIndicesKey>,
    /// Count of actual `build_display_indices` runs (tests assert cache hits skip rebuild).
    pub display_indices_rebuild_count: u64,
    /// Cached per-row layout from the last `refresh_list_layout` build.
    pub(crate) cached_list_layout: Vec<history::EntryLayout>,
    /// Last inputs used to build `cached_list_layout`.
    list_layout_key: Option<ListLayoutKey>,
    /// Count of actual `build_list_layout` runs (tests assert cache hits skip rebuild).
    pub list_layout_rebuild_count: u64,
    /// Bumped when `expanded_text_entries` changes; part of the list-layout cache key.
    pub(crate) expanded_version: u64,
    /// Per-entry text row heights (survives list-layout rebuilds when row inputs unchanged).
    pub(crate) entry_height_cache: EntryHeightCache,
    /// Last frame's count of laid-out visible cards (for perf validation).
    pub last_visible_count: usize,
    /// Pre-downscaled image thumbnails (keyed by entry id; cleared on width resize).
    pub thumb_cache: ThumbCache,
    /// In-flight async thumbnail loads for disk-backed image entries.
    pub thumb_load_state: ThumbLoadState,
    /// Scaled preview pixmap for the open image modal (single slot; cleared on dismiss).
    pub preview_cache: Option<PreviewImageCache>,
    /// Reused across frames so text is not re-rasterized every paint.
    pub glyph_cache: GlyphCache,
    /// Full-frame RGBA scratch for `render_app` (reallocated on client resize only).
    pub(crate) scratch: Option<Pixmap>,
    /// Last `(width, height)` used for `scratch` (invalidated when buffer is recreated).
    scratch_size: (u32, u32),
    /// Last hover target; used to skip repaints on mouse-move when unchanged.
    pub hover_key: HoverKey,
    /// Text history cards expanded beyond the default line cap.
    pub expanded_text_entries: HashSet<u64>,
    /// Expand/collapse button hit targets from the last paint (entry id, rect).
    pub expand_button_rects: Vec<(u64, crate::ui::widgets::WidgetRect)>,
    /// Total laid-out list height from the last main-view paint (scrollbar NC hit-test).
    pub last_content_height: f32,
    /// GetTickCount deadline; scrollbar visible while `now < scrollbar_visible_until`.
    pub scrollbar_visible_until: u32,
    /// Grab offset within the thumb when dragging (`mouse_y - thumb_y`).
    pub scrollbar_drag_grab_y: Option<f32>,
    /// Suppress row copy when the press started on the scrollbar track/thumb.
    pub scrollbar_suppress_click: bool,
    /// Throttle fade repaints (~60 Hz).
    pub scrollbar_last_fade_tick: u32,
    /// Wheel scroll offset changed but repaint was throttled (~60 Hz cap).
    pub(crate) needs_scroll_repaint: bool,
    /// Last history-list repaint driven by wheel scroll (`GetTickCount`).
    pub(crate) last_scroll_repaint_tick: Option<u32>,
}

/// Minimum interval between wheel-driven history repaints (~60 Hz).
pub const SCROLL_REPAINT_MIN_MS: u32 = 16;

/// Whether `handle_input` should invalidate immediately after a wheel event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelRepaint {
    /// Caller should `InvalidateRect` now.
    Now,
    /// Defer; caller should call `request_window_repaint` once (first event in burst).
    Schedule,
    /// Defer; a repaint is already scheduled for this burst.
    Deferred,
}

impl UiState {
    pub fn new() -> Self {
        Self {
            search_focused: true,
            search_caret: 0,
            search_sel_anchor: 0,
            ..Self::default()
        }
    }

    /// Take the paint scratch buffer (reallocate when `(width, height)` changes).
    /// Callers must return it via [`Self::return_scratch`] after painting.
    pub(crate) fn take_scratch(&mut self, width: u32, height: u32) -> Option<Pixmap> {
        if self.scratch_size != (width, height) {
            self.scratch = Pixmap::new(width, height);
            self.scratch_size = (width, height);
        }
        self.scratch.take()
    }

    pub(crate) fn return_scratch(&mut self, pixmap: Pixmap) {
        self.scratch = Some(pixmap);
    }

    /// Apply completed worker replies; returns true when at least one thumb was inserted.
    pub fn apply_thumb_replies(&mut self, replies: &[thumb_loader::ThumbLoadReply]) -> bool {
        let mut changed = false;
        for reply in replies {
            if reply.generation != self.thumb_load_state.generation {
                continue;
            }
            self.thumb_load_state
                .inflight
                .remove(&(reply.entry_id, reply.dst_w, reply.dst_h));
            if let Some(pixmap) = &reply.pixmap {
                self.thumb_cache.insert(
                    reply.entry_id,
                    reply.dst_w,
                    reply.dst_h,
                    Arc::clone(pixmap),
                );
                changed = true;
            }
        }
        changed
    }

    /// Gate wheel-driven repaints to ~60 Hz; scroll offset must already be updated.
    pub fn wheel_scroll_repaint(&mut self, now: u32) -> WheelRepaint {
        match self.last_scroll_repaint_tick {
            None => {
                self.last_scroll_repaint_tick = Some(now);
                self.needs_scroll_repaint = false;
                WheelRepaint::Now
            }
            Some(last) if now.saturating_sub(last) >= SCROLL_REPAINT_MIN_MS => {
                self.last_scroll_repaint_tick = Some(now);
                self.needs_scroll_repaint = false;
                WheelRepaint::Now
            }
            Some(_) => {
                let schedule = !self.needs_scroll_repaint;
                self.needs_scroll_repaint = true;
                if schedule {
                    WheelRepaint::Schedule
                } else {
                    WheelRepaint::Deferred
                }
            }
        }
    }

    /// After `WM_PAINT`, clear a pending wheel repaint and stamp the throttle clock.
    pub fn clear_scroll_repaint_after_paint(&mut self, now: u32) {
        if self.needs_scroll_repaint {
            self.needs_scroll_repaint = false;
            self.last_scroll_repaint_tick = Some(now);
        }
    }

    /// If wheel events were throttled and the interval has elapsed, request one more frame.
    pub fn take_deferred_scroll_repaint(&mut self, now: u32) -> bool {
        if !self.needs_scroll_repaint {
            return false;
        }
        let Some(last) = self.last_scroll_repaint_tick else {
            return false;
        };
        if now.saturating_sub(last) < SCROLL_REPAINT_MIN_MS {
            return false;
        }
        self.last_scroll_repaint_tick = Some(now);
        self.needs_scroll_repaint = false;
        true
    }

    pub fn begin_frame(&mut self) {
        self.mouse_left_pressed = false;
        self.hot_widget = 0;
    }

    pub fn set_mouse(&mut self, x: f32, y: f32) {
        self.mouse_x = x;
        self.mouse_y = y;
    }

    pub fn on_left_down(&mut self, x: f32, y: f32) {
        self.mouse_x = x;
        self.mouse_y = y;
        self.mouse_left_down = true;
    }

    pub fn on_left_up(&mut self, x: f32, y: f32) {
        self.mouse_x = x;
        self.mouse_y = y;
        self.mouse_left_down = false;
        self.mouse_left_pressed = true;
    }

    pub fn on_right_down(&mut self, x: f32, y: f32) {
        self.mouse_x = x;
        self.mouse_y = y;
        self.mouse_right_down = true;
    }

    pub fn on_right_up(&mut self) {
        self.mouse_right_down = false;
    }

    /// Populate edit buffers when opening the settings overlay.
    pub fn open_settings(&mut self, config: &Config) {
        self.show_settings = true;
        self.search_focused = false;
        self.settings_error = None;
        self.settings_scroll = 0.0;
        self.settings_focus = SettingsFocus::None;
        self.settings_caret = 0;
        self.settings_sel_anchor = 0;
        self.settings_edit_max_entries = config.max_entries.to_string();
        self.settings_edit_hotkey = config.hotkey.clone();
        self.settings_edit_max_image_mb = format_max_image_edit(config.max_image_size_mb);
        self.settings_edit_jpeg_quality = config.jpeg_quality.to_string();
        self.settings_rects = SettingsRects::default();
    }

    /// Text buffer for the currently focused settings field.
    pub fn settings_field_text(&self, focus: SettingsFocus) -> &str {
        match focus {
            SettingsFocus::None => "",
            SettingsFocus::MaxEntries => &self.settings_edit_max_entries,
            SettingsFocus::Hotkey => &self.settings_edit_hotkey,
            SettingsFocus::MaxImageSizeMb => &self.settings_edit_max_image_mb,
            SettingsFocus::JpegQuality => &self.settings_edit_jpeg_quality,
        }
    }

    pub fn settings_field_text_mut(&mut self, focus: SettingsFocus) -> &mut String {
        match focus {
            SettingsFocus::MaxEntries => &mut self.settings_edit_max_entries,
            SettingsFocus::Hotkey => &mut self.settings_edit_hotkey,
            SettingsFocus::MaxImageSizeMb => &mut self.settings_edit_max_image_mb,
            SettingsFocus::JpegQuality => &mut self.settings_edit_jpeg_quality,
            SettingsFocus::None => &mut self.settings_edit_max_entries,
        }
    }
}

/// Format `max_image_size_mb` for the settings numeric field.
pub fn format_max_image_edit(mb: f32) -> String {
    if mb.fract() == 0.0 {
        format!("{mb:.1}")
    } else {
        mb.to_string()
    }
}

/// Build display order: pinned entries first (MRU among pinned), then unpinned.
pub fn build_display_indices(
    entries: &[ClipEntry],
    filter: EntryFilter,
    query: &str,
) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    let mut pinned = Vec::new();
    let mut unpinned = Vec::new();
    for (idx, entry) in entries.iter().enumerate() {
        if !entry_matches_filter(entry, filter, &q) {
            continue;
        }
        if entry.is_pinned {
            pinned.push(idx);
        } else {
            unpinned.push(idx);
        }
    }
    pinned.extend(unpinned);
    pinned
}

/// True when `display_indices` already reflects the current entries version, filter, and query.
pub(crate) fn display_indices_key_matches(app: &App, ui: &UiState) -> bool {
    let Some(ref key) = ui.display_indices_key else {
        return false;
    };
    key.entries_version == app.entries_version
        && key.filter == ui.filter
        && key.query == app.filter_query
}

impl UiState {
    pub(crate) fn store_display_indices_key(&mut self, app: &App) {
        self.display_indices_key = Some(DisplayIndicesKey {
            entries_version: app.entries_version,
            filter: self.filter,
            query: app.filter_query.clone(),
        });
    }

    /// Invalidate the list-layout cache when expand/collapse toggles.
    pub(crate) fn bump_expanded_version(&mut self) {
        self.expanded_version = self.expanded_version.wrapping_add(1);
    }

    pub(crate) fn store_list_layout_key(&mut self, app: &App, thumb_max_w: f32) {
        self.list_layout_key = Some(ListLayoutKey {
            entries_version: app.entries_version,
            filter: self.filter,
            query: app.filter_query.clone(),
            expanded_version: self.expanded_version,
            thumb_max_w_bucket: thumb_max_w as u32,
        });
    }
}

/// True when `cached_list_layout` already reflects current layout inputs.
pub(crate) fn list_layout_key_matches(app: &App, ui: &UiState, thumb_max_w: f32) -> bool {
    let Some(ref key) = ui.list_layout_key else {
        return false;
    };
    key.entries_version == app.entries_version
        && key.filter == ui.filter
        && key.query == app.filter_query
        && key.expanded_version == ui.expanded_version
        && key.thumb_max_w_bucket == thumb_max_w as u32
}

pub fn entry_matches_filter(entry: &ClipEntry, filter: EntryFilter, query: &str) -> bool {
    use crate::models::EntryKind;

    let kind_ok = match filter {
        EntryFilter::All => true,
        EntryFilter::Text => matches!(entry.kind, EntryKind::Text | EntryKind::RichText),
        EntryFilter::Images => entry.kind == EntryKind::Image,
        EntryFilter::Pinned => entry.is_pinned,
    };
    if !kind_ok {
        return false;
    }
    if query.is_empty() {
        return true;
    }
    // Image cards are not meaningfully searchable (preview is "Image WxH"; clipboard
    // text/source_app often contains "Screenshot" and matches arbitrary letters).
    if entry.kind == EntryKind::Image {
        return false;
    }
    let preview = history::entry_preview(entry).to_lowercase();
    preview.contains(query)
        || entry
            .html
            .as_deref()
            .is_some_and(|h| h.to_lowercase().contains(query))
        || entry
            .source_app
            .as_deref()
            .is_some_and(|s| s.to_lowercase().contains(query))
}

#[cfg(test)]
mod display_indices_cache_tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use crate::hash::hash_text;
    use crate::models::{ClipEntry, EntryKind};
    use crate::store::{LoadResult, Store};
    use crate::ui::search::refresh_display_indices;
    use std::fs;

    fn temp_data_dir(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("trayvault-ui-{prefix}-{}", std::process::id()))
    }

    fn sample_text_entry(id: u64, text: &str, pinned: bool) -> ClipEntry {
        ClipEntry {
            id,
            created_at: id,
            kind: EntryKind::Text,
            text: Some(text.into()),
            html: None,
            image: None,
            image_pixels: None,
            source_app: None,
            is_pinned: pinned,
            hash: hash_text(text),
        }
    }

    fn app_with_entries() -> App {
        let dir = temp_data_dir("display-cache");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        let store = Store::open_for_test(dir);
        let mut app = App::new(Config::default(), LoadResult::default(), store);
        app.entries = vec![
            sample_text_entry(1, "Alpha", true),
            sample_text_entry(2, "Beta", false),
            sample_text_entry(3, "Gamma", false),
        ];
        app.entries_version = 1;
        app
    }

    #[test]
    fn refresh_skips_rebuild_when_key_unchanged() {
        let app = app_with_entries();
        let mut ui = UiState::default();

        refresh_display_indices(&app, &mut ui);
        assert_eq!(ui.display_indices_rebuild_count, 1);
        assert_eq!(ui.display_indices, vec![0, 1, 2]);

        let indices = ui.display_indices.clone();
        refresh_display_indices(&app, &mut ui);
        assert_eq!(ui.display_indices_rebuild_count, 1);
        assert_eq!(ui.display_indices, indices);
    }

    #[test]
    fn refresh_rebuilds_on_entries_version_filter_or_query_change() {
        let mut app = app_with_entries();
        let mut ui = UiState::default();
        refresh_display_indices(&app, &mut ui);
        assert_eq!(ui.display_indices_rebuild_count, 1);

        app.entries_version = app.entries_version.wrapping_add(1);
        refresh_display_indices(&app, &mut ui);
        assert_eq!(ui.display_indices_rebuild_count, 2);

        ui.filter = EntryFilter::Pinned;
        refresh_display_indices(&app, &mut ui);
        assert_eq!(ui.display_indices_rebuild_count, 3);
        assert_eq!(ui.display_indices, vec![0]);

        ui.filter = EntryFilter::All;
        app.filter_query = "beta".into();
        refresh_display_indices(&app, &mut ui);
        assert_eq!(ui.display_indices_rebuild_count, 4);
        assert_eq!(ui.display_indices, vec![1]);
    }

    #[test]
    fn pinned_first_order_matches_build_display_indices() {
        let app = app_with_entries();
        let mut ui = UiState::default();
        refresh_display_indices(&app, &mut ui);
        assert_eq!(
            ui.display_indices,
            build_display_indices(&app.entries, ui.filter, &app.filter_query)
        );
    }

    #[test]
    fn empty_filter_result_is_cached() {
        let mut app = app_with_entries();
        app.filter_query = "zzz-no-match".into();
        let mut ui = UiState::default();

        refresh_display_indices(&app, &mut ui);
        assert!(ui.display_indices.is_empty());
        assert_eq!(ui.display_indices_rebuild_count, 1);

        refresh_display_indices(&app, &mut ui);
        assert!(ui.display_indices.is_empty());
        assert_eq!(ui.display_indices_rebuild_count, 1);
    }
}

#[cfg(test)]
mod wheel_scroll_repaint_tests {
    use super::*;

    #[test]
    fn wheel_scroll_repaint_immediate_then_throttles() {
        let mut ui = UiState::default();
        assert_eq!(ui.wheel_scroll_repaint(0), WheelRepaint::Now);
        assert_eq!(ui.wheel_scroll_repaint(5), WheelRepaint::Schedule);
        assert_eq!(ui.wheel_scroll_repaint(10), WheelRepaint::Deferred);
        assert!(ui.needs_scroll_repaint);
        assert_eq!(ui.wheel_scroll_repaint(20), WheelRepaint::Now);
        assert!(!ui.needs_scroll_repaint);
    }

    #[test]
    fn deferred_scroll_repaint_after_interval() {
        let mut ui = UiState::default();
        assert_eq!(ui.wheel_scroll_repaint(100), WheelRepaint::Now);
        assert_eq!(ui.wheel_scroll_repaint(105), WheelRepaint::Schedule);
        assert!(!ui.take_deferred_scroll_repaint(110));
        assert!(ui.take_deferred_scroll_repaint(100 + SCROLL_REPAINT_MIN_MS));
        assert!(!ui.needs_scroll_repaint);
    }

    #[test]
    fn clear_scroll_repaint_after_paint_clears_pending() {
        let mut ui = UiState::default();
        assert_eq!(ui.wheel_scroll_repaint(0), WheelRepaint::Now);
        assert_eq!(ui.wheel_scroll_repaint(1), WheelRepaint::Schedule);
        ui.clear_scroll_repaint_after_paint(8);
        assert!(!ui.needs_scroll_repaint);
        assert_eq!(ui.last_scroll_repaint_tick, Some(8));
    }
}

#[cfg(test)]
mod entry_matches_filter_tests {
    use super::*;
    use crate::hash::hash_text;
    use crate::models::{EntryKind, ImageRef};

    fn sample_image_entry(text: Option<&str>, source_app: Option<&str>) -> ClipEntry {
        ClipEntry {
            id: 1,
            created_at: 1,
            kind: EntryKind::Image,
            text: text.map(str::to_string),
            html: None,
            image: Some(ImageRef {
                hash: "abc".into(),
                width: 1920,
                height: 1080,
            }),
            image_pixels: None,
            source_app: source_app.map(str::to_string),
            is_pinned: false,
            hash: hash_text("image"),
        }
    }

    #[test]
    fn image_visible_with_empty_query() {
        let entry = sample_image_entry(Some("Screenshot.png"), Some("SnippingTool.exe"));
        assert!(entry_matches_filter(&entry, EntryFilter::All, ""));
        assert!(entry_matches_filter(&entry, EntryFilter::Images, ""));
    }

    #[test]
    fn image_excluded_when_query_non_empty() {
        let entry = sample_image_entry(Some("Screenshot.png"), Some("SnippingTool.exe"));
        assert!(!entry_matches_filter(&entry, EntryFilter::All, "s"));
        assert!(!entry_matches_filter(
            &entry,
            EntryFilter::Images,
            "screenshot"
        ));
        assert!(!entry_matches_filter(&entry, EntryFilter::Pinned, "shot"));
    }
}
