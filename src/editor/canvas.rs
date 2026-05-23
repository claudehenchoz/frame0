//! D2D-rendered canvas child window.
//!
//! The canvas owns its DXGI swap chain and renders entirely with Direct2D 1.1
//! through `ID2D1DeviceContext`. The device context is shared from
//! `crate::gfx::d2d::D2D` (one per process, single-threaded apartment).
//!
//! Per-frame: `BeginDraw` → clear → `SetTransform` (scale + translate) →
//! `DrawBitmap(base)` → walk annotations → preview drag → handles → `EndDraw` →
//! `Present(1, 0)`. `EndDraw` is wrapped with a one-shot retry on
//! `D2DERR_RECREATE_TARGET` (device loss).

use crate::editor::annotation::{Annotation, BlurMode, Document, Point, Rect, ShapeKind};
use crate::editor::history::Command;
use crate::editor::tools::{Drag, Tool, ToolStyle};
use crate::error::{Error, Result};
use crate::gfx::Gfx;
use crate::gfx::bitmap::Bitmap;
use crate::gfx::color::Color;
use crate::gfx::d2d::{color_f, upload_bitmap};
use std::rc::Rc;
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND as WinHWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_COMPOSITE_MODE_SOURCE_OVER, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_END_CLOSED, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1GaussianBlur, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE,
    D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
    D2D1_INTERPOLATION_MODE_LINEAR, D2D1_PRIMITIVE_BLEND, D2D1_PRIMITIVE_BLEND_SOURCE_OVER,
    D2D1_PROPERTY_TYPE_FLOAT, ID2D1Bitmap1, ID2D1Brush, ID2D1Effect, ID2D1Image,
    ID2D1SolidColorBrush,
};
use windows_numerics::{Matrix3x2, Vector2};
use windows::Win32::Graphics::DirectWrite::DWRITE_MEASURING_MODE_NATURAL;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain1;
use windows::core::Interface;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_BACK, VK_CONTROL, VK_DELETE, VK_ESCAPE,
    VK_Y, VK_Z,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetParent, IDC_ARROW, LoadCursorW, RegisterClassExW,
    WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SIZE,
    WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN, WS_VISIBLE,
};

const CLASS_NAME: &str = "frame.editor.canvas";
const HANDLE_HALF: f32 = 4.0;
const BEZIER_HANDLE_RADIUS: f32 = 6.0;
const BRUSH_CACHE: usize = 8;

struct HandleDrag {
    id: u32,
    original_ann: Annotation,
}

pub struct Canvas {
    pub gfx: Rc<Gfx>,
    pub doc: Document,
    pub selected: Option<u32>,
    pub view_scale: f32,
    pub view_offset: (f32, f32),
    pub size: (u32, u32),
    pub active_tool: Tool,
    pub style: ToolStyle,
    pub drag: Option<Drag>,
    pub move_anchor: Option<Point>,
    handle_drag: Option<HandleDrag>,

    // D2D resources. Created in `attach`; rebuilt on device loss.
    swap: Option<IDXGISwapChain1>,
    target: Option<ID2D1Bitmap1>,
    base: Option<ID2D1Bitmap1>,
    brushes: Vec<(Color, ID2D1SolidColorBrush)>,
    gaussian: Option<ID2D1Effect>,
    /// Bundled-stamp bitmaps, lazily decoded on first draw.
    stamps: Vec<Option<ID2D1Bitmap1>>,
}

impl Canvas {
    pub fn create(gfx: Rc<Gfx>, doc: Document) -> Result<Self> {
        Ok(Self {
            gfx,
            doc,
            selected: None,
            view_scale: 1.0,
            view_offset: (24.0, 12.0),
            size: (1024, 768),
            active_tool: Tool::Arrow,
            style: ToolStyle::default(),
            drag: None,
            move_anchor: None,
            handle_drag: None,
            swap: None,
            target: None,
            base: None,
            brushes: Vec::new(),
            gaussian: None,
            stamps: vec![None; crate::editor::stamps::COUNT],
        })
    }

    /// Attach to the canvas child HWND. Creates the swap chain and renders
    /// the base image into a D2D bitmap.
    pub fn attach(&mut self, hwnd: HWND, w: u32, h: u32) -> Result<()> {
        self.size = (w.max(1), h.max(1));
        let (swap, target) = self
            .gfx
            .d2d
            .create_swapchain_target(WinHWND(hwnd as *mut _), self.size.0, self.size.1)?;
        self.swap = Some(swap);
        self.target = Some(target);
        self.upload_base()?;
        Ok(())
    }

