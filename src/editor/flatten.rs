use crate::editor::Document;
use crate::editor::annotation::{Annotation, BlurMode, Point, Rect, ShapeKind};
use crate::error::Result;
use crate::gfx::bitmap::Bitmap;
use crate::gfx::color::Color;

const SSAA: u32 = 2;

/// Software-rasterised flatten path. Supports all annotation types including
/// text glyphs (GDI-rendered), bezier curves (subdivided), and magnify scaled
/// bitmap compositing. Uses 2× supersampling anti-aliasing (SSAA) for all
/// annotation edges so export/clipboard output matches the smooth D2D preview.
pub fn flatten(doc: &Document) -> Result<Bitmap> {
    let w = doc.base.width;
    let h = doc.base.height;
    let w2 = w * SSAA;
    let h2 = h * SSAA;

    // 1. Upsample base image to SSAA resolution (nearest neighbour).
    let mut ssaa = Bitmap::new(w2, h2);
    let sbgra = &doc.base.bgra;
    for y in 0..h2 {
        let sy = y / SSAA;
        let src_off = (sy * w * 4) as usize;
        for x in 0..w2 {
            let sx = x / SSAA;
            let si = src_off + (sx * 4) as usize;
            let di = ((y * w2 + x) * 4) as usize;
            ssaa.bgra[di..di + 4].copy_from_slice(&sbgra[si..si + 4]);
        }
    }
    let ssaa_base = ssaa.clone();

    // 2. Rasterise annotations at SSAA resolution (coordinates scaled by 2×
    //    give the existing rasterizer sub-pixel precision).
    for ann in &doc.annotations {
        rasterize(&mut ssaa, &ssaa_base, &scale_annotation(ann, SSAA as f32));
    }

    // 3. Downsample SSAA×SSAA blocks via box filter (averaging).
    let mut out = Bitmap::new(w, h);
    let block = (SSAA * SSAA) as u32;
    for y in 0..h {
        for x in 0..w {
            let mut r = 0u32; let mut g = 0u32; let mut b = 0u32; let mut a = 0u32;
            for dy in 0..SSAA {
                for dx in 0..SSAA {
                    let si = (((y * SSAA + dy) * w2 + (x * SSAA + dx)) * 4) as usize;
                    b += ssaa.bgra[si] as u32;
                    g += ssaa.bgra[si + 1] as u32;
                    r += ssaa.bgra[si + 2] as u32;
                    a += ssaa.bgra[si + 3] as u32;
                }
            }
            let di = ((y * w + x) * 4) as usize;
            out.bgra[di]     = (b / block) as u8;
            out.bgra[di + 1] = (g / block) as u8;
            out.bgra[di + 2] = (r / block) as u8;
            out.bgra[di + 3] = (a / block) as u8;
        }
    }
    Ok(out)
}

/// Return a copy of `ann` with all spatial coordinates and sizes multiplied
/// by `s`. Non-spatial fields (colors, text strings, blur intensity, shape
/// kind/rotation, stamp asset/rotation) are passed through unchanged.
fn scale_annotation(ann: &Annotation, s: f32) -> Annotation {
    match *ann {
        Annotation::Arrow { id, start, end, color, stroke, bezier } => {
            Annotation::Arrow {
                id,
                start: Point::new(start.x * s, start.y * s),
                end: Point::new(end.x * s, end.y * s),
                color,
                stroke: stroke * s,
                bezier: bezier.map(|b| Point::new(b.x * s, b.y * s)),
            }
        }
        Annotation::Shape { id, kind, bounds, outline, fill, stroke, rotation } => {
            Annotation::Shape {
                id,
                kind,
                bounds: Rect { x: bounds.x * s, y: bounds.y * s, w: bounds.w * s, h: bounds.h * s },
                outline,
                fill,
                stroke: stroke * s,
                rotation,
            }
        }
        Annotation::Text { id, bounds, ref text, color, size } => {
            Annotation::Text {
                id,
                bounds: Rect { x: bounds.x * s, y: bounds.y * s, w: bounds.w * s, h: bounds.h * s },
                text: text.clone(),
                color,
                size: size * s,
            }
        }
        Annotation::Step { id, center, radius, ref value, fill, text } => {
            Annotation::Step {
                id,
                center: Point::new(center.x * s, center.y * s),
                radius: radius * s,
                value: value.clone(),
                fill,
                text,
            }
        }
        Annotation::Highlighter { id, bounds, color } => {
            Annotation::Highlighter {
                id,
                bounds: Rect { x: bounds.x * s, y: bounds.y * s, w: bounds.w * s, h: bounds.h * s },
                color,
            }
        }
        Annotation::Blur { id, bounds, mode, fill, intensity } => {
            Annotation::Blur {
                id,
                bounds: Rect { x: bounds.x * s, y: bounds.y * s, w: bounds.w * s, h: bounds.h * s },
                mode,
                fill,
                intensity,
            }
        }
        Annotation::Magnify { id, source, display, line_width, color } => {
            Annotation::Magnify {
                id,
                source: Rect { x: source.x * s, y: source.y * s, w: source.w * s, h: source.h * s },
                display: Rect { x: display.x * s, y: display.y * s, w: display.w * s, h: display.h * s },
                line_width: line_width * s,
                color,
            }
        }
        Annotation::Stamp { id, asset, bounds, rotation } => {
            Annotation::Stamp {
                id,
                asset,
                bounds: Rect { x: bounds.x * s, y: bounds.y * s, w: bounds.w * s, h: bounds.h * s },
                rotation,
            }
        }
    }
}

