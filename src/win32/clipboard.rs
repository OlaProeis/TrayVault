//! Clipboard format listener, content readers, capture pipeline, and copy-back.
//!
//! Registers `AddClipboardFormatListener` on the main window and captures text,
//! HTML, and DIB images on `WM_CLIPBOARDUPDATE`. History orchestration lives in
//! [`crate::app::App`]; this module reads/writes clipboard formats only.

#![allow(dead_code)] // public API for Tasks 8–12

use std::cell::RefCell;
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use crate::config::Config;
use crate::error::{ClipError, Result};
use crate::hash::{hash_clip_entry, hash_to_hex};
use crate::log;
use crate::models::{ClipEntry, EntryKind, ImageRef};
use crate::win32::ffi::{
    self, AddClipboardFormatListener, CloseClipboard, CloseHandle, EmptyClipboard,
    EnumClipboardFormats, GetClipboardData, GetForegroundWindow, GetWindowThreadProcessId,
    GlobalAlloc, GlobalFree, GlobalLock, GlobalSize, GlobalUnlock, OpenClipboard, OpenProcess,
    QueryFullProcessImageNameW, RegisterClipboardFormatW, RemoveClipboardFormatListener,
    SetClipboardData, CF_DIB, CF_DIBV5, CF_UNICODETEXT, GMEM_MOVEABLE, HANDLE, HWND,
    PROCESS_NAME_NATIVE, PROCESS_QUERY_LIMITED_INFORMATION,
};
use crate::win32::{last_error, wide};

// ---------------------------------------------------------------------------
// Capture config (wired from [`Config`] in Task 7)
// ---------------------------------------------------------------------------

/// Capture toggles and limits passed into the clipboard monitor.
#[derive(Clone, Copy, Debug)]
pub struct CaptureConfig {
    pub capture_images: bool,
    pub capture_rich_text: bool,
    pub pause_capture: bool,
    /// When true, skip capture if the content hash exists anywhere in history.
    pub deduplicate_global: bool,
    /// Maximum decoded image payload size in bytes (`max_image_size_mb` from config).
    pub max_image_size_bytes: u64,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            capture_images: true,
            capture_rich_text: true,
            pause_capture: false,
            deduplicate_global: false,
            max_image_size_bytes: 5 * 1024 * 1024,
        }
    }
}

impl CaptureConfig {
    pub fn from_config(config: &Config) -> Self {
        let max_bytes = (config.max_image_size_mb * 1024.0 * 1024.0).max(0.0) as u64;
        Self {
            capture_images: config.capture_images,
            capture_rich_text: config.capture_rich_text,
            pause_capture: config.pause_capture,
            deduplicate_global: config.deduplicate_global,
            max_image_size_bytes: max_bytes,
        }
    }
}

impl Config {
    /// Map application settings to clipboard capture toggles.
    pub fn capture_config(&self) -> CaptureConfig {
        CaptureConfig::from_config(self)
    }
}

// ---------------------------------------------------------------------------
// Decoded image (internal)
// ---------------------------------------------------------------------------

/// Raw BGRA pixels decoded from a DIB clipboard format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Registered format ids
// ---------------------------------------------------------------------------

struct ClipboardFormats {
    html: u32,
    can_include: u32,
    exclude: u32,
}

impl ClipboardFormats {
    fn register() -> Self {
        Self {
            html: register_format("HTML Format"),
            can_include: register_format("CanIncludeInClipboardHistory"),
            exclude: register_format("ExcludeClipboardContentFromMonitorProcessing"),
        }
    }

    fn log_ids(&self) {
        log::info(&format!(
            "clipboard formats: HTML={} CanInclude={} Exclude={}",
            self.html, self.can_include, self.exclude
        ));
    }
}

fn register_format(name: &str) -> u32 {
    let wide_name = wide(name);
    // SAFETY: NUL-terminated UTF-16 format name.
    let id = unsafe { RegisterClipboardFormatW(wide_name.as_ptr()) };
    if id == 0 {
        log::warn(&format!("RegisterClipboardFormatW failed for `{name}`"));
    }
    id
}

// ---------------------------------------------------------------------------
// Self-copy suppression
// ---------------------------------------------------------------------------

/// Tracks TrayVault-originated clipboard writes so we do not re-capture them.
#[derive(Clone, Debug, Default)]
struct SelfCopyGuard {
    suppress_next: bool,
}

impl SelfCopyGuard {
    /// Call immediately before `SetClipboardData` (Task 7 copy-back).
    pub fn mark_own_write(&mut self) {
        self.suppress_next = true;
    }

