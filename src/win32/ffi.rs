//! Hand-declared Win32 FFI surface.
//!
//! This module is the single source of truth for raw bindings. TrayVault does
//! **not** depend on `windows`/`windows-sys`; every type alias, `#[repr(C)]`
//! struct, constant, and `extern "system"` function below is written by hand
//! and must match the Win32 ABI for `x86_64-pc-windows-msvc` exactly.
//!
//! # ABI conventions
//! - Handles (`HWND`, `HINSTANCE`, `HDC`, ...) are pointer-sized opaque values.
//!   They are modeled as [`isize`] (not raw pointers) so they are `Copy`,
//!   `Send`, comparable, and easy to store in app state. On the Win64 calling
//!   convention an integer and a pointer are passed identically in registers,
//!   so this is ABI-correct. A *null* handle is `0`.
//! - `extern "system"` resolves to the Win64 calling convention (same as
//!   `extern "C"` on x64) — this is what every Win32 API uses.
//! - All string APIs are the wide (`*W`, UTF-16) variants. Build NUL-terminated
//!   UTF-16 buffers with [`crate::win32::wide`].
//!
//! # Extending this module
//! Bindings are grouped by owning DLL. When a later task needs new APIs, add the
//! declaration to the matching `#[link(...)]` block (or a new one), keeping the
//! same hand-verified style. Known future additions and their DLLs:
//! - clipboard read/write + listener  → `user32` (`OpenClipboard`, `GetClipboardData`, `AddClipboardFormatListener`, ...)
//! - source-app + process query       → `kernel32` (`OpenProcess`, `QueryFullProcessImageNameW`)
//! - system tray                      → `shell32` (`Shell_NotifyIconW`, `NOTIFYICONDATAW`)
//! - menus                            → `user32` (`CreatePopupMenu`, `TrackPopupMenu`, ...)
//! - autostart + theme detection      → `advapi32` (`RegCreateKeyExW`, `RegSetValueExW`, `RegQueryValueExW`, ...)

// Win32 type/function names intentionally mirror the platform headers
// (uppercase acronyms, non-snake-case), and many bindings are declared ahead of
// the task that first uses them.
#![allow(
    non_snake_case,
    non_camel_case_types,
    dead_code,
    clippy::upper_case_acronyms
)]

use core::ffi::c_void;
use core::ptr::null_mut;

// ---------------------------------------------------------------------------
// Primitive type aliases
// ---------------------------------------------------------------------------

pub type BOOL = i32;
pub type BYTE = u8;
pub type WORD = u16;
pub type DWORD = u32;
pub type UINT = u32;
pub type INT = i32;
pub type LONG = i32;
pub type ULONG = u32;
pub type SHORT = i16;
pub type WCHAR = u16;
pub type ATOM = u16;

/// Pointer-sized message parameters / results.
pub type WPARAM = usize;
pub type LPARAM = isize;
pub type LRESULT = isize;
/// Pointer-sized unsigned integer (`UINT_PTR`), e.g. timer ids.
pub type UINT_PTR = usize;

/// Opaque, pointer-sized OS handle. Null is `0`.
pub type HANDLE = isize;
pub type HWND = HANDLE;
pub type HINSTANCE = HANDLE;
pub type HMODULE = HANDLE;
pub type HICON = HANDLE;
pub type HCURSOR = HANDLE; // semantically an HICON
pub type HBRUSH = HANDLE;
pub type HMENU = HANDLE;
pub type HDC = HANDLE;
pub type HBITMAP = HANDLE;
pub type HGDIOBJ = HANDLE;
pub type HFONT = HANDLE;
pub type HGLOBAL = HANDLE;
pub type HKEY = HANDLE;

/// Registry value type: 32-bit unsigned integer.
pub const REG_DWORD: DWORD = 4;

/// Registry value type: null-terminated wide string.
pub const REG_SZ: DWORD = 1;

/// `RegOpenKeyExW` / `RegCreateKeyExW` desired access: read.
pub const KEY_READ: DWORD = 0x20019;

/// `RegCreateKeyExW` / `RegOpenKeyExW` desired access: write values.
pub const KEY_WRITE: DWORD = 0x20006;

/// `RegSetValueExW` / `RegDeleteValueW` desired access.
pub const KEY_SET_VALUE: DWORD = 0x0002;

/// Win32 error: registry value not found (`RegDeleteValueW`).
pub const ERROR_FILE_NOT_FOUND: DWORD = 2;

/// `RegQueryValueExW` success.
pub const ERROR_SUCCESS: DWORD = 0;

/// Predefined root: `HKEY_CURRENT_USER`.
pub const HKEY_CURRENT_USER: HKEY = 0x8000_0001_u32 as isize;

pub type LPVOID = *mut c_void;
pub type LPCVOID = *const c_void;
pub type LPWSTR = *mut WCHAR;
pub type LPCWSTR = *const WCHAR;

/// Window procedure signature. `None` is a null `WNDPROC`.
pub type WNDPROC = Option<unsafe extern "system" fn(HWND, UINT, WPARAM, LPARAM) -> LRESULT>;

/// `TIMERPROC` callback (we pass `None` and handle `WM_TIMER` instead).
pub type TIMERPROC = Option<unsafe extern "system" fn(HWND, UINT, UINT_PTR, DWORD)>;

