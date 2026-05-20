use crate::editor::annotation::{Annotation, BlurMode, Document, Point, Rect, ShapeKind};
use crate::editor::tools::{Drag, Tool, ToolStyle};
use crate::error::Result;
use crate::gfx::Gfx;
use crate::gfx::bitmap::Bitmap;
use crate::gfx::color::Color;
use crate::win::wide;
use std::rc::Rc;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    CreateCompatibleDC, CreateDIBSection, CreateFontW, CreatePen, CreateSolidBrush,
    DEFAULT_CHARSET, DIB_RGB_COLORS, DeleteDC, DeleteObject, Ellipse, FW_NORMAL, FillRect,
    FrameRect, GetStockObject, HALFTONE, HBITMAP, HBRUSH, HDC, LineTo, MoveToEx, NULL_BRUSH,
    OUT_DEFAULT_PRECIS, PS_SOLID, Polygon, Rectangle, SRCCOPY, SelectObject, SetBkMode,
    SetStretchBltMode, SetTextColor, StretchBlt, TRANSPARENT, TextOutW, VARIABLE_PITCH,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

pub struct Canvas {
    pub gfx: Rc<Gfx>,
    pub doc: Document,
    pub base_dib: Option<HBITMAP>,
    pub selected: Option<u32>,
    pub view_scale: f32,
    pub view_offset: (f32, f32),
    pub size: (u32, u32),
    pub active_tool: Tool,
    pub style: ToolStyle,
    pub drag: Option<Drag>,
    pub move_anchor: Option<Point>,
}

impl Canvas {
    pub fn create(gfx: Rc<Gfx>, doc: Document) -> Result<Self> {
        let mut s = Self {
            gfx,
            doc,
            base_dib: None,
            selected: None,
            view_scale: 1.0,
            view_offset: (40.0, 8.0),
            size: (1024, 768),
            active_tool: Tool::Arrow,
            style: ToolStyle::default(),
            drag: None,
            move_anchor: None,
        };
        s.upload_base();
        Ok(s)
    }

    pub fn attach(&mut self, _hwnd: HWND, w: u32, h: u32) -> Result<()> {
        self.size = (w.max(1), h.max(1));
        Ok(())
    }

    pub fn resize(&mut self, w: u32, h: u32) -> Result<()> {
        self.size = (w.max(1), h.max(1));
        Ok(())
    }

    pub fn replace_document(&mut self, doc: Document) -> Result<()> {
        self.cleanup_dib();
        self.doc = doc;
        self.selected = None;
        self.upload_base();
        Ok(())
    }

    fn cleanup_dib(&mut self) {
        if let Some(dib) = self.base_dib.take() {
            unsafe {
                DeleteObject(dib as _);
            }
        }
    }

    fn upload_base(&mut self) {
        let bm = &self.doc.base;
        unsafe {
            let mut bi: BITMAPINFO = core::mem::zeroed();
            bi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = bm.width as i32;
            bi.bmiHeader.biHeight = -(bm.height as i32);
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB as u32;
            let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
            let dib = CreateDIBSection(
                0,
                &bi,
                DIB_RGB_COLORS,
                &mut bits,
                0,
                0,
            );
            if dib != 0 && !bits.is_null() {
                core::ptr::copy_nonoverlapping(bm.bgra.as_ptr(), bits as *mut u8, bm.bgra.len());
                self.base_dib = Some(dib);
            }
        }
    }

