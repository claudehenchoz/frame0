//! Property panel child window.
//!
//! GDI-rendered strip along the right side of the editor. Shows
//! tool-specific style properties (stroke width, font size, blur
//! intensity, fill toggle, stamp picker) with +/- steppers.
//! Lives as a sibling of the toolbar and canvas.

use crate::editor::annotation::Annotation;
use crate::editor::history::Command;
use crate::editor::stamps;
use crate::editor::tools::{Tool, ToolStyle};
use crate::editor::window::EditorState;
use crate::error::{Error, Result};
use crate::win::wide;
use std::cell::RefCell;
use std::rc::Rc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, InvalidateRect, PAINTSTRUCT,
    SelectObject, SetBkMode, SetTextColor, SRCCOPY, TRANSPARENT, TextOutW,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, FW_NORMAL, OUT_DEFAULT_PRECIS,
    VARIABLE_PITCH,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetParent, IDC_ARROW, LoadCursorW,
    RegisterClassExW, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_PAINT, WNDCLASSEXW, WS_CHILD, WS_VISIBLE,
};

const CLASS_NAME: &str = "frame.editor.property";

const ROW_H: i32 = 28;
const LABEL_X: i32 = 10;
const VALUE_X: i32 = 100;
const BTN_X: i32 = 130;
const BTN_W: i32 = 20;
const BTN_H: i32 = 20;
const BTN_GAP: i32 = 24;
const STAMP_SIZE: i32 = 40;
const STAMP_GAP: i32 = 6;

fn tool_name(t: Tool) -> &'static str {
    match t {
        Tool::Move => "Move",
        Tool::SelectionRect => "Select",
        Tool::Arrow => "Arrow",
        Tool::BezierArrow => "Curve",
        Tool::ShapeRect => "Rect",
        Tool::ShapeEllipse => "Ellipse",
        Tool::Text => "Text",
        Tool::Step => "Step",
        Tool::Highlighter => "Hilite",
        Tool::BlurGaussian => "Blur",
        Tool::BlurSolid => "Redact",
        Tool::Magnify => "Magnify",
        Tool::Stamp => "Stamp",
    }
}

enum PropRow {
    Title,
    ToolName,
    Stroke,
    FontSize,
    FillToggle,
    BlurIntensity,
    StampPicker,
}

fn get_rows(tool: Tool) -> Vec<PropRow> {
    let mut rows = vec![PropRow::Title, PropRow::ToolName];
    match tool {
        Tool::Arrow | Tool::BezierArrow | Tool::ShapeRect | Tool::ShapeEllipse | Tool::Magnify => {
            rows.push(PropRow::Stroke);
        }
        _ => {}
    }
    if matches!(tool, Tool::Text) {
        rows.push(PropRow::FontSize);
    }
    if matches!(tool, Tool::ShapeRect | Tool::ShapeEllipse) {
        rows.push(PropRow::FillToggle);
    }
    if matches!(tool, Tool::BlurGaussian) {
        rows.push(PropRow::BlurIntensity);
    }
    if matches!(tool, Tool::Stamp) {
        rows.push(PropRow::StampPicker);
    }
    rows
}

fn row_y(index: usize) -> i32 {
    (8 + index as i32 * ROW_H).max(8)
}

fn make_segoe(size: i32) -> isize {
    unsafe {
        CreateFontW(
            -size,
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
        )
    }
}

fn draw_label(hdc: isize, row: i32, text: &str) {
    unsafe {
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, 0x00E0E0E0);
        let w = wide(text);
        TextOutW(hdc, LABEL_X, row + 5, w.as_ptr(), (w.len() - 1) as i32);
    }
}

fn draw_value(hdc: isize, row: i32, text: &str) {
    unsafe {
        SetTextColor(hdc, 0x00FFFFFF);
        let w = wide(text);
        TextOutW(hdc, VALUE_X, row + 5, w.as_ptr(), (w.len() - 1) as i32);
    }
}

fn draw_buttons(hdc: isize, row: i32) -> (RECT, RECT) {
    unsafe {
        let minus_r = RECT {
            left: BTN_X,
            top: row,
            right: BTN_X + BTN_W,
            bottom: row + BTN_H,
        };
        let plus_r = RECT {
            left: BTN_X + BTN_GAP,
            top: row,
            right: BTN_X + BTN_GAP + BTN_W,
            bottom: row + BTN_H,
        };
        let btn_bg = CreateSolidBrush(0x00404850);
        FillRect(hdc, &minus_r, btn_bg);
        FillRect(hdc, &plus_r, btn_bg);
        DeleteObject(btn_bg as _);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, 0x00FFFFFF);
        let minus = wide("-");
        let plus = wide("+");
        TextOutW(hdc, minus_r.left + 6, minus_r.top + 2, minus.as_ptr(), 1);
        TextOutW(hdc, plus_r.left + 6, plus_r.top + 2, plus.as_ptr(), 1);
        (minus_r, plus_r)
    }
}

