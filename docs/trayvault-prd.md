# Product Requirements Document — TrayVault

**Version:** 2.0
**Status:** Draft
**Date:** 2026-05-30

---

## Overview

TrayVault is a minimal clipboard history manager for **Windows**, written in pure Rust and built almost entirely **from scratch**. It captures clipboard history (text, rich text, images), stores it locally, and presents it in a fast, searchable UI accessible via a global hotkey or the system tray.

The project is a deliberate systems-programming exercise: rather than leaning on a GUI framework and a stack of convenience crates, TrayVault hand-writes its Win32 integration (windowing, clipboard, tray, hotkey, autostart) via raw FFI, renders its own immediate-mode UI on a software rasterizer, and implements its own storage, hashing, and config layers. The result should still be a polished, shippable app.

The guiding philosophy: *do one thing and do it exceptionally well, and understand every layer underneath it.* Every design decision is evaluated against two questions: "does this make clipboard history better?" and "can we own this layer ourselves?"

### Why Windows-only

Linux already has many mature, platform-tuned clipboard managers; macOS does too. The real gap — and the most interesting from-scratch target — is a lean, native, dependency-light Windows clipboard manager. Restricting to Windows also removes the most expensive from-scratch work (X11 selection/INCR protocol, Wayland, the macOS Objective-C runtime), leaving a clean, bounded, single-platform codebase. Cross-platform support is explicitly deferred, not designed for.

---

## Goals

- Capture clipboard history (text, rich text, images) automatically while running
- Present history in a clean, searchable, custom-rendered UI via global hotkey or system tray
- Run silently in the background with minimal resource use, event-driven (no polling)
- Be 100% pure Rust, MIT licensed, with no GUI framework and no web renderer
- Build the OS-integration, UI, storage, hashing, and config layers from scratch
- Ship a versioned Windows binary on GitHub Releases

## Non-Goals

- macOS or Linux support (explicitly deferred; do not design abstractions for it now)
- Snippet/template libraries (PasteBar-style boards)
- Cloud sync or cross-device sharing
- Scripting or automation hooks
- Browser extensions
- Any GUI framework (no egui/eframe, no winit, no Electron, no Tauri, no Node.js)
- Any GPU rendering dependency (no wgpu, no glow) — rendering is CPU/software only

---

## Technology Stack

The from-scratch principle means **the logic is ours**; we do not re-implement well-understood pure-Rust *algorithms* (font parsing, 2D rasterization) where doing so would be transcription work or a multi-month rabbit hole with no learning payoff. We do **not** use crates that wrap or hide the operating system — all Win32 access is hand-declared FFI.

### Dependencies (the entire list)

| Crate | Version | License | Role | Why not from scratch |
|-------|---------|---------|------|----------------------|
| `tiny-skia` | latest stable | BSD-3-Clause | CPU 2D rasterizer (rounded rects, AA, clipping, image blit) | High-quality anti-aliasing is a deep algorithmic problem; tiny-skia is small, pure Rust, production-grade. |
| `fontdue` | latest stable | MIT/Apache-2.0 | TrueType parsing + glyph rasterization + naive Latin layout | A correct TTF rasterizer is the single largest from-scratch rabbit hole; explicitly out of scope. |

Everything else is **std-only** and hand-written. No `windows`/`windows-sys`, no `winit`, no `softbuffer`, no `arboard`, no `clipboard-rs`, no `tray-icon`, no `global-hotkey`, no `auto-launch`, no `rusqlite`, no `serde`/`toml`, no `dirs`, no `image`, no `sha2`, no `chrono`, no `anyhow`, no `tracing`.

> All dependency versions are pinned in `Cargo.toml` at the latest stable at project init, and `Cargo.lock` is committed. Run `cargo tree -d` before any bump. Both deps are MIT-compatible, satisfying the MIT-license goal.

### Hand-written subsystems