    /// Render the canvas into `hdc`. The canvas paints its own background in
    /// logical (0,0) .. (self.size.0, self.size.1); the caller is responsible
    /// for positioning via `SetViewportOrgEx` if the canvas is offset within
    /// its host window (e.g. below a toolbar).
    pub fn render(&self, _hwnd: HWND, hdc: HDC) {
        unsafe {
            let rc = RECT {
                left: 0,
                top: 0,
                right: self.size.0 as i32,
                bottom: self.size.1 as i32,
            };

            let bg = CreateSolidBrush(rgb_to_colorref(Color::rgb(0x1A, 0x1B, 0x1D)));
            FillRect(hdc, &rc, bg);
            DeleteObject(bg as _);

            if let Some(dib) = self.base_dib {
                let mem_dc = CreateCompatibleDC(hdc);
                let old = SelectObject(mem_dc, dib as _);
                SetStretchBltMode(hdc, HALFTONE as i32);
                let dx = self.view_offset.0 as i32;
                let dy = self.view_offset.1 as i32;
                let dw = (self.doc.base.width as f32 * self.view_scale) as i32;
                let dh = (self.doc.base.height as f32 * self.view_scale) as i32;
                StretchBlt(
                    hdc,
                    dx,
                    dy,
                    dw,
                    dh,
                    mem_dc,
                    0,
                    0,
                    self.doc.base.width as i32,
                    self.doc.base.height as i32,
                    SRCCOPY,
                );
                SelectObject(mem_dc, old);
                DeleteDC(mem_dc);
            }

            for ann in &self.doc.annotations {
                self.draw_annotation(hdc, ann);
            }
            if let Some(drag) = self.drag {
                self.draw_preview(hdc, drag);
            }
            if let Some(sel) = self.selected {
                if let Some(i) = self.doc.find_index(sel) {
                    let bb = self.doc.annotations[i].bounding_box();
                    self.draw_handles(hdc, bb);
                }
            }
        }
    }

    unsafe fn draw_annotation(&self, hdc: HDC, ann: &Annotation) {
        unsafe {
            match ann {
                Annotation::Arrow { start, end, color, stroke, .. } => {
                    self.draw_arrow(hdc, *start, *end, *color, *stroke);
                }
                Annotation::Shape { kind, bounds, outline, fill, stroke, .. } => {
                    self.draw_shape(hdc, *kind, *bounds, *outline, *fill, *stroke);
                }
                Annotation::Text { bounds, text, color, size, .. } => {
                    self.draw_text(hdc, *bounds, text, *color, *size);
                }
                Annotation::Step { center, radius, value, fill, text, .. } => {
                    self.draw_step(hdc, *center, *radius, value, *fill, *text);
                }
                Annotation::Highlighter { bounds, color, .. } => {
                    self.draw_highlight(hdc, *bounds, *color);
                }
                Annotation::Blur { bounds, mode, fill, .. } => {
                    let c = match mode {
                        BlurMode::Solid => *fill,
                        BlurMode::Blur => Color::rgba(40, 40, 40, 255),
                    };
                    self.draw_filled_rect(hdc, *bounds, c);
                }
            }
        }
    }

    fn to_screen(&self, p: Point) -> (i32, i32) {
        (
            (self.view_offset.0 + p.x * self.view_scale) as i32,
            (self.view_offset.1 + p.y * self.view_scale) as i32,
        )
    }

    fn to_screen_rect(&self, r: Rect) -> RECT {
        let (x, y) = self.to_screen(Point::new(r.x, r.y));
        let (x2, y2) = self.to_screen(Point::new(r.x + r.w, r.y + r.h));
        RECT { left: x, top: y, right: x2, bottom: y2 }
    }

    unsafe fn draw_arrow(&self, hdc: HDC, start: Point, end: Point, color: Color, stroke: f32) {
        unsafe {
            let (sx, sy) = self.to_screen(start);
            let (ex, ey) = self.to_screen(end);
            let pen_w = ((stroke * self.view_scale) as i32).max(1);
            let pen = CreatePen(PS_SOLID as i32, pen_w, rgb_to_colorref(color));
            let old_pen = SelectObject(hdc, pen as _);
            let dx = (ex - sx) as f32;
            let dy = (ey - sy) as f32;
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let ux = dx / len;
            let uy = dy / len;
            let head = (stroke * 4.0 + 6.0) * self.view_scale;
            let shaft_ex = (ex as f32 - ux * head * 0.6) as i32;
            let shaft_ey = (ey as f32 - uy * head * 0.6) as i32;
            MoveToEx(hdc, sx, sy, core::ptr::null_mut());
            LineTo(hdc, shaft_ex, shaft_ey);

            let ortho_x = -uy;
            let ortho_y = ux;
            let p2x = (ex as f32 - ux * head + ortho_x * head * 0.5) as i32;
            let p2y = (ey as f32 - uy * head + ortho_y * head * 0.5) as i32;
            let p3x = (ex as f32 - ux * head - ortho_x * head * 0.5) as i32;
            let p3y = (ey as f32 - uy * head - ortho_y * head * 0.5) as i32;
            let fill = CreateSolidBrush(rgb_to_colorref(color));
            let old_brush = SelectObject(hdc, fill as _);
            let pts = [
                windows_sys::Win32::Foundation::POINT { x: ex, y: ey },
                windows_sys::Win32::Foundation::POINT { x: p2x, y: p2y },
                windows_sys::Win32::Foundation::POINT { x: p3x, y: p3y },
            ];
            Polygon(hdc, pts.as_ptr(), pts.len() as i32);
            SelectObject(hdc, old_brush);
            DeleteObject(fill as _);
            SelectObject(hdc, old_pen);
            DeleteObject(pen as _);
        }
    }

