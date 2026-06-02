//! GDI-based glyph rasterization for the bundled Roboto UI font.
//!
//! Task 16 (Option B): load Roboto via `AddFontMemResourceEx`, measure with
//! `GetTextMetricsW` / `GetTextExtentPoint32W` / `GetCharABCWidthsFloatW`, then
//! rasterize each glyph baseline-aligned (`TextOutW` + `TA_BASELINE`) onto an
//! offscreen 32-bpp DIB and trim to its inked bounding box. `GdiFlush` is called
//! before the DIB bits are read (GDI batches drawing into a `CreateDIBSection`
//! bitmap). Supports arbitrary Unicode via the OS shaper.

use std::collections::HashMap;
use std::mem;
use std::ptr;
use std::sync::{Mutex, OnceLock};

use crate::error::{ClipError, Result};
use crate::win32::{ffi, last_error, wide};

/// Bundled UI font (Roboto Regular, Apache-2.0).
const FONT_BYTES: &[u8] = include_bytes!("../../assets/Roboto-Regular.ttf");
const FONT_FACE: &str = "Roboto";
const FALLBACK_FONT_FACE: &str = "Segoe UI";

/// Metrics + alpha8 bitmap compatible with the old fontdue layout in `ui::text`.
#[derive(Clone, Debug)]
pub struct RasterizedGlyph {
    pub width: usize,
    pub height: usize,
    pub advance_width: f32,
    pub left: f32,
    pub top: f32,
    pub pixels: Vec<u8>,
}

/// Process-wide GDI font rasterizer (main thread; UI paint path).
struct UiFontRasterizer {
    mem_dc: ffi::HDC,
    font_resource: ffi::HANDLE,
    fonts: HashMap<u32, ffi::HFONT>,
    fallback_fonts: HashMap<u32, ffi::HFONT>,
}

impl UiFontRasterizer {
    fn new() -> Result<Self> {
        // SAFETY: null HDC requests a screen-compatible memory DC.
        let mem_dc = unsafe { ffi::CreateCompatibleDC(0) };
        if mem_dc == 0 {
            return Err(last_error("CreateCompatibleDC"));
        }

        let mut num_fonts: ffi::DWORD = 0;
        // SAFETY: `FONT_BYTES` is a valid in-memory TTF for the process lifetime.
        let font_resource = unsafe {
            ffi::AddFontMemResourceEx(
                FONT_BYTES.as_ptr().cast::<std::ffi::c_void>().cast_mut(),
                FONT_BYTES.len() as ffi::DWORD,
                ptr::null_mut(),
                &mut num_fonts,
            )
        };
        if font_resource == 0 {
            unsafe {
                ffi::DeleteDC(mem_dc);
            }
            return Err(last_error("AddFontMemResourceEx"));
        }

        Ok(Self {
            mem_dc,
            font_resource,
            fonts: HashMap::new(),
            fallback_fonts: HashMap::new(),
        })
    }

    fn rasterize(&mut self, ch: char, size_px: f32) -> Result<RasterizedGlyph> {
        let size_key = size_px.to_bits();
        if let std::collections::hash_map::Entry::Vacant(e) = self.fonts.entry(size_key) {
            let hfont = create_ui_font(size_px)?;
            e.insert(hfont);
        }

        let hfont = self.fonts[&size_key];
        // SAFETY: `mem_dc` is valid; `hfont` was created by GDI.
        let prev_font = unsafe { ffi::SelectObject(self.mem_dc, hfont) };
        if prev_font == 0 {
            return Err(last_error("SelectObject(font)"));
        }

        let primary_ok = glyph_in_font(self.mem_dc, ch);
        let result = rasterize_on_dc(self.mem_dc, ch);
        // SAFETY: restore the previous GDI font.
        unsafe {
            ffi::SelectObject(self.mem_dc, prev_font);
        }
        if primary_ok
            && result
                .as_ref()
                .is_ok_and(|g| g.pixels.iter().any(|&b| b > 0))
        {
            return result;
        }

        let size_key = size_px.to_bits();
        if let std::collections::hash_map::Entry::Vacant(e) = self.fallback_fonts.entry(size_key) {
            let hfont = create_font(FALLBACK_FONT_FACE, size_px)?;
            e.insert(hfont);
        }
        let fallback = self.fallback_fonts[&size_key];
        let prev = unsafe { ffi::SelectObject(self.mem_dc, fallback) };
        if prev == 0 {
            return result;
        }
        let fallback_result = rasterize_on_dc(self.mem_dc, ch);
        unsafe {
            ffi::SelectObject(self.mem_dc, prev);
        }
        fallback_result
    }
}