| Subsystem | Win32 mechanism / approach | Module |
|-----------|----------------------------|--------|
| FFI bindings | Hand-declared `extern "system"` fns, `#[repr(C)]` structs, constants | `win32/ffi.rs` |
| Window + event loop | `RegisterClassW` / `CreateWindowExW` / `WndProc` / `GetMessageW` | `win32/window.rs` |
| Pixel presentation | GDI `CreateDIBSection` + `StretchDIBits` (blit CPU RGBA buffer) | `win32/gdi.rs` |
| Clipboard monitor | `AddClipboardFormatListener` → `WM_CLIPBOARDUPDATE` | `win32/clipboard.rs` |
| Clipboard read/write | `OpenClipboard` / `GetClipboardData` / `SetClipboardData` | `win32/clipboard.rs` |
| System tray | `Shell_NotifyIconW` + `CreatePopupMenu` / `TrackPopupMenu` | `win32/tray.rs` |
| Global hotkey | `RegisterHotKey` → `WM_HOTKEY` | `win32/hotkey.rs` |
| Autostart | `RegSetValueExW` on `HKCU\…\Run` | `win32/autostart.rs` |
| Source-app name | `GetForegroundWindow` → `GetWindowThreadProcessId` → `QueryFullProcessImageNameW` | `win32/clipboard.rs` |
| Content hashing | Hand-rolled SHA-256 | `hash.rs` |
| Storage | Atomically-rewritten metadata file + content-addressed image blobs | `store/` |
| Config | Hand-rolled `key = value` parser | `config.rs` |
| Logging | Minimal file logger | `log.rs` |
| UI | Immediate-mode widgets on tiny-skia + fontdue | `ui/` |

---

## Architecture

```
TrayVault/
├── src/
│   ├── main.rs            # WinMain bootstrap, single message loop
│   ├── app.rs             # App state, event orchestration, view routing
│   ├── models.rs          # ClipEntry, EntryKind, ImageRef
│   ├── hash.rs            # hand-rolled SHA-256
│   ├── config.rs          # hand-rolled key=value parser + defaults
│   ├── log.rs             # minimal file logger
│   ├── win32/
│   │   ├── mod.rs
│   │   ├── ffi.rs         # extern "system" decls, structs, constants
│   │   ├── window.rs      # window class, WndProc, message loop
│   │   ├── gdi.rs         # CreateDIBSection + StretchDIBits present
│   │   ├── clipboard.rs   # listener + read/write + source app
│   │   ├── tray.rs        # Shell_NotifyIconW + popup menu
│   │   ├── hotkey.rs      # RegisterHotKey + WM_HOTKEY
│   │   └── autostart.rs   # registry Run key
│   ├── store/
│   │   ├── mod.rs         # public store API, prune-to-cap, worker thread
│   │   ├── meta.rs        # atomic metadata file (entry index)
│   │   └── blobs.rs       # content-addressed image blob files
│   └── ui/
│       ├── mod.rs
│       ├── render.rs      # tiny-skia Pixmap -> BGRA -> GDI buffer
│       ├── text.rs        # fontdue glyph cache + Latin layout
│       ├── theme.rs       # named color palette (light/dark)
│       ├── widgets.rs     # immediate-mode primitives (card, button, scroll, input)
│       ├── history.rs     # virtualized history list view
│       ├── search.rs      # search bar + filter chips
│       ├── preview.rs     # image preview panel
│       └── settings.rs    # settings view
├── assets/
│   └── icon.ico           # app + tray icon (multi-resolution)
├── .github/workflows/
│   ├── ci.yml             # fmt + clippy + build + test on PR
│   └── release.yml        # build + upload on version tag
├── Cargo.toml
├── Cargo.lock             # committed
├── CHANGELOG.md
├── LICENSE                # MIT
└── README.md
```

### Threading Model

```
Main thread (Win32 message loop — the ONLY UI/OS thread)
    ├── WM_CLIPBOARDUPDATE  → capture pipeline (read, hash, dedup, enqueue)
    ├── WM_HOTKEY           → toggle window visibility
    ├── tray callback msg   → menu / show-hide
    ├── input + WM_PAINT    → immediate-mode UI render via GDI
    └── timer (WM_TIMER)    → relative-time refresh, repaint-on-demand

Storage worker thread
    └── receives owned write/delete jobs via std::sync::mpsc
    └── performs atomic metadata rewrite + blob file IO off the UI thread
    └── reports results back via a channel drained in the message loop
```

Because clipboard, hotkey, and tray all deliver messages to the same `HWND`, there is a **single** message loop and no windowing-framework integration layer. Disk IO is the only thing offloaded, to keep capture and rendering jank-free. The capture pipeline must never block the message loop on disk.

---

## Data Model

### ClipEntry

```rust
pub struct ClipEntry {
    pub id: u64,
    pub created_at: u64,            // Unix epoch milliseconds (std::time::SystemTime)
    pub kind: EntryKind,
    pub text: Option<String>,       // plain text, or plain-text preview of rich text
    pub html: Option<String>,       // raw HTML for RichText entries
    pub image: Option<ImageRef>,    // metadata for Image entries; pixels live in a blob
    pub source_app: Option<String>, // best-effort foreground process name
    pub is_pinned: bool,
    pub hash: [u8; 32],             // SHA-256 of normalized content, for dedup
}

pub enum EntryKind {
    Text,
    RichText, // HTML clipboard content
    Image,
}

pub struct ImageRef {
    pub hash: String,   // hex SHA-256; also the blob filename
    pub width: u32,
    pub height: u32,
}
```