// ---------------------------------------------------------------------------
// Structs (layouts verified against the Win32 headers)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct POINT {
    pub x: LONG,
    pub y: LONG,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RECT {
    pub left: LONG,
    pub top: LONG,
    pub right: LONG,
    pub bottom: LONG,
}

/// Used by [`WM_NCCALCSIZE`] when `wParam` is non-zero.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct WINDOWPOS {
    pub hwnd: HWND,
    pub hwndInsertAfter: HWND,
    pub x: INT,
    pub y: INT,
    pub cx: INT,
    pub cy: INT,
    pub flags: UINT,
}

/// [`WM_NCCALCSIZE`] layout when `wParam` is non-zero.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NCCALCSIZE_PARAMS {
    pub rgrc: [RECT; 3],
    pub lppos: *mut WINDOWPOS,
}

impl Default for NCCALCSIZE_PARAMS {
    fn default() -> Self {
        Self {
            rgrc: [RECT::default(); 3],
            lppos: null_mut(),
        }
    }
}

/// [`DwmExtendFrameIntoClientArea`] margins.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MARGINS {
    pub cxLeftWidth: INT,
    pub cxRightWidth: INT,
    pub cyTopHeight: INT,
    pub cyBottomHeight: INT,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: UINT,
    pub wParam: WPARAM,
    pub lParam: LPARAM,
    pub time: DWORD,
    pub pt: POINT,
}

/// Window class used with [`RegisterClassW`] (the non-EX variant).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WNDCLASSW {
    pub style: UINT,
    pub lpfnWndProc: WNDPROC,
    pub cbClsExtra: INT,
    pub cbWndExtra: INT,
    pub hInstance: HINSTANCE,
    pub hIcon: HICON,
    pub hCursor: HCURSOR,
    pub hbrBackground: HBRUSH,
    pub lpszMenuName: LPCWSTR,
    pub lpszClassName: LPCWSTR,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PAINTSTRUCT {
    pub hdc: HDC,
    pub fErase: BOOL,
    pub rcPaint: RECT,
    pub fRestore: BOOL,
    pub fIncUpdate: BOOL,
    pub rgbReserved: [BYTE; 32],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RGBQUAD {
    pub rgbBlue: BYTE,
    pub rgbGreen: BYTE,
    pub rgbRed: BYTE,
    pub rgbReserved: BYTE,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct BITMAPINFOHEADER {
    pub biSize: DWORD,
    pub biWidth: LONG,
    pub biHeight: LONG,
    pub biPlanes: WORD,
    pub biBitCount: WORD,
    pub biCompression: DWORD,
    pub biSizeImage: DWORD,
    pub biXPelsPerMeter: LONG,
    pub biYPelsPerMeter: LONG,
    pub biClrUsed: DWORD,
    pub biClrImportant: DWORD,
}

/// `BITMAPINFO` with a single inline color entry. For true-color DIBs the
/// color table is unused; the trailing `bmiColors` is a flexible array in C, so
/// when more entries are needed allocate a byte buffer and cast.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct BITMAPINFO {
    pub bmiHeader: BITMAPINFOHEADER,
    pub bmiColors: [RGBQUAD; 1],
}

/// [`SetTextColor`] / [`SetBkColor`] packed BGR.
pub type COLORREF = DWORD;

/// [`SetBkMode`] transparent background (glyph outline + text-out).
pub const TRANSPARENT: INT = 1;

/// [`SetBkMode`] opaque background.
pub const OPAQUE: INT = 2;

/// [`GetTextMetricsW`] output.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TEXTMETRICW {
    pub tmHeight: LONG,
    pub tmAscent: LONG,
    pub tmDescent: LONG,
    pub tmInternalLeading: LONG,
    pub tmExternalLeading: LONG,
    pub tmAveCharWidth: LONG,
    pub tmMaxCharWidth: LONG,
    pub tmWeight: LONG,
    pub tmOverhang: LONG,
    pub tmDigitizedAspectX: LONG,
    pub tmDigitizedAspectY: LONG,
    pub tmFirstChar: WCHAR,
    pub tmLastChar: WCHAR,
    pub tmDefaultChar: WCHAR,
    pub tmBreakChar: WCHAR,
    pub tmItalic: BYTE,
    pub tmUnderlined: BYTE,
    pub tmStruckOut: BYTE,
    pub tmPitchAndFamily: BYTE,
    pub tmCharSet: BYTE,
}

/// [`GetTextExtentPoint32W`] / [`GetCharABCWidthsFloatW`] size.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SIZE {
    pub cx: LONG,
    pub cy: LONG,
}

/// [`GetCharABCWidthsFloatW`] spacing triple.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ABCFLOAT {
    pub abcfA: f32,
    pub abcfB: f32,
    pub abcfC: f32,
}

/// GDI [`GetGlyphOutlineW`] / monochrome bitmap header in outline buffers.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct BITMAP {
    pub bmType: LONG,
    pub bmWidth: LONG,
    pub bmHeight: LONG,
    pub bmWidthBytes: LONG,
    pub bmPlanes: WORD,
    pub bmBitsPixel: WORD,
    pub bmBits: LPVOID,
}

impl Default for BITMAP {
    fn default() -> Self {
        Self {
            bmType: 0,
            bmWidth: 0,
            bmHeight: 0,
            bmWidthBytes: 0,
            bmPlanes: 0,
            bmBitsPixel: 0,
            bmBits: null_mut(),
        }
    }
}