fn rasterize(bm: &mut Bitmap, base: &Bitmap, ann: &Annotation) {
    match ann {
        Annotation::Arrow {
            start,
            end,
            color,
            stroke,
            bezier,
            ..
        } => {
            if let Some(b) = bezier {
                raster_bezier_arrow(bm, *start, *end, *b, *color, *stroke);
            } else {
                raster_arrow(bm, *start, *end, *color, *stroke);
            }
        }
        Annotation::Shape {
            kind,
            bounds,
            outline,
            fill,
            stroke,
            ..
        } => raster_shape(bm, *kind, *bounds, *outline, *fill, *stroke),
        Annotation::Text {
            bounds,
            text,
            color,
            size,
            ..
        } => {
            if !text.is_empty() {
                raster_text_gdi(bm, *bounds, text, *color, *size);
            }
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
        Annotation::Magnify {
            source,
            display,
            line_width,
            color,
            ..
        } => {
            raster_magnify_bitmap(bm, base, *source, *display);
            raster_rect_outline(bm, *source, *color, *line_width);
            raster_rect_outline(bm, *display, *color, *line_width);
            raster_thick_line(
                bm,
                Point::new(source.x + source.w * 0.5, source.y + source.h * 0.5),
                Point::new(display.x + display.w * 0.5, display.y + display.h * 0.5),
                *color,
                *line_width,
            );
        }
        Annotation::Stamp { asset, bounds, .. } => {
            raster_stamp(bm, *asset, *bounds);
        }
    }
}

fn raster_stamp(bm: &mut Bitmap, asset: u8, bounds: Rect) {
    let stamp = crate::editor::stamps::bitmap(asset);
    let sw = stamp.width as f32;
    let sh = stamp.height as f32;
    let dw = bounds.w.max(1.0);
    let dh = bounds.h.max(1.0);
    let x0 = bounds.x as i32;
    let y0 = bounds.y as i32;
    let x1 = (bounds.x + dw) as i32;
    let y1 = (bounds.y + dh) as i32;
    for y in y0..y1 {
        for x in x0..x1 {
            let u = ((x - x0) as f32 / dw * sw) as i32;
            let v = ((y - y0) as f32 / dh * sh) as i32;
            if u < 0 || v < 0 || (u as u32) >= stamp.width || (v as u32) >= stamp.height {
                continue;
            }
            let i = ((v as u32 * stamp.width + u as u32) * 4) as usize;
            let c = Color {
                b: stamp.bgra[i],
                g: stamp.bgra[i + 1],
                r: stamp.bgra[i + 2],
                a: stamp.bgra[i + 3],
            };
            if c.a > 0 {
                put_pixel(bm, x, y, c);
            }
        }
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

// ---- v2.1 flatten parity: text, bezier, magnify bitmap ----

fn raster_text_gdi(bm: &mut Bitmap, bounds: Rect, text: &str, color: Color, font_size: f32) {
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, CreateFontW,
        DeleteDC, DeleteObject, DIB_RGB_COLORS, GetDIBits, GetDC, ReleaseDC,
        SelectObject, SetBkMode, SetTextColor, TextOutW, TRANSPARENT,
        CreateSolidBrush, FillRect,
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, FW_NORMAL, OUT_DEFAULT_PRECIS,
        VARIABLE_PITCH,
    };
    unsafe {
        // Compute integer pixel dimensions covering the text rect.
        let rw = bounds.w.ceil() as i32;
        let rh = bounds.h.ceil() as i32;
        if rw <= 0 || rh <= 0 {
            return;
        }

        let screen_dc = GetDC(0);
        let mem_dc = CreateCompatibleDC(screen_dc);
        ReleaseDC(0, screen_dc);

        // 32bpp top-down DIB.
        let mut bi: BITMAPINFO = core::mem::zeroed();
        bi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = rw;
        bi.bmiHeader.biHeight = -rh;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB as u32;

        let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
        let dib = CreateDIBSection(mem_dc, &bi, DIB_RGB_COLORS, &mut bits, 0, 0);
        if dib == 0 || bits.is_null() {
            DeleteDC(mem_dc);
            return;
        }
        let old = SelectObject(mem_dc, dib as _);

        // Black fill.
        SetBkMode(mem_dc, TRANSPARENT as i32);
        let fill_brush = windows_sys::Win32::Graphics::Gdi::CreateSolidBrush(0x00000000);
        let fill_r = windows_sys::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: rw,
            bottom: rh,
        };
        windows_sys::Win32::Graphics::Gdi::FillRect(mem_dc, &fill_r, fill_brush);
        windows_sys::Win32::Graphics::Gdi::DeleteObject(fill_brush as _);

        // Create font and draw white text.
        let face = crate::win::wide("Segoe UI");
        let font = CreateFontW(
            -(font_size as i32).max(8),
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            VARIABLE_PITCH as u32,
            face.as_ptr(),
        );
        let old_font = SelectObject(mem_dc, font as _);
        SetTextColor(mem_dc, 0x00FFFFFF);
        let wtext = crate::win::wide(text);
        TextOutW(mem_dc, 0, 0, wtext.as_ptr(), (wtext.len() - 1) as i32);

        // Read back the intensity from R channel as alpha coverage.
        let mut row_buf = vec![0u8; (rw * 4) as usize];
        bi.bmiColors = [core::mem::zeroed()];
        for y in 0..rh {
            let _get = GetDIBits(
                mem_dc,
                dib,
                y as u32,
                1,
                row_buf.as_mut_ptr() as *mut _,
                &mut bi as *mut _,
                DIB_RGB_COLORS,
            );
            // GDI renders top-down DIBs with row 0 at the top of the image.
            let out_y = (bounds.y as i32) + y;
            for x in 0..rw {
                let pi = (x * 4) as usize;
                let cov = row_buf[pi + 2] as u32; // R channel of white-on-black
                if cov > 0 {
                    let out_x = (bounds.x as i32) + x;
                    let a = (color.a as u32 * cov / 255) as u8;
                    let blended = Color { r: color.r, g: color.g, b: color.b, a };
                    put_pixel(bm, out_x, out_y, blended);
                }
            }
        }

        SelectObject(mem_dc, old_font);
        DeleteObject(font as _);
        SelectObject(mem_dc, old);
        DeleteObject(dib as _);
        DeleteDC(mem_dc);
    }
}

fn raster_bezier_arrow(
    bm: &mut Bitmap,
    start: Point,
    end: Point,
    cp: Point,
    color: Color,
    stroke: f32,
) {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / len;
    let uy = dy / len;
    let head_size = stroke * 4.0 + 6.0;
    let shaft_end = Point::new(end.x - ux * head_size * 0.6, end.y - uy * head_size * 0.6);

    // Subdivide quadratic bezier into N segments and draw thick lines.
    let n = 20;
    let mut prev = start;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let tm = 1.0 - t;
        let x = tm * tm * start.x + 2.0 * tm * t * cp.x + t * t * shaft_end.x;
        let y = tm * tm * start.y + 2.0 * tm * t * cp.y + t * t * shaft_end.y;
        let cur = Point::new(x, y);
        raster_thick_line(bm, prev, cur, color, stroke);
        prev = cur;
    }

    // Arrow head triangle.
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

fn raster_magnify_bitmap(
    bm: &mut Bitmap,
    base: &Bitmap,
    source: Rect,
    display: Rect,
) {
    if source.w <= 0.0 || source.h <= 0.0 || display.w <= 0.0 || display.h <= 0.0 {
        return;
    }
    let dw = display.w as i32;
    let dh = display.h as i32;
    let dx0 = display.x as i32;
    let dy0 = display.y as i32;
    let sw = source.w;
    let sh = source.h;
    let sx0 = source.x;
    let sy0 = source.y;
    let bw = base.width as f32;
    let bh = base.height as f32;
    for dy in 0..dh {
        for dx in 0..dw {
            // Nearest-neighbour sampling from source rect.
            let u = sx0 + (dx as f32 + 0.5) / dw as f32 * sw;
            let v = sy0 + (dy as f32 + 0.5) / dh as f32 * sh;
            let ix = u as i32;
            let iy = v as i32;
            if ix < 0 || iy < 0 || ix as f32 >= bw || iy as f32 >= bh {
                continue;
            }
            let src_idx = ((iy as u32 * base.width + ix as u32) * 4) as usize;
            let c = Color {
                b: base.bgra[src_idx],
                g: base.bgra[src_idx + 1],
                r: base.bgra[src_idx + 2],
                a: base.bgra[src_idx + 3],
            };
            put_pixel(bm, dx0 + dx, dy0 + dy, c);
        }
    }
}
