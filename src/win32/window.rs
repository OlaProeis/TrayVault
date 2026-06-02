//! Main window: class registration, [`WndProc`], and the single-threaded message loop.
//!
//! All OS callbacks (`WM_PAINT`, `WM_TIMER`, `WM_HOTKEY`, `WM_CLIPBOARDUPDATE`,
//! tray messages, etc.) arrive here on the main thread and are routed to stubs or
//! future app-layer handlers.

#![allow(dead_code, clippy::type_complexity)] // public API surface for later tasks

use std::cell::Cell;
use std::mem::MaybeUninit;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Once;

use crate::config::{clamp_client_dimensions, Config};
use crate::error::Result;
use crate::log;
use crate::win32::gdi::{self, GdiBuffer};
use crate::win32::{ffi, last_error, wide};

use ffi::{
    AdjustWindowRectEx, BringWindowToTop, CreateSolidBrush, CreateWindowExW, DefWindowProcW,
    DestroyWindow, DispatchMessageW, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
    GetClientRect, GetKeyState, GetMessageW, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect,
    InvalidateRect, IsIconic, IsWindowVisible, KillTimer, LoadCursorW, PostQuitMessage,
    RegisterClassW, ScreenToClient, SetFocus, SetForegroundWindow, SetTimer, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, TranslateMessage, CW_USEDEFAULT, DWMNCRP_DISABLED,
    DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWM_BORDER_DARK_GRAY, GWL_EXSTYLE,
    GWL_STYLE, HDC, HINSTANCE, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT,
    HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, HWND, IDC_ARROW, LPARAM, LRESULT, MARGINS, MSG, POINT,
    RECT, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOW, UINT,
    VK_SHIFT, WM_ACTIVATE, WM_APP, WM_CAPTURECHANGED, WM_CHAR, WM_CLIPBOARDUPDATE, WM_CLOSE,
    WM_COMMAND, WM_DESTROY, WM_ENTERSIZEMOVE, WM_ERASEBKGND, WM_EXITSIZEMOVE, WM_GETICON,
    WM_HOTKEY, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_MOVE, WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCHITTEST, WM_NCLBUTTONDOWN,
    WM_NCLBUTTONUP, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SIZE, WM_TIMER, WNDCLASSW, WPARAM,
    WS_BORDERLESS, WS_CAPTION, WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU,
    WS_THICKFRAME,
};

// ---------------------------------------------------------------------------
// Window class / timer / custom message ids
// ---------------------------------------------------------------------------

const CLASS_NAME: &str = "TrayVaultBorderlessWindow";
/// Resize hit-test margin in client pixels (used with `WM_NCHITTEST`).
pub const RESIZE_BORDER: i32 = 6;
/// Minimum visible strip (px) when clamping a saved position onto the virtual screen.
const SCREEN_CLAMP_MARGIN: i32 = 48;
/// Must match [`crate::ui::titlebar::TITLE_BAR_HEIGHT`].
const TITLE_BAR_PX: i32 = 28;
/// Max pointer movement (screen px) to treat an `HTCAPTION` press in the search field as a click.
const SEARCH_CLICK_SLOP: i32 = 4;
const TIMER_ID: usize = 1;
/// Tray callback message id (Task 10).
pub const WM_TRAY_CALLBACK: UINT = WM_APP + 1;
/// Async thumbnail load completed (Task 39).
pub const WM_THUMB_READY: UINT = WM_APP + 2;

/// `WM_SIZE` wParam: window is minimized.
const SIZE_MINIMIZED: WPARAM = 1;

/// BGRA fill after resize before the next full UI paint (matches dark theme background).
const RESIZE_FILL_BGRA: (u8, u8, u8, u8) = (0x1E, 0x1E, 0x1E, 0xFF);

// ---------------------------------------------------------------------------
// Input events (Task 9 — routed to UI layer)
// ---------------------------------------------------------------------------

/// Mouse/keyboard events forwarded from [`WndProc`] to the app/UI layer.
#[derive(Clone, Copy, Debug)]
pub enum InputEvent {
    KeyDown(u32),
    Char(u32),
    MouseMove(i32, i32),
    LButtonDown(i32, i32),
    LButtonUp(i32, i32),
    LButtonDblClk(i32, i32),
    RButtonDown(i32, i32),
    RButtonUp(i32, i32),
    MouseWheel(i32, i32, i16),
    /// Mouse capture left this window (e.g. system stole capture during scrollbar drag).
    CaptureLost,
}

// ---------------------------------------------------------------------------
// Callback hooks (app layer plugs in later — Task 7)
// ---------------------------------------------------------------------------

