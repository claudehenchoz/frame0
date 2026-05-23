//! Toolbar child window. GDI-rendered strip across the top of the editor.
//! Owns its own WM_PAINT (double-buffered) and WM_LBUTTONDOWN — translates
//! clicks into tool / palette changes on the shared `EditorState`.

use crate::editor::tools::Tool;
use crate::error::{Error, Result};
use crate::gfx::color::Color;
use crate::win::wide;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt as GdiBitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, InvalidateRect, PAINTSTRUCT, SRCCOPY,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetParent, IDC_ARROW, LoadCursorW,
    RegisterClassExW, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_PAINT, WNDCLASSEXW, WS_CHILD, WS_VISIBLE,
};

#[derive(Clone, Copy, Debug)]
pub struct ToolEntry {
    pub tool: Tool,
    pub label: &'static str,
}

pub const TOOLS: &[ToolEntry] = &[
    ToolEntry { tool: Tool::Move, label: "Move" },
    ToolEntry { tool: Tool::SelectionRect, label: "Select" },
    ToolEntry { tool: Tool::Arrow, label: "Arrow" },
    ToolEntry { tool: Tool::BezierArrow, label: "Curve" },
    ToolEntry { tool: Tool::ShapeRect, label: "Rect" },
    ToolEntry { tool: Tool::ShapeEllipse, label: "Ellipse" },
    ToolEntry { tool: Tool::Text, label: "Text" },
    ToolEntry { tool: Tool::Step, label: "Step" },
    ToolEntry { tool: Tool::Highlighter, label: "Hilite" },
    ToolEntry { tool: Tool::BlurGaussian, label: "Blur" },
    ToolEntry { tool: Tool::BlurSolid, label: "Redact" },
    ToolEntry { tool: Tool::Magnify, label: "Magnify" },
    ToolEntry { tool: Tool::Stamp, label: "Stamp" },
];

pub const PALETTE: &[Color] = &[
    Color::RED,
    Color::BLUE,
    Color::GREEN,
    Color::YELLOW,
    Color::BLACK,
    Color::WHITE,
];

pub const SWATCH_SIZE: i32 = 24;
pub const BUTTON_W: i32 = 60;
pub const BUTTON_STRIDE: i32 = 64;

const CLASS_NAME: &str = "frame.editor.toolbar";

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
    let parent = unsafe { GetParent(hwnd) };
    let state = crate::editor::window::get_state(parent);
    let Some(state) = state else {
        return unsafe { DefWindowProcW(hwnd, msg, w, l) };
    };
    unsafe {
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
                GdiBitBlt(hdc, 0, 0, cw, ch, mem_dc, 0, 0, SRCCOPY);
                SelectObject(mem_dc, old);
                DeleteObject(mem_bmp as _);
                DeleteDC(mem_dc);
                EndPaint(hwnd, &ps);
                0
            }
            WM_LBUTTONDOWN => {
                let x = (l & 0xFFFF) as i16 as i32;
                handle_click(&state, x);
                InvalidateRect(hwnd, core::ptr::null(), 0);
                // Repaint the canvas too (selected-color outline lives on this state).
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

unsafe fn paint(
    hdc: windows_sys::Win32::Graphics::Gdi::HDC,
    cw: i32,
    ch: i32,
    st: &crate::editor::window::EditorState,
) {
    unsafe {
        let full = RECT { left: 0, top: 0, right: cw, bottom: ch };
        let bg = CreateSolidBrush(0x00222426);
        FillRect(hdc, &full, bg);
        DeleteObject(bg as _);

        let mut x = 8;
        for entry in TOOLS {
            draw_tool_button(hdc, x, 8, ch, entry, st.canvas.active_tool == entry.tool);
            x += BUTTON_STRIDE;
        }
        x += 12;
        for c in PALETTE {
            let top = (ch - SWATCH_SIZE) / 2;
            let r = RECT {
                left: x,
                top,
                right: x + SWATCH_SIZE,
                bottom: top + SWATCH_SIZE,
            };
            let bgr = (c.b as u32) | ((c.g as u32) << 8) | ((c.r as u32) << 16);
            let br = CreateSolidBrush(bgr);
            FillRect(hdc, &r, br);
            DeleteObject(br as _);
            if *c == st.canvas.style.color {
                let outline = CreateSolidBrush(0x00FFFFFF);
                let or = RECT {
                    left: r.left - 2,
                    top: r.top - 2,
                    right: r.right + 2,
                    bottom: r.top - 1,
                };
                FillRect(hdc, &or, outline);
                DeleteObject(outline as _);
            }
            x += SWATCH_SIZE + 6;
        }
    }
}

unsafe fn draw_tool_button(
    hdc: windows_sys::Win32::Graphics::Gdi::HDC,
    x: i32,
    y: i32,
    ch: i32,
    entry: &ToolEntry,
    active: bool,
) {
    unsafe {
        let r = RECT { left: x, top: y, right: x + BUTTON_W, bottom: ch - 8 };
        let color = if active { 0x00505964 } else { 0x002E3236 };
        let bg = CreateSolidBrush(color);
        FillRect(hdc, &r, bg);
        DeleteObject(bg as _);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, 0x00E0E0E0);
        let label = wide(entry.label);
        TextOutW(hdc, x + 8, y + 4, label.as_ptr(), (label.len() - 1) as i32);
    }
}

fn handle_click(state: &std::rc::Rc<std::cell::RefCell<crate::editor::window::EditorState>>, x: i32) {
    let mut st = state.borrow_mut();
    let mut cx = 8;
    for entry in TOOLS {
        if x >= cx && x < cx + BUTTON_W {
            st.canvas.active_tool = entry.tool;
            return;
        }
        cx += BUTTON_STRIDE;
    }
    cx += 12;
    for c in PALETTE {
        if x >= cx && x < cx + SWATCH_SIZE {
            st.canvas.style.color = *c;
            return;
        }
        cx += SWATCH_SIZE + 6;
    }
}
