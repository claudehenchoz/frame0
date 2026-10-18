//! Direct2D document renderer shared by the on-screen canvas and export.
//!
//! `Renderer` owns the device-bound resources (brush LRU, effects, uploaded
//! bitmaps). `draw_document` paints the base image plus every annotation
//! under a caller-supplied world transform; `flatten` uses the same code path
//! to render into an offscreen target and read the pixels back, so exported
//! PNG/JPG/clipboard output is pixel-identical to what the canvas shows.

use crate::editor::annotation::{
    Annotation, BlurMode, Document, EndStyle, Point, Rect, ShapeKind, Tail, arrow_polyline,
};
use crate::error::{Error, Result};
use crate::gfx::Gfx;
use crate::gfx::bitmap::Bitmap;
use crate::gfx::color::Color;
use crate::gfx::d2d::{color_f, upload_bitmap};
use std::rc::{Rc, Weak};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_POINT_2U, D2D_RECT_F, D2D_RECT_U, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED,
    D2D1_BEZIER_SEGMENT, D2D1_BORDER_MODE_HARD, D2D1_COMPOSITE_MODE_SOURCE_OVER,
    D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED,
    D2D1_FIGURE_END_OPEN, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1GaussianBlur, CLSID_D2D1Shadow, D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
    D2D1_BITMAP_OPTIONS_CPU_READ, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_CAP_STYLE_FLAT, D2D1_CAP_STYLE_ROUND, D2D1_COMBINE_MODE_UNION, D2D1_DASH_STYLE_SOLID,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE, D2D1_GAUSSIANBLUR_PROP_BORDER_MODE,
    D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
    D2D1_INTERPOLATION_MODE_LINEAR, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_LINE_JOIN_ROUND, D2D1_MAP_OPTIONS_READ, D2D1_PRIMITIVE_BLEND_MIN,
    D2D1_PRIMITIVE_BLEND_SOURCE_OVER, D2D1_PROPERTY_TYPE_ENUM, D2D1_PROPERTY_TYPE_FLOAT,
    D2D1_PROPERTY_TYPE_VECTOR4, D2D1_ROUNDED_RECT, D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION,
    D2D1_SHADOW_PROP_COLOR, D2D1_STROKE_STYLE_PROPERTIES1, ID2D1Bitmap, ID2D1Bitmap1, ID2D1Brush,
    ID2D1DeviceContext, ID2D1Effect, ID2D1Geometry, ID2D1Image, ID2D1PathGeometry,
    ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_METRICS, IDWriteTextLayout,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::Interface;
use windows_numerics::{Matrix3x2, Vector2};

const BRUSH_CACHE: usize = 12;
pub const FONT: &str = "Segoe UI";
const EMOJI_FONT: &str = "Segoe UI Emoji";
/// Callout text inset from the body edge.
pub const CALLOUT_PAD: f32 = 10.0;

#[derive(Clone, Copy, Default)]
pub struct RenderOpts {
    /// Render blur/pixelate regions as solid fills (irreversible redaction).
    pub secure: bool,
    /// Skip this annotation (it's being edited in place).
    pub skip: Option<u32>,
}

pub struct Renderer {
    brushes: Vec<(Color, ID2D1SolidColorBrush)>,
    gaussian: Option<ID2D1Effect>,
    shadow: Option<ID2D1Effect>,
    round: Option<ID2D1StrokeStyle>,
    base: Option<(u32, *const Bitmap, ID2D1Bitmap1)>,
    images: Vec<(Weak<Bitmap>, ID2D1Bitmap1)>,
    files: Vec<(String, ID2D1Bitmap1)>,
    pixelated: Vec<([i32; 6], ID2D1Bitmap1)>,
}

pub fn rect_f(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F { left: r.x, top: r.y, right: r.x + r.w, bottom: r.y + r.h }
}

pub fn v2(p: Point) -> Vector2 {
    Vector2 { X: p.x, Y: p.y }
}

pub fn identity() -> Matrix3x2 {
    Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 }
}

impl Renderer {
    pub fn new() -> Self {
        Self {
            brushes: Vec::new(),
            gaussian: None,
            shadow: None,
            round: None,
            base: None,
            images: Vec::new(),
            files: Vec::new(),
            pixelated: Vec::new(),
        }
    }

    /// Drop every device-bound resource (device loss).
    pub fn clear(&mut self) {
        *self = Renderer::new();
    }

