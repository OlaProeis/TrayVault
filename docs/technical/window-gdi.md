# Main Window, Message Loop, and GDI Presentation

Task 2 modules: `src/win32/window.rs` and `src/win32/gdi.rs`.

## Window (`window.rs`)

- **`Window::create(hinstance, config)`** — registers the Unicode class `TrayVaultBorderlessWindow` once, creates a captionless popup window (`WS_POPUP | WS_THICKFRAME`) at the saved client size (default 900×640) and screen position (`window_x` / `window_y` when both set; otherwise `CW_USEDEFAULT`), strips any residual caption styles via `SetWindowLongPtrW`, registers the class with a dark solid `hbrBackground` (so a live-resize grow never flashes system gray), disables DWM non-client rendering (`DwmSetWindowAttribute`, `DwmExtendFrameIntoClientArea`), handles `WM_NCCALCSIZE` / `WM_NCACTIVATE` (entire window is client area; no system frame flash), and `WM_NCHITTEST` (edge resize grips), with `WS_EX_TOOLWINDOW`, and allocates the GDI back-buffer to the initial client size.
- **`Window::show(show_in_taskbar)` / `hide`** — delegate to [`show_window`](../../src/win32/window.rs) / [`hide_window`](../../src/win32/window.rs) (`ShowWindow` + visibility tracking). `show()` also calls `BringWindowToTop`, `SetForegroundWindow`, and `SetFocus`, starts the 2 s relative-time `WM_TIMER`, then `set_taskbar_button_visible()` per config. `hide()` stops the timer and clears the taskbar button before hiding. `request_repaint` calls `request_window_repaint` → `InvalidateRect`.
- **`Window::run_message_loop`** — blocking `GetMessageW` / `TranslateMessage` / `DispatchMessageW` loop on the main thread only. Stores the active [`Window`] in an `AtomicPtr` for the loop duration so `WndProc` can dispatch without extra FFI. Tray/hotkey/close callbacks use [`HWND`]-based helpers ([`show_window`], [`hide_window`], [`is_window_visible`], [`request_window_repaint`]) so they never form `&Window` while `dispatch` holds `&mut Window`.

### Dispatched messages

| Message | Behavior |
|---------|----------|
| `WM_PAINT` | `WindowCallbacks::on_paint` rasterizes UI (Task 8) into the BGRA buffer, blit via GDI |
| `WM_TIMER` | Optional callback; `InvalidateRect` only when the app reports dirty (relative-time refresh while visible) |
| `WM_HOTKEY` | Stub callback hook (Task 11) |
| `WM_CLIPBOARDUPDATE` | `WindowCallbacks::on_clipboard_update` → clipboard capture (Task 3) |
| `WM_APP + 1` | Tray callback → `WindowCallbacks::on_tray` (see `system-tray.md`) |
| `WM_COMMAND` | Tray context-menu selections and other command ids |
| `WM_CLOSE` | Hide to tray via `on_close` hook (does not destroy) |
| `WM_SIZE` | Recreate DIB section; fill dark BGRA; during the modal loop, blit immediately via `GetDC` so the newly exposed strip is never white (no frame repaint) |
| `WM_ENTERSIZEMOVE` / `WM_EXITSIZEMOVE` | Track modal move/resize; refresh DWM attrs; on exit, `on_geometry_changed` persists placement to `config.toml`, then a plain `InvalidateRect` (one clean `WM_PAINT`) |
| `WM_MOVE` | No-op — a move changes no pixels (DWM relocates the surface); painting/repainting the frame here flashes a white edge while dragging |
| `WM_NCCALCSIZE` | Return `0` leaving `rgrc[0]` (the proposed new window rect) unmodified, so the whole window rect is client area — no non-client frame |
| `WM_NCACTIVATE` | Return `1` — skip default non-client activation paint (avoids gray border on drag / tray focus) |
| `WM_NCHITTEST` | Edge resize grips; search field + title-bar gaps → `HTCAPTION` (drag); settings/close → `HTCLIENT` |
| `WM_NCLBUTTONDOWN` | Default via `DefWindowProcW` (starts caption drag); records screen point when press starts in search field |
| `WM_NCLBUTTONUP` | Search click without drag (≤4px slop) → synthetic `LButtonUp` to input; otherwise default |
| `WM_ERASEBKGND` | Delegate to `DefWindowProcW` so the **dark class brush** erases the region. Normal repaints use `InvalidateRect(bErase = 0)`, so this fires only on resize/uncover — keeping the freshly exposed strip dark (not system gray) until `WM_PAINT` |
| `WM_DESTROY` | `KillTimer` (safety net), `PostQuitMessage` |

