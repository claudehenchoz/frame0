use crate::editor::Document;
use crate::editor::annotation::{Annotation, BlurMode, Point, Rect, ShapeKind};
use crate::error::Result;
use crate::gfx::bitmap::Bitmap;
use crate::gfx::color::Color;

/// Software-rasterised flatten path. Supports: arrows (Bresenham + arrowhead
/// triangle), shapes (rectangle/ellipse outline + fill), step markers (filled
/// circle), highlighter (alpha-blended rect), blur(solid) (opaque rect). Text
/// glyph rasterization is currently a placeholder (text appears as a bordered
/// rectangle on export — text content is preserved in `.frame` saves).
pub fn flatten(doc: &Document) -> Result<Bitmap> {
    let mut out = doc.base.clone();
    for ann in &doc.annotations {
        rasterize(&mut out, ann);
    }
    Ok(out)
}

fn rasterize(bm: &mut Bitmap, ann: &Annotation) {
    match ann {
        Annotation::Arrow {
            start,
            end,
            color,
            stroke,
            ..
        } => raster_arrow(bm, *start, *end, *color, *stroke),
        Annotation::Shape {
            kind,
            bounds,
            outline,
            fill,
            stroke,
            ..
        } => raster_shape(bm, *kind, *bounds, *outline, *fill, *stroke),
        Annotation::Text { bounds, color, .. } => {
            // Placeholder rectangle outline (text glyph rasterization deferred to v2).
            raster_rect_outline(bm, *bounds, *color, 1.0);
        }
        Annotation::Step {
            center,
            radius,
            fill,
            ..
        } => raster_filled_circle(bm, center.x, center.y, *radius, *fill),
        Annotation::Highlighter { bounds, color, .. } => raster_filled_rect(bm, *bounds, *color),
        Annotation::Blur {
            bounds,
            mode,
            fill,
            ..
        } => match mode {
            BlurMode::Solid => raster_filled_rect(bm, *bounds, *fill),
            BlurMode::Blur => raster_box_blur(bm, *bounds, 12),
        },
    }
}

fn put_pixel(bm: &mut Bitmap, x: i32, y: i32, c: Color) {
    if x < 0 || y < 0 || (x as u32) >= bm.width || (y as u32) >= bm.height {
        return;
    }
    let i = ((y as u32 * bm.width + x as u32) * 4) as usize;
    blend(&mut bm.bgra[i..i + 4], c);
}

fn blend(dst: &mut [u8], c: Color) {
    let a = c.a as u32;
    let inv = 255 - a;
    dst[0] = ((c.b as u32 * a + dst[0] as u32 * inv) / 255) as u8;
    dst[1] = ((c.g as u32 * a + dst[1] as u32 * inv) / 255) as u8;
    dst[2] = ((c.r as u32 * a + dst[2] as u32 * inv) / 255) as u8;
    dst[3] = 0xFF;
}

fn raster_filled_rect(bm: &mut Bitmap, r: Rect, color: Color) {
    let x0 = (r.x as i32).max(0);
    let y0 = (r.y as i32).max(0);
    let x1 = ((r.x + r.w) as i32).min(bm.width as i32);
    let y1 = ((r.y + r.h) as i32).min(bm.height as i32);
    for y in y0..y1 {
        for x in x0..x1 {
            put_pixel(bm, x, y, color);
        }
    }
}

fn raster_rect_outline(bm: &mut Bitmap, r: Rect, color: Color, stroke: f32) {
    let s = stroke.max(1.0) as i32;
    let x0 = r.x as i32;
    let y0 = r.y as i32;
    let x1 = (r.x + r.w) as i32;
    let y1 = (r.y + r.h) as i32;
    for off in 0..s {
        for x in x0..=x1 {
            put_pixel(bm, x, y0 + off, color);
            put_pixel(bm, x, y1 - off, color);
        }
        for y in y0..=y1 {
            put_pixel(bm, x0 + off, y, color);
            put_pixel(bm, x1 - off, y, color);
        }
    }
}

fn raster_shape(
    bm: &mut Bitmap,
    kind: ShapeKind,
    bounds: Rect,
    outline: Color,
    fill: Color,
    stroke: f32,
) {
    match kind {
        ShapeKind::Rectangle => {
            if fill.a > 0 {
                raster_filled_rect(bm, bounds, fill);
            }
            raster_rect_outline(bm, bounds, outline, stroke);
        }
        ShapeKind::Ellipse => {
            let cx = bounds.x + bounds.w / 2.0;
            let cy = bounds.y + bounds.h / 2.0;
            let rx = bounds.w / 2.0;
            let ry = bounds.h / 2.0;
            if fill.a > 0 {
                let y0 = (cy - ry) as i32;
                let y1 = (cy + ry) as i32;
                for y in y0..=y1 {
                    let dy = (y as f32 + 0.5 - cy) / ry;
                    let inside = 1.0 - dy * dy;
                    if inside <= 0.0 {
                        continue;
                    }
                    let half = rx * inside.sqrt();
                    let x0 = (cx - half) as i32;
                    let x1 = (cx + half) as i32;
                    for x in x0..=x1 {
                        put_pixel(bm, x, y, fill);
                    }
                }
            }
            // Outline: sample on the ring band [r-stroke .. r]
            let s = stroke.max(1.0);
            let r2_outer = 1.0;
            let r2_inner = (1.0 - s / rx.min(ry)).max(0.0);
            let r2_inner = r2_inner * r2_inner;
            let y0 = (cy - ry - s) as i32;
            let y1 = (cy + ry + s) as i32;
            for y in y0..=y1 {
                let dy = (y as f32 + 0.5 - cy) / ry;
                for x in (cx - rx - s) as i32..=(cx + rx + s) as i32 {
                    let dx = (x as f32 + 0.5 - cx) / rx;
                    let r2 = dx * dx + dy * dy;
                    if r2 <= r2_outer && r2 >= r2_inner {
                        put_pixel(bm, x, y, outline);
                    }
                }
            }
        }
    }
}

