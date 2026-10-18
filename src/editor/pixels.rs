// Frame Zero - a tiny Windows screenshot and annotation tool
// Copyright (C) 2026 Claude Henchoz
// SPDX-License-Identifier: GPL-3.0-or-later
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! CPU raster operations on the base image: selection masks, fills,
//! effects, extraction and the cut-out band removal. All functions are pure
//! (`&Bitmap` in, new `Bitmap` out) so callers can wrap them in undoable
//! `Command::Base` steps.

use crate::editor::annotation::{Point, Rect};
use crate::gfx::bitmap::Bitmap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SelShape {
    Rectangle,
    Ellipse,
    Polygon,
    Freehand,
}

/// A pixel selection on the base image. `outline` is the closed polygon in
/// document coordinates (rect/ellipse are converted to polygons too).
#[derive(Clone, Debug)]
pub struct PixelSel {
    pub shape: SelShape,
    pub outline: Vec<Point>,
    pub inverted: bool,
}

impl PixelSel {
    pub fn from_rect(shape: SelShape, r: Rect) -> Self {
        let outline = match shape {
            SelShape::Ellipse => {
                let c = r.center();
                (0..96)
                    .map(|i| {
                        let t = i as f32 / 96.0 * core::f32::consts::TAU;
                        Point::new(c.x + t.cos() * r.w * 0.5, c.y + t.sin() * r.h * 0.5)
                    })
                    .collect()
            }
            _ => r.corners().to_vec(),
        };
        Self { shape, outline, inverted: false }
    }

    pub fn bounds(&self) -> Rect {
        Rect::bounding(&self.outline)
    }

    /// Pixel bounds clamped to the image; the whole image when inverted.
    pub fn pixel_bounds(&self, w: u32, h: u32) -> (i32, i32, i32, i32) {
        if self.inverted {
            return (0, 0, w as i32, h as i32);
        }
        let b = self.bounds();
        (
            (b.x.floor() as i32).clamp(0, w as i32),
            (b.y.floor() as i32).clamp(0, h as i32),
            (b.right().ceil() as i32).clamp(0, w as i32),
            (b.bottom().ceil() as i32).clamp(0, h as i32),
        )
    }

    /// 0/255 coverage mask for the full image (pixel centres inside).
    pub fn mask(&self, w: u32, h: u32) -> Vec<u8> {
        let mut m = vec![0u8; (w * h) as usize];
        let n = self.outline.len();
        if n >= 3 {
            let (_, y0, _, y1) = {
                let b = self.bounds();
                (0, (b.y.floor() as i32).max(0), 0, (b.bottom().ceil() as i32).min(h as i32))
            };
            let mut xs: Vec<f32> = Vec::new();
            for y in y0..y1 {
                let yc = y as f32 + 0.5;
                xs.clear();
                for i in 0..n {
                    let a = self.outline[i];
                    let b = self.outline[(i + 1) % n];
                    if (a.y <= yc && b.y > yc) || (b.y <= yc && a.y > yc) {
                        xs.push(a.x + (yc - a.y) / (b.y - a.y) * (b.x - a.x));
                    }
                }
                xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
                for pair in xs.chunks_exact(2) {
                    let xa = ((pair[0] - 0.5).ceil() as i32).max(0);
                    let xb = ((pair[1] - 0.5).floor() as i32).min(w as i32 - 1);
                    if xb >= xa {
                        let row = (y as u32 * w) as usize;
                        m[row + xa as usize..=row + xb as usize].fill(255);
                    }
                }
            }
        }
        if self.inverted {
            m.iter_mut().for_each(|v| *v = 255 - *v);
        }
        m
    }
}

