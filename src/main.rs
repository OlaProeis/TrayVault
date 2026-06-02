//! TrayVault — a from-scratch clipboard history manager for Windows.
//!
//! Task 1 established the Win32 FFI surface ([`win32`]), error type ([`error`]),
//! and file logger ([`log`]). Task 2 adds the main window, message loop, and
//! GDI pixel presentation ([`win32::window`], [`win32::gdi`]). Task 3 adds
//! clipboard capture ([`win32::clipboard`], [`models`]). Task 7 wires
//! [`app::App`] orchestration (config, prune-to-cap, copy-back). Task 10 adds
//! system tray integration ([`win32::tray`]). Task 11 adds global hotkey
//! registration ([`win32::hotkey`]). Task 12 adds Run-key autostart
//! ([`win32::autostart`]) and `--minimized` startup parsing.

// Release builds are GUI apps (no console window); debug builds keep the console
// so logs and panics are visible during development.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("TrayVault targets Windows only.");

mod app;
mod config;
mod error;
mod hash;
mod log;
mod models;
mod store;
mod ui;
mod win32;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::ptr;
use std::rc::Rc;

use error::Result;
use ui::input::{handle_input, Viewport};
use ui::settings_input::SettingsHooks;
use ui::UiState;
use win32::autostart::sync_autostart_from_config;
use win32::clipboard::shared_monitor;
use win32::ffi::{DestroyWindow, InvalidateRect};
use win32::hotkey::{HotkeyHandle, TRAYVAULT_HOTKEY_ID};
use win32::tray::{show_tray_notification, TrayIcon, TrayMenuAction};
use win32::window::{request_window_repaint, WindowCallbacks};

struct TrayContext {
    app: Rc<RefCell<app::App>>,
    ui: Rc<RefCell<UiState>>,
    monitor: Rc<RefCell<win32::clipboard::ClipboardMonitor>>,
    config_path: PathBuf,
    quitting: Rc<Cell<bool>>,
    tray_ptr: *mut TrayIcon,
    hotkey_ptr: *mut HotkeyHandle,
}