fn raster_filled_circle(bm: &mut Bitmap, cx: f32, cy: f32, r: f32, color: Color) {
    let y0 = (cy - r) as i32;
    let y1 = (cy + r) as i32;
    let r2 = r * r;
    for y in y0..=y1 {
        let dy = y as f32 + 0.5 - cy;
        let h = r2 - dy * dy;
        if h <= 0.0 {
            continue;
        }
        let half = h.sqrt();
        let x0 = (cx - half) as i32;
        let x1 = (cx + half) as i32;
        for x in x0..=x1 {
            put_pixel(bm, x, y, color);
        }
    }
}

fn raster_arrow(bm: &mut Bitmap, start: Point, end: Point, color: Color, stroke: f32) {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / len;
    let uy = dy / len;
    let head_size = stroke * 4.0 + 6.0;
    let shaft_end = Point::new(end.x - ux * head_size * 0.6, end.y - uy * head_size * 0.6);
    raster_thick_line(bm, start, shaft_end, color, stroke);
    // Head triangle: rasterize via three filled triangles edges
    let ortho_x = -uy;
    let ortho_y = ux;
    let p1 = end;
    let p2 = Point::new(
        end.x - ux * head_size + ortho_x * head_size * 0.5,
        end.y - uy * head_size + ortho_y * head_size * 0.5,
    );
    let p3 = Point::new(
        end.x - ux * head_size - ortho_x * head_size * 0.5,
        end.y - uy * head_size - ortho_y * head_size * 0.5,
    );
    fill_triangle(bm, p1, p2, p3, color);
}

fn raster_thick_line(bm: &mut Bitmap, a: Point, b: Point, color: Color, stroke: f32) {
    let half = (stroke / 2.0).max(0.5);
    let steps = ((b.x - a.x).abs().max((b.y - a.y).abs()) as i32).max(1);
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let cx = a.x + (b.x - a.x) * t;
        let cy = a.y + (b.y - a.y) * t;
        let r0 = -half as i32;
        let r1 = half as i32;
        for j in r0..=r1 {
            for i in r0..=r1 {
                if (i * i + j * j) as f32 <= half * half {
                    put_pixel(bm, (cx + i as f32) as i32, (cy + j as f32) as i32, color);
                }
            }
        }
    }
}

fn fill_triangle(bm: &mut Bitmap, a: Point, b: Point, c: Point, color: Color) {
    let mut pts = [a, b, c];
    pts.sort_by(|p, q| p.y.partial_cmp(&q.y).unwrap_or(core::cmp::Ordering::Equal));
    let [t, m, bot] = pts;
    if (bot.y - t.y).abs() < 0.5 {
        return;
    }
    let total_h = bot.y - t.y;
    for y in (t.y as i32)..=(bot.y as i32) {
        let segment = (y as f32) > m.y || (m.y - t.y).abs() < 0.5;
        let alpha = (y as f32 - t.y) / total_h;
        let beta_h = if segment { bot.y - m.y } else { m.y - t.y };
        let beta = if beta_h.abs() < 0.5 {
            0.0
        } else {
            ((y as f32) - if segment { m.y } else { t.y }) / beta_h
        };
        let ax = t.x + (bot.x - t.x) * alpha;
        let bx = if segment {
            m.x + (bot.x - m.x) * beta
        } else {
            t.x + (m.x - t.x) * beta
        };
        let (lo, hi) = if ax < bx { (ax, bx) } else { (bx, ax) };
        for x in (lo as i32)..=(hi as i32) {
            put_pixel(bm, x, y, color);
        }
    }
}

fn raster_box_blur(bm: &mut Bitmap, bounds: Rect, radius: i32) {
    let x0 = (bounds.x as i32).max(0);
    let y0 = (bounds.y as i32).max(0);
    let x1 = ((bounds.x + bounds.w) as i32).min(bm.width as i32);
    let y1 = ((bounds.y + bounds.h) as i32).min(bm.height as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let w = bm.width as i32;
    let h = bm.height as i32;
    let mut src = bm.bgra.clone();
    for y in y0..y1 {
        for x in x0..x1 {
            let mut r = 0u32;
            let mut g = 0u32;
            let mut b = 0u32;
            let mut n = 0u32;
            for j in -radius..=radius {
                for i in -radius..=radius {
                    let sx = (x + i).clamp(0, w - 1);
                    let sy = (y + j).clamp(0, h - 1);
                    let idx = ((sy * w + sx) * 4) as usize;
                    b += src[idx] as u32;
                    g += src[idx + 1] as u32;
                    r += src[idx + 2] as u32;
                    n += 1;
                }
            }
            let idx = ((y * w + x) * 4) as usize;
            src[idx] = (b / n) as u8;
            src[idx + 1] = (g / n) as u8;
            src[idx + 2] = (r / n) as u8;
            src[idx + 3] = 0xFF;
        }
    }
    bm.bgra = src;
}