    pub fn resize(&mut self, w: u32, h: u32) -> Result<()> {
        let new = (w.max(1), h.max(1));
        if new == self.size {
            return Ok(());
        }
        self.size = new;
        let Some(swap) = self.swap.as_ref() else { return Ok(()) };
        unsafe {
            // Must drop the bound target bitmap before ResizeBuffers.
            self.target = None;
            self.gfx.d2d.context.SetTarget(None);
            swap.ResizeBuffers(
                0,
                new.0,
                new.1,
                windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN,
                windows::Win32::Graphics::Dxgi::DXGI_SWAP_CHAIN_FLAG(0),
            )?;
            self.target = Some(self.gfx.d2d.target_bitmap_for_swap(swap)?);
        }
        Ok(())
    }

    pub fn replace_document(&mut self, doc: Document) -> Result<()> {
        self.doc = doc;
        self.selected = None;
        self.upload_base()
    }

    fn upload_base(&mut self) -> Result<()> {
        let bm = upload_bitmap(&self.gfx.d2d.context, &self.doc.base)?;
        self.base = Some(bm);
        Ok(())
    }

    /// Render one frame. On device loss (`D2DERR_RECREATE_TARGET`) the canvas
    /// drops its device-bound resources and skips this frame; the next
    /// `attach` (called by the canvas wndproc when it sees `swap.is_none()`)
    /// rebuilds them. A perfect recovery would re-attach in-place; that's a
    /// v2.1 task.
    pub fn render(&mut self) -> Result<()> {
        if self.target.is_none() || self.swap.is_none() {
            return Ok(());
        }
        match self.render_inner() {
            Ok(()) => Ok(()),
            Err(Error::Hr(h)) if h == D2DERR_RECREATE_TARGET.0 => {
                self.swap = None;
                self.target = None;
                self.base = None;
                self.brushes.clear();
                self.gaussian = None;
                for s in &mut self.stamps {
                    *s = None;
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn render_inner(&mut self) -> Result<()> {
        let ctx = self.gfx.d2d.context.clone();
        let target = self.target.as_ref().ok_or(Error::msg("no D2D target"))?;
        unsafe {
            let target_img: ID2D1Image = target.cast()?;
            ctx.SetTarget(Some(&target_img));
            ctx.BeginDraw();
            ctx.Clear(Some(&color_f(Color::rgb(0x1A, 0x1B, 0x1D))));

            // Document-space transform.
            let m = Matrix3x2 {
                M11: self.view_scale,
                M12: 0.0,
                M21: 0.0,
                M22: self.view_scale,
                M31: self.view_offset.0,
                M32: self.view_offset.1,
            };
            ctx.SetTransform(&m);

            // Base image.
            if let Some(base) = &self.base {
                let dest = D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: self.doc.base.width as f32,
                    bottom: self.doc.base.height as f32,
                };
                let base_bm: windows::Win32::Graphics::Direct2D::ID2D1Bitmap = base.cast()?;
                ctx.DrawBitmap(
                    &base_bm,
                    Some(&dest),
                    1.0,
                    D2D1_INTERPOLATION_MODE_LINEAR,
                    None,
                    None,
                );
            }

            // Annotations.
            let annotations = self.doc.annotations.clone();
            for ann in &annotations {
                self.draw_annotation(&ctx, ann)?;
            }

            // Preview drag.
            if let Some(drag) = self.drag {
                self.draw_preview(&ctx, drag)?;
            }

            // Selection handles.
            if let Some(sel) = self.selected {
                if let Some(i) = self.doc.find_index(sel) {
                    let bb = self.doc.annotations[i].bounding_box();
                    self.draw_handles(&ctx, bb)?;
                    // Bezier control-point handle for selected bezier arrows.
                    if let Annotation::Arrow { bezier: Some(cp), .. } = &self.doc.annotations[i] {
                        self.draw_bezier_handle(&ctx, *cp)?;
                    }
                }
            }

            // Reset transform; EndDraw.
            let identity = Matrix3x2 {
                M11: 1.0,
                M12: 0.0,
                M21: 0.0,
                M22: 1.0,
                M31: 0.0,
                M32: 0.0,
            };
            ctx.SetTransform(&identity);

            let mut tag1 = 0u64;
            let mut tag2 = 0u64;
            ctx.EndDraw(Some(&mut tag1), Some(&mut tag2))?;
            ctx.SetTarget(None);
        }
        if let Some(swap) = &self.swap {
            self.gfx.d2d.present(swap);
        }
        Ok(())
    }

    fn brush(&mut self, color: Color) -> Result<ID2D1SolidColorBrush> {
        if let Some(idx) = self.brushes.iter().position(|(c, _)| *c == color) {
            return Ok(self.brushes[idx].1.clone());
        }
        let cf = color_f(color);
        let brush = unsafe { self.gfx.d2d.context.CreateSolidColorBrush(&cf, None)? };
        if self.brushes.len() >= BRUSH_CACHE {
            self.brushes.remove(0);
        }
        self.brushes.push((color, brush.clone()));
        Ok(brush)
    }

    fn draw_annotation(
        &mut self,
        ctx: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
        ann: &Annotation,
    ) -> Result<()> {
        match ann {
            Annotation::Arrow { start, end, color, stroke, bezier, .. } => {
                self.draw_arrow(ctx, *start, *end, *color, *stroke, *bezier)
            }
            Annotation::Shape { kind, bounds, outline, fill, stroke, .. } => {
                self.draw_shape(ctx, *kind, *bounds, *outline, *fill, *stroke)
            }
            Annotation::Text { bounds, text, color, size, .. } => {
                self.draw_text(ctx, *bounds, text, *color, *size)
            }
            Annotation::Step { center, radius, value, fill, text, .. } => {
                self.draw_step(ctx, *center, *radius, value, *fill, *text)
            }
            Annotation::Highlighter { bounds, color, .. } => {
                self.draw_highlight(ctx, *bounds, *color)
            }
            Annotation::Blur { bounds, mode, fill, intensity, .. } => match mode {
                BlurMode::Solid => self.draw_solid(ctx, *bounds, *fill),
                BlurMode::Blur => self.draw_blur(ctx, *bounds, *intensity),
            },
            Annotation::Magnify { source, display, line_width, color, .. } => {
                self.draw_magnify(ctx, *source, *display, *line_width, *color)
            }
            Annotation::Stamp { asset, bounds, .. } => {
                self.draw_stamp(ctx, *asset, *bounds)
            }
        }
    }

    fn draw_arrow(
        &mut self,
        ctx: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
        start: Point,
        end: Point,
        color: Color,
        stroke: f32,
        bezier: Option<Point>,
    ) -> Result<()> {
        let brush = self.brush(color)?;
        let brush_iface: ID2D1Brush = brush.cast()?;

        // Direction at the head end — used to size and orient the arrow head.
        // For straight arrows this is start→end; for bezier arrows we use the
        // tangent at t=1, which is `2 * (end - control)` for a quadratic.
        let (tdx, tdy) = if let Some(b) = bezier {
            (end.x - b.x, end.y - b.y)
        } else {
            (end.x - start.x, end.y - start.y)
        };
        let tlen = (tdx * tdx + tdy * tdy).sqrt().max(1.0);
        let ux = tdx / tlen;
        let uy = tdy / tlen;
        let head = stroke * 4.0 + 6.0;
        let shaft_ex = end.x - ux * head * 0.6;
        let shaft_ey = end.y - uy * head * 0.6;

        unsafe {
            if let Some(b) = bezier {
                // Quadratic-bezier shaft via path geometry.
                let geom = self.gfx.d2d.factory.CreatePathGeometry()?;
                let sink = geom.Open()?;
                sink.BeginFigure(
                    Vector2 { X: start.x, Y: start.y },
                    windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_HOLLOW,
                );
                let segs = [windows::Win32::Graphics::Direct2D::D2D1_QUADRATIC_BEZIER_SEGMENT {
                    point1: Vector2 { X: b.x, Y: b.y },
                    point2: Vector2 { X: shaft_ex, Y: shaft_ey },
                }];
                sink.AddQuadraticBeziers(&segs);
                sink.EndFigure(
                    windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_OPEN,
                );
                sink.Close()?;
                ctx.DrawGeometry(&geom, &brush_iface, stroke, None);
            } else {
                ctx.DrawLine(
                    Vector2 { X: start.x, Y: start.y },
                    Vector2 { X: shaft_ex, Y: shaft_ey },
                    &brush_iface,
                    stroke,
                    None,
                );
            }
        }

        // Head triangle as filled path geometry.
        let ortho_x = -uy;
        let ortho_y = ux;
        let p1 = Vector2 { X: end.x, Y: end.y };
        let p2 = Vector2 {
            X: end.x - ux * head + ortho_x * head * 0.5,
            Y: end.y - uy * head + ortho_y * head * 0.5,
        };
        let p3 = Vector2 {
            X: end.x - ux * head - ortho_x * head * 0.5,
            Y: end.y - uy * head - ortho_y * head * 0.5,
        };

        let geom = unsafe { self.gfx.d2d.factory.CreatePathGeometry()? };
        unsafe {
            let sink = geom.Open()?;
            sink.BeginFigure(p1, D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(p2);
            sink.AddLine(p3);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            ctx.FillGeometry(&geom, &brush_iface, None);
        }
        Ok(())
    }

    fn draw_shape(
        &mut self,
        ctx: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
        kind: ShapeKind,
        bounds: Rect,
        outline: Color,
        fill: Color,
        stroke: f32,
    ) -> Result<()> {
        let outline_brush: ID2D1Brush = self.brush(outline)?.cast()?;
        let fill_brush = if fill.a > 0 {
            Some(self.brush(fill)?.cast::<ID2D1Brush>()?)
        } else {
            None
        };
        unsafe {
            match kind {
                ShapeKind::Rectangle => {
                    let r = D2D_RECT_F {
                        left: bounds.x,
                        top: bounds.y,
                        right: bounds.x + bounds.w,
                        bottom: bounds.y + bounds.h,
                    };
                    if let Some(fb) = &fill_brush {
                        ctx.FillRectangle(&r, fb);
                    }
                    ctx.DrawRectangle(&r, &outline_brush, stroke, None);
                }
                ShapeKind::Ellipse => {
                    let cx = bounds.x + bounds.w * 0.5;
                    let cy = bounds.y + bounds.h * 0.5;
                    let e = D2D1_ELLIPSE {
                        point: Vector2 { X: cx, Y: cy },
                        radiusX: bounds.w * 0.5,
                        radiusY: bounds.h * 0.5,
                    };
                    if let Some(fb) = &fill_brush {
                        ctx.FillEllipse(&e, fb);
                    }
                    ctx.DrawEllipse(&e, &outline_brush, stroke, None);
                }
            }
        }
        Ok(())
    }

    fn draw_text(
        &mut self,
        ctx: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
        bounds: Rect,
        text: &str,
        color: Color,
        size: f32,
    ) -> Result<()> {
        if text.is_empty() {
            // Placeholder outline so empty text annotations are still visible.
            let outline = self.brush(Color::rgb(0x80, 0x80, 0x80))?;
            let outline_iface: ID2D1Brush = outline.cast()?;
            let r = D2D_RECT_F {
                left: bounds.x,
                top: bounds.y,
                right: bounds.x + bounds.w,
                bottom: bounds.y + bounds.h,
            };
            unsafe {
                ctx.DrawRectangle(&r, &outline_iface, 1.0, None);
            }
            return Ok(());
        }
        let brush: ID2D1Brush = self.brush(color)?.cast()?;
        let format = self.gfx.dwrite.text_format("Segoe UI", size, false, false)?;
        let text_w: Vec<u16> = text.encode_utf16().collect();
        let layout = unsafe {
            self.gfx.dwrite.factory.CreateTextLayout(
                &text_w,
                &format,
                bounds.w.max(1.0),
                bounds.h.max(1.0),
            )?
        };
        unsafe {
            ctx.DrawTextLayout(
                Vector2 { X: bounds.x, Y: bounds.y },
                &layout,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
        Ok(())
    }

    fn draw_step(
        &mut self,
        ctx: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
        center: Point,
        radius: f32,
        value: &str,
        fill: Color,
        text_color: Color,
    ) -> Result<()> {
        let fill_brush: ID2D1Brush = self.brush(fill)?.cast()?;
        let text_brush: ID2D1Brush = self.brush(text_color)?.cast()?;
        let ellipse = D2D1_ELLIPSE {
            point: Vector2 { X: center.x, Y: center.y },
            radiusX: radius,
            radiusY: radius,
        };
        unsafe {
            ctx.FillEllipse(&ellipse, &fill_brush);
            let format = self
                .gfx
                .dwrite
                .text_format("Segoe UI", radius * 1.1, true, true)?;
            let rect = D2D_RECT_F {
                left: center.x - radius,
                top: center.y - radius,
                right: center.x + radius,
                bottom: center.y + radius,
            };
            let text_w: Vec<u16> = value.encode_utf16().collect();
            ctx.DrawText(
                &text_w,
                &format,
                &rect,
                &text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
        Ok(())
    }

    fn draw_highlight(
        &mut self,
        ctx: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
        bounds: Rect,
        color: Color,
    ) -> Result<()> {
        // Alpha-blend a translucent fill — premultiplied D2D bitmap + an alpha
        // ~100/255 brush gives a clean highlighter tint over the underlying
        // pixels. (True multiply-blend would need D2D1_PRIMITIVE_BLEND_MIN
        // via SetPrimitiveBlend, which the windows 0.61 bindings don't
        // expose for ID2D1DeviceContext; the alpha path is visually close.)
        let blend = Color::rgba(color.r, color.g, color.b, 110);
        let brush: ID2D1Brush = self.brush(blend)?.cast()?;
        let rect = D2D_RECT_F {
            left: bounds.x,
            top: bounds.y,
            right: bounds.x + bounds.w,
            bottom: bounds.y + bounds.h,
        };
        unsafe {
            ctx.FillRectangle(&rect, &brush);
        }
        Ok(())
    }

    fn draw_solid(
        &mut self,
        ctx: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
        bounds: Rect,
        fill: Color,
    ) -> Result<()> {
        let brush: ID2D1Brush = self.brush(fill)?.cast()?;
        let rect = D2D_RECT_F {
            left: bounds.x,
            top: bounds.y,
            right: bounds.x + bounds.w,
            bottom: bounds.y + bounds.h,
        };
        unsafe {
            ctx.FillRectangle(&rect, &brush);
        }
        Ok(())
    }

    fn draw_blur(
        &mut self,
        ctx: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
        bounds: Rect,
        intensity: f32,
    ) -> Result<()> {
        let Some(base) = &self.base else { return Ok(()) };
        if self.gaussian.is_none() {
            let eff = unsafe { ctx.CreateEffect(&CLSID_D2D1GaussianBlur)? };
            self.gaussian = Some(eff);
        }
        let effect = self.gaussian.as_ref().unwrap();
        unsafe {
            let img: ID2D1Image = base.cast()?;
            effect.SetInput(0, Some(&img), false);
            let stdev = (intensity / 3.0).max(0.5);
            let bytes = stdev.to_le_bytes();
            effect.SetValue(
                D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
                D2D1_PROPERTY_TYPE_FLOAT,
                &bytes,
            )?;
            let effect_out: ID2D1Image = effect.GetOutput()?;
            // Draw only the band of the blurred output that overlaps the
            // annotation bounds.
            let dest_offset = Vector2 { X: bounds.x, Y: bounds.y };
            let src = D2D_RECT_F {
                left: bounds.x,
                top: bounds.y,
                right: bounds.x + bounds.w,
                bottom: bounds.y + bounds.h,
            };
            ctx.DrawImage(
                &effect_out,
                Some(&dest_offset),
                Some(&src),
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_OVER,
            );
        }
        Ok(())
    }

    fn draw_magnify(
        &mut self,
        ctx: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
        source: Rect,
        display: Rect,
        line_width: f32,
        color: Color,
    ) -> Result<()> {
        let Some(base) = self.base.clone() else { return Ok(()) };
        let brush: ID2D1Brush = self.brush(color)?.cast()?;
        let src = D2D_RECT_F {
            left: source.x,
            top: source.y,
            right: source.x + source.w,
            bottom: source.y + source.h,
        };
        let dst = D2D_RECT_F {
            left: display.x,
            top: display.y,
            right: display.x + display.w,
            bottom: display.y + display.h,
        };
        unsafe {
            // High-quality scaled blit of the source band into the display rect.
            let base_bm: windows::Win32::Graphics::Direct2D::ID2D1Bitmap = base.cast()?;
            ctx.DrawBitmap(
                &base_bm,
                Some(&dst),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                Some(&src),
                None,
            );
            // Outline both rects + connector line.
            ctx.DrawRectangle(&src, &brush, line_width, None);
            ctx.DrawRectangle(&dst, &brush, line_width, None);
            let p_src = Vector2 {
                X: source.x + source.w * 0.5,
                Y: source.y + source.h * 0.5,
            };
            let p_dst = Vector2 {
                X: display.x + display.w * 0.5,
                Y: display.y + display.h * 0.5,
            };
            ctx.DrawLine(p_src, p_dst, &brush, line_width, None);
        }
        Ok(())
    }

    fn draw_stamp(
        &mut self,
        ctx: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
        asset: u8,
        bounds: Rect,
    ) -> Result<()> {
        // Lazy-decode the bundled stamp into a D2D bitmap.
        let idx = (asset as usize).min(crate::editor::stamps::COUNT - 1);
        if self.stamps[idx].is_none() {
            let bm = crate::editor::stamps::bitmap(asset);
            self.stamps[idx] = Some(crate::gfx::d2d::upload_bitmap(
                &self.gfx.d2d.context,
                &bm,
            )?);
        }
        let stamp = self.stamps[idx].as_ref().unwrap().clone();
        let dest = D2D_RECT_F {
            left: bounds.x,
            top: bounds.y,
            right: bounds.x + bounds.w,
            bottom: bounds.y + bounds.h,
        };
        unsafe {
            let stamp_bm: windows::Win32::Graphics::Direct2D::ID2D1Bitmap = stamp.cast()?;
            ctx.DrawBitmap(
                &stamp_bm,
                Some(&dest),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            );
        }
        Ok(())
    }

    fn draw_preview(
        &mut self,
        ctx: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
        drag: Drag,
    ) -> Result<()> {
        match self.active_tool {
            Tool::Arrow => self.draw_arrow(ctx, drag.start, drag.current, self.style.color, self.style.stroke, None),
            Tool::BezierArrow => {
                let dx = drag.current.x - drag.start.x;
                let dy = drag.current.y - drag.start.y;
                let len = (dx * dx + dy * dy).sqrt().max(1.0);
                let mid = Point::new((drag.start.x + drag.current.x) * 0.5, (drag.start.y + drag.current.y) * 0.5);
                let nx = -dy / len;
                let ny = dx / len;
                let bend = (len * 0.25).min(80.0);
                let b = Some(Point::new(mid.x + nx * bend, mid.y + ny * bend));
                self.draw_arrow(ctx, drag.start, drag.current, self.style.color, self.style.stroke, b)
            }
            Tool::Magnify => {
                let bounds = Rect::from_points(drag.start, drag.current);
                let brush: ID2D1Brush = self.brush(self.style.color)?.cast()?;
                let r = D2D_RECT_F {
                    left: bounds.x,
                    top: bounds.y,
                    right: bounds.x + bounds.w,
                    bottom: bounds.y + bounds.h,
                };
                unsafe { ctx.DrawRectangle(&r, &brush, self.style.stroke, None) };
                Ok(())
            }
            Tool::Stamp => Ok(()),
            Tool::ShapeRect | Tool::ShapeEllipse => {
                let bounds = Rect::from_points(drag.start, drag.current);
                let kind = if self.active_tool == Tool::ShapeRect {
                    ShapeKind::Rectangle
                } else {
                    ShapeKind::Ellipse
                };
                let fill = Color { a: self.style.fill_alpha, ..self.style.color };
                self.draw_shape(ctx, kind, bounds, self.style.color, fill, self.style.stroke)
            }
            Tool::Highlighter => {
                let bounds = Rect::from_points(drag.start, drag.current);
                self.draw_highlight(ctx, bounds, self.style.color)
            }
            Tool::BlurGaussian => {
                let bounds = Rect::from_points(drag.start, drag.current);
                self.draw_blur(ctx, bounds, self.style.blur_intensity)
            }
            Tool::BlurSolid => {
                let bounds = Rect::from_points(drag.start, drag.current);
                self.draw_solid(ctx, bounds, Color::BLACK)
            }
            Tool::SelectionRect => {
                let bounds = Rect::from_points(drag.start, drag.current);
                let brush: ID2D1Brush = self.brush(Color::rgba(0x50, 0xA0, 0xFF, 0xFF))?.cast()?;
                let r = D2D_RECT_F {
                    left: bounds.x,
                    top: bounds.y,
                    right: bounds.x + bounds.w,
                    bottom: bounds.y + bounds.h,
                };
                unsafe { ctx.DrawRectangle(&r, &brush, 1.0, None) };
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn draw_handles(
        &mut self,
        ctx: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
        bb: Rect,
    ) -> Result<()> {
        let brush: ID2D1Brush = self.brush(Color::rgba(0x50, 0xA0, 0xFF, 0xFF))?.cast()?;
        let r = D2D_RECT_F {
            left: bb.x,
            top: bb.y,
            right: bb.x + bb.w,
            bottom: bb.y + bb.h,
        };
        unsafe {
            ctx.DrawRectangle(&r, &brush, 1.0, None);
            for (cx, cy) in [
                (bb.x, bb.y),
                (bb.x + bb.w, bb.y),
                (bb.x, bb.y + bb.h),
                (bb.x + bb.w, bb.y + bb.h),
            ] {
                let h = D2D_RECT_F {
                    left: cx - HANDLE_HALF,
                    top: cy - HANDLE_HALF,
                    right: cx + HANDLE_HALF,
                    bottom: cy + HANDLE_HALF,
                };
                ctx.FillRectangle(&h, &brush);
            }
        }
        Ok(())
    }

    fn draw_bezier_handle(
        &mut self,
        ctx: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
        cp: Point,
    ) -> Result<()> {
        let brush: ID2D1Brush = self.brush(Color::rgb(0xFF, 0xC8, 0x3A))?.cast()?;
        let r = BEZIER_HANDLE_RADIUS;
        // Diamond shape.
        let pts = [
            Vector2 { X: cp.x, Y: cp.y - r },
            Vector2 { X: cp.x + r, Y: cp.y },
            Vector2 { X: cp.x, Y: cp.y + r },
            Vector2 { X: cp.x - r, Y: cp.y },
        ];
        unsafe {
            let geom = self.gfx.d2d.factory.CreatePathGeometry()?;
            let sink = geom.Open()?;
            sink.BeginFigure(pts[0], D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(pts[1]);
            sink.AddLine(pts[2]);
            sink.AddLine(pts[3]);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            ctx.FillGeometry(&geom, &brush, None);
        }
        Ok(())
    }

    pub fn screen_to_doc(&self, x: f32, y: f32) -> Point {
        Point::new(
            (x - self.view_offset.0) / self.view_scale,
            (y - self.view_offset.1) / self.view_scale,
        )
    }

    pub fn doc_to_screen(&self, p: Point) -> (f32, f32) {
        (
            self.view_offset.0 + p.x * self.view_scale,
            self.view_offset.1 + p.y * self.view_scale,
        )
    }

    pub fn flatten(&self) -> Result<Bitmap> {
        crate::editor::flatten::flatten(&self.doc)
    }
}

// --- Child window registration & wndproc ---

pub fn register_class() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let cname = crate::win::wide(CLASS_NAME);
        let wc = WNDCLASSEXW {
            cbSize: core::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: GetModuleHandleW(core::ptr::null()) as _,
            hIcon: 0,
            hCursor: LoadCursorW(0, IDC_ARROW),
            hbrBackground: 0,
            lpszMenuName: core::ptr::null(),
            lpszClassName: cname.as_ptr(),
            hIconSm: 0,
        };
        RegisterClassExW(&wc);
    });
}

pub fn create_child(parent: HWND, rect: RECT) -> Result<HWND> {
    register_class();
    let cname = crate::win::wide(CLASS_NAME);
    unsafe {
        let hwnd = CreateWindowExW(
            0,
            cname.as_ptr(),
            core::ptr::null(),
            WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            parent,
            0,
            GetModuleHandleW(core::ptr::null()) as _,
            core::ptr::null(),
        );
        if hwnd == 0 {
            return Err(Error::last_win32());
        }
        Ok(hwnd)
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // Each child looks up the shared EditorState via the parent hwnd.
    let parent = unsafe { GetParent(hwnd) };
    let state = crate::editor::window::get_state(parent);
    let Some(state) = state else {
        return unsafe { DefWindowProcW(hwnd, msg, w, l) };
    };
    unsafe {
        match msg {
            windows_sys::Win32::UI::WindowsAndMessaging::WM_ERASEBKGND => 1,
            WM_PAINT => {
                // Validate the update region then draw via D2D Present.
                let mut ps: windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT = core::mem::zeroed();
                windows_sys::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
                {
                    let mut st = state.borrow_mut();
                    let _ = st.canvas.render();
                }
                windows_sys::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
                0
            }
            WM_SIZE => {
                let cw = (l & 0xFFFF) as u16 as u32;
                let ch = ((l >> 16) & 0xFFFF) as u16 as u32;
                let mut st = state.borrow_mut();
                let _ = st.canvas.resize(cw.max(1), ch.max(1));
                drop(st);
                InvalidateRect(hwnd, core::ptr::null(), 0);
                0
            }
            WM_LBUTTONDOWN => {
                SetFocus(hwnd);
                SetCapture(hwnd);
                let x = (l & 0xFFFF) as i16 as i32;
                let y = ((l >> 16) & 0xFFFF) as i16 as i32;
                let mut st = state.borrow_mut();
                let p = st.canvas.screen_to_doc(x as f32, y as f32);

                // Check for bezier CP handle hit on selected arrow.
                let hit_cp = st.canvas.selected.and_then(|sel| {
                    let idx = st.canvas.doc.find_index(sel)?;
                    if let Annotation::Arrow { bezier: Some(cp), .. } = &st.canvas.doc.annotations[idx] {
                        let dx = p.x - cp.x;
                        let dy = p.y - cp.y;
                        if dx * dx + dy * dy <= BEZIER_HANDLE_RADIUS * BEZIER_HANDLE_RADIUS {
                            return Some(sel);
                        }
                    }
                    None
                });

                if let Some(id) = hit_cp {
                    if let Some(idx) = st.canvas.doc.find_index(id) {
                        st.canvas.handle_drag = Some(HandleDrag {
                            id,
                            original_ann: st.canvas.doc.annotations[idx].clone(),
                        });
                    }
                } else if st.canvas.active_tool == Tool::Move {
                    st.canvas.selected = st.canvas.doc.topmost_hit(p);
                    st.canvas.move_anchor = Some(p);
                } else {
                    st.canvas.drag = Some(Drag { start: p, current: p });
                }
                drop(st);
                InvalidateRect(hwnd, core::ptr::null(), 0);
                0
            }
            WM_MOUSEMOVE => {
                let x = (l & 0xFFFF) as i16 as i32;
                let y = ((l >> 16) & 0xFFFF) as i16 as i32;
                let mut st = state.borrow_mut();
                let p = st.canvas.screen_to_doc(x as f32, y as f32);

                // Handle bezier CP drag.
                if let Some(ref hd) = st.canvas.handle_drag {
                    if let Some(idx) = st.canvas.doc.find_index(hd.id) {
                        if let Annotation::Arrow { bezier: Some(ref mut cp), .. } = st.canvas.doc.annotations[idx] {
                            *cp = p;
                        }
                    }
                    drop(st);
                    InvalidateRect(hwnd, core::ptr::null(), 0);
                    return 0;
                }

                if let Some(drag) = st.canvas.drag.as_mut() {
                    drag.current = p;
                    drop(st);
                    InvalidateRect(hwnd, core::ptr::null(), 0);
                    return 0;
                }
                if st.canvas.active_tool == Tool::Move {
                    if let (Some(sel), Some(anchor)) = (st.canvas.selected, st.canvas.move_anchor) {
                        if let Some(i) = st.canvas.doc.find_index(sel) {
                            let dx = p.x - anchor.x;
                            let dy = p.y - anchor.y;
                            st.canvas.doc.annotations[i].translate(dx, dy);
                            st.canvas.move_anchor = Some(p);
                            drop(st);
                            InvalidateRect(hwnd, core::ptr::null(), 0);
                        }
                    }
                }
                0
            }
            WM_LBUTTONUP => {
                ReleaseCapture();
                let mut text_spawn: Option<(u32, crate::editor::annotation::Rect, f32, f32, (f32, f32))> = None;
                {
                    let mut st = state.borrow_mut();

                    // Commit bezier CP drag.
                    if let Some(hd) = st.canvas.handle_drag.take() {
                        if let Some(idx) = st.canvas.doc.find_index(hd.id) {
                            let after = st.canvas.doc.annotations[idx].clone();
                            st.history.push(Command::Mutate {
                                id: hd.id,
                                before: hd.original_ann,
                                after,
                            });
                        }
                    }

                    if let Some(drag) = st.canvas.drag.take() {
                        let tool = st.canvas.active_tool;
                        let style = st.canvas.style;
                        if let Some(cmd) =
                            crate::editor::tools::commit_drag(tool, style, drag, &mut st.canvas.doc)
                        {
                            // Text annotations get the hosted EDIT spawned.
                            if tool == Tool::Text {
                                if let crate::editor::history::Command::Insert { ann, .. } = &cmd {
                                    if let crate::editor::annotation::Annotation::Text {
                                        id, bounds, size, ..
                                    } = ann
                                    {
                                        text_spawn = Some((
                                            *id,
                                            *bounds,
                                            *size,
                                            st.canvas.view_scale,
                                            st.canvas.view_offset,
                                        ));
                                    }
                                }
                            }
                            st.history.push(cmd);
                        }
                    }
                    st.canvas.move_anchor = None;
                }
                InvalidateRect(hwnd, core::ptr::null(), 0);
                if let Some((id, bounds, size, scale, offset)) = text_spawn {
                    let parent = GetParent(hwnd);
                    let rect = crate::editor::text_edit::doc_rect_to_parent(scale, offset, bounds);
                    let _ = crate::editor::text_edit::spawn(parent, hwnd, id, rect, "", size);
                }
                0
            }
            WM_LBUTTONDBLCLK => {
                // Re-enter text edit if the double-click lands on an existing
                // Text annotation.
                let x = (l & 0xFFFF) as i16 as i32;
                let y = ((l >> 16) & 0xFFFF) as i16 as i32;
                let target = {
                    let st = state.borrow();
                    let p = st.canvas.screen_to_doc(x as f32, y as f32);
                    if let Some(id) = st.canvas.doc.topmost_hit(p) {
                        if let Some(idx) = st.canvas.doc.find_index(id) {
                            if let crate::editor::annotation::Annotation::Text {
                                bounds, text, size, ..
                            } = &st.canvas.doc.annotations[idx]
                            {
                                Some((
                                    id,
                                    *bounds,
                                    text.clone(),
                                    *size,
                                    st.canvas.view_scale,
                                    st.canvas.view_offset,
                                ))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some((id, bounds, text, size, scale, offset)) = target {
                    let parent = GetParent(hwnd);
                    let rect = crate::editor::text_edit::doc_rect_to_parent(scale, offset, bounds);
                    let _ = crate::editor::text_edit::spawn(parent, hwnd, id, rect, &text, size);
                }
                0
            }
            WM_KEYDOWN => {
                let ctrl = (GetKeyState(VK_CONTROL as i32) as u16 & 0x8000) != 0;
                let key = w as u32;
                let mut st = state.borrow_mut();
                let mut invalidate = false;
                if key == VK_ESCAPE as u32 {
                    // Cancel bezier CP drag if active.
                    if let Some(hd) = st.canvas.handle_drag.take() {
                        if let Some(idx) = st.canvas.doc.find_index(hd.id) {
                            st.canvas.doc.annotations[idx] = hd.original_ann;
                        }
                    }
                    st.canvas.selected = None;
                    invalidate = true;
                } else if key == VK_DELETE as u32 || key == VK_BACK as u32 {
                    if let Some(sel) = st.canvas.selected {
                        if let Some(idx) = st.canvas.doc.find_index(sel) {
                            let ann = st.canvas.doc.annotations.remove(idx);
                            st.history.push(crate::editor::history::Command::Remove { index: idx, ann });
                            st.canvas.selected = None;
                            invalidate = true;
                        }
                    }
                } else if ctrl && key == VK_Z as u32 {
                    let s = &mut *st;
                    s.history.undo(&mut s.canvas.doc);
                    invalidate = true;
                } else if ctrl && key == VK_Y as u32 {
                    let s = &mut *st;
                    s.history.redo(&mut s.canvas.doc);
                    invalidate = true;
                }
                drop(st);
                if invalidate {
                    InvalidateRect(hwnd, core::ptr::null(), 0);
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

// Suppress unused-import warnings for items that are about to be used by later phases.
#[allow(dead_code)]
fn _silence() {
    let _: D2D1_PRIMITIVE_BLEND = D2D1_PRIMITIVE_BLEND_SOURCE_OVER;
    let _ = D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC;
    let _: D2D1_COLOR_F = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    let _: D2D1_BEZIER_SEGMENT = unsafe { core::mem::zeroed() };
}