/// Optional handlers invoked from [`WndProc`]. All default to no-ops / logging.
#[derive(Default)]
pub struct WindowCallbacks {
    pub on_timer: Option<Box<dyn FnMut() -> bool>>,
    pub on_hotkey: Option<Box<dyn FnMut(u32)>>,
    pub on_clipboard_update: Option<Box<dyn FnMut()>>,
    pub on_tray: Option<Box<dyn FnMut(WPARAM, LPARAM)>>,
    pub on_command: Option<Box<dyn FnMut(u32)>>,
    /// Invoked when the user closes the window chrome (hide-to-tray by default).
    pub on_close: Option<Box<dyn FnMut()>>,
    /// Invoked after the user finishes a move or resize (`WM_EXITSIZEMOVE`).
    pub on_geometry_changed: Option<Box<dyn FnMut()>>,
    pub on_paint: Option<Box<dyn FnMut(&mut GdiBuffer)>>,
    /// Invoked when a background thumbnail load posts [`WM_THUMB_READY`].
    pub on_thumb_ready: Option<Box<dyn FnMut()>>,
    pub on_input: Option<Box<dyn FnMut(InputEvent, HWND, i32, i32)>>,
    /// Return true to force `HTCLIENT` instead of a right-edge resize grip (scrollbar gutter).
    pub on_nc_client_override: Option<Box<dyn FnMut(i32, i32, i32, i32) -> bool>>,
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

/// The TrayVault main window and its GDI back-buffer.
pub struct Window {
    hwnd: HWND,
    gdi: GdiBuffer,
    callbacks: WindowCallbacks,
    visible: Cell<bool>,
    /// Screen coords of `WM_NCLBUTTONDOWN` when the press started in the search field.
    search_nc_down: Cell<Option<(i32, i32)>>,
    /// True during the modal move/resize loop (`WM_ENTERSIZEMOVE` … `WM_EXITSIZEMOVE`).
    in_size_move: Cell<bool>,
    /// `WM_CAPTURECHANGED` deferred until `dispatch` returns (avoids re-entering `on_input`
    /// while `RefCell` borrows from the current message are still active).
    pending_capture_lost: Cell<bool>,
    /// Application icon set via `WM_SETICON` (small + large). Destroyed on drop.
    icon: ffi::HICON,
}

impl Window {
    /// Register the window class (once) and create the `HWND`.
    pub fn create(hinstance: HINSTANCE, cfg: &Config) -> Result<Self> {
        register_class(hinstance)?;

        let class_name = CLASS_NAME_WIDE.get().expect("window class registered");
        let title = wide("TrayVault");

        let ex_style = WS_EX_TOOLWINDOW;
        let style = WS_BORDERLESS;
        let (client_w, client_h) =
            clamp_client_dimensions(cfg.window_client_w, cfg.window_client_h);
        let (win_w, win_h) =
            client_to_window_size(client_w as i32, client_h as i32, style, ex_style);
        let (pos_x, pos_y) = match (cfg.window_x, cfg.window_y) {
            (Some(x), Some(y)) => {
                let (x, y) = clamp_screen_position(x, y, win_w, win_h);
                (x, y)
            }
            _ => (CW_USEDEFAULT, CW_USEDEFAULT),
        };

        // SAFETY: `class_name` and `title` are NUL-terminated UTF-16; `hinstance`
        // is the module handle of this executable.
        let hwnd = unsafe {
            CreateWindowExW(
                ex_style,
                class_name.as_ptr(),
                title.as_ptr(),
                style,
                pos_x,
                pos_y,
                win_w,
                win_h,
                0,
                0,
                hinstance,
                ptr::null_mut(),
            )
        };
        if hwnd == 0 {
            return Err(last_error("CreateWindowExW"));
        }

        apply_borderless_style(hwnd);

        let icon = match crate::win32::tray::load_app_icon() {
            Ok(h) => {
                // SAFETY: hwnd and h are valid; WM_SETICON does not transfer ownership.
                unsafe {
                    ffi::SendMessageW(hwnd, ffi::WM_SETICON, ffi::ICON_SMALL, h as ffi::LPARAM);
                    ffi::SendMessageW(hwnd, ffi::WM_SETICON, ffi::ICON_BIG, h as ffi::LPARAM);
                }
                h
            }
            Err(e) => {
                log::warn(&format!("window icon load failed: {e}"));
                0
            }
        };

        let mut window = Self {
            hwnd,
            gdi: GdiBuffer::empty(),
            callbacks: WindowCallbacks::default(),
            visible: Cell::new(false),
            search_nc_down: Cell::new(None),
            in_size_move: Cell::new(false),
            pending_capture_lost: Cell::new(false),
            icon,
        };

        window.gdi.resize_to_client(hwnd)?;

        log::info(&format!("window created (hwnd = 0x{hwnd:016X})"));
        Ok(window)
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn gdi(&self) -> &GdiBuffer {
        &self.gdi
    }

    pub fn gdi_mut(&mut self) -> &mut GdiBuffer {
        &mut self.gdi
    }

    pub fn set_callbacks(&mut self, callbacks: WindowCallbacks) {
        self.callbacks = callbacks;
    }

    pub fn show(&self, show_in_taskbar: bool) {
        show_window(self.hwnd, show_in_taskbar);
        self.visible.set(true);
    }

    pub fn hide(&self) {
        hide_window(self.hwnd);
        self.visible.set(false);
    }

    pub fn is_visible(&self) -> bool {
        is_window_visible(self.hwnd)
    }

    pub fn request_repaint(&self) {
        request_window_repaint(self.hwnd);
    }

    /// Run the main-thread message loop until `WM_QUIT`.
    pub fn run_message_loop(&mut self) -> Result<()> {
        log::info("entering message loop");
        set_active_window(self);

        let result = self.pump_messages();

        clear_active_window();
        log::info("message loop exited");
        result
    }

    fn pump_messages(&mut self) -> Result<()> {
        let mut msg = MaybeUninit::<MSG>::uninit();
        loop {
            // SAFETY: `msg` points at valid stack storage.
            let ret = unsafe { GetMessageW(msg.as_mut_ptr(), 0, 0, 0) };
            if ret == 0 {
                // WM_QUIT received.
                let _msg = unsafe { msg.assume_init() };
                return Ok(());
            }
            if ret == -1 {
                return Err(last_error("GetMessageW"));
            }

            let msg = unsafe { msg.assume_init() };
            // SAFETY: standard message-loop translation/dispatch.
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    fn dispatch(&mut self, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        log_message(msg);

        let result = match msg {
            WM_PAINT => self.on_paint(),
            WM_TIMER => self.on_timer(wparam),
            WM_HOTKEY => self.on_hotkey(wparam),
            WM_CLIPBOARDUPDATE => self.on_clipboard_update(),
            WM_TRAY_CALLBACK => self.on_tray(wparam, lparam),
            WM_THUMB_READY => self.on_thumb_ready(),
            WM_COMMAND => self.on_command(wparam),
            WM_ENTERSIZEMOVE => {
                self.in_size_move.set(true);
                apply_dwm_borderless(self.hwnd);
                0
            }
            WM_EXITSIZEMOVE => {
                self.in_size_move.set(false);
                apply_dwm_borderless(self.hwnd);
                if let Some(ref mut hook) = self.callbacks.on_geometry_changed {
                    hook();
                }
                // Plain invalidation → one clean WM_PAINT. Do NOT force a synchronous
                // non-client (`RDW_FRAME`) repaint here — that re-exposes the system
                // frame and flashes a white edge.
                self.request_repaint();
                0
            }
            // A move changes no pixels — DWM relocates the composed surface. Painting
            // or repainting the frame on every WM_MOVE fights DWM and flashes a white
            // band along the outer edge while dragging, so we do nothing here.
            WM_MOVE => 0,
            WM_SIZE => self.on_size(wparam, lparam),
            WM_KEYDOWN => self.on_key_down(wparam),
            WM_CHAR => self.on_char(wparam),
            WM_CAPTURECHANGED => self.on_capture_changed(lparam),
            WM_MOUSEMOVE => self.on_mouse_move(lparam),
            WM_LBUTTONDOWN => self.on_lbutton_down(lparam),
            WM_LBUTTONUP => self.on_lbutton_up(lparam),
            WM_LBUTTONDBLCLK => self.on_lbutton_dblclk(lparam),
            WM_RBUTTONDOWN => self.on_rbutton_down(lparam),
            WM_RBUTTONUP => self.on_rbutton_up(),
            WM_MOUSEWHEEL => self.on_mouse_wheel(wparam, lparam),
            WM_ERASEBKGND => {
                // Let the default handler erase with the dark class brush. Normal
                // repaints use `InvalidateRect(bErase = 0)` so this fires only on
                // resize/uncover — exactly when the freshly exposed strip must be
                // dark (not system gray) before `WM_PAINT` redraws the UI.
                // SAFETY: standard default processing; `wparam` is the erase HDC.
                unsafe { DefWindowProcW(self.hwnd, msg, wparam, lparam) }
            }
            WM_ACTIVATE => {
                if loword(wparam as u32) != 0 {
                    apply_dwm_borderless(self.hwnd);
                }
                0
            }
            WM_NCACTIVATE => 1, // skip default NC frame paint (drag / tray focus flash)
            WM_NCCALCSIZE => self.on_nc_calc_size(wparam, lparam),
            WM_NCHITTEST => self.on_nc_hit_test(lparam),
            WM_NCLBUTTONDOWN => self.on_nc_lbutton_down(wparam, lparam),
            WM_NCLBUTTONUP => self.on_nc_lbutton_up(wparam, lparam),
            WM_CLOSE => {
                if let Some(ref mut hook) = self.callbacks.on_close {
                    hook();
                } else {
                    // SAFETY: request destroy; WM_DESTROY will post quit.
                    unsafe {
                        DestroyWindow(self.hwnd);
                    }
                }
                0
            }
            WM_DESTROY => {
                // SAFETY: stop the relative-time timer if still running (show/hide normally pair start/stop).
                unsafe {
                    KillTimer(self.hwnd, TIMER_ID);
                }
                // SAFETY: ends the message loop started by `run_message_loop`.
                unsafe {
                    PostQuitMessage(0);
                }
                0
            }
            WM_GETICON => {
                // Task Manager queries window icons via WM_GETICON (then GCL_HICON). WM_SETICON
                // alone is enough for the shell taskbar but not for Task Manager's process list.
                if self.icon != 0 && (wparam == ffi::ICON_BIG || wparam == ffi::ICON_SMALL) {
                    return self.icon as LRESULT;
                }
                // SAFETY: fall back to class icon / default handling.
                unsafe { DefWindowProcW(self.hwnd, msg, wparam, lparam) }
            }
            _ => {
                // SAFETY: delegate unhandled messages to the default handler.
                unsafe { DefWindowProcW(self.hwnd, msg, wparam, lparam) }
            }
        };
        self.flush_pending_capture_lost();
        result
    }

    fn on_paint(&mut self) -> LRESULT {
        if let Err(err) = self.paint_client() {
            log::error(&format!("WM_PAINT failed: {err}"));
        }
        0
    }

    /// Rasterize into the DIB and blit during `WM_PAINT`.
    fn paint_client(&mut self) -> Result<()> {
        gdi::with_paint(self.hwnd, |hdc| self.blit_ui(hdc))
    }

    /// Rasterize into the DIB and blit outside `WM_PAINT` (live move/resize).
    fn present_client(&mut self) -> Result<()> {
        gdi::with_dc(self.hwnd, |hdc| self.blit_ui(hdc))
    }

    fn blit_ui(&mut self, hdc: HDC) -> Result<()> {
        if let Some(ref mut hook) = self.callbacks.on_paint {
            hook(&mut self.gdi);
        }
        self.gdi.present_internal(hdc)
    }

    fn on_timer(&mut self, wparam: WPARAM) -> LRESULT {
        if wparam == TIMER_ID {
            let repaint = self.callbacks.on_timer.as_mut().is_some_and(|hook| hook());
            if repaint {
                self.request_repaint();
            }
        }
        0
    }

    fn on_hotkey(&mut self, wparam: WPARAM) -> LRESULT {
        let id = wparam as u32;
        if let Some(ref mut hook) = self.callbacks.on_hotkey {
            hook(id);
        }
        0
    }

    fn on_clipboard_update(&mut self) -> LRESULT {
        if let Some(ref mut hook) = self.callbacks.on_clipboard_update {
            hook();
        }
        0
    }

    fn on_tray(&mut self, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if let Some(ref mut hook) = self.callbacks.on_tray {
            hook(wparam, lparam);
        }
        0
    }

    fn on_thumb_ready(&mut self) -> LRESULT {
        if let Some(ref mut hook) = self.callbacks.on_thumb_ready {
            hook();
        }
        0
    }

    fn on_command(&mut self, wparam: WPARAM) -> LRESULT {
        let command_id = loword(wparam as u32) as u32;
        if let Some(ref mut hook) = self.callbacks.on_command {
            hook(command_id);
        }
        0
    }

    fn on_size(&mut self, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if wparam == SIZE_MINIMIZED {
            return 0;
        }
        let width = loword(lparam as u32) as i32;
        let height = hiword(lparam as u32) as i32;
        if width <= 0 || height <= 0 {
            return 0;
        }
        match self.gdi.resize(width, height) {
            Ok(()) => {
                let (b, g, r, a) = RESIZE_FILL_BGRA;
                self.gdi.fill_solid(b, g, r, a);
                self.request_repaint();
                if self.in_size_move.get() {
                    // Blit the freshly resized (dark-filled + re-rendered) buffer now so
                    // the newly exposed strip is never left showing white/garbage during
                    // the modal resize loop. No `RDW_FRAME` repaint — that flashes the
                    // system frame at the outer edge.
                    if let Err(err) = self.present_client() {
                        log::error(&format!("live resize present failed: {err}"));
                    }
                }
            }
            Err(err) => log::error(&format!("WM_SIZE resize failed: {err}")),
        }
        0
    }

    fn on_key_down(&mut self, wparam: WPARAM) -> LRESULT {
        if let Some(ref mut hook) = self.callbacks.on_input {
            hook(
                InputEvent::KeyDown(wparam as u32),
                self.hwnd,
                self.gdi.width(),
                self.gdi.height(),
            );
            self.request_repaint();
        }
        0
    }

    fn on_char(&mut self, wparam: WPARAM) -> LRESULT {
        if let Some(ref mut hook) = self.callbacks.on_input {
            hook(
                InputEvent::Char(wparam as u32),
                self.hwnd,
                self.gdi.width(),
                self.gdi.height(),
            );
            self.request_repaint();
        }
        0
    }

    fn on_capture_changed(&mut self, lparam: LPARAM) -> LRESULT {
        let gaining = lparam as HWND;
        if gaining != self.hwnd {
            self.pending_capture_lost.set(true);
        }
        0
    }

    fn flush_pending_capture_lost(&mut self) {
        if !self.pending_capture_lost.replace(false) {
            return;
        }
        if let Some(ref mut hook) = self.callbacks.on_input {
            hook(
                InputEvent::CaptureLost,
                self.hwnd,
                self.gdi.width(),
                self.gdi.height(),
            );
        }
    }

    fn on_mouse_move(&mut self, lparam: LPARAM) -> LRESULT {
        let (x, y) = client_coords(lparam);
        if let Some(ref mut hook) = self.callbacks.on_input {
            hook(
                InputEvent::MouseMove(x, y),
                self.hwnd,
                self.gdi.width(),
                self.gdi.height(),
            );
        }
        0
    }

    fn on_lbutton_down(&mut self, lparam: LPARAM) -> LRESULT {
        let (x, y) = client_coords(lparam);
        if let Some(ref mut hook) = self.callbacks.on_input {
            hook(
                InputEvent::LButtonDown(x, y),
                self.hwnd,
                self.gdi.width(),
                self.gdi.height(),
            );
            self.request_repaint();
        }
        0
    }

    fn on_lbutton_up(&mut self, lparam: LPARAM) -> LRESULT {
        let (x, y) = client_coords(lparam);
        if let Some(ref mut hook) = self.callbacks.on_input {
            hook(
                InputEvent::LButtonUp(x, y),
                self.hwnd,
                self.gdi.width(),
                self.gdi.height(),
            );
            self.request_repaint();
        }
        0
    }

    fn on_lbutton_dblclk(&mut self, lparam: LPARAM) -> LRESULT {
        let (x, y) = client_coords(lparam);
        if let Some(ref mut hook) = self.callbacks.on_input {
            hook(
                InputEvent::LButtonDblClk(x, y),
                self.hwnd,
                self.gdi.width(),
                self.gdi.height(),
            );
            self.request_repaint();
        }
        0
    }

    fn on_rbutton_down(&mut self, lparam: LPARAM) -> LRESULT {
        let (x, y) = client_coords(lparam);
        if let Some(ref mut hook) = self.callbacks.on_input {
            hook(
                InputEvent::RButtonDown(x, y),
                self.hwnd,
                self.gdi.width(),
                self.gdi.height(),
            );
            self.request_repaint();
        }
        0
    }

    fn on_rbutton_up(&mut self) -> LRESULT {
        if let Some(ref mut hook) = self.callbacks.on_input {
            hook(
                InputEvent::RButtonUp(0, 0),
                self.hwnd,
                self.gdi.width(),
                self.gdi.height(),
            );
            self.request_repaint();
        }
        0
    }

    /// Tell Windows the entire window is client area (no native caption/frame paint).
    fn on_nc_calc_size(&self, _wparam: WPARAM, _lparam: LPARAM) -> LRESULT {
        // When `wParam != 0`, `rgrc[0]` already holds the proposed *new* window rect.
        // Returning 0 without shrinking it makes the whole window rect the client area,
        // so there is no non-client frame.
        //
        // Do NOT copy `rgrc[1]` (the *old* window rect) into `rgrc[0]`: during a
        // resize-grow the old rect is smaller than the new window, which leaves the
        // freshly added strip on the grown edge as non-client → a gray frame band that
        // lags the drag and can persist after release.
        0
    }

    fn on_nc_lbutton_down(&self, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if wparam == HTCAPTION {
            let pt = screen_point(lparam);
            if self.search_input_hit_screen(pt) {
                self.search_nc_down.set(Some((pt.x, pt.y)));
            }
        }
        // SAFETY: default processing starts caption drag / activation.
        unsafe { DefWindowProcW(self.hwnd, WM_NCLBUTTONDOWN, wparam, lparam) }
    }

    fn on_nc_lbutton_up(&mut self, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let mut handled_click = false;
        if wparam == HTCAPTION {
            if let Some((start_x, start_y)) = self.search_nc_down.get() {
                let pt = screen_point(lparam);
                let dx = pt.x - start_x;
                let dy = pt.y - start_y;
                if dx * dx + dy * dy <= SEARCH_CLICK_SLOP * SEARCH_CLICK_SLOP
                    && self.search_input_hit_screen(pt)
                {
                    let (cx, cy) = self.screen_to_client(pt);
                    if let Some(ref mut hook) = self.callbacks.on_input {
                        hook(
                            InputEvent::LButtonUp(cx, cy),
                            self.hwnd,
                            self.gdi.width(),
                            self.gdi.height(),
                        );
                        self.request_repaint();
                        handled_click = true;
                    }
                }
            }
        }
        self.search_nc_down.set(None);
        if handled_click {
            0
        } else {
            // SAFETY: complete default non-client button handling.
            unsafe { DefWindowProcW(self.hwnd, WM_NCLBUTTONUP, wparam, lparam) }
        }
    }

    fn search_input_hit_screen(&self, pt: POINT) -> bool {
        let w = self.gdi.width();
        if w <= 0 {
            return false;
        }
        let (cx, cy) = self.screen_to_client(pt);
        crate::ui::titlebar::search_input_is_hit(cx as f32, cy as f32, w as f32)
    }

    fn screen_to_client(&self, pt: POINT) -> (i32, i32) {
        let mut client_pt = pt;
        // SAFETY: `hwnd` is valid; `client_pt` is updated in place.
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut client_pt);
        }
        (client_pt.x, client_pt.y)
    }

    /// Edge hit-testing for resize grips on a captionless window.
    fn on_nc_hit_test(&mut self, lparam: LPARAM) -> LRESULT {
        let mut pt = POINT {
            x: signed_loword(lparam),
            y: signed_hiword(lparam),
        };
        // SAFETY: `hwnd` is valid; `pt` is screen space.
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut pt);
        }

