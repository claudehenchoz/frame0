//! Bundled stamp assets. Each entry is a 32×32 BGRA buffer encoded inline as
//! a tiny vector geometry; we generate the pixels at startup rather than
//! embedding PNGs to avoid pulling decode paths for stamp data.
//!
//! v2 ships four geometric stamps (check / x / dot / star). v2.1 adds a
//! user-imports folder (`%APPDATA%\frame\stamps\*.png`).

use crate::gfx::bitmap::Bitmap;
use crate::gfx::color::Color;

pub const COUNT: usize = 4;
pub const SIZE: u32 = 32;

pub fn label(idx: u8) -> &'static str {
    match idx {
        0 => "✓",
        1 => "✗",
        2 => "●",
        3 => "★",
        _ => "?",
    }
}

/// Render the requested stamp into a fresh BGRA bitmap.
pub fn bitmap(idx: u8) -> Bitmap {
    let mut bm = Bitmap::new(SIZE, SIZE);
    match idx {
        0 => check(&mut bm),
        1 => x_mark(&mut bm),
        2 => dot(&mut bm),
        3 => star(&mut bm),
        _ => dot(&mut bm),
    }
    bm
}

fn put(bm: &mut Bitmap, x: i32, y: i32, c: Color) {
    if x < 0 || y < 0 || (x as u32) >= bm.width || (y as u32) >= bm.height {
        return;
    }
    let i = ((y as u32 * bm.width + x as u32) * 4) as usize;
    bm.bgra[i] = c.b;
    bm.bgra[i + 1] = c.g;
    bm.bgra[i + 2] = c.r;
    bm.bgra[i + 3] = c.a;
}

fn line(bm: &mut Bitmap, x0: f32, y0: f32, x1: f32, y1: f32, c: Color, w: f32) {
    let steps = ((x1 - x0).abs().max((y1 - y0).abs()) as i32).max(1);
    let half = (w * 0.5).max(0.5);
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let cx = x0 + (x1 - x0) * t;
        let cy = y0 + (y1 - y0) * t;
        let r0 = -half as i32;
        let r1 = half as i32;
        for j in r0..=r1 {
            for i in r0..=r1 {
                if (i * i + j * j) as f32 <= half * half {
                    put(bm, (cx + i as f32) as i32, (cy + j as f32) as i32, c);
                }
            }
        }
    }
}

fn check(bm: &mut Bitmap) {
    let c = Color::rgb(0x2E, 0xA8, 0x4F);
    line(bm, 6.0, 16.0, 13.0, 23.0, c, 3.5);
    line(bm, 13.0, 23.0, 26.0, 9.0, c, 3.5);
}

fn x_mark(bm: &mut Bitmap) {
    let c = Color::rgb(0xE5, 0x3E, 0x3E);
    line(bm, 7.0, 7.0, 25.0, 25.0, c, 3.5);
    line(bm, 25.0, 7.0, 7.0, 25.0, c, 3.5);
}

fn dot(bm: &mut Bitmap) {
    let c = Color::rgb(0x2C, 0x7B, 0xE5);
    let cx = 16.0;
    let cy = 16.0;
    let r = 10.0;
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            if dx * dx + dy * dy <= r * r {
                put(bm, x, y, c);
            }
        }
    }
}

fn star(bm: &mut Bitmap) {
    let c = Color::rgb(0xFF, 0xC8, 0x3A);
    // Five-pointed star via simple radial filling.
    let cx = 16.0;
    let cy = 16.0;
    let outer = 14.0;
    let inner = 6.0;
    let pts: Vec<(f32, f32)> = (0..10)
        .map(|i| {
            let theta = (i as f32) * std::f32::consts::PI / 5.0 - std::f32::consts::FRAC_PI_2;
            let r = if i % 2 == 0 { outer } else { inner };
            (cx + theta.cos() * r, cy + theta.sin() * r)
        })
        .collect();
    // Fill via scan-line: for each row, find the leftmost/rightmost x of any
    // polygon edge intersecting that row. Cheap for 32×32.
    for y in 0..SIZE as i32 {
        let yf = y as f32 + 0.5;
        let mut xs: Vec<f32> = Vec::new();
        for i in 0..pts.len() {
            let (x0, y0) = pts[i];
            let (x1, y1) = pts[(i + 1) % pts.len()];
            if (y0 <= yf && y1 > yf) || (y1 <= yf && y0 > yf) {
                let t = (yf - y0) / (y1 - y0);
                xs.push(x0 + (x1 - x0) * t);
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
        let mut i = 0;
        while i + 1 < xs.len() {
            for x in (xs[i] as i32)..=(xs[i + 1] as i32) {
                put(bm, x, y, c);
            }
            i += 2;
        }
    }
}
