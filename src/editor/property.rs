//! Property panel child window.
//!
//! GDI-rendered strip along the right side of the editor. Shows the
//! properties of the selected annotation (or, with nothing selected, the
//! active tool's quick style), as a list of rows: steppers (‹ value ›),
//! cycling choices, toggles, action buttons, notes, and the stamp browser.
//! Changing a value updates the quick style *and* the selection, so later
//! annotations inherit the current properties.

use crate::editor::annotation::{Annotation, BlurMode, Color, EndStyle, Point, Rect, ShapeKind, Tail};
use crate::editor::pixels::{CutEdge, Effect, SelShape};
use crate::editor::stamps;
use crate::editor::tools::Tool;
use crate::editor::window::{self, EditorState};
use crate::error::{Error, Result};
use crate::gfx::bitmap::Bitmap;
use crate::win::wide;
use std::cell::RefCell;
use std::rc::Rc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush,
    DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER, DT_LEFT, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK,
    DeleteDC, DeleteObject, DrawTextW, EndPaint, FW_NORMAL, FW_SEMIBOLD, FillRect, HDC,
    InvalidateRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, RoundRect, SRCCOPY, SelectObject,
    SetBkMode, SetDIBitsToDevice, SetTextColor, TRANSPARENT, VARIABLE_PITCH,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetParent, GetWindowTextLengthW, GetWindowTextW,
    IDC_ARROW, LoadCursorW, RegisterClassExW, SW_HIDE, SW_SHOW, SendMessageW, SetWindowPos,
    ShowWindow, SWP_NOZORDER, WM_COMMAND, WM_CTLCOLOREDIT, WM_ERASEBKGND, WM_LBUTTONDOWN,
    WM_PAINT, WM_SETFONT, WM_SIZE, WNDCLASSEXW, WS_BORDER, WS_CHILD, WS_VISIBLE,
};

const CLASS_NAME: &str = "frame0.editor.property";

const PAD: i32 = 10;
const ROW_H: i32 = 26;
const LABEL_W: i32 = 78;
const BTN_W: i32 = 20;
const CELL: i32 = 36;
const COLS: i32 = 5;
const SEARCH_ID: usize = 9001;
const EN_CHANGE: u32 = 0x0300;

// BGR colours (match toolbar).
const C_SURFACE: u32 = 0x00201E1C;
const C_ELEVATED: u32 = 0x00282525;
const C_HOVER: u32 = 0x00322E2E;
const C_ACTIVE: u32 = 0x00423D3A;
const C_BORDER: u32 = 0x003D3A3A;
const C_ACCENT: u32 = 0x00D0804A;
const C_LABEL: u32 = 0x009D9A9A;
const C_VALUE: u32 = 0x00E5E4E4;
const C_WARN: u32 = 0x0040B8F0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum P {
    Stroke,
    FontSize,
    Fill,
    Shadow,
    Head,
    Tail,
    Curved,
    BlurMode,
    Intensity,
    StepStyle,
    StepSize,
    Magnification,
    LineWidth,
    Radius,
    Rotation,
    AddTail,
    RemoveTail,
    EditText,
    SelShape,
    SelBg,
    Snap,
    CutDir,
    CutEdge,
    StampCategory,
    OpenStamps,
    Group,
    Ungroup,
    Delete,
    Front,
    Back,
    Flatten,
    SelDelete,
    SelCrop,
    SelInvert,
    SelCopy,
    Fx(u8),
}