    fn should_skip(&mut self) -> bool {
        if self.suppress_next {
            self.suppress_next = false;
            log::info("clipboard update suppressed (self-originated)");
            return true;
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Clipboard monitor
// ---------------------------------------------------------------------------

/// Owns capture config, format ids, and listener registration.
pub struct ClipboardMonitor {
    hwnd: HWND,
    config: CaptureConfig,
    formats: ClipboardFormats,
    self_copy: SelfCopyGuard,
}

impl ClipboardMonitor {
    pub fn new(hwnd: HWND, config: CaptureConfig) -> Self {
        let formats = ClipboardFormats::register();
        formats.log_ids();
        Self {
            hwnd,
            config,
            formats,
            self_copy: SelfCopyGuard::default(),
        }
    }

    pub fn config(&self) -> CaptureConfig {
        self.config
    }

    pub fn set_config(&mut self, config: CaptureConfig) {
        self.config = config;
    }

    pub fn mark_own_write(&mut self) {
        self.self_copy.mark_own_write();
    }

    /// Register the window as a clipboard format listener.
    pub fn register_listener(&self) -> Result<()> {
        // SAFETY: `hwnd` is a valid top-level window.
        let ok = unsafe { AddClipboardFormatListener(self.hwnd) };
        if ok == 0 {
            return Err(last_error("AddClipboardFormatListener"));
        }
        log::info("clipboard format listener registered");
        Ok(())
    }

    pub fn unregister_listener(&self) -> Result<()> {
        // SAFETY: `hwnd` matches the registered listener.
        let ok = unsafe { RemoveClipboardFormatListener(self.hwnd) };
        if ok == 0 {
            return Err(last_error("RemoveClipboardFormatListener"));
        }
        Ok(())
    }

    /// `WM_CLIPBOARDUPDATE` handler — read clipboard and return a new entry.
    pub fn try_capture(&mut self) -> Result<Option<ClipEntry>> {
        if self.config.pause_capture {
            return Ok(None);
        }
        if self.self_copy.should_skip() {
            return Ok(None);
        }
        self.capture_once()
    }

    fn capture_once(&mut self) -> Result<Option<ClipEntry>> {
        open_clipboard_with_retry(self.hwnd)?;

        let result = (|| {
            if is_sensitive_excluded(self.formats.can_include, self.formats.exclude)? {
                log::info("clipboard capture skipped (sensitive-content exclusion)");
                return Ok(None);
            }

            let text = read_text()?;
            let html = if self.config.capture_rich_text {
                read_html(self.formats.html)?
            } else {
                None
            };
            let image = if self.config.capture_images {
                read_image_with_limit(self.config.max_image_size_bytes)?
            } else {
                None
            };

            build_entry(text, html, image)
        })();

        // SAFETY: paired with OpenClipboard.
        if unsafe { CloseClipboard() } == 0 {
            log::warn(&format!(
                "CloseClipboard failed after capture: {}",
                last_error("CloseClipboard")
            ));
        }

        if let Ok(Some(ref entry)) = result {
            let kind = entry.kind;
            let source = entry.source_app.clone();
            let text_len = entry.text.as_ref().map(|s| s.len()).unwrap_or(0);
            let html_len = entry.html.as_ref().map(|s| s.len()).unwrap_or(0);
            let image_dims = entry.image.as_ref().map(|i| (i.width, i.height));
            log::info(&format!(
                "captured {kind:?} source={source:?} text_len={text_len} html_len={html_len} image={image_dims:?}"
            ));
        }

        result
    }
}

/// Shared handle for wiring the monitor into `WindowCallbacks`.
pub type SharedClipboardMonitor = Rc<RefCell<ClipboardMonitor>>;

pub fn shared_monitor(hwnd: HWND, config: CaptureConfig) -> SharedClipboardMonitor {
    Rc::new(RefCell::new(ClipboardMonitor::new(hwnd, config)))
}

// ---------------------------------------------------------------------------
// Clipboard open with retry
// ---------------------------------------------------------------------------

fn open_clipboard_with_retry(hwnd: HWND) -> Result<()> {
    const RETRY_MS: u64 = 100;
    for attempt in 0..2 {
        // SAFETY: `hwnd` is the listener owner window.
        if unsafe { OpenClipboard(hwnd) } != 0 {
            return Ok(());
        }
        if attempt == 0 {
            thread::sleep(Duration::from_millis(RETRY_MS));
        }
    }
    Err(last_error("OpenClipboard"))
}

// ---------------------------------------------------------------------------
// Sensitive-content exclusion
// ---------------------------------------------------------------------------

fn is_sensitive_excluded(can_include_fmt: u32, exclude_fmt: u32) -> Result<bool> {
    if exclude_fmt != 0 && clipboard_has_format(exclude_fmt) {
        return Ok(true);
    }
    if can_include_fmt != 0 && clipboard_has_format(can_include_fmt) {
        return Ok(!can_include_flag_allows(can_include_fmt)?);
    }
    Ok(false)
}

fn clipboard_has_format(format: u32) -> bool {
    let mut fmt = 0u32;
    loop {
        // SAFETY: clipboard must be open; `fmt` is the previous format or 0 to start.
        fmt = unsafe { EnumClipboardFormats(fmt) };
        if fmt == 0 {
            return false;
        }
        if fmt == format {
            return true;
        }
    }
}

fn can_include_flag_allows(format: u32) -> Result<bool> {
    let handle = get_clipboard_data_handle(format)?;
    let bytes = lock_global_bytes(handle)?;
    // Value is a small ASCII digit string: "1" = allow, "0" = deny.
    let text = String::from_utf8_lossy(&bytes);
    let trimmed = text.trim();
    Ok(trimmed != "0")
}

// ---------------------------------------------------------------------------
// Content readers
// ---------------------------------------------------------------------------

/// Read `CF_UNICODETEXT` as UTF-8 `String`.
pub fn read_text() -> Result<Option<String>> {
    let Some(handle) = try_get_clipboard_data(CF_UNICODETEXT) else {
        return Ok(None);
    };
    let wide = lock_global_wide(handle)?;
    Ok(Some(wide_to_string(&wide)))
}

/// Read registered `"HTML Format"` and extract the HTML fragment.
pub fn read_html(html_format: u32) -> Result<Option<String>> {
    if html_format == 0 {
        return Ok(None);
    }
    let Some(handle) = try_get_clipboard_data(html_format) else {
        return Ok(None);
    };
    let bytes = lock_global_bytes(handle)?;
    Ok(extract_html_fragment(&bytes))
}

/// Read `CF_DIBV5` or `CF_DIB` and decode to BGRA pixels.
pub fn read_image() -> Result<Option<DecodedImage>> {
    read_image_with_limit(u64::MAX)
}

fn read_image_with_limit(max_bytes: u64) -> Result<Option<DecodedImage>> {
    let handle = try_get_clipboard_data(CF_DIBV5).or_else(|| try_get_clipboard_data(CF_DIB));
    let Some(handle) = handle else {
        return Ok(None);
    };
    let bytes = lock_global_bytes(handle)?;
    match parse_dib_to_bgra(&bytes, max_bytes) {
        Ok(image) => Ok(Some(image)),
        Err(ClipError::Other(msg))
            if msg == "DIB dimensions overflow" || msg == "DIB pixel data exceeds size limit" =>
        {
            log::info(&format!("clipboard image skipped ({msg})"));
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

fn try_get_clipboard_data(format: u32) -> Option<HANDLE> {
    // SAFETY: clipboard is open.
    let handle = unsafe { GetClipboardData(format) };
    if handle == 0 {
        None
    } else {
        Some(handle)
    }
}

fn get_clipboard_data_handle(format: u32) -> Result<HANDLE> {
    try_get_clipboard_data(format)
        .ok_or_else(|| ClipError::Other(format!("GetClipboardData({format}) returned null")))
}

fn lock_global_bytes(handle: HANDLE) -> Result<Vec<u8>> {
    // SAFETY: `handle` is a valid global memory object from `GetClipboardData`.
    let ptr = unsafe { GlobalLock(handle) };
    if ptr.is_null() {
        return Err(last_error("GlobalLock"));
    }
    // SAFETY: `GlobalSize` on a locked global object.
    let size = unsafe { GlobalSize(handle) };
    let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, size) };
    let bytes = slice.to_vec();
    // SAFETY: paired with GlobalLock.
    let _ = unsafe { GlobalUnlock(handle) };
    Ok(bytes)
}

fn lock_global_wide(handle: HANDLE) -> Result<Vec<u16>> {
    let bytes = lock_global_bytes(handle)?;
    if bytes.len() < 2 {
        return Ok(Vec::new());
    }
    let mut wide = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        wide.push(u16::from_le_bytes([chunk[0], chunk[1]]));
    }
    // Trim trailing NUL units.
    while wide.last() == Some(&0) {
        wide.pop();
    }
    Ok(wide)
}

fn wide_to_string(wide: &[u16]) -> String {
    String::from_utf16_lossy(wide)
}

// ---------------------------------------------------------------------------
// HTML Format parsing
// ---------------------------------------------------------------------------

/// Parse the CF_HTML header and return the fragment (or full HTML slice).
pub fn extract_html_fragment(data: &[u8]) -> Option<String> {
    if let Some(fragment) = extract_html_by_marker_bytes(data, b"StartFragment:", b"EndFragment:") {
        return Some(fragment);
    }
    extract_html_by_marker_bytes(data, b"StartHTML:", b"EndHTML:")
}

fn extract_html_by_marker_bytes(data: &[u8], start_key: &[u8], end_key: &[u8]) -> Option<String> {
    let start_off = parse_html_offset_bytes(data, start_key)?;
    let end_off = parse_html_offset_bytes(data, end_key)?;
    if end_off <= start_off || end_off > data.len() {
        return None;
    }
    String::from_utf8(data[start_off..end_off].to_vec()).ok()
}

fn parse_html_offset_bytes(data: &[u8], key: &[u8]) -> Option<usize> {
    let pos = data.windows(key.len()).position(|w| w == key)?;
    let rest = &data[pos + key.len()..];
    let digits: String = rest
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .map(|b| *b as char)
        .collect();
    digits.parse().ok()
}

// ---------------------------------------------------------------------------
// DIB → BGRA
// ---------------------------------------------------------------------------

/// Decode a device-independent bitmap (DIB) memory block to top-down BGRA pixels.
///
/// `max_bytes` is the maximum allowed decoded BGRA buffer size (`width * height * 4`);
/// pass `u64::MAX` when no budget applies.
pub fn parse_dib_to_bgra(data: &[u8], max_bytes: u64) -> Result<DecodedImage> {
    if data.len() < std::mem::size_of::<ffi::BITMAPINFOHEADER>() {
        return Err(ClipError::Other(
            "DIB too small for BITMAPINFOHEADER".into(),
        ));
    }

    let header = read_bitmap_info_header(data)?;
    let width = header.bi_width.unsigned_abs();
    let raw_height = header.bi_height;
    let top_down = raw_height < 0;
    let height = raw_height.unsigned_abs();

    if width == 0 || height == 0 {
        return Err(ClipError::Other("DIB has zero width or height".into()));
    }

    let needed = (width as u64)
        .checked_mul(height as u64)
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| ClipError::Other("DIB dimensions overflow".into()))?;

    if needed > max_bytes {
        return Err(ClipError::Other("DIB pixel data exceeds size limit".into()));
    }

    if needed > isize::MAX as u64 {
        return Err(ClipError::Other("DIB dimensions overflow".into()));
    }

    let header_size = header.bi_size as usize;
    let color_table_len = color_table_byte_len(&header, data.len().saturating_sub(header_size));
    let pixel_offset = header_size.saturating_add(color_table_len);

    if pixel_offset > data.len() {
        return Err(ClipError::Other("DIB pixel offset past buffer end".into()));
    }

    let pixels_raw = &data[pixel_offset..];
    let mut out = vec![0u8; needed as usize];

    match header.bi_bit_count {
        32 => decode_bgra32(pixels_raw, width, height, top_down, &mut out)?,
        24 => decode_bgr24(pixels_raw, width, height, top_down, &mut out)?,
        8 => decode_indexed8(
            pixels_raw,
            width,
            height,
            top_down,
            &header,
            &data[header_size..],
            &mut out,
        )?,
        n => {
            return Err(ClipError::Other(format!("unsupported DIB bit depth: {n}")));
        }
    }

    Ok(DecodedImage {
        width,
        height,
        pixels: out,
    })
}

#[derive(Clone, Copy, Debug)]
struct DibHeader {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_clr_used: u32,
}

impl DibHeader {
    fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 40 {
            return None;
        }
        Some(Self {
            bi_size: u32::from_le_bytes(data[0..4].try_into().ok()?),
            bi_width: i32::from_le_bytes(data[4..8].try_into().ok()?),
            bi_height: i32::from_le_bytes(data[8..12].try_into().ok()?),
            bi_planes: u16::from_le_bytes(data[12..14].try_into().ok()?),
            bi_bit_count: u16::from_le_bytes(data[14..16].try_into().ok()?),
            bi_compression: u32::from_le_bytes(data[16..20].try_into().ok()?),
            bi_clr_used: u32::from_le_bytes(data[32..36].try_into().ok()?),
        })
    }
}

fn read_bitmap_info_header(data: &[u8]) -> Result<DibHeader> {
    DibHeader::from_bytes(data).ok_or_else(|| ClipError::Other("invalid BITMAPINFOHEADER".into()))
}

fn color_table_byte_len(header: &DibHeader, remaining: usize) -> usize {
    if header.bi_bit_count > 8 {
        return 0;
    }
    let entries = if header.bi_clr_used != 0 {
        header.bi_clr_used as usize
    } else {
        1usize << header.bi_bit_count
    };
    entries.saturating_mul(4).min(remaining)
}

trait SignedAbs {
    fn unsigned_abs(self) -> u32;
}

impl SignedAbs for i32 {
    fn unsigned_abs(self) -> u32 {
        self.unsigned_abs()
    }
}

fn row_stride(width: u32, bpp: u32) -> usize {
    let bytes = (width * bpp).div_ceil(8) as usize;
    (bytes + 3) & !3
}

fn decode_bgra32(
    src: &[u8],
    width: u32,
    height: u32,
    top_down: bool,
    out: &mut [u8],
) -> Result<()> {
    let stride = row_stride(width, 32);
    let row_bytes = (width * 4) as usize;
    for row in 0..height {
        let src_row = if top_down { row } else { height - 1 - row };
        let src_start = (src_row as usize) * stride;
        let dst_start = (row as usize) * row_bytes;
        let end = src_start.saturating_add(row_bytes);
        if end > src.len() || dst_start + row_bytes > out.len() {
            return Err(ClipError::Other("DIB 32-bpp row out of bounds".into()));
        }
        out[dst_start..dst_start + row_bytes].copy_from_slice(&src[src_start..end]);
    }
    Ok(())
}

fn decode_bgr24(src: &[u8], width: u32, height: u32, top_down: bool, out: &mut [u8]) -> Result<()> {
    let stride = row_stride(width, 24);
    for row in 0..height {
        let src_row = if top_down { row } else { height - 1 - row };
        let src_start = (src_row as usize) * stride;
        let dst_row = (row as usize) * (width as usize) * 4;
        for col in 0..width as usize {
            let si = src_start + col * 3;
            let di = dst_row + col * 4;
            if si + 3 > src.len() || di + 4 > out.len() {
                return Err(ClipError::Other("DIB 24-bpp pixel out of bounds".into()));
            }
            out[di] = src[si];
            out[di + 1] = src[si + 1];
            out[di + 2] = src[si + 2];
            out[di + 3] = 0xFF;
        }
    }
    Ok(())
}

fn decode_indexed8(
    src: &[u8],
    width: u32,
    height: u32,
    top_down: bool,
    header: &DibHeader,
    color_table: &[u8],
    out: &mut [u8],
) -> Result<()> {
    let stride = row_stride(width, 8);
    let colors = color_table_byte_len(header, color_table.len()) / 4;
    for row in 0..height {
        let src_row = if top_down { row } else { height - 1 - row };
        let src_start = (src_row as usize) * stride;
        let dst_row = (row as usize) * (width as usize) * 4;
        for col in 0..width as usize {
            let si = src_start + col;
            if si >= src.len() {
                return Err(ClipError::Other("DIB 8-bpp index out of bounds".into()));
            }
            let idx = src[si] as usize;
            if idx >= colors {
                return Err(ClipError::Other("DIB palette index out of range".into()));
            }
            let ci = idx * 4;
            let di = dst_row + col * 4;
            if ci + 3 >= color_table.len() || di + 4 > out.len() {
                return Err(ClipError::Other("DIB palette entry out of bounds".into()));
            }
            out[di] = color_table[ci];
            out[di + 1] = color_table[ci + 1];
            out[di + 2] = color_table[ci + 2];
            out[di + 3] = 0xFF;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Source app attribution
// ---------------------------------------------------------------------------

/// Best-effort foreground process executable name (e.g. `"chrome.exe"`).
pub fn capture_source_app() -> Option<String> {
    // SAFETY: no arguments.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd == 0 {
        return None;
    }

    let mut pid = 0u32;
    // SAFETY: `hwnd` is valid; `pid` is writable.
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    if pid == 0 {
        return None;
    }

    query_process_executable_name(pid).ok()
}

fn query_process_executable_name(pid: u32) -> Result<String> {
    // SAFETY: limited query access to an existing process.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process == 0 {
        return Err(last_error("OpenProcess"));
    }

    let result = read_process_image_name(process);
    // SAFETY: handle from OpenProcess.
    let _ = unsafe { CloseHandle(process) };
    result
}

fn read_process_image_name(process: HANDLE) -> Result<String> {
    let mut buf = [0u16; 1024];
    let mut size = buf.len() as u32;
    // SAFETY: valid process handle and writable buffer.
    let ok = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_NATIVE, buf.as_mut_ptr(), &mut size)
    };
    if ok == 0 {
        return Err(last_error("QueryFullProcessImageNameW"));
    }
    let path = String::from_utf16_lossy(&buf[..size as usize]);
    Ok(executable_name_from_path(&path).unwrap_or(path))
}