impl Drop for UiFontRasterizer {
    fn drop(&mut self) {
        for hfont in self.fonts.values().chain(self.fallback_fonts.values()) {
            if *hfont != 0 {
                // SAFETY: each handle came from `CreateFontIndirectW`.
                unsafe {
                    ffi::DeleteObject(*hfont);
                }
            }
        }
        if self.font_resource != 0 {
            // SAFETY: paired with `AddFontMemResourceEx`.
            unsafe {
                ffi::RemoveFontMemResourceEx(self.font_resource);
            }
        }
        if self.mem_dc != 0 {
            // SAFETY: paired with `CreateCompatibleDC`.
            unsafe {
                ffi::DeleteDC(self.mem_dc);
            }
        }
    }
}

static UI_RASTERIZER: OnceLock<Mutex<UiFontRasterizer>> = OnceLock::new();

fn with_rasterizer<F, T>(f: F) -> Result<T>
where
    F: FnOnce(&mut UiFontRasterizer) -> Result<T>,
{
    let lock = UI_RASTERIZER.get_or_init(|| {
        Mutex::new(
            UiFontRasterizer::new()
                .expect("bundled Roboto-Regular.ttf must load via AddFontMemResourceEx"),
        )
    });
    let mut guard = lock
        .lock()
        .map_err(|_| ClipError::Other("glyph rasterizer mutex poisoned".into()))?;
    f(&mut guard)
}

/// Rasterize one glyph at `size_px` using the bundled Roboto face.
pub fn rasterize_glyph(ch: char, size_px: f32) -> Result<RasterizedGlyph> {
    if ch == '\u{2026}' {
        return compose_ellipsis(size_px);
    }
    with_rasterizer(|r| r.rasterize(ch, size_px))
}

fn compose_ellipsis(size_px: f32) -> Result<RasterizedGlyph> {
    let dot_r = (size_px * 0.09).max(1.0);
    let step = (size_px * 0.24).max(2.0);
    let advance_width = step * 3.0;
    let width = advance_width.ceil() as usize;
    let height = (size_px * 0.45).ceil().max(1.0) as usize;
    let top = -(size_px * 0.32);
    let mut pixels = vec![0u8; width * height];
    let cy = (height as f32 * 0.62).round() as i32;
    for i in 0..3 {
        let cx = ((i as f32 + 0.5) * step).round() as i32;
        stamp_disc(&mut pixels, width, height, cx, cy, dot_r, 220);
    }
    Ok(RasterizedGlyph {
        width,
        height,
        advance_width,
        left: 0.0,
        top,
        pixels,
    })
}

fn stamp_disc(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    cx: i32,
    cy: i32,
    radius: f32,
    alpha: u8,
) {
    let r = radius.ceil() as i32;
    let r2 = radius * radius;
    for dy in -r..=r {
        for dx in -r..=r {
            if (dx * dx + dy * dy) as f32 > r2 {
                continue;
            }
            let x = cx + dx;
            let y = cy + dy;
            if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                continue;
            }
            let idx = y as usize * width + x as usize;
            pixels[idx] = pixels[idx].max(alpha);
        }
    }
}

fn create_ui_font(size_px: f32) -> Result<ffi::HFONT> {
    create_font(FONT_FACE, size_px)
}

fn create_font(face: &str, size_px: f32) -> Result<ffi::HFONT> {
    let face = wide(face);
    let mut lf = ffi::LOGFONTW {
        lfHeight: -(size_px.round().max(1.0) as ffi::LONG),
        lfWidth: 0,
        lfEscapement: 0,
        lfOrientation: 0,
        lfWeight: ffi::FW_NORMAL,
        lfItalic: 0,
        lfUnderline: 0,
        lfStrikeOut: 0,
        lfCharSet: ffi::DEFAULT_CHARSET,
        lfOutPrecision: ffi::OUT_TT_PRECIS,
        lfClipPrecision: ffi::CLIP_DEFAULT_PRECIS,
        lfQuality: ffi::ANTIALIASED_QUALITY,
        lfPitchAndFamily: ffi::DEFAULT_PITCH_AND_FAMILY,
        lfFaceName: [0; ffi::LF_FACESIZE],
    };
    let copy_len = face.len().min(ffi::LF_FACESIZE);
    lf.lfFaceName[..copy_len].copy_from_slice(&face[..copy_len]);

    // SAFETY: `lf` is a valid LOGFONTW; face name is NUL-terminated UTF-16.
    let hfont = unsafe { ffi::CreateFontIndirectW(&lf) };
    if hfont == 0 {
        Err(last_error("CreateFontIndirectW"))
    } else {
        Ok(hfont)
    }
}