    pub fn brush(&mut self, ctx: &ID2D1DeviceContext, color: Color) -> Result<ID2D1Brush> {
        if let Some(idx) = self.brushes.iter().position(|(c, _)| *c == color) {
            let e = self.brushes.remove(idx);
            let b = e.1.cast()?;
            self.brushes.push(e);
            return Ok(b);
        }
        let brush = unsafe { ctx.CreateSolidColorBrush(&color_f(color), None)? };
        if self.brushes.len() >= BRUSH_CACHE {
            self.brushes.remove(0);
        }
        self.brushes.push((color, brush.clone()));
        Ok(brush.cast()?)
    }

    fn round_style(&mut self, gfx: &Gfx) -> Result<ID2D1StrokeStyle> {
        if let Some(s) = &self.round {
            return Ok(s.clone());
        }
        let props = D2D1_STROKE_STYLE_PROPERTIES1 {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            dashCap: D2D1_CAP_STYLE_FLAT,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
            transformType: windows::Win32::Graphics::Direct2D::D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
        };
        let s: ID2D1StrokeStyle = unsafe { gfx.d2d.factory.CreateStrokeStyle(&props, None)?.into() };
        self.round = Some(s.clone());
        Ok(s)
    }

    pub fn base_bitmap(&mut self, ctx: &ID2D1DeviceContext, doc: &Document) -> Result<ID2D1Bitmap1> {
        let ptr = Rc::as_ptr(&doc.base);
        if let Some((g, p, bm)) = &self.base {
            if *g == doc.base_gen && *p == ptr {
                return Ok(bm.clone());
            }
        }
        let bm = upload_bitmap(ctx, &doc.base)?;
        self.base = Some((doc.base_gen, ptr, bm.clone()));
        self.pixelated.clear();
        Ok(bm)
    }

    fn image_bitmap(&mut self, ctx: &ID2D1DeviceContext, img: &Rc<Bitmap>) -> Result<ID2D1Bitmap1> {
        self.images.retain(|(w, _)| w.strong_count() > 0);
        if let Some((_, bm)) = self.images.iter().find(|(w, _)| core::ptr::eq(w.as_ptr(), Rc::as_ptr(img))) {
            return Ok(bm.clone());
        }
        let bm = upload_bitmap(ctx, img)?;
        self.images.push((Rc::downgrade(img), bm.clone()));
        Ok(bm)
    }

    fn file_stamp(&mut self, ctx: &ID2D1DeviceContext, id: &str) -> Result<Option<ID2D1Bitmap1>> {
        if let Some((_, bm)) = self.files.iter().find(|(k, _)| k == id) {
            return Ok(Some(bm.clone()));
        }
        let Some(src) = crate::editor::stamps::file_bitmap(id) else { return Ok(None) };
        let bm = upload_bitmap(ctx, &src)?;
        self.files.push((id.to_string(), bm.clone()));
        Ok(Some(bm))
    }

    /// Paint base + annotations. `world` maps document space to target space.
    pub fn draw_document(
        &mut self,
        gfx: &Gfx,
        ctx: &ID2D1DeviceContext,
        doc: &Document,
        world: &Matrix3x2,
        opts: RenderOpts,
    ) -> Result<()> {
        let base = self.base_bitmap(ctx, doc)?;
        unsafe {
            ctx.SetTransform(world);
            let base_bm: ID2D1Bitmap = base.cast()?;
            ctx.DrawBitmap(
                &base_bm,
                Some(&rect_f(doc.canvas_rect())),
                1.0,
                D2D1_INTERPOLATION_MODE_LINEAR,
                None,
                None,
            );
        }
        for ann in &doc.annotations {
            if opts.skip == Some(ann.id()) {
                continue;
            }
            self.draw_annotation(gfx, ctx, doc, &base, ann, world, opts)?;
        }
        unsafe { ctx.SetTransform(world) };
        Ok(())
    }