Time is stored as `u64` Unix-millis derived from `SystemTime`; relative formatting ("2 min ago") is computed in `ui/text.rs`. No `chrono`.

### Storage Format

TrayVault does not use SQLite. Two pieces:

1. **Metadata file** — `entries.dat` in the data dir. A versioned, line-oriented text format (one record per line, tab-separated fields, with text fields escaped). Rewritten **atomically** on every change: write to `entries.dat.tmp`, `fsync`, then rename over `entries.dat`. This is corruption-proof for a small (≤ cap) history and human-inspectable for debugging.
2. **Image blobs** — `blobs/<hex-sha256>.dib` files. Images are stored as raw pixel data (the clipboard already provides decoded DIB pixels — no PNG/DEFLATE needed). Content-addressed by hash, so image dedup is "does the file already exist?".

```
%LOCALAPPDATA%\TrayVault\
├── entries.dat
├── entries.dat.tmp        (transient)
├── config.toml
├── trayvault.log
└── blobs\
    └── <hex-sha256>.dib
```

The metadata format carries a header line with a schema version. Schema changes bump the version; the loader migrates forward. On unreadable/garbled data, back up to `entries.dat.bak` and start fresh (never crash).

---

## Features

### F1 — Clipboard Capture

- Monitor via `AddClipboardFormatListener`; handle `WM_CLIPBOARDUPDATE` (event-driven, no polling)
- On change, read available formats: `CF_UNICODETEXT` (text), `"HTML Format"` registered format (rich text), `CF_DIBV5`/`CF_DIB` (images — raw pixels, no decoder needed)
- Compute SHA-256 of normalized content; if it matches the most recent entry, discard
- Global deduplication toggle: optionally skip if the hash exists *anywhere* in history
- **Sensitive-content safety:** before capturing, check the registered clipboard formats `CanIncludeInClipboardHistory` and `ExcludeClipboardContentFromMonitorProcessing`. If present and indicating exclusion, **do not capture** (respects password managers/browsers). This is a correctness requirement, not optional.
- Capture source application name: `GetForegroundWindow` → `GetWindowThreadProcessId` → `OpenProcess` → `QueryFullProcessImageNameW`, reduced to the executable file name
- Do **not** capture changes originating from TrayVault's own copy-back (track an internal "we just set the clipboard" flag and/or compare the clipboard sequence number)
- Configurable entry cap (default 500). When exceeded, delete oldest non-pinned entries (and their orphaned blobs)
- Images above `max_image_size_mb` are discarded before storage

### F2 — History UI (custom-rendered)

- Main window is a vertically scrolling list of clipboard entries, drawn with tiny-skia and presented via GDI
- Each entry ("card") shows:
  - **Text:** first 2–3 lines, truncated with ellipsis
  - **Rich text:** plain-text preview (tags stripped for display)
  - **Image:** thumbnail (max 120px height) + dimensions label
  - Relative capture time ("2 min ago", "yesterday")
  - Source app name if available (small muted label)
  - Pin indicator for pinned entries
- Pinned entries pinned to the top, separated by a divider
- Clicking a card copies it back to the clipboard immediately; optionally hides the window (`close_on_copy`)
- Right-click context menu on a card: Copy, Pin/Unpin, Delete
- Double-click an image card opens the larger preview panel
- Empty state: friendly message explaining entries will appear as you copy
- **Virtualized rendering:** only visible cards are laid out and drawn, so large histories stay at 60 FPS

### F3 — Search

- Search bar at the top of the history view, focused automatically when the window opens
- Real-time substring filtering as the user types (case-insensitive) over text/HTML-preview/source-app; image entries are excluded while the query is non-empty (browse images via the **Images** chip with an empty search bar)
- Filter chips: All / Text / Images / Pinned
- Keyboard navigation: `↑`/`↓` move selection, `Enter` copies the selected entry, `Esc` clears search then closes the window
- Text input is Latin-focused (fontdue); full IME/international input is out of scope for v0.1

### F4 — System Tray

- App runs in the tray when the window is hidden; no taskbar button while hidden (use a tool window / `WS_EX_TOOLWINDOW`-style approach so it stays out of Alt-Tab and the taskbar)
- Tray icon via `Shell_NotifyIconW`, receiving its callback message in the shared `WndProc`
- Left-click: show/hide the main window
- Right-click context menu (`CreatePopupMenu` + `TrackPopupMenu`):
  - Show TrayVault
  - Pause/Resume capture (toggles the listener)
  - Settings
  - Quit