        let w = self.gdi.width();
        let h = self.gdi.height();
        if w <= 0 || h <= 0 {
            return HTCLIENT;
        }

        let cx = pt.x;
        let cy = pt.y;
        let b = RESIZE_BORDER;

        if cy < TITLE_BAR_PX
            && crate::ui::titlebar::title_bar_is_client_hit(cx as f32, cy as f32, w as f32)
        {
            return HTCLIENT;
        }

        if cx < b {
            if cy < b {
                HTTOPLEFT
            } else if cy >= h - b {
                HTBOTTOMLEFT
            } else {
                HTLEFT
            }
        } else if cx >= w - b {
            if cy < b {
                HTTOPRIGHT
            } else if cy >= h - b {
                HTBOTTOMRIGHT
            } else if self
                .callbacks
                .on_nc_client_override
                .as_mut()
                .is_some_and(|hook| hook(cx, cy, w, h))
            {
                HTCLIENT
            } else {
                HTRIGHT
            }
        } else if cy < b {
            HTTOP
        } else if cy >= h - b {
            HTBOTTOM
        } else if cy < TITLE_BAR_PX {
            // Title-bar drag via HTCAPTION (do not use SendMessage SC_MOVE — re-enters WndProc
            // while app/ui RefCells are borrowed and panics).
            2 // HTCAPTION
        } else {
            HTCLIENT
        }
    }

