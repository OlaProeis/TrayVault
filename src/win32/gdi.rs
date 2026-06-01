//! GDI DIB-section back-buffer and CPU pixel presentation.
//!
//! TrayVault renders into a 32-bit BGRA buffer (top-down DIB via negative
//! [`BITMAPINFOHEADER::biHeight`]) and blits it to the window client area with
//! [`StretchDIBits`]. UI produces RGBA via `ui::pixmap`; callers must
//! convert to BGRA before passing pixels here.

#![allow(dead_code)] // public accessors used by later tasks (UI, app layer)

use std::mem;
use std::ptr;

use crate::error::Result;
use crate::win32::{ffi, last_error};

use ffi::{
    BeginPaint, CreateDIBSection, DeleteObject, EndPaint, GetClientRect, GetDC, ReleaseDC,
    StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, DWORD, HBITMAP, HDC, HWND,
    PAINTSTRUCT, RECT, RGBQUAD, SRCCOPY,
};

/// CPU back-buffer backed by a GDI DIB section.
pub struct GdiBuffer {
    bitmap: HBITMAP,
    bits: *mut u8,
    width: i32,
    height: i32,
    stride: i32,
    bmi: BITMAPINFO,
}

impl GdiBuffer {
    /// Bytes required for a `width` × `height` 32-bit BGRA image.
    pub fn required_bytes(width: i32, height: i32) -> usize {
        (width.max(0) as usize)
            .saturating_mul(height.max(0) as usize)
            .saturating_mul(4)
    }

    /// Empty placeholder; call [`resize`](Self::resize) before use.
    pub fn empty() -> Self {
        Self {
            bitmap: 0,
            bits: ptr::null_mut(),
            width: 0,
            height: 0,
            stride: 0,
            bmi: Self::make_bmi(0, 0),
        }
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn stride(&self) -> i32 {
        self.stride
    }

    /// Raw BGRA pixel bytes (length = `stride * height`).
    ///
    /// Valid only after a successful [`resize`](Self::resize).
    pub fn bits_mut(&mut self) -> &mut [u8] {
        if self.bits.is_null() || self.width <= 0 || self.height <= 0 {
            return &mut [];
        }
        let len = Self::required_bytes(self.width, self.height);
        // SAFETY: `bits` points at the DIB section allocation of `len` bytes.
        unsafe { std::slice::from_raw_parts_mut(self.bits, len) }
    }

    /// (Re)allocate the DIB section for `width` × `height` client pixels.
    pub fn resize(&mut self, width: i32, height: i32) -> Result<()> {
        if width <= 0 || height <= 0 {
            self.destroy();
            return Ok(());
        }

        if width == self.width && height == self.height && !self.bits.is_null() {
            return Ok(());
        }

        self.destroy();
        self.width = width;
        self.height = height;
        self.stride = width * 4;
        self.bmi = Self::make_bmi(width, height);

        let mut bits: ffi::LPVOID = ptr::null_mut();
        // SAFETY: `bmi` describes a valid 32-bpp top-down DIB; `ppvBits` receives
        // the CPU-accessible pointer. Passing null `HDC` is documented as OK.
        let bitmap = unsafe { CreateDIBSection(0, &self.bmi, DIB_RGB_COLORS, &mut bits, 0, 0) };
        if bitmap == 0 {
            return Err(last_error("CreateDIBSection"));
        }

        self.bitmap = bitmap;
        self.bits = bits.cast();
        Ok(())
    }

    /// Fill the internal buffer with a solid BGRA color.
    pub fn fill_solid(&mut self, blue: u8, green: u8, red: u8, alpha: u8) {
        for px in self.bits_mut().chunks_exact_mut(4) {
            px[0] = blue;
            px[1] = green;
            px[2] = red;
            px[3] = alpha;
        }
    }

    /// Blit `pixels` (`src_width` × `src_height`, 32-bpp BGRA, top-down) to `hdc`.
    pub fn present(&self, hdc: HDC, pixels: &[u8], src_width: i32, src_height: i32) -> Result<()> {
        let needed = Self::required_bytes(src_width, src_height);
        if pixels.len() < needed {
            return Err(crate::error::ClipError::Other(format!(
                "present: expected at least {needed} bytes, got {}",
                pixels.len()
            )));
        }

        let dest_w = self.width.max(1);
        let dest_h = self.height.max(1);

        // SAFETY: `pixels` holds a valid top-down BGRA image; `bmi` matches.
        let rows = unsafe {
            StretchDIBits(
                hdc,
                0,
                0,
                dest_w,
                dest_h,
                0,
                0,
                src_width,
                src_height,
                pixels.as_ptr().cast::<std::ffi::c_void>(),
                &self.bmi,
                DIB_RGB_COLORS,
                SRCCOPY,
            )
        };
        if rows == 0 {
            return Err(last_error("StretchDIBits"));
        }
        Ok(())
    }

    /// Blit the internal DIB buffer to `hdc`.
    pub fn present_internal(&self, hdc: HDC) -> Result<()> {
        if self.bits.is_null() || self.width <= 0 || self.height <= 0 {
            return Ok(());
        }
        let len = Self::required_bytes(self.width, self.height);
        // SAFETY: `bits` is the live DIB section allocation of `len` bytes.
        let pixels = unsafe { std::slice::from_raw_parts(self.bits, len) };
        self.present(hdc, pixels, self.width, self.height)
    }

    /// Resize to the current client area of `hwnd`.
    pub fn resize_to_client(&mut self, hwnd: HWND) -> Result<()> {
        let mut rect = RECT::default();
        // SAFETY: `rect` is a valid out-parameter.
        let ok = unsafe { GetClientRect(hwnd, &mut rect) };
        if ok == 0 {
            return Err(last_error("GetClientRect"));
        }
        self.resize(rect.right - rect.left, rect.bottom - rect.top)
    }

    fn make_bmi(width: i32, height: i32) -> BITMAPINFO {
        BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as DWORD,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [RGBQUAD::default(); 1],
        }
    }

