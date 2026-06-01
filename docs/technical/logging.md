# Logging

Minimal, dependency-free file logger in `src/log.rs` (pure `std` — no `tracing`/`log`).

## Location & format
- Writes to `%LOCALAPPDATA%\TrayVault\trayvault.log` (append mode; directory created if missing).
- Line format: `YYYY-MM-DD HH:MM:SS.mmmZ [LEVEL] message` (UTC).
- The UTC timestamp is computed by hand via Howard Hinnant's civil-from-days algorithm (`format_utc`) — no `chrono`.

## Rotation
- On init (`open_log`), if `trayvault.log` is at least 5 MiB, it is renamed to `trayvault.log.1` (any existing `.1` is removed first) and a fresh log is opened.
- Single-generation rotation only; best-effort — a failed rotation falls back to plain append.

## Message logging (WndProc)
- `src/win32/window.rs` logs only rare/lifecycle Win32 messages (hotkey, clipboard, tray, command, size, key/char, close/destroy, etc.).
- High-frequency messages (`WM_MOUSEMOVE`, `WM_PAINT`, `WM_TIMER`, `WM_MOUSEWHEEL`, `WM_NCHITTEST`) are not logged to avoid synchronous disk I/O on the UI thread.

## API
- `log::init()` — best-effort, idempotent (only the first call takes effect).
- `log::info(msg)`, `log::warn(msg)`, `log::error(msg)`.

## Behavior
- **Best-effort:** if the log file can't be opened, logging becomes a silent no-op and never affects the app.
- **Thread-safe:** the file is guarded by a `Mutex` inside a `OnceLock`, so the storage worker thread (Task 5) can log too.
- **Debug builds** also mirror every line to `stderr` for convenience; release builds (which run under `windows_subsystem = "windows"`, no console) only write the file.
- A panic hook in `main.rs` routes panic info through `log::error` so crashes leave a trace even with no console.

## Init order
`main()` calls `log::init()` first, then installs the panic hook, then runs — so the earliest failures are captured.
