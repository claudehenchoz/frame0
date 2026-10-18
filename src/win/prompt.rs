//! Tiny modal input dialog built from plain Win32 controls (no resource
//! files): one labelled EDIT per field plus OK / Cancel. Enter = OK,
//! Esc = Cancel, Tab cycles fields.

use crate::win::wide;
use std::cell::RefCell;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    COLOR_WINDOW, CreateFontIndirectW, CreateSolidBrush, DT_CENTER, DT_SINGLELINE, DT_VCENTER,
    DeleteObject, DrawTextW, FillRect, GetMonitorInfoW, GetSysColor, GetSysColorBrush, HDC, HFONT,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, SelectObject, SetBkColor,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_FOCUS, ODS_SELECTED};
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, SystemParametersInfoForDpi};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BS_OWNERDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, ES_AUTOHSCROLL,
    GetCursorPos, GetWindowTextLengthW, GetWindowTextW, IDC_ARROW, IDCANCEL, IDOK, LoadCursorW,
    NONCLIENTMETRICSW, RegisterClassExW, SPI_GETNONCLIENTMETRICS,
    SW_SHOW, SendMessageW, SetForegroundWindow, ShowWindow, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLORSTATIC, WM_DRAWITEM, WM_SETFONT, WNDCLASSEXW, WS_BORDER, WS_CAPTION, WS_CHILD,
    WS_EX_DLGMODALFRAME, WS_EX_TOPMOST, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

const CLASS_NAME: &str = "frame0.prompt";
const EM_SETSEL: u32 = 0x00B1;
const DM_GETDEFID: u32 = 0x0400;
const ID_EDIT0: usize = 1000;

struct State {
    edits: Vec<HWND>,
    result: Option<Vec<String>>,
    done: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Ask for one or more values. `fields` are (label, initial value) pairs.
/// Returns the edited values on OK, `None` on Cancel / close.
pub fn ask(owner: HWND, title: &str, fields: &[(&str, &str)]) -> Option<Vec<String>> {
    if STATE.with(|s| s.borrow().is_some()) {
        return None; // one prompt at a time
    }
    unsafe {
        register_class();
        let dpi = GetDpiForSystem().max(96);
        let px = |v: i32| v * dpi as i32 / 96;
        let (margin, label_w, edit_w, row_h, gap, btn_w, btn_h) =
            (px(14), px(110), px(200), px(24), px(10), px(84), px(28));
        let client_w = margin * 2 + label_w + edit_w;
        let n = fields.len() as i32;
        let client_h = margin * 2 + n * row_h + (n - 1).max(0) * gap + px(18) + btn_h;

        let ex = WS_EX_DLGMODALFRAME | WS_EX_TOPMOST;
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
        let mut wr = RECT { left: 0, top: 0, right: client_w, bottom: client_h };
        AdjustWindowRectExForDpi(&mut wr, style, 0, ex, dpi);
        let (ww, wh) = (wr.right - wr.left, wr.bottom - wr.top);

        // Centre on the monitor under the cursor.
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let mut mi: MONITORINFO = core::mem::zeroed();
        mi.cbSize = core::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST), &mut mi);
        let w = mi.rcWork;
        let (x, y) = (w.left + (w.right - w.left - ww) / 2, w.top + (w.bottom - w.top - wh) / 2);

        let hinst = GetModuleHandleW(core::ptr::null());
        let cname = wide(CLASS_NAME);
        let hwnd = CreateWindowExW(
            ex, cname.as_ptr(), wide(title).as_ptr(), style, x, y, ww, wh,
            owner, 0, hinst, core::ptr::null(),
        );
        if hwnd == 0 {
            return None;
        }