/// Fixed-point value used by [`MAT2`] (`value` + `fract`/65536).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FIXED {
    pub fract: SHORT,
    pub value: SHORT,
}

/// World transform for [`GetGlyphOutlineW`].
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MAT2 {
    pub eM11: FIXED,
    pub eM12: FIXED,
    pub eM21: FIXED,
    pub eM22: FIXED,
}

/// Glyph placement metrics returned by [`GetGlyphOutlineW`].
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct GLYPHMETRICS {
    pub gmBlackBoxX: UINT,
    pub gmBlackBoxY: UINT,
    pub gmptGlyphOrigin: POINT,
    pub gmCellIncX: SHORT,
    pub gmCellIncY: SHORT,
}

/// `LOGFONTW::lfFaceName` capacity.
pub const LF_FACESIZE: usize = 32;

/// [`CreateFontIndirectW`] descriptor.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LOGFONTW {
    pub lfHeight: LONG,
    pub lfWidth: LONG,
    pub lfEscapement: LONG,
    pub lfOrientation: LONG,
    pub lfWeight: LONG,
    pub lfItalic: DWORD,
    pub lfUnderline: DWORD,
    pub lfStrikeOut: DWORD,
    pub lfCharSet: DWORD,
    pub lfOutPrecision: DWORD,
    pub lfClipPrecision: DWORD,
    pub lfQuality: DWORD,
    pub lfPitchAndFamily: DWORD,
    pub lfFaceName: [WCHAR; LF_FACESIZE],
}

/// System-tray notification data (`NOTIFYICONDATAW`, Vista+ layout).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NOTIFYICONDATAW {
    pub cbSize: DWORD,
    pub hWnd: HWND,
    pub uID: UINT,
    pub uFlags: UINT,
    pub uCallbackMessage: UINT,
    pub hIcon: HICON,
    pub szTip: [WCHAR; 128],
    pub dwState: DWORD,
    pub dwStateMask: DWORD,
    pub szInfo: [WCHAR; 256],
    pub uVersion: UINT,
    pub szInfoTitle: [WCHAR; 64],
    pub dwInfoFlags: DWORD,
    pub guidItem: [u8; 16],
    pub hBalloonIcon: HICON,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

// Window messages.
pub const WM_NULL: UINT = 0x0000;
pub const WM_CREATE: UINT = 0x0001;
pub const WM_DESTROY: UINT = 0x0002;
pub const WM_SIZE: UINT = 0x0005;
pub const WM_MOVE: UINT = 0x0003;
pub const WM_ENTERSIZEMOVE: UINT = 0x0231;
pub const WM_EXITSIZEMOVE: UINT = 0x0232;
pub const WM_SETFOCUS: UINT = 0x0007;
pub const WM_KILLFOCUS: UINT = 0x0008;
pub const WM_PAINT: UINT = 0x000F;
pub const WM_ACTIVATE: UINT = 0x0006;
pub const WM_CLOSE: UINT = 0x0010;
pub const WM_SYSCOMMAND: UINT = 0x0112;
pub const WM_QUIT: UINT = 0x0012;
pub const WM_ERASEBKGND: UINT = 0x0014;
pub const WM_NCACTIVATE: UINT = 0x0086;
pub const WM_NCCALCSIZE: UINT = 0x0083;
pub const WM_NCHITTEST: UINT = 0x0084;
pub const WM_NCLBUTTONDOWN: UINT = 0x00A1;
pub const WM_NCLBUTTONUP: UINT = 0x00A2;
pub const WM_SHOWWINDOW: UINT = 0x0018;
pub const WM_KEYDOWN: UINT = 0x0100;
pub const WM_KEYUP: UINT = 0x0101;
pub const WM_CHAR: UINT = 0x0102;
pub const WM_COMMAND: UINT = 0x0111;
pub const WM_TIMER: UINT = 0x0113;
pub const WM_MOUSEMOVE: UINT = 0x0200;
pub const WM_LBUTTONDOWN: UINT = 0x0201;
pub const WM_LBUTTONUP: UINT = 0x0202;
pub const WM_LBUTTONDBLCLK: UINT = 0x0203;
pub const WM_RBUTTONDOWN: UINT = 0x0204;
pub const WM_RBUTTONUP: UINT = 0x0205;
pub const WM_MOUSEWHEEL: UINT = 0x020A;
pub const WM_SETCURSOR: UINT = 0x0020;

// System command codes (WM_SYSCOMMAND wParam).
pub const SC_MOVE: WPARAM = 0xF010;
pub const HTCAPTION: WPARAM = 2;

// Virtual-key codes.
pub const VK_BACK: i32 = 0x08;
pub const VK_TAB: i32 = 0x09;
pub const VK_SHIFT: i32 = 0x10;
pub const VK_CONTROL: i32 = 0x11;
pub const VK_SPACE: i32 = 0x20;
pub const VK_PRIOR: i32 = 0x21; // Page Up
pub const VK_NEXT: i32 = 0x22; // Page Down
pub const VK_END: i32 = 0x23;
pub const VK_HOME: i32 = 0x24;
pub const VK_LEFT: i32 = 0x25;
pub const VK_UP: i32 = 0x26;
pub const VK_RIGHT: i32 = 0x27;
pub const VK_DOWN: i32 = 0x28;
pub const VK_INSERT: i32 = 0x2D;
pub const VK_DELETE: i32 = 0x2E;
pub const VK_RETURN: i32 = 0x0D;
pub const VK_ESCAPE: i32 = 0x1B;
pub const VK_F1: i32 = 0x70;
pub const VK_F12: i32 = 0x7B;
pub const VK_OEM_2: i32 = 0xBF; // `/` — Shift produces `?`

