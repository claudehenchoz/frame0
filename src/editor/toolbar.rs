//! Toolbar child window. GDI-rendered strip across the top of the editor.
//! Owns its own WM_PAINT (double-buffered): tool buttons drawn with Segoe
//! Fluent Icons glyphs (Segoe MDL2 Assets on Windows 10), colour swatches with
//! a custom-colour picker, undo/redo buttons, and a hover hint.

use crate::editor::tools::Tool;
use crate::editor::window::EditorState;
use crate::error::{Error, Result};
use crate::gfx::color::Color;
use crate::win::wide;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt as GdiBitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_CHARSET, DT_CENTER,
    DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, Ellipse, EndPaint, FW_NORMAL,
    FW_SEMIBOLD, FillRect, GetTextFaceW, HDC, InvalidateRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    PS_SOLID, RoundRect, SRCCOPY, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
    VARIABLE_PITCH,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::Dialogs::{CC_ANYCOLOR, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetFocus, TRACKMOUSEEVENT, TME_LEAVE, TrackMouseEvent};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetParent, IDC_ARROW, LoadCursorW,
    RegisterClassExW, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_PAINT, WNDCLASSEXW, WS_CHILD,
    WS_VISIBLE,
};

const WM_MOUSELEAVE: u32 = 0x02A3;

pub const PALETTE: &[Color] = &[
    Color::RED,
    Color::ORANGE,
    Color::YELLOW,
    Color::GREEN,
    Color::BLUE,
    Color::PURPLE,
    Color::BLACK,
    Color::WHITE,
];

fn glyph(t: Tool) -> u16 {
    match t {
        Tool::Move => 0xE7C2,
        Tool::Select => 0xEA48,
        Tool::Arrow => 0xE8AD,
        Tool::Rect => 0xE739,
        Tool::Ellipse => 0xEA3A,
        Tool::Polygon => 0xEA44,
        Tool::Callout => 0xE8BD,
        Tool::Text => 0xE8D2,
        Tool::Step => 0, // drawn as a numbered disc
        Tool::Highlighter => 0xE7E6,
        Tool::Blur => 0xE727,
        Tool::Stamp => 0xE734,
        Tool::Magnify => 0xE8A3,
        Tool::CutOut => 0xE8C6,
    }
}
const GLYPH_UNDO: u16 = 0xE7A7;
const GLYPH_REDO: u16 = 0xE7A6;

// Layout constants
const LEFT: i32 = 8;
const BTN_W: i32 = 34;
const BTN_H: i32 = 32;
const BTN_STRIDE: i32 = 37;
const BTN_TOP: i32 = 6;
const CORNER: i32 = 6;
const SEP_W: i32 = 10;
const SWATCH_D: i32 = 20;
const SWATCH_S: i32 = 26;
const SEP_LINE_TOP: i32 = 12;
const ACCENT_BAR_H: i32 = 2;

// UI colour BGR values (pre-computed from Color constants)
const C_SURFACE: u32 = 0x00201E1C; // Color::SURFACE  #1C1E20
const C_ELEVATED: u32 = 0x00282525; // Color::ELEVATED #252528
const C_HOVER: u32 = 0x00322E2E; // Color::HOVER     #2E2E32
const C_ACTIVE: u32 = 0x00423D3A; // Color::ACTIVE    #3A3D42
const C_BORDER: u32 = 0x003D3A3A; // Color::BORDER    #3A3A3D
const C_ACCENT: u32 = 0x00D0804A; // Color::ACCENT    #4A80D0
const C_SWATCH_RING: u32 = 0x004D4A4A; // Color::SWATCH_RING #4A4A4D
const C_ICON_IDLE: u32 = 0x00C8C8CA;
const C_ICON_ACTIVE: u32 = 0x00FFFFFF;
const C_TEXT_LABEL: u32 = 0x009D9A9A;

const CLASS_NAME: &str = "frame.editor.toolbar";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Tool(usize),
    Swatch(usize),
    Custom,
    Undo,
    Redo,
}

