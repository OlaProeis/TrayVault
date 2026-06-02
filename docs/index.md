# Documentation Index

## Index Rules
- This file is a **documentation map only** — no project history, task lists, or architecture overviews.
- Each entry gets a **one-line description**; keep descriptions short and factual.
- When adding a doc under `docs/` or `docs/technical/`, add its entry here immediately.
- Do not duplicate content from other docs; link by filename only.

## Core Context
- `ai-context.md` - Core project architecture, rules, conventions, search bar, bilinear image thumbnails, history card hover/hit-test, title-bar drag, borderless DWM frame handling, and taskbar visibility (`show_in_taskbar`).
- `../roadmap.md` - Release roadmap (0.1.1, 0.2.0, deferred items); complements `CHANGELOG.md`.

## Technical Docs
- `technical/win32-ffi.md` - Hand-declared Win32 FFI surface, ABI conventions, safe-wrapper pattern, and layout APIs (`GetWindowRect`, `GetSystemMetrics`, `IsIconic`).
- `technical/error-handling.md` - The `ClipError` type, `Result` alias, and the OS-error translation rule.
- `technical/app-lifecycle.md` - Startup bootstrap, tray quit sequence, window placement capture on shutdown, idempotent post-loop teardown, and clean exit (code 0 + storage worker join).
- `technical/logging.md` - Pure-std file logger, log location/format, size-based rotation, hot-path message filtering, and panic-hook integration.
- `technical/window-gdi.md` - Main window class, message loop, WndProc dispatch, GDI DIB presentation, window icon (`WM_SETICON` small+large via `load_app_icon()`), persisted window placement (size/position in config), borderless DWM frame (no gray NC border), title-bar `WM_NCHITTEST` drag (search field + gaps via `HTCAPTION`; settings/close `HTCLIENT`), `WM_NCLBUTTONUP` search click recovery, optional taskbar button via `show_in_taskbar`, and visibility-gated 2 s relative-time timer.
- `technical/message-loop-callbacks.md` - `AtomicPtr` active window, HWND-based tray/hotkey/close/geometry callbacks, aliasing-safe `WndProc` dispatch pattern, and deferred `WM_CAPTURECHANGED` for scrollbar capture (no `ReleaseCapture` from input handlers).
- `technical/clipboard-capture.md` - Clipboard listener, format readers/writers, sensitive-content skip, and copy-back.
- `technical/clipboard-hardening.md` - DIB decode size bounds before allocation and copy-back `GlobalFree` on `SetClipboardData` failure.
- `technical/app-orchestration.md` - App state, history cap pruning, dedup, storage coordination, image pixel lifecycle after capture, window geometry persistence (`persist_window_geometry`, shutdown capture), and message-loop hooks.
- `technical/hashing-dedup.md` - Hand-rolled SHA-256, content normalization, and capture deduplication rules.
- `technical/storage.md` - Metadata file, `MoveFileExW` atomic `entries.dat` replace, content-addressed blob store (TVB1/WIC + legacy raw), background persistence worker, and in-memory `image_pixels` release after capture persist.
- `technical/compressed-blob-storage.md` - TVB1 on-disk header, WIC encode/decode (PNG default, JPEG optional), legacy raw read path, WIC-failure raw write fallback, config keys, and worker threading.
- `technical/compressed-blob-storage-assessment.md` - Task 34 research: WIC vs GDI+, PNG/JPEG tradeoffs, proposed `TVB1` blob header, migration/dedup/read-path analysis, and recommended implementation scope (no code).
- `technical/config.md` - Hand-rolled `config.toml` parser, settings struct (including `show_in_taskbar` and window placement keys), and load/save behavior.
- `technical/pixmap-rasterizer.md` - Hand-rolled RGBA8 `Pixmap`, BGRA→RGBA for clipboard images, solid fill, blit, and bilinear image scale (replaces tiny-skia).
- `technical/rendering.md` - UI render pipeline, GDI glyph rasterization (baseline-aligned `TextOutW` + mandatory `GdiFlush`), themes, and BGRA handoff to GDI.
- `technical/render-performance.md` - Paint-path direct BGRA handoff, persistent RGBA scratch buffer on `UiState`, hover repaint gating, wheel-scroll repaint coalescing (~60 Hz), and borrow-based `GlyphCache` lookups.
- `technical/history-list-performance.md` - Wheel-scroll repaint throttle and binary-search viewport culling / hit-test on cached `EntryLayout` rows.
- `technical/ui-perf-caches.md` - Display-indices dirty-key cache (`entries_version` + filter + query), list-layout cache (`cached_list_layout` + expand/width invalidation), binary-search `visible_layout_range` / `hit_test_entry`, per-entry text row layout cache (`entry_height_cache` with pre-wrapped draw lines), hover repaint gating (`HoverKey` on `MouseMove`), persistent `glyph_cache` reuse for input hit-tests, and single-slot image preview pixmap cache.
- `technical/text-card-layout.md` - Unified `text_card_layout` (at most two wrap passes), `EntryLayout` draw-line handoff, and per-entry layout cache pairing with list-layout rebuild.
- `technical/ui-views.md` - Immediate-mode widgets, virtualized history, history list scrollbar (auto-hide, thumb drag, NC hit-test override), history card hover/hit-test alignment, image thumbnail and preview-modal scale/cache, sticky header, search focus/caret/selection/scroll (`search_edit.rs`, 14px metrics, Ctrl+A, Shift+arrows, caption-drag click path), pinning/context menu, mouse-up hit testing, and input routing.
- `technical/thumbnail-cache.md` - Pre-downscaled history thumbnails (1200×900 caps, taller fit box), LRU eviction, cache-first lookup, and `(entry_id, dst_w, dst_h)` keys.
- `technical/async-thumbnail-loading.md` - Background thumb-loader worker, `WM_THUMB_READY` repaint wakeup, in-flight/stale-reply tracking, failed-load retry, and non-blocking disk miss path in `draw_thumbnail`.
- `technical/blob-reference-counting.md` - Reference-aware blob delete when multiple entries share a content-addressed `.dib` file.
- `technical/system-tray.md` - Tray icon from embedded `assets/icon.ico` bytes (clipboard+padlock design; stock fallback), `load_app_icon()` shared with main window `WM_SETICON`, NOTIFYICON_VERSION_4 callback parsing, left-click toggle debounce (`GetTickCount`, 400 ms), context menu placement, show/hide behavior, taskbar visibility when open, balloon notifications, and quit entry point (see app-lifecycle.md for teardown).
- `technical/global-hotkey.md` - Global hotkey string parser, RegisterHotKey registration, conflict handling, and WM_HOTKEY toggle.
- `technical/autostart-startup.md` - Run-key autostart registration, `--minimized` startup flag, settings toggle, and registry sync at launch.
- `technical/settings-panel.md` - Settings overlay controls, config binding (including show-in-taskbar toggle), About section (version + GitHub link), persistence, hotkey re-register, and pause/tray sync.
- `technical/crate-extraction-assessment.md` - Pros and cons of extracting pixmap, GDI glyph rasterizer, UI stack, Win32 FFI, and other modules into standalone crates.
- `technical/release-pipeline.md` - GitHub Actions CI and release workflows, `build.rs` exe icon embedding, WiX MSI (optional Start-with-Windows feature), and release artifacts.
