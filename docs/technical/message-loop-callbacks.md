# Message Loop Callback Soundness

How TrayVault routes tray, hotkey, and close actions from `WndProc` without Rust aliasing violations.

## Problem

`WndProc` dispatches into `Window::dispatch(&mut self, …)` via a process-global active-window pointer. Callbacks registered in `wire_callbacks` (`main.rs`) run **inside** that `&mut` borrow. Capturing `*const Window` and forming `&Window` in those callbacks overlapped the live `&mut Window` — undefined behavior under Rust's aliasing rules (even when methods were `&self` + `Cell`).

A `static mut ACTIVE_WINDOW` also triggered the `static_mut_refs` lint.

## Solution

Two coordinated changes in [`window.rs`](../../src/win32/window.rs) and [`main.rs`](../../src/main.rs):

### 1. `AtomicPtr` active window

```text
static ACTIVE_WINDOW: AtomicPtr<Window>
```

Set on the main thread at `run_message_loop` entry (`Ordering::SeqCst`); cleared before return. `wnd_proc` loads the pointer and calls `dispatch` with a single `&mut Window` — no `static mut`.

### 2. HWND-based operations in callbacks

Tray, command, hotkey, close, and geometry callbacks receive **`HWND` (Copy)**, not `*const Window`. Window operations they need are free functions or `App` methods that only need `HWND`:

| Function | Role |
|----------|------|
| `show_window(hwnd, show_in_taskbar)` | Show, focus, taskbar bit, DWM refresh, invalidate |
| `hide_window(hwnd)` | Clear taskbar bit, `ShowWindow(SW_HIDE)` |
| `is_window_visible(hwnd)` | `IsWindowVisible` query |
| `request_window_repaint(hwnd)` | Full-client `InvalidateRect` |

`Window::show` / `hide` / `is_visible` / `request_repaint` delegate to these helpers.

`apply_tray_action`, `show_main_window`, `hide_main_window`, and `quit_app` in `main.rs` take `HWND` instead of `&Window`.

### `on_geometry_changed`

Registered in `wire_callbacks` as `WindowCallbacks::on_geometry_changed`. Fired from `WM_EXITSIZEMOVE` after a modal move/resize ends. The closure calls `App::persist_window_geometry(hwnd, config_path)` — no `&Window` borrow extension beyond the existing `dispatch` frame. See [`config.md`](config.md) (Window placement).

### Deferred `WM_CAPTURECHANGED`

History scrollbar thumb drag uses `SetCapture` so drag continues outside the client rect. Losing capture posts `WM_CAPTURECHANGED`, which would otherwise call `on_input` **inside** the current message handler (e.g. synchronously during `LButtonUp` if `ReleaseCapture()` were called from input code).

`Window::on_capture_changed` sets `pending_capture_lost` when another HWND gains capture; `flush_pending_capture_lost` runs at the end of `dispatch()` and forwards `InputEvent::CaptureLost` to clear `scrollbar_drag_grab_y`. Drag end on mouse-up only clears UI state — no explicit `ReleaseCapture()`.

## Invariants (unchanged)

- **HTCAPTION drag** — search field and title-bar gaps still use native caption drag via `WM_NCHITTEST`; synthetic search clicks via `WM_NCLBUTTONUP`.
- **No re-entrant Win32 calls from `on_input`** while `App` / `UiState` `RefCell` borrows are active — e.g. `SendMessage(WM_SYSCOMMAND, SC_MOVE)`, or `ReleaseCapture()` (both can synchronously re-enter `WndProc` and crash with `RefCell already mutably borrowed` / `STATUS_STACK_BUFFER_OVERRUN`). See [`window-gdi.md`](window-gdi.md) (Title bar hit-testing) and [`ui-views.md`](ui-views.md) (History scrollbar).

## Related docs

- [`window-gdi.md`](window-gdi.md) — message loop, WndProc dispatch, show/hide/taskbar
- [`app-lifecycle.md`](app-lifecycle.md) — `quit_app` teardown sequence
- [`system-tray.md`](system-tray.md) — tray callback → `on_tray` / `on_command`; left-click toggle debounced in `TrayIcon::handle_callback` before `apply_tray_action`
