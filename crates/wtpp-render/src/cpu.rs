//! CPU drawing of the same quads the GPU pipeline draws (see
//! `shaders/cells.wgsl`), for machines without a GPU: a VM or a remote
//! session, where the GPU path runs on an emulated GPU (llvmpipe, WARP) and
//! is many times slower than drawing the cells directly.
//!
//! The target is premultiplied RGBA8, which Slint's software renderer takes
//! as is.

use crate::{Instance, KIND_COLOR, KIND_MASK, KIND_SOLID};

/// A rasterized glyph: coverage only (tinted when drawn), or a colour bitmap
/// (emoji), stored premultiplied.
pub(crate) struct Bitmap {
    width: u32,
    height: u32,
    /// One byte per pixel for a mask, four (premultiplied RGBA) for colour.
    data: Vec<u8>,
}

/// Glyphs kept before the store starts over (bounded memory).
const MAX_GLYPHS: usize = 16384;

/// Glyph bitmaps by slot (the slot goes where the GPU path keeps the atlas
/// position).
#[derive(Default)]
pub(crate) struct Glyphs {
    bitmaps: Vec<Bitmap>,
}

impl Glyphs {
    pub(crate) fn clear(&mut self) {
        self.bitmaps.clear();
    }

    /// Store a glyph from its straight-alpha RGBA pixels; returns its slot,
    /// or `None` once the store is full (start over, like the GPU atlas).
    pub(crate) fn insert(&mut self, width: u32, height: u32, rgba: &[u8], color: bool) -> Option<u32> {
        if self.bitmaps.len() >= MAX_GLYPHS {
            return None;
        }
        let data = if color {
            rgba.as_chunks::<4>().0.iter().flat_map(|&p| premultiply(p)).collect()
        } else {
            rgba.as_chunks::<4>().0.iter().map(|p| p[3]).collect()
        };
        self.bitmaps.push(Bitmap { width, height, data });
        Some((self.bitmaps.len() - 1) as u32)
    }
}

fn premultiply([r, g, b, a]: [u8; 4]) -> [u8; 4] {
    [mul(r, a), mul(g, a), mul(b, a), a]
}

/// `a * b / 255`, rounded.
#[inline]
fn mul(a: u8, b: u8) -> u8 {
    let t = a as u32 * b as u32 + 128;
    ((t + (t >> 8)) >> 8) as u8
}

/// Source-over for premultiplied colours.
#[inline]
fn blend(dst: &mut [u8; 4], src: [u8; 4]) {
    let keep = 255 - src[3];
    for i in 0..4 {
        dst[i] = src[i] + mul(dst[i], keep);
    }
}