/// Extract `"notepad.exe"` from `"C:\\Windows\\System32\\notepad.exe"`.
pub fn executable_name_from_path(path: &str) -> Option<String> {
    let name = path.rsplit(['\\', '/']).next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

// ---------------------------------------------------------------------------
// Entry assembly
// ---------------------------------------------------------------------------

fn build_entry(
    text: Option<String>,
    html: Option<String>,
    image: Option<DecodedImage>,
) -> Result<Option<ClipEntry>> {
    let has_text = text.as_ref().is_some_and(|t| !t.is_empty());
    let has_html = html.as_ref().is_some_and(|h| !h.is_empty());
    let has_image = image.is_some();

    if !has_text && !has_html && !has_image {
        return Ok(None);
    }

    let (kind, text, html, image_ref, image_pixels) = if has_image {
        let img = image.expect("has_image");
        let preview = text
            .filter(|t| !t.is_empty())
            .or_else(|| html_to_plain_preview(html.as_deref()));
        (
            EntryKind::Image,
            preview,
            None,
            Some(ImageRef {
                hash: String::new(), // filled after hash is computed
                width: img.width,
                height: img.height,
            }),
            Some(img.pixels),
        )
    } else if has_html {
        let plain = text
            .filter(|t| !t.is_empty())
            .or_else(|| html_to_plain_preview(html.as_deref()));
        (EntryKind::RichText, plain, html, None, None)
    } else {
        (EntryKind::Text, text, None, None, None)
    };

    let hash = hash_clip_entry(
        kind,
        text.as_deref(),
        html.as_deref(),
        image_pixels.as_deref(),
    );

    let image = image_ref.map(|mut image_ref| {
        image_ref.hash = hash_to_hex(hash);
        image_ref
    });

    Ok(Some(ClipEntry {
        id: 0,
        created_at: ClipEntry::now_millis(),
        kind,
        text,
        html,
        image,
        image_pixels,
        source_app: capture_source_app(),
        is_pinned: false,
        hash,
    }))
}

fn html_to_plain_preview(html: Option<&str>) -> Option<String> {
    let html = html?;
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

// ---------------------------------------------------------------------------
// Copy-back (write history entry to clipboard)
// ---------------------------------------------------------------------------

/// Write a history entry to the system clipboard (formats as appropriate).
pub fn write_entry_to_clipboard(
    hwnd: HWND,
    entry: &ClipEntry,
    image_pixels: Option<&[u8]>,
) -> Result<()> {
    open_clipboard_with_retry(hwnd)?;

    let result = (|| {
        // SAFETY: clipboard is open.
        if unsafe { EmptyClipboard() } == 0 {
            return Err(last_error("EmptyClipboard"));
        }

        let html_format = register_format("HTML Format");

        match entry.kind {
            EntryKind::Text => {
                if let Some(text) = entry.text.as_deref().filter(|t| !t.is_empty()) {
                    set_unicode_text(text)?;
                }
            }
            EntryKind::RichText => {
                if let Some(text) = entry.text.as_deref().filter(|t| !t.is_empty()) {
                    set_unicode_text(text)?;
                }
                if let Some(html) = entry.html.as_deref().filter(|h| !h.is_empty()) {
                    set_html_format(html_format, html)?;
                }
            }
            EntryKind::Image => {
                if let Some(text) = entry.text.as_deref().filter(|t| !t.is_empty()) {
                    set_unicode_text(text)?;
                }
                if let (Some(image), Some(pixels)) = (&entry.image, image_pixels) {
                    set_dib_image(image.width, image.height, pixels)?;
                }
            }
        }

        Ok(())
    })();

    // SAFETY: paired with OpenClipboard.
    if unsafe { CloseClipboard() } == 0 {
        log::warn(&format!(
            "CloseClipboard failed after copy-back: {}",
            last_error("CloseClipboard")
        ));
    }

    result
}

fn set_unicode_text(text: &str) -> Result<()> {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    let bytes = wide_to_bytes(&wide);
    let handle = alloc_moveable_global(&bytes)?;
    set_clipboard_or_free(CF_UNICODETEXT, handle, "SetClipboardData(CF_UNICODETEXT)")
}

fn set_html_format(html_format: u32, fragment: &str) -> Result<()> {
    if html_format == 0 {
        return Ok(());
    }
    let payload = build_html_clipboard_payload(fragment);
    let handle = alloc_moveable_global(payload.as_bytes())?;
    set_clipboard_or_free(html_format, handle, "SetClipboardData(HTML Format)")
}

fn set_dib_image(width: u32, height: u32, pixels: &[u8]) -> Result<()> {
    let dib = encode_bgra_dib(width, height, pixels);
    let handle = alloc_moveable_global(&dib)?;
    set_clipboard_or_free(CF_DIB, handle, "SetClipboardData(CF_DIB)")
}

/// Transfer global memory to the clipboard, or free it if `SetClipboardData` fails.
fn set_clipboard_or_free(format: u32, handle: HANDLE, api: &'static str) -> Result<()> {
    // SAFETY: clipboard is open; `handle` is a valid GMEM_MOVEABLE global object.
    let out = unsafe { SetClipboardData(format, handle) };
    if out == 0 {
        // SAFETY: ownership stays with the caller when SetClipboardData fails.
        unsafe {
            let _ = GlobalFree(handle);
        }
        return Err(last_error(api));
    }
    Ok(())
}

fn alloc_moveable_global(data: &[u8]) -> Result<HANDLE> {
    // SAFETY: GlobalAlloc for clipboard transfer ownership to the OS.
    let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, data.len()) };
    if handle == 0 {
        return Err(last_error("GlobalAlloc"));
    }
    // SAFETY: handle from successful GlobalAlloc.
    let ptr = unsafe { GlobalLock(handle) };
    if ptr.is_null() {
        return Err(last_error("GlobalLock"));
    }
    // SAFETY: locked buffer is `data.len()` bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
        let _ = GlobalUnlock(handle);
    }
    Ok(handle)
}