/// Box-average `block`×`block` cells of the region into a small bitmap (one
/// pixel per cell). The renderer scales it back up with nearest-neighbour.
pub fn pixelate_blocks(base: &Bitmap, x0: i32, y0: i32, x1: i32, y1: i32, block: i32) -> Bitmap {
    let bw = ((x1 - x0) + block - 1) / block;
    let bh = ((y1 - y0) + block - 1) / block;
    let mut out = Bitmap::new(bw.max(1) as u32, bh.max(1) as u32);
    let w = base.width as i32;
    for by in 0..bh {
        for bx in 0..bw {
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for y in (y0 + by * block)..(y0 + (by + 1) * block).min(y1) {
                for x in (x0 + bx * block)..(x0 + (bx + 1) * block).min(x1) {
                    let i = ((y * w + x) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += base.bgra[i + c] as u32;
                    }
                    n += 1;
                }
            }
            let o = ((by * bw + bx) * 4) as usize;
            for c in 0..4 {
                out.bgra[o + c] = (acc[c] / n.max(1)) as u8;
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    Blur,
    Pixelate,
    Grayscale,
    Invert,
    Brighten,
    Darken,
}

/// Apply an effect to the masked pixels.
pub fn apply_effect(base: &Bitmap, sel: &PixelSel, fx: Effect) -> Bitmap {
    let (w, h) = (base.width, base.height);
    let mask = sel.mask(w, h);
    let (x0, y0, x1, y1) = sel.pixel_bounds(w, h);
    let mut out = base.clone();
    let processed = match fx {
        Effect::Blur => Some(box_blur(base, x0, y0, x1, y1, 6, 3)),
        Effect::Pixelate => {
            let mut p = base.clone();
            let block = 10;
            let small = pixelate_blocks(base, x0, y0, x1, y1, block);
            for y in y0..y1 {
                for x in x0..x1 {
                    let sx = (x - x0) / block;
                    let sy = (y - y0) / block;
                    let si = ((sy as u32 * small.width + sx as u32) * 4) as usize;
                    let di = ((y as u32 * w + x as u32) * 4) as usize;
                    p.bgra[di..di + 4].copy_from_slice(&small.bgra[si..si + 4]);
                }
            }
            Some(p)
        }
        _ => None,
    };
    for y in y0..y1 {
        for x in x0..x1 {
            let mi = (y as u32 * w + x as u32) as usize;
            if mask[mi] == 0 {
                continue;
            }
            let i = mi * 4;
            match (&processed, fx) {
                (Some(p), _) => out.bgra[i..i + 4].copy_from_slice(&p.bgra[i..i + 4]),
                (None, Effect::Grayscale) => {
                    let l = (base.bgra[i] as u32 * 29 + base.bgra[i + 1] as u32 * 150 + base.bgra[i + 2] as u32 * 77) >> 8;
                    out.bgra[i] = l as u8;
                    out.bgra[i + 1] = l as u8;
                    out.bgra[i + 2] = l as u8;
                }
                (None, Effect::Invert) => {
                    for c in 0..3 {
                        out.bgra[i + c] = 255 - base.bgra[i + c];
                    }
                }
                (None, Effect::Brighten) => {
                    for c in 0..3 {
                        out.bgra[i + c] = (base.bgra[i + c] as u32 + (255 - base.bgra[i + c] as u32) * 3 / 10) as u8;
                    }
                }
                (None, Effect::Darken) => {
                    for c in 0..3 {
                        out.bgra[i + c] = (base.bgra[i + c] as u32 * 7 / 10) as u8;
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Separable box blur (repeated `passes` times ≈ Gaussian) over a region,
/// sampling outside the region so edges blend in.
fn box_blur(base: &Bitmap, x0: i32, y0: i32, x1: i32, y1: i32, r: i32, passes: u32) -> Bitmap {
    let (w, h) = (base.width as i32, base.height as i32);
    let pad = r * passes as i32;
    let (ax, ay, bx, by) = ((x0 - pad).max(0), (y0 - pad).max(0), (x1 + pad).min(w), (y1 + pad).min(h));
    let mut cur = base.clone();
    let mut tmp = base.clone();
    for _ in 0..passes {
        for y in ay..by {
            for x in ax..bx {
                let mut acc = [0u32; 4];
                let mut n = 0;
                for k in (x - r).max(0)..=(x + r).min(w - 1) {
                    let i = ((y * w + k) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += cur.bgra[i + c] as u32;
                    }
                    n += 1;
                }
                let o = ((y * w + x) * 4) as usize;
                for c in 0..4 {
                    tmp.bgra[o + c] = (acc[c] / n) as u8;
                }
            }
        }
        for y in ay..by {
            for x in ax..bx {
                let mut acc = [0u32; 4];
                let mut n = 0;
                for k in (y - r).max(0)..=(y + r).min(h - 1) {
                    let i = ((k * w + x) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += tmp.bgra[i + c] as u32;
                    }
                    n += 1;
                }
                let o = ((y * w + x) * 4) as usize;
                for c in 0..4 {
                    cur.bgra[o + c] = (acc[c] / n) as u8;
                }
            }
        }
    }
    cur
}

/// Clear the selection to transparent, or "auto-fill" it from the
/// surrounding pixels: each hole pixel blends the nearest known pixel to its
/// left, right, top and bottom, weighted by inverse distance (a Coons-patch
/// style interpolation that yields smooth gradients across the hole), then
/// a light blur hides banding.
pub fn fill_selection(base: &Bitmap, sel: &PixelSel, auto: bool) -> Bitmap {
    let (w, h) = (base.width as i32, base.height as i32);
    let mask = sel.mask(base.width, base.height);
    let mut out = base.clone();
    if !auto {
        for (i, m) in mask.iter().enumerate() {
            if *m != 0 {
                out.bgra[i * 4..i * 4 + 4].fill(0);
            }
        }
        return out;
    }
    let (x0, y0, x1, y1) = sel.pixel_bounds(base.width, base.height);
    if x1 <= x0 || y1 <= y0 {
        return out;
    }
    let (bw, bh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    // Per-direction nearest known pixel index (into the full image) and distance.
    let none = (usize::MAX, 0i32);
    let mut acc = vec![[0f32; 5]; bw * bh];
    let known = |x: i32, y: i32| mask[(y * w + x) as usize] == 0;
    let mut add = |acc: &mut Vec<[f32; 5]>, bx: usize, by: usize, src: (usize, i32)| {
        if src.0 == usize::MAX {
            return;
        }
        let wgt = 1.0 / (src.1 as f32).max(1.0);
        let a = &mut acc[by * bw + bx];
        for c in 0..4 {
            a[c] += base.bgra[src.0 * 4 + c] as f32 * wgt;
        }
        a[4] += wgt;
    };
    for y in y0..y1 {
        let by = (y - y0) as usize;
        let mut last = none;
        for x in 0..x1 {
            if known(x, y) {
                last = ((y * w + x) as usize, x);
            } else if x >= x0 && last.0 != usize::MAX {
                add(&mut acc, (x - x0) as usize, by, (last.0, x - last.1));
            }
        }
        last = none;
        for x in (x0..w).rev() {
            if known(x, y) {
                last = ((y * w + x) as usize, x);
            } else if x < x1 && last.0 != usize::MAX {
                add(&mut acc, (x - x0) as usize, by, (last.0, last.1 - x));
            }
        }
    }
    for x in x0..x1 {
        let bx = (x - x0) as usize;
        let mut last = none;
        for y in 0..y1 {
            if known(x, y) {
                last = ((y * w + x) as usize, y);
            } else if y >= y0 && last.0 != usize::MAX {
                add(&mut acc, bx, (y - y0) as usize, (last.0, y - last.1));
            }
        }
        last = none;
        for y in (y0..h).rev() {
            if known(x, y) {
                last = ((y * w + x) as usize, y);
            } else if y < y1 && last.0 != usize::MAX {
                add(&mut acc, bx, (y - y0) as usize, (last.0, last.1 - y));
            }
        }
    }
    for by in 0..bh {
        for bx in 0..bw {
            let (x, y) = (x0 + bx as i32, y0 + by as i32);
            let i = (y * w + x) as usize;
            if mask[i] == 0 {
                continue;
            }
            let a = acc[by * bw + bx];
            if a[4] > 0.0 {
                for c in 0..4 {
                    out.bgra[i * 4 + c] = (a[c] / a[4]).round().clamp(0.0, 255.0) as u8;
                }
            } else {
                out.bgra[i * 4..i * 4 + 4].fill(0); // whole image selected
            }
        }
    }
    let smooth = box_blur(&out, x0, y0, x1, y1, 2, 2);
    for (i, m) in mask.iter().enumerate() {
        if *m != 0 {
            out.bgra[i * 4..i * 4 + 4].copy_from_slice(&smooth.bgra[i * 4..i * 4 + 4]);
        }
    }
    out
}

/// Copy the selected pixels into a tight bitmap (unselected → transparent).
/// Returns the bitmap and its top-left in document pixels.
pub fn extract(base: &Bitmap, sel: &PixelSel) -> Option<(Bitmap, i32, i32)> {
    let (w, h) = (base.width, base.height);
    let mask = sel.mask(w, h);
    let (x0, y0, x1, y1) = sel.pixel_bounds(w, h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut out = Bitmap::new((x1 - x0) as u32, (y1 - y0) as u32);
    for y in y0..y1 {
        for x in x0..x1 {
            let mi = (y as u32 * w + x as u32) as usize;
            if mask[mi] != 0 {
                let o = (((y - y0) as u32 * out.width + (x - x0) as u32) * 4) as usize;
                out.bgra[o..o + 4].copy_from_slice(&base.bgra[mi * 4..mi * 4 + 4]);
            }
        }
    }
    Some((out, x0, y0))
}

pub fn crop(base: &Bitmap, x0: i32, y0: i32, x1: i32, y1: i32) -> Bitmap {
    let (x0, y0) = (x0.max(0), y0.max(0));
    let (x1, y1) = (x1.min(base.width as i32), y1.min(base.height as i32));
    let mut out = Bitmap::new((x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32);
    for y in y0..y1 {
        let s = ((y as u32 * base.width + x0 as u32) * 4) as usize;
        let d = (((y - y0) as u32 * out.width) * 4) as usize;
        let n = ((x1 - x0) * 4) as usize;
        out.bgra[d..d + n].copy_from_slice(&base.bgra[s..s + n]);
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CutEdge {
    Straight,
    Zigzag,
    Wave,
}

/// Remove the band `[a, b)` (rows when `horizontal`, else columns) and join
/// the two halves. Non-straight edges leave a torn seam: the halves meet
/// along a zigzag/wave line with a thin transparent gap and soft shadow.
pub fn cut_band(base: &Bitmap, horizontal: bool, a: i32, b: i32, edge: CutEdge) -> Bitmap {
    let t = if horizontal { transpose(base) } else { base.clone() };
    // Work in "columns removed" space on `t` (rows became columns).
    let (w, h) = (t.width as i32, t.height as i32);
    let (a, b) = (a.clamp(0, w), b.clamp(0, w));
    if b - a < 1 || b - a >= w {
        return base.clone();
    }
    let nw = w - (b - a);
    let mut out = Bitmap::new(nw as u32, h as u32);
    let amp = 6.0f32;
    let period = 18.0f32;
    let seam = |y: i32| -> f32 {
        match edge {
            CutEdge::Straight => 0.0,
            CutEdge::Zigzag => {
                let p = (y as f32 / period).fract();
                (if p < 0.5 { p * 2.0 } else { 2.0 - p * 2.0 }) * 2.0 * amp - amp
            }
            CutEdge::Wave => (y as f32 / period * core::f32::consts::TAU).sin() * amp,
        }
    };
    let gap = if edge == CutEdge::Straight { 0.0 } else { 3.0 };
    for y in 0..h {
        let s = a as f32 + seam(y);
        for x in 0..nw {
            let xf = x as f32 + 0.5;
            let src_x = if xf < s - gap * 0.5 {
                Some(x)
            } else if xf > s + gap * 0.5 {
                Some(x + (b - a))
            } else {
                None
            };
            let o = ((y * nw + x) * 4) as usize;
            if let Some(sx) = src_x {
                let sx = sx.clamp(0, w - 1);
                let i = ((y * w + sx) * 4) as usize;
                out.bgra[o..o + 4].copy_from_slice(&t.bgra[i..i + 4]);
                if gap > 0.0 {
                    // Soft shadow on the right half next to the seam.
                    let d = xf - (s + gap * 0.5);
                    if d >= 0.0 && d < 5.0 {
                        let k = (180.0 + d * 15.0) as u32;
                        for c in 0..3 {
                            out.bgra[o + c] = (out.bgra[o + c] as u32 * k / 255) as u8;
                        }
                    }
                }
            }
        }
    }
    if horizontal { transpose(&out) } else { out }
}

fn transpose(b: &Bitmap) -> Bitmap {
    let mut out = Bitmap::new(b.height, b.width);
    for y in 0..b.height {
        for x in 0..b.width {
            let s = ((y * b.width + x) * 4) as usize;
            let d = ((x * b.height + y) * 4) as usize;
            out.bgra[d..d + 4].copy_from_slice(&b.bgra[s..s + 4]);
        }
    }
    out
}