    fn on_mouse_wheel(&mut self, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let delta = hiword(wparam as u32) as i16;
        let mut pt = POINT {
            x: loword(lparam as u32) as i32,
            y: hiword(lparam as u32) as i32,
        };
        // SAFETY: convert screen coords to client space for hit testing.
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut pt);
        }
        if let Some(ref mut hook) = self.callbacks.on_input {
            hook(
                InputEvent::MouseWheel(pt.x, pt.y, delta),
                self.hwnd,
                self.gdi.width(),
                self.gdi.height(),
            );
            self.request_repaint();
        }
        0
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        if self.hwnd != 0 {
            // SAFETY: destroy the native window if still alive.
            unsafe {
                DestroyWindow(self.hwnd);
            }
            self.hwnd = 0;
        }
        // Icon handle is process-lifetime (shared with tray + window class); not destroyed here.
        self.icon = 0;
    }
}

// ---------------------------------------------------------------------------
// WndProc + active-window pointer (main thread only)
// ---------------------------------------------------------------------------

/// Pointer to the [`Window`] currently running its message loop.
///
/// Stored in an [`AtomicPtr`] (main thread only; `SeqCst` load/store) so
/// `WndProc` can dispatch without `static mut`. Callbacks must not form
/// `&Window` from this pointer while `dispatch` holds `&mut Window` — use
/// [`HWND`]-based helpers ([`show_window`], [`hide_window`], etc.) instead.
static ACTIVE_WINDOW: AtomicPtr<Window> = AtomicPtr::new(ptr::null_mut());