    pub fn draw_annotation(
        &mut self,
        gfx: &Gfx,
        ctx: &ID2D1DeviceContext,
        doc: &Document,
        base: &ID2D1Bitmap1,
        ann: &Annotation,
        world: &Matrix3x2,
        opts: RenderOpts,
    ) -> Result<()> {
        let rot = ann.rotation();
        let local = if rot != 0.0 {
            Matrix3x2::rotation_around(rot, v2(ann.frame().center()))
        } else {
            identity()
        };
        let shadow = matches!(
            ann,
            Annotation::Arrow { shadow: true, .. }
                | Annotation::Shape { shadow: true, .. }
                | Annotation::Callout { shadow: true, .. }
        );
        if shadow {
            // Record the annotation into a command list, then draw it twice:
            // once through the Shadow effect (offset), once plain.
            unsafe {
                let list = ctx.CreateCommandList()?;
                let prev = ctx.GetTarget()?;
                ctx.SetTarget(&list);
                ctx.SetTransform(&local);
                let r = self.draw_kind(gfx, ctx, doc, base, ann, opts);
                ctx.SetTarget(&prev);
                r?;
                list.Close()?;
                if self.shadow.is_none() {
                    self.shadow = Some(ctx.CreateEffect(&CLSID_D2D1Shadow)?);
                }
                let fx = self.shadow.as_ref().unwrap();
                let img: ID2D1Image = list.cast()?;
                fx.SetInput(0, &img, false);
                fx.SetValue(D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &3.0f32.to_le_bytes())?;
                let col: [f32; 4] = [0.0, 0.0, 0.0, 0.45];
                let mut bytes = [0u8; 16];
                for (i, c) in col.iter().enumerate() {
                    bytes[i * 4..i * 4 + 4].copy_from_slice(&c.to_le_bytes());
                }
                fx.SetValue(D2D1_SHADOW_PROP_COLOR.0 as u32, D2D1_PROPERTY_TYPE_VECTOR4, &bytes)?;
                let out = fx.GetOutput()?;
                ctx.SetTransform(world);
                ctx.DrawImage(&out, Some(&Vector2 { X: 4.0, Y: 4.0 }), None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
                ctx.DrawImage(&img, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
            }
            return Ok(());
        }
        unsafe { ctx.SetTransform(&(local * world)) };
        self.draw_kind(gfx, ctx, doc, base, ann, opts)
    }

    fn draw_kind(
        &mut self,
        gfx: &Gfx,
        ctx: &ID2D1DeviceContext,
        doc: &Document,
        base: &ID2D1Bitmap1,
        ann: &Annotation,
        opts: RenderOpts,
    ) -> Result<()> {
        match ann {
            Annotation::Arrow { start, end, color, stroke, bezier, head, tail, .. } => {
                self.draw_arrow(gfx, ctx, *start, *end, *color, *stroke, *bezier, *head, *tail)
            }
            Annotation::Shape { kind, bounds, points, outline, fill, stroke, .. } => {
                self.draw_shape(gfx, ctx, *kind, *bounds, points, *outline, *fill, *stroke)
            }
            Annotation::Callout { bounds, text, fill, outline, text_color, size, stroke, tails, radius, .. } => {
                self.draw_callout(gfx, ctx, *bounds, text, *fill, *outline, *text_color, *size, *stroke, tails, *radius)
            }
            Annotation::Text { bounds, text, color, size, .. } => self.draw_text(gfx, ctx, *bounds, text, *color, *size),
            Annotation::Step { center, radius, value, fill, text, .. } => {
                self.draw_step(gfx, ctx, *center, *radius, value, *fill, *text)
            }
            Annotation::Highlighter { bounds, color, .. } => self.draw_highlight(ctx, *bounds, *color),
            Annotation::Blur { bounds, mode, fill, intensity, .. } => {
                if opts.secure && *mode != BlurMode::Solid {
                    return self.fill_rect(ctx, *bounds, *fill);
                }
                match mode {
                    BlurMode::Solid => self.fill_rect(ctx, *bounds, *fill),
                    BlurMode::Blur => self.draw_blur(ctx, base, *bounds, *intensity),
                    BlurMode::Pixelate => self.draw_pixelate(ctx, doc, *bounds, *intensity),
                }
            }
            Annotation::Magnify { source, display, line_width, color, .. } => {
                self.draw_magnify(ctx, base, *source, *display, *line_width, *color)
            }
            Annotation::Stamp { asset, bounds, .. } => self.draw_stamp(gfx, ctx, asset, *bounds),
            Annotation::Image { bounds, bitmap, .. } => {
                let bm: ID2D1Bitmap = self.image_bitmap(ctx, bitmap)?.cast()?;
                unsafe {
                    ctx.DrawBitmap(&bm, Some(&rect_f(*bounds)), 1.0, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None);
                }
                Ok(())
            }
        }
    }

    fn path(&self, gfx: &Gfx, pts: &[Point], closed: bool) -> Result<ID2D1PathGeometry> {
        unsafe {
            let geom = gfx.d2d.factory.CreatePathGeometry()?;
            let sink = geom.Open()?;
            if let Some(first) = pts.first() {
                sink.BeginFigure(v2(*first), if closed { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW });
                for p in &pts[1..] {
                    sink.AddLine(v2(*p));
                }
                sink.EndFigure(if closed { D2D1_FIGURE_END_CLOSED } else { D2D1_FIGURE_END_OPEN });
            }
            sink.Close()?;
            Ok(geom.into())
        }
    }

    pub fn draw_arrow(
        &mut self,
        gfx: &Gfx,
        ctx: &ID2D1DeviceContext,
        start: Point,
        end: Point,
        color: Color,
        stroke: f32,
        bezier: Option<(Point, Point)>,
        head: EndStyle,
        tail: EndStyle,
    ) -> Result<()> {
        let brush = self.brush(ctx, color)?;
        let style = self.round_style(gfx)?;
        let size = stroke * 3.0 + 8.0;
        // Tangent directions at each end (pointing outward).
        let (h_from, t_from) = match bezier {
            Some((c1, c2)) => (if c2.dist(end) > 0.5 { c2 } else { start }, if c1.dist(start) > 0.5 { c1 } else { end }),
            None => (start, end),
        };
        let unit = |from: Point, to: Point| {
            let dx = to.x - from.x;
            let dy = to.y - from.y;
            let l = (dx * dx + dy * dy).sqrt().max(1e-3);
            (dx / l, dy / l)
        };
        let hu = unit(h_from, end);
        let tu = unit(t_from, start);
        // Pull the shaft back so it doesn't poke through arrowheads.
        let inset = |s: EndStyle| if s == EndStyle::Arrow { size * 0.7 } else { 0.0 };
        let s2 = Point::new(start.x - tu.0 * inset(tail), start.y - tu.1 * inset(tail));
        let e2 = Point::new(end.x - hu.0 * inset(head), end.y - hu.1 * inset(head));
        unsafe {
            let geom = gfx.d2d.factory.CreatePathGeometry()?;
            let sink = geom.Open()?;
            sink.BeginFigure(v2(s2), D2D1_FIGURE_BEGIN_HOLLOW);
            match bezier {
                Some((c1, c2)) => sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: v2(c1), point2: v2(c2), point3: v2(e2) }),
                None => sink.AddLine(v2(e2)),
            }
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            sink.Close()?;
            ctx.DrawGeometry(&geom, &brush, stroke, &style);
        }
        self.draw_end(gfx, ctx, end, hu, head, size, stroke, &brush)?;
        self.draw_end(gfx, ctx, start, tu, tail, size, stroke, &brush)?;
        Ok(())
    }