fn wide_to_bytes(wide: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(wide.len() * 2);
    for unit in wide {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

/// Build a top-down 32-bpp DIB from BGRA pixels (for copy-back).
pub fn encode_bgra_dib(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut header = vec![0u8; 40];
    header[0..4].copy_from_slice(&40u32.to_le_bytes());
    header[4..8].copy_from_slice(&width.to_le_bytes());
    header[8..12].copy_from_slice(&(-(height as i32)).to_le_bytes());
    header[12..14].copy_from_slice(&1u16.to_le_bytes());
    header[14..16].copy_from_slice(&32u16.to_le_bytes());
    header[16..20].copy_from_slice(&ffi::BI_RGB.to_le_bytes());
    header[20..24].copy_from_slice(&(pixels.len() as u32).to_le_bytes());
    let mut out = header;
    out.extend_from_slice(pixels);
    out
}

fn build_html_clipboard_payload(fragment: &str) -> String {
    let prefix = "<html><body><!--StartFragment-->";
    let suffix = "<!--EndFragment--></body></html>";
    let html_body = format!("{prefix}{fragment}{suffix}");
    const HEADER_PLACEHOLDER: &str = "Version:1.0\r\nStartHTML:000000000\r\nEndHTML:000000000\r\nStartFragment:000000000\r\nEndFragment:000000000\r\n";
    let header_len = HEADER_PLACEHOLDER.len();
    let start_html = header_len;
    let end_html = header_len + html_body.len();
    let start_frag = header_len + prefix.len();
    let end_frag = start_frag + fragment.len();
    format!(
        "Version:1.0\r\nStartHTML:{start_html:09}\r\nEndHTML:{end_html:09}\r\nStartFragment:{start_frag:09}\r\nEndFragment:{end_frag:09}\r\n{html_body}"
    )
}

// ---------------------------------------------------------------------------
// Unit tests (portable logic only)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_name_from_windows_path() {
        assert_eq!(
            executable_name_from_path(r"C:\Windows\System32\notepad.exe"),
            Some("notepad.exe".into())
        );
        assert_eq!(
            executable_name_from_path("/usr/bin/code"),
            Some("code".into())
        );
        assert_eq!(executable_name_from_path(""), None);
    }

    #[test]
    fn html_fragment_extraction() {
        let content = "<html><body><!--StartFragment--><b>Hi</b><!--EndFragment--></body></html>";
        let frag = "<b>Hi</b>";
        let frag_start = content.find(frag).expect("fragment in content");
        let frag_end = frag_start + frag.len();

        let header_len = format!(
            "Version:1.0\r\nStartHTML:{:09}\r\nEndHTML:{:09}\r\nStartFragment:{:09}\r\nEndFragment:{:09}\r\n",
            0, 0, 0, 0
        )
        .len();
        let body_start = header_len;
        let html = format!(
            "Version:1.0\r\nStartHTML:{body_start:09}\r\nEndHTML:{end_html:09}\r\nStartFragment:{start_frag:09}\r\nEndFragment:{end_frag:09}\r\n{content}",
            end_html = body_start + content.len(),
            start_frag = body_start + frag_start,
            end_frag = body_start + frag_end,
        );

        let fragment = extract_html_fragment(html.as_bytes()).expect("fragment");
        assert_eq!(fragment, frag);
    }

    #[test]
    fn dib_24_bpp_decode() {
        let width = 2u32;
        let height = 2u32;
        let mut header = vec![0u8; 40];
        header[0..4].copy_from_slice(&40u32.to_le_bytes());
        header[4..8].copy_from_slice(&(width as i32).to_le_bytes());
        header[8..12].copy_from_slice(&(-(height as i32)).to_le_bytes()); // top-down
        header[12..14].copy_from_slice(&1u16.to_le_bytes());
        header[14..16].copy_from_slice(&24u16.to_le_bytes());
        header[16..20].copy_from_slice(&ffi::BI_RGB.to_le_bytes());

        // 2x2 24-bpp, stride 8: row0 BGR BGR, row1 BGR BGR
        let pixels: [u8; 16] = [
            0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, // red, green
            0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, // blue, black
        ];
        let mut data = header;
        data.extend_from_slice(&pixels);

        let img = parse_dib_to_bgra(&data, u64::MAX).expect("parse");
        assert_eq!(img.width, 2);
        assert_eq!(img.height, 2);
        assert_eq!(img.pixels.len(), 2 * 2 * 4);
        // top-left red BGRA
        assert_eq!(&img.pixels[0..4], &[0xFF, 0x00, 0x00, 0xFF]);
    }

    #[test]
    fn dib_32_bpp_bottom_up() {
        let width = 1u32;
        let height = 2u32;
        let mut header = vec![0u8; 40];
        header[0..4].copy_from_slice(&40u32.to_le_bytes());
        header[4..8].copy_from_slice(&(width as i32).to_le_bytes());
        header[8..12].copy_from_slice(&(height as i32).to_le_bytes()); // bottom-up
        header[12..14].copy_from_slice(&1u16.to_le_bytes());
        header[14..16].copy_from_slice(&32u16.to_le_bytes());
        header[16..20].copy_from_slice(&ffi::BI_RGB.to_le_bytes());

        // bottom row first in memory: white then black when flipped to top-down
        let pixels: [u8; 8] = [0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0xFF];
        let mut data = header;
        data.extend_from_slice(&pixels);

        let img = parse_dib_to_bgra(&data, u64::MAX).expect("parse");
        assert_eq!(img.pixels[0..4], [0x00, 0x00, 0x00, 0xFF]); // top row = former bottom
        assert_eq!(img.pixels[4..8], [0xFF, 0xFF, 0xFF, 0xFF]);
    }

    fn dib_header(width: u32, height: i32, bit_count: u16) -> Vec<u8> {
        let mut header = vec![0u8; 40];
        header[0..4].copy_from_slice(&40u32.to_le_bytes());
        header[4..8].copy_from_slice(&(width as i32).to_le_bytes());
        header[8..12].copy_from_slice(&height.to_le_bytes());
        header[12..14].copy_from_slice(&1u16.to_le_bytes());
        header[14..16].copy_from_slice(&bit_count.to_le_bytes());
        header[16..20].copy_from_slice(&ffi::BI_RGB.to_le_bytes());
        header
    }

    #[test]
    fn dib_oversized_dimensions_rejected_before_allocation() {
        // 50000×50000×4 fits in u64 but would overflow u32×4 in the old path; reject via budget before allocating.
        let data = dib_header(50_000, -50_000, 24);
        let err = parse_dib_to_bgra(&data, 1024).unwrap_err();
        assert!(matches!(
            err,
            ClipError::Other(msg) if msg == "DIB pixel data exceeds size limit"
        ));
    }

    #[test]
    fn dib_u64_dimension_product_overflow() {
        // |i32::MIN| × |i32::MIN| × 4 overflows u64 checked multiplication.
        let data = dib_header(2147483648, i32::MIN, 24);
        let err = parse_dib_to_bgra(&data, u64::MAX).unwrap_err();
        assert!(matches!(err, ClipError::Other(msg) if msg == "DIB dimensions overflow"));
    }

    #[test]
    fn dib_isize_max_dimensions_rejected() {
        // i32::MAX × i32::MAX × 4 fits u64 but exceeds vec capacity (isize::MAX).
        let data = dib_header(i32::MAX as u32, i32::MAX, 24);
        let err = parse_dib_to_bgra(&data, u64::MAX).unwrap_err();
        assert!(matches!(err, ClipError::Other(msg) if msg == "DIB dimensions overflow"));
    }

    #[test]
    fn dib_small_valid_rejected_when_max_bytes_tiny() {
        let mut data = dib_header(2, -2, 24);
        data.extend_from_slice(&[0u8; 16]);
        let err = parse_dib_to_bgra(&data, 8).unwrap_err();
        assert!(matches!(
            err,
            ClipError::Other(msg) if msg == "DIB pixel data exceeds size limit"
        ));
    }
}
