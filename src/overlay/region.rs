//! Region-selection overlay.
//!
//! Mouse: drag to select (release confirms), right-click cancels.
//! Keys:  Esc cancel · arrows nudge the cursor 1 px (Shift: 10 px) ·
//!        Enter starts a keyboard selection at the cursor / confirms the
//!        current one · hold Space while dragging to move the selection ·
//!        F = whole monitor under the cursor · W = switch to window pick.
//! Fixed-size mode: a W×H frame follows the cursor; click / Enter confirms.

use super::{Backdrop, lparam_xy};
use crate::capture::ScreenRect;
use crate::gfx::bitmap::Bitmap;
use std::cell::RefCell;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, COLORONCOLOR, CreateSolidBrush, DeleteObject, EndPaint, FillRect, InvalidateRect,
    PAINTSTRUCT, SetStretchBltMode, StretchBlt, SRCCOPY,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT,
    VK_SHIFT, VK_SPACE, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, GetCursorPos, SetCursorPos, WM_ERASEBKGND, WM_KEYDOWN,
    WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDOWN,
};

const CLASS_NAME: &str = "frame0.overlay.region";
const LOUPE_SRC: i32 = 21; // source pixels across (odd → centre pixel)
const LOUPE_ZOOM: i32 = 6;

pub enum Picked {
    Rect(ScreenRect),
    /// User pressed W: continue with the window-pick overlay.
    Window,
}

struct State {
    bd: Backdrop,
    origin: (i32, i32),
    fixed: Option<(i32, i32)>,
    cursor: (i32, i32),
    dragging: bool, // mouse button held
    kb_select: bool, // Enter-started keyboard selection
    start: (i32, i32),
    end: (i32, i32),
    space: bool,
    result: Option<Picked>,
    done: bool,
}

thread_local! {
    static STATE: RefCell<Option<Box<State>>> = const { RefCell::new(None) };
}

/// Show the overlay over the virtual screen with `bg` as the frozen
/// backdrop. `fixed` switches to fixed-size placement.
pub fn pick(bg: &Bitmap, virt: (i32, i32, i32, i32), fixed: Option<(i32, i32)>) -> Option<Picked> {
    if STATE.with(|s| s.borrow().is_some()) {
        return None;
    }
    let origin = (virt.0, virt.1);
    let st = Box::new(State {
        bd: Backdrop::new(bg),
        origin,
        fixed: fixed.map(|(w, h)| (w.clamp(1, virt.2), h.clamp(1, virt.3))),
        cursor: super::cursor_client(origin),
        dragging: false,
        kb_select: false,
        start: (0, 0),
        end: (0, 0),
        space: false,
        result: None,
        done: false,
    });
    STATE.with(|s| *s.borrow_mut() = Some(st));
    unsafe {
        let hwnd = super::create_window(CLASS_NAME, Some(wndproc), virt);
        if hwnd == 0 {
            STATE.with(|s| s.borrow_mut().take());
            return None;
        }
        super::show(hwnd);
        crate::win::window::modal_loop(0, &|| STATE.with(|s| s.borrow().as_ref().map_or(true, |s| s.done)));
        if STATE.with(|s| s.borrow().as_ref().is_some_and(|s| s.done)) {
            // Already destroyed by finish(); nothing to do.
        } else {
            DestroyWindow(hwnd);
        }
    }
    STATE.with(|s| s.borrow_mut().take()).and_then(|s| s.result)
}

