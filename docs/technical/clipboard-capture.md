# Clipboard Capture Pipeline

Modules: `src/models.rs`, `src/win32/clipboard.rs`. History orchestration: `src/app.rs` (see `app-orchestration.md`).

## Data model (`models.rs`)

- **`ClipEntry`** — id, `created_at` (Unix millis), `kind`, optional `text` / `html`, optional `ImageRef`, in-memory `image_pixels`, `source_app`, `is_pinned`, `hash` (SHA-256; see `hashing-dedup.md`).
- **`EntryKind`** — `Text`, `RichText`, `Image`.
- **`ImageRef`** — hex content hash, width, height.

## Listener (`clipboard.rs`)

- **`ClipboardMonitor::register_listener`** — `AddClipboardFormatListener(hwnd)` on the main window.
- **`try_capture`** — invoked from `WindowCallbacks::on_clipboard_update` on `WM_CLIPBOARDUPDATE`; returns `Option<ClipEntry>` for `App::on_clipboard_captured`.

### Capture flow

1. Skip if `pause_capture` or self-originated update (`mark_own_write` before copy-back).
2. `OpenClipboard(hwnd)` with one ~100 ms retry; log and drop on failure.
3. Sensitive-content check via `CanIncludeInClipboardHistory` (skip when value is `"0"`) and `ExcludeClipboardContentFromMonitorProcessing` (skip when format is present).
4. Read formats: `CF_UNICODETEXT`, `"HTML Format"` (if `capture_rich_text`), `CF_DIBV5` / `CF_DIB` (if `capture_images`, subject to `max_image_size_bytes`).
5. Build `ClipEntry` (compute SHA-256); return to app layer for dedup and storage.
6. `CloseClipboard`.

## Copy-back

- **`write_entry_to_clipboard`** — `EmptyClipboard`, then set formats appropriate to `EntryKind`.
- **`set_clipboard_or_free`** — shared `SetClipboardData` wrapper; `GlobalFree`s the handle on failure (ownership transfers to the OS only on success).
- **`encode_bgra_dib`** — top-down 32-bpp DIB for `CF_DIB` image copy-back.
- Self-capture suppression: call `mark_own_write()` immediately before writing.

## Readers

| Function | Format | Notes |
|----------|--------|-------|
| `read_text` | `CF_UNICODETEXT` | `GlobalLock` → UTF-16 → UTF-8 |
| `read_html` | `"HTML Format"` | Byte offsets from `StartFragment`/`EndFragment` (fallback `StartHTML`/`EndHTML`) |
| `read_image` | `CF_DIBV5`, `CF_DIB` | `parse_dib_to_bgra` → top-down BGRA (24/32/8 bpp); rejects overflow/oversize dimensions before allocating; skipped when decoded size exceeds `max_image_size_bytes` |

Captured image pixels stay **BGRA** on the clipboard path, on disk (`blobs/*.dib`), and on copy-back (`encode_bgra_dib`). After capture, `ClipEntry::image_pixels` holds decoded bytes only until `App::on_clipboard_captured` enqueues persist; the field is then cleared and UI/copy-back load via `Store::read_blob` when needed. Display converts BGRA → RGBA in [`thumb_cache.rs`](../../src/ui/thumb_cache.rs) and [`preview.rs`](../../src/ui/preview.rs) — see [`pixmap-rasterizer.md`](pixmap-rasterizer.md) and [`storage.md`](storage.md).

## Config

`CaptureConfig` is built from `Config::capture_config()` (`capture_images`, `capture_rich_text`, `pause_capture`, `deduplicate_global`, `max_image_size_bytes`).

## Source app

`GetForegroundWindow` → `GetWindowThreadProcessId` → `OpenProcess` → `QueryFullProcessImageNameW` → executable name only (`executable_name_from_path`).

## Unit tests

Portable logic in `clipboard.rs` `#[cfg(test)]`: DIB decode/encode, HTML fragment extraction, executable name parsing.