### Window icon (`WM_SETICON`, `WM_GETICON`, class `hIcon`)

`load_app_icon()` in [`tray.rs`](../../src/win32/tray.rs) returns a process-lifetime `HICON` (cached in a `OnceLock`) shared by the tray, window class, and main window.

During class registration, `WNDCLASSW.hIcon` is set to that handle so `GCL_HICON` queries succeed. After `CreateWindowExW`, `Window::create` sends `WM_SETICON` twice — `ICON_SMALL` (taskbar button) and `ICON_BIG` (Alt+Tab). `WndProc` handles `WM_GETICON` explicitly (Task Manager reads window icons this way; `WM_SETICON` alone is not enough for the process list).

The handle is not destroyed on window or tray teardown (process exit only).

`WM_SETICON`, `WM_GETICON`, `ICON_SMALL`, and `ICON_BIG` are declared in [`ffi.rs`](../../src/win32/ffi.rs). The icon asset lives in [`assets/icon.ico`](../../assets/icon.ico) (embedded at compile time in `tray.rs`; also linked into the `.exe` via `build.rs` for Explorer). See [`system-tray.md`](system-tray.md) (Icon lifecycle).

### Relative-time timer

The 2 s `WM_TIMER` (`TIMER_ID = 1`) refreshes relative-time labels (`App::on_timer_tick`). It is started in [`show_window`](../../src/win32/window.rs) when the window is shown and stopped in [`hide_window`](../../src/win32/window.rs) when hidden to tray — not at create time — so hidden tray mode does not wake every 2 s. `SetTimer` with an existing id resets the interval (safe on repeated show). `KillTimer` on destroy is a safety net if hide was skipped.

`WindowCallbacks` holds optional `Box<dyn FnMut …>` hooks wired from `main.rs` (`on_paint`, `on_input`, `on_clipboard_update`, `on_tray`, `on_hotkey`, `on_close`, `on_geometry_changed`, etc.).

## GDI back-buffer (`gdi.rs`)

- **`GdiBuffer`** — `CreateDIBSection` 32-bpp top-down BGRA (`biHeight` negative). Exposes `bits_mut`, `fill_solid`, `resize`, `resize_to_client`.
- **`present(hdc, pixels, w, h)`** — `StretchDIBits` blit from a caller-owned BGRA slice.
- **`present_internal(hdc)`** — blit from the internal DIB bits.
- **`with_paint(hwnd, f)`** — `BeginPaint` / `EndPaint` RAII-style helper.

Task 8 UI renders RGBA via `src/ui/pixmap.rs`; convert to **BGRA** before calling `present`.

### Title bar hit-testing (borderless drag)

Custom chrome is 28px tall ([`titlebar.rs`](../../src/ui/titlebar.rs)). `on_nc_hit_test` in [`window.rs`](../../src/win32/window.rs):

- **Settings and close buttons** (right 56px): `HTCLIENT` so left-up reaches [`input.rs`](../../src/ui/input.rs).
- **Search field** and any remaining title-bar gap: `HTCAPTION` so Windows handles drag natively. A click without movement in the search field is recovered on `WM_NCLBUTTONUP` (≤4px slop) and forwarded as `LButtonUp` to focus the field and place the caret — see [`ui-views.md`](ui-views.md) (Search and filters).
- **Do not** call `SendMessageW(WM_SYSCOMMAND, SC_MOVE | HTCAPTION)` from `on_input` while `App`/`UiState` `RefCell` borrows are active — that re-enters `WndProc` and panics (`RefCell already mutably borrowed` / `STATUS_STACK_BUFFER_OVERRUN`).