/// True when the font selected into `hdc` has a real cmap entry for `ch` (not
/// `.notdef`). GDI would otherwise draw a tofu box that still has ink, which
/// blocked the empty-ink Segoe fallback (e.g. U+2011 non-breaking hyphen).
fn glyph_in_font(hdc: ffi::HDC, ch: char) -> bool {
    let text = wide(&ch.to_string());
    let char_count = ch.len_utf16() as ffi::INT;
    let mut index = ffi::GLYPH_INDEX_UNAVAILABLE;
    // SAFETY: `text` is NUL-terminated UTF-16; `index` is a single out-slot.
    let ret = unsafe {
        ffi::GetGlyphIndicesW(
            hdc,
            text.as_ptr(),
            char_count,
            &mut index,
            ffi::GGI_MARK_UNAVAIL,
        )
    };
    if ret == ffi::GDI_ERROR {
        return false;
    }
    index != ffi::GLYPH_INDEX_UNAVAILABLE
}

fn glyph_advance(hdc: ffi::HDC, ch: char, cell_inc: f32) -> f32 {
    let text = wide(&ch.to_string());
    let char_count = (text.len() - 1) as ffi::INT;
    let mut size = ffi::SIZE::default();
    // SAFETY: extent matches what GDI uses for string positioning.
    if unsafe { ffi::GetTextExtentPoint32W(hdc, text.as_ptr(), char_count, &mut size) } != 0 {
        return size.cx as f32;
    }

    let code = ch as u32;
    let mut abc = ffi::ABCFLOAT::default();
    if unsafe { ffi::GetCharABCWidthsFloatW(hdc, code, code, &mut abc) } != 0 {
        return abc.abcfA + abc.abcfB + abc.abcfC;
    }
    cell_inc
}

/// Rasterize `ch` by drawing it baseline-aligned into an offscreen 32-bpp DIB,
/// then trimming to the inked bounding box.
///
/// `TextOutW` with `TA_BASELINE` puts the glyph baseline on a known row, so we
/// avoid `GGO` glyph-origin math entirely. A fixed margin around the
/// text-metrics cell keeps anti-aliased edges and the occasional glyph that
/// draws past the ascent/descent from being clipped. Reported `left`/`top` are
/// offsets from the pen origin / baseline (y grows downward), matching the
/// former fontdue layout that `ui::text` expects.
fn rasterize_on_dc(hdc: ffi::HDC, ch: char) -> Result<RasterizedGlyph> {
    let mut tm = ffi::TEXTMETRICW::default();
    // SAFETY: `hdc` has a UI font selected; `tm` is a valid out-parameter.
    if unsafe { ffi::GetTextMetricsW(hdc, &mut tm) } == 0 {
        return Err(last_error("GetTextMetricsW"));
    }

    let advance_width = glyph_advance(hdc, ch, tm.tmAveCharWidth as f32);

    // ABC bearings tell us how far ink can overhang the pen origin on each side.
    let code = ch as u32;
    let mut abc = ffi::ABCFLOAT::default();
    let has_abc = unsafe { ffi::GetCharABCWidthsFloatW(hdc, code, code, &mut abc) } != 0;
    let overhang_left = if has_abc {
        (-abc.abcfA).max(0.0).ceil() as i32
    } else {
        0
    };
    let overhang_right = if has_abc {
        (-abc.abcfC).max(0.0).ceil() as i32
    } else {
        0
    };

    let text = wide(&ch.to_string());
    let char_count = (text.len() - 1) as ffi::INT;
    let mut size = ffi::SIZE::default();
    // SAFETY: `text` is NUL-terminated UTF-16; `size` is a valid out-parameter.
    if unsafe { ffi::GetTextExtentPoint32W(hdc, text.as_ptr(), char_count, &mut size) } == 0 {
        return Err(last_error("GetTextExtentPoint32W"));
    }

    // Anti-aliased ink can bleed ~1px past the cell; the margin also catches
    // glyphs that reach slightly above the ascent or below the descent.
    const MARGIN: i32 = 2;
    let pen_x = overhang_left + MARGIN;
    let baseline_row = tm.tmAscent + MARGIN;
    let width = (size.cx.max(1) + overhang_left + overhang_right + 2 * MARGIN).max(1) as usize;
    let height = (tm.tmHeight.max(1) + 2 * MARGIN).max(1) as usize;

    let raw = draw_text_cell(hdc, &text, char_count, pen_x, baseline_row, width, height)?;
    let (pixels, out_w, out_h, trim_left, trim_top) = trim_alpha8(raw, width, height);
    if pixels.is_empty() {
        return Ok(RasterizedGlyph {
            width: 0,
            height: 0,
            advance_width,
            left: 0.0,
            top: 0.0,
            pixels,
        });
    }

    Ok(RasterizedGlyph {
        width: out_w,
        height: out_h,
        advance_width,
        left: (trim_left as i32 - pen_x) as f32,
        top: (trim_top as i32 - baseline_row) as f32,
        pixels,
    })
}

