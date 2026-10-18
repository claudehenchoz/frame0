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

//! Hosted in-place text editor.
//!
//! Text, Callout and Step annotations are edited with a Win32 `EDIT` control
//! created as a child of the editor's **parent** (sibling of the canvas —
//! child windows over a DXGI swap chain don't compose cleanly). The EDIT is
//! sub-classed via `SetWindowLongPtrW` so we can capture `VK_RETURN`
//! (commit), `VK_ESCAPE` (cancel), and `WM_KILLFOCUS` (commit on click-away).
//! Ctrl+Enter inserts a newline (Text/Callout).
//!
//! On commit, the new text is written into the annotation via a `Mutate`
//! command on the canvas history.

use crate::editor::annotation::{Annotation, Rect};
use crate::editor::canvas::TextEditReq;
use crate::editor::history::Command;
use crate::editor::layout::TOOLBAR_HEIGHT;
use crate::win::wide;
use std::cell::RefCell;
use std::collections::HashMap;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DeleteObject, FW_NORMAL,
    InvalidateRect, OUT_DEFAULT_PRECIS, VARIABLE_PITCH,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE, VK_RETURN};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, CreateWindowExW, DestroyWindow, GWLP_WNDPROC, GetWindowTextLengthW,
    GetWindowTextW, HWND_TOP, SWP_NOMOVE, SWP_NOSIZE, SendMessageW, SetWindowPos, SetWindowLongPtrW, WM_CHAR, WM_KEYDOWN, WM_KILLFOCUS, WM_SETFONT,
    WS_BORDER, WS_CHILD, WS_VISIBLE,
};

// EDIT control styles (not all are in windows-sys WindowsAndMessaging).
const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_CENTER: u32 = 0x0001;
const ES_WANTRETURN: u32 = 0x1000;
const EM_SETSEL: u32 = 0x00B1;

struct EditCtx {
    parent: HWND,
    canvas_hwnd: HWND,
    annotation_id: u32,
    /// Original EDIT wndproc, stored so we can chain via `CallWindowProcW`.
    original_proc: isize,
    font: isize,
}

thread_local! {
    static EDITS: RefCell<HashMap<HWND, EditCtx>> = RefCell::new(HashMap::new());
}

/// Spawn an EDIT over the annotation described by `req` (canvas-client
/// coordinates).
pub fn spawn_for(parent: HWND, canvas_hwnd: HWND, req: TextEditReq) {
    let state = crate::editor::window::get_state(parent);
    let Some(state) = state else { return };
    let (multiline, centered) = {
        let mut st = state.borrow_mut();
        let kind = st.canvas.doc.get(req.id).cloned();
        match kind {
            Some(Annotation::Text { .. }) => {
                st.canvas.editing = Some(req.id);
                (true, false)
            }
            Some(Annotation::Callout { .. }) => (true, true),
            Some(Annotation::Step { .. }) => (false, true),
            _ => return,
        }
    };
    unsafe { InvalidateRect(canvas_hwnd, core::ptr::null(), 0) };
    let class = wide("EDIT");
    let text = wide(&req.text.replace('\n', "\r\n"));
    let mut style = WS_CHILD | WS_VISIBLE | WS_BORDER | ES_AUTOVSCROLL;
    if multiline {
        style |= ES_MULTILINE | ES_WANTRETURN;
    }
    if centered {
        style |= ES_CENTER | ES_MULTILINE;
    }
    let r: Rect = req.rect;
    unsafe {
        let hinst = GetModuleHandleW(core::ptr::null()) as _;
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            text.as_ptr(),
            style,
            r.x as i32,
            r.y as i32 + TOOLBAR_HEIGHT,
            (r.w as i32).max(60),
            (r.h as i32).max((req.size * 1.6) as i32).max(24),
            parent,
            0,
            hinst,
            core::ptr::null(),
        );
        if hwnd == 0 {
            return;
        }

        let face = wide(crate::editor::render::FONT);
        let font = CreateFontW(
            -(req.size.max(8.0) as i32),
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
        SendMessageW(hwnd, EM_SETSEL, 0, -1);
        SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);

        let original = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, edit_wndproc as *const () as isize);
        EDITS.with(|m| {
            m.borrow_mut().insert(
                hwnd,
                EditCtx { parent, canvas_hwnd, annotation_id: req.id, original_proc: original, font },
            );
        });
        SetFocus(hwnd);
    }
}

fn read_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..n as usize]).replace("\r\n", "\n")
    }
}

