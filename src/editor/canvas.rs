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

//! D2D-rendered canvas child window.
//!
//! The canvas owns a GDI-compatible D2D target and renders through `Renderer`
//! (`editor/render.rs`) using the process-wide `ID2D1DeviceContext`. It also
//! owns the interaction state machine: object selection (click, Shift-click,
//! marquee), move / resize / rotate handles, kind-specific edit points
//! (arrow endpoints + bezier handles, polygon vertices, callout tails,
//! magnifier source), the drawing tools, the pixel Selection tool and the
//! Cut-out tool. Every document change goes through `History` as a command.

use crate::editor::annotation::{
    Annotation, AnnotationId, Document, Part, Point, Rect, ShapeKind, Tail,
};
use crate::editor::history::{self, Command, History};
use crate::editor::pixels::{self, Effect, PixelSel, SelShape};
use crate::editor::render::{self, RenderOpts, Renderer, identity, rect_f, v2};
use crate::editor::tools::{self, Tool, ToolStyle};
use crate::error::{Error, Result};
use crate::gfx::Gfx;
use crate::gfx::bitmap::Bitmap;
use crate::gfx::color::Color;
use crate::gfx::d2d::{color_f, upload_bitmap};
use std::rc::Rc;
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_BRUSH_PROPERTIES1, D2D1_CAP_STYLE_FLAT, D2D1_DASH_STYLE_DASH, D2D1_ELLIPSE,
    D2D1_EXTEND_MODE_WRAP, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_LINE_JOIN_MITER,
    D2D1_STROKE_STYLE_PROPERTIES1, ID2D1Bitmap1, ID2D1BitmapBrush1, ID2D1Brush, ID2D1DeviceContext,
    ID2D1Image, ID2D1StrokeStyle,
};
use windows::core::Interface;
use windows_numerics::Matrix3x2;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{GetDC, InvalidateRect, ReleaseDC, ScreenToClient};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN,
    VK_ESCAPE, VK_LEFT, VK_MENU, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetParent, HTCLIENT, IDC_ARROW, IDC_CROSS, IDC_HAND,
    IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, LoadCursorW, PostMessageW,
    RegisterClassExW, SetCursor, WM_CAPTURECHANGED, WM_COMMAND, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WM_SIZE, WNDCLASSEXW, CS_DBLCLKS,
    WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_VISIBLE,
};

const CLASS_NAME: &str = "frame0.editor.canvas";
/// Handle half-size in screen pixels.
const HANDLE: f32 = 5.0;
const ROT_OFFSET: f32 = 28.0;
const ACCENT: Color = Color::rgb(0x3D, 0x8B, 0xFF);
const MARGIN: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Handle {
    Frame(u8),
    Rotate,
    Edit(usize),
}

enum Gesture {
    Idle,
    Pan { last: (f32, f32) },
    Create { start: Point, cur: Point },
    Polygon { pts: Vec<Point>, hover: Point },
    Freehand { pts: Vec<Point> },
    Marquee { start: Point, cur: Point, additive: bool },
    Move { start: Point, last: Point, part: Part, originals: Vec<Annotation>, moved: bool },
    Resize { handle: u8, frame0: Rect, rot: f32, originals: Vec<Annotation> },
    Rotate { center: Point, original: Annotation, angle0: f32 },
    EditPoint { idx: usize, original: Annotation },
    SelRect { start: Point, cur: Point },
    Band { start: Point, cur: Point },
    PendingLift { start: Point },
}

/// Text-edit request handed back to the wndproc (spawned after the borrow ends).
pub struct TextEditReq {
    pub id: AnnotationId,
    pub rect: Rect,
    pub text: String,
    pub size: f32,
}

pub struct Canvas {
    pub gfx: Rc<Gfx>,
    pub doc: Document,
    pub history: History,
    pub selected: Vec<AnnotationId>,
    pub pixel_sel: Option<PixelSel>,
    pub view_scale: f32,
    pub view_offset: (f32, f32),
    pub size: (u32, u32),
    pub active_tool: Tool,
    pub style: ToolStyle,
    pub editing: Option<AnnotationId>,
    pub dirty: bool,
    pub hwnd: HWND,
    gesture: Gesture,
    cursor: *const u16,
    space_down: bool,
    clip: Vec<Annotation>,
    clip_seq: u32,
    /// True once the user zooms/pans; until then resizes re-fit the image.
    user_view: bool,

    renderer: Renderer,
    /// GDI-compatible offscreen target, blitted to the window on paint (see
    /// `D2D::create_gdi_target` for why this isn't a swap chain).
    target: Option<ID2D1Bitmap1>,
    checker: Option<ID2D1BitmapBrush1>,
    dash: Option<ID2D1StrokeStyle>,
}

