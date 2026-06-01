# TrayVault - AI Context

## Rules (DO NOT UPDATE)
- Never auto-update this file or current-handover-prompt.md — only update when explicitly requested.
- Only do the task specified, do not start the next task, or go over scope.
- Run `cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check` after changes to verify.
- Follow existing code patterns and conventions.
- Document by feature (e.g., `clipboard-watcher.md`), not by task.
- Update `docs/index.md` when adding new documentation.
- Use Context7 MCP tool to fetch library documentation when needed (resolve library ID first, then fetch docs).

## Project Identity
TrayVault is a **Windows-only** clipboard history manager written in **pure Rust, almost entirely from scratch**. See `trayvault-prd.md` (v2.0) for the authoritative spec. Goal: lean, native, dependency-light; own every layer.

## Tech Stack
- **Language:** Rust 2021, MSRV 1.85, target `x86_64-pc-windows-msvc`.
- **OS integration:** hand-declared Win32 FFI (NO `windows`/`windows-sys`, NO `winit`). Window, message loop, GDI present, clipboard, tray, hotkey, autostart, registry — all by hand.
- **Zero third-party deps:** glyph rasterization is Win32 GDI (`src/win32/glyph_raster.rs`, bundled Roboto via `AddFontMemResourceEx`); UI pixels are hand-rolled (`src/ui/pixmap.rs`). `fontdue` removed in Task 16.
- **From scratch:** SHA-256, storage, config parser, logging, immediate-mode UI.

## Architecture
Single Win32 message loop on the main thread owns the window AND receives clipboard (`WM_CLIPBOARDUPDATE`), hotkey (`WM_HOTKEY`), and tray callback messages — no windowing framework. Disk IO is offloaded to a storage worker thread via `std::sync::mpsc`. UI is immediate-mode: `pixmap.rs` (RGBA fills/blits) + GDI-rasterized glyphs → in-place RGBA→BGRA into the GDI DIB (`write_rgba_to_bgra`) → `StretchDIBits`. Event-driven (no busy polling); a 2 s `WM_TIMER` refreshes relative-time labels **only while the window is visible** (`show_window` / `hide_window` in `window.rs`).

**App layer:** `App` owns most-recent-first history, config, dedup (`hash_index`), cap pruning, and storage job enqueue. Clipboard monitor reads/writes formats only. Pin state lives on `ClipEntry::is_pinned`; `App::toggle_pin` flips it and enqueues persist. Cap prune never removes pinned rows.

**UI input (immediate-mode):** Actions run on `LButtonUp`, not during paint (`begin_frame` clears `mouse_left_pressed`). Right-click menu opens on paint while `mouse_right_down`; item clicks use left **up**. Do **not** clear `UiState::context_menu` on every `LButtonDown` — only when the press is outside the menu (`dismiss_context_menu_unless_hit`), or Pin/Copy/Delete never fire. Shared menu labels: `widgets::context_menu_labels`. Details: `docs/technical/ui-views.md` (Pinning section).

**Text rendering:** GDI glyph rasterizer in `src/win32/glyph_raster.rs` (bundled Roboto, Segoe UI fallback). Each glyph is drawn baseline-aligned (`TextOutW` + `TA_BASELINE`) onto an offscreen 32-bpp `CreateDIBSection` (null HDC → guaranteed 32-bpp), then trimmed to its ink box; `GlyphCache` in `src/ui/text.rs` caches by `(char, size)` and blends alpha8 into the RGBA pixmap. Lookups return borrows (no bitmap clone on hit); `advance()` serves metric-only paths. **Gotcha:** GDI batches drawing into DIB sections, so `GdiFlush()` is **mandatory** after `TextOutW` and before reading the bits — skipping it yields "shredded" text with dropped scanlines. Details: `docs/technical/rendering.md`, `docs/technical/render-performance.md`.

**Data model:** `ClipEntry { id, created_at: u64 millis, kind, text, html, image: ImageRef, source_app, is_pinned, hash: [u8;32] }`; `EntryKind { Text, RichText, Image }`. Image entries hold `image_pixels` only briefly after capture until `enqueue_persist` (worker owns a clone for blob write); then pixels load from `blobs/` via `Store::read_blob`. Dedup via SHA-256 in `hash.rs`.

