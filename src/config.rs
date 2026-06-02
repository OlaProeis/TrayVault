//! Hand-rolled `key = value` config loader for `config.toml`.
//!
//! Supports a minimal TOML subset: top-level keys only, no tables or arrays.
//! Per-line parse failures are logged and leave the default for that key intact.

#![allow(dead_code)] // full consumer wiring lands in Tasks 8–12

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{ClipError, Result};
use crate::log;
use crate::store::Store;

/// Filename inside the TrayVault data directory.
pub const CONFIG_FILE: &str = "config.toml";

/// Default main-window client width in pixels.
pub const DEFAULT_WINDOW_CLIENT_W: u32 = 900;
/// Default main-window client height in pixels.
pub const DEFAULT_WINDOW_CLIENT_H: u32 = 640;
/// Minimum client width accepted from config or capture.
pub const MIN_WINDOW_CLIENT_W: u32 = 400;
/// Minimum client height accepted from config or capture.
pub const MIN_WINDOW_CLIENT_H: u32 = 320;
/// Maximum client width (sanity cap for corrupt config).
pub const MAX_WINDOW_CLIENT_W: u32 = 8192;
/// Maximum client height (sanity cap for corrupt config).
pub const MAX_WINDOW_CLIENT_H: u32 = 8192;

/// Previous default hotkeys; upgraded automatically on load (see `migrate_legacy_defaults`).
const LEGACY_DEFAULT_HOTKEYS: &[&str] = &["Ctrl+Shift+V", "Ctrl+Win+V", "Ctrl+Alt+V"];

/// UI theme selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    Light,
    Dark,
    System,
}

/// On-disk image blob codec for new writes (`config.toml`: `image_blob_codec`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageBlobCodec {
    Png,
    Jpeg,
}

impl ImageBlobCodec {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "png" => Some(Self::Png),
            "jpeg" => Some(Self::Jpeg),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
        }
    }
}

impl Default for ImageBlobCodec {
    fn default() -> Self {
        Self::Png
    }
}

/// Snapshot of blob-write settings passed to the storage worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlobWriteConfig {
    pub codec: ImageBlobCodec,
    pub jpeg_quality: u8,
}

impl BlobWriteConfig {
    pub fn from_config(config: &Config) -> Self {
        Self {
            codec: config.image_blob_codec,
            jpeg_quality: config.jpeg_quality,
        }
    }
}

impl ThemeMode {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "Light" => Some(Self::Light),
            "Dark" => Some(Self::Dark),
            "System" => Some(Self::System),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::System => "System",
        }
    }
}

/// Application settings persisted at `%LOCALAPPDATA%\TrayVault\config.toml`.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub max_entries: u32,
    pub deduplicate_global: bool,
    pub hotkey: String,
    pub autostart: bool,
    pub theme: ThemeMode,
    pub capture_images: bool,
    pub capture_rich_text: bool,
    pub close_on_copy: bool,
    /// When true, the main window appears on the taskbar while it is visible.
    pub show_in_taskbar: bool,
    pub pause_capture: bool,
    pub max_image_size_mb: f32,
    /// Codec for new image blobs (`png` or `jpeg`).
    pub image_blob_codec: ImageBlobCodec,
    /// JPEG quality 1–100 when `image_blob_codec` is `jpeg`.
    pub jpeg_quality: u8,
    /// Last window top-left (screen coords). `None` until the user has moved the window once.
    pub window_x: Option<i32>,
    pub window_y: Option<i32>,
    /// Last client-area size in pixels.
    pub window_client_w: u32,
    pub window_client_h: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_entries: 500,
            deduplicate_global: false,
            hotkey: "Alt+V".into(),
            autostart: false,
            theme: ThemeMode::System,
            capture_images: true,
            capture_rich_text: true,
            close_on_copy: true,
            show_in_taskbar: true,
            pause_capture: false,
            max_image_size_mb: 5.0,
            image_blob_codec: ImageBlobCodec::Png,
            jpeg_quality: 90,
            window_x: None,
            window_y: None,
            window_client_w: DEFAULT_WINDOW_CLIENT_W,
            window_client_h: DEFAULT_WINDOW_CLIENT_H,
        }
    }
}