fn key_down(vk: u16) -> bool {
    unsafe { (GetKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

impl Canvas {
    pub fn create(gfx: Rc<Gfx>, doc: Document) -> Result<Self> {
        Ok(Self {
            gfx,
            doc,
            history: History::new(),
            selected: Vec::new(),
            pixel_sel: None,
            view_scale: 1.0,
            view_offset: (MARGIN, MARGIN),
            size: (1024, 768),
            active_tool: Tool::Arrow,
            style: ToolStyle::default(),
            editing: None,
            dirty: false,
            hwnd: 0,
            gesture: Gesture::Idle,
            cursor: IDC_ARROW,
            space_down: false,
            clip: Vec::new(),
            clip_seq: 0,
            user_view: false,
            renderer: Renderer::new(),
            target: None,
            checker: None,
            dash: None,
        })
    }

    pub fn attach(&mut self, hwnd: HWND, w: u32, h: u32) -> Result<()> {
        self.hwnd = hwnd;
        self.size = (w.max(1), h.max(1));
        self.target = Some(self.gfx.d2d.create_gdi_target(self.size.0, self.size.1)?);
        self.zoom_fit();
        Ok(())
    }

    pub fn resize(&mut self, w: u32, h: u32) -> Result<()> {
        let new = (w.max(1), h.max(1));
        if new == self.size {
            return Ok(());
        }
        self.size = new;
        if !self.user_view {
            self.zoom_fit();
        } else {
            self.clamp_view();
        }
        if self.target.is_none() {
            return Ok(());
        }
        self.target = None;
        self.target = Some(self.gfx.d2d.create_gdi_target(new.0, new.1)?);
        Ok(())
    }

    pub fn replace_document(&mut self, doc: Document) -> Result<()> {
        self.doc = doc;
        self.selected.clear();
        self.pixel_sel = None;
        self.gesture = Gesture::Idle;
        self.history.clear();
        self.dirty = false;
        self.zoom_fit();
        Ok(())
    }

    // ---- view ----

    pub fn world(&self) -> Matrix3x2 {
        Matrix3x2 {
            M11: self.view_scale,
            M12: 0.0,
            M21: 0.0,
            M22: self.view_scale,
            M31: self.view_offset.0,
            M32: self.view_offset.1,
        }
    }

    pub fn screen_to_doc(&self, x: f32, y: f32) -> Point {
        Point::new((x - self.view_offset.0) / self.view_scale, (y - self.view_offset.1) / self.view_scale)
    }

    pub fn doc_to_screen(&self, p: Point) -> Point {
        Point::new(self.view_offset.0 + p.x * self.view_scale, self.view_offset.1 + p.y * self.view_scale)
    }

    pub fn zoom_fit(&mut self) {
        let (cw, ch) = (self.size.0 as f32, self.size.1 as f32);
        let (w, h) = (self.doc.base.width as f32, self.doc.base.height as f32);
        let s = ((cw - 2.0 * MARGIN) / w).min((ch - 2.0 * MARGIN) / h).min(1.0).max(0.05);
        self.view_scale = s;
        self.view_offset = ((cw - w * s) * 0.5, (ch - h * s) * 0.5);
        self.user_view = false;
        self.clamp_view();
    }

    /// Zoom to `scale` keeping the document point under screen `(sx, sy)` fixed.
    pub fn zoom_to(&mut self, scale: f32, sx: f32, sy: f32) {
        let scale = scale.clamp(0.05, 32.0);
        let p = self.screen_to_doc(sx, sy);
        self.view_scale = scale;
        self.view_offset = (sx - p.x * scale, sy - p.y * scale);
        self.user_view = true;
        self.clamp_view();
    }

    pub fn zoom_step(&mut self, dir: f32) {
        let steps = [0.1, 0.25, 0.33, 0.5, 0.67, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0];
        let cur = self.view_scale;
        let next = if dir > 0.0 {
            steps.iter().copied().find(|s| *s > cur * 1.01).unwrap_or(16.0)
        } else {
            steps.iter().rev().copied().find(|s| *s < cur * 0.99).unwrap_or(0.1)
        };
        self.zoom_to(next, self.size.0 as f32 * 0.5, self.size.1 as f32 * 0.5);
    }

    fn clamp_view(&mut self) {
        let (cw, ch) = (self.size.0 as f32, self.size.1 as f32);
        let w = self.doc.base.width as f32 * self.view_scale;
        let h = self.doc.base.height as f32 * self.view_scale;
        let fix = |off: f32, content: f32, view: f32| {
            if content + 2.0 * MARGIN <= view {
                (view - content) * 0.5
            } else {
                off.min(MARGIN).max(view - content - MARGIN)
            }
        };
        self.view_offset = (fix(self.view_offset.0, w, cw), fix(self.view_offset.1, h, ch));
    }

    fn tol(&self) -> f32 {
        4.0 / self.view_scale
    }

    // ---- rendering ----

    pub fn render(&mut self) -> Result<()> {
        if self.target.is_none() {
            return Ok(());
        }
        match self.render_inner() {
            Ok(()) => Ok(()),
            Err(Error::Hr(h)) if h == D2DERR_RECREATE_TARGET.0 => {
                self.target = None;
                self.checker = None;
                self.dash = None;
                self.renderer.clear();
                // Rebuild in place so the next paint works.
                if self.hwnd != 0 {
                    let (w, h) = self.size;
                    let _ = self.attach(self.hwnd, w, h);
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn ensure_ui_resources(&mut self, ctx: &ID2D1DeviceContext) -> Result<()> {
        if self.checker.is_none() {
            let mut bm = Bitmap::new(16, 16);
            for y in 0..16u32 {
                for x in 0..16u32 {
                    let v = if (x / 8 + y / 8) % 2 == 0 { 0xFF } else { 0xD8 };
                    let i = ((y * 16 + x) * 4) as usize;
                    bm.bgra[i..i + 4].copy_from_slice(&[v, v, v, 0xFF]);
                }
            }
            let up = upload_bitmap(ctx, &bm)?;
            let props = D2D1_BITMAP_BRUSH_PROPERTIES1 {
                extendModeX: D2D1_EXTEND_MODE_WRAP,
                extendModeY: D2D1_EXTEND_MODE_WRAP,
                interpolationMode: D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            };
            self.checker = Some(unsafe { ctx.CreateBitmapBrush(&up, Some(&props), None)? });
        }
        if self.dash.is_none() {
            let props = D2D1_STROKE_STYLE_PROPERTIES1 {
                startCap: D2D1_CAP_STYLE_FLAT,
                endCap: D2D1_CAP_STYLE_FLAT,
                dashCap: D2D1_CAP_STYLE_FLAT,
                lineJoin: D2D1_LINE_JOIN_MITER,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_DASH,
                dashOffset: 0.0,
                transformType: windows::Win32::Graphics::Direct2D::D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
            };
            self.dash = Some(unsafe { self.gfx.d2d.factory.CreateStrokeStyle(&props, None)?.into() });
        }
        Ok(())
    }

    fn render_inner(&mut self) -> Result<()> {
        let gfx = self.gfx.clone();
        let ctx = gfx.d2d.context.clone();
        let target = self.target.clone().ok_or(Error::msg("no D2D target"))?;
        self.ensure_ui_resources(&ctx)?;
        let world = self.world();
        unsafe {
            let target_img: ID2D1Image = target.cast()?;
            ctx.SetTarget(&target_img);
            ctx.BeginDraw();
            ctx.Clear(Some(&color_f(Color::rgb(0x16, 0x17, 0x19))));
            ctx.SetTransform(&world);
            let page = self.doc.canvas_rect();
            if let Some(ch) = &self.checker {
                let b: ID2D1Brush = ch.cast()?;
                ctx.SetTransform(&identity());
                let s0 = self.doc_to_screen(Point::new(0.0, 0.0));
                let s1 = self.doc_to_screen(Point::new(page.w, page.h));
                ctx.FillRectangle(&D2D_RECT_F { left: s0.x, top: s0.y, right: s1.x, bottom: s1.y }, &b);
                ctx.SetTransform(&world);
            }
        }
        let r = self.renderer.draw_document(&gfx, &ctx, &self.doc, &world, RenderOpts { secure: false, skip: self.editing });
        let r = r.and_then(|_| self.draw_overlay(&gfx, &ctx));
        unsafe {
            ctx.SetTransform(&identity());
            let r = r.and_then(|_| {
                let hdc = GetDC(self.hwnd);
                let b = gfx.d2d.blit(hdc, self.size.0, self.size.1);
                ReleaseDC(self.hwnd, hdc);
                b
            });
            let end = ctx.EndDraw(None, None);
            ctx.SetTarget(None);
            r?;
            end?;
        }
        Ok(())
    }

    /// Selection handles, gesture previews, pixel-selection outline.
    fn draw_overlay(&mut self, gfx: &Gfx, ctx: &ID2D1DeviceContext) -> Result<()> {
        let world = self.world();
        let accent = self.renderer.brush(ctx, ACCENT)?;
        let white = self.renderer.brush(ctx, Color::WHITE)?;
        let black = self.renderer.brush(ctx, Color::rgb(0x10, 0x10, 0x10))?;
        let dash = self.dash.clone();

        // Gesture previews (document space).
        unsafe { ctx.SetTransform(&world) };
        match &self.gesture {
            Gesture::Create { start, cur } => {
                let end = tools::constrain(self.active_tool, *start, *cur, key_down(VK_SHIFT));
                if let Some(a) = tools::create_from_drag(self.active_tool, &self.style, *start, end, &self.doc) {
                    let base = self.renderer.base_bitmap(ctx, &self.doc)?;
                    if matches!(a, Annotation::Text { .. }) {
                        let r = a.frame();
                        unsafe { ctx.DrawRectangle(&rect_f(r), &accent, 1.0 / self.view_scale, dash.as_ref()) };
                    } else {
                        self.renderer.draw_annotation(gfx, ctx, &self.doc, &base, &a, &world, RenderOpts::default())?;
                    }
                }
            }
            Gesture::Polygon { pts, hover } => {
                let mut all = pts.clone();
                all.push(*hover);
                if self.active_tool == Tool::Polygon {
                    let ob = self.renderer.brush(ctx, self.style.color)?;
                    for w in all.windows(2) {
                        unsafe { ctx.DrawLine(v2(w[0]), v2(w[1]), &ob, self.style.stroke, None) };
                    }
                } else {
                    for w in all.windows(2) {
                        unsafe {
                            ctx.DrawLine(v2(w[0]), v2(w[1]), &black, 1.0 / self.view_scale, None);
                            ctx.DrawLine(v2(w[0]), v2(w[1]), &white, 1.0 / self.view_scale, dash.as_ref());
                        }
                    }
                }
                if let Some(first) = pts.first() {
                    let r = 5.0 / self.view_scale;
                    unsafe { ctx.DrawEllipse(&D2D1_ELLIPSE { point: v2(*first), radiusX: r, radiusY: r }, &accent, 1.5 / self.view_scale, None) };
                }
            }
            Gesture::Freehand { pts } => {
                for w in pts.windows(2) {
                    unsafe {
                        ctx.DrawLine(v2(w[0]), v2(w[1]), &black, 1.0 / self.view_scale, None);
                        ctx.DrawLine(v2(w[0]), v2(w[1]), &white, 1.0 / self.view_scale, dash.as_ref());
                    }
                }
            }
            Gesture::Marquee { start, cur, .. } => {
                let r = Rect::from_points(*start, *cur);
                let fill = self.renderer.brush(ctx, Color::rgba(0x3D, 0x8B, 0xFF, 0x30))?;
                unsafe {
                    ctx.FillRectangle(&rect_f(r), &fill);
                    ctx.DrawRectangle(&rect_f(r), &accent, 1.0 / self.view_scale, None);
                }
            }
            Gesture::SelRect { start, cur } => {
                let end = tools::constrain(Tool::Select, *start, *cur, key_down(VK_SHIFT));
                let sel = PixelSel::from_rect(self.style.sel_shape, Rect::from_points(*start, end));
                self.draw_ants(ctx, &sel.outline, &black, &white, dash.as_ref())?;
            }
            Gesture::Band { start, cur } => {
                let page = self.doc.canvas_rect();
                let band = if self.style.cut_horizontal {
                    let (a, b) = (start.y.min(cur.y), start.y.max(cur.y));
                    Rect { x: page.x, y: a, w: page.w, h: b - a }
                } else {
                    let (a, b) = (start.x.min(cur.x), start.x.max(cur.x));
                    Rect { x: a, y: page.y, w: b - a, h: page.h }
                };
                let fill = self.renderer.brush(ctx, Color::rgba(0xE5, 0x3E, 0x3E, 0x50))?;
                let red = self.renderer.brush(ctx, Color::rgb(0xE5, 0x3E, 0x3E))?;
                unsafe {
                    ctx.FillRectangle(&rect_f(band), &fill);
                    ctx.DrawRectangle(&rect_f(band), &red, 1.5 / self.view_scale, dash.as_ref());
                }
            }
            _ => {}
        }

        if let Some(sel) = &self.pixel_sel {
            let outline = sel.outline.clone();
            self.draw_ants(ctx, &outline, &black, &white, dash.as_ref())?;
            if sel.inverted {
                let page = self.doc.canvas_rect();
                unsafe {
                    ctx.DrawRectangle(&rect_f(page), &black, 1.0 / self.view_scale, None);
                    ctx.DrawRectangle(&rect_f(page), &white, 1.0 / self.view_scale, dash.as_ref());
                }
            }
        }

        // Selection frames + handles, in screen space.
        unsafe { ctx.SetTransform(&identity()) };
        let sel: Vec<Annotation> = self.selected.iter().filter_map(|id| self.doc.get(*id).cloned()).collect();
        for a in &sel {
            let f = a.frame();
            let c = f.center();
            let r = a.rotation();
            let pts: Vec<Point> = f.corners().iter().map(|p| self.doc_to_screen(p.rotated(c, r))).collect();
            if a.has_frame() {
                for i in 0..4 {
                    unsafe { ctx.DrawLine(v2(pts[i]), v2(pts[(i + 1) % 4]), &accent, 1.0, None) };
                }
            }
            if let Annotation::Magnify { source, .. } = a {
                let s0 = self.doc_to_screen(Point::new(source.x, source.y));
                let s1 = self.doc_to_screen(Point::new(source.right(), source.bottom()));
                unsafe { ctx.DrawRectangle(&D2D_RECT_F { left: s0.x, top: s0.y, right: s1.x, bottom: s1.y }, &accent, 1.0, dash.as_ref()) };
            }
            if let Annotation::Arrow { start, end, bezier: Some((c1, c2)), .. } = a {
                let (s, e, a1, a2) = (self.doc_to_screen(*start), self.doc_to_screen(*end), self.doc_to_screen(*c1), self.doc_to_screen(*c2));
                unsafe {
                    ctx.DrawLine(v2(s), v2(a1), &accent, 1.0, dash.as_ref());
                    ctx.DrawLine(v2(e), v2(a2), &accent, 1.0, dash.as_ref());
                }
            }
        }
        if sel.len() > 1 {
            if let Some(u) = self.selection_frame() {
                let s0 = self.doc_to_screen(Point::new(u.x, u.y));
                let s1 = self.doc_to_screen(Point::new(u.right(), u.bottom()));
                unsafe { ctx.DrawRectangle(&D2D_RECT_F { left: s0.x, top: s0.y, right: s1.x, bottom: s1.y }, &accent, 1.0, dash.as_ref()) };
            }
        }
        let editing_handles = !matches!(self.gesture, Gesture::Move { moved: true, .. });
        if editing_handles {
            for (h, p) in self.handles() {
                match h {
                    Handle::Frame(_) => unsafe {
                        let r = D2D_RECT_F { left: p.x - HANDLE, top: p.y - HANDLE, right: p.x + HANDLE, bottom: p.y + HANDLE };
                        ctx.FillRectangle(&r, &white);
                        ctx.DrawRectangle(&r, &accent, 1.5, None);
                    },
                    Handle::Rotate => unsafe {
                        let e = D2D1_ELLIPSE { point: v2(p), radiusX: HANDLE + 1.0, radiusY: HANDLE + 1.0 };
                        ctx.FillEllipse(&e, &white);
                        ctx.DrawEllipse(&e, &accent, 1.5, None);
                    },
                    Handle::Edit(_) => unsafe {
                        let yellow = self.renderer.brush(ctx, Color::rgb(0xFF, 0xC8, 0x3A))?;
                        let e = D2D1_ELLIPSE { point: v2(p), radiusX: HANDLE + 0.5, radiusY: HANDLE + 0.5 };
                        ctx.FillEllipse(&e, &yellow);
                        ctx.DrawEllipse(&e, &black, 1.0, None);
                    },
                }
            }
        }
        Ok(())
    }

    fn draw_ants(&self, ctx: &ID2D1DeviceContext, outline: &[Point], black: &ID2D1Brush, white: &ID2D1Brush, dash: Option<&ID2D1StrokeStyle>) -> Result<()> {
        if outline.len() < 2 {
            return Ok(());
        }
        let n = outline.len();
        let w = 1.0 / self.view_scale;
        for i in 0..n {
            let (a, b) = (outline[i], outline[(i + 1) % n]);
            unsafe {
                ctx.DrawLine(v2(a), v2(b), black, w, None);
                ctx.DrawLine(v2(a), v2(b), white, w, dash);
            }
        }
        Ok(())
    }

    // ---- selection helpers ----

    pub fn single(&self) -> Option<&Annotation> {
        if self.selected.len() == 1 { self.doc.get(self.selected[0]) } else { None }
    }

    pub fn selection_frame(&self) -> Option<Rect> {
        let mut u: Option<Rect> = None;
        for id in &self.selected {
            if let Some(a) = self.doc.get(*id) {
                let b = a.bounding_box();
                u = Some(match u {
                    Some(r) => r.union(&b),
                    None => b,
                });
            }
        }
        u
    }

    pub fn select(&mut self, ids: &[AnnotationId]) {
        self.selected = self.doc.expand_groups(ids);
    }

    /// Screen-space handle positions for the current selection.
    pub fn handles(&self) -> Vec<(Handle, Point)> {
        let mut out = Vec::new();
        if matches!(self.active_tool, Tool::Select | Tool::CutOut) {
            return out;
        }
        let (frame, rot, rotatable, edits) = if let Some(a) = self.single() {
            if self.doc.groups.contains_key(&a.id()) {
                return out;
            }
            (if a.has_frame() { Some(a.frame()) } else { None }, a.rotation(), a.rotatable(), a.edit_points())
        } else if self.selected.len() > 1 {
            (self.selection_frame(), 0.0, false, Vec::new())
        } else {
            return out;
        };
        if let Some(f) = frame {
            let c = f.center();
            for i in 0..8u8 {
                out.push((Handle::Frame(i), self.doc_to_screen(frame_point(f, i).rotated(c, rot))));
            }
            if rotatable {
                let top = Point::new(c.x, f.y - ROT_OFFSET / self.view_scale);
                out.push((Handle::Rotate, self.doc_to_screen(top.rotated(c, rot))));
            }
        }
        for (i, p) in edits.iter().enumerate() {
            out.push((Handle::Edit(i), self.doc_to_screen(*p)));
        }
        out
    }

    fn handle_at(&self, sx: f32, sy: f32) -> Option<Handle> {
        // Edit points win over frame handles (they're usually what you want).
        let hs = self.handles();
        let near = |p: &Point| (p.x - sx).abs() <= HANDLE + 3.0 && (p.y - sy).abs() <= HANDLE + 3.0;
        hs.iter().find(|(h, p)| matches!(h, Handle::Edit(_)) && near(p))
            .or_else(|| hs.iter().find(|(_, p)| near(p)))
            .map(|(h, _)| *h)
    }

    fn cursor_for(&self, sx: f32, sy: f32, p: Point) -> *const u16 {
        if self.space_down || matches!(self.gesture, Gesture::Pan { .. }) {
            return IDC_HAND;
        }
        if let Some(h) = self.handle_at(sx, sy) {
            return match h {
                Handle::Frame(i) => {
                    let rot = self.single().map(|a| a.rotation()).unwrap_or(0.0);
                    let octant = ((i as f32 * 45.0 + rot + 22.5).rem_euclid(180.0) / 45.0) as u8;
                    match octant {
                        0 => IDC_SIZENWSE,
                        1 => IDC_SIZENS,
                        2 => IDC_SIZENESW,
                        _ => IDC_SIZEWE,
                    }
                }
                Handle::Rotate => IDC_HAND,
                Handle::Edit(_) => IDC_CROSS,
            };
        }
        match self.active_tool {
            Tool::Move => {
                if self.doc.topmost_hit(p, self.tol()).is_some() { IDC_SIZEALL } else { IDC_ARROW }
            }
            Tool::Select => {
                if self.pixel_sel.as_ref().map(|s| s.mask_contains(p)).unwrap_or(false) { IDC_SIZEALL } else { IDC_CROSS }
            }
            _ => {
                let over_sel = self.selected.iter().any(|id| self.doc.get(*id).map(|a| a.hit_test(p, self.tol())).unwrap_or(false));
                if over_sel { IDC_SIZEALL } else { IDC_CROSS }
            }
        }
    }

    // ---- commands ----

    fn exec(&mut self, cmd: Command) {
        history::execute(&mut self.doc, &mut self.history, cmd);
        self.dirty = true;
    }

    fn insert_new(&mut self, mut a: Annotation) -> AnnotationId {
        let id = self.doc.alloc_id();
        a.set_id(id);
        let index = self.doc.annotations.len();
        self.exec(Command::Insert { index, ann: a });
        id
    }

    fn prune_selection(&mut self) {
        let doc = &self.doc;
        self.selected.retain(|id| doc.find_index(*id).is_some());
    }

    pub fn undo(&mut self) -> bool {
        self.cancel_gesture();
        let r = self.history.undo(&mut self.doc);
        self.prune_selection();
        self.dirty |= r;
        r
    }

    pub fn redo(&mut self) -> bool {
        self.cancel_gesture();
        let r = self.history.redo(&mut self.doc);
        self.prune_selection();
        self.dirty |= r;
        r
    }

    pub fn delete_selection(&mut self) -> bool {
        if !self.selected.is_empty() {
            let cmd = history::remove_many(&self.doc, &self.selected);
            self.exec(cmd);
            self.selected.clear();
            return true;
        }
        if self.pixel_sel.is_some() {
            return self.sel_fill(self.style.sel_autofill);
        }
        false
    }

    pub fn select_all(&mut self) {
        if self.active_tool == Tool::Select {
            self.pixel_sel = Some(PixelSel::from_rect(SelShape::Rectangle, self.doc.canvas_rect()));
        } else {
            self.selected = self.doc.annotations.iter().map(|a| a.id()).collect();
        }
    }

    pub fn group(&mut self) -> bool {
        if self.selected.len() < 2 {
            return false;
        }
        let before = self.doc.groups.clone();
        let mut after = before.clone();
        let g = after.values().copied().max().unwrap_or(0) + 1;
        for id in &self.selected {
            after.insert(*id, g);
        }
        self.exec(Command::Groups { before, after });
        true
    }

    pub fn ungroup(&mut self) -> bool {
        let before = self.doc.groups.clone();
        let mut after = before.clone();
        for id in &self.selected {
            after.remove(id);
        }
        if after.len() == before.len() {
            return false;
        }
        self.exec(Command::Groups { before, after });
        true
    }

    /// Reorder the selection: `to_end` true = front, `step` true = one place.
    pub fn reorder(&mut self, front: bool, step: bool) -> bool {
        if self.selected.is_empty() {
            return false;
        }
        let n = self.doc.annotations.len();
        let mut idx: Vec<usize> = self.selected.iter().filter_map(|id| self.doc.find_index(*id)).collect();
        idx.sort_unstable();
        let mut order: Vec<Annotation> = self.doc.annotations.clone();
        let picked: Vec<Annotation> = idx.iter().map(|&i| order[i].clone()).collect();
        for &i in idx.iter().rev() {
            order.remove(i);
        }
        let insert_at = if step {
            if front { (idx[idx.len() - 1] + 2 - idx.len()).min(order.len()) } else { idx[0].saturating_sub(1) }
        } else if front {
            order.len()
        } else {
            0
        };
        for (k, a) in picked.into_iter().enumerate() {
            order.insert(insert_at + k, a);
        }
        if order.iter().map(|a| a.id()).eq(self.doc.annotations.iter().map(|a| a.id())) {
            return false;
        }
        // Express as remove-all + insert-all so undo restores the order.
        let mut cmds = vec![history::remove_many(&self.doc, &self.doc.annotations.iter().map(|a| a.id()).collect::<Vec<_>>())];
        for (i, a) in order.into_iter().enumerate() {
            cmds.push(Command::Insert { index: i, ann: a });
        }
        let _ = n;
        self.exec(Command::Batch(cmds));
        true
    }

    pub fn duplicate(&mut self) -> bool {
        if self.selected.is_empty() {
            return false;
        }
        let items: Vec<Annotation> = self.selected.iter().filter_map(|id| self.doc.get(*id).cloned()).collect();
        self.paste_annotations(items, 16.0);
        true
    }

    fn paste_annotations(&mut self, items: Vec<Annotation>, offset: f32) {
        let mut cmds = Vec::new();
        let mut ids = Vec::new();
        let mut index = self.doc.annotations.len();
        for mut a in items {
            let id = self.doc.alloc_id();
            a.set_id(id);
            a.translate(offset, offset);
            ids.push(id);
            cmds.push(Command::Insert { index, ann: a });
            index += 1;
        }
        self.exec(Command::Batch(cmds));
        self.selected = ids;
    }

    /// Copy: annotations go to the internal clipboard (plus a flattened image
    /// of their area to the system clipboard); a pixel selection copies its
    /// pixels.
    pub fn copy(&mut self) -> bool {
        if let Some(sel) = &self.pixel_sel {
            if let Some((bm, _, _)) = pixels::extract(&self.doc.base, sel) {
                let _ = put_clipboard_image(self.hwnd, &bm);
                return true;
            }
            return false;
        }
        if self.selected.is_empty() {
            return false;
        }
        self.clip = self.selected.iter().filter_map(|id| self.doc.get(*id).cloned()).collect();
        if let Some(r) = self.selection_frame() {
            if let Ok(full) = render::flatten(&self.gfx, &self.doc, RenderOpts::default()) {
                let bm = pixels::crop(&full, r.x.floor() as i32, r.y.floor() as i32, r.right().ceil() as i32, r.bottom().ceil() as i32);
                let _ = put_clipboard_image(self.hwnd, &bm);
            }
        }
        self.clip_seq = clipboard_seq();
        true
    }

    pub fn cut(&mut self) -> bool {
        if !self.copy() {
            return false;
        }
        if self.pixel_sel.is_some() {
            self.sel_fill(self.style.sel_autofill)
        } else {
            self.delete_selection()
        }
    }

    pub fn paste(&mut self) -> bool {
        if !self.clip.is_empty() && clipboard_seq() == self.clip_seq {
            self.paste_annotations(self.clip.clone(), 16.0);
            if self.active_tool == Tool::Select {
                self.active_tool = Tool::Move;
            }
            return true;
        }
        if let Some(bm) = crate::win::clipboard::read_image(self.hwnd) {
            self.paste_image(bm);
            return true;
        }
        false
    }

    /// Insert a raster as a movable Image layer centred in the view.
    pub fn paste_image(&mut self, bm: Bitmap) {
        let c = self.screen_to_doc(self.size.0 as f32 * 0.5, self.size.1 as f32 * 0.5);
        let (w, h) = (bm.width as f32, bm.height as f32);
        let a = Annotation::Image { id: 0, bounds: Rect { x: (c.x - w * 0.5).round(), y: (c.y - h * 0.5).round(), w, h }, bitmap: Rc::new(bm), rotation: 0.0 };
        let id = self.insert_new(a);
        self.pixel_sel = None;
        self.selected = vec![id];
        self.active_tool = Tool::Move;
    }

    pub fn nudge(&mut self, dx: f32, dy: f32) -> bool {
        if self.selected.is_empty() {
            return false;
        }
        let mut cmds = Vec::new();
        for id in self.selected.clone() {
            if let Some(i) = self.doc.find_index(id) {
                let before = self.doc.annotations[i].clone();
                let mut after = before.clone();
                after.translate(dx, dy);
                cmds.push(Command::Mutate { id, before, after });
            }
        }
        self.exec(Command::Batch(cmds));
        true
    }

    /// Mutate the current selection with `f` as one undo step.
    pub fn mutate_selected(&mut self, f: &dyn Fn(&mut Annotation) -> bool) -> bool {
        let mut cmds = Vec::new();
        for id in self.selected.clone() {
            if let Some(i) = self.doc.find_index(id) {
                let before = self.doc.annotations[i].clone();
                let mut after = before.clone();
                if f(&mut after) {
                    cmds.push(Command::Mutate { id, before, after });
                }
            }
        }
        if cmds.is_empty() {
            return false;
        }
        self.exec(Command::Batch(cmds));
        true
    }

    /// Bake every annotation into the base image (editor "flatten()").
    pub fn flatten_annotations(&mut self) -> bool {
        if self.doc.annotations.is_empty() {
            return false;
        }
        let Ok(flat) = render::flatten(&self.gfx, &self.doc, RenderOpts::default()) else { return false };
        let ids: Vec<AnnotationId> = self.doc.annotations.iter().map(|a| a.id()).collect();
        let mut cmds = vec![history::remove_many(&self.doc, &ids)];
        cmds.push(Command::Base { before: self.doc.base.clone(), after: Rc::new(flat) });
        if !self.doc.groups.is_empty() {
            cmds.push(Command::Groups { before: self.doc.groups.clone(), after: Default::default() });
        }
        self.exec(Command::Batch(cmds));
        self.selected.clear();
        true
    }

    // ---- pixel selection ops ----

    pub fn sel_fill(&mut self, auto: bool) -> bool {
        let Some(sel) = self.pixel_sel.clone() else { return false };
        let out = pixels::fill_selection(&self.doc.base, &sel, auto);
        self.exec(Command::Base { before: self.doc.base.clone(), after: Rc::new(out) });
        true
    }

    pub fn sel_effect(&mut self, fx: Effect) -> bool {
        let Some(sel) = self.pixel_sel.clone() else { return false };
        let out = pixels::apply_effect(&self.doc.base, &sel, fx);
        self.exec(Command::Base { before: self.doc.base.clone(), after: Rc::new(out) });
        true
    }

    pub fn sel_invert(&mut self) -> bool {
        match self.pixel_sel.as_mut() {
            Some(s) => {
                s.inverted = !s.inverted;
                true
            }
            None => false,
        }
    }

    pub fn sel_crop(&mut self) -> bool {
        let Some(sel) = self.pixel_sel.clone() else { return false };
        if sel.inverted {
            return false;
        }
        let (x0, y0, x1, y1) = sel.pixel_bounds(self.doc.base.width, self.doc.base.height);
        if x1 - x0 < 1 || y1 - y0 < 1 {
            return false;
        }
        let out = pixels::crop(&self.doc.base, x0, y0, x1, y1);
        let mut cmds = vec![Command::Base { before: self.doc.base.clone(), after: Rc::new(out) }];
        for a in &self.doc.annotations {
            let mut after = a.clone();
            after.translate(-x0 as f32, -y0 as f32);
            cmds.push(Command::Mutate { id: a.id(), before: a.clone(), after });
        }
        self.exec(Command::Batch(cmds));
        self.pixel_sel = None;
        self.zoom_fit();
        true
    }

    /// Lift the selected pixels into a movable Image layer, filling the hole.
    fn sel_lift(&mut self) -> Option<AnnotationId> {
        let sel = self.pixel_sel.clone()?;
        let (bm, x, y) = pixels::extract(&self.doc.base, &sel)?;
        let filled = pixels::fill_selection(&self.doc.base, &sel, self.style.sel_autofill);
        let id = self.doc.alloc_id();
        let a = Annotation::Image { id, bounds: Rect { x: x as f32, y: y as f32, w: bm.width as f32, h: bm.height as f32 }, bitmap: Rc::new(bm), rotation: 0.0 };
        let index = self.doc.annotations.len();
        self.exec(Command::Batch(vec![
            Command::Base { before: self.doc.base.clone(), after: Rc::new(filled) },
            Command::Insert { index, ann: a },
        ]));
        self.pixel_sel = None;
        Some(id)
    }

    fn apply_cut(&mut self, a: f32, b: f32) {
        let (a, b) = (a.min(b).round() as i32, a.max(b).round() as i32);
        if b - a < 2 {
            return;
        }
        let horizontal = self.style.cut_horizontal;
        let out = pixels::cut_band(&self.doc.base, horizontal, a, b, self.style.cut_edge);
        let removed = (b - a) as f32;
        let mut cmds = vec![Command::Base { before: self.doc.base.clone(), after: Rc::new(out) }];
        for ann in &self.doc.annotations {
            let c = ann.frame().center();
            let pos = if horizontal { c.y } else { c.x };
            if pos >= b as f32 {
                let mut after = ann.clone();
                if horizontal { after.translate(0.0, -removed) } else { after.translate(-removed, 0.0) }
                cmds.push(Command::Mutate { id: ann.id(), before: ann.clone(), after });
            }
        }
        self.exec(Command::Batch(cmds));
        self.clamp_view();
    }

    /// Snap a point to annotation frame edges and the canvas edges.
    fn snap(&self, p: Point) -> Point {
        if !self.style.snap {
            return p;
        }
        let lim = 8.0 / self.view_scale;
        let mut xs = vec![0.0, self.doc.base.width as f32];
        let mut ys = vec![0.0, self.doc.base.height as f32];
        for a in &self.doc.annotations {
            let b = a.bounding_box();
            xs.push(b.x);
            xs.push(b.right());
            ys.push(b.y);
            ys.push(b.bottom());
        }
        let pick = |v: f32, cands: &[f32]| {
            cands.iter().copied().filter(|c| (c - v).abs() <= lim).min_by(|a, b| (a - v).abs().partial_cmp(&(b - v).abs()).unwrap()).unwrap_or(v)
        };
        Point::new(pick(p.x, &xs), pick(p.y, &ys))
    }

    pub fn cancel_gesture(&mut self) {
        match std::mem::replace(&mut self.gesture, Gesture::Idle) {
            Gesture::Move { originals, moved: true, .. } | Gesture::Resize { originals, .. } => {
                for o in originals {
                    if let Some(i) = self.doc.find_index(o.id()) {
                        self.doc.annotations[i] = o;
                    }
                }
            }
            Gesture::Rotate { original, .. } | Gesture::EditPoint { original, .. } => {
                if let Some(i) = self.doc.find_index(original.id()) {
                    self.doc.annotations[i] = original;
                }
            }
            _ => {}
        }
    }

    pub fn set_tool(&mut self, t: Tool) {
        self.cancel_gesture();
        if t != Tool::Select {
            self.pixel_sel = None;
        }
        if t == Tool::Select || t == Tool::CutOut {
            self.selected.clear();
        }
        self.active_tool = t;
    }

    /// Load the properties of the selected annotation into the quick style
    /// so the next annotation inherits them.
    pub fn adopt_style_from_selection(&mut self) {
        let Some(a) = self.single().cloned() else { return };
        let s = &mut self.style;
        match a {
            Annotation::Arrow { color, stroke, head, tail, shadow, bezier, .. } => {
                s.color = color;
                s.stroke = stroke;
                s.head = head;
                s.tail = tail;
                s.shadow = shadow;
                s.curved = bezier.is_some();
            }
            Annotation::Shape { outline, fill, stroke, shadow, .. } => {
                s.color = outline;
                s.fill_alpha = fill.a;
                s.stroke = stroke;
                s.shadow = shadow;
            }
            Annotation::Text { color, size, .. } => {
                s.color = color;
                s.font_size = size;
            }
            Annotation::Step { fill, radius, value, .. } => {
                s.color = fill;
                s.step_radius = radius;
                s.step_letters = value.chars().all(|c| c.is_ascii_alphabetic());
            }
            _ => {}
        }
    }

    // ---- pointer handling ----

    fn on_down(&mut self, sx: f32, sy: f32, shift: bool, ctrl: bool) {
        let p = self.screen_to_doc(sx, sy);
        if self.space_down {
            self.gesture = Gesture::Pan { last: (sx, sy) };
            return;
        }
        // Polygon tool / polygon selection accumulate clicks.
        if let Gesture::Polygon { pts, .. } = &mut self.gesture {
            let close = pts.len() >= 3 && pts[0].dist(p) * self.view_scale <= 8.0;
            if close {
                self.finish_polygon();
            } else {
                pts.push(p);
            }
            return;
        }
        // Handles of the current selection.
        if let Some(h) = self.handle_at(sx, sy) {
            match h {
                Handle::Frame(i) => {
                    let (frame0, rot) = match self.single() {
                        Some(a) if a.has_frame() => (a.frame(), a.rotation()),
                        _ => (self.selection_frame().unwrap_or(Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }), 0.0),
                    };
                    let originals = self.selected.iter().filter_map(|id| self.doc.get(*id).cloned()).collect();
                    self.gesture = Gesture::Resize { handle: i, frame0, rot, originals };
                }
                Handle::Rotate => {
                    if let Some(a) = self.single().cloned() {
                        let c = a.frame().center();
                        let angle0 = (p.y - c.y).atan2(p.x - c.x).to_degrees();
                        self.gesture = Gesture::Rotate { center: c, original: a, angle0 };
                    }
                }
                Handle::Edit(idx) => {
                    if let Some(a) = self.single().cloned() {
                        self.gesture = Gesture::EditPoint { idx, original: a };
                    }
                }
            }
            return;
        }
        let tol = self.tol();
        match self.active_tool {
            Tool::Move => {
                match self.doc.topmost_hit(p, tol) {
                    Some(id) => {
                        if shift || ctrl {
                            let grp = self.doc.expand_groups(&[id]);
                            if self.selected.contains(&id) {
                                self.selected.retain(|x| !grp.contains(x));
                            } else {
                                self.selected.extend(grp);
                            }
                        } else if !self.selected.contains(&id) {
                            self.select(&[id]);
                        }
                        let part = if self.selected.len() == 1 {
                            self.doc.get(id).and_then(|a| a.part_at(p, tol)).unwrap_or(Part::Whole)
                        } else {
                            Part::Whole
                        };
                        self.begin_move(p, part);
                    }
                    None => {
                        if !shift {
                            self.selected.clear();
                        }
                        self.gesture = Gesture::Marquee { start: p, cur: p, additive: shift };
                    }
                }
            }
            Tool::Select => {
                if self.pixel_sel.as_ref().map(|s| s.mask_contains(p)).unwrap_or(false) {
                    self.gesture = Gesture::PendingLift { start: p };
                    return;
                }
                self.pixel_sel = None;
                let p = self.snap(p);
                self.gesture = match self.style.sel_shape {
                    SelShape::Polygon => Gesture::Polygon { pts: vec![p], hover: p },
                    SelShape::Freehand => Gesture::Freehand { pts: vec![p] },
                    _ => Gesture::SelRect { start: p, cur: p },
                };
            }
            Tool::CutOut => self.gesture = Gesture::Band { start: p, cur: p },
            Tool::Polygon => {
                self.selected.clear();
                self.gesture = Gesture::Polygon { pts: vec![p], hover: p };
            }
            _ => {
                // Drawing tools can still grab an already-selected object.
                let grab = self.selected.iter().copied().find(|id| self.doc.get(*id).map(|a| a.hit_test(p, tol)).unwrap_or(false));
                if let Some(id) = grab {
                    let part = self.doc.get(id).and_then(|a| a.part_at(p, tol)).unwrap_or(Part::Whole);
                    self.begin_move(p, if self.selected.len() == 1 { part } else { Part::Whole });
                    return;
                }
                self.selected.clear();
                self.gesture = Gesture::Create { start: p, cur: p };
            }
        }
    }

    fn begin_move(&mut self, p: Point, part: Part) {
        let originals = self.selected.iter().filter_map(|id| self.doc.get(*id).cloned()).collect();
        self.gesture = Gesture::Move { start: p, last: p, part, originals, moved: false };
    }

    fn on_move(&mut self, sx: f32, sy: f32, buttons: bool) {
        let p = self.screen_to_doc(sx, sy);
        let shift = key_down(VK_SHIFT);
        let scale = self.view_scale;
        let snapped = if matches!(self.gesture, Gesture::SelRect { .. }) { self.snap(p) } else { p };
        match &mut self.gesture {
            Gesture::Pan { last } => {
                self.view_offset.0 += sx - last.0;
                self.view_offset.1 += sy - last.1;
                *last = (sx, sy);
                self.user_view = true;
                self.clamp_view();
            }
            Gesture::Create { cur, .. } | Gesture::Marquee { cur, .. } | Gesture::Band { cur, .. } => *cur = p,
            Gesture::SelRect { cur, .. } => *cur = snapped,
            Gesture::Polygon { hover, .. } => *hover = p,
            Gesture::Freehand { pts } => {
                if pts.last().map(|l| l.dist(p) * scale >= 2.0).unwrap_or(true) {
                    pts.push(p);
                }
            }
            Gesture::Move { start, last, part, moved, .. } => {
                if !buttons {
                    return;
                }
                if !*moved && start.dist(p) * scale < 3.0 {
                    return;
                }
                *moved = true;
                let (dx, dy) = (p.x - last.x, p.y - last.y);
                *last = p;
                let part = *part;
                for id in self.selected.clone() {
                    if let Some(i) = self.doc.find_index(id) {
                        self.doc.annotations[i].translate_part(part, dx, dy);
                    }
                }
            }
            Gesture::Resize { handle, frame0, rot, originals } => {
                let (h, f0, r) = (*handle, *frame0, *rot);
                let c0 = f0.center();
                let local = p.rotated(c0, -r);
                let mut nf = resize_frame(f0, h, local, shift && h % 2 == 0);
                if r != 0.0 {
                    // Keep the opposite handle fixed on screen.
                    let anchor_w = frame_point(f0, (h + 4) % 8).rotated(c0, r);
                    let anchor_n = frame_point(nf, (h + 4) % 8).rotated(nf.center(), r);
                    nf = nf.translated(anchor_w.x - anchor_n.x, anchor_w.y - anchor_n.y);
                }
                let originals = originals.clone();
                let multi = originals.len() > 1;
                for o in originals {
                    if let Some(i) = self.doc.find_index(o.id()) {
                        let mut a = o.clone();
                        if multi {
                            let of = o.frame();
                            a.set_frame(of, f0.map_rect(&nf, of));
                        } else {
                            a.set_frame(f0, nf);
                        }
                        self.doc.annotations[i] = a;
                    }
                }
            }
            Gesture::Rotate { center, original, angle0 } => {
                let ang = (p.y - center.y).atan2(p.x - center.x).to_degrees();
                let mut deg = original.rotation() + ang - *angle0;
                if shift {
                    deg = (deg / 15.0).round() * 15.0;
                }
                let mut a = original.clone();
                a.set_rotation(deg);
                if let Some(i) = self.doc.find_index(a.id()) {
                    self.doc.annotations[i] = a;
                }
            }
            Gesture::EditPoint { idx, original } => {
                let mut a = original.clone();
                let mut q = p;
                if shift {
                    if let Annotation::Arrow { start, end, .. } = &a {
                        let anchor = if *idx == 1 { *start } else if *idx == 0 { *end } else { q };
                        q = tools::constrain(Tool::Arrow, anchor, q, true);
                    }
                }
                a.set_edit_point(*idx, q);
                if let Some(i) = self.doc.find_index(a.id()) {
                    self.doc.annotations[i] = a;
                }
            }
            Gesture::PendingLift { start } => {
                if start.dist(p) * scale >= 3.0 {
                    let start = *start;
                    if let Some(id) = self.sel_lift() {
                        self.selected = vec![id];
                        self.active_tool = Tool::Move;
                        self.begin_move(start, Part::Whole);
                        self.on_move(sx, sy, true);
                    }
                }
            }
            Gesture::Idle => {}
        }
    }

    /// Returns a text-edit request when the gesture created a text-bearing
    /// annotation.
    fn on_up(&mut self, sx: f32, sy: f32) -> Option<TextEditReq> {
        let p = self.screen_to_doc(sx, sy);
        let shift = key_down(VK_SHIFT);
        let g = std::mem::replace(&mut self.gesture, Gesture::Idle);
        match g {
            Gesture::Polygon { pts, hover } => {
                // Keep collecting clicks.
                self.gesture = Gesture::Polygon { pts, hover };
            }
            Gesture::Create { start, .. } => {
                let end = tools::constrain(self.active_tool, start, p, shift);
                if let Some(a) = tools::create_from_drag(self.active_tool, &self.style, start, end, &self.doc) {
                    let is_text = matches!(a, Annotation::Text { .. } | Annotation::Callout { .. });
                    let id = self.insert_new(a);
                    self.selected = vec![id];
                    if is_text {
                        return self.text_edit_request(id);
                    }
                }
            }
            Gesture::Marquee { start, cur, additive } => {
                let r = Rect::from_points(start, cur);
                let mut hits: Vec<AnnotationId> = self.doc.annotations.iter().filter(|a| {
                    let b = a.bounding_box();
                    b.x >= r.x && b.y >= r.y && b.right() <= r.right() && b.bottom() <= r.bottom()
                }).map(|a| a.id()).collect();
                if additive {
                    hits.extend(self.selected.iter().copied());
                }
                self.select(&hits);
            }
            Gesture::Move { originals, moved, .. } => {
                if moved {
                    let mut cmds = Vec::new();
                    for o in originals {
                        if let Some(i) = self.doc.find_index(o.id()) {
                            cmds.push(Command::Mutate { id: o.id(), before: o, after: self.doc.annotations[i].clone() });
                        }
                    }
                    self.history.push(Command::Batch(cmds));
                    self.dirty = true;
                }
            }
            Gesture::Resize { originals, .. } => {
                let mut cmds = Vec::new();
                for o in originals {
                    if let Some(i) = self.doc.find_index(o.id()) {
                        cmds.push(Command::Mutate { id: o.id(), before: o, after: self.doc.annotations[i].clone() });
                    }
                }
                self.history.push(Command::Batch(cmds));
                self.dirty = true;
            }
            Gesture::Rotate { original, .. } | Gesture::EditPoint { original, .. } => {
                if let Some(i) = self.doc.find_index(original.id()) {
                    let after = self.doc.annotations[i].clone();
                    self.history.push(Command::Mutate { id: original.id(), before: original, after });
                    self.dirty = true;
                }
            }
            Gesture::SelRect { start, cur } => {
                let end = tools::constrain(Tool::Select, start, cur, shift);
                let r = Rect::from_points(start, end);
                if r.w * self.view_scale < 3.0 || r.h * self.view_scale < 3.0 {
                    // Click: snap-to-object selects the frame under the cursor.
                    if self.style.snap {
                        if let Some(id) = self.doc.topmost_hit(start, self.tol()) {
                            let b = self.doc.get(id).map(|a| a.bounding_box()).unwrap();
                            self.pixel_sel = Some(PixelSel::from_rect(SelShape::Rectangle, b));
                        }
                    }
                } else {
                    self.pixel_sel = Some(PixelSel::from_rect(self.style.sel_shape, r));
                }
            }
            Gesture::Freehand { pts } => {
                if pts.len() >= 3 && Rect::bounding(&pts).w * self.view_scale >= 3.0 {
                    self.pixel_sel = Some(PixelSel { shape: SelShape::Freehand, outline: pts, inverted: false });
                }
            }
            Gesture::Band { start, cur } => {
                if self.style.cut_horizontal { self.apply_cut(start.y, cur.y) } else { self.apply_cut(start.x, cur.x) }
            }
            Gesture::PendingLift { .. } => {}
            Gesture::Pan { .. } | Gesture::Idle => {}
        }
        None
    }

    pub fn finish_polygon(&mut self) {
        if let Gesture::Polygon { mut pts, .. } = std::mem::replace(&mut self.gesture, Gesture::Idle) {
            // A double-click adds a duplicate final point; drop near-duplicates.
            pts.dedup_by(|a, b| a.dist(*b) < 1.0);
            if self.active_tool == Tool::Polygon {
                if let Some(a) = tools::create_polygon(&self.style, pts) {
                    let id = self.insert_new(a);
                    self.selected = vec![id];
                }
            } else if pts.len() >= 3 {
                self.pixel_sel = Some(PixelSel { shape: SelShape::Polygon, outline: pts, inverted: false });
            }
        }
    }

    pub fn polygon_in_progress(&self) -> bool {
        matches!(self.gesture, Gesture::Polygon { .. })
    }

    /// Request an in-place editor for a text-bearing annotation.
    pub fn text_edit_request(&self, id: AnnotationId) -> Option<TextEditReq> {
        let a = self.doc.get(id)?;
        let (rect, text, size) = match a {
            Annotation::Text { bounds, text, size, .. } => (*bounds, text.clone(), *size),
            Annotation::Callout { bounds, text, size, .. } => (bounds.inflated(-render::CALLOUT_PAD), text.clone(), *size),
            Annotation::Step { center, radius, value, .. } => {
                let r = radius.max(18.0);
                (Rect { x: center.x - r, y: center.y - r * 0.6, w: r * 2.0 + 20.0, h: r * 1.2 }, value.clone(), radius * 0.9)
            }
            _ => return None,
        };
        let tl = self.doc_to_screen(Point::new(rect.x, rect.y));
        Some(TextEditReq {
            id,
            rect: Rect { x: tl.x, y: tl.y, w: rect.w * self.view_scale, h: rect.h * self.view_scale },
            text,
            size: size * self.view_scale,
        })
    }

    pub fn flatten(&self, secure: bool) -> Result<Bitmap> {
        render::flatten(&self.gfx, &self.doc, RenderOpts { secure, skip: None })
    }

    /// Whether any blur/pixelate (reversible) redactions are present.
    pub fn has_soft_redactions(&self) -> bool {
        self.doc.annotations.iter().any(|a| matches!(a, Annotation::Blur { mode, .. } if *mode != crate::editor::annotation::BlurMode::Solid))
    }
}

impl PixelSel {
    pub fn mask_contains(&self, p: Point) -> bool {
        let inside = crate::editor::annotation::point_in_polygon(p, &self.outline);
        inside != self.inverted
    }
}

/// Handle positions on a frame: 0 TL, 1 T, 2 TR, 3 R, 4 BR, 5 B, 6 BL, 7 L.
pub fn frame_point(f: Rect, i: u8) -> Point {
    let (l, t, r, b, cx, cy) = (f.x, f.y, f.right(), f.bottom(), f.center().x, f.center().y);
    match i {
        0 => Point::new(l, t),
        1 => Point::new(cx, t),
        2 => Point::new(r, t),
        3 => Point::new(r, cy),
        4 => Point::new(r, b),
        5 => Point::new(cx, b),
        6 => Point::new(l, b),
        _ => Point::new(l, cy),
    }
}

fn resize_frame(f: Rect, h: u8, p: Point, keep_aspect: bool) -> Rect {
    let (mut l, mut t, mut r, mut b) = (f.x, f.y, f.right(), f.bottom());
    if matches!(h, 0 | 6 | 7) {
        l = p.x;
    }
    if matches!(h, 2 | 3 | 4) {
        r = p.x;
    }
    if matches!(h, 0 | 1 | 2) {
        t = p.y;
    }
    if matches!(h, 4 | 5 | 6) {
        b = p.y;
    }
    if keep_aspect && f.w > 0.0 && f.h > 0.0 {
        let s = ((r - l) / f.w).abs().max(((b - t) / f.h).abs());
        let (nw, nh) = (f.w * s, f.h * s);
        if matches!(h, 0 | 6) { l = r - nw * (r - l).signum() } else { r = l + nw * (r - l).signum() }
        if matches!(h, 0 | 2) { t = b - nh * (b - t).signum() } else { b = t + nh * (b - t).signum() }
    }
    let nf = Rect::from_points(Point::new(l, t), Point::new(r, b));
    Rect { w: nf.w.max(2.0), h: nf.h.max(2.0), ..nf }
}

fn clipboard_seq() -> u32 {
    unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() }
}

pub fn put_clipboard_image(hwnd: HWND, bm: &Bitmap) -> Result<()> {
    let png = crate::gfx::gdiplus::encode_png_bytes(bm)?;
    crate::win::clipboard::write_image(hwnd, bm, &png)
}

// --- Child window registration & wndproc ---

pub fn register_class() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let cname = crate::win::wide(CLASS_NAME);
        let wc = WNDCLASSEXW {
            cbSize: core::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: GetModuleHandleW(core::ptr::null()) as _,
            hIcon: 0,
            hCursor: 0,
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
            WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
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

fn xy(l: LPARAM) -> (f32, f32) {
    ((l & 0xFFFF) as i16 as f32, ((l >> 16) & 0xFFFF) as i16 as f32)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let parent = unsafe { GetParent(hwnd) };
    let Some(state) = crate::editor::window::get_state(parent) else {
        return unsafe { DefWindowProcW(hwnd, msg, w, l) };
    };
    let refresh = |all: bool| unsafe {
        InvalidateRect(hwnd, core::ptr::null(), 0);
        if all {
            crate::editor::window::refresh_panels(parent);
        }
    };
    unsafe {
        match msg {
            windows_sys::Win32::UI::WindowsAndMessaging::WM_ERASEBKGND => 1,
            WM_PAINT => {
                let mut ps: windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT = core::mem::zeroed();
                windows_sys::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
                if let Ok(mut st) = state.try_borrow_mut() {
                    let _ = st.canvas.render();
                }
                windows_sys::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
                0
            }
            WM_SIZE => {
                let cw = (l & 0xFFFF) as u16 as u32;
                let ch = ((l >> 16) & 0xFFFF) as u16 as u32;
                let _ = state.borrow_mut().canvas.resize(cw.max(1), ch.max(1));
                refresh(false);
                0
            }
            WM_SETCURSOR => {
                if (l & 0xFFFF) as u32 == HTCLIENT {
                    let c = state.borrow().canvas.cursor;
                    SetCursor(LoadCursorW(0, c));
                    return 1;
                }
                DefWindowProcW(hwnd, msg, w, l)
            }
            WM_MBUTTONDOWN => {
                SetFocus(hwnd);
                SetCapture(hwnd);
                let (x, y) = xy(l);
                state.borrow_mut().canvas.gesture = Gesture::Pan { last: (x, y) };
                0
            }
            WM_MBUTTONUP => {
                ReleaseCapture();
                state.borrow_mut().canvas.gesture = Gesture::Idle;
                0
            }
            WM_LBUTTONDOWN => {
                SetFocus(hwnd);
                SetCapture(hwnd);
                let (x, y) = xy(l);
                let shift = (w & 0x4) != 0;
                let ctrl = (w & 0x8) != 0;
                state.borrow_mut().canvas.on_down(x, y, shift, ctrl);
                refresh(true);
                0
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(l);
                let buttons = (w & 0x13) != 0;
                let mut st = state.borrow_mut();
                let busy = !matches!(st.canvas.gesture, Gesture::Idle);
                st.canvas.on_move(x, y, buttons);
                let p = st.canvas.screen_to_doc(x, y);
                st.canvas.cursor = st.canvas.cursor_for(x, y, p);
                let c = st.canvas.cursor;
                drop(st);
                SetCursor(LoadCursorW(0, c));
                if busy {
                    refresh(false);
                }
                0
            }
            WM_LBUTTONUP => {
                ReleaseCapture();
                let (x, y) = xy(l);
                let req = state.borrow_mut().canvas.on_up(x, y);
                refresh(true);
                if let Some(r) = req {
                    crate::editor::text_edit::spawn_for(parent, hwnd, r);
                }
                0
            }
            WM_CAPTURECHANGED => {
                let mut st = state.borrow_mut();
                if matches!(st.canvas.gesture, Gesture::Pan { .. }) {
                    st.canvas.gesture = Gesture::Idle;
                }
                0
            }
            WM_LBUTTONDBLCLK => {
                let (x, y) = xy(l);
                let req = {
                    let mut st = state.borrow_mut();
                    if st.canvas.polygon_in_progress() {
                        st.canvas.finish_polygon();
                        None
                    } else {
                        let p = st.canvas.screen_to_doc(x, y);
                        let tol = st.canvas.tol();
                        match st.canvas.doc.topmost_hit(p, tol) {
                            Some(id) => {
                                st.canvas.selected = vec![id];
                                st.canvas.text_edit_request(id)
                            }
                            None => None,
                        }
                    }
                };
                refresh(true);
                if let Some(r) = req {
                    crate::editor::text_edit::spawn_for(parent, hwnd, r);
                }
                0
            }
            WM_RBUTTONUP => {
                let (x, y) = xy(l);
                {
                    let mut st = state.borrow_mut();
                    // Right-click on an unselected object selects it first.
                    if st.canvas.active_tool != Tool::Select {
                        let p = st.canvas.screen_to_doc(x, y);
                        let tol = st.canvas.tol();
                        if let Some(id) = st.canvas.doc.topmost_hit(p, tol) {
                            if !st.canvas.selected.contains(&id) {
                                st.canvas.select(&[id]);
                            }
                        }
                    }
                }
                refresh(true);
                let mut pt = POINT { x: x as i32, y: y as i32 };
                windows_sys::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut pt);
                crate::editor::window::show_context_menu(parent, pt.x, pt.y);
                0
            }
            WM_MOUSEWHEEL => {
                let delta = ((w >> 16) & 0xFFFF) as i16 as f32 / 120.0;
                let mut pt = POINT { x: (l & 0xFFFF) as i16 as i32, y: ((l >> 16) & 0xFFFF) as i16 as i32 };
                ScreenToClient(hwnd, &mut pt);
                let ctrl = (w & 0x8) != 0;
                let shift = (w & 0x4) != 0;
                {
                    let mut st = state.borrow_mut();
                    let c = &mut st.canvas;
                    if ctrl {
                        let s = c.view_scale * 1.15f32.powf(delta);
                        c.zoom_to(s, pt.x as f32, pt.y as f32);
                    } else if shift {
                        c.view_offset.0 += delta * 60.0;
                        c.user_view = true;
                        c.clamp_view();
                    } else {
                        c.view_offset.1 += delta * 60.0;
                        c.user_view = true;
                        c.clamp_view();
                    }
                }
                refresh(true);
                0
            }
            WM_KEYUP => {
                if w as u16 == VK_SPACE {
                    state.borrow_mut().canvas.space_down = false;
                }
                0
            }
            WM_KEYDOWN => {
                let key = w as u16;
                let ctrl = key_down(VK_CONTROL);
                let shift = key_down(VK_SHIFT);
                let alt = key_down(VK_MENU);
                // Menu accelerators are routed to the parent's WM_COMMAND.
                if let Some(cmd) = crate::editor::window::accelerator(key, ctrl, shift, alt) {
                    PostMessageW(parent, WM_COMMAND, cmd as WPARAM, 0);
                    return 0;
                }
                let mut st = state.borrow_mut();
                let c = &mut st.canvas;
                let changed = match key {
                    VK_SPACE => {
                        c.space_down = true;
                        false
                    }
                    VK_ESCAPE => {
                        let was_busy = !matches!(c.gesture, Gesture::Idle);
                        c.cancel_gesture();
                        if !was_busy {
                            c.selected.clear();
                            c.pixel_sel = None;
                        }
                        true
                    }
                    VK_RETURN => {
                        c.finish_polygon();
                        true
                    }
                    VK_DELETE | VK_BACK => c.delete_selection(),
                    VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN => {
                        let d = if shift { 10.0 } else { 1.0 };
                        let (dx, dy) = match key {
                            VK_LEFT => (-d, 0.0),
                            VK_RIGHT => (d, 0.0),
                            VK_UP => (0.0, -d),
                            _ => (0.0, d),
                        };
                        c.nudge(dx, dy)
                    }
                    k if !ctrl && !alt && (b'A' as u16..=b'Z' as u16).contains(&k) => {
                        match Tool::ALL.iter().find(|t| t.key() as u16 == k) {
                            Some(t) => {
                                c.set_tool(*t);
                                true
                            }
                            None => false,
                        }
                    }
                    _ => false,
                };
                drop(st);
                if changed {
                    refresh(true);
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}