    unsafe fn draw_shape(
        &self,
        hdc: HDC,
        kind: ShapeKind,
        bounds: Rect,
        outline: Color,
        fill: Color,
        stroke: f32,
    ) {
        unsafe {
            let r = self.to_screen_rect(bounds);
            let pen_w = ((stroke * self.view_scale) as i32).max(1);
            let pen = CreatePen(PS_SOLID as i32, pen_w, rgb_to_colorref(outline));
            let old_pen = SelectObject(hdc, pen as _);
            let (brush, owned) = if fill.a > 0 {
                (CreateSolidBrush(rgb_to_colorref(fill)), true)
            } else {
                (GetStockObject(NULL_BRUSH as i32) as HBRUSH, false)
            };
            let old_brush = SelectObject(hdc, brush as _);
            match kind {
                ShapeKind::Rectangle => {
                    Rectangle(hdc, r.left, r.top, r.right, r.bottom);
                }
                ShapeKind::Ellipse => {
                    Ellipse(hdc, r.left, r.top, r.right, r.bottom);
                }
            }
            SelectObject(hdc, old_brush);
            if owned {
                DeleteObject(brush as _);
            }
            SelectObject(hdc, old_pen);
            DeleteObject(pen as _);
        }
    }

    unsafe fn draw_text(&self, hdc: HDC, bounds: Rect, text: &str, color: Color, size: f32) {
        unsafe {
            if text.is_empty() {
                let r = self.to_screen_rect(bounds);
                let pen = CreatePen(PS_SOLID as i32, 1, rgb_to_colorref(Color::rgb(180, 180, 180)));
                let old_pen = SelectObject(hdc, pen as _);
                let nb = GetStockObject(NULL_BRUSH as i32);
                let old_brush = SelectObject(hdc, nb);
                Rectangle(hdc, r.left, r.top, r.right, r.bottom);
                SelectObject(hdc, old_brush);
                SelectObject(hdc, old_pen);
                DeleteObject(pen as _);
                return;
            }
            let (x, y) = self.to_screen(Point::new(bounds.x, bounds.y));
            let font = CreateFontW(
                -((size * self.view_scale) as i32),
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
                wide("Segoe UI").as_ptr(),
            );
            let old_font = SelectObject(hdc, font as _);
            SetBkMode(hdc, TRANSPARENT as i32);
            SetTextColor(hdc, rgb_to_colorref(color));
            let tw: Vec<u16> = text.encode_utf16().collect();
            TextOutW(hdc, x, y, tw.as_ptr(), tw.len() as i32);
            SelectObject(hdc, old_font);
            DeleteObject(font as _);
        }
    }

    unsafe fn draw_step(
        &self,
        hdc: HDC,
        center: Point,
        radius: f32,
        value: &str,
        fill: Color,
        text_color: Color,
    ) {
        unsafe {
            let (cx, cy) = self.to_screen(center);
            let r = (radius * self.view_scale) as i32;
            let brush = CreateSolidBrush(rgb_to_colorref(fill));
            let old_brush = SelectObject(hdc, brush as _);
            let pen = CreatePen(PS_SOLID as i32, 1, rgb_to_colorref(fill));
            let old_pen = SelectObject(hdc, pen as _);
            Ellipse(hdc, cx - r, cy - r, cx + r, cy + r);
            SelectObject(hdc, old_pen);
            SelectObject(hdc, old_brush);
            DeleteObject(pen as _);
            DeleteObject(brush as _);

            let font = CreateFontW(
                -((radius * self.view_scale * 1.3) as i32),
                0,
                0,
                0,
                700,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                VARIABLE_PITCH as u32,
                wide("Segoe UI").as_ptr(),
            );
            let old_font = SelectObject(hdc, font as _);
            SetBkMode(hdc, TRANSPARENT as i32);
            SetTextColor(hdc, rgb_to_colorref(text_color));
            let tw: Vec<u16> = value.encode_utf16().collect();
            let approx_w = tw.len() as i32 * (radius as i32);
            TextOutW(hdc, cx - approx_w / 4, cy - r, tw.as_ptr(), tw.len() as i32);
            SelectObject(hdc, old_font);
            DeleteObject(font as _);
        }
    }