### F5 — Global Hotkey

- Default: `Alt+V` (configurable)
- `RegisterHotKey` against the app window; `WM_HOTKEY` toggles window show/hide from any app
- On registration conflict, show a one-time tray balloon notification and continue running without the hotkey
- Hotkey string is parsed by a small hand-written parser into Win32 modifier flags + virtual-key code

### F6 — Auto-Startup

- Settings toggle: "Start TrayVault when I log in"
- Writes/removes `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run\TrayVault` via `RegSetValueExW`/`RegDeleteValueW`
- The registered command includes a `--minimized` flag; when launched with it, the app starts hidden to the tray
- On failure (e.g., permission), surface the error in the settings view

### F7 — Light / Dark Theme

- Light and dark palettes defined in `ui/theme.rs` (not borrowed defaults — we render everything ourselves anyway)
- Three options: Light, Dark, Follow system
- System detection reads `HKCU\…\Themes\Personalize\AppsUseLightTheme` from the registry (no `dark-light` crate)
- Palette guidance:
  - Warm off-white surfaces in light mode
  - True dark (not pure black) surfaces in dark mode
  - Single accent color (teal or slate blue — decide during implementation)
  - Muted secondary text color
  - Clear visual distinction between pinned and unpinned cards

### F8 — Settings Panel

Settings stored at `%LOCALAPPDATA%\TrayVault\config.toml`, parsed by a hand-written `key = value` reader (a minimal TOML subset — top-level keys only, no tables/arrays needed).

| Setting | Type | Default | Notes |
|---------|------|---------|-------|
| `max_entries` | u32 | 500 | History cap before oldest non-pinned entries are pruned |
| `deduplicate_global` | bool | false | Skip entry if hash exists anywhere in history |
| `hotkey` | String | `"Alt+V"` | Parsed into Win32 modifiers + VK |
| `autostart` | bool | false | Toggles the registry Run key |
| `theme` | Enum | `System` | `Light`, `Dark`, `System` |
| `capture_images` | bool | true | Toggle image capture |
| `capture_rich_text` | bool | true | Toggle HTML clipboard capture |
| `close_on_copy` | bool | true | Hide window after copying an entry |
| `pause_capture` | bool | false | Temporarily disable the listener |
| `max_image_size_mb` | f32 | 5.0 | Discard images above this threshold |
| `window_client_w` | u32 | 900 | Last main-window client width (pixels) |
| `window_client_h` | u32 | 640 | Last main-window client height (pixels) |
| `window_x` | i32 (optional) | omitted | Last outer-window screen X; written with `window_y` after user move |
| `window_y` | i32 (optional) | omitted | Last outer-window screen Y; written with `window_x` after user move |

### F9 — Help Overlay

- A lightweight overlay listing the hotkeys and basic usage, toggled with `F1` or `?`
- Rendered as a modal panel over the history view; dismissed with `Esc`

---

## Help / Hotkey Reference

| Action | Key |
|--------|-----|
| Show/hide window (global) | `Alt+V` (configurable) |
| Move selection | `↑` / `↓` |
| Copy selected entry | `Enter` |
| Clear search / close window | `Esc` |
| Open help overlay | `F1` or `?` |
| Pin/unpin selected | `Ctrl+P` |
| Delete selected | `Delete` |

---

## Platform Support

| Feature | Windows 10/11 (x86_64) |
|---------|------------------------|
| Text capture | ✅ |
| Image capture | ✅ (DIB) |
| Rich text (HTML) | ✅ |
| Source-app tracking | ✅ |
| Sensitive-content exclusion | ✅ |
| System tray | ✅ |
| Global hotkey | ✅ |
| Auto-startup | ✅ (registry) |
| Light/dark system theme | ✅ (registry) |

Target: Windows 10 1809+ and Windows 11, `x86_64-pc-windows-msvc`. (ARM64 Windows deferred.)

---

## Performance Targets

- **Memory:** < 50 MB RSS at rest with up to 500 text entries; < 150 MB with image-heavy history (no GPU context to inflate this)
- **CPU:** ~0% idle — fully event-driven; no busy polling anywhere. Repaint only on input, capture, or the relative-time timer
- **Startup:** < 300 ms to tray icon visible from cold launch
- **UI responsiveness:** 60 FPS during interaction; history list virtualizes layout and drawing (only visible cards)
- **Image storage:** raw DIB pixels stored as content-addressed blobs; images > `max_image_size_mb` discarded; thumbnails decoded and bilinear-downscaled once, then cached in memory