/// Clamp client dimensions to supported UI bounds.
pub fn clamp_client_dimensions(w: u32, h: u32) -> (u32, u32) {
    (
        w.clamp(MIN_WINDOW_CLIENT_W, MAX_WINDOW_CLIENT_W),
        h.clamp(MIN_WINDOW_CLIENT_H, MAX_WINDOW_CLIENT_H),
    )
}

impl Config {
    /// Resolve the config path inside the TrayVault data directory.
    pub fn default_path() -> PathBuf {
        Store::data_dir()
            .unwrap_or_else(|| std::env::temp_dir().join("trayvault-fallback"))
            .join(CONFIG_FILE)
    }

    /// Load settings from `path`, falling back to defaults for a missing file or bad lines.
    pub fn load_or_default(path: &Path) -> Self {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let contents = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                log::warn(&format!(
                    "config: failed to read {}: {err}; using defaults",
                    path.display()
                ));
                return Self::default();
            }
        };

        let mut config = Self::default();
        for (line_no, line) in contents.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let Some((key, value)) = split_key_value(trimmed) else {
                log::warn(&format!(
                    "config: line {}: missing '=' in `{}`; ignoring",
                    line_no + 1,
                    trimmed
                ));
                continue;
            };

            if let Err(msg) = apply_key(&mut config, key, value) {
                log::warn(&format!(
                    "config: line {}: key `{key}`: {msg}; keeping default",
                    line_no + 1
                ));
            }
        }

        migrate_legacy_defaults(&mut config, path);
        normalize_window_fields(&mut config);

        config
    }

    /// Write all settings to `path` using canonical formatting.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let body = self.format_canonical();
        let tmp_path = path.with_extension("toml.tmp");
        {
            let mut file = fs::File::create(&tmp_path)?;
            file.write_all(body.as_bytes())?;
            file.sync_all()?;
        }
        fs::rename(tmp_path, path)?;
        Ok(())
    }

    fn format_canonical(&self) -> String {
        let mut body = format!(
            "# TrayVault configuration\n\
             max_entries = {}\n\
             deduplicate_global = {}\n\
             hotkey = \"{}\"\n\
             autostart = {}\n\
             theme = {}\n\
             capture_images = {}\n\
             capture_rich_text = {}\n\
             close_on_copy = {}\n\
             show_in_taskbar = {}\n\
             pause_capture = {}\n\
             max_image_size_mb = {}\n\
             image_blob_codec = \"{}\"\n\
             jpeg_quality = {}\n\
             window_client_w = {}\n\
             window_client_h = {}\n",
            self.max_entries,
            self.deduplicate_global,
            escape_hotkey(&self.hotkey),
            self.autostart,
            self.theme.as_str(),
            self.capture_images,
            self.capture_rich_text,
            self.close_on_copy,
            self.show_in_taskbar,
            self.pause_capture,
            format_float(self.max_image_size_mb),
            self.image_blob_codec.as_str(),
            self.jpeg_quality,
            self.window_client_w,
            self.window_client_h,
        );

        if let (Some(x), Some(y)) = (self.window_x, self.window_y) {
            body.push_str(&format!("window_x = {x}\nwindow_y = {y}\n"));
        }

        body
    }
}

fn escape_hotkey(hotkey: &str) -> String {
    hotkey.replace('\\', "\\\\").replace('"', "\\\"")
}

/// One-time upgrades for settings that changed between releases.
/// Clamp stored client size and drop orphan position keys.
fn normalize_window_fields(config: &mut Config) {
    let (w, h) = clamp_client_dimensions(config.window_client_w, config.window_client_h);
    config.window_client_w = w;
    config.window_client_h = h;
    if config.window_x.is_none() || config.window_y.is_none() {
        config.window_x = None;
        config.window_y = None;
    }
}