/// Win32 error: hotkey id/modifier/vk combo already registered.
pub const ERROR_HOTKEY_ALREADY_REGISTERED: u32 = 1409;

pub const WM_SETICON: UINT = 0x0080;
/// Retrieve the large or small icon associated with a window (Task Manager, Alt+Tab helpers).
pub const WM_GETICON: UINT = 0x007F;
/// `WM_SETICON` / `WM_GETICON` wParam: the small (16×16) window icon.
pub const ICON_SMALL: WPARAM = 0;
/// `WM_SETICON` / `WM_GETICON` wParam: the large (32×32) window icon.
pub const ICON_BIG: WPARAM = 1;

pub const WM_HOTKEY: UINT = 0x0312;
pub const WM_CLIPBOARDUPDATE: UINT = 0x031D;
/// Base for application-defined messages (e.g. the tray callback).
pub const WM_APP: UINT = 0x8000;
pub const WM_USER: UINT = 0x0400;

/// Tray activation notifications (carried in `LOWORD(lParam)` with `NOTIFYICON_VERSION_4`).
pub const NIN_SELECT: UINT = WM_USER;
pub const NIN_KEYSELECT: UINT = WM_USER + 1;

// Tray icon mouse messages (carried in `lParam` of the tray callback).
pub const WM_CONTEXTMENU: UINT = 0x007B;

// `Shell_NotifyIconW` commands.
pub const NIM_ADD: DWORD = 0x0000_0000;
pub const NIM_MODIFY: DWORD = 0x0000_0001;
pub const NIM_DELETE: DWORD = 0x0000_0002;
pub const NIM_SETVERSION: DWORD = 0x0000_0004;

// `NOTIFYICONDATAW` flags.
pub const NIF_MESSAGE: UINT = 0x0000_0001;
pub const NIF_ICON: UINT = 0x0000_0002;
pub const NIF_TIP: UINT = 0x0000_0004;
pub const NIF_STATE: UINT = 0x0000_0008;
pub const NIF_INFO: UINT = 0x0000_0010;

// Balloon notification styles.
pub const NIIF_NONE: DWORD = 0x0000_0000;
pub const NIIF_INFO: DWORD = 0x0000_0001;
pub const NIIF_WARNING: DWORD = 0x0000_0002;
pub const NIIF_ERROR: DWORD = 0x0000_0003;

/// `NOTIFYICONDATAW` balloon / extended version (Vista+).
pub const NOTIFYICON_VERSION_4: UINT = 4;

// Menu flags.
pub const MF_STRING: UINT = 0x0000_0000;
pub const MF_SEPARATOR: UINT = 0x0000_0800;
pub const MF_CHECKED: UINT = 0x0000_0008;
pub const MF_GRAYED: UINT = 0x0000_0001;
pub const MF_ENABLED: UINT = 0x0000_0000;

// `TrackPopupMenu` flags.
pub const TPM_LEFTALIGN: UINT = 0x0000;
pub const TPM_RIGHTBUTTON: UINT = 0x0002;
pub const TPM_BOTTOMALIGN: UINT = 0x0020;
pub const TPM_RETURNCMD: UINT = 0x0100;

// `LoadImageW` types / flags.
pub const IMAGE_ICON: UINT = 1;
pub const LR_LOADFROMFILE: UINT = 0x0010;
pub const LR_DEFAULTSIZE: UINT = 0x0040;

/// Stock application icon pseudo-handle for `LoadIconW`.
pub const IDI_APPLICATION: LPCWSTR = 32512 as LPCWSTR;

// Window styles.
pub const WS_OVERLAPPED: DWORD = 0x0000_0000;
pub const WS_POPUP: DWORD = 0x8000_0000;
pub const WS_VISIBLE: DWORD = 0x1000_0000;
pub const WS_CAPTION: DWORD = 0x00C0_0000;
pub const WS_SYSMENU: DWORD = 0x0008_0000;
pub const WS_THICKFRAME: DWORD = 0x0004_0000;
pub const WS_MINIMIZEBOX: DWORD = 0x0002_0000;
pub const WS_MAXIMIZEBOX: DWORD = 0x0001_0000;
pub const WS_OVERLAPPEDWINDOW: DWORD =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

/// Popup window with a sizing frame but no system caption (custom title bar).
pub const WS_BORDERLESS: DWORD = WS_POPUP | WS_THICKFRAME;

// GetWindowLong / SetWindowLong indices.
pub const GWL_STYLE: i32 = -16;
pub const GWL_EXSTYLE: i32 = -20;

// SetWindowPos flags.
pub const SWP_NOSIZE: UINT = 0x0001;
pub const SWP_NOMOVE: UINT = 0x0002;
pub const SWP_NOZORDER: UINT = 0x0004;
pub const SWP_NOACTIVATE: UINT = 0x0010;
pub const SWP_FRAMECHANGED: UINT = 0x0020;

