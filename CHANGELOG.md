# Changelog

All notable changes to TrayVault are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

_Planned for 0.2.0: winget manifest, HiDPI, copy-as-plain-text, and remaining polish. See [roadmap.md](roadmap.md)._

## [0.1.1] - 2026-06-02

### Added

- **WIC compressed image blobs** — new captures stored as `TVB1` header + PNG (default) or JPEG payload; Settings → image blob codec and JPEG quality (1–100); existing raw `.dib` files still read
- Page Up / Page Down scroll the history list by one visible viewport

### Changed

- **Image blob disk format** — content-addressed `.dib` files now use WIC compression (typically much smaller than raw BGRA); hash and dedup still use decoded pixel bytes
- **Sharper list thumbnails** for screenshots — raised display caps to 1200×900 px and a taller fit box (`inner_w × 0.85`) so landscape captures (e.g. ~1800×1400) use full card width instead of being height-limited to ~640 px wide

### Fixed

- History scrollbar thumb drag no longer crashes on mouse-up (`STATUS_STACK_BUFFER_OVERRUN`) — `ReleaseCapture()` was removed from the drag-end path (Windows releases capture on button-up), and `WM_CAPTURECHANGED` is deferred until after `WndProc` returns so `App`/`UiState` `RefCell` borrows are not re-entered during input handling
- **Gray blank image cards** when WIC PNG encode failed on persist (`CreateEncoder` / codec not found) — `write_blob` now falls back to legacy raw BGRA on disk so captures still save and thumbnails can load
- **Stuck gray thumbnail placeholders** when the async thumb worker could not read or scale a blob — failed loads clear the in-flight key and post `WM_THUMB_READY` so the next paint retries (covers persist race and missing blobs)
- **Tofu boxes for Unicode hyphens** (U+2010/U+2011/U+2012, e.g. “WIC‑based”) — bundled Roboto can expose a cmap slot that still draws `.notdef`; those code points now rasterize as ASCII `-`

### Performance

- Cache filtered display list until entries, filter chip, or search query change
- Cache full history row layout until entries, filter, query, expand/collapse state, or card width change
- Cache per-entry text card height and pre-wrapped draw lines; at most two word-wrap passes per text row when layout is rebuilt
- Skip full-window repaint on mouse move when hover target is unchanged (history cards, filter chips, settings, close)
- Reuse the persistent glyph cache for filter-chip, search, and context-menu hit-testing (no cold cache per mouse move)
- Cache scaled image in the preview modal across repaints while the modal is open
- Release in-memory image pixels after background persist (thumbnails and preview load from blob on demand)
- Run the 2 s relative-time refresh timer only while the main window is visible
- Reuse the full-frame RGBA paint buffer across repaints (reallocate only on window resize)
- LRU eviction for image thumbnails (64 entries) instead of clearing the entire thumbnail cache when full
- Coalesce wheel-scroll repaints during fast wheel input (~60 Hz cap) while scroll offset still updates every notch
- Faster history list paint and hit-testing on large lists (binary search on cached row layout)

## [0.1.0] - 2026-06-01

### Added

- Clipboard history for text, rich text, and images with deduplication and configurable cap
- Searchable history with filter chips, pin/unpin, and context menu actions
- Borderless main window with custom title bar, themes (light / dark / system), and sticky search header
- Bilinear-filtered image thumbnails and full-screen preview modal
- System tray integration with show/hide and quit
- Global hotkey toggle (default `Alt+V`, configurable)
- Optional Windows autostart via Run key and `--minimized` startup flag
- Settings panel for hotkey, history limits, capture pause, taskbar visibility, and themes
- Local storage under `%LOCALAPPDATA%\TrayVault\` (metadata, image blobs, config, log)
- GitHub Actions CI (`fmt`, `clippy`, `build`, `test`) and release workflow (`.exe`, zip, MSI)
- MSI installer with optional **Start TrayVault when Windows starts** feature (Run key, tray-only launch)
- Roboto font attribution (`NOTICES.md`, `assets/Roboto-LICENSE.txt`)

### Fixed

- Search no longer surfaces image entries when typing — screenshot filenames and source-app metadata had caused false matches on common letters

### Distribution

- **Standalone:** `trayvault.exe` from GitHub Releases
- **Portable zip:** `trayvault-windows-x86_64.zip` (exe + icon + README + LICENSE + font notices)
- **Installer:** `trayvault-windows-x86_64.msi` (per-user install under `%LOCALAPPDATA%\TrayVault`, optional autostart and Start Menu shortcut)

[Unreleased]: https://github.com/OlaProeis/TrayVault/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/OlaProeis/TrayVault/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/OlaProeis/TrayVault/releases/tag/v0.1.0