/// Hit rectangles, computed once from the layout constants.
fn layout() -> Vec<(Hit, RECT)> {
    let mut v = Vec::new();
    let mut x = LEFT;
    for i in 0..Tool::ALL.len() {
        v.push((Hit::Tool(i), RECT { left: x, top: BTN_TOP, right: x + BTN_W, bottom: BTN_TOP + BTN_H }));
        x += BTN_STRIDE;
    }
    x += SEP_W * 2;
    let sy = BTN_TOP + (BTN_H - SWATCH_D) / 2;
    for j in 0..PALETTE.len() {
        v.push((Hit::Swatch(j), RECT { left: x, top: sy, right: x + SWATCH_D, bottom: sy + SWATCH_D }));
        x += SWATCH_S;
    }
    v.push((Hit::Custom, RECT { left: x, top: sy, right: x + SWATCH_D, bottom: sy + SWATCH_D }));
    x += SWATCH_S;
    x += SEP_W * 2;
    v.push((Hit::Undo, RECT { left: x, top: BTN_TOP, right: x + BTN_W, bottom: BTN_TOP + BTN_H }));
    x += BTN_STRIDE;
    v.push((Hit::Redo, RECT { left: x, top: BTN_TOP, right: x + BTN_W, bottom: BTN_TOP + BTN_H }));
    v
}

fn hit(x: i32, y: i32) -> Option<Hit> {
    layout().into_iter().find(|(_, r)| x >= r.left && x < r.right && y >= r.top - 4 && y < r.bottom + 4).map(|(h, _)| h)
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

thread_local! {
    static HOVER: RefCell<Option<Hit>> = const { RefCell::new(None) };
    static CUSTOM: RefCell<[u32; 16]> = const { RefCell::new([0x00FFFFFF; 16]) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let parent = unsafe { GetParent(hwnd) };
    let Some(state) = crate::editor::window::get_state(parent) else {
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
                if let Ok(st) = state.try_borrow() {
                    paint(mem_dc, cw, ch, &st);
                }
                GdiBitBlt(hdc, 0, 0, cw, ch, mem_dc, 0, 0, SRCCOPY);
                SelectObject(mem_dc, old);
                DeleteObject(mem_bmp as _);
                DeleteDC(mem_dc);
                EndPaint(hwnd, &ps);
                0
            }
            WM_MOUSEMOVE => {
                let x = (l & 0xFFFF) as i16 as i32;
                let y = ((l >> 16) & 0xFFFF) as i16 as i32;
                let h = hit(x, y);
                let changed = HOVER.with(|c| {
                    let mut c = c.borrow_mut();
                    let ch = *c != h;
                    *c = h;
                    ch
                });
                if changed {
                    let mut tme = TRACKMOUSEEVENT { cbSize: core::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    TrackMouseEvent(&mut tme);
                    InvalidateRect(hwnd, core::ptr::null(), 0);
                }
                0
            }
            WM_MOUSELEAVE => {
                HOVER.with(|c| *c.borrow_mut() = None);
                InvalidateRect(hwnd, core::ptr::null(), 0);
                0
            }
            WM_LBUTTONDOWN => {
                let x = (l & 0xFFFF) as i16 as i32;
                let y = ((l >> 16) & 0xFFFF) as i16 as i32;
                if let Some(h) = hit(x, y) {
                    handle_click(parent, &state, h);
                }
                let cv = state.borrow().canvas_hwnd;
                InvalidateRect(cv, core::ptr::null(), 0);
                crate::editor::window::refresh_panels(parent);
                SetFocus(cv);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

fn handle_click(parent: HWND, state: &Rc<RefCell<EditorState>>, h: Hit) {
    match h {
        Hit::Tool(i) => state.borrow_mut().canvas.set_tool(Tool::ALL[i]),
        Hit::Swatch(j) => set_color(state, PALETTE[j]),
        Hit::Custom => {
            let cur = state.borrow().canvas.style.color;
            if let Some(c) = pick_color(parent, cur) {
                set_color(state, c);
            }
        }
        Hit::Undo => {
            state.borrow_mut().canvas.undo();
        }
        Hit::Redo => {
            state.borrow_mut().canvas.redo();
        }
    }
}

/// Set the quick-style colour and recolour the selection (one undo step).
pub fn set_color(state: &Rc<RefCell<EditorState>>, c: Color) {
    let mut st = state.borrow_mut();
    st.canvas.style.color = c;
    st.canvas.mutate_selected(&|a| crate::editor::property::apply_color(a, c));
}

pub fn pick_color(owner: HWND, initial: Color) -> Option<Color> {
    CUSTOM.with(|cust| unsafe {
        let mut cust = cust.borrow_mut();
        let mut cc: CHOOSECOLORW = core::mem::zeroed();
        cc.lStructSize = core::mem::size_of::<CHOOSECOLORW>() as u32;
        cc.hwndOwner = owner;
        cc.rgbResult = initial.as_colorref();
        cc.lpCustColors = cust.as_mut_ptr();
        cc.Flags = CC_RGBINIT | CC_FULLOPEN | CC_ANYCOLOR;
        if ChooseColorW(&mut cc) != 0 { Some(Color::from_colorref(cc.rgbResult)) } else { None }
    })
}

/// Icon font face: Fluent (Windows 11) or MDL2 (Windows 10).
fn icon_face() -> &'static str {
    static FACE: OnceLock<&'static str> = OnceLock::new();
    FACE.get_or_init(|| unsafe {
        let screen = windows_sys::Win32::Graphics::Gdi::GetDC(0);
        let f = make_font("Segoe Fluent Icons", 16, FW_NORMAL as i32);
        let old = SelectObject(screen, f as _);
        let mut buf = [0u16; 64];
        let n = GetTextFaceW(screen, buf.len() as i32, buf.as_mut_ptr());
        SelectObject(screen, old);
        DeleteObject(f as _);
        windows_sys::Win32::Graphics::Gdi::ReleaseDC(0, screen);
        let got = String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize]);
        if got.eq_ignore_ascii_case("Segoe Fluent Icons") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" }
    })
}

fn make_font(face: &str, px: i32, weight: i32) -> isize {
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
            wide(face).as_ptr(),
        )
    }
}