/// [`RedrawWindow`] flags — repaint client + non-client during modal move/resize.
pub const RDW_INVALIDATE: UINT = 0x0001;
pub const RDW_UPDATENOW: UINT = 0x0100;
pub const RDW_ALLCHILDREN: UINT = 0x0080;
pub const RDW_FRAME: UINT = 0x0400;

// WM_NCHITTEST return values.
pub const HTCLIENT: LRESULT = 1;
pub const HTLEFT: LRESULT = 10;
pub const HTRIGHT: LRESULT = 11;
pub const HTTOP: LRESULT = 12;
pub const HTTOPLEFT: LRESULT = 13;
pub const HTTOPRIGHT: LRESULT = 14;
pub const HTBOTTOM: LRESULT = 15;
pub const HTBOTTOMLEFT: LRESULT = 16;
pub const HTBOTTOMRIGHT: LRESULT = 17;

// Extended window styles.
pub const WS_EX_TOPMOST: DWORD = 0x0000_0008;
pub const WS_EX_TOOLWINDOW: DWORD = 0x0000_0080;
pub const WS_EX_LAYERED: DWORD = 0x0008_0000;
pub const WS_EX_NOACTIVATE: DWORD = 0x0800_0000;
pub const WS_EX_APPWINDOW: DWORD = 0x0004_0000;

// ShowWindow commands.
pub const SW_HIDE: INT = 0;
pub const SW_SHOWNORMAL: INT = 1;
pub const SW_SHOWMINIMIZED: INT = 2;
pub const SW_SHOWNOACTIVATE: INT = 4;
pub const SW_SHOW: INT = 5;
pub const SW_RESTORE: INT = 9;

/// Default position/size sentinel for `CreateWindowExW` (`0x80000000`).
pub const CW_USEDEFAULT: INT = 0x8000_0000u32 as INT;

/// `GetSystemMetrics` indices for the virtual screen (all monitors).
pub const SM_XVIRTUALSCREEN: INT = 76;
pub const SM_YVIRTUALSCREEN: INT = 77;
pub const SM_CXVIRTUALSCREEN: INT = 78;
pub const SM_CYVIRTUALSCREEN: INT = 79;

/// Standard arrow cursor resource id, packed as a pseudo-pointer for `LoadCursorW`.
pub const IDC_ARROW: LPCWSTR = 32512 as LPCWSTR;

/// `COLOR_WINDOW`; pass `(COLOR_WINDOW + 1) as HBRUSH` as a class background.
pub const COLOR_WINDOW: INT = 5;

// Desktop Window Manager (DWM) attributes.
pub const DWMWA_NCRENDERING_POLICY: DWORD = 2;
pub const DWMNCRP_DISABLED: DWORD = 1;
pub const DWMWA_WINDOW_CORNER_PREFERENCE: DWORD = 33;
pub const DWMWCP_DONOTROUND: DWORD = 1;
pub const DWMWA_BORDER_COLOR: DWORD = 34;
/// Prefer dark-mode NC/chrome (`DwmSetWindowAttribute`; Win10 1809+).
pub const DWMWA_USE_IMMERSIVE_DARK_MODE: DWORD = 20;
/// Remove the Win11 visible frame border (`DwmSetWindowAttribute` / `DWMWA_BORDER_COLOR`).
pub const DWMWA_COLOR_NONE: DWORD = 0xFFFF_FFFE;
/// Default dark chrome border (`COLORREF` 0x00BBGGRR) — matches [`Theme::dark`] background.
pub const DWM_BORDER_DARK_GRAY: DWORD = 0x001E_1E1E;

// GlobalAlloc flags.
pub const GMEM_FIXED: UINT = 0x0000;
pub const GMEM_MOVEABLE: UINT = 0x0002;
pub const GMEM_ZEROINIT: UINT = 0x0040;
pub const GHND: UINT = GMEM_MOVEABLE | GMEM_ZEROINIT;
pub const GPTR: UINT = GMEM_FIXED | GMEM_ZEROINIT;

// DIB / GDI.
pub const BI_RGB: DWORD = 0;
pub const BI_BITFIELDS: DWORD = 3;
pub const DIB_RGB_COLORS: UINT = 0;
pub const SRCCOPY: DWORD = 0x00CC_0020;

// RegisterHotKey modifier flags (`fsModifiers`).
pub const MOD_ALT: UINT = 0x0001;
pub const MOD_CONTROL: UINT = 0x0002;
pub const MOD_SHIFT: UINT = 0x0004;
pub const MOD_WIN: UINT = 0x0008;
/// Prevents auto-repeat from firing `WM_HOTKEY` repeatedly while held.
pub const MOD_NOREPEAT: UINT = 0x4000;

// Standard clipboard format ids (`CF_*`).
pub const CF_TEXT: UINT = 1;
pub const CF_BITMAP: UINT = 2;
pub const CF_UNICODETEXT: UINT = 13;
pub const CF_DIB: UINT = 8;
pub const CF_DIBV5: UINT = 17;

// Process access / name flags.
pub const PROCESS_QUERY_LIMITED_INFORMATION: DWORD = 0x1000;
pub const PROCESS_NAME_NATIVE: DWORD = 0x0001;

// `MoveFileExW` flags.
pub const MOVEFILE_REPLACE_EXISTING: DWORD = 0x1;
pub const MOVEFILE_WRITE_THROUGH: DWORD = 0x8;

