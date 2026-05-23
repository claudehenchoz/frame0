//! Hosted in-place text editor.
//!
//! When the Text tool commits, or the user double-clicks an existing Text
//! annotation, we spawn a Win32 `EDIT` control as a child of the editor's
//! **parent** (sibling of the canvas — child windows over a DXGI swap chain
//! don't compose cleanly). The EDIT is sub-classed via `SetWindowLongPtrW`
//! so we can capture `VK_RETURN` (commit), `VK_ESCAPE` (cancel), and
//! `WM_KILLFOCUS` (commit on click-away).
//!
//! On commit, the new text is written into the annotation via a `Mutate`
//! command on the editor's history.

use crate::editor::annotation::Annotation;
use crate::editor::history::Command;
use crate::editor::layout::TOOLBAR_HEIGHT;
use crate::error::Result;
use crate::win::wide;
use std::cell::RefCell;
use std::collections::HashMap;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, FW_NORMAL,
    InvalidateRect, OUT_DEFAULT_PRECIS, VARIABLE_PITCH,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE, VK_RETURN};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, CreateWindowExW, DestroyWindow, GWLP_WNDPROC, GetWindowTextLengthW,
    GetWindowTextW, SendMessageW, SetWindowLongPtrW, WM_KEYDOWN, WM_KILLFOCUS, WM_SETFONT,
    WS_BORDER, WS_CHILD, WS_VISIBLE,
};

// EDIT control styles (not all are in windows-sys WindowsAndMessaging).
const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_WANTRETURN: u32 = 0x1000;

struct EditCtx {
    parent: HWND,
    canvas_hwnd: HWND,
    annotation_id: u32,
    /// Original EDIT wndproc, stored so we can chain via `CallWindowProcW`.
    original_proc: isize,
    /// Set while we're committing/cancelling so the WM_KILLFOCUS that fires
    /// during DestroyWindow doesn't re-enter and double-commit.
    finishing: bool,
}

thread_local! {
    static EDITS: RefCell<HashMap<HWND, EditCtx>> = RefCell::new(HashMap::new());
}

/// Spawn an EDIT child positioned at `parent_rect` (parent-relative client
/// coordinates) over an existing empty Text annotation. The initial text is
/// loaded from the annotation; on commit, the new text is written back via
/// a Mutate command.
pub fn spawn(
    parent: HWND,
    canvas_hwnd: HWND,
    annotation_id: u32,
    parent_rect: RECT,
    initial_text: &str,
    font_size: f32,
) -> Result<()> {
    let class = wide("EDIT");
    let text = wide(initial_text);
    unsafe {
        let hinst = GetModuleHandleW(core::ptr::null()) as _;
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_BORDER | ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN,
            parent_rect.left,
            parent_rect.top,
            (parent_rect.right - parent_rect.left).max(40),
            (parent_rect.bottom - parent_rect.top).max(24),
            parent,
            0,
            hinst,
            core::ptr::null(),
        );
        if hwnd == 0 {
            return Err(crate::error::Error::last_win32());
        }

        // Font matching the annotation's size.
        let face = wide("Segoe UI");
        let font = CreateFontW(
            -font_size as i32,
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
            face.as_ptr(),
        );
        SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 1);

        // Subclass.
        let original = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, edit_wndproc as *const () as isize);
        EDITS.with(|m| {
            m.borrow_mut().insert(
                hwnd,
                EditCtx {
                    parent,
                    canvas_hwnd,
                    annotation_id,
                    original_proc: original,
                    finishing: false,
                },
            );
        });

        SetFocus(hwnd);
    }
    Ok(())
}

fn read_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

fn finish(hwnd: HWND, commit: bool) {
    let ctx = EDITS.with(|m| m.borrow_mut().remove(&hwnd));
    let Some(mut ctx) = ctx else { return };
    if ctx.finishing {
        return;
    }
    ctx.finishing = true;

    if commit {
        let new_text = read_text(hwnd);
        let state = crate::editor::window::get_state(ctx.parent);
        if let Some(state) = state {
            let mut st = state.borrow_mut();
            if let Some(idx) = st.canvas.doc.find_index(ctx.annotation_id) {
                let before = st.canvas.doc.annotations[idx].clone();
                let mut after = before.clone();
                if let Annotation::Text { text, .. } = &mut after {
                    if new_text.is_empty() {
                        // Empty → treat as cancel and remove the annotation.
                        let ann = st.canvas.doc.annotations.remove(idx);
                        st.history.push(Command::Remove { index: idx, ann });
                    } else {
                        *text = new_text;
                        st.canvas.doc.annotations[idx] = after.clone();
                        st.history.push(Command::Mutate {
                            id: ctx.annotation_id,
                            before,
                            after,
                        });
                    }
                }
            }
            let cv = st.canvas_hwnd;
            drop(st);
            unsafe { InvalidateRect(cv, core::ptr::null(), 0) };
        }
    } else {
        // Cancel: remove the freshly-inserted (empty) annotation if it still
        // has empty text, so escaping a brand-new Text click is a no-op.
        let state = crate::editor::window::get_state(ctx.parent);
        if let Some(state) = state {
            let mut st = state.borrow_mut();
            if let Some(idx) = st.canvas.doc.find_index(ctx.annotation_id) {
                if let Annotation::Text { text, .. } = &st.canvas.doc.annotations[idx] {
                    if text.is_empty() {
                        let ann = st.canvas.doc.annotations.remove(idx);
                        st.history.push(Command::Remove { index: idx, ann });
                    }
                }
            }
            let cv = st.canvas_hwnd;
            drop(st);
            unsafe { InvalidateRect(cv, core::ptr::null(), 0) };
        }
    }

    unsafe {
        DestroyWindow(hwnd);
    }
    let _ = ctx.canvas_hwnd;
}

unsafe extern "system" fn edit_wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let original = EDITS.with(|m| m.borrow().get(&hwnd).map(|c| c.original_proc));
    let Some(original) = original else {
        unsafe { return CallWindowProcW(None, hwnd, msg, w, l) };
    };
    unsafe {
        match msg {
            WM_KEYDOWN => {
                let vk = w as u32;
                if vk == VK_RETURN as u32 {
                    let ctrl = (windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState(
                        windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL as i32,
                    ) as u16
                        & 0x8000)
                        != 0;
                    if !ctrl {
                        finish(hwnd, true);
                        return 0;
                    }
                    // Ctrl+Enter inserts a newline — let it through.
                } else if vk == VK_ESCAPE as u32 {
                    finish(hwnd, false);
                    return 0;
                }
            }
            WM_KILLFOCUS => {
                // Click-away → commit. `finish` guards re-entry.
                finish(hwnd, true);
                return 0;
            }
            _ => {}
        }
        let proc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT =
            core::mem::transmute(original);
        CallWindowProcW(Some(proc), hwnd, msg, w, l)
    }
}

/// Map a document-space `Rect` to a parent-window-relative `RECT`. The canvas
/// child sits at `(0, TOOLBAR_HEIGHT)` within the parent's client area.
pub fn doc_rect_to_parent(view_scale: f32, view_offset: (f32, f32), bounds: crate::editor::annotation::Rect) -> RECT {
    let left = view_offset.0 + bounds.x * view_scale;
    let top = view_offset.1 + bounds.y * view_scale + TOOLBAR_HEIGHT as f32;
    let right = left + bounds.w * view_scale;
    let bottom = top + bounds.h * view_scale;
    RECT {
        left: left as i32,
        top: top as i32,
        right: right as i32,
        bottom: bottom as i32,
    }
}