/// Draw `text` baseline-aligned into a fresh `width`x`height` 32-bpp DIB and
/// return its alpha8 coverage. `GdiFlush` runs before the bits are read because
/// GDI batches drawing into a `CreateDIBSection` bitmap (per its remarks);
/// skipping the flush yields half-written, "shredded" scanlines.
fn draw_text_cell(
    hdc: ffi::HDC,
    text: &[u16],
    char_count: ffi::INT,
    pen_x: i32,
    baseline_row: i32,
    width: usize,
    height: usize,
) -> Result<Vec<u8>> {
    let bmi = make_bmi(width as ffi::LONG, height as ffi::LONG);
    let mut bits: ffi::LPVOID = ptr::null_mut();
    // Null HDC forces a true 32-bpp section regardless of the mono memory DC.
    let hbmp = unsafe { ffi::CreateDIBSection(0, &bmi, ffi::DIB_RGB_COLORS, &mut bits, 0, 0) };
    if hbmp == 0 {
        return Err(last_error("CreateDIBSection(glyph)"));
    }

    let prev_bmp = unsafe { ffi::SelectObject(hdc, hbmp) };
    if prev_bmp == 0 {
        unsafe {
            ffi::DeleteObject(hbmp);
        }
        return Err(last_error("SelectObject(bitmap)"));
    }

    let row_stride = dib_row_stride(width);
    let slice_len = row_stride * height;
    // SAFETY: the section owns `slice_len` bytes; valid until `DeleteObject`.
    let slice = unsafe { std::slice::from_raw_parts_mut(bits.cast::<u8>(), slice_len) };
    slice.fill(0);

    // SAFETY: `hdc` is the rasterizer's private memory DC with the font selected.
    unsafe {
        ffi::SetBkMode(hdc, ffi::OPAQUE);
        ffi::SetBkColor(hdc, 0);
        ffi::SetTextColor(hdc, 0x00FF_FFFF);
        ffi::SetTextAlign(hdc, ffi::TA_LEFT | ffi::TA_BASELINE);
    }

    let ok = unsafe { ffi::TextOutW(hdc, pen_x, baseline_row, text.as_ptr(), char_count) };
    let result = if ok == 0 {
        Err(last_error("TextOutW"))
    } else {
        // SAFETY: required before reading section bits GDI has drawn into.
        unsafe {
            ffi::GdiFlush();
        }
        Ok(bgra_to_alpha8(slice, width, height, row_stride))
    };

    unsafe {
        ffi::SelectObject(hdc, prev_bmp);
        ffi::DeleteObject(hbmp);
    }
    result
}

fn dib_row_stride(width: usize) -> usize {
    width * 4
}

fn bgra_to_alpha8(bgra: &[u8], width: usize, height: usize, row_stride: usize) -> Vec<u8> {
    let mut pixels = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let idx = y * row_stride + x * 4;
            if idx + 2 >= bgra.len() {
                continue;
            }
            let b = u16::from(bgra[idx]);
            let g = u16::from(bgra[idx + 1]);
            let r = u16::from(bgra[idx + 2]);
            pixels[y * width + x] = ((r + g + b) / 3) as u8;
        }
    }
    pixels
}

