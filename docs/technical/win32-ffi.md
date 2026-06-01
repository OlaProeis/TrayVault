# Win32 FFI Foundation

TrayVault talks to Windows through **hand-declared FFI** — no `windows`/`windows-sys` crate. All raw bindings live in `src/win32/ffi.rs`; safe wrappers live in `src/win32/mod.rs`.

## ABI conventions (must hold for `x86_64-pc-windows-msvc`)
- **Handles are `isize`**, not raw pointers. `HWND`, `HINSTANCE`, `HDC`, etc. all alias `HANDLE = isize`. On Win64 an integer and a pointer pass identically in registers, so this is ABI-correct, and it keeps handles `Copy`/`Send`/comparable and storable in app state. **Null handle is `0`.**
- **`extern "system"`** is used for every binding (the Win64 calling convention).
- **Wide (UTF-16) APIs only** — always the `*W` variants. Build NUL-terminated buffers with `win32::wide(&str) -> Vec<u16>`; keep the `Vec` alive while the pointer is used.
- `WPARAM = usize`, `LPARAM/LRESULT = isize`, `UINT_PTR = usize`.

## What's declared (grouped by owning DLL)
- **kernel32:** `GetLastError`, `SetLastError`, `GetModuleHandleW`, `GetCurrentProcessId`, `GetCurrentThreadId`, `GetTickCount` (tray left-click toggle debounce in `tray.rs`), `GlobalAlloc`/`Free`/`Lock`/`Unlock`/`Size`, `MoveFileExW` (`MOVEFILE_REPLACE_EXISTING`, `MOVEFILE_WRITE_THROUGH` — atomic `entries.dat` promote in `store/meta.rs`).
- **user32:** window class + lifecycle (`RegisterClassW`, `CreateWindowExW`, `DestroyWindow`, `DefWindowProcW`), message loop (`GetMessageW`, `TranslateMessage`, `DispatchMessageW`, `PostQuitMessage`, `PostMessageW`, `SendMessageW`), visibility/layout (`ShowWindow`, `UpdateWindow`, `GetClientRect`, `GetWindowRect`, `GetSystemMetrics`, `IsIconic`, `InvalidateRect`, `SetWindowPos`, `AdjustWindowRectEx`), DC (`GetDC`, `ReleaseDC`), painting (`BeginPaint`, `EndPaint`), `LoadCursorW`, timers (`SetTimer`, `KillTimer`), hotkeys (`RegisterHotKey`, `UnregisterHotKey`).
- **gdi32:** `CreateDIBSection`, `StretchDIBits`, `CreateCompatibleDC`, `DeleteDC`, `SelectObject`, `DeleteObject`, `CreateSolidBrush` (dark window class background — resize-exposed regions clear dark, not system gray).
- **dwmapi:** `DwmExtendFrameIntoClientArea`, `DwmSetWindowAttribute` (borderless frame — disable DWM NC rendering, immersive dark mode, border color).
- **user32 (paint):** `RedrawWindow`, `UpdateWindow` — bindings available; the modal move/resize path now relies on plain `InvalidateRect` + immediate `GetDC` blit (no forced `RDW_FRAME` repaint, which flashed a white outer edge).

**Structs:** `POINT`, `RECT`, `MSG`, `WNDCLASSW`, `PAINTSTRUCT`, `RGBQUAD`, `BITMAPINFOHEADER`, `BITMAPINFO`, `NCCALCSIZE_PARAMS`, `WINDOWPOS`, `MARGINS`.
**Types:** `WNDPROC`, `TIMERPROC` (both `Option<unsafe extern "system" fn ...>`).
**Constants:** `WM_*`, `WS_*`, `WS_EX_*`, `SW_*`, `CW_USEDEFAULT`, `SM_XVIRTUALSCREEN`, `SM_YVIRTUALSCREEN`, `SM_CXVIRTUALSCREEN`, `SM_CYVIRTUALSCREEN`, `IDC_ARROW`, `COLOR_WINDOW`, `DWMWA_*`, `DWMNCRP_*`, `GMEM_*`, `BI_RGB`, `DIB_RGB_COLORS`, `SRCCOPY`, `MOD_*`.

Window placement persistence uses `GetWindowRect`, `GetClientRect`, `IsIconic`, and virtual-screen `GetSystemMetrics` indices — see [`window-gdi.md`](window-gdi.md) (Window placement persistence).

## Safe-wrapper pattern (`src/win32/mod.rs`)
Keep `unsafe` blocks tight; check the documented failure value; translate failures into `ClipError` via `last_error(api)` **immediately** after the failing call (an intervening Win32 call can clobber the thread-local error code).

```rust
let h = unsafe { GetModuleHandleW(std::ptr::null()) };
if h == 0 { return Err(last_error("GetModuleHandleW")); }
```

Provided helpers: `last_error`, `wide`, `current_process_id`, `current_thread_id`, `current_module_handle`.

## Extending
Add new APIs to the matching `#[link(...)]` block, keeping the hand-verified style. The module header in `ffi.rs` lists the planned future additions and their DLLs (clipboard/menus → user32, process query → kernel32, tray → shell32, registry → advapi32, DWM frame → dwmapi).

## Lint note
`ffi.rs` carries a module-level `#![allow(non_snake_case, non_camel_case_types, dead_code, clippy::upper_case_acronyms)]` so the bindings can mirror the Win32 header names exactly and be declared ahead of first use.