fn migrate_legacy_defaults(config: &mut Config, path: &Path) {
    let previous = config.hotkey.clone();
    let is_legacy = LEGACY_DEFAULT_HOTKEYS
        .iter()
        .any(|legacy| previous.eq_ignore_ascii_case(legacy));
    if !is_legacy {
        return;
    }

    config.hotkey = Config::default().hotkey.clone();
    log::info(&format!(
        "config: migrated hotkey from {previous} to {}",
        config.hotkey
    ));

    if let Err(err) = config.save(path) {
        log::warn(&format!(
            "config: failed to persist hotkey migration to {}: {err}",
            path.display()
        ));
    }
}

fn format_float(value: f32) -> String {
    if value.fract() == 0.0 {
        format!("{:.1}", value)
    } else {
        value.to_string()
    }
}

/// Split `key = value` on the first `=`, trimming surrounding whitespace.
fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let eq = line.find('=')?;
    let key = line[..eq].trim();
    let value = line[eq + 1..].trim();
    if key.is_empty() {
        return None;
    }
    Some((key, value))
}

fn apply_key(config: &mut Config, key: &str, raw_value: &str) -> std::result::Result<(), String> {
    match key {
        "max_entries" => {
            config.max_entries =
                parse_u32(raw_value).ok_or_else(|| format!("invalid integer `{raw_value}`"))?;
        }
        "deduplicate_global" => {
            config.deduplicate_global =
                parse_bool(raw_value).ok_or_else(|| format!("invalid boolean `{raw_value}`"))?;
        }
        "hotkey" => {
            config.hotkey = parse_quoted_string(raw_value)
                .ok_or_else(|| format!("expected quoted string, got `{raw_value}`"))?;
        }
        "autostart" => {
            config.autostart =
                parse_bool(raw_value).ok_or_else(|| format!("invalid boolean `{raw_value}`"))?;
        }
        "theme" => {
            config.theme = ThemeMode::parse(raw_value).ok_or_else(|| {
                format!("invalid theme `{raw_value}` (expected Light, Dark, or System)")
            })?;
        }
        "capture_images" => {
            config.capture_images =
                parse_bool(raw_value).ok_or_else(|| format!("invalid boolean `{raw_value}`"))?;
        }
        "capture_rich_text" => {
            config.capture_rich_text =
                parse_bool(raw_value).ok_or_else(|| format!("invalid boolean `{raw_value}`"))?;
        }
        "close_on_copy" => {
            config.close_on_copy =
                parse_bool(raw_value).ok_or_else(|| format!("invalid boolean `{raw_value}`"))?;
        }
        "show_in_taskbar" => {
            config.show_in_taskbar =
                parse_bool(raw_value).ok_or_else(|| format!("invalid boolean `{raw_value}`"))?;
        }
        "pause_capture" => {
            config.pause_capture =
                parse_bool(raw_value).ok_or_else(|| format!("invalid boolean `{raw_value}`"))?;
        }
        "max_image_size_mb" => {
            config.max_image_size_mb =
                parse_f32(raw_value).ok_or_else(|| format!("invalid float `{raw_value}`"))?;
            if !config.max_image_size_mb.is_finite() || config.max_image_size_mb <= 0.0 {
                return Err(format!(
                    "max_image_size_mb must be a positive finite number, got `{raw_value}`"
                ));
            }
        }
        "image_blob_codec" => {
            let value = parse_quoted_string(raw_value).unwrap_or_else(|| raw_value.to_string());
            config.image_blob_codec = ImageBlobCodec::parse(&value).ok_or_else(|| {
                format!("invalid image_blob_codec `{value}` (expected png or jpeg)")
            })?;
        }
        "jpeg_quality" => {
            let q = parse_u32(raw_value).ok_or_else(|| format!("invalid integer `{raw_value}`"))?;
            if !(1..=100).contains(&q) {
                return Err(format!("jpeg_quality must be 1–100, got `{raw_value}`"));
            }
            config.jpeg_quality = q as u8;
        }
        "window_x" => {
            config.window_x =
                Some(parse_i32(raw_value).ok_or_else(|| format!("invalid integer `{raw_value}`"))?);
        }
        "window_y" => {
            config.window_y =
                Some(parse_i32(raw_value).ok_or_else(|| format!("invalid integer `{raw_value}`"))?);
        }
        "window_client_w" => {
            config.window_client_w =
                parse_u32(raw_value).ok_or_else(|| format!("invalid integer `{raw_value}`"))?;
        }
        "window_client_h" => {
            config.window_client_h =
                parse_u32(raw_value).ok_or_else(|| format!("invalid integer `{raw_value}`"))?;
        }
        other => return Err(format!("unknown key `{other}`")),
    }
    Ok(())
}

fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_u32(s: &str) -> Option<u32> {
    s.parse().ok()
}

fn parse_i32(s: &str) -> Option<i32> {
    s.parse().ok()
}

fn parse_f32(s: &str) -> Option<f32> {
    s.parse().ok()
}

/// Parse a double-quoted string with `\"` and `\\` escapes.
fn parse_quoted_string(s: &str) -> Option<String> {
    if s.len() < 2 || !s.starts_with('"') || !s.ends_with('"') {
        return None;
    }

    let inner = &s[1..s.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let esc = chars.next()?;
            match esc {
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                _ => return None,
            }
        } else if ch == '"' {
            return None;
        } else {
            out.push(ch);
        }
    }
    Some(out)
}

/// Strict validation helper for callers that must reject invalid config (Task 7+).
pub fn validate_config(config: &Config) -> Result<()> {
    if config.max_entries == 0 {
        return Err(ClipError::Config(
            "max_entries must be greater than 0".into(),
        ));
    }
    if config.hotkey.trim().is_empty() {
        return Err(ClipError::Config("hotkey must not be empty".into()));
    }
    if !config.max_image_size_mb.is_finite() || config.max_image_size_mb <= 0.0 {
        return Err(ClipError::Config(
            "max_image_size_mb must be a positive finite number".into(),
        ));
    }
    if !(1..=100).contains(&config.jpeg_quality) {
        return Err(ClipError::Config(
            "jpeg_quality must be between 1 and 100".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_config_path(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "trayvault-config-{prefix}-{}-config.toml",
            std::process::id()
        ))
    }

    #[test]
    fn split_key_value_trims_whitespace() {
        assert_eq!(
            split_key_value("  max_entries   =   42  "),
            Some(("max_entries", "42"))
        );
    }

    #[test]
    fn parse_quoted_string_handles_escapes() {
        assert_eq!(
            parse_quoted_string(r#""Ctrl+Alt+V""#),
            Some("Ctrl+Alt+V".into())
        );
        assert_eq!(
            parse_quoted_string(r#""say \"hi\"""#),
            Some("say \"hi\"".into())
        );
        assert_eq!(parse_quoted_string("unquoted"), None);
    }

    #[test]
    fn clamp_client_dimensions_enforces_bounds() {
        assert_eq!(clamp_client_dimensions(100, 100), (400, 320));
        assert_eq!(clamp_client_dimensions(900, 640), (900, 640));
        assert_eq!(
            clamp_client_dimensions(99_999, 99_999),
            (MAX_WINDOW_CLIENT_W, MAX_WINDOW_CLIENT_H)
        );
    }

    #[test]
    fn load_or_default_reads_window_geometry() {
        let path = temp_config_path("window-geom");
        fs::write(
            &path,
            "window_x = 120\nwindow_y = 80\nwindow_client_w = 1024\nwindow_client_h = 768\n",
        )
        .unwrap();

        let config = Config::load_or_default(&path);
        assert_eq!(config.window_x, Some(120));
        assert_eq!(config.window_y, Some(80));
        assert_eq!(config.window_client_w, 1024);
        assert_eq!(config.window_client_h, 768);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn apply_key_sets_fields() {
        let mut config = Config::default();
        apply_key(&mut config, "max_entries", "1000").unwrap();
        apply_key(&mut config, "theme", "Dark").unwrap();
        apply_key(&mut config, "hotkey", r#""Alt+V""#).unwrap();
        assert_eq!(config.max_entries, 1000);
        assert_eq!(config.theme, ThemeMode::Dark);
        assert_eq!(config.hotkey, "Alt+V");
    }

    #[test]
    fn load_or_default_migrates_legacy_default_hotkey() {
        let path = temp_config_path("legacy-hotkey");
        fs::write(&path, "hotkey = \"Ctrl+Shift+V\"\nmax_entries = 100\n").unwrap();

        let config = Config::load_or_default(&path);
        assert_eq!(config.hotkey, "Alt+V");
        assert_eq!(config.max_entries, 100);

        let on_disk = fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains(r#"hotkey = "Alt+V""#));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn load_or_default_migrates_legacy_win_hotkey() {
        let path = temp_config_path("legacy-win-hotkey");
        fs::write(&path, "hotkey = \"Ctrl+Win+V\"\n").unwrap();

        let config = Config::load_or_default(&path);
        assert_eq!(config.hotkey, "Alt+V");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn load_or_default_migrates_previous_default_hotkey() {
        let path = temp_config_path("prev-default-hotkey");
        fs::write(&path, "hotkey = \"Ctrl+Alt+V\"\n").unwrap();

        let config = Config::load_or_default(&path);
        assert_eq!(config.hotkey, "Alt+V");

        let on_disk = fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains(r#"hotkey = "Alt+V""#));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn load_or_default_keeps_custom_hotkey() {
        let path = temp_config_path("custom-hotkey");
        fs::write(&path, "hotkey = \"Alt+F2\"\n").unwrap();

        let config = Config::load_or_default(&path);
        assert_eq!(config.hotkey, "Alt+F2");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn load_or_default_missing_file_uses_defaults() {
        let path = temp_config_path("missing");
        let _ = fs::remove_file(&path);
        let config = Config::load_or_default(&path);
        assert_eq!(config, Config::default());
    }

    #[test]
    fn load_or_default_partial_invalid_keeps_defaults_for_bad_keys() {
        let path = temp_config_path("partial");
        fs::write(
            &path,
            "# comment\n\
             max_entries = 250\n\
             theme = NotATheme\n\
             hotkey = unquoted\n\
             deduplicate_global = true\n",
        )
        .unwrap();

        let config = Config::load_or_default(&path);
        assert_eq!(config.max_entries, 250);
        assert!(config.deduplicate_global);
        assert_eq!(config.theme, ThemeMode::System);
        assert_eq!(config.hotkey, "Alt+V");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn load_or_default_ignores_blank_and_comment_lines() {
        let path = temp_config_path("comments");
        fs::write(&path, "\n# full-line comment\n\npause_capture = true\n").unwrap();

        let config = Config::load_or_default(&path);
        assert!(config.pause_capture);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn save_reload_round_trip() {
        let path = temp_config_path("roundtrip");
        let original = Config {
            max_entries: 42,
            deduplicate_global: true,
            hotkey: r#"Ctrl+"V""#.into(),
            autostart: true,
            theme: ThemeMode::Light,
            capture_images: false,
            capture_rich_text: false,
            close_on_copy: false,
            show_in_taskbar: false,
            pause_capture: true,
            max_image_size_mb: 2.5,
            image_blob_codec: ImageBlobCodec::Jpeg,
            jpeg_quality: 85,
            window_x: Some(42),
            window_y: Some(24),
            window_client_w: 1100,
            window_client_h: 700,
        };

        original.save(&path).unwrap();
        let loaded = Config::load_or_default(&path);
        assert_eq!(loaded, original);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn validate_config_rejects_zero_max_entries() {
        let config = Config {
            max_entries: 0,
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }
}