unsafe fn fill(hdc: HDC, r: RECT, c: u32) {
    unsafe {
        let b = CreateSolidBrush(c);
        FillRect(hdc, &r, b);
        DeleteObject(b as _);
    }
}

unsafe fn draw_glyph(hdc: HDC, r: RECT, g: u16, color: u32) {
    unsafe {
        let mut rr = r;
        SetTextColor(hdc, color);
        DrawTextW(hdc, [g, 0].as_ptr(), 1, &mut rr, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
}

unsafe fn paint(hdc: HDC, cw: i32, ch: i32, st: &EditorState) {
    unsafe {
        fill(hdc, RECT { left: 0, top: 0, right: cw, bottom: ch }, C_SURFACE);
        fill(hdc, RECT { left: 0, top: ch - 1, right: cw, bottom: ch }, C_BORDER);
        SetBkMode(hdc, TRANSPARENT as i32);
        let icon_font = make_font(icon_face(), 18, FW_NORMAL as i32);
        let small_font = make_font("Segoe UI", 12, FW_SEMIBOLD as i32);
        let label_font = make_font("Segoe UI", 13, FW_NORMAL as i32);
        let old_font = SelectObject(hdc, icon_font as _);
        let hover = HOVER.with(|c| *c.borrow());
        let mut last_right = 0;

        for (h, r) in layout() {
            let is_hover = hover == Some(h);
            match h {
                Hit::Tool(i) => {
                    let t = Tool::ALL[i];
                    let active = st.canvas.active_tool == t;
                    draw_button_bg(hdc, r, active, is_hover);
                    let col = if active { C_ICON_ACTIVE } else { C_ICON_IDLE };
                    if t == Tool::Step {
                        // Numbered disc.
                        let c = st.canvas.style.color.as_colorref();
                        let pen = CreatePen(PS_SOLID, 1, col);
                        let br = CreateSolidBrush(if active { c } else { C_ELEVATED });
                        let op = SelectObject(hdc, pen as _);
                        let ob = SelectObject(hdc, br as _);
                        let (cx, cy) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
                        Ellipse(hdc, cx - 9, cy - 9, cx + 10, cy + 10);
                        SelectObject(hdc, op);
                        SelectObject(hdc, ob);
                        DeleteObject(pen as _);
                        DeleteObject(br as _);
                        SelectObject(hdc, small_font as _);
                        let mut rr = r;
                        SetTextColor(hdc, col);
                        DrawTextW(hdc, wide("1").as_ptr(), 1, &mut rr, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                        SelectObject(hdc, icon_font as _);
                    } else {
                        draw_glyph(hdc, r, glyph(t), col);
                    }
                    if i + 1 == Tool::ALL.len() {
                        let x = r.right + SEP_W;
                        fill(hdc, RECT { left: x, top: SEP_LINE_TOP, right: x + 1, bottom: ch - SEP_LINE_TOP }, C_BORDER);
                    }
                }
                Hit::Swatch(j) => {
                    let c = PALETTE[j];
                    draw_swatch(hdc, r, c.as_colorref(), c == st.canvas.style.color, is_hover);
                }
                Hit::Custom => {
                    let cur = st.canvas.style.color;
                    let custom = !PALETTE.contains(&cur);
                    if custom {
                        draw_swatch(hdc, r, cur.as_colorref(), true, is_hover);
                    } else {
                        draw_swatch(hdc, r, C_ELEVATED, false, is_hover);
                        draw_glyph(hdc, RECT { top: r.top - 1, ..r }, 0xE710, C_ICON_IDLE);
                    }
                    let x = r.right + SEP_W;
                    fill(hdc, RECT { left: x, top: SEP_LINE_TOP, right: x + 1, bottom: ch - SEP_LINE_TOP }, C_BORDER);
                }
                Hit::Undo | Hit::Redo => {
                    let enabled = if h == Hit::Undo { !st.canvas.history.undo.is_empty() } else { !st.canvas.history.redo.is_empty() };
                    draw_button_bg(hdc, r, false, is_hover && enabled);
                    draw_glyph(hdc, r, if h == Hit::Undo { GLYPH_UNDO } else { GLYPH_REDO }, if enabled { C_ICON_IDLE } else { 0x00555555 });
                    last_right = r.right;
                }
            }
        }

        // Hover hint.
        let hint = match hover {
            Some(Hit::Tool(i)) => {
                let t = Tool::ALL[i];
                format!("{}  ({})", t.name(), t.key() as char)
            }
            Some(Hit::Swatch(_)) => "Colour — applies to the selection too".to_string(),
            Some(Hit::Custom) => "Custom colour…".to_string(),
            Some(Hit::Undo) => "Undo  (Ctrl+Z)".to_string(),
            Some(Hit::Redo) => "Redo  (Ctrl+Y)".to_string(),
            None => String::new(),
        };
        if !hint.is_empty() {
            SelectObject(hdc, label_font as _);
            SetTextColor(hdc, C_TEXT_LABEL);
            let w = wide(&hint);
            TextOutW(hdc, last_right + 16, (ch - 16) / 2, w.as_ptr(), (w.len() - 1) as i32);
        }

        SelectObject(hdc, old_font);
        DeleteObject(icon_font as _);
        DeleteObject(small_font as _);
        DeleteObject(label_font as _);
    }
}

unsafe fn draw_button_bg(hdc: HDC, r: RECT, active: bool, hover: bool) {
    unsafe {
        let fill_c = if active { C_ACTIVE } else if hover { C_HOVER } else { C_SURFACE };
        let brush = CreateSolidBrush(fill_c);
        let pen = CreatePen(PS_SOLID, 1, if active || hover { C_BORDER } else { C_SURFACE });
        let old_pen = SelectObject(hdc, pen as _);
        let old_brush = SelectObject(hdc, brush as _);
        RoundRect(hdc, r.left, r.top, r.right, r.bottom, CORNER, CORNER);
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        DeleteObject(pen as _);
        DeleteObject(brush as _);
        if active {
            fill(hdc, RECT { left: r.left + 8, top: r.bottom - ACCENT_BAR_H - 1, right: r.right - 8, bottom: r.bottom - 1 }, C_ACCENT);
        }
    }
}

unsafe fn draw_swatch(hdc: HDC, r: RECT, bgr: u32, selected: bool, hover: bool) {
    unsafe {
        let fill_b = CreateSolidBrush(bgr);
        let ring_col = if selected { 0x00FFFFFF } else if hover { 0x00909090 } else { C_SWATCH_RING };
        let pen = CreatePen(PS_SOLID, if selected { 2 } else { 1 }, ring_col);
        let old_pen = SelectObject(hdc, pen as _);
        let old_brush = SelectObject(hdc, fill_b as _);
        Ellipse(hdc, r.left, r.top, r.right, r.bottom);
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        DeleteObject(pen as _);
        DeleteObject(fill_b as _);
    }
}
