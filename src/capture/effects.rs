//! Post-capture image effects (border, drop shadow), applied before output.

use crate::gfx::bitmap::Bitmap;

const BORDER: u32 = 1;
const BORDER_BGRA: [u8; 4] = [0x50, 0x50, 0x50, 0xFF];
const SHADOW_BLUR: i32 = 14; // soft edge radius
const SHADOW_OFFSET: i32 = 5; // down/right offset
const SHADOW_ALPHA: f32 = 0.45;

pub fn apply(bmp: Bitmap, fx: u32) -> Bitmap {
    let mut b = bmp;
    if fx & super::FX_BORDER != 0 {
        b = border(&b);
    }
    if fx & super::FX_SHADOW != 0 {
        b = shadow(&b);
    }
    b
}

fn border(src: &Bitmap) -> Bitmap {
    let (w, h) = (src.width + 2 * BORDER, src.height + 2 * BORDER);
    let mut out = Bitmap::new(w, h);
    for px in out.bgra.chunks_exact_mut(4) {
        px.copy_from_slice(&BORDER_BGRA);
    }
    blit(&mut out, src, BORDER, BORDER);
    out
}

/// Pads the image and draws a soft, offset black shadow behind it. The
/// shadow of a rectangle under a separable blur is separable, so the alpha
/// is just the product of two 1-D ramps.
fn shadow(src: &Bitmap) -> Bitmap {
    let pad = SHADOW_BLUR + SHADOW_OFFSET;
    let (sw, sh) = (src.width as i32, src.height as i32);
    let (w, h) = (sw + 2 * pad, sh + 2 * pad);
    let mut out = Bitmap::new(w as u32, h as u32);
    // Shadow rect (before blur), in output coords.
    let (l, t) = (pad + SHADOW_OFFSET, pad + SHADOW_OFFSET);
    let (r, b) = (l + sw, t + sh);
    let ramp = |v: i32, lo: i32, hi: i32| -> f32 {
        // Coverage of [lo, hi) by a box of width 2*BLUR centred at v, smoothed.
        let r = SHADOW_BLUR as f32;
        let v = v as f32 + 0.5;
        let a = ((v + r).min(hi as f32) - (v - r).max(lo as f32)).max(0.0) / (2.0 * r);
        let a = a.min(1.0);
        a * a * (3.0 - 2.0 * a)
    };
    let col: Vec<f32> = (0..w).map(|x| ramp(x, l, r)).collect();
    for y in 0..h {
        let ry = ramp(y, t, b) * SHADOW_ALPHA * 255.0;
        if ry <= 0.0 {
            continue;
        }
        let row = &mut out.bgra[(y * w * 4) as usize..((y + 1) * w * 4) as usize];
        for (x, px) in row.chunks_exact_mut(4).enumerate() {
            px[3] = (col[x] * ry) as u8;
        }
    }
    blit(&mut out, src, pad as u32, pad as u32);
    out
}

fn blit(dst: &mut Bitmap, src: &Bitmap, x: u32, y: u32) {
    let ds = dst.width as usize * 4;
    let ss = src.width as usize * 4;
    for row in 0..src.height as usize {
        let d = (y as usize + row) * ds + x as usize * 4;
        dst.bgra[d..d + ss].copy_from_slice(&src.bgra[row * ss..(row + 1) * ss]);
    }
}
