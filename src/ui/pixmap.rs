//! Hand-rolled RGBA8 pixel buffer for UI rasterization (replaces tiny-skia `Pixmap`).

/// Normalized RGBA color (0..=1 per channel), matching the former tiny-skia `Color` API.
#[derive(Clone, Copy, Debug)]
pub struct Color {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

/// 8-bit RGBA channels for glyph blending.
#[derive(Clone, Copy, Debug)]
pub struct ColorU8 {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

impl ColorU8 {
    pub fn red(self) -> u8 {
        self.r
    }

    pub fn green(self) -> u8 {
        self.g
    }

    pub fn blue(self) -> u8 {
        self.b
    }

    pub fn alpha(self) -> u8 {
        self.a
    }
}

impl Color {
    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: f32::from(r) / 255.0,
            g: f32::from(g) / 255.0,
            b: f32::from(b) / 255.0,
            a: f32::from(a) / 255.0,
        }
    }

    pub fn to_color_u8(self) -> ColorU8 {
        ColorU8 {
            r: (self.r * 255.0).round().clamp(0.0, 255.0) as u8,
            g: (self.g * 255.0).round().clamp(0.0, 255.0) as u8,
            b: (self.b * 255.0).round().clamp(0.0, 255.0) as u8,
            a: (self.a * 255.0).round().clamp(0.0, 255.0) as u8,
        }
    }
}

/// Top-down RGBA8 pixmap (same layout as the former tiny-skia buffer).
#[derive(Clone, Debug)]
pub struct Pixmap {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Pixmap {
    pub fn new(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let len = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        Some(Self {
            width,
            height,
            pixels: vec![0; len],
        })
    }

    pub fn from_vec(pixels: Vec<u8>, width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        if pixels.len() != expected {
            return None;
        }
        Some(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn data(&self) -> &[u8] {
        &self.pixels
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }

    /// Build from BGRA8 pixels (capture pipeline and blob store format).
    #[allow(dead_code)]
    pub fn from_bgra_vec(pixels: Vec<u8>, width: u32, height: u32) -> Option<Self> {
        Self::from_vec(bgra_to_rgba(&pixels), width, height)
    }
}

/// Solid axis-aligned fill; clips to pixmap bounds; no anti-aliasing.
pub fn fill_rect(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, color: Color) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }

    let c = color.to_color_u8();
    let x0 = x.floor().max(0.0) as u32;
    let y0 = y.floor().max(0.0) as u32;
    let x1 = (x + w).ceil().min(pixmap.width as f32) as u32;
    let y1 = (y + h).ceil().min(pixmap.height as f32) as u32;
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let stride = pixmap.width as usize;
    for py in y0..y1 {
        let row = py as usize * stride * 4;
        for px in x0..x1 {
            let idx = row + px as usize * 4;
            pixmap.pixels[idx] = c.r;
            pixmap.pixels[idx + 1] = c.g;
            pixmap.pixels[idx + 2] = c.b;
            pixmap.pixels[idx + 3] = c.a;
        }
    }
}

/// 1:1 blit of `src` onto `dst` at integer `(dst_x, dst_y)`.
pub fn blit(dst: &mut Pixmap, src: &Pixmap, dst_x: f32, dst_y: f32) {
    let start_x = dst_x.round() as i32;
    let start_y = dst_y.round() as i32;
    let src_w = src.width as i32;
    let src_h = src.height as i32;
    let dst_w = dst.width as i32;
    let dst_h = dst.height as i32;

    for sy in 0..src_h {
        let dy = start_y + sy;
        if dy < 0 || dy >= dst_h {
            continue;
        }
        for sx in 0..src_w {
            let dx = start_x + sx;
            if dx < 0 || dx >= dst_w {
                continue;
            }
            let si = ((sy as u32 * src.width + sx as u32) * 4) as usize;
            let di = ((dy as u32 * dst.width + dx as u32) * 4) as usize;
            dst.pixels[di..di + 4].copy_from_slice(&src.pixels[si..si + 4]);
        }
    }
}

/// Bilinear scale blit (shared with thumbnail downscale; preview uses pre-scaled cache + `blit`).
#[allow(dead_code)]
pub fn blit_scaled(dst: &mut Pixmap, src: &Pixmap, dst_x: f32, dst_y: f32, scale: f32) {
    if scale <= 0.0 {
        return;
    }
    let dst_w = ((src.width as f32) * scale).round().max(1.0) as u32;
    let dst_h = ((src.height as f32) * scale).round().max(1.0) as u32;
    let scaled = scale_bilinear_rgba(src.data(), src.width, src.height, dst_w, dst_h);
    let Some(tmp) = Pixmap::from_vec(scaled, dst_w, dst_h) else {
        return;
    };
    blit(dst, &tmp, dst_x, dst_y);
}

/// Bilinear resize of an RGBA8 buffer (history thumbnails and preview modal).
pub fn scale_bilinear_rgba(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return out;
    }
    if src_w == dst_w && src_h == dst_h {
        out.copy_from_slice(src);
        return out;
    }

    let src_w_f = src_w as f32;
    let src_h_f = src_h as f32;
    let dst_w_f = dst_w as f32;
    let dst_h_f = dst_h as f32;