---

## Error Handling & Resilience

- All Win32 FFI is wrapped in safe Rust functions that translate failures into a `ClipError` enum; the message loop never unwraps on OS calls
- If clipboard access fails (another app holds the lock), retry once after 100 ms, then silently discard — never crash
- If a storage write fails, log it and keep the in-memory entry; retry on next change — the listener must not halt
- If hotkey registration fails at startup, show a one-time tray notification and continue
- If autostart registry write fails, show the error in the settings view
- If the metadata file is unreadable or a schema version is unknown and unmigratable, back up to `entries.dat.bak` and start fresh
- A top-level catch in `WinMain` logs panics to `trayvault.log` before exit

---

## Release & Distribution

### GitHub Releases

Each tagged release (`v0.x.0`) triggers GitHub Actions to build and upload:

| Artifact | Target | Notes |
|----------|--------|-------|
| `trayvault-windows-x86_64.zip` | `x86_64-pc-windows-msvc` | Bundled `.exe` + icon |

`cargo install trayvault` support (crates.io publish) is deferred; GitHub Releases only for v0.1.

### CI Pipeline (GitHub Actions)

- `ci.yml` (on PR / push): `cargo fmt --check`, `cargo clippy -D warnings`, `cargo build`, `cargo test` on `windows-latest`
- `release.yml` (on tag `v*`): build `--release` for `x86_64-pc-windows-msvc`, zip the `.exe` + icon, upload to the release

```yaml
on:
  push:
    tags: ['v*']

jobs:
  build:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: x86_64-pc-windows-msvc
      - run: cargo build --release --target x86_64-pc-windows-msvc
      # zip + upload steps
```

### Versioning

Semantic versioning `MAJOR.MINOR.PATCH`; pre-1.0 uses `0.MINOR.PATCH`. `CHANGELOG.md` maintained manually per release.

### README Minimum Contents

- What TrayVault is (one paragraph) + that it is Windows-only and built from scratch
- Screenshot / GIF of the UI
- Installation (download the zip from Releases)
- Build from source (`cargo build --release`)
- Configuration file location (`%LOCALAPPDATA%\TrayVault\config.toml`)
- Hotkey reference
- License (MIT)

---

## Open Questions / Decisions Deferred

1. ~~**Window positioning:**~~ **Resolved:** remember last client size and screen position in `config.toml`; restore on startup; save on `WM_EXITSIZEMOVE` and quit. See `docs/technical/config.md` (Window placement).
2. **HiDPI:** per-monitor DPI awareness (`SetProcessDpiAwarenessContext`) and scaling of the tiny-skia render target. Decide the DPI strategy before the UI milestone hardens.
3. ~~**Thumbnail downscaling quality:**~~ **Resolved:** bilinear filtering via `scale_bilinear_rgba` in `pixmap.rs` for history thumbnails (`thumb_cache.rs`) and preview modal (`preview.rs`). Same output dimensions and cache keys as before; `scale_nearest_rgba` retained for tests only. See `docs/technical/pixmap-rasterizer.md`.
4. **Metadata format:** line-oriented text (chosen for v0.1, debuggable) vs. a compact binary format later if profiling shows rewrite cost matters at the 500-entry cap.
5. **crates.io publish / `cargo install`:** deferred; revisit after v0.1.

---

## Milestones

| Milestone | Deliverables |
|-----------|--------------|
| **M1 — Win32 Foundation** | Hand-declared FFI module; window class + single message loop; GDI present a test buffer; error type; minimal file logger |
| **M2 — Clipboard Capture** | Clipboard listener; read text/HTML/DIB; hand-rolled SHA-256; dedup; sensitive-content skip; source-app name; in-memory history |
| **M3 — Storage & Config** | Atomic metadata file; content-addressed blob store; storage worker thread; config parser; load-on-startup; prune-to-cap |
| **M4 — UI Core** | tiny-skia→GDI render pipeline; fontdue glyph cache + Latin layout; theme palettes; immediate-mode widget primitives; virtualized history list; relative time |
| **M5 — Interaction** | Search + filter chips; keyboard nav; click-to-copy (ignore own change); context menu; pin/unpin/delete; image preview; empty state; help overlay |
| **M6 — System Integration** | Tray icon + menu; global hotkey toggle; autostart registry; pause/resume; settings view wired; `--minimized` flag; light/dark detection |
| **M7 — Release** | Multi-resolution icon; README; CHANGELOG; `ci.yml` + `release.yml`; zipped Windows binary; `v0.1.0` tag |