fn norm(a: (i32, i32), b: (i32, i32)) -> (i32, i32, i32, i32) {
    (a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
}

impl State {
    /// Current candidate rect in client coords, if any.
    fn selection(&self) -> Option<(i32, i32, i32, i32)> {
        if let Some((fw, fh)) = self.fixed {
            let x = (self.cursor.0 - fw / 2).clamp(0, (self.bd.w - fw).max(0));
            let y = (self.cursor.1 - fh / 2).clamp(0, (self.bd.h - fh).max(0));
            return Some((x, y, fw, fh));
        }
        if self.dragging || self.kb_select {
            return Some(norm(self.start, self.end));
        }
        None
    }

    fn to_screen(&self, r: (i32, i32, i32, i32)) -> ScreenRect {
        (r.0 + self.origin.0, r.1 + self.origin.1, r.2, r.3)
    }
}

unsafe fn finish(hwnd: HWND, st: &mut State, result: Option<Picked>) {
    st.result = result;
    st.done = true;
    if st.dragging {
        st.dragging = false;
        unsafe { ReleaseCapture() };
    }
    unsafe { DestroyWindow(hwnd) };
}

unsafe fn confirm(hwnd: HWND, st: &mut State) {
    if let Some(r) = st.selection() {
        if r.2 >= 2 && r.3 >= 2 {
            let sr = st.to_screen(r);
            unsafe { finish(hwnd, st, Some(Picked::Rect(sr))) };
            return;
        }
    }
    // Too small: drop the selection and keep going.
    st.dragging = false;
    st.kb_select = false;
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let p = STATE.with(|s| s.borrow_mut().as_mut().map(|b| &mut **b as *mut State));
    let Some(p) = p else { return unsafe { DefWindowProcW(hwnd, msg, w, l) } };
    // SAFETY: the box lives until `pick` returns, which only happens after
    // the modal loop ends; the wndproc is never re-entered for this state.
    let st = unsafe { &mut *p };
    if st.done {
        return unsafe { DefWindowProcW(hwnd, msg, w, l) };
    }
    unsafe {
        match msg {
            WM_LBUTTONDOWN => {
                let pt = lparam_xy(l);
                st.cursor = pt;
                if st.fixed.is_some() {
                    confirm(hwnd, st);
                    return 0;
                }
                st.dragging = true;
                st.kb_select = false;
                st.start = pt;
                st.end = pt;
                SetCapture(hwnd);
                InvalidateRect(hwnd, core::ptr::null(), 0);
                0
            }
            WM_MOUSEMOVE => {
                let pt = lparam_xy(l);
                if (st.dragging || st.kb_select) && st.space {
                    let (dx, dy) = (pt.0 - st.cursor.0, pt.1 - st.cursor.1);
                    st.start = (st.start.0 + dx, st.start.1 + dy);
                    st.end = (st.end.0 + dx, st.end.1 + dy);
                } else if st.dragging || st.kb_select {
                    st.end = pt;
                }
                st.cursor = pt;
                InvalidateRect(hwnd, core::ptr::null(), 0);
                0
            }
            WM_LBUTTONUP => {
                if st.dragging {
                    ReleaseCapture();
                    st.end = lparam_xy(l);
                    confirm(hwnd, st);
                    st.dragging = false;
                    InvalidateRect(hwnd, core::ptr::null(), 0);
                }
                0
            }
            WM_RBUTTONDOWN => {
                finish(hwnd, st, None);
                0
            }
            WM_KEYDOWN => {
                let vk = w as u16;
                let step = if GetKeyState(VK_SHIFT as i32) < 0 { 10 } else { 1 };
                let nudge = match vk {
                    VK_LEFT => Some((-step, 0)),
                    VK_RIGHT => Some((step, 0)),
                    VK_UP => Some((0, -step)),
                    VK_DOWN => Some((0, step)),
                    _ => None,
                };
                if let Some((dx, dy)) = nudge {
                    let mut pt = POINT { x: 0, y: 0 };
                    GetCursorPos(&mut pt);
                    SetCursorPos(pt.x + dx, pt.y + dy); // WM_MOUSEMOVE follows
                    return 0;
                }
                match vk {
                    VK_ESCAPE => finish(hwnd, st, None),
                    VK_RETURN => {
                        if st.fixed.is_some() || st.dragging || st.kb_select {
                            confirm(hwnd, st);
                        } else {
                            st.kb_select = true;
                            st.start = st.cursor;
                            st.end = st.cursor;
                        }
                        InvalidateRect(hwnd, core::ptr::null(), 0);
                    }
                    VK_SPACE => st.space = true,
                    0x46 /* F */ => {
                        let (sx, sy) = (st.cursor.0 + st.origin.0, st.cursor.1 + st.origin.1);
                        if let Some(m) = crate::capture::monitors::at_point(sx, sy) {
                            finish(hwnd, st, Some(Picked::Rect((m.x, m.y, m.w, m.h))));
                        }
                    }
                    0x57 /* W */ => finish(hwnd, st, Some(Picked::Window)),
                    _ => {}
                }
                0
            }
            WM_KEYUP => {
                if w as u16 == VK_SPACE {
                    st.space = false;
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
        let dc = bd.back_dc();
        bd.begin();

        let (cx, cy) = st.cursor;
        let sel = st.selection();
        if let Some((x, y, w, h)) = sel {
            bd.reveal(x, y, w, h);
            bd.outline(x, y, w, h, bd.px(1).max(1), super::ACCENT);
            let mut dim = (w.to_string(), h.to_string());
            dim.0.push_str(" × ");
            dim.0.push_str(&dim.1);
            let ly = if y + h + bd.px(28) < bd.h { y + h + bd.px(6) } else { (y - bd.px(28)).max(0) };
            bd.label(x.max(0), ly, &dim.0);
        } else {
            // Crosshair guides through the cursor.
            let br = CreateSolidBrush(0x00A0A0A0);
            FillRect(dc, &RECT { left: 0, top: cy, right: bd.w, bottom: cy + 1 }, br);
            FillRect(dc, &RECT { left: cx, top: 0, right: cx + 1, bottom: bd.h }, br);
            DeleteObject(br as _);
        }

        // Loupe + coordinates, offset from the cursor, flipped near edges.
        let size = LOUPE_SRC * LOUPE_ZOOM;
        let gap = bd.px(24);
        let mut lx = cx + gap;
        let mut ly = cy + gap;
        if lx + size + bd.px(8) > bd.w {
            lx = cx - gap - size;
        }
        if ly + size + bd.px(60) > bd.h {
            ly = cy - gap - size - bd.px(30);
        }
        let black = CreateSolidBrush(0);
        FillRect(dc, &RECT { left: lx, top: ly, right: lx + size, bottom: ly + size }, black);
        DeleteObject(black as _);
        SetStretchBltMode(dc, COLORONCOLOR as _);
        let half = LOUPE_SRC / 2;
        StretchBlt(dc, lx, ly, size, size, bd.clear_dc(), cx - half, cy - half, LOUPE_SRC, LOUPE_SRC, SRCCOPY);
        // Centre-pixel box.
        let c = half * LOUPE_ZOOM;
        super::outline(dc, lx + c, ly + c, LOUPE_ZOOM, LOUPE_ZOOM, 1, 0x00FFFFFF);
        bd.outline(lx, ly, size, size, 1, super::ACCENT);

        let (sx, sy) = (cx + st.origin.0, cy + st.origin.1);
        let mut text = sx.to_string();
        text.push_str(", ");
        text.push_str(&sy.to_string());
        let (_, lh) = bd.label(lx, ly + size + bd.px(4), &text);
        if sel.is_none() || st.fixed.is_some() {
            let hint = if st.fixed.is_some() {
                "Click / Enter: capture   Esc: cancel"
            } else {
                "F: monitor   W: window   Esc: cancel"
            };
            bd.label(lx, ly + size + bd.px(8) + lh, hint);
        }

        bd.present(hdc);
        EndPaint(hwnd, &ps);
    }
}