    for dy in 0..dst_h {
        let sy = ((dy as f32 + 0.5) * src_h_f / dst_h_f - 0.5).clamp(0.0, src_h_f - 1.0);
        let y0 = sy.floor() as u32;
        let y1 = (y0 + 1).min(src_h - 1);
        let fy = sy - y0 as f32;

        for dx in 0..dst_w {
            let sx = ((dx as f32 + 0.5) * src_w_f / dst_w_f - 0.5).clamp(0.0, src_w_f - 1.0);
            let x0 = sx.floor() as u32;
            let x1 = (x0 + 1).min(src_w - 1);
            let fx = sx - x0 as f32;

            let c00 = rgba_sample_f32(src, src_w, x0, y0);
            let c10 = rgba_sample_f32(src, src_w, x1, y0);
            let c01 = rgba_sample_f32(src, src_w, x0, y1);
            let c11 = rgba_sample_f32(src, src_w, x1, y1);

            let di = ((dy * dst_w + dx) * 4) as usize;
            for ch in 0..4 {
                let top = lerp_f32(c00[ch], c10[ch], fx);
                let bot = lerp_f32(c01[ch], c11[ch], fx);
                out[di + ch] = lerp_f32(top, bot, fy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

fn rgba_sample_f32(src: &[u8], src_w: u32, x: u32, y: u32) -> [f32; 4] {
    let i = ((y * src_w + x) * 4) as usize;
    [
        f32::from(src[i]),
        f32::from(src[i + 1]),
        f32::from(src[i + 2]),
        f32::from(src[i + 3]),
    ]
}

fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Nearest-neighbor resize of an RGBA8 buffer (legacy; retained for tests).
#[allow(dead_code)]
pub fn scale_nearest_rgba(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return out;
    }
    for dy in 0..dst_h {
        let sy = (dy as u64 * src_h as u64 / dst_h as u64) as u32;
        for dx in 0..dst_w {
            let sx = (dx as u64 * src_w as u64 / dst_w as u64) as u32;
            let si = ((sy * src_w + sx) * 4) as usize;
            let di = ((dy * dst_w + dx) * 4) as usize;
            out[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    out
}

pub fn rgba_to_color(rgba: [u8; 4]) -> Color {
    Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3])
}

/// Convert a BGRA8 buffer (clipboard / blob store layout) to RGBA8 for [`Pixmap`].
pub fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(bgra.len());
    for px in bgra.chunks_exact(4) {
        rgba.push(px[2]);
        rgba.push(px[1]);
        rgba.push(px[0]);
        rgba.push(px[3]);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Pixmap {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            v.extend_from_slice(&rgba);
        }
        Pixmap::from_vec(v, w, h).expect("pixmap")
    }

    #[test]
    fn bgra_to_rgba_swaps_red_and_blue() {
        let bgra = vec![0, 128, 255, 255];
        assert_eq!(bgra_to_rgba(&bgra), vec![255, 128, 0, 255]);
    }

    #[test]
    fn fill_rect_writes_color_inside_bounds() {
        let mut pm = Pixmap::new(8, 8).expect("pixmap");
        fill_rect(
            &mut pm,
            2.0,
            2.0,
            3.0,
            2.0,
            Color::from_rgba8(10, 20, 30, 255),
        );
        let idx = (2 * 8 + 2) as usize * 4;
        assert_eq!(&pm.data()[idx..idx + 4], &[10, 20, 30, 255]);
        // outside fill stays zero
        assert_eq!(pm.data()[0], 0);
    }

    #[test]
    fn fill_rect_clips_outside_pixmap() {
        let mut pm = Pixmap::new(4, 4).expect("pixmap");
        fill_rect(
            &mut pm,
            -2.0,
            -2.0,
            10.0,
            10.0,
            Color::from_rgba8(1, 2, 3, 4),
        );
        assert!(pm.data().iter().all(|&b| b != 0));
    }

    #[test]
    fn blit_copies_2x2_to_destination() {
        let src = solid(2, 2, [1, 2, 3, 4]);
        let mut dst = Pixmap::new(4, 4).expect("pixmap");
        blit(&mut dst, &src, 1.0, 1.0);
        let idx = (4 + 1) as usize * 4;
        assert_eq!(&dst.data()[idx..idx + 4], &[1, 2, 3, 4]);
    }

    #[test]
    fn blit_scaled_nearest_halves_size() {
        let src = solid(4, 4, [9, 8, 7, 6]);
        let mut dst = Pixmap::new(8, 8).expect("pixmap");
        blit_scaled(&mut dst, &src, 0.0, 0.0, 0.5);
        assert_eq!(dst.data()[0..4], [9, 8, 7, 6]);
    }

    #[test]
    fn scale_nearest_matches_thumb_downscale() {
        let src = solid(100, 50, [255, 0, 0, 255]);
        let scaled = scale_nearest_rgba(src.data(), 100, 50, 40, 20);
        assert_eq!(scaled.len(), 40 * 20 * 4);
    }

    #[test]
    fn scale_bilinear_preserves_solid_color() {
        let src = solid(100, 50, [255, 0, 0, 255]);
        let scaled = scale_bilinear_rgba(src.data(), 100, 50, 40, 20);
        assert_eq!(scaled.len(), 40 * 20 * 4);
        assert!(scaled.chunks_exact(4).all(|px| px == [255, 0, 0, 255]));
    }

    #[test]
    fn scale_bilinear_blends_horizontal_edge() {
        let mut pm = Pixmap::new(2, 1).expect("pixmap");
        pm.data_mut()[0..4].copy_from_slice(&[0, 0, 0, 255]);
        pm.data_mut()[4..8].copy_from_slice(&[255, 255, 255, 255]);
        let scaled = scale_bilinear_rgba(pm.data(), 2, 1, 4, 1);
        assert_eq!(scaled[0], 0);
        assert_eq!(scaled[4], 64);
        assert_eq!(scaled[8], 191);
        assert_eq!(scaled[12], 255);
    }
}
