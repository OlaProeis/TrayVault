# App Lifecycle & Clean Shutdown

Bootstrap and teardown live in `src/main.rs`; persistence flush is `App::shutdown()` in `src/app.rs`.

## Startup (`run`)

1. `log::init()` → load config → sync autostart → `Store::load_initial()`.
2. `Window::create(hinstance, &config)` (restores saved client size/position) → register clipboard listener → construct `App` + `UiState`.
3. Register tray icon and global hotkey; wire `WindowCallbacks`.
4. Show window (unless `--minimized`) → enter `run_message_loop()`.

## Normal quit (tray **Quit**)

`TrayMenuAction::Quit` → `quit_app()`:

1. Set shared `quitting` flag (`Rc<Cell<bool>>`) — idempotent guard.
2. `App::shutdown(hwnd, config_path)` — sync pause flag, capture window placement into config, save config, `Store::flush()`.
3. Unregister clipboard listener (warn on failure, non-fatal).
4. Remove tray icon.
5. Unregister global hotkey.
6. `DestroyWindow` → `WM_DESTROY` → `PostQuitMessage` → message loop returns.

## Post-loop teardown (`run` after message loop)

The loop exits only via `PostQuitMessage`. Teardown must be **idempotent** because `quit_app` already performed most cleanup:

| Step | When `quitting == true` (normal quit) | When `quitting == false` (abnormal) |
|------|---------------------------------------|-------------------------------------|
| `App::shutdown` | Skipped (already done) | Runs as fallback |
| `unregister_listener` | Skipped | Runs (warn on Err) |
| `hotkey.unregister` | Skipped | Runs |
| `App::join_storage` | **Always** | **Always** |

- Post-loop steps must **not** use `?` on `unregister_listener` — the window is destroyed and the listener may already be unregistered; treating that as fatal previously forced exit code 1 and skipped `join_storage`.
- End with `log::info("TrayVault exited cleanly")` and `Ok(())` so `main()` exits with code 0.

## Window close (X button)

`on_close` hides the window to tray — does **not** quit and does **not** write window placement to disk. See [`system-tray.md`](system-tray.md).

## Window placement persistence

Placement is stored in `config.toml` (`window_client_w`, `window_client_h`, optional `window_x` / `window_y`). See [`config.md`](config.md) and [`window-gdi.md`](window-gdi.md).

- **During session:** move/resize ends with `WM_EXITSIZEMOVE` → `App::persist_window_geometry` in `wire_callbacks`.
- **On quit:** `App::shutdown(hwnd, config_path)` captures geometry again (covers users who moved the window without triggering a modal `WM_EXITSIZEMOVE`, or who only resized via edge grips that still exit the modal loop).

Post-loop fallback `shutdown(hwnd, …)` uses the same path when `quitting == false`.

## Related

- Tray quit entry point: [`system-tray.md`](system-tray.md) (Shutdown path).
- Config keys and clamp rules: [`config.md`](config.md) (Window placement).
- Storage flush/join: [`storage.md`](storage.md).
- Log rotation and WndProc message filtering: [`logging.md`](logging.md).