enum Row {
    Header(String),
    Stepper(&'static str, String, P),
    Choice(&'static str, String, P),
    Toggle(&'static str, bool, P),
    Button(&'static str, P),
    Buttons(Vec<(&'static str, P)>),
    Note(String, bool),
    Search,
    Stamps(Vec<String>),
    Gap,
}

/// What the panel is describing: a selected annotation or the active tool.
fn context_kind(st: &EditorState) -> (Option<Annotation>, Tool) {
    let c = &st.canvas;
    (c.single().cloned(), c.active_tool)
}

fn fill_label(a: u8) -> &'static str {
    match a {
        0 => "None",
        255 => "Solid",
        _ => "Translucent",
    }
}

fn end_label(e: EndStyle) -> &'static str {
    match e {
        EndStyle::Arrow => "Arrow",
        EndStyle::Circle => "Circle",
        EndStyle::Square => "Square",
        EndStyle::None => "None",
    }
}

fn rows(st: &EditorState) -> Vec<Row> {
    let c = &st.canvas;
    let s = &c.style;
    let (sel, tool) = context_kind(st);
    let mut v = Vec::new();
    let n = c.selected.len();

    if n > 1 {
        v.push(Row::Header(format!("{} objects selected", n)));
        let grouped = c.selected.iter().any(|id| c.doc.groups.contains_key(id));
        v.push(Row::Buttons(vec![("Group", P::Group), ("Ungroup", P::Ungroup)]));
        if grouped {
            v.push(Row::Note("Grouped objects select, move and resize together.".into(), false));
        }
        v.push(Row::Buttons(vec![("Front", P::Front), ("Back", P::Back)]));
        v.push(Row::Button("Delete", P::Delete));
        return v;
    }

    let title = match &sel {
        Some(a) => a.name().to_string(),
        None => tool.name().to_string(),
    };
    v.push(Row::Header(title));

    let kind_tool = match &sel {
        Some(Annotation::Arrow { .. }) => Tool::Arrow,
        Some(Annotation::Shape { kind: ShapeKind::Rectangle, .. }) => Tool::Rect,
        Some(Annotation::Shape { kind: ShapeKind::Ellipse, .. }) => Tool::Ellipse,
        Some(Annotation::Shape { kind: ShapeKind::Polygon, .. }) => Tool::Polygon,
        Some(Annotation::Callout { .. }) => Tool::Callout,
        Some(Annotation::Text { .. }) => Tool::Text,
        Some(Annotation::Step { .. }) => Tool::Step,
        Some(Annotation::Highlighter { .. }) => Tool::Highlighter,
        Some(Annotation::Blur { .. }) => Tool::Blur,
        Some(Annotation::Stamp { .. }) => Tool::Stamp,
        Some(Annotation::Magnify { .. }) => Tool::Magnify,
        Some(Annotation::Image { .. }) => Tool::Move,
        None => tool,
    };

    match kind_tool {
        Tool::Move => {
            if sel.is_none() {
                v.push(Row::Note("Click an object to select it, Shift-click to add, drag on empty space to select several. Drag handles to resize; the round handle rotates.".into(), false));
            }
        }
        Tool::Arrow => {
            v.push(Row::Stepper("Width", format!("{:.0}", s.stroke), P::Stroke));
            v.push(Row::Choice("Head", end_label(s.head).into(), P::Head));
            v.push(Row::Choice("Tail", end_label(s.tail).into(), P::Tail));
            v.push(Row::Toggle("Curved", s.curved, P::Curved));
            v.push(Row::Toggle("Shadow", s.shadow, P::Shadow));
            if sel.is_none() {
                v.push(Row::Note("Drag from tail to head. Shift keeps it horizontal or vertical. Curved arrows have two yellow handles to bend them.".into(), false));
            }
        }
        Tool::Rect | Tool::Ellipse | Tool::Polygon => {
            v.push(Row::Stepper("Outline", format!("{:.0}", s.stroke), P::Stroke));
            v.push(Row::Choice("Fill", fill_label(s.fill_alpha).into(), P::Fill));
            v.push(Row::Toggle("Shadow", s.shadow, P::Shadow));
            if sel.is_none() {
                let hint = if kind_tool == Tool::Polygon {
                    "Click to add points; double-click, Enter or click the first point to close."
                } else {
                    "Drag to draw. Shift makes a square / circle."
                };
                v.push(Row::Note(hint.into(), false));
            }
        }
        Tool::Callout => {
            v.push(Row::Stepper("Font", format!("{:.0}", s.font_size), P::FontSize));
            v.push(Row::Choice("Fill", (if s.fill_alpha == 0 { "White" } else { fill_label(s.fill_alpha) }).into(), P::Fill));
            v.push(Row::Stepper("Corners", format!("{:.0}", s.corner_radius), P::Radius));
            v.push(Row::Toggle("Shadow", s.shadow, P::Shadow));
            if let Some(Annotation::Callout { tails, .. }) = &sel {
                v.push(Row::Buttons(vec![("Add tail", P::AddTail), ("Remove tail", P::RemoveTail)]));
                v.push(Row::Note(format!("{} tail(s): drag the yellow handles to aim them.", tails.len()), false));
                v.push(Row::Button("Edit text", P::EditText));
            } else {
                v.push(Row::Note("Click where the tail should point (or drag the bubble), then type.".into(), false));
            }
        }
        Tool::Text => {
            v.push(Row::Stepper("Font", format!("{:.0}", s.font_size), P::FontSize));
            if sel.is_some() {
                v.push(Row::Button("Edit text", P::EditText));
            } else {
                v.push(Row::Note("Click or drag a box, then type. Enter commits, Ctrl+Enter adds a line.".into(), false));
            }
        }
        Tool::Step => {
            v.push(Row::Choice("Labels", (if s.step_letters { "A, B, C" } else { "1, 2, 3" }).into(), P::StepStyle));
            v.push(Row::Stepper("Size", format!("{:.0}", s.step_radius), P::StepSize));
            if sel.is_some() {
                v.push(Row::Button("Edit label", P::EditText));
            } else {
                v.push(Row::Note("Each click places the next marker. Double-click a marker to change its label.".into(), false));
            }
        }
        Tool::Highlighter => {
            v.push(Row::Note("Drag over an area to highlight it with the current colour.".into(), false));
        }
        Tool::Blur => {
            let mode = match s.blur_mode {
                BlurMode::Blur => "Blur",
                BlurMode::Pixelate => "Pixelate",
                BlurMode::Solid => "Solid",
            };
            v.push(Row::Choice("Mode", mode.into(), P::BlurMode));
            if s.blur_mode != BlurMode::Solid {
                v.push(Row::Stepper("Strength", format!("{:.0}", s.blur_intensity), P::Intensity));
                v.push(Row::Note("Warning: blur and pixelation may be reversible with third-party tools. Use Solid for sensitive data — on export you'll be offered to convert them.".into(), true));
            } else {
                v.push(Row::Note("Solid redaction is flattened into exported images and cannot be recovered.".into(), false));
            }
        }
        Tool::Stamp => {
            v.push(Row::Choice("Category", s.stamp_category.clone(), P::StampCategory));
            v.push(Row::Search);
            v.push(Row::Stamps(stamps::search(&s.stamp_category, &s.stamp_query)));
            v.push(Row::Button("Open stamps folder", P::OpenStamps));
        }
        Tool::Magnify => {
            let m = match &sel {
                Some(Annotation::Magnify { source, display, .. }) => display.w / source.w.max(1.0),
                _ => s.magnification,
            };
            v.push(Row::Stepper("Zoom", format!("{:.1}×", m), P::Magnification));
            v.push(Row::Stepper("Line", format!("{:.0}", (s.stroke * 0.5).max(1.0)), P::LineWidth));
            if sel.is_none() {
                v.push(Row::Note("Drag over the area to magnify. Then drag the inset to move it, or the dashed source box to re-aim it.".into(), false));
            }
        }
        Tool::Select => {
            let shape = match s.sel_shape {
                SelShape::Rectangle => "Rectangle",
                SelShape::Ellipse => "Ellipse",
                SelShape::Polygon => "Polygon",
                SelShape::Freehand => "Freehand",
            };
            v.push(Row::Choice("Shape", shape.into(), P::SelShape));
            v.push(Row::Choice("Fill gap", (if s.sel_autofill { "Auto-fill" } else { "Transparent" }).into(), P::SelBg));
            v.push(Row::Toggle("Snap", s.snap, P::Snap));
            if c.pixel_sel.is_some() {
                v.push(Row::Gap);
                v.push(Row::Buttons(vec![("Delete", P::SelDelete), ("Crop", P::SelCrop)]));
                v.push(Row::Buttons(vec![("Invert", P::SelInvert), ("Copy", P::SelCopy)]));
                v.push(Row::Buttons(vec![("Blur", P::Fx(0)), ("Pixelate", P::Fx(1))]));
                v.push(Row::Buttons(vec![("Grayscale", P::Fx(2)), ("Negative", P::Fx(3))]));
                v.push(Row::Buttons(vec![("Brighten", P::Fx(4)), ("Darken", P::Fx(5))]));
                v.push(Row::Note("Drag inside the selection to move those pixels.".into(), false));
            } else {
                v.push(Row::Note("Select pixels of the screenshot. With Snap on, a click selects the object under the cursor.".into(), false));
            }
        }
        Tool::CutOut => {
            v.push(Row::Choice("Remove", (if s.cut_horizontal { "Rows" } else { "Columns" }).into(), P::CutDir));
            let e = match s.cut_edge {
                CutEdge::Straight => "Straight",
                CutEdge::Zigzag => "Zigzag",
                CutEdge::Wave => "Wave",
            };
            v.push(Row::Choice("Edge", e.into(), P::CutEdge));
            v.push(Row::Note("Drag across the image to remove that band and join the rest.".into(), false));
        }
    }

    if let Some(a) = &sel {
        if a.rotatable() {
            v.push(Row::Stepper("Rotation", format!("{:.0}°", a.rotation()), P::Rotation));
        }
        v.push(Row::Gap);
        v.push(Row::Buttons(vec![("Front", P::Front), ("Back", P::Back)]));
        v.push(Row::Button("Delete", P::Delete));
    } else if kind_tool != Tool::Select && kind_tool != Tool::CutOut && !c.doc.annotations.is_empty() {
        v.push(Row::Gap);
        v.push(Row::Button("Flatten annotations", P::Flatten));
    }
    v
}

fn row_height(r: &Row, w: i32) -> i32 {
    match r {
        Row::Header(_) => 34,
        Row::Note(t, _) => {
            // Rough wrap estimate: ~6.3 px per char at 12 px Segoe UI.
            let chars_per_line = ((w - 2 * PAD) as f32 / 6.3).max(10.0);
            let lines = (t.chars().count() as f32 / chars_per_line).ceil() as i32 + 1;
            lines * 16 + 6
        }
        Row::Stamps(ids) => ((ids.len() as i32 + COLS - 1) / COLS).max(1) * CELL + 6,
        Row::Gap => 8,
        _ => ROW_H + 4,
    }
}

fn layout(st: &EditorState, w: i32) -> Vec<(Row, RECT)> {
    let mut y = 0;
    rows(st)
        .into_iter()
        .map(|r| {
            let h = row_height(&r, w);
            let rc = RECT { left: 0, top: y, right: w, bottom: y + h };
            y += h;
            (r, rc)
        })
        .collect()
}

pub fn register_class() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let cname = wide(CLASS_NAME);
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

thread_local! {
    static SEARCH: RefCell<HWND> = const { RefCell::new(0) };
    static SEARCH_FONT: RefCell<isize> = const { RefCell::new(0) };
    static EDIT_BRUSH: RefCell<isize> = const { RefCell::new(0) };
    /// Cached stamp grid render: (key, bitmap).
    static GRID: RefCell<Option<(String, Bitmap)>> = const { RefCell::new(None) };
}

pub fn create_child(parent: HWND, rect: RECT) -> Result<HWND> {
    register_class();
    let cname = wide(CLASS_NAME);
    unsafe {
        let hwnd = CreateWindowExW(
            0,
            cname.as_ptr(),
            core::ptr::null(),
            WS_CHILD | WS_VISIBLE,
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
        // Stamp search box (shown only for the Stamp tool).
        let edit = CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            core::ptr::null(),
            WS_CHILD | WS_BORDER | 0x0080, /* ES_AUTOHSCROLL */
            PAD,
            0,
            100,
            22,
            hwnd,
            SEARCH_ID as _,
            GetModuleHandleW(core::ptr::null()) as _,
            core::ptr::null(),
        );
        let font = make_font(13, FW_NORMAL as i32);
        SendMessageW(edit, WM_SETFONT, font as WPARAM, 1);
        // EM_SETCUEBANNER: placeholder text.
        SendMessageW(edit, 0x1501, 1, wide("Search stamps…").as_ptr() as LPARAM);
        SEARCH.with(|s| *s.borrow_mut() = edit);
        SEARCH_FONT.with(|s| *s.borrow_mut() = font);
        Ok(hwnd)
    }
}

fn make_font(px: i32, weight: i32) -> isize {
    unsafe {
        CreateFontW(
            -px,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            VARIABLE_PITCH as u32,
            wide("Segoe UI").as_ptr(),
        )
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        let parent = GetParent(hwnd);
        let Some(state) = window::get_state(parent) else {
            return DefWindowProcW(hwnd, msg, w, l);
        };
        match msg {
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = core::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rc: RECT = core::mem::zeroed();
                GetClientRect(hwnd, &mut rc);
                let cw = rc.right.max(1);
                let ch = rc.bottom.max(1);
                let mem_dc = CreateCompatibleDC(hdc);
                let mem_bmp = CreateCompatibleBitmap(hdc, cw, ch);
                let old = SelectObject(mem_dc, mem_bmp as _);
                if let Ok(st) = state.try_borrow() {
                    paint(mem_dc, cw, ch, &st);
                }
                BitBlt(hdc, 0, 0, cw, ch, mem_dc, 0, 0, SRCCOPY);
                SelectObject(mem_dc, old);
                DeleteObject(mem_bmp as _);
                DeleteDC(mem_dc);
                EndPaint(hwnd, &ps);
                0
            }
            WM_SIZE => {
                InvalidateRect(hwnd, core::ptr::null(), 0);
                0
            }
            WM_CTLCOLOREDIT => {
                let dc = w as HDC;
                SetTextColor(dc, C_VALUE);
                windows_sys::Win32::Graphics::Gdi::SetBkColor(dc, C_ELEVATED);
                EDIT_BRUSH.with(|b| {
                    let mut b = b.borrow_mut();
                    if *b == 0 {
                        *b = CreateSolidBrush(C_ELEVATED);
                    }
                    *b as LRESULT
                })
            }
            WM_COMMAND => {
                if (w & 0xFFFF) == SEARCH_ID && ((w >> 16) & 0xFFFF) as u32 == EN_CHANGE {
                    let q = read_text(l as HWND);
                    state.borrow_mut().canvas.style.stamp_query = q;
                    InvalidateRect(hwnd, core::ptr::null(), 0);
                }
                0
            }
            WM_LBUTTONDOWN => {
                let x = (l & 0xFFFF) as i16 as i32;
                let y = ((l >> 16) & 0xFFFF) as i16 as i32;
                let mut rc: RECT = core::mem::zeroed();
                GetClientRect(hwnd, &mut rc);
                let text_req = handle_click(parent, &state, x, y, rc.right);
                let cv = state.borrow().canvas_hwnd;
                InvalidateRect(cv, core::ptr::null(), 0);
                window::refresh_panels(parent);
                SetFocus(cv);
                if let Some(r) = text_req {
                    crate::editor::text_edit::spawn_for(parent, cv, r);
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

fn read_text(h: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(h);
        let mut buf = vec![0u16; (n + 1).max(1) as usize];
        let got = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..got.max(0) as usize])
    }
}

unsafe fn fill(hdc: HDC, r: RECT, c: u32) {
    unsafe {
        let b = CreateSolidBrush(c);
        FillRect(hdc, &r, b);
        DeleteObject(b as _);
    }
}

unsafe fn text(hdc: HDC, r: RECT, s: &str, color: u32, flags: u32) {
    unsafe {
        let w: Vec<u16> = s.encode_utf16().collect();
        let mut rr = r;
        SetTextColor(hdc, color);
        DrawTextW(hdc, w.as_ptr(), w.len() as i32, &mut rr, flags);
    }
}

unsafe fn button(hdc: HDC, r: RECT, label: &str, active: bool) {
    unsafe {
        let brush = CreateSolidBrush(if active { C_ACTIVE } else { C_HOVER });
        let pen = CreatePen(PS_SOLID, 1, if active { C_ACCENT } else { C_BORDER });
        let op = SelectObject(hdc, pen as _);
        let ob = SelectObject(hdc, brush as _);
        RoundRect(hdc, r.left, r.top, r.right, r.bottom, 6, 6);
        SelectObject(hdc, op);
        SelectObject(hdc, ob);
        DeleteObject(pen as _);
        DeleteObject(brush as _);
        text(hdc, r, label, C_VALUE, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
}

/// Split a row into the label area and the control area.
fn control_rect(r: RECT) -> RECT {
    RECT { left: r.left + PAD + LABEL_W, top: r.top + 2, right: r.right - PAD, bottom: r.top + 2 + ROW_H }
}

fn arrows(c: RECT) -> (RECT, RECT) {
    (
        RECT { left: c.left, top: c.top, right: c.left + BTN_W, bottom: c.bottom },
        RECT { left: c.right - BTN_W, top: c.top, right: c.right, bottom: c.bottom },
    )
}

fn paint(hdc: HDC, cw: i32, ch: i32, st: &EditorState) {
    unsafe {
        fill(hdc, RECT { left: 0, top: 0, right: cw, bottom: ch }, C_SURFACE);
        fill(hdc, RECT { left: 0, top: 0, right: 1, bottom: ch }, C_BORDER);
        SetBkMode(hdc, TRANSPARENT as i32);
        let font = make_font(13, FW_NORMAL as i32);
        let bold = make_font(14, FW_SEMIBOLD as i32);
        let small = make_font(12, FW_NORMAL as i32);
        let old_font = SelectObject(hdc, font as _);
        let mut search_at: Option<RECT> = None;

        for (row, r) in layout(st, cw) {
            match &row {
                Row::Header(t) => {
                    SelectObject(hdc, bold as _);
                    fill(hdc, RECT { left: 1, top: r.top, right: cw, bottom: r.bottom - 6 }, C_ELEVATED);
                    text(hdc, RECT { left: PAD, top: r.top, right: cw - PAD, bottom: r.bottom - 6 }, t, C_VALUE, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    SelectObject(hdc, font as _);
                }
                Row::Stepper(label, value, _) | Row::Choice(label, value, _) => {
                    text(hdc, RECT { left: PAD, top: r.top + 2, right: PAD + LABEL_W, bottom: r.top + 2 + ROW_H }, label, C_LABEL, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    let c = control_rect(r);
                    fill(hdc, c, C_ELEVATED);
                    let (a, b) = arrows(c);
                    let (la, ra) = if matches!(row, Row::Stepper(..)) { ("−", "+") } else { ("‹", "›") };
                    button(hdc, a, la, false);
                    button(hdc, b, ra, false);
                    text(hdc, RECT { left: a.right, right: b.left, ..c }, value, C_VALUE, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                }
                Row::Toggle(label, on, _) => {
                    text(hdc, RECT { left: PAD, top: r.top + 2, right: PAD + LABEL_W, bottom: r.top + 2 + ROW_H }, label, C_LABEL, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    let c = control_rect(r);
                    button(hdc, c, if *on { "On" } else { "Off" }, *on);
                }
                Row::Button(label, _) => {
                    button(hdc, RECT { left: PAD, top: r.top + 2, right: cw - PAD, bottom: r.top + 2 + ROW_H }, label, false);
                }
                Row::Buttons(items) => {
                    let n = items.len() as i32;
                    let gap = 6;
                    let bw = (cw - 2 * PAD - gap * (n - 1)) / n;
                    for (i, (label, _)) in items.iter().enumerate() {
                        let x = PAD + i as i32 * (bw + gap);
                        button(hdc, RECT { left: x, top: r.top + 2, right: x + bw, bottom: r.top + 2 + ROW_H }, label, false);
                    }
                }
                Row::Note(t, warn) => {
                    SelectObject(hdc, small as _);
                    let rr = RECT { left: PAD, top: r.top + 3, right: cw - PAD, bottom: r.bottom };
                    if *warn {
                        fill(hdc, RECT { left: PAD - 4, top: r.top, right: PAD - 2, bottom: r.bottom - 4 }, C_WARN);
                    }
                    text(hdc, rr, t, if *warn { C_WARN } else { C_LABEL }, DT_LEFT | DT_WORDBREAK);
                    SelectObject(hdc, font as _);
                }
                Row::Search => search_at = Some(RECT { left: PAD, top: r.top + 3, right: cw - PAD, bottom: r.top + 3 + 22 }),
                Row::Stamps(ids) => paint_stamps(hdc, st, ids, r, cw),
                Row::Gap => fill(hdc, RECT { left: PAD, top: r.top + 3, right: cw - PAD, bottom: r.top + 4 }, C_BORDER),
            }
        }

        // Position/show the search box.
        SEARCH.with(|s| {
            let h = *s.borrow();
            if h != 0 {
                match search_at {
                    Some(r) => {
                        SetWindowPos(h, 0, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER);
                        ShowWindow(h, SW_SHOW);
                    }
                    None => {
                        ShowWindow(h, SW_HIDE);
                    }
                }
            }
        });

        SelectObject(hdc, old_font);
        DeleteObject(font as _);
        DeleteObject(bold as _);
        DeleteObject(small as _);
    }
}

/// Render the stamp grid through the D2D renderer (so emoji are in colour),
/// cache the pixels, and blit them.
fn paint_stamps(hdc: HDC, st: &EditorState, ids: &[String], r: RECT, cw: i32) {
    let gw = (COLS * CELL) as u32;
    let rows_n = ((ids.len() as i32 + COLS - 1) / COLS).max(1);
    let gh = (rows_n * CELL) as u32;
    let selected = &st.canvas.style.stamp_asset;
    let key = format!("{}|{}|{}", ids.join(","), selected, gh);
    let cached = GRID.with(|g| g.borrow().as_ref().filter(|(k, _)| *k == key).map(|(_, b)| b.clone()));
    let bm = match cached {
        Some(b) => b,
        None => {
            let mut base = Bitmap::new(gw, gh);
            for px in base.bgra.chunks_exact_mut(4) {
                px.copy_from_slice(&[0x20, 0x1E, 0x1C, 0xFF]);
            }
            let mut doc = crate::editor::annotation::Document::new(base);
            for (i, id) in ids.iter().enumerate() {
                let (cx, cy) = ((i as i32 % COLS * CELL) as f32, (i as i32 / COLS * CELL) as f32);
                if id == selected {
                    doc.annotations.push(Annotation::Shape {
                        id: 0,
                        kind: ShapeKind::Rectangle,
                        bounds: Rect { x: cx + 1.0, y: cy + 1.0, w: CELL as f32 - 2.0, h: CELL as f32 - 2.0 },
                        points: Vec::new(),
                        outline: Color::rgb(0x4A, 0x80, 0xD0),
                        fill: Color::rgb(0x3A, 0x3D, 0x42),
                        stroke: 1.5,
                        rotation: 0.0,
                        shadow: false,
                    });
                }
                let asp = stamps::aspect(id).min(2.6);
                let h = if asp > 1.2 { 26.0 / asp.max(1.0) * 1.3 } else { 26.0 };
                let w = (h * asp).min(32.0);
                let h = w / asp;
                doc.annotations.push(Annotation::Stamp {
                    id: 0,
                    asset: id.clone(),
                    bounds: Rect { x: cx + (CELL as f32 - w) * 0.5, y: cy + (CELL as f32 - h) * 0.5, w, h },
                    rotation: 0.0,
                });
            }
            let b = crate::editor::render::flatten(&st.canvas.gfx, &doc, Default::default()).unwrap_or(Bitmap::new(gw, gh));
            GRID.with(|g| *g.borrow_mut() = Some((key, b.clone())));
            b
        }
    };
    let x0 = ((cw - gw as i32) / 2).max(PAD);
    unsafe {
        let mut bi: BITMAPINFO = core::mem::zeroed();
        bi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = bm.width as i32;
        bi.bmiHeader.biHeight = -(bm.height as i32);
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB as u32;
        SetDIBitsToDevice(hdc, x0, r.top + 3, bm.width, bm.height, 0, 0, 0, bm.height, bm.bgra.as_ptr() as _, &bi, DIB_RGB_COLORS);
        if ids.is_empty() {
            text(hdc, RECT { left: PAD, top: r.top, right: cw - PAD, bottom: r.bottom }, "No stamps match.", C_LABEL, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
    }
}

fn handle_click(parent: HWND, state: &Rc<RefCell<EditorState>>, x: i32, y: i32, cw: i32) -> Option<crate::editor::canvas::TextEditReq> {
    let lay = layout(&state.borrow(), cw);
    for (row, r) in lay {
        if y < r.top || y >= r.bottom {
            continue;
        }
        let c = control_rect(r);
        let (a, b) = arrows(c);
        let dir = if x >= a.left && x < a.right {
            -1
        } else if x >= b.left && x < b.right {
            1
        } else {
            0
        };
        match row {
            Row::Stepper(_, _, p) => {
                if dir != 0 {
                    apply(parent, state, p, dir);
                }
            }
            Row::Choice(_, _, p) => {
                if x >= c.left {
                    apply(parent, state, p, if dir == 0 { 1 } else { dir });
                }
            }
            Row::Toggle(_, _, p) => {
                if x >= c.left {
                    apply(parent, state, p, 1);
                }
            }
            Row::Button(_, p) => return apply(parent, state, p, 1),
            Row::Buttons(items) => {
                let n = items.len() as i32;
                let gap = 6;
                let bw = (cw - 2 * PAD - gap * (n - 1)) / n;
                let i = ((x - PAD) / (bw + gap)).clamp(0, n - 1) as usize;
                return apply(parent, state, items[i].1, 1);
            }
            Row::Stamps(ids) => {
                let x0 = ((cw - COLS * CELL) / 2).max(PAD);
                let col = (x - x0) / CELL;
                let rw = (y - r.top - 3) / CELL;
                if col >= 0 && col < COLS && rw >= 0 {
                    let i = (rw * COLS + col) as usize;
                    if let Some(id) = ids.get(i) {
                        let mut st = state.borrow_mut();
                        st.canvas.style.stamp_asset = id.clone();
                        let id = id.clone();
                        st.canvas.mutate_selected(&|a| match a {
                            Annotation::Stamp { asset, bounds, .. } => {
                                *asset = id.clone();
                                let asp = stamps::aspect(&id);
                                bounds.w = bounds.h * asp;
                                true
                            }
                            _ => false,
                        });
                        if st.canvas.active_tool != Tool::Stamp && st.canvas.single().is_none() {
                            st.canvas.set_tool(Tool::Stamp);
                        }
                    }
                }
            }
            _ => {}
        }
        break;
    }
    None
}

fn cycle<T: Copy + PartialEq>(opts: &[T], cur: T, dir: i32) -> T {
    let i = opts.iter().position(|o| *o == cur).unwrap_or(0) as i32;
    let n = opts.len() as i32;
    opts[((i + dir).rem_euclid(n)) as usize]
}

/// Recolour an annotation (used by the toolbar palette).
pub fn apply_color(a: &mut Annotation, c: Color) -> bool {
    match a {
        Annotation::Arrow { color, .. } | Annotation::Text { color, .. } | Annotation::Highlighter { color, .. } | Annotation::Magnify { color, .. } => *color = c,
        Annotation::Shape { outline, fill, .. } => {
            *outline = c;
            if fill.a > 0 {
                *fill = Color { a: fill.a, ..c };
            }
        }
        Annotation::Callout { outline, fill, text_color, .. } => {
            *outline = c;
            if *fill != Color::WHITE && fill.a > 0 {
                *fill = Color { a: fill.a, ..c };
                if fill.a == 255 {
                    *text_color = if c == Color::WHITE || c == Color::YELLOW { Color::BLACK } else { Color::WHITE };
                }
            }
        }
        Annotation::Step { fill, text, .. } => {
            *fill = c;
            *text = if c == Color::WHITE || c == Color::YELLOW { Color::BLACK } else { Color::WHITE };
        }
        Annotation::Blur { mode: BlurMode::Solid, fill, .. } => *fill = c,
        _ => return false,
    }
    true
}

fn apply(parent: HWND, state: &Rc<RefCell<EditorState>>, p: P, dir: i32) -> Option<crate::editor::canvas::TextEditReq> {
    let d = dir as f32;
    let mut st = state.borrow_mut();
    let c = &mut st.canvas;
    let s = &mut c.style;
    match p {
        P::Stroke => {
            s.stroke = (s.stroke + d).clamp(1.0, 30.0);
            let v = s.stroke;
            c.mutate_selected(&|a| match a {
                Annotation::Arrow { stroke, .. } | Annotation::Shape { stroke, .. } => {
                    *stroke = v;
                    true
                }
                _ => false,
            });
        }
        P::LineWidth => {
            s.stroke = (s.stroke + 2.0 * d).clamp(2.0, 30.0);
            let v = (s.stroke * 0.5).max(1.0);
            c.mutate_selected(&|a| match a {
                Annotation::Magnify { line_width, .. } => {
                    *line_width = v;
                    true
                }
                _ => false,
            });
        }
        P::FontSize => {
            s.font_size = (s.font_size + 2.0 * d).clamp(8.0, 120.0);
            let v = s.font_size;
            let gfx = c.gfx.clone();
            c.mutate_selected(&|a| match a {
                Annotation::Text { size, bounds, text, .. } => {
                    *size = v;
                    let (_, h) = crate::editor::render::measure_text(&gfx, text, v, bounds.w);
                    bounds.h = h.max(v * 1.4) + 2.0;
                    true
                }
                Annotation::Callout { size, .. } => {
                    *size = v;
                    true
                }
                _ => false,
            });
        }
        P::Fill => {
            s.fill_alpha = cycle(&[0u8, 90, 255], s.fill_alpha, dir);
            let (fa, col) = (s.fill_alpha, s.color);
            c.mutate_selected(&|a| match a {
                Annotation::Shape { fill, outline, .. } => {
                    *fill = Color { a: fa, ..*outline };
                    true
                }
                Annotation::Callout { fill, outline, text_color, .. } => {
                    *fill = if fa == 0 { Color::WHITE } else { Color { a: fa, ..*outline } };
                    *text_color = if fa == 255 && *outline != Color::WHITE && *outline != Color::YELLOW { Color::WHITE } else { Color::rgb(0x20, 0x20, 0x20) };
                    let _ = col;
                    true
                }
                _ => false,
            });
        }
        P::Shadow => {
            s.shadow = !s.shadow;
            let v = s.shadow;
            c.mutate_selected(&|a| match a {
                Annotation::Arrow { shadow, .. } | Annotation::Shape { shadow, .. } | Annotation::Callout { shadow, .. } => {
                    *shadow = v;
                    true
                }
                _ => false,
            });
        }
        P::Head | P::Tail => {
            let opts = [EndStyle::Arrow, EndStyle::Circle, EndStyle::Square, EndStyle::None];
            let head = p == P::Head;
            let v = if head {
                s.head = cycle(&opts, s.head, dir);
                s.head
            } else {
                s.tail = cycle(&opts, s.tail, dir);
                s.tail
            };
            c.mutate_selected(&|a| match a {
                Annotation::Arrow { head: h, tail: t, .. } => {
                    if head { *h = v } else { *t = v }
                    true
                }
                _ => false,
            });
        }
        P::Curved => {
            s.curved = !s.curved;
            let v = s.curved;
            c.mutate_selected(&|a| match a {
                Annotation::Arrow { bezier, start, end, .. } => {
                    *bezier = if v { Some(crate::editor::annotation::default_bezier(*start, *end)) } else { None };
                    true
                }
                _ => false,
            });
        }
        P::BlurMode => {
            s.blur_mode = cycle(&[BlurMode::Blur, BlurMode::Pixelate, BlurMode::Solid], s.blur_mode, dir);
            let v = s.blur_mode;
            c.mutate_selected(&|a| match a {
                Annotation::Blur { mode, .. } => {
                    *mode = v;
                    true
                }
                _ => false,
            });
        }
        P::Intensity => {
            s.blur_intensity = (s.blur_intensity + 2.0 * d).clamp(2.0, 60.0);
            let v = s.blur_intensity;
            c.mutate_selected(&|a| match a {
                Annotation::Blur { intensity, .. } => {
                    *intensity = v;
                    true
                }
                _ => false,
            });
        }
        P::StepStyle => {
            s.step_letters = !s.step_letters;
            let letters = s.step_letters;
            // Convert the selected marker's label to the other style.
            c.mutate_selected(&|a| match a {
                Annotation::Step { value, .. } => {
                    let n = if letters { value.parse::<u32>().ok() } else { Some(value.bytes().fold(0u32, |acc, b| acc * 26 + (b.to_ascii_uppercase().wrapping_sub(b'A') as u32 + 1))) };
                    if let Some(n) = n {
                        *value = crate::editor::annotation::step_label(n.max(1), letters);
                    }
                    true
                }
                _ => false,
            });
        }
        P::StepSize => {
            s.step_radius = (s.step_radius + 2.0 * d).clamp(8.0, 64.0);
            let v = s.step_radius;
            c.mutate_selected(&|a| match a {
                Annotation::Step { radius, .. } => {
                    *radius = v;
                    true
                }
                _ => false,
            });
        }
        P::Magnification => {
            let cur = match c.single() {
                Some(Annotation::Magnify { source, display, .. }) => display.w / source.w.max(1.0),
                _ => c.style.magnification,
            };
            let v = ((cur * 2.0).round() / 2.0 + 0.5 * d).clamp(1.5, 8.0);
            c.style.magnification = v;
            c.mutate_selected(&|a| match a {
                Annotation::Magnify { source, display, .. } => {
                    // Keep the inset size; re-aim the source around its centre.
                    let ctr = source.center();
                    let (w, h) = (display.w / v, display.h / v);
                    *source = Rect { x: ctr.x - w * 0.5, y: ctr.y - h * 0.5, w, h };
                    true
                }
                _ => false,
            });
        }
        P::Radius => {
            s.corner_radius = (s.corner_radius + 4.0 * d).clamp(0.0, 60.0);
            let v = s.corner_radius;
            c.mutate_selected(&|a| match a {
                Annotation::Callout { radius, .. } => {
                    *radius = v;
                    true
                }
                _ => false,
            });
        }
        P::Rotation => {
            c.mutate_selected(&|a| {
                let r = ((a.rotation() / 15.0).round() + d) * 15.0;
                a.set_rotation(r);
                true
            });
        }
        P::AddTail => {
            c.mutate_selected(&|a| match a {
                Annotation::Callout { bounds, tails, .. } => {
                    let n = tails.len() as f32;
                    let tip = Point::new(bounds.right() + 40.0 + n * 10.0, bounds.bottom() + 40.0 + n * 30.0);
                    tails.push(Tail { anchor: Point::new(0.5, 0.5), tip });
                    true
                }
                _ => false,
            });
        }
        P::RemoveTail => {
            c.mutate_selected(&|a| match a {
                Annotation::Callout { tails, .. } if !tails.is_empty() => {
                    tails.pop();
                    true
                }
                _ => false,
            });
        }
        P::EditText => {
            if let Some(id) = c.selected.first().copied() {
                return c.text_edit_request(id);
            }
        }
        P::SelShape => s.sel_shape = cycle(&[SelShape::Rectangle, SelShape::Ellipse, SelShape::Polygon, SelShape::Freehand], s.sel_shape, dir),
        P::SelBg => s.sel_autofill = !s.sel_autofill,
        P::Snap => s.snap = !s.snap,
        P::CutDir => s.cut_horizontal = !s.cut_horizontal,
        P::CutEdge => s.cut_edge = cycle(&[CutEdge::Straight, CutEdge::Zigzag, CutEdge::Wave], s.cut_edge, dir),
        P::StampCategory => {
            let cats = stamps::categories();
            let i = cats.iter().position(|x| *x == s.stamp_category).unwrap_or(0) as i32;
            s.stamp_category = cats[((i + dir).rem_euclid(cats.len() as i32)) as usize].clone();
        }
        P::OpenStamps => {
            let dir = stamps::stamps_dir();
            let _ = std::fs::create_dir_all(&dir);
            stamps::refresh();
            unsafe {
                windows_sys::Win32::UI::Shell::ShellExecuteW(parent, wide("open").as_ptr(), wide(&dir).as_ptr(), core::ptr::null(), core::ptr::null(), 1);
            }
        }
        P::Group => {
            c.group();
        }
        P::Ungroup => {
            c.ungroup();
        }
        P::Delete => {
            c.delete_selection();
        }
        P::Front => {
            c.reorder(true, false);
        }
        P::Back => {
            c.reorder(false, false);
        }
        P::Flatten => {
            c.flatten_annotations();
        }
        P::SelDelete => {
            let auto = c.style.sel_autofill;
            c.sel_fill(auto);
        }
        P::SelCrop => {
            c.sel_crop();
        }
        P::SelInvert => {
            c.sel_invert();
        }
        P::SelCopy => {
            c.copy();
        }
        P::Fx(i) => {
            let fx = [Effect::Blur, Effect::Pixelate, Effect::Grayscale, Effect::Invert, Effect::Brighten, Effect::Darken][i as usize];
            c.sel_effect(fx);
        }
    }
    None
}