fn main() {
    // Logging first, so any early failure (including a panic) is captured.
    log::init();
    install_panic_hook();

    if let Err(err) = run() {
        log::error(&format!("fatal: {err}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let minimized = parse_minimized_flag();

    log::info(&format!(
        "TrayVault {} starting (pid {}, thread {})",
        env!("CARGO_PKG_VERSION"),
        win32::current_process_id(),
        win32::current_thread_id(),
    ));

    let config_path = config::Config::default_path();
    let config = config::Config::load_or_default(&config_path);
    log::info(&format!(
        "config loaded from {} (max_entries={}, theme={:?}, hotkey={}, autostart={})",
        config_path.display(),
        config.max_entries,
        config.theme,
        config.hotkey,
        config.autostart
    ));

    if let Ok(exe_path) = std::env::current_exe() {
        sync_autostart_from_config(config.autostart, &exe_path);
    } else {
        log::warn("could not resolve executable path for autostart sync");
    }

    if minimized {
        log::info("started with --minimized (tray-only until toggled)");
    }

    let (loaded, store) = store::Store::load_initial();
    log::info(&format!(
        "loaded {} persisted entries (next id {})",
        loaded.entries.len(),
        loaded.next_id
    ));

    let hinstance = win32::current_module_handle()?;
    log::info(&format!("module handle = 0x{hinstance:016X}"));

    let mut window = win32::window::Window::create(hinstance, &config)?;
    let hwnd = window.hwnd();

    let capture_config = config.capture_config();
    let monitor = shared_monitor(hwnd, capture_config);
    monitor.borrow_mut().register_listener()?;

    let app = Rc::new(RefCell::new(app::App::new(config, loaded, store)));
    let ui = Rc::new(RefCell::new(UiState::new()));
    let quitting = Rc::new(Cell::new(false));

    app.borrow().thumb_loader().set_notify_hwnd(hwnd);

    app.borrow_mut()
        .apply_capture_config(&mut monitor.borrow_mut());

    let mut tray = TrayIcon::new(hwnd, hinstance)?;
    update_tray_tooltip(&tray, app.borrow().pause_capture)?;

    let mut hotkey = HotkeyHandle::new();
    if let Err(err) = hotkey.try_register(hwnd, &app.borrow().config.hotkey) {
        match &err {
            error::ClipError::HotkeyConflict { hotkey: hk } => {
                log::warn(&format!("global hotkey conflict: {err}"));
                show_tray_notification(
                    &tray,
                    "TrayVault",
                    &format!(
                        "Could not register hotkey {hk} — it may be in use by another app. \
                         Use the tray icon to open TrayVault."
                    ),
                );
            }
            _ => log::warn(&format!("global hotkey registration failed: {err}")),
        }
    }

    let tray_ctx = TrayContext {
        app: Rc::clone(&app),
        ui: Rc::clone(&ui),
        monitor: Rc::clone(&monitor),
        config_path: config_path.clone(),
        quitting: Rc::clone(&quitting),
        tray_ptr: &mut tray as *mut TrayIcon,
        hotkey_ptr: &mut hotkey as *mut HotkeyHandle,
    };

    wire_callbacks(&mut window, hwnd, &tray_ctx);

    // Autostart passes `--minimized` to stay tray-only; a normal launch shows the window.
    if minimized {
        log::info("running in tray (--minimized; window hidden until toggled)");
    } else {
        log::info("showing main window on startup");
        show_main_window(hwnd, &app, &ui);
    }
    window.run_message_loop()?;

    if !quitting.get() {
        // WM_DESTROY without an explicit quit (shouldn't happen in normal use).
        if let Err(err) = app.borrow_mut().shutdown(hwnd, &config_path) {
            log::warn(&format!("shutdown save failed: {err}"));
        }
        if let Err(err) = monitor.borrow_mut().unregister_listener() {
            log::warn(&format!("clipboard listener unregister failed: {err}"));
        }
        hotkey.unregister(hwnd);
    }
    app.borrow_mut().join_storage();
    log::info("TrayVault exited cleanly");
    Ok(())
}

fn wire_callbacks(window: &mut win32::window::Window, hwnd: win32::ffi::HWND, ctx: &TrayContext) {
    let app_clip = Rc::clone(&ctx.app);
    let monitor_clip = Rc::clone(&ctx.monitor);
    let app_paint = Rc::clone(&ctx.app);
    let ui_paint = Rc::clone(&ctx.ui);

    let app_thumb = Rc::clone(&ctx.app);
    let ui_thumb = Rc::clone(&ctx.ui);

    let app_input = Rc::clone(&ctx.app);
    let ui_input = Rc::clone(&ctx.ui);
    let monitor_input = Rc::clone(&ctx.monitor);
    let ctx_input = ctx.clone_handles();

    let ctx_tray = ctx.clone_handles();
    let ctx_cmd = ctx.clone_handles();
    let ctx_hotkey = ctx.clone_handles();
    let ctx_close = ctx.clone_handles();
    let ctx_geom = ctx.clone_handles();

    let ui_nc = Rc::clone(&ctx.ui);

    window.set_callbacks(WindowCallbacks {
        on_clipboard_update: Some(Box::new(move || {
            match monitor_clip.borrow_mut().try_capture() {
                Ok(entry) => app_clip.borrow_mut().on_clipboard_captured(entry),
                Err(err) => log::warn(&format!("clipboard capture skipped: {err}")),
            }
            if app_clip.borrow_mut().take_needs_repaint() {
                // SAFETY: `hwnd` is the live main window.
                unsafe {
                    let _ = InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
        })),
        on_timer: Some(Box::new({
            let app_timer = Rc::clone(&ctx.app);
            move || app_timer.borrow_mut().on_timer_tick()
        })),
        on_paint: Some(Box::new(move |gdi| {
            let app_ref = app_paint.borrow();
            let mut ui_ref = ui_paint.borrow_mut();
            let w = gdi.width().max(0) as u32;
            let h = gdi.height().max(0) as u32;
            if w == 0 || h == 0 {
                return;
            }
            let needed = win32::gdi::GdiBuffer::required_bytes(gdi.width(), gdi.height());
            ui::render::render_app(&app_ref, &mut ui_ref, (w, h), &mut gdi.bits_mut()[..needed]);
            let now = ui::scroll_bar::tick_now();
            ui_ref.clear_scroll_repaint_after_paint(now);
            if ui_ref.take_deferred_scroll_repaint(now) {
                request_window_repaint(hwnd);
            }
            if ui_ref.scrollbar_fading(now) {
                let last = ui_ref.scrollbar_last_fade_tick;
                if now.saturating_sub(last) >= 16 {
                    ui_ref.scrollbar_last_fade_tick = now;
                    request_window_repaint(hwnd);
                }
            }
        })),
        on_thumb_ready: Some(Box::new(move || {
            let replies = app_thumb.borrow().thumb_loader().drain_replies();
            let mut ui_ref = ui_thumb.borrow_mut();
            if ui_ref.apply_thumb_replies(&replies) {
                request_window_repaint(hwnd);
            }
        })),
        on_nc_client_override: Some(Box::new(move |cx, cy, w, h| {
            ui::scroll_bar::overrides_right_edge_resize_for_state(&ui_nc.borrow(), cx, cy, w, h)
        })),
        on_input: Some(Box::new(move |event, hwnd, width, height| {
            let mut app_mut = app_input.borrow_mut();
            let mut ui_mut = ui_input.borrow_mut();
            let mut mon = monitor_input.borrow_mut();
            let viewport = Viewport {
                hwnd,
                width,
                height,
            };
            let hooks = SettingsHooks {
                hwnd,
                hotkey: ctx_input.hotkey_ptr,
                tray: ctx_input.tray_ptr,
            };
            let settings_hooks = if ui_mut.show_settings {
                Some(&hooks)
            } else {
                None
            };
            if handle_input(
                event,
                &mut app_mut,
                &mut ui_mut,
                viewport,
                &mut mon,
                settings_hooks,
            ) {
                app_mut.set_needs_repaint();
                // SAFETY: `hwnd` is the live main window.
                unsafe {
                    let _ = InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
        })),
        on_tray: Some(Box::new(move |wparam, lparam| {
            // SAFETY: `tray_ptr` valid for the message-loop lifetime in `run`.
            let tray_ref = unsafe { &*ctx_tray.tray_ptr };
            let pause = ctx_tray.app.borrow().pause_capture;
            if let Some(action) = tray_ref.handle_callback(wparam, lparam, pause) {
                apply_tray_action(action, hwnd, &ctx_tray);
            }
        })),
        on_command: Some(Box::new(move |command_id| {
            if let Some(action) = TrayIcon::menu_action(command_id) {
                apply_tray_action(action, hwnd, &ctx_cmd);
            }
        })),
        on_hotkey: Some(Box::new(move |id| {
            if id != TRAYVAULT_HOTKEY_ID as u32 {
                return;
            }
            // SAFETY: `hotkey_ptr` valid for the message-loop lifetime in `run`.
            let hotkey_ref = unsafe { &*ctx_hotkey.hotkey_ptr };
            if !hotkey_ref.is_registered() {
                return;
            }
            if win32::window::is_window_visible(hwnd) {
                hide_main_window(hwnd, &ctx_hotkey.app);
            } else {
                show_main_window(hwnd, &ctx_hotkey.app, &ctx_hotkey.ui);
            }
        })),
        on_close: Some(Box::new(move || {
            hide_main_window(hwnd, &ctx_close.app);
        })),
        on_geometry_changed: Some(Box::new(move || {
            if let Err(err) = ctx_geom
                .app
                .borrow_mut()
                .persist_window_geometry(hwnd, &ctx_geom.config_path)
            {
                log::warn(&format!("window geometry save failed: {err}"));
            }
        })),
    });
}

impl TrayContext {
    fn clone_handles(&self) -> Self {
        Self {
            app: Rc::clone(&self.app),
            ui: Rc::clone(&self.ui),
            monitor: Rc::clone(&self.monitor),
            config_path: self.config_path.clone(),
            quitting: Rc::clone(&self.quitting),
            tray_ptr: self.tray_ptr,
            hotkey_ptr: self.hotkey_ptr,
        }
    }
}

fn apply_tray_action(action: TrayMenuAction, hwnd: win32::ffi::HWND, ctx: &TrayContext) {
    match action {
        TrayMenuAction::ToggleWindow => {
            if win32::window::is_window_visible(hwnd) {
                hide_main_window(hwnd, &ctx.app);
            } else {
                show_main_window(hwnd, &ctx.app, &ctx.ui);
            }
        }
        TrayMenuAction::ShowWindow => show_main_window(hwnd, &ctx.app, &ctx.ui),
        TrayMenuAction::TogglePause => {
            let paused = match ctx.app.borrow_mut().toggle_pause_capture() {
                Ok(p) => p,
                Err(err) => {
                    log::warn(&format!("pause toggle failed: {err}"));
                    ctx.app.borrow().pause_capture
                }
            };
            ctx.monitor
                .borrow_mut()
                .set_config(ctx.app.borrow().config.capture_config());
            // SAFETY: `tray_ptr` valid for the message-loop lifetime in `run`.
            let tray_ref = unsafe { &*ctx.tray_ptr };
            let _ = update_tray_tooltip(tray_ref, paused);
        }
        TrayMenuAction::Settings => {
            show_main_window(hwnd, &ctx.app, &ctx.ui);
            let config = ctx.app.borrow().config.clone();
            ctx.ui.borrow_mut().open_settings(&config);
            request_window_repaint(hwnd);
        }
        TrayMenuAction::Quit => {
            // SAFETY: `tray_ptr` valid for the message-loop lifetime in `run`.
            let tray_mut = unsafe { &mut *ctx.tray_ptr };
            quit_app(hwnd, ctx, tray_mut);
        }
    }
}

fn show_main_window(
    hwnd: win32::ffi::HWND,
    app: &Rc<RefCell<app::App>>,
    ui: &Rc<RefCell<UiState>>,
) {
    let show_in_taskbar = app.borrow().config.show_in_taskbar;
    win32::window::show_window(hwnd, show_in_taskbar);
    app.borrow_mut().on_show_window();
    let caret = app.borrow().filter_query.len();
    let mut ui_mut = ui.borrow_mut();
    ui_mut.search_focused = true;
    ui_mut.search_caret = caret;
    ui_mut.search_sel_anchor = caret;
}

fn hide_main_window(hwnd: win32::ffi::HWND, app: &Rc<RefCell<app::App>>) {
    win32::window::hide_window(hwnd);
    app.borrow_mut().on_hide_window();
}

fn update_tray_tooltip(tray: &TrayIcon, pause_capture: bool) -> Result<()> {
    let tip = if pause_capture {
        "TrayVault (capture paused)"
    } else {
        "TrayVault"
    };
    tray.set_tooltip(tip)
}

fn quit_app(hwnd: win32::ffi::HWND, ctx: &TrayContext, tray: &mut TrayIcon) {
    if ctx.quitting.get() {
        return;
    }
    ctx.quitting.set(true);

    if let Err(err) = ctx.app.borrow_mut().shutdown(hwnd, &ctx.config_path) {
        log::warn(&format!("shutdown save failed: {err}"));
    }

    if let Err(err) = ctx.monitor.borrow_mut().unregister_listener() {
        log::warn(&format!("clipboard listener unregister failed: {err}"));
    }

    tray.remove();

    // SAFETY: `hotkey_ptr` valid for the message-loop lifetime in `run`.
    let hotkey_ref = unsafe { &mut *ctx.hotkey_ptr };
    hotkey_ref.unregister(hwnd);

    // SAFETY: ends the message loop via WM_DESTROY → PostQuitMessage.
    unsafe {
        DestroyWindow(hwnd);
    }
}

/// Returns true when `--minimized` is present on the command line.
fn parse_minimized_flag() -> bool {
    std::env::args().any(|arg| arg == "--minimized")
}

/// Route panics to the log file before the default handler runs, so crashes in
/// release builds (which have no console) leave a trace in `trayvault.log`.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error(&format!("panic: {info}"));
        default_hook(info);
    }));
}