fn set_active_window(window: &mut Window) {
    ACTIVE_WINDOW.store(ptr::from_mut(window), Ordering::SeqCst);
}

fn clear_active_window() {
    ACTIVE_WINDOW.store(ptr::null_mut(), Ordering::SeqCst);
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = ACTIVE_WINDOW.load(Ordering::SeqCst);
    if !ptr.is_null() {
        // SAFETY: pointer is valid for the message-loop lifetime on the main thread.
        let window = unsafe { &mut *ptr };
        if window.hwnd == hwnd {
            return window.dispatch(msg, wparam, lparam);
        }
    }
    // SAFETY: default processing for messages outside the active loop (e.g. WM_CREATE).
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

static REGISTER_ONCE: Once = Once::new();
/// Process-lifetime UTF-16 class name — `RegisterClassW` stores the pointer.
static CLASS_NAME_WIDE: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();

fn register_class(hinstance: HINSTANCE) -> Result<()> {
    let mut err = None;
    REGISTER_ONCE.call_once(|| {
        let class_name = CLASS_NAME_WIDE.get_or_init(|| wide(CLASS_NAME));
        // SAFETY: `LoadCursorW` with `IDC_ARROW` is a documented pseudo-handle.
        let cursor = unsafe { LoadCursorW(0, IDC_ARROW) };
        // Solid dark brush matching the UI background. When the window grows during a
        // live resize, Windows clears the newly exposed strip with the class brush
        // before our paint lands; a dark brush keeps that transient fill invisible
        // instead of flashing the system gray default. Process-lifetime handle.
        // SAFETY: `CreateSolidBrush` takes a COLORREF (0x00BBGGRR) and cannot fail here.
        let background = unsafe { CreateSolidBrush(DWM_BORDER_DARK_GRAY) };
        let class_icon = crate::win32::tray::load_app_icon().unwrap_or_else(|e| {
            log::warn(&format!("window class icon load failed: {e}"));
            0
        });
        let wc = WNDCLASSW {
            style: 0, // no CS_HREDRAW/CS_VREDRAW — avoids partial invalidation stripes on resize
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: class_icon,
            hCursor: cursor,
            hbrBackground: background,
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        // SAFETY: `wc` is valid; `class_name` lives in `CLASS_NAME_WIDE` for process lifetime.
        let atom = unsafe { RegisterClassW(&wc) };
        if atom == 0 {
            err = Some(last_error("RegisterClassW"));
        }
    });
    err.map_or(Ok(()), Err)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn loword(v: u32) -> u16 {
    (v & 0xFFFF) as u16
}

fn hiword(v: u32) -> u16 {
    ((v >> 16) & 0xFFFF) as u16
}

fn client_coords(lparam: LPARAM) -> (i32, i32) {
    (loword(lparam as u32) as i32, hiword(lparam as u32) as i32)
}

fn signed_loword(lparam: LPARAM) -> i32 {
    ((lparam as i32) & 0xFFFF) as i16 as i32
}

fn signed_hiword(lparam: LPARAM) -> i32 {
    (((lparam as i32) >> 16) & 0xFFFF) as i16 as i32
}

fn screen_point(lparam: LPARAM) -> POINT {
    POINT {
        x: signed_loword(lparam),
        y: signed_hiword(lparam),
    }
}

/// Show the main window, bring it to the foreground, and optionally expose a taskbar button.
pub fn show_window(hwnd: HWND, show_in_taskbar: bool) {
    // SAFETY: `hwnd` is a valid top-level window on the message-loop thread.
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        SetFocus(hwnd);
    }
    set_taskbar_button_visible(hwnd, show_in_taskbar);
    apply_dwm_borderless(hwnd);
    start_relative_time_timer(hwnd);
    request_window_repaint(hwnd);
}

/// Hide the main window and remove its taskbar button.
pub fn hide_window(hwnd: HWND) {
    stop_relative_time_timer(hwnd);
    set_taskbar_button_visible(hwnd, false);
    // SAFETY: `hwnd` is a valid top-level window on the message-loop thread.
    unsafe {
        ShowWindow(hwnd, SW_HIDE);
    }
}

const RELATIVE_TIME_TIMER_MS: u32 = 2_000;

fn start_relative_time_timer(hwnd: HWND) {
    // SAFETY: `hwnd` is valid; no timer proc — WM_TIMER is handled in `on_timer`.
    let timer = unsafe { SetTimer(hwnd, TIMER_ID, RELATIVE_TIME_TIMER_MS, None) };
    if timer == 0 {
        log::warn("SetTimer failed — relative-time refresh disabled while visible");
    }
}

fn stop_relative_time_timer(hwnd: HWND) {
    // SAFETY: KillTimer on a non-existent timer is harmless.
    unsafe {
        KillTimer(hwnd, TIMER_ID);
    }
}

/// Query Win32 visibility (covers Esc/copy-back paths that call `ShowWindow` directly).
pub fn is_window_visible(hwnd: HWND) -> bool {
    // SAFETY: `hwnd` is valid.
    unsafe { IsWindowVisible(hwnd) != 0 }
}

/// Invalidate the full client area so the next `WM_PAINT` redraws.
pub fn request_window_repaint(hwnd: HWND) {
    // SAFETY: full-client invalidation; `bErase = false` — we paint everything.
    unsafe {
        let _ = InvalidateRect(hwnd, ptr::null(), 0);
    }
}

/// Show or hide the main window on the Windows taskbar.
///
/// When hidden, TrayVault always uses `WS_EX_TOOLWINDOW` so it stays out of the
/// taskbar and Alt+Tab. When visible, callers pass `show_in_taskbar` from config.
pub fn set_taskbar_button_visible(hwnd: HWND, visible: bool) {
    // SAFETY: `hwnd` is the live main window on the message-loop thread.
    unsafe {
        let mut ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if visible {
            ex_style &= !WS_EX_TOOLWINDOW;
        } else {
            ex_style |= WS_EX_TOOLWINDOW;
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);
        let _ = SetWindowPos(
            hwnd,
            0,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// Read the live window placement into `config` (skipped when minimized).
///
/// Returns `true` when fields were updated.
pub fn capture_geometry_into_config(hwnd: HWND, config: &mut Config) -> bool {
    // SAFETY: `hwnd` is the main window on the message-loop thread.
    if unsafe { IsIconic(hwnd) } != 0 {
        return false;
    }

    let mut window_rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let mut client_rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: valid `RECT` out pointers for a valid `hwnd`.
    let ok_window = unsafe { GetWindowRect(hwnd, &mut window_rect) };
    let ok_client = unsafe { GetClientRect(hwnd, &mut client_rect) };
    if ok_window == 0 || ok_client == 0 {
        log::warn("window geometry: GetWindowRect/GetClientRect failed");
        return false;
    }

    let client_w = (client_rect.right - client_rect.left).max(0) as u32;
    let client_h = (client_rect.bottom - client_rect.top).max(0) as u32;
    if client_w == 0 || client_h == 0 {
        return false;
    }

    let outer_w = window_rect.right - window_rect.left;
    let outer_h = window_rect.bottom - window_rect.top;
    let (x, y) = clamp_screen_position(window_rect.left, window_rect.top, outer_w, outer_h);
    let (w, h) = clamp_client_dimensions(client_w, client_h);

    config.window_x = Some(x);
    config.window_y = Some(y);
    config.window_client_w = w;
    config.window_client_h = h;
    true
}

/// Keep at least [`SCREEN_CLAMP_MARGIN`] pixels of the window on the virtual screen.
fn clamp_screen_position(x: i32, y: i32, outer_w: i32, outer_h: i32) -> (i32, i32) {
    // SAFETY: `GetSystemMetrics` has no failure mode for these indices.
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    let margin = SCREEN_CLAMP_MARGIN;
    let min_x = vx - outer_w + margin;
    let max_x = vx + vw - margin;
    let min_y = vy - outer_h + margin;
    let max_y = vy + vh - margin;
    let x = if min_x <= max_x {
        x.clamp(min_x, max_x)
    } else {
        vx
    };
    let y = if min_y <= max_y {
        y.clamp(min_y, max_y)
    } else {
        vy
    };
    (x, y)
}

fn client_to_window_size(client_w: i32, client_h: i32, style: u32, ex_style: u32) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: client_w,
        bottom: client_h,
    };
    // SAFETY: valid `RECT`; style/ex_style match `CreateWindowExW`.
    let ok = unsafe { AdjustWindowRectEx(&mut rect, style, 0, ex_style) };
    if ok == 0 {
        return (client_w, client_h);
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

/// Strip any caption/system-menu styles Windows may add and refresh the frame.
fn apply_borderless_style(hwnd: HWND) {
    // SAFETY: `hwnd` is a valid window just created on this thread.
    unsafe {
        let mut style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        style &= !(WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX);
        style |= WS_THICKFRAME;
        SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize);
        apply_dwm_borderless(hwnd);
        let _ = SetWindowPos(
            hwnd,
            0,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// Disable DWM non-client rendering so resize/move does not flash a system frame.
fn apply_dwm_borderless(hwnd: HWND) {
    // SAFETY: `hwnd` is valid; attribute buffers are stack locals.
    unsafe {
        let policy = DWMNCRP_DISABLED;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            (&policy as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        );
        let corner = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&corner as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        );
        let dark = 1u32;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        );
        let border = DWM_BORDER_DARK_GRAY;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            (&border as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        );
        // Zero margins: extending the DWM glass frame (-1) desyncs from GDI during
        // live resize and can flash a white/light band at the outer edge (Win10/11).
        let margins = MARGINS::default();
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
    }
}

fn log_message(msg: UINT) {
    let name = match msg {
        WM_HOTKEY => "WM_HOTKEY",
        WM_CLIPBOARDUPDATE => "WM_CLIPBOARDUPDATE",
        WM_TRAY_CALLBACK => "WM_TRAY_CALLBACK",
        WM_COMMAND => "WM_COMMAND",
        WM_SIZE => "WM_SIZE",
        WM_KEYDOWN => "WM_KEYDOWN",
        WM_CHAR => "WM_CHAR",
        WM_LBUTTONDOWN => "WM_LBUTTONDOWN",
        WM_NCACTIVATE => "WM_NCACTIVATE",
        WM_NCCALCSIZE => "WM_NCCALCSIZE",
        WM_CLOSE => "WM_CLOSE",
        WM_DESTROY => "WM_DESTROY",
        _ => return,
    };
    log::info(&format!("msg: {name} (0x{msg:04X})"));
}

/// Returns true when either shift key is held.
pub fn shift_held() -> bool {
    // SAFETY: documented GetKeyState usage — high bit set means down.
    unsafe { (GetKeyState(VK_SHIFT) as u16) & 0x8000 != 0 }
}