**Storage:** atomic line-oriented `entries.dat` (tmp write + `MoveFileExW` replace) + content-addressed image blobs in `blobs/<hex-sha256>.dib`. Worker API: `Store::load_initial()`, `enqueue_persist()`, `enqueue_delete()`. Blob delete is reference-aware when entries share a hash (`App::enqueue_removed_entry_delete`). No SQLite. Data dir: `%LOCALAPPDATA%\TrayVault\`.

**Search bar (immediate-mode, no native edit control):** Filter text on `App::filter_query`; focus/caret/selection on `UiState` (`search_focused`, `search_caret`, `search_sel_anchor`, `search_input_rect`). Layout in `search_edit.rs` (14px metrics aligned with `input_box`, horizontal scroll when overflowing, selection highlight). On show, search is focused with caret and anchor at end of query (`show_main_window` in `main.rs`). Title bar in `titlebar.rs`; filter chrome in `search.rs`. `HTCAPTION` drag + `WM_NCLBUTTONUP` click recovery (≤4px slop → synthetic `LButtonUp`). Clicks on **mouse-up** only. **Keys (search focused):** Left/Right/Home/End (Shift extends selection), Delete/backspace on selection or codepoint, Ctrl+A select all; ↑/↓ still move the filtered list. **Query matching:** case-insensitive substring on text/HTML/source app; **image entries hidden while query is non-empty** (empty query still shows images per filter chip). **Sticky header:** `search::draw_sticky_header` after history list. Details: `docs/technical/ui-views.md`.

**Image list thumbnails:** Scale-to-fit card inner width with hard caps `MAX_THUMB_WIDTH` 800px and `MAX_THUMB_HEIGHT` 520px in `history.rs` (both dimensions matter for landscape shots). Pre-downscale via **bilinear** `scale_bilinear_rgba` in `pixmap.rs`, cached in `thumb_cache.rs` keyed by `(entry_id, dst_w, dst_h)` on `UiState::thumb_cache`; paint checks `ThumbCache::get()` before disk blob reads. **Image preview modal:** single-slot scaled pixmap cache in `UiState::preview_cache` (`preview.rs`); same bilinear downscale; keyed by `(entry_id, dst_w, dst_h)`, cleared on Esc. Details: `docs/technical/thumbnail-cache.md`, `docs/technical/ui-perf-caches.md`, `docs/technical/ui-views.md`, `docs/technical/pixmap-rasterizer.md`.

**History card hover / hit-test:** Hover highlight is drawn during paint via `card()` + `pointer_over`. `hit_test_entry()` in `history.rs` must use the **same card bounds** as draw (`content_x`, `content_w`, `layout.height - 4`) — not the full layout row — or hover lags when moving up (old row still “hit” while cursor left the visible card). `MouseMove` on the main view always requests repaint so highlight tracks the cursor every frame; `HoverKey` is still updated but no longer gates repaints. Row copy, double-click preview, and hover all share this hit test. Details: `docs/technical/ui-views.md` (History card hover).

**Borderless title bar:** Drag via `WM_NCHITTEST` → `HTCAPTION` in `window.rs` (28px band; search field + gaps draggable; settings and close buttons stay `HTCLIENT`). Search clicks without drag use `WM_NCLBUTTONDOWN`/`WM_NCLBUTTONUP` + 4px slop → synthetic `LButtonUp`. Do **not** call `SendMessageW(SC_MOVE)` from input while `RefCell` borrows are active — re-enters `WndProc` and panics. **No gray frame:** `WS_POPUP | WS_THICKFRAME` + DWM can flash a system-color NC band on drag or tray focus (`SetForegroundWindow`). Suppress via `WM_NCCALCSIZE` (client rect = proposed window rect), `WM_NCACTIVATE` → `1`, `hbrBackground = 0`, and `DwmSetWindowAttribute(DWMWA_NCRENDERING_POLICY, DWMNCRP_DISABLED)` + `DwmExtendFrameIntoClientArea`. Details: `docs/technical/window-gdi.md` (Borderless frame section).

**Taskbar visibility (`show_in_taskbar`):** Main window is created with `WS_EX_TOOLWINDOW` (tray-first; hidden = no taskbar button, out of Alt+Tab). Config key `show_in_taskbar` (default `true`) controls whether the button appears **while the window is visible**. `show_window` / `hide_window` in `window.rs` call `set_taskbar_button_visible()` and start/stop the relative-time timer. All hide paths use `win32::window::hide_window` (including `input.rs`, `hide_main_window` in `main.rs`). Settings toggle applies immediately when the window is open. Details: `docs/technical/window-gdi.md` (Taskbar visibility, Relative-time timer).

## Conventions
- **Modularity:** one feature per file; Win32 bindings grouped by DLL in `src/win32/ffi.rs`.
- **Handles:** modeled as `isize` (null = 0). `extern "system"`, wide (`*W`) APIs only.
- **Errors:** `Result<T>`/`ClipError`; translate Win32 failures via `last_error(api)` immediately. Never panic on OS failure — log and recover.
- **License:** MIT throughout.

## Where Things Live
| Want to... | Look in... |
|------------|------------|
| Raw Win32 bindings (types/structs/consts/fns) | `src/win32/ffi.rs` |
| Safe wrappers, `wide()`, `last_error()` | `src/win32/mod.rs` |
| Error type | `src/error.rs` |
| File logger (rotation, hot-path filtering) | `src/log.rs`, `docs/technical/logging.md` |
| Clean quit / post-loop teardown | `src/main.rs` (`quit_app`, `quitting` guard), `docs/technical/app-lifecycle.md` |
| Entry point, panic hook, bootstrap | `src/main.rs` |
| Window class + message loop + WndProc | `src/win32/window.rs`, `docs/technical/{window-gdi,message-loop-callbacks}.md` |
| GDI DIB back-buffer + present | `src/win32/gdi.rs` |
| Clipboard capture + copy-back | `src/win32/clipboard.rs`, `docs/technical/{clipboard-capture,clipboard-hardening}.md` |
| App state / orchestration | `src/app.rs` |
| Data types | `src/models.rs` |
| Storage | `src/store/`, `docs/technical/{storage,blob-reference-counting}.md` |
| Config | `src/config.rs` |
| SHA-256 / dedup | `src/hash.rs` |
| RGBA pixmap fill/blit/scale (bilinear image downscale) | `src/ui/pixmap.rs`, `docs/technical/pixmap-rasterizer.md` |
| UI render / text / theme | `src/ui/{render,text,theme}.rs`, `docs/technical/render-performance.md` |
| GDI glyph rasterization (font load, `TextOutW`, `GdiFlush`) | `src/win32/glyph_raster.rs`, `docs/technical/rendering.md` |
| UI widgets / views / input | `src/ui/{widgets,history,search,search_edit,titlebar,preview,settings,settings_input,input,mod}.rs` |
| Image list thumbnails (scale, caps, cache-first lookup) | `src/ui/{history,thumb_cache}.rs`, `docs/technical/thumbnail-cache.md` |
| History card hover / row hit-test | `src/ui/{history,input,widgets}.rs`, `docs/technical/ui-views.md` (History card hover) |
| Title bar drag / borderless frame / `WM_NCHITTEST` | `src/win32/window.rs`, `src/ui/titlebar.rs`, `docs/technical/window-gdi.md` |
| Taskbar button when open (`show_in_taskbar`) | `Config::show_in_taskbar`, `src/win32/window.rs` (`set_taskbar_button_visible`), `src/main.rs` (`show_main_window`), `src/ui/{settings,settings_input,input}.rs`, `docs/technical/window-gdi.md` |
| Search focus, caret, selection, scroll, filter query, display-indices cache | `src/ui/{search,search_edit,titlebar,render,input,mod}.rs`, `App::{filter_query,entries_version}`, `docs/technical/{ui-views,ui-perf-caches}.md` |
| UI perf caches (display indices, glyph hit-tests, preview pixmap) | `src/ui/{search,mod,preview,input}.rs`, `src/app.rs`, `docs/technical/ui-perf-caches.md` |
| Pin / unpin, context menu actions | `src/app.rs` (`toggle_pin`), `src/ui/input.rs`, `src/ui/widgets.rs` (`context_menu_labels`, `hit_test_context_menu`) |
| Tray / hotkey / autostart | `src/win32/{tray,hotkey,autostart}.rs`, `docs/technical/system-tray.md` (embedded icon, `load_app_icon()`) |
| App icon (tray + taskbar/Alt+Tab) | `assets/icon.ico` (embedded), `src/win32/tray.rs` (`load_app_icon`, `EMBEDDED_ICON`), `src/win32/window.rs` (`WM_SETICON`), `docs/technical/system-tray.md`, `docs/technical/window-gdi.md` (Window icon) |
| CI / release pipelines | `.github/workflows/` *(Task 14)* |

## Status
- **Done:** Tasks 1–13, 15–28 — Win32 FFI through GDI text rasterization, clean shutdown (exit 0), log rotation/hot-path filtering, thumbnail cache-first lookup, reference-aware blob deletion, clipboard DIB size bounds, copy-back handle cleanup, atomic `entries.dat` via `MoveFileExW`, embedded-only tray icon load, message-loop callback soundness (`AtomicPtr` + HWND helpers), search caret navigation keys, direct BGRA paint handoff, and borrow-based glyph cache. See `docs/technical/`.
- **Task 14 (release pipeline) — partial:** `assets/icon.ico` (clipboard + padlock design, 16/32/48/256px ICO) created and integrated — tray uses `load_app_icon()` from embedded bytes; main window sets the icon via `WM_SETICON` (taskbar button + Alt+Tab). `Cargo.toml` metadata, committed `Cargo.lock`, `README.md`, and `LICENSE` also exist. Still missing: `.github/workflows/` (CI + tagged release), `CHANGELOG.md`, and an embedded `.exe` icon resource (`build.rs` + `RT_GROUP_ICON` — makes the binary itself show the icon in Windows Explorer).
- **Done (Tasks 29–33):** display-indices dirty-key cache (`App::entries_version`), persistent `glyph_cache` for input hit-tests, single-slot preview pixmap cache, release `image_pixels` after persist enqueue, visibility-gated relative-time timer. See `docs/technical/ui-perf-caches.md`, `docs/technical/storage.md`, `docs/technical/window-gdi.md`.