    unsafe fn draw_highlight(&self, hdc: HDC, bounds: Rect, color: Color) {
        let approx = Color {
            a: 255,
            r: ((color.r as u16 + 220) / 2) as u8,
            g: ((color.g as u16 + 220) / 2) as u8,
            b: ((color.b as u16 + 220) / 2) as u8,
        };
        unsafe { self.draw_filled_rect(hdc, bounds, approx) }
    }

    unsafe fn draw_filled_rect(&self, hdc: HDC, bounds: Rect, color: Color) {
        unsafe {
            let r = self.to_screen_rect(bounds);
            let brush = CreateSolidBrush(rgb_to_colorref(color));
            FillRect(hdc, &r, brush);
            DeleteObject(brush as _);
        }
    }

    unsafe fn draw_preview(&self, hdc: HDC, drag: Drag) {
        unsafe {
            match self.active_tool {
                Tool::Arrow => self.draw_arrow(hdc, drag.start, drag.current, self.style.color, self.style.stroke),
                Tool::ShapeRect | Tool::ShapeEllipse => {
                    let bounds = Rect::from_points(drag.start, drag.current);
                    let kind = if self.active_tool == Tool::ShapeRect {
                        ShapeKind::Rectangle
                    } else {
                        ShapeKind::Ellipse
                    };
                    let fill = Color { a: self.style.fill_alpha, ..self.style.color };
                    self.draw_shape(hdc, kind, bounds, self.style.color, fill, self.style.stroke);
                }
                Tool::Highlighter => {
                    let bounds = Rect::from_points(drag.start, drag.current);
                    self.draw_highlight(hdc, bounds, self.style.color);
                }
                Tool::BlurGaussian | Tool::BlurSolid => {
                    let bounds = Rect::from_points(drag.start, drag.current);
                    self.draw_filled_rect(hdc, bounds, Color::rgba(0, 0, 0, 255));
                }
                Tool::SelectionRect => {
                    let bounds = Rect::from_points(drag.start, drag.current);
                    let r = self.to_screen_rect(bounds);
                    let pen = CreatePen(PS_SOLID as i32, 1, rgb_to_colorref(Color::rgb(80, 160, 255)));
                    let old_pen = SelectObject(hdc, pen as _);
                    let nb = GetStockObject(NULL_BRUSH as i32);
                    let old_brush = SelectObject(hdc, nb);
                    Rectangle(hdc, r.left, r.top, r.right, r.bottom);
                    SelectObject(hdc, old_brush);
                    SelectObject(hdc, old_pen);
                    DeleteObject(pen as _);
                }
                _ => {}
            }
        }
    }

    unsafe fn draw_handles(&self, hdc: HDC, bb: Rect) {
        unsafe {
            let r = self.to_screen_rect(bb);
            let outline = CreateSolidBrush(rgb_to_colorref(Color::rgb(80, 160, 255)));
            FrameRect(hdc, &r, outline);
            DeleteObject(outline as _);
            for (cx, cy) in [
                (r.left, r.top),
                (r.right, r.top),
                (r.left, r.bottom),
                (r.right, r.bottom),
            ] {
                let hr = RECT { left: cx - 3, top: cy - 3, right: cx + 3, bottom: cy + 3 };
                let h = CreateSolidBrush(rgb_to_colorref(Color::rgb(80, 160, 255)));
                FillRect(hdc, &hr, h);
                DeleteObject(h as _);
            }
        }
    }

    pub fn screen_to_doc(&self, x: f32, y: f32) -> Point {
        Point::new(
            (x - self.view_offset.0) / self.view_scale,
            (y - self.view_offset.1) / self.view_scale,
        )
    }

    pub fn flatten(&self) -> Result<Bitmap> {
        crate::editor::flatten::flatten(&self.doc)
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        self.cleanup_dib();
    }
}

fn rgb_to_colorref(c: Color) -> u32 {
    (c.r as u32) | ((c.g as u32) << 8) | ((c.b as u32) << 16)
}