    fn draw_end(
        &mut self,
        gfx: &Gfx,
        ctx: &ID2D1DeviceContext,
        tip: Point,
        (ux, uy): (f32, f32),
        style: EndStyle,
        size: f32,
        stroke: f32,
        brush: &ID2D1Brush,
    ) -> Result<()> {
        match style {
            EndStyle::None => {}
            EndStyle::Arrow => {
                let (ox, oy) = (-uy, ux);
                let p2 = Point::new(tip.x - ux * size + ox * size * 0.55, tip.y - uy * size + oy * size * 0.55);
                let p3 = Point::new(tip.x - ux * size - ox * size * 0.55, tip.y - uy * size - oy * size * 0.55);
                let back = Point::new(tip.x - ux * size * 0.8, tip.y - uy * size * 0.8);
                let geom = self.path(gfx, &[tip, p2, back, p3], true)?;
                let rs = self.round_style(gfx)?;
                unsafe {
                    ctx.FillGeometry(&geom, brush, None);
                    ctx.DrawGeometry(&geom, brush, (stroke * 0.5).max(1.0), &rs);
                }
            }
            EndStyle::Circle => unsafe {
                let r = stroke * 1.4 + 3.0;
                ctx.FillEllipse(&D2D1_ELLIPSE { point: v2(tip), radiusX: r, radiusY: r }, brush);
            },
            EndStyle::Square => unsafe {
                let r = stroke * 1.3 + 3.0;
                ctx.FillRectangle(&rect_f(Rect { x: tip.x - r, y: tip.y - r, w: r * 2.0, h: r * 2.0 }), brush);
            },
        }
        Ok(())
    }