// ---------------------------------------------------------------------------
// kernel32
// ---------------------------------------------------------------------------

#[link(name = "kernel32")]
extern "system" {
    pub fn GetLastError() -> DWORD;
    pub fn SetLastError(dwErrCode: DWORD);
    pub fn GetModuleHandleW(lpModuleName: LPCWSTR) -> HMODULE;
    pub fn GetCurrentProcessId() -> DWORD;
    pub fn GetCurrentThreadId() -> DWORD;
    pub fn GetTickCount() -> DWORD;

    pub fn GlobalAlloc(uFlags: UINT, dwBytes: usize) -> HGLOBAL;
    pub fn GlobalFree(hMem: HGLOBAL) -> HGLOBAL;
    pub fn GlobalLock(hMem: HGLOBAL) -> LPVOID;
    pub fn GlobalUnlock(hMem: HGLOBAL) -> BOOL;
    pub fn GlobalSize(hMem: HGLOBAL) -> usize;

    pub fn OpenProcess(dwDesiredAccess: DWORD, bInheritHandle: BOOL, dwProcessId: DWORD) -> HANDLE;
    pub fn CloseHandle(hObject: HANDLE) -> BOOL;
    pub fn QueryFullProcessImageNameW(
        hProcess: HANDLE,
        dwFlags: DWORD,
        lpExeName: LPWSTR,
        lpdwSize: *mut DWORD,
    ) -> BOOL;

    pub fn MoveFileExW(lpExistingFileName: LPCWSTR, lpNewFileName: LPCWSTR, dwFlags: DWORD)
        -> BOOL;
}

// ---------------------------------------------------------------------------
// user32
// ---------------------------------------------------------------------------