fn to_u8(color: [f32; 4]) -> [u8; 4] {
    color.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The pixels a quad covers on one axis: those whose centre lies in
/// `[start, start + len)`, as the GPU rasterizes it.
fn span(start: f32, len: f32, limit: u32) -> (u32, u32) {
    let first = (start - 0.5).ceil().clamp(0.0, limit as f32);
    let end = (start + len - 0.5).ceil().clamp(0.0, limit as f32);
    (first as u32, (end as u32).max(first as u32))
}

/// Draw `instances` over `pixels` (`width`×`height`, premultiplied RGBA),
/// first filled with `clear`.
pub(crate) fn draw(pixels: &mut [[u8; 4]], width: u32, height: u32, clear: [f32; 4], instances: &[Instance], glyphs: &Glyphs) {
    let clear = to_u8(clear);
    let clear = premultiply(clear);
    pixels.fill(clear);
    let stride = width as usize;
    for inst in instances {
        let (x0, x1) = span(inst.pos[0], inst.size[0], width);
        let (y0, y1) = span(inst.pos[1], inst.size[1], height);
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        let color = to_u8(inst.color);
        match inst.kind {
            KIND_SOLID => {
                let src = premultiply(color);
                for y in y0..y1 {
                    let row = &mut pixels[y as usize * stride..][x0 as usize..x1 as usize];
                    if src[3] == 255 {
                        row.fill(src);
                    } else {
                        row.iter_mut().for_each(|p| blend(p, src));
                    }
                }
            }
            KIND_MASK | KIND_COLOR => {
                let Some(bitmap) = glyphs.bitmaps.get(inst.uv_pos[0] as usize) else { continue };
                // Glyphs sit on whole pixels; texel (0, 0) is at the quad's
                // first covered pixel.
                let (gx, gy) = (inst.pos[0].round() as i64, inst.pos[1].round() as i64);
                for y in y0..y1 {
                    let ty = y as i64 - gy;
                    if ty < 0 || ty >= bitmap.height as i64 {
                        continue;
                    }
                    let row = &mut pixels[y as usize * stride..];
                    for x in x0..x1 {
                        let tx = x as i64 - gx;
                        if tx < 0 || tx >= bitmap.width as i64 {
                            continue;
                        }
                        let t = (ty * bitmap.width as i64 + tx) as usize;
                        let src = if inst.kind == KIND_MASK {
                            let a = mul(color[3], bitmap.data[t]);
                            if a == 0 {
                                continue;
                            }
                            [mul(color[0], a), mul(color[1], a), mul(color[2], a), a]
                        } else {
                            let p = &bitmap.data[t * 4..t * 4 + 4];
                            [p[0], p[1], p[2], p[3]]
                        };
                        blend(&mut row[x as usize], src);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> Instance {
        Instance::solid(x, y, w, h, color)
    }

    #[test]
    fn spans_follow_pixel_centres() {
        assert_eq!(span(0.0, 3.0, 10), (0, 3));
        assert_eq!(span(2.4, 1.0, 10), (2, 3));
        assert_eq!(span(2.6, 1.0, 10), (3, 4));
        assert_eq!(span(-5.0, 7.0, 10), (0, 2));
        assert_eq!(span(8.0, 7.0, 10), (8, 10));
        assert_eq!(span(12.0, 2.0, 10), (10, 10));
    }

    #[test]
    fn clears_then_draws_solids_and_blends() {
        let mut px = vec![[0u8; 4]; 4 * 2];
        let insts = [
            solid(1.0, 0.0, 2.0, 1.0, [1.0, 0.0, 0.0, 1.0]),
            solid(0.0, 1.0, 4.0, 1.0, [1.0, 1.0, 1.0, 0.5]),
        ];
        draw(&mut px, 4, 2, [0.0, 0.0, 1.0, 1.0], &insts, &Glyphs::default());
        assert_eq!(px[0], [0, 0, 255, 255]);
        assert_eq!(px[1], [255, 0, 0, 255]);
        assert_eq!(px[3], [0, 0, 255, 255]);
        // 50% white over blue.
        assert_eq!(px[4], [128, 128, 255, 255]);
    }

    #[test]
    fn see_through_frames_stay_premultiplied() {
        let mut px = vec![[9u8; 4]; 2];
        let insts = [solid(0.0, 0.0, 1.0, 1.0, [1.0, 0.0, 0.0, 0.5])];
        draw(&mut px, 2, 1, [0.0; 4], &insts, &Glyphs::default());
        assert_eq!(px[0], [128, 0, 0, 128]);
        assert_eq!(px[1], [0, 0, 0, 0]);
    }

    #[test]
    fn mask_glyphs_are_tinted_and_clipped() {
        let mut glyphs = Glyphs::default();
        // 2×1 glyph: full coverage, then half.
        let slot = glyphs.insert(2, 1, &[255, 255, 255, 255, 255, 255, 255, 128], false).unwrap();
        let mut px = vec![[0u8; 4]; 3];
        let glyph = Instance {
            pos: [1.0, 0.0],
            size: [2.0, 1.0],
            uv_pos: [slot as f32, 0.0],
            color: [0.0, 1.0, 0.0, 1.0],
            kind: KIND_MASK,
        };
        draw(&mut px, 3, 1, [0.0, 0.0, 0.0, 1.0], &[glyph], &glyphs);
        assert_eq!(px[0], [0, 0, 0, 255]);
        assert_eq!(px[1], [0, 255, 0, 255]);
        assert_eq!(px[2], [0, 128, 0, 255]);
        // Partly off the left edge.
        let mut px = vec![[0u8; 4]; 1];
        draw(&mut px, 1, 1, [0.0; 4], &[Instance { pos: [-1.0, 0.0], ..glyph }], &glyphs);
        assert_eq!(px[0], [0, 128, 0, 128]);
    }

    #[test]
    fn colour_glyphs_keep_their_colour() {
        let mut glyphs = Glyphs::default();
        let slot = glyphs.insert(1, 1, &[200, 100, 0, 255], true).unwrap();
        let mut px = vec![[0u8; 4]; 1];
        let glyph = Instance { pos: [0.0, 0.0], size: [1.0, 1.0], uv_pos: [slot as f32, 0.0], color: [1.0; 4], kind: KIND_COLOR };
        draw(&mut px, 1, 1, [0.0, 0.0, 0.0, 1.0], &[glyph], &glyphs);
        assert_eq!(px[0], [200, 100, 0, 255]);
    }
}
