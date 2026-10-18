//! Window-pick overlay with hover highlight.
//!
//! Shows the frozen desktop; the top-most capturable window under the cursor
//! is revealed and outlined with its title. Left-click confirms, Esc /
//! right-click cancels. Hit-testing uses the window list (and DWM frame
//! bounds) captured together with the snapshot, so what is highlighted is
//! what the user saw.

use super::{Backdrop, lparam_xy};
use crate::capture::ScreenRect;
use crate::capture::windows::{WindowEntry, list};
use crate::gfx::bitmap::Bitmap;
use std::cell::RefCell;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MOUSEMOVE,
    WM_PAINT, WM_RBUTTONDOWN,
};

const CLASS_NAME: &str = "frame.overlay.windowpick";

struct State {
    bd: Backdrop,
    origin: (i32, i32),
    windows: Vec<WindowEntry>,
    hovered: Option<usize>,
    result: Option<(HWND, ScreenRect)>,
    done: bool,
}

thread_local! {
    static STATE: RefCell<Option<Box<State>>> = const { RefCell::new(None) };
}

/// Returns the picked window and its visible frame rect (screen coords).
pub fn pick(bg: &Bitmap, virt: (i32, i32, i32, i32)) -> Option<(HWND, ScreenRect)> {
    if STATE.with(|s| s.borrow().is_some()) {
        return None;
    }
    let origin = (virt.0, virt.1);
    // Enumerate before our overlay exists; minimized windows aren't on screen.
    let windows: Vec<WindowEntry> = list(0).into_iter().filter(|e| !e.minimized).collect();
    let mut st = Box::new(State { bd: Backdrop::new(bg), origin, windows, hovered: None, result: None, done: false });
    let (cx, cy) = super::cursor_client(origin);
    st.hovered = hit_test(&st.windows, cx + origin.0, cy + origin.1);
    STATE.with(|s| *s.borrow_mut() = Some(st));
    unsafe {
        let hwnd = super::create_window(CLASS_NAME, Some(wndproc), virt);
        if hwnd == 0 {
            STATE.with(|s| s.borrow_mut().take());
            return None;
        }
        super::show(hwnd);
        crate::win::window::modal_loop(0, &|| STATE.with(|s| s.borrow().as_ref().map_or(true, |s| s.done)));
        if !STATE.with(|s| s.borrow().as_ref().is_some_and(|s| s.done)) {
            DestroyWindow(hwnd);
        }
    }
    STATE.with(|s| s.borrow_mut().take()).and_then(|s| s.result)
}

/// EnumWindows yields top-most first, so the first hit is the visible one.
fn hit_test(windows: &[WindowEntry], x: i32, y: i32) -> Option<usize> {
    windows.iter().position(|e| {
        let r = e.rect;
        x >= r.left && x < r.right && y >= r.top && y < r.bottom
    })
}

unsafe fn finish(hwnd: HWND, st: &mut State, result: Option<(HWND, ScreenRect)>) {
    st.result = result;
    st.done = true;
    unsafe { DestroyWindow(hwnd) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let p = STATE.with(|s| s.borrow_mut().as_mut().map(|b| &mut **b as *mut State));
    let Some(p) = p else { return unsafe { DefWindowProcW(hwnd, msg, w, l) } };
    let st = unsafe { &mut *p };
    if st.done {
        return unsafe { DefWindowProcW(hwnd, msg, w, l) };
    }
    unsafe {
        match msg {
            WM_MOUSEMOVE => {
                let (x, y) = lparam_xy(l);
                let h = hit_test(&st.windows, x + st.origin.0, y + st.origin.1);
                if h != st.hovered {
                    st.hovered = h;
                    InvalidateRect(hwnd, core::ptr::null(), 0);
                }
                0
            }
            WM_LBUTTONDOWN => {
                let (x, y) = lparam_xy(l);
                st.hovered = hit_test(&st.windows, x + st.origin.0, y + st.origin.1);
                let res = st.hovered.map(|i| {
                    let e = &st.windows[i];
                    let r = e.rect;
                    (e.hwnd, (r.left, r.top, r.right - r.left, r.bottom - r.top))
                });
                if res.is_some() {
                    finish(hwnd, st, res);
                }
                0
            }
            WM_RBUTTONDOWN => {
                finish(hwnd, st, None);
                0
            }
            WM_KEYDOWN => {
                if w as u16 == VK_ESCAPE {
                    finish(hwnd, st, None);
                }
                0
            }
            WM_PAINT => {
                paint(hwnd, st);
                0
            }
            WM_ERASEBKGND => 1,
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

unsafe fn paint(hwnd: HWND, st: &State) {
    unsafe {
        let mut ps: PAINTSTRUCT = core::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let bd = &st.bd;
        bd.begin();
        if let Some(i) = st.hovered {
            let e = &st.windows[i];
            let x = (e.rect.left - st.origin.0).max(0);
            let y = (e.rect.top - st.origin.1).max(0);
            let r = (e.rect.right - st.origin.0).min(bd.w);
            let b = (e.rect.bottom - st.origin.1).min(bd.h);
            let (w, h) = (r - x, b - y);
            bd.reveal(x, y, w, h);
            let k = bd.px(3);
            // Keep the outline on-screen for maximized windows.
            bd.outline(x + k, y + k, w - 2 * k, h - 2 * k, k, super::ACCENT);
            let mut text = crate::win::tray::menu_label(&e.title, 80).replace("&&", "&");
            text.push_str("   ");
            text.push_str(&w.to_string());
            text.push_str(" × ");
            text.push_str(&h.to_string());
            bd.label(x + 2 * k, y + 2 * k, &text);
        } else {
            bd.label(bd.px(16), bd.px(16), "Click a window to capture   Esc: cancel");
        }
        bd.present(hdc);
        EndPaint(hwnd, &ps);
    }
}