fn finish(hwnd: HWND, commit: bool) {
    // Read first: WM_GETTEXT is routed through the subclass, which needs the
    // context to reach the original proc. Removing the context afterwards
    // makes re-entrant calls (the WM_KILLFOCUS fired during DestroyWindow)
    // no-ops.
    if EDITS.with(|m| !m.borrow().contains_key(&hwnd)) {
        return;
    }
    let new_text = read_text(hwnd);
    let ctx = EDITS.with(|m| m.borrow_mut().remove(&hwnd));
    let Some(ctx) = ctx else { return };

    if let Some(state) = crate::editor::window::get_state(ctx.parent) {
        let mut st = state.borrow_mut();
        st.canvas.editing = None;
        let gfx = st.canvas.gfx.clone();
        if let Some(idx) = st.canvas.doc.find_index(ctx.annotation_id) {
            let before = st.canvas.doc.annotations[idx].clone();
            let mut after = before.clone();
            let mut remove = false;
            match &mut after {
                Annotation::Text { text, bounds, size, .. } => {
                    if commit {
                        *text = new_text;
                    }
                    if text.is_empty() {
                        remove = true;
                    } else {
                        // Grow the box to fit the wrapped text.
                        let (_, h) = crate::editor::render::measure_text(&gfx, text, *size, bounds.w);
                        bounds.h = bounds.h.max(h + 2.0);
                    }
                }
                Annotation::Callout { text, bounds, size, .. } => {
                    if commit {
                        *text = new_text;
                        let pad = crate::editor::render::CALLOUT_PAD;
                        let (_, h) = crate::editor::render::measure_text(&gfx, text, *size, bounds.w - 2.0 * pad);
                        bounds.h = bounds.h.max(h + 2.0 * pad);
                    }
                }
                Annotation::Step { value, .. } => {
                    let v = new_text.trim();
                    if commit && !v.is_empty() {
                        *value = v.chars().take(4).collect();
                    }
                }
                _ => {}
            }
            let c = &mut st.canvas;
            if remove {
                // An empty Text box (new or cleared) is dropped. If it was
                // just created, the Insert is still on top of the undo stack:
                // pop it rather than recording a Remove.
                let fresh = matches!(c.history.undo.last(), Some(Command::Insert { ann, .. }) if ann.id() == ctx.annotation_id);
                if fresh {
                    c.history.undo.pop();
                    c.doc.annotations.remove(idx);
                } else {
                    let ann = c.doc.annotations.remove(idx);
                    c.history.push(Command::Remove { index: idx, ann });
                }
                c.selected.retain(|id| *id != ctx.annotation_id);
            } else if !annotations_equal(&before, &after) {
                c.doc.annotations[idx] = after.clone();
                c.history.push(Command::Mutate { id: ctx.annotation_id, before, after });
                c.dirty = true;
            }
        }
        drop(st);
        unsafe { InvalidateRect(ctx.canvas_hwnd, core::ptr::null(), 0) };
        crate::editor::window::refresh_panels(ctx.parent);
    }

    unsafe {
        DestroyWindow(hwnd);
        DeleteObject(ctx.font as _);
        SetFocus(ctx.canvas_hwnd);
    }
}

fn annotations_equal(a: &Annotation, b: &Annotation) -> bool {
    match (a, b) {
        (Annotation::Text { text: t1, bounds: b1, .. }, Annotation::Text { text: t2, bounds: b2, .. }) => t1 == t2 && b1 == b2,
        (Annotation::Callout { text: t1, bounds: b1, .. }, Annotation::Callout { text: t2, bounds: b2, .. }) => t1 == t2 && b1 == b2,
        (Annotation::Step { value: v1, .. }, Annotation::Step { value: v2, .. }) => v1 == v2,
        _ => false,
    }
}

/// True while an in-place editor is open (used to avoid stealing keys).
pub fn active() -> bool {
    EDITS.with(|m| !m.borrow().is_empty())
}

unsafe extern "system" fn edit_wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let original = EDITS.with(|m| m.borrow().get(&hwnd).map(|c| c.original_proc));
    let Some(original) = original else {
        unsafe { return CallWindowProcW(None, hwnd, msg, w, l) };
    };
    unsafe {
        match msg {
            WM_KEYDOWN => {
                let vk = w as u16;
                if vk == VK_RETURN {
                    let ctrl = (windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState(
                        windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL as i32,
                    ) as u16
                        & 0x8000)
                        != 0;
                    if !ctrl {
                        finish(hwnd, true);
                        return 0;
                    }
                    // Ctrl+Enter inserts a newline: feed the EDIT a CR.
                    let proc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT =
                        core::mem::transmute(original);
                    return CallWindowProcW(Some(proc), hwnd, WM_CHAR, 0x0D, 0);
                } else if vk == VK_ESCAPE {
                    finish(hwnd, false);
                    return 0;
                }
            }
            WM_CHAR => {
                // Swallow the CR/ESC chars that follow a commit/cancel keydown,
                // and the Ctrl+Enter LF.
                if w == 0x0D || w == 0x1B || w == 0x0A {
                    return 0;
                }
            }
            WM_KILLFOCUS => {
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