    pub fn draw_shape(
        &mut self,
        gfx: &Gfx,
        ctx: &ID2D1DeviceContext,
        kind: ShapeKind,
        bounds: Rect,
        points: &[Point],
        outline: Color,
        fill: Color,
        stroke: f32,
    ) -> Result<()> {
        let ob = self.brush(ctx, outline)?;
        let fb = if fill.a > 0 { Some(self.brush(ctx, fill)?) } else { None };
        let style = self.round_style(gfx)?;
        unsafe {
            match kind {
                ShapeKind::Rectangle => {
                    let r = rect_f(bounds);
                    if let Some(fb) = &fb {
                        ctx.FillRectangle(&r, fb);
                    }
                    if stroke > 0.0 {
                        ctx.DrawRectangle(&r, &ob, stroke, &style);
                    }
                }
                ShapeKind::Ellipse => {
                    let e = D2D1_ELLIPSE { point: v2(bounds.center()), radiusX: bounds.w * 0.5, radiusY: bounds.h * 0.5 };
                    if let Some(fb) = &fb {
                        ctx.FillEllipse(&e, fb);
                    }
                    if stroke > 0.0 {
                        ctx.DrawEllipse(&e, &ob, stroke, &style);
                    }
                }
                ShapeKind::Polygon => {
                    if points.len() < 2 {
                        return Ok(());
                    }
                    let geom = self.path(gfx, points, true)?;
                    if let Some(fb) = &fb {
                        ctx.FillGeometry(&geom, fb, None);
                    }
                    if stroke > 0.0 {
                        ctx.DrawGeometry(&geom, &ob, stroke, &style);
                    }
                }
            }
        }
        Ok(())
    }

    /// Body + tails as one unioned geometry so the outline is continuous.
    pub fn callout_geometry(&self, gfx: &Gfx, bounds: Rect, tails: &[Tail], radius: f32) -> Result<ID2D1Geometry> {
        unsafe {
            let rr = D2D1_ROUNDED_RECT { rect: rect_f(bounds), radiusX: radius, radiusY: radius };
            let mut geom: ID2D1Geometry = gfx.d2d.factory.CreateRoundedRectangleGeometry(&rr)?.cast()?;
            for t in tails {
                let a = Point::new(bounds.x + t.anchor.x * bounds.w, bounds.y + t.anchor.y * bounds.h);
                let dx = t.tip.x - a.x;
                let dy = t.tip.y - a.y;
                let l = (dx * dx + dy * dy).sqrt();
                if l < 1.0 {
                    continue;
                }
                let half = (bounds.w.min(bounds.h) * 0.22).clamp(6.0, 40.0);
                let (ox, oy) = (-dy / l * half, dx / l * half);
                let tri = self.path(gfx, &[Point::new(a.x + ox, a.y + oy), t.tip, Point::new(a.x - ox, a.y - oy)], true)?;
                let combined = gfx.d2d.factory.CreatePathGeometry()?;
                let sink = combined.Open()?;
                geom.CombineWithGeometry(&tri, D2D1_COMBINE_MODE_UNION, None, 0.25, &sink)?;
                sink.Close()?;
                geom = combined.cast()?;
            }
            Ok(geom)
        }
    }