### Borderless frame (no gray system border)

The window is `WS_POPUP | WS_THICKFRAME` with a custom title bar. Windows/DWM can still paint a visible **light-gray system frame** around the client area in two common cases:

| Trigger | Why |
|---------|-----|
| Title-bar drag (`HTCAPTION`) | Modal move loop repaints non-client chrome |
| Tray right-click menu | `SetForegroundWindow(hwnd)` activates the window; default NC activation paint runs |

Symptoms: a light (white or gray) band outside the dark UI — the system `COLOR_WINDOW` brush, DWM glass frame desync during live resize/move, or Win11 border chrome bleeding through a gap between the outer window rect and the client rect.

**Fix (all in [`window.rs`](../../src/win32/window.rs)):**

| Piece | What |
|-------|------|
| Window class | `hbrBackground` = a **solid dark brush** (`CreateSolidBrush(0x001E1E1E)`, matches the UI background) so a live-resize grow clears the newly exposed strip dark instead of flashing system gray; **no** `CS_HREDRAW`/`CS_VREDRAW` — avoids partial-invalidation stripes |
| `WM_NCCALCSIZE` | Return `0` and **leave `rgrc[0]` unmodified** (it already holds the proposed *new* window rect), so the client rect covers the full window rect. Do **not** copy `rgrc[1]` (the *old* window rect) into `rgrc[0]` — during a resize-grow that sets the client smaller than the new window and leaves a gray non-client strip on the grown edge that lags the drag and can persist after release |
| `WM_NCACTIVATE` | Return `1` — skip default non-client activation/deactivation paint |
| Modal move | `WM_MOVE` is a **no-op**. A move does not change any pixels — DWM relocates the already-composed surface. Re-presenting and (especially) forcing a non-client repaint on every `WM_MOVE` fights DWM's relocation and flashes a 1–2px white band along the outer edge while dragging. |
| Modal resize | On each `WM_SIZE` inside the modal loop, blit the freshly resized (dark-filled + re-rendered) buffer immediately via `present_client()` (`GetDC`, not `BeginPaint`) so the newly exposed strip never shows white. **No** `RedrawWindow(RDW_FRAME)` — repainting the non-client frame is itself a white-edge source. `WM_EXITSIZEMOVE` ends with a plain `InvalidateRect` (one clean `WM_PAINT`). |
| DWM (`apply_dwm_borderless`) | `DWMNCRP_DISABLED`, `DWMWCP_DONOTROUND`, `DWMWA_USE_IMMERSIVE_DARK_MODE`, `DWMWA_BORDER_COLOR` = dark gray (`0x001E1E1E`), `DwmExtendFrameIntoClientArea` with **zero** margins (not `-1` — sheet-of-glass extension composites translucent on the newly exposed strip during a live grow and can flash white). Applied once on create/show/activate and on enter/exit of the modal loop — **not** per `WM_MOVE`/`WM_SIZE`. |

FFI for DWM lives in [`ffi.rs`](../../src/win32/ffi.rs) (`dwmapi` link block: `MARGINS`, `NCCALCSIZE_PARAMS`, `DwmExtendFrameIntoClientArea`, `DwmSetWindowAttribute`). Do **not** set `hbrBackground` to `0`/`COLOR_WINDOW + 1` (no brush → resize-exposed strip flashes gray; the white system brush flashes white), copy `rgrc[1]` into `rgrc[0]` in `WM_NCCALCSIZE` (gray strip on resize-grow), use `DwmExtendFrameIntoClientArea(-1)`, delegate `WM_NCCALCSIZE` to `DefWindowProcW` when `wParam != 0`, or reintroduce a per-`WM_MOVE`/`WM_SIZE` `RedrawWindow(RDW_FRAME)` — each brings the border/edge flash back.

Live resize presentation uses [`gdi::with_dc`](../../src/win32/gdi.rs) (`GetDC`/`ReleaseDC`); `WM_PAINT` still uses [`gdi::with_paint`](../../src/win32/gdi.rs) (`BeginPaint`/`EndPaint`).