        let mut ncm: NONCLIENTMETRICSW = core::mem::zeroed();
        ncm.cbSize = core::mem::size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, ncm.cbSize, &mut ncm as *mut _ as _, 0, dpi);
        let font: HFONT = CreateFontIndirectW(&ncm.lfMessageFont);

        let child = |class: &str, text: &str, style: u32, id: usize, x: i32, y: i32, w: i32, h: i32| {
            let c = CreateWindowExW(
                0, wide(class).as_ptr(), wide(text).as_ptr(), WS_CHILD | WS_VISIBLE | style,
                x, y, w, h, hwnd, id as _, hinst, core::ptr::null(),
            );
            SendMessageW(c, WM_SETFONT, font as WPARAM, 0);
            c
        };

        let mut edits = Vec::with_capacity(fields.len());
        for (i, (label, init)) in fields.iter().enumerate() {
            let y = margin + i as i32 * (row_h + gap);
            child("STATIC", label, 0x0200 /* SS_CENTERIMAGE */, 0, margin, y, label_w - px(8), row_h);
            let e = child(
                "EDIT", init, WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
                ID_EDIT0 + i, margin + label_w, y, edit_w, row_h,
            );
            edits.push(e);
        }
        let by = client_h - margin - btn_h;
        child("BUTTON", "OK", WS_TABSTOP | BS_OWNERDRAW as u32, IDOK as usize,
            client_w - margin - 2 * btn_w - px(8), by, btn_w, btn_h);
        child("BUTTON", "Cancel", WS_TABSTOP | BS_OWNERDRAW as u32, IDCANCEL as usize,
            client_w - margin - btn_w, by, btn_w, btn_h);

        let first = edits.first().copied().unwrap_or(0);
        STATE.with(|s| *s.borrow_mut() = Some(State { edits, result: None, done: false }));

        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        if first != 0 {
            SetFocus(first);
            SendMessageW(first, EM_SETSEL, 0, -1);
        }

        crate::win::window::modal_loop(hwnd, &|| STATE.with(|s| s.borrow().as_ref().map_or(true, |s| s.done)));

        DestroyWindow(hwnd);
        DeleteObject(font as _);
        STATE.with(|s| s.borrow_mut().take()).and_then(|s| s.result)
    }
}

unsafe fn register_class() {
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
            hInstance: GetModuleHandleW(core::ptr::null()),
            hIcon: 0,
            hCursor: LoadCursorW(0, IDC_ARROW),
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            lpszMenuName: core::ptr::null(),
            lpszClassName: cname.as_ptr(),
            hIconSm: 0,
        };
        RegisterClassExW(&wc);
    });
}

fn finish(ok: bool) {
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            if ok {
                st.result = Some(st.edits.iter().map(|&e| window_text(e)).collect());
            }
            st.done = true;
        }
    });
}

fn window_text(hwnd: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut buf = vec![0u16; n + 1];
        let got = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
        String::from_utf16_lossy(&buf[..got]).trim().to_string()
    }
}

unsafe fn draw_button(dis: &DRAWITEMSTRUCT) {
    unsafe {
        let primary = dis.CtlID == IDOK as u32;
        let pressed = dis.itemState & ODS_SELECTED != 0;
        let (bg, fg) = match (primary, pressed) {
            (true, false) => (0x00D47800, 0x00FFFFFF),
            (true, true) => (0x00A45C00, 0x00FFFFFF),
            (false, false) => (0x00E6E6E6, 0x00202020),
            (false, true) => (0x00CCCCCC, 0x00202020),
        };
        let hdc: HDC = dis.hDC;
        let br = CreateSolidBrush(bg);
        FillRect(hdc, &dis.rcItem, br);
        DeleteObject(br as _);
        if dis.itemState & ODS_FOCUS != 0 {
            let mut r = dis.rcItem;
            r.left += 2;
            r.top += 2;
            r.right -= 2;
            r.bottom -= 2;
            windows_sys::Win32::Graphics::Gdi::DrawFocusRect(hdc, &r);
        }
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, fg);
        let text = wide(if primary { "OK" } else { "Cancel" });
        let mut r = dis.rcItem;
        DrawTextW(hdc, text.as_ptr(), -1, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                match (w & 0xFFFF) as i32 {
                    IDOK => finish(true),
                    IDCANCEL => finish(false),
                    _ => {}
                }
                0
            }
            DM_GETDEFID => (IDOK as isize) | (0x534B << 16), // DC_HASDEFID
            WM_CLOSE => {
                finish(false);
                0
            }
            WM_CTLCOLORSTATIC => {
                SetBkColor(w as HDC, GetSysColor(COLOR_WINDOW));
                GetSysColorBrush(COLOR_WINDOW)
            }
            WM_DRAWITEM => {
                draw_button(&*(l as *const DRAWITEMSTRUCT));
                1
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}