fn draw_stamp_swatch(hdc: isize, x: i32, y: i32, asset: u8, selected: bool) -> RECT {
    let r = RECT {
        left: x,
        top: y,
        right: x + STAMP_SIZE,
        bottom: y + STAMP_SIZE,
    };
    unsafe {
        let bg = CreateSolidBrush(if selected { 0x00505964 } else { 0x002E3236 });
        FillRect(hdc, &r, bg);
        DeleteObject(bg as _);
        // Draw the stamp's label character centered in the swatch.
        let label = stamps::label(asset);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, 0x00E0E0E0);
        let w = wide(label);
        // Center the label (assumes ~1 char wide stamps).
        TextOutW(hdc, x + (STAMP_SIZE - 12) / 2, y + 10, w.as_ptr(), (w.len() - 1) as i32);
    }
    r
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
        Ok(hwnd)
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        let parent = GetParent(hwnd);
        let state = crate::editor::window::get_state(parent);
        let Some(state) = state else {
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
                paint(mem_dc, cw, ch, &state.borrow());
                BitBlt(hdc, 0, 0, cw, ch, mem_dc, 0, 0, SRCCOPY);
                SelectObject(mem_dc, old);
                DeleteObject(mem_bmp as _);
                DeleteDC(mem_dc);
                EndPaint(hwnd, &ps);
                0
            }
            WM_LBUTTONDOWN => {
                let x = (l & 0xFFFF) as i16 as i32;
                let y = ((l >> 16) & 0xFFFF) as i16 as i32;
                handle_click(&state, x, y);
                InvalidateRect(hwnd, core::ptr::null(), 0);
                let st = state.borrow();
                if st.canvas_hwnd != 0 {
                    InvalidateRect(st.canvas_hwnd, core::ptr::null(), 0);
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

fn paint(
    hdc: isize,
    cw: i32,
    ch: i32,
    st: &EditorState,
) {
    unsafe {
        let full = RECT { left: 0, top: 0, right: cw, bottom: ch };
        let bg = CreateSolidBrush(0x001A1B1D);
        FillRect(hdc, &full, bg);
        DeleteObject(bg as _);

        let title_bar = RECT { left: 0, top: 0, right: cw, bottom: 28 };
        let title_bg = CreateSolidBrush(0x00222426);
        FillRect(hdc, &title_bar, title_bg);
        DeleteObject(title_bg as _);

        let font = make_segoe(13);
        let old_font = SelectObject(hdc, font as _);

        let rows = get_rows(st.canvas.active_tool);
        let style = &st.canvas.style;

        for (i, row) in rows.iter().enumerate() {
            let ry = row_y(i);
            match row {
                PropRow::Title => {
                    SetBkMode(hdc, TRANSPARENT as i32);
                    SetTextColor(hdc, 0x00CCCCCC);
                    let w = wide("Properties");
                    TextOutW(hdc, 8, 6, w.as_ptr(), (w.len() - 1) as i32);
                }
                PropRow::ToolName => {
                    draw_label(hdc, ry, "Tool:");
                    draw_value(hdc, ry, tool_name(st.canvas.active_tool));
                }
                PropRow::Stroke => {
                    draw_label(hdc, ry, "Stroke:");
                    let val = format!("{:.0}", style.stroke);
                    draw_value(hdc, ry, &val);
                    draw_buttons(hdc, ry);
                }
                PropRow::FontSize => {
                    draw_label(hdc, ry, "Font:");
                    let val = format!("{:.0}", style.font_size);
                    draw_value(hdc, ry, &val);
                    draw_buttons(hdc, ry);
                }
                PropRow::FillToggle => {
                    draw_label(hdc, ry, "Fill:");
                    let label = if style.fill_alpha > 0 { "On " } else { "Off" };
                    draw_value(hdc, ry, label);
                    draw_buttons(hdc, ry);
                }
                PropRow::BlurIntensity => {
                    draw_label(hdc, ry, "Blur:");
                    let val = format!("{:.0}", style.blur_intensity);
                    draw_value(hdc, ry, &val);
                    draw_buttons(hdc, ry);
                }
                PropRow::StampPicker => {
                    let sx = 10i32;
                    for asset in 0..stamps::COUNT.min(4) as u8 {
                        let sely = style.stamp_asset == asset;
                        let sr = draw_stamp_swatch(hdc, sx + asset as i32 * (STAMP_SIZE + STAMP_GAP), ry, asset, sely);
                        // Track stamp picker rects via static for click handling? We'll recompute.
                        let _ = sr;
                    }
                }
            }
        }

        // If an annotation is selected, show a summary row.
        if let Some(sel) = st.canvas.selected {
            if let Some(idx) = st.canvas.doc.find_index(sel) {
                let ann = &st.canvas.doc.annotations[idx];
                let ann_name = match ann {
                    Annotation::Arrow { .. } => "Arrow",
                    Annotation::Shape { kind, .. } => match kind {
                        crate::editor::annotation::ShapeKind::Rectangle => "Rect",
                        crate::editor::annotation::ShapeKind::Ellipse => "Ellipse",
                    },
                    Annotation::Text { .. } => "Text",
                    Annotation::Step { .. } => "Step",
                    Annotation::Highlighter { .. } => "Hilite",
                    Annotation::Blur { .. } => "Blur",
                    Annotation::Magnify { .. } => "Magnify",
                    Annotation::Stamp { .. } => "Stamp",
                };
                let sel_y = row_y(rows.len()) + 8;
                let sep = RECT { left: 4, top: sel_y - 4, right: cw - 4, bottom: sel_y - 2 };
                let sep_br = CreateSolidBrush(0x00303438);
                FillRect(hdc, &sep, sep_br);
                DeleteObject(sep_br as _);
                SetBkMode(hdc, TRANSPARENT as i32);
                SetTextColor(hdc, 0x00AAAAAA);
                let info = format!("Selected: {}", ann_name);
                let w = wide(&info);
                TextOutW(hdc, 8, sel_y + 2, w.as_ptr(), (w.len() - 1) as i32);
            }
        }

        SelectObject(hdc, old_font);
        DeleteObject(font as _);
    }
}

fn handle_click(state: &Rc<RefCell<EditorState>>, x: i32, y: i32) {
    let rows = get_rows(state.borrow().canvas.active_tool);
    for (i, row) in rows.iter().enumerate() {
        let ry = row_y(i);
        if y < ry || y >= ry + ROW_H {
            continue;
        }
        // Check if click is on - or + button areas.
        let on_minus = x >= BTN_X && x < BTN_X + BTN_W;
        let on_plus = x >= BTN_X + BTN_GAP && x < BTN_X + BTN_GAP + BTN_W;
        if !on_minus && !on_plus && !matches!(row, PropRow::StampPicker) {
            continue;
        }
        let mut st = state.borrow_mut();
        match row {
            PropRow::Stroke => {
                let delta = if on_plus { 1.0 } else { -1.0 };
                st.canvas.style.stroke = (st.canvas.style.stroke + delta).clamp(1.0, 20.0);
                let new_val = st.canvas.style.stroke;
                maybe_mutate_selected(&mut st, |ann| match ann {
                    Annotation::Arrow { stroke, .. } | Annotation::Shape { stroke, .. } => {
                        *stroke = new_val;
                        true
                    }
                    _ => false,
                });
            }
            PropRow::FontSize => {
                let delta = if on_plus { 2.0 } else { -2.0 };
                st.canvas.style.font_size = (st.canvas.style.font_size + delta).clamp(8.0, 72.0);
                let new_val = st.canvas.style.font_size;
                maybe_mutate_selected(&mut st, |ann| match ann {
                    Annotation::Text { size, .. } => {
                        *size = new_val;
                        true
                    }
                    _ => false,
                });
            }
            PropRow::FillToggle => {
                st.canvas.style.fill_alpha = if st.canvas.style.fill_alpha > 0 { 0 } else { 100 };
                let new_val = st.canvas.style.fill_alpha;
                maybe_mutate_selected(&mut st, |ann| match ann {
                    Annotation::Shape { fill, .. } => {
                        fill.a = new_val;
                        true
                    }
                    _ => false,
                });
            }
            PropRow::BlurIntensity => {
                let delta = if on_plus { 1.0 } else { -1.0 };
                st.canvas.style.blur_intensity =
                    (st.canvas.style.blur_intensity + delta).clamp(1.0, 30.0);
                let new_val = st.canvas.style.blur_intensity;
                maybe_mutate_selected(&mut st, |ann| match ann {
                    Annotation::Blur { intensity, .. } => {
                        *intensity = new_val;
                        true
                    }
                    _ => false,
                });
            }
            PropRow::StampPicker => {
                for asset in 0..stamps::COUNT.min(4) as u8 {
                    let sx = 10 + asset as i32 * (STAMP_SIZE + STAMP_GAP);
                    if x >= sx && x < sx + STAMP_SIZE && y >= ry && y < ry + STAMP_SIZE {
                        st.canvas.style.stamp_asset = asset;
                        let chosen_asset = asset;
                        maybe_mutate_selected(&mut st, |ann| match ann {
                            Annotation::Stamp { asset, .. } => {
                                *asset = chosen_asset;
                                true
                            }
                            _ => false,
                        });
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}

fn maybe_mutate_selected<F>(st: &mut std::cell::RefMut<'_, EditorState>, f: F)
where
    F: FnOnce(&mut Annotation) -> bool,
{
    let Some(sel) = st.canvas.selected else { return };
    let Some(idx) = st.canvas.doc.find_index(sel) else { return };
    let before = st.canvas.doc.annotations[idx].clone();
    let mut after = before.clone();
    if f(&mut after) {
        st.canvas.doc.annotations[idx] = after.clone();
        st.history.push(Command::Mutate {
            id: sel,
            before,
            after,
        });
    }
}