### Window placement persistence

Remembers the main window’s client size and screen position across process restarts. Keys and load rules: [`config.md`](config.md) (Window placement).

| Phase | Code path |
|-------|-----------|
| **Create** | `Window::create(hinstance, &config)` — `clamp_client_dimensions` → `client_to_window_size` → `CreateWindowExW` with saved outer size; position from `window_x`/`window_y` when both set, else `CW_USEDEFAULT` |
| **After move/resize** | `WM_EXITSIZEMOVE` → `WindowCallbacks::on_geometry_changed` (wired in `main.rs`) → `App::persist_window_geometry(hwnd, config_path)` |
| **Quit** | `App::shutdown(hwnd, config_path)` → `capture_geometry_into_config` → `Config::save` |

[`capture_geometry_into_config`](../../src/win32/window.rs) reads `GetWindowRect` (outer position/size) and `GetClientRect` (client dimensions), updates `Config`, and returns `false` without mutating config when `IsIconic` (minimized). [`clamp_screen_position`](../../src/win32/window.rs) uses `GetSystemMetrics` (`SM_XVIRTUALSCREEN`, `SM_YVIRTUALSCREEN`, `SM_CXVIRTUALSCREEN`, `SM_CYVIRTUALSCREEN`) so at least 48px of the window remains on the virtual screen after monitor layout changes.

Hide-to-tray (`on_close`, hotkey, Esc) does **not** persist placement — only `WM_EXITSIZEMOVE` and shutdown do. Thumbnail/preview caches still invalidate on resize via existing `WM_SIZE` handling ([`thumbnail-cache.md`](thumbnail-cache.md), [`ui-perf-caches.md`](ui-perf-caches.md)).

### Taskbar visibility (`show_in_taskbar`)

TrayVault is tray-first: the main window is created with `WS_EX_TOOLWINDOW` so a hidden window does not appear on the taskbar or in Alt+Tab.

Users can opt into normal taskbar behavior while the window is open via **`show_in_taskbar`** in [`config.toml`](config.md) (default `true`). Settings label: **Show in taskbar when open** ([`settings-panel.md`](settings-panel.md)).

| State | Taskbar button |
|-------|----------------|
| Hidden | Never (always `WS_EX_TOOLWINDOW`) |
| Visible + `show_in_taskbar = true` | Yes (`WS_EX_TOOLWINDOW` cleared) |
| Visible + `show_in_taskbar = false` | No (tool window stays) |

Implementation in [`window.rs`](../../src/win32/window.rs):

- **`set_taskbar_button_visible(hwnd, visible)`** — read/modify `GWL_EXSTYLE`, set or clear `WS_EX_TOOLWINDOW`, then `SetWindowPos(..., SWP_FRAMECHANGED)` so the shell refreshes the taskbar entry.
- **`Window::show(show_in_taskbar)`** — after `ShowWindow` and focus helpers, calls `set_taskbar_button_visible(hwnd, show_in_taskbar)`.
- **`Window::hide()`** — calls `set_taskbar_button_visible(hwnd, false)` before `ShowWindow(SW_HIDE)`.

Call sites must stay in sync — any path that hides without `Window::hide()` / [`hide_window`](../../src/win32/window.rs) must still clear the taskbar bit:

- [`main.rs`](../../src/main.rs) — `show_main_window` / `hide_main_window` take `HWND` and call `show_window` / `hide_window`; tray/hotkey/close callbacks use the same helpers (no `&Window` from `WndProc` while `dispatch` holds `&mut`).
- [`input.rs`](../../src/ui/input.rs) — `hide_window()` (Esc, close button, copy-back) calls `set_taskbar_button_visible` before `ShowWindow(SW_HIDE)`.
- [`settings_input.rs`](../../src/ui/settings_input.rs) — toggling the setting while the window is visible applies `set_taskbar_button_visible` immediately.

Task Manager still lists the process as `TrayVault` (often under Background processes); the taskbar setting only affects the shell taskbar button, not process enumeration.

## Bootstrap (`main.rs`)

`run()` creates the window, shows it, and blocks in `run_message_loop` until the user closes the window.