    fn destroy(&mut self) {
        if self.bitmap != 0 {
            // SAFETY: `bitmap` was returned by `CreateDIBSection`.
            unsafe {
                DeleteObject(self.bitmap);
            }
            self.bitmap = 0;
        }
        self.bits = ptr::null_mut();
        self.width = 0;
        self.height = 0;
        self.stride = 0;
    }
}

impl Drop for GdiBuffer {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// Run `GetDC` / `ReleaseDC` around `f` (for live resize/move outside `WM_PAINT`).
pub fn with_dc<F>(hwnd: HWND, f: F) -> Result<()>
where
    F: FnOnce(HDC) -> Result<()>,
{
    // SAFETY: paired GetDC/ReleaseDC for `hwnd`.
    let hdc = unsafe { GetDC(hwnd) };
    if hdc == 0 {
        return Err(last_error("GetDC"));
    }
    let result = f(hdc);
    // SAFETY: releases the DC from GetDC above.
    let released = unsafe { ReleaseDC(hwnd, hdc) };
    if released == 0 {
        return Err(last_error("ReleaseDC"));
    }
    result
}

/// Run `BeginPaint` / `EndPaint` around `f`, returning any error from `f`.
pub fn with_paint<F>(hwnd: HWND, f: F) -> Result<()>
where
    F: FnOnce(HDC) -> Result<()>,
{
    let mut ps = PAINTSTRUCT {
        hdc: 0,
        fErase: 0,
        rcPaint: RECT::default(),
        fRestore: 0,
        fIncUpdate: 0,
        rgbReserved: [0; 32],
    };
    // SAFETY: `ps` is a valid out-parameter for `BeginPaint`.
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    if hdc == 0 {
        return Err(last_error("BeginPaint"));
    }
    let result = f(hdc);
    // SAFETY: paired with the preceding `BeginPaint` for this `hwnd`.
    let ok = unsafe { EndPaint(hwnd, &ps) };
    if ok == 0 {
        return Err(last_error("EndPaint"));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::GdiBuffer;

    #[test]
    fn required_bytes_matches_bgra_stride() {
        assert_eq!(GdiBuffer::required_bytes(640, 480), 640 * 480 * 4);
        assert_eq!(GdiBuffer::required_bytes(0, 100), 0);
    }
}