#[link(name = "user32")]
extern "system" {
    // Window class + lifecycle.
    pub fn RegisterClassW(lpWndClass: *const WNDCLASSW) -> ATOM;
    pub fn UnregisterClassW(lpClassName: LPCWSTR, hInstance: HINSTANCE) -> BOOL;
    pub fn CreateWindowExW(
        dwExStyle: DWORD,
        lpClassName: LPCWSTR,
        lpWindowName: LPCWSTR,
        dwStyle: DWORD,
        X: INT,
        Y: INT,
        nWidth: INT,
        nHeight: INT,
        hWndParent: HWND,
        hMenu: HMENU,
        hInstance: HINSTANCE,
        lpParam: LPVOID,
    ) -> HWND;
    pub fn DestroyWindow(hWnd: HWND) -> BOOL;
    pub fn DefWindowProcW(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> LRESULT;

    // Message loop.
    pub fn GetMessageW(
        lpMsg: *mut MSG,
        hWnd: HWND,
        wMsgFilterMin: UINT,
        wMsgFilterMax: UINT,
    ) -> BOOL;
    pub fn TranslateMessage(lpMsg: *const MSG) -> BOOL;
    pub fn DispatchMessageW(lpMsg: *const MSG) -> LRESULT;
    pub fn PostQuitMessage(nExitCode: INT);
    pub fn PostMessageW(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> BOOL;
    pub fn SendMessageW(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> LRESULT;

    // Visibility / layout.
    pub fn ShowWindow(hWnd: HWND, nCmdShow: INT) -> BOOL;
    pub fn IsWindowVisible(hWnd: HWND) -> BOOL;
    pub fn UpdateWindow(hWnd: HWND) -> BOOL;
    pub fn GetClientRect(hWnd: HWND, lpRect: *mut RECT) -> BOOL;
    pub fn GetWindowRect(hWnd: HWND, lpRect: *mut RECT) -> BOOL;
    pub fn GetSystemMetrics(nIndex: INT) -> INT;
    pub fn IsIconic(hWnd: HWND) -> BOOL;
    pub fn AdjustWindowRectEx(
        lpRect: *mut RECT,
        dwStyle: DWORD,
        bMenu: BOOL,
        dwExStyle: DWORD,
    ) -> BOOL;
    pub fn SetWindowPos(
        hWnd: HWND,
        hWndInsertAfter: HWND,
        X: INT,
        Y: INT,
        cx: INT,
        cy: INT,
        uFlags: UINT,
    ) -> BOOL;
    pub fn GetWindowLongPtrW(hWnd: HWND, nIndex: i32) -> isize;
    pub fn SetWindowLongPtrW(hWnd: HWND, nIndex: i32, dwNewLong: isize) -> isize;
    pub fn InvalidateRect(hWnd: HWND, lpRect: *const RECT, bErase: BOOL) -> BOOL;
    pub fn RedrawWindow(
        hWnd: HWND,
        lprcUpdate: *const RECT,
        hrgnUpdate: isize,
        flags: UINT,
    ) -> BOOL;

    // Device contexts (GetDC/ReleaseDC live in user32, not gdi32).
    pub fn GetDC(hWnd: HWND) -> HDC;
    pub fn ReleaseDC(hWnd: HWND, hDC: HDC) -> INT;

    // Painting.
    pub fn BeginPaint(hWnd: HWND, lpPaint: *mut PAINTSTRUCT) -> HDC;
    pub fn EndPaint(hWnd: HWND, lpPaint: *const PAINTSTRUCT) -> BOOL;

    // Resources.
    pub fn LoadCursorW(hInstance: HINSTANCE, lpCursorName: LPCWSTR) -> HCURSOR;

    // Timers.
    pub fn SetTimer(
        hWnd: HWND,
        nIDEvent: UINT_PTR,
        uElapse: UINT,
        lpTimerFunc: TIMERPROC,
    ) -> UINT_PTR;
    pub fn KillTimer(hWnd: HWND, uIDEvent: UINT_PTR) -> BOOL;

    // Global hotkeys (Task 11).
    pub fn RegisterHotKey(hWnd: HWND, id: INT, fsModifiers: UINT, vk: UINT) -> BOOL;
    pub fn UnregisterHotKey(hWnd: HWND, id: INT) -> BOOL;

    // Clipboard.
    pub fn OpenClipboard(hWndNewOwner: HWND) -> BOOL;
    pub fn CloseClipboard() -> BOOL;
    pub fn EmptyClipboard() -> BOOL;
    pub fn GetClipboardData(uFormat: UINT) -> HANDLE;
    pub fn SetClipboardData(uFormat: UINT, hMem: HANDLE) -> HANDLE;
    pub fn EnumClipboardFormats(format: UINT) -> UINT;
    pub fn RegisterClipboardFormatW(lpszFormat: LPCWSTR) -> UINT;
    pub fn AddClipboardFormatListener(hwnd: HWND) -> BOOL;
    pub fn RemoveClipboardFormatListener(hwnd: HWND) -> BOOL;
    pub fn GetClipboardSequenceNumber() -> DWORD;

    // Foreground window / process attribution.
    pub fn GetForegroundWindow() -> HWND;
    pub fn GetWindowThreadProcessId(hWnd: HWND, lpdwProcessId: *mut DWORD) -> DWORD;

    pub fn GetKeyState(nVirtKey: i32) -> i16;
    pub fn ReleaseCapture() -> BOOL;
    pub fn ScreenToClient(hWnd: HWND, lpPoint: *mut POINT) -> BOOL;
    pub fn GetCursorPos(lpPoint: *mut POINT) -> BOOL;
    pub fn SetFocus(hWnd: HWND) -> HWND;
    pub fn SetForegroundWindow(hWnd: HWND) -> BOOL;
    pub fn BringWindowToTop(hWnd: HWND) -> BOOL;

    // Menus (system tray context menu — Task 10).
    pub fn CreatePopupMenu() -> HMENU;
    pub fn AppendMenuW(hMenu: HMENU, uFlags: UINT, uIDNewItem: usize, lpNewItem: LPCWSTR) -> BOOL;
    pub fn TrackPopupMenu(
        hMenu: HMENU,
        uFlags: UINT,
        x: INT,
        y: INT,
        nReserved: INT,
        hWnd: HWND,
        prcRect: *const RECT,
    ) -> BOOL;
    pub fn DestroyMenu(hMenu: HMENU) -> BOOL;

    // Icons (tray + window).
    pub fn LoadImageW(
        hInst: HINSTANCE,
        name: LPCWSTR,
        uType: UINT,
        cxDesired: INT,
        cyDesired: INT,
        fuLoad: UINT,
    ) -> HANDLE;
    pub fn LoadIconW(hInstance: HINSTANCE, lpIconName: LPCWSTR) -> HICON;
    pub fn DestroyIcon(hIcon: HICON) -> BOOL;
    pub fn CreateIconFromResourceEx(
        presbits: *mut BYTE,
        dwResSize: DWORD,
        fIcon: BOOL,
        dwVer: DWORD,
        cxDesired: INT,
        cyDesired: INT,
        uFlags: UINT,
    ) -> HICON;
}

// ---------------------------------------------------------------------------
// gdi32
// ---------------------------------------------------------------------------

pub const FW_NORMAL: LONG = 400;
pub const DEFAULT_CHARSET: DWORD = 1;
pub const OUT_TT_PRECIS: DWORD = 4;
pub const CLIP_DEFAULT_PRECIS: DWORD = 0;
pub const CLEARTYPE_QUALITY: DWORD = 5;
pub const ANTIALIASED_QUALITY: DWORD = 4;
pub const DEFAULT_PITCH: DWORD = 0;
pub const FF_DONTCARE: DWORD = 0;
pub const DEFAULT_PITCH_AND_FAMILY: DWORD = DEFAULT_PITCH | FF_DONTCARE;

pub const GGO_METRICS: UINT = 0;
pub const GGO_BITMAP: UINT = 1;
pub const GGO_GRAY8_BITMAP: UINT = 6;

// `SetTextAlign` flags. `TA_BASELINE` makes `TextOutW`'s `y` the glyph baseline
// row (instead of the default `TA_TOP` cell-top), which lets the rasterizer
// place glyphs at a known baseline without GGO origin math.
pub const TA_LEFT: UINT = 0;
pub const TA_TOP: UINT = 0;
pub const TA_BASELINE: UINT = 24;
pub const TA_NOUPDATECP: UINT = 0;

pub const FR_PRIVATE: DWORD = 0x10;

#[link(name = "gdi32")]
extern "system" {
    pub fn CreateDIBSection(
        hdc: HDC,
        pbmi: *const BITMAPINFO,
        usage: UINT,
        ppvBits: *mut LPVOID,
        hSection: HANDLE,
        offset: DWORD,
    ) -> HBITMAP;
    pub fn StretchDIBits(
        hdc: HDC,
        xDest: INT,
        yDest: INT,
        DestWidth: INT,
        DestHeight: INT,
        xSrc: INT,
        ySrc: INT,
        SrcWidth: INT,
        SrcHeight: INT,
        lpBits: LPCVOID,
        lpbmi: *const BITMAPINFO,
        iUsage: UINT,
        rop: DWORD,
    ) -> INT;
    pub fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
    pub fn CreateCompatibleDC(hdc: HDC) -> HDC;
    pub fn DeleteDC(hdc: HDC) -> BOOL;
    pub fn SelectObject(hdc: HDC, h: HGDIOBJ) -> HGDIOBJ;
    pub fn DeleteObject(ho: HGDIOBJ) -> BOOL;
    pub fn AddFontMemResourceEx(
        pbFont: LPVOID,
        cbFont: DWORD,
        pdv: LPVOID,
        pcFonts: *mut DWORD,
    ) -> HANDLE;
    pub fn RemoveFontMemResourceEx(h: HANDLE) -> BOOL;
    pub fn CreateFontIndirectW(lplf: *const LOGFONTW) -> HFONT;
    pub fn GetTextMetricsW(hdc: HDC, lptm: *mut TEXTMETRICW) -> BOOL;
    pub fn GetTextExtentPoint32W(hdc: HDC, lpString: LPCWSTR, c: INT, lpSize: *mut SIZE) -> BOOL;
    pub fn GetCharABCWidthsFloatW(
        hdc: HDC,
        iFirst: UINT,
        iLast: UINT,
        lpABCF: *mut ABCFLOAT,
    ) -> BOOL;
    pub fn SetBkMode(hdc: HDC, mode: INT) -> INT;
    pub fn SetBkColor(hdc: HDC, color: COLORREF) -> COLORREF;
    pub fn SetTextColor(hdc: HDC, color: COLORREF) -> COLORREF;
    pub fn SetTextAlign(hdc: HDC, align: UINT) -> UINT;
    pub fn TextOutW(hdc: HDC, x: INT, y: INT, lpString: LPCWSTR, c: INT) -> BOOL;
    /// Flushes the batched GDI drawing queue. **Required** before reading the
    /// bit values of a `CreateDIBSection` bitmap that GDI has drawn into.
    pub fn GdiFlush() -> BOOL;
    pub fn GetGlyphOutlineW(
        hdc: HDC,
        uChar: UINT,
        fuFormat: UINT,
        lpgm: *mut GLYPHMETRICS,
        cbBuffer: DWORD,
        lpBuffer: LPVOID,
        lpmat2: *const MAT2,
    ) -> DWORD;
}

// ---------------------------------------------------------------------------
// shell32 (system tray — Task 10)
// ---------------------------------------------------------------------------

#[link(name = "shell32")]
extern "system" {
    pub fn Shell_NotifyIconW(dwMessage: DWORD, lpData: *mut NOTIFYICONDATAW) -> BOOL;
    pub fn ShellExecuteW(
        hwnd: HWND,
        lpOperation: LPCWSTR,
        lpFile: LPCWSTR,
        lpParameters: LPCWSTR,
        lpDirectory: LPCWSTR,
        nShowCmd: INT,
    ) -> HINSTANCE;
}

// ---------------------------------------------------------------------------
// dwmapi (borderless frame / DWM chrome — Task 2)
// ---------------------------------------------------------------------------

#[link(name = "dwmapi")]
extern "system" {
    pub fn DwmExtendFrameIntoClientArea(hWnd: HWND, pMarInset: *const MARGINS) -> HRESULT;
    pub fn DwmSetWindowAttribute(
        hwnd: HWND,
        dwAttribute: DWORD,
        pvAttribute: LPCVOID,
        cbAttribute: DWORD,
    ) -> HRESULT;
}

/// Win32 `HRESULT` success sentinel.
pub type HRESULT = i32;
pub const S_OK: HRESULT = 0;

// ---------------------------------------------------------------------------
// advapi32 (registry — autostart Task 12, system theme Task 8)
// ---------------------------------------------------------------------------

#[link(name = "advapi32")]
extern "system" {
    pub fn RegOpenKeyExW(
        hKey: HKEY,
        lpSubKey: LPCWSTR,
        ulOptions: DWORD,
        samDesired: DWORD,
        phkResult: *mut HKEY,
    ) -> LONG;

    pub fn RegQueryValueExW(
        hKey: HKEY,
        lpValueName: LPCWSTR,
        lpReserved: *mut DWORD,
        lpType: *mut DWORD,
        lpData: LPVOID,
        lpcbData: *mut DWORD,
    ) -> LONG;

    pub fn RegCloseKey(hKey: HKEY) -> LONG;

    pub fn RegCreateKeyExW(
        hKey: HKEY,
        lpSubKey: LPCWSTR,
        Reserved: DWORD,
        lpClass: LPWSTR,
        dwOptions: DWORD,
        samDesired: DWORD,
        lpSecurityAttributes: LPVOID,
        phkResult: *mut HKEY,
        lpdwDisposition: *mut DWORD,
    ) -> LONG;

    pub fn RegSetValueExW(
        hKey: HKEY,
        lpValueName: LPCWSTR,
        Reserved: DWORD,
        dwType: DWORD,
        lpData: *const BYTE,
        cbData: DWORD,
    ) -> LONG;

    pub fn RegDeleteValueW(hKey: HKEY, lpValueName: LPCWSTR) -> LONG;
}