fn trim_alpha8(
    pixels: Vec<u8>,
    width: usize,
    height: usize,
) -> (Vec<u8>, usize, usize, usize, usize) {
    if width == 0 || height == 0 || pixels.is_empty() {
        return (pixels, width, height, 0, 0);
    }

    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    let mut any = false;

    for y in 0..height {
        for x in 0..width {
            if pixels[y * width + x] > 0 {
                any = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    if !any {
        return (Vec::new(), 0, 0, 0, 0);
    }

    let out_w = max_x - min_x + 1;
    let out_h = max_y - min_y + 1;
    let mut out = vec![0u8; out_w * out_h];
    for y in 0..out_h {
        for x in 0..out_w {
            out[y * out_w + x] = pixels[(min_y + y) * width + (min_x + x)];
        }
    }
    (out, out_w, out_h, min_x, min_y)
}

fn make_bmi(width: ffi::LONG, height: ffi::LONG) -> ffi::BITMAPINFO {
    ffi::BITMAPINFO {
        bmiHeader: ffi::BITMAPINFOHEADER {
            biSize: mem::size_of::<ffi::BITMAPINFOHEADER>() as ffi::DWORD,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: ffi::BI_RGB,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [ffi::RGBQUAD::default(); 1],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterize_ascii_letter() {
        let glyph = rasterize_glyph('a', 14.0).expect("rasterize");
        assert!(glyph.advance_width > 0.0);
        assert!(!glyph.pixels.is_empty());
        assert_eq!(glyph.pixels.len(), glyph.width * glyph.height);
        let nz = glyph.pixels.iter().filter(|&&b| b > 0).count();
        assert!(nz > 20, "expected anti-aliased coverage, got nz={nz}");
    }

    #[test]
    fn rasterize_ellipsis_and_cjk() {
        let ellipsis = rasterize_glyph('\u{2026}', 14.0).expect("ellipsis");
        assert!(ellipsis.advance_width > 0.0);
        assert!(ellipsis.pixels.iter().any(|&b| b > 0));

        let cjk = rasterize_glyph('中', 14.0).expect("cjk");
        assert!(cjk.advance_width > 0.0);
        assert!(cjk.pixels.iter().any(|&b| b > 0));
    }

    /// Roboto lacks U+2011; GDI used to substitute `.notdef` (tofu) with ink,
    /// so the Segoe fallback never ran. Segoe should render a real hyphen.
    #[test]
    fn rasterize_non_breaking_hyphen() {
        let g = rasterize_glyph('\u{2011}', 14.0).expect("nbhyphen");
        assert!(g.advance_width > 0.0);
        assert!(
            g.pixels.iter().any(|&b| b > 0),
            "expected visible hyphen, not empty/tofu"
        );
        let nz = g.pixels.iter().filter(|&&b| b > 0).count();
        assert!(
            nz < 80,
            "tofu .notdef boxes are much denser than a hyphen; got nz={nz}"
        );
    }

    /// Render a glyph to ASCII art (5 luma buckets) for failure diagnostics.
    fn ascii_art(g: &RasterizedGlyph) -> String {
        if g.width == 0 || g.height == 0 {
            return "<empty>".to_string();
        }
        const RAMP: [char; 5] = [' ', '.', ':', '+', '#'];
        let mut out = String::with_capacity((g.width + 1) * g.height);
        for row in 0..g.height {
            for col in 0..g.width {
                let a = usize::from(g.pixels[row * g.width + col]);
                out.push(RAMP[a * (RAMP.len() - 1) / 255]);
            }
            out.push('\n');
        }
        out
    }

    /// Glyphs whose strokes span the full ink height (`l`, `I`, `H` are plain
    /// vertical bars in Roboto) must have at least one inked pixel on **every**
    /// trimmed scanline. A missing interior scanline means the DIB bits were read
    /// before `GdiFlush` finished the `TextOutW` draw — the defect that made all
    /// UI text look "horizontally shredded" with dropped rows.
    #[test]
    fn vertical_strokes_have_no_missing_scanlines() {
        for &ch in &['l', 'I', 'H'] {
            for &size in &[12.0_f32, 14.0, 16.0] {
                let g = rasterize_glyph(ch, size).expect("rasterize");
                assert!(g.width > 0 && g.height > 0, "ch={ch:?} size={size} empty");
                assert_eq!(g.pixels.len(), g.width * g.height);
                for row in 0..g.height {
                    let inked = (0..g.width).any(|col| g.pixels[row * g.width + col] > 0);
                    assert!(
                        inked,
                        "ch={ch:?} size={size}: scanline {row}/{} empty (shredded glyph)\n{}",
                        g.height,
                        ascii_art(&g),
                    );
                }
            }
        }
    }
}