    fn draw_callout(
        &mut self,
        gfx: &Gfx,
        ctx: &ID2D1DeviceContext,
        bounds: Rect,
        text: &str,
        fill: Color,
        outline: Color,
        text_color: Color,
        size: f32,
        stroke: f32,
        tails: &[Tail],
        radius: f32,
    ) -> Result<()> {
        let geom = self.callout_geometry(gfx, bounds, tails, radius)?;
        let fb = self.brush(ctx, fill)?;
        let ob = self.brush(ctx, outline)?;
        let style = self.round_style(gfx)?;
        unsafe {
            if fill.a > 0 {
                ctx.FillGeometry(&geom, &fb, None);
            }
            if stroke > 0.0 {
                ctx.DrawGeometry(&geom, &ob, stroke, &style);
            }
        }
        if !text.is_empty() {
            let inner = bounds.inflated(-CALLOUT_PAD);
            let layout = text_layout(gfx, text, size, inner.w.max(1.0), inner.h.max(1.0), true)?;
            let tb = self.brush(ctx, text_color)?;
            unsafe {
                ctx.DrawTextLayout(v2(Point::new(inner.x, inner.y)), &layout, &tb, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            }
        }
        Ok(())
    }

    fn draw_text(&mut self, gfx: &Gfx, ctx: &ID2D1DeviceContext, bounds: Rect, text: &str, color: Color, size: f32) -> Result<()> {
        if text.is_empty() {
            let b = self.brush(ctx, Color::rgba(0x80, 0x80, 0x80, 0xC0))?;
            unsafe { ctx.DrawRectangle(&rect_f(bounds), &b, 1.0, None) };
            return Ok(());
        }
        let layout = text_layout(gfx, text, size, bounds.w.max(1.0), bounds.h.max(1.0), false)?;
        let brush = self.brush(ctx, color)?;
        unsafe {
            ctx.DrawTextLayout(v2(Point::new(bounds.x, bounds.y)), &layout, &brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
        Ok(())
    }

    fn draw_step(&mut self, gfx: &Gfx, ctx: &ID2D1DeviceContext, center: Point, radius: f32, value: &str, fill: Color, text_color: Color) -> Result<()> {
        let fb = self.brush(ctx, fill)?;
        let tb = self.brush(ctx, text_color)?;
        let len = value.chars().count().max(1) as f32;
        let fsize = radius * if len <= 1.0 { 1.15 } else if len <= 2.0 { 0.95 } else { 0.7 };
        unsafe {
            ctx.FillEllipse(&D2D1_ELLIPSE { point: v2(center), radiusX: radius, radiusY: radius }, &fb);
            let format = gfx.dwrite.text_format(FONT, fsize, true, true)?;
            let rect = rect_f(Rect { x: center.x - radius, y: center.y - radius, w: radius * 2.0, h: radius * 2.0 });
            let w: Vec<u16> = value.encode_utf16().collect();
            ctx.DrawText(&w, &format, &rect, &tb, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        }
        Ok(())
    }

    fn draw_highlight(&mut self, ctx: &ID2D1DeviceContext, bounds: Rect, color: Color) -> Result<()> {
        // Translucent marker fill: reads on both light and dark content
        // (a MIN/multiply blend would vanish on dark-mode screenshots).
        let brush = self.brush(ctx, Color::rgba(color.r, color.g, color.b, 0x6E))?;
        unsafe { ctx.FillRectangle(&rect_f(bounds), &brush) };
        Ok(())
    }

    fn fill_rect(&mut self, ctx: &ID2D1DeviceContext, bounds: Rect, fill: Color) -> Result<()> {
        let brush = self.brush(ctx, fill)?;
        unsafe { ctx.FillRectangle(&rect_f(bounds), &brush) };
        Ok(())
    }

    fn draw_blur(&mut self, ctx: &ID2D1DeviceContext, base: &ID2D1Bitmap1, bounds: Rect, intensity: f32) -> Result<()> {
        if self.gaussian.is_none() {
            let eff = unsafe { ctx.CreateEffect(&CLSID_D2D1GaussianBlur)? };
            unsafe {
                eff.SetValue(D2D1_GAUSSIANBLUR_PROP_BORDER_MODE.0 as u32, D2D1_PROPERTY_TYPE_ENUM, &(D2D1_BORDER_MODE_HARD.0 as u32).to_le_bytes())?;
            }
            self.gaussian = Some(eff);
        }
        let effect = self.gaussian.as_ref().unwrap();
        unsafe {
            let img: ID2D1Image = base.cast()?;
            effect.SetInput(0, &img, false);
            let stdev = (intensity / 2.0).max(0.5);
            effect.SetValue(D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &stdev.to_le_bytes())?;
            let out: ID2D1Image = effect.GetOutput()?;
            ctx.DrawImage(&out, Some(&v2(Point::new(bounds.x, bounds.y))), Some(&rect_f(bounds)), D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
        }
        Ok(())
    }

    fn draw_pixelate(&mut self, ctx: &ID2D1DeviceContext, doc: &Document, bounds: Rect, intensity: f32) -> Result<()> {
        let block = (intensity.round() as i32).max(2);
        // Snap the grid to document pixels so blocks stay put while dragging.
        let x0 = (bounds.x.floor() as i32).max(0);
        let y0 = (bounds.y.floor() as i32).max(0);
        let x1 = (bounds.right().ceil() as i32).min(doc.base.width as i32);
        let y1 = (bounds.bottom().ceil() as i32).min(doc.base.height as i32);
        if x1 <= x0 || y1 <= y0 {
            return Ok(());
        }
        let key = [x0, y0, x1, y1, block, doc.base_gen as i32];
        let bm = match self.pixelated.iter().find(|(k, _)| *k == key) {
            Some((_, b)) => b.clone(),
            None => {
                let small = crate::editor::pixels::pixelate_blocks(&doc.base, x0, y0, x1, y1, block);
                let b = upload_bitmap(ctx, &small)?;
                if self.pixelated.len() >= 8 {
                    self.pixelated.remove(0);
                }
                self.pixelated.push((key, b.clone()));
                b
            }
        };
        let bw = ((x1 - x0) + block - 1) / block;
        let bh = ((y1 - y0) + block - 1) / block;
        let dest = Rect { x: x0 as f32, y: y0 as f32, w: (bw * block) as f32, h: (bh * block) as f32 };
        unsafe {
            ctx.PushAxisAlignedClip(&rect_f(bounds), windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED);
            let b: ID2D1Bitmap = bm.cast()?;
            ctx.DrawBitmap(&b, Some(&rect_f(dest)), 1.0, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None, None);
            ctx.PopAxisAlignedClip();
        }
        Ok(())
    }

    fn draw_magnify(&mut self, ctx: &ID2D1DeviceContext, base: &ID2D1Bitmap1, source: Rect, display: Rect, line_width: f32, color: Color) -> Result<()> {
        let brush = self.brush(ctx, color)?;
        unsafe {
            // Connector first so it sits under the inset.
            let (a, b) = nearest_corners(source, display);
            ctx.DrawLine(v2(a), v2(b), &brush, line_width, None);
            let base_bm: ID2D1Bitmap = base.cast()?;
            ctx.DrawBitmap(&base_bm, Some(&rect_f(display)), 1.0, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, Some(&rect_f(source)), None);
            ctx.DrawRectangle(&rect_f(source), &brush, line_width, None);
            ctx.DrawRectangle(&rect_f(display), &brush, line_width, None);
        }
        Ok(())
    }

    fn draw_stamp(&mut self, gfx: &Gfx, ctx: &ID2D1DeviceContext, asset: &str, bounds: Rect) -> Result<()> {
        if let Some(e) = asset.strip_prefix("e:") {
            let size = bounds.h.min(bounds.w) * 0.82;
            let format = gfx.dwrite.text_format(EMOJI_FONT, size.max(4.0), false, true)?;
            let w: Vec<u16> = e.encode_utf16().collect();
            let brush = self.brush(ctx, Color::BLACK)?;
            unsafe {
                ctx.DrawText(&w, &format, &rect_f(bounds), &brush, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, DWRITE_MEASURING_MODE_NATURAL);
            }
        } else if let Some(t) = asset.strip_prefix("l:") {
            let c = crate::editor::stamps::label_color(t);
            let stroke = (bounds.h * 0.07).max(1.5);
            let r = bounds.inflated(-stroke * 0.5);
            let rr = D2D1_ROUNDED_RECT { rect: rect_f(r), radiusX: r.h * 0.22, radiusY: r.h * 0.22 };
            let fill = self.brush(ctx, Color::rgba(c.r, c.g, c.b, 0x28))?;
            let ob = self.brush(ctx, c)?;
            let format = gfx.dwrite.text_format(FONT, (bounds.h * 0.5).max(4.0), true, true)?;
            let w: Vec<u16> = t.encode_utf16().collect();
            unsafe {
                ctx.FillRoundedRectangle(&rr, &fill);
                ctx.DrawRoundedRectangle(&rr, &ob, stroke, None);
                ctx.DrawText(&w, &format, &rect_f(bounds), &ob, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
            }
        } else if let Some(bm) = self.file_stamp(ctx, asset)? {
            let b: ID2D1Bitmap = bm.cast()?;
            unsafe {
                ctx.DrawBitmap(&b, Some(&rect_f(bounds)), 1.0, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None);
            }
        } else {
            // Missing user file: dashed placeholder.
            let b = self.brush(ctx, Color::rgb(0x90, 0x90, 0x90))?;
            unsafe { ctx.DrawRectangle(&rect_f(bounds), &b, 1.0, None) };
        }
        Ok(())
    }
}

/// Pick the pair of corners (one from each rect) closest to each other.
fn nearest_corners(a: Rect, b: Rect) -> (Point, Point) {
    let mut best = (a.center(), b.center());
    let mut d = f32::MAX;
    for p in a.corners() {
        for q in b.corners() {
            let dd = p.dist(q);
            if dd < d {
                d = dd;
                best = (p, q);
            }
        }
    }
    best
}

pub fn text_layout(gfx: &Gfx, text: &str, size: f32, w: f32, h: f32, centered: bool) -> Result<IDWriteTextLayout> {
    let format = gfx.dwrite.text_format(FONT, size.max(1.0), false, false)?;
    let wtext: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let layout = gfx.dwrite.factory.CreateTextLayout(&wtext, &format, w, h)?;
        if centered {
            let _ = layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            let _ = layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        }
        Ok(layout)
    }
}

/// Measure wrapped text; returns (width, height) of the laid-out block.
pub fn measure_text(gfx: &Gfx, text: &str, size: f32, max_w: f32) -> (f32, f32) {
    let t = if text.is_empty() { " " } else { text };
    let Ok(layout) = text_layout(gfx, t, size, max_w.max(1.0), 100_000.0, false) else {
        return (max_w, size * 1.4);
    };
    let mut m = DWRITE_TEXT_METRICS::default();
    unsafe {
        if layout.GetMetrics(&mut m).is_err() {
            return (max_w, size * 1.4);
        }
    }
    (m.widthIncludingTrailingWhitespace, m.height)
}

/// Render `doc` at 1:1 into a CPU bitmap (straight alpha, BGRA). Large
/// documents are rendered in tiles to stay under the device's max texture size.
pub fn flatten(gfx: &Gfx, doc: &Document, opts: RenderOpts) -> Result<Bitmap> {
    let ctx = &gfx.d2d.context;
    let (w, h) = (doc.base.width, doc.base.height);
    let mut out = Bitmap::new(w, h);
    let mut renderer = Renderer::new();
    let tile = 4096u32;
    let fmt = D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED };
    let mut ty = 0;
    while ty < h {
        let th = tile.min(h - ty);
        let mut tx = 0;
        while tx < w {
            let tw = tile.min(w - tx);
            unsafe {
                let size = D2D_SIZE_U { width: tw, height: th };
                let tprops = D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: fmt,
                    dpiX: 96.0,
                    dpiY: 96.0,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                    colorContext: core::mem::ManuallyDrop::new(None),
                };
                let target = ctx.CreateBitmap(size, None, 0, &tprops)?;
                let rprops = D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: fmt,
                    dpiX: 96.0,
                    dpiY: 96.0,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                    colorContext: core::mem::ManuallyDrop::new(None),
                };
                let readback = ctx.CreateBitmap(size, None, 0, &rprops)?;
                let img: ID2D1Image = target.cast()?;
                ctx.SetTarget(&img);
                ctx.BeginDraw();
                ctx.Clear(Some(&color_f(Color::TRANSPARENT)));
                let world = Matrix3x2 { M31: -(tx as f32), M32: -(ty as f32), ..identity() };
                let r = renderer.draw_document(gfx, ctx, doc, &world, opts);
                ctx.SetTransform(&identity());
                let end = ctx.EndDraw(None, None);
                ctx.SetTarget(None);
                r?;
                end?;
                readback.CopyFromBitmap(Some(&D2D_POINT_2U { x: 0, y: 0 }), &target, Some(&D2D_RECT_U { left: 0, top: 0, right: tw, bottom: th }))?;
                let mapped = readback.Map(D2D1_MAP_OPTIONS_READ)?;
                for row in 0..th {
                    let src = core::slice::from_raw_parts(mapped.bits.add((row * mapped.pitch) as usize), (tw * 4) as usize);
                    let off = (((ty + row) * w + tx) * 4) as usize;
                    let dst = &mut out.bgra[off..off + (tw * 4) as usize];
                    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                        let a = s[3] as u32;
                        if a == 255 || a == 0 {
                            d.copy_from_slice(s);
                        } else {
                            // Un-premultiply.
                            d[0] = ((s[0] as u32 * 255 + a / 2) / a).min(255) as u8;
                            d[1] = ((s[1] as u32 * 255 + a / 2) / a).min(255) as u8;
                            d[2] = ((s[2] as u32 * 255 + a / 2) / a).min(255) as u8;
                            d[3] = a as u8;
                        }
                    }
                }
                readback.Unmap()?;
            }
            tx += tw;
        }
        ty += th;
    }
    if out.bgra.is_empty() {
        return Err(Error::msg("empty document"));
    }
    Ok(out)
}
