//! Editor parent window. Owns the menu, the layout of three children (toolbar,
//! canvas, property panel), and the shared `EditorState`. Child windows look
//! the state up via the thread-local map keyed by the parent's HWND.

use crate::app::App;
use crate::editor::annotation::Document;
use crate::editor::canvas::Canvas;
use crate::editor::layout::{TOOLBAR_HEIGHT, canvas_rect, property_rect, toolbar_rect};
use crate::editor::pixels::Effect;
use crate::editor::tools::Tool;
use crate::error::{Error, Result};
use crate::gfx::bitmap::Bitmap;
use crate::win::wide;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{InvalidateRect, UpdateWindow};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, BringWindowToTop, CW_USEDEFAULT, CheckMenuItem, CreateMenu, CreatePopupMenu,
    CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, EnableMenuItem, GetClientRect,
    GetMenu, GetSystemMetrics, IDC_ARROW, IDCANCEL, IDYES, LoadCursorW, MB_ICONWARNING,
    MB_YESNOCANCEL, MF_BYCOMMAND, MF_CHECKED, MF_GRAYED, MF_ENABLED, MF_POPUP, MF_SEPARATOR,
    MF_STRING, MF_UNCHECKED, MessageBoxW, RegisterClassExW, SM_CXSCREEN, SM_CYSCREEN,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE, SW_SHOW, SetForegroundWindow, SetMenu,
    SetWindowPos, SetWindowTextW, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
    WM_ACTIVATE, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_ERASEBKGND, WM_INITMENUPOPUP, WM_SIZE,
    WNDCLASSEXW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

const CLASS_NAME: &str = "frame.editor";

pub struct EditorWindow {
    pub hwnd: HWND,
    pub state: Rc<RefCell<EditorState>>,
}

pub struct EditorState {
    pub app: Weak<App>,
    pub canvas: Canvas,
    pub current_path: Option<String>,
    pub hwnd: HWND,
    pub canvas_hwnd: HWND,
    pub toolbar_hwnd: HWND,
    pub property_hwnd: HWND,
    // UI hover tracking (set by toolbar child, cleared on click)
    pub hovered_tool: Option<usize>,
    pub hovered_swatch: Option<usize>,
}

thread_local! {
    // Key = any window owned by the editor (parent or child). All entries point
    // to the same Rc, so any wndproc can borrow shared state regardless of
    // which window received the message. (HashMap::new isn't const, so this
    // is the non-`const { }` flavour of thread_local.)
    static EDITORS: RefCell<HashMap<HWND, Rc<RefCell<EditorState>>>> =
        RefCell::new(HashMap::new());
}

pub fn get_state(hwnd: HWND) -> Option<Rc<RefCell<EditorState>>> {
    EDITORS.with(|m| m.borrow().get(&hwnd).cloned())
}

fn put_state(hwnd: HWND, state: Rc<RefCell<EditorState>>) {
    EDITORS.with(|m| {
        m.borrow_mut().insert(hwnd, state);
    });
}

fn clear_state(hwnd: HWND) {
    EDITORS.with(|m| {
        m.borrow_mut().remove(&hwnd);
    });
}

impl EditorWindow {
    pub fn create(app: Rc<App>, bmp: Bitmap) -> Result<Self> {
        let doc = Document::new(bmp);
        Self::create_with_doc_impl(app, doc, None)
    }
    pub fn create_with_doc(app: Rc<App>, doc: Document) -> Result<Self> {
        Self::create_with_doc_impl(app, doc, None)
    }

    fn create_with_doc_impl(app: Rc<App>, doc: Document, path: Option<String>) -> Result<Self> {
        register_class();
        let cname = wide(CLASS_NAME);
        unsafe {
            let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN).max(1024), GetSystemMetrics(SM_CYSCREEN).max(720));
            let init_w = ((doc.base.width as i32) + crate::editor::layout::PROPERTY_WIDTH + 80).min(sw * 9 / 10).max(980);
            let init_h = ((doc.base.height as i32) + TOOLBAR_HEIGHT + 120).min(sh * 9 / 10).max(640);
            let hwnd = CreateWindowExW(
                0,
                cname.as_ptr(),
                wide("frame").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                init_w,
                init_h,
                0,
                0,
                GetModuleHandleW(core::ptr::null()) as _,
                core::ptr::null(),
            );
            if hwnd == 0 {
                return Err(Error::last_win32());
            }

            attach_menu(hwnd);

            let canvas = Canvas::create(app.gfx.clone(), doc)?;
            let state = Rc::new(RefCell::new(EditorState {
                app: Rc::downgrade(&app),
                canvas,
                current_path: path,
                hwnd,
                canvas_hwnd: 0,
                toolbar_hwnd: 0,
                property_hwnd: 0,
                hovered_tool: None,
                hovered_swatch: None,
            }));
            put_state(hwnd, state.clone());

            let mut client: RECT = core::mem::zeroed();
            GetClientRect(hwnd, &mut client);
            let tb_hwnd = crate::editor::toolbar::create_child(hwnd, toolbar_rect(&client))?;
            let cv_hwnd = crate::editor::canvas::create_child(hwnd, canvas_rect(&client))?;
            let pr_hwnd = crate::editor::property::create_child(hwnd, property_rect(&client))?;

            put_state(tb_hwnd, state.clone());
            put_state(cv_hwnd, state.clone());
            put_state(pr_hwnd, state.clone());

            {
                let cr = canvas_rect(&client);
                let mut st = state.borrow_mut();
                st.canvas_hwnd = cv_hwnd;
                st.toolbar_hwnd = tb_hwnd;
                st.property_hwnd = pr_hwnd;
                st.canvas.attach(cv_hwnd, (cr.right - cr.left).max(1) as u32, (cr.bottom - cr.top).max(1) as u32)?;
            }

            ShowWindow(hwnd, SW_SHOW);
            UpdateWindow(hwnd);
            SetForegroundWindow(hwnd);
            SetFocus(cv_hwnd);
            update_title(&state);

            Ok(Self { hwnd, state })
        }
    }

    pub fn activate(&self) {
        unsafe {
            ShowWindow(self.hwnd, SW_RESTORE);
            SetForegroundWindow(self.hwnd);
            BringWindowToTop(self.hwnd);
        }
    }

    pub fn replace_image(&mut self, bmp: Bitmap) {
        self.replace_document(Document::new(bmp));
    }

    pub fn replace_document(&mut self, doc: Document) {
        // A new capture must not silently throw away unsaved annotations.
        if !confirm_discard(self.hwnd, &self.state) {
            return;
        }
        {
            let mut st = self.state.borrow_mut();
            if st.canvas.replace_document(doc).is_err() {
                return;
            }
            st.current_path = None;
        }
        update_title(&self.state);
        refresh_panels(self.hwnd);
        unsafe {
            InvalidateRect(self.state.borrow().canvas_hwnd, core::ptr::null(), 0);
        }
    }
}

fn register_class() {
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

/// Repaint the toolbar + property panel and refresh the title.
pub fn refresh_panels(parent: HWND) {
    if let Some(state) = get_state(parent) {
        let (tb, pr) = match state.try_borrow() {
            Ok(st) => (st.toolbar_hwnd, st.property_hwnd),
            Err(_) => return,
        };
        unsafe {
            InvalidateRect(tb, core::ptr::null(), 0);
            InvalidateRect(pr, core::ptr::null(), 0);
        }
        update_title(&state);
    }
}

fn update_title(state: &Rc<RefCell<EditorState>>) {
    let Ok(st) = state.try_borrow() else { return };
    let name = st
        .current_path
        .as_deref()
        .map(|p| p.rsplit('\\').next().unwrap_or(p).to_string())
        .unwrap_or_else(|| "Untitled".to_string());
    let t = format!(
        "{}{} — frame ({}×{}, {}%)",
        if st.canvas.dirty { "• " } else { "" },
        name,
        st.canvas.doc.base.width,
        st.canvas.doc.base.height,
        (st.canvas.view_scale * 100.0).round()
    );
    unsafe { SetWindowTextW(st.hwnd, wide(&t).as_ptr()) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let state = get_state(hwnd);
    let Some(state) = state else {
        return unsafe { DefWindowProcW(hwnd, msg, w, l) };
    };
    unsafe {
        match msg {
            WM_ERASEBKGND => 1,
            WM_SIZE => {
                let mut client: RECT = core::mem::zeroed();
                GetClientRect(hwnd, &mut client);
                let (tb_hwnd, cv_hwnd, pr_hwnd) = {
                    let st = state.borrow();
                    (st.toolbar_hwnd, st.canvas_hwnd, st.property_hwnd)
                };
                for (h, r) in [(tb_hwnd, toolbar_rect(&client)), (cv_hwnd, canvas_rect(&client)), (pr_hwnd, property_rect(&client))] {
                    if h != 0 {
                        SetWindowPos(h, 0, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                    }
                }
                0
            }
            WM_ACTIVATE => {
                if (w & 0xFFFF) != 0 && !crate::editor::text_edit::active() {
                    let cv = state.borrow().canvas_hwnd;
                    SetFocus(cv);
                }
                0
            }
            WM_INITMENUPOPUP => {
                update_menu_state(hwnd, &state);
                0
            }
            WM_COMMAND => {
                handle_command(hwnd, &state, (w & 0xFFFF) as u16);
                0
            }
            WM_CLOSE => {
                if confirm_discard(hwnd, &state) {
                    DestroyWindow(hwnd);
                }
                0
            }
            WM_DESTROY => {
                let (tb, cv, pr) = {
                    let st = state.borrow();
                    (st.toolbar_hwnd, st.canvas_hwnd, st.property_hwnd)
                };
                for h in [tb, cv, pr] {
                    if h != 0 {
                        clear_state(h);
                    }
                }
                clear_state(hwnd);
                let app = state.borrow().app.upgrade();
                if let Some(app) = app {
                    app.editor_closed();
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

/// Ask before throwing away unsaved annotations. Returns true to proceed.
fn confirm_discard(hwnd: HWND, state: &Rc<RefCell<EditorState>>) -> bool {
    let dirty = state.borrow().canvas.dirty;
    if !dirty {
        return true;
    }
    let r = unsafe {
        MessageBoxW(
            hwnd,
            wide("Save changes to this screenshot as a .frame project?").as_ptr(),
            wide("frame").as_ptr(),
            MB_YESNOCANCEL | MB_ICONWARNING,
        )
    };
    if r == IDCANCEL {
        return false;
    }
    if r == IDYES {
        return save_project(hwnd, state, false);
    }
    true
}

// Menu command ids.
pub const CMD_SAVE_PROJECT: u16 = 200;
pub const CMD_OPEN: u16 = 201;
pub const CMD_EXPORT_PNG: u16 = 202;
pub const CMD_EXPORT_JPG: u16 = 203;
pub const CMD_COPY_IMAGE: u16 = 204;
pub const CMD_CLOSE: u16 = 205;
pub const CMD_SAVE_AS: u16 = 206;
pub const CMD_NEW_CLIPBOARD: u16 = 207;
pub const CMD_UNDO: u16 = 210;
pub const CMD_REDO: u16 = 211;
pub const CMD_CUT: u16 = 212;
pub const CMD_COPY: u16 = 213;
pub const CMD_PASTE: u16 = 214;
pub const CMD_DUPLICATE: u16 = 215;
pub const CMD_DELETE: u16 = 216;
pub const CMD_SELECT_ALL: u16 = 217;
pub const CMD_GROUP: u16 = 218;
pub const CMD_UNGROUP: u16 = 219;
pub const CMD_FRONT: u16 = 220;
pub const CMD_FORWARD: u16 = 221;
pub const CMD_BACKWARD: u16 = 222;
pub const CMD_BACK: u16 = 223;
pub const CMD_FLATTEN: u16 = 224;
pub const CMD_SEL_INVERT: u16 = 230;
pub const CMD_SEL_FILL: u16 = 231;
pub const CMD_SEL_CROP: u16 = 232;
pub const CMD_FX_BLUR: u16 = 233;
pub const CMD_FX_PIXELATE: u16 = 234;
pub const CMD_FX_GRAY: u16 = 235;
pub const CMD_FX_INVERT: u16 = 236;
pub const CMD_FX_BRIGHTEN: u16 = 237;
pub const CMD_FX_DARKEN: u16 = 238;
pub const CMD_BG_TRANSPARENT: u16 = 239;
pub const CMD_BG_AUTOFILL: u16 = 240;
pub const CMD_ZOOM_IN: u16 = 250;
pub const CMD_ZOOM_OUT: u16 = 251;
pub const CMD_ZOOM_FIT: u16 = 252;
pub const CMD_ZOOM_100: u16 = 253;
pub const CMD_TOOL_BASE: u16 = 300;

const VK_OEM_PLUS: u16 = 0xBB;
const VK_OEM_MINUS: u16 = 0xBD;
const VK_OEM_4: u16 = 0xDB; // [
const VK_OEM_6: u16 = 0xDD; // ]

/// Keyboard shortcuts for menu commands (handled in the canvas, which has
/// focus, and posted to the parent's WM_COMMAND).
pub fn accelerator(vk: u16, ctrl: bool, shift: bool, _alt: bool) -> Option<u16> {
    if !ctrl {
        return None;
    }
    let k = vk as u8;
    Some(match (k, shift) {
        (b'S', false) => CMD_SAVE_PROJECT,
        (b'S', true) => CMD_SAVE_AS,
        (b'O', _) => CMD_OPEN,
        (b'E', false) => CMD_EXPORT_PNG,
        (b'E', true) => CMD_EXPORT_JPG,
        (b'C', true) => CMD_COPY_IMAGE,
        (b'W', _) => CMD_CLOSE,
        (b'N', _) => CMD_NEW_CLIPBOARD,
        (b'Z', false) => CMD_UNDO,
        (b'Z', true) | (b'Y', _) => CMD_REDO,
        (b'X', _) => CMD_CUT,
        (b'C', false) => CMD_COPY,
        (b'V', _) => CMD_PASTE,
        (b'D', _) => CMD_DUPLICATE,
        (b'A', _) => CMD_SELECT_ALL,
        (b'G', false) => CMD_GROUP,
        (b'G', true) => CMD_UNGROUP,
        (b'I', true) => CMD_SEL_INVERT,
        (b'0', _) => CMD_ZOOM_FIT,
        (b'1', _) => CMD_ZOOM_100,
        _ => match (vk, shift) {
            (VK_OEM_PLUS, _) => CMD_ZOOM_IN,
            (VK_OEM_MINUS, _) => CMD_ZOOM_OUT,
            (VK_OEM_6, true) => CMD_FRONT,
            (VK_OEM_6, false) => CMD_FORWARD,
            (VK_OEM_4, false) => CMD_BACKWARD,
            (VK_OEM_4, true) => CMD_BACK,
            _ => return None,
        },
    })
}

unsafe fn item(menu: isize, id: u16, label: &str) {
    let w = wide(label);
    unsafe { AppendMenuW(menu, MF_STRING, id as usize, w.as_ptr()) };
}

unsafe fn sep(menu: isize) {
    unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, core::ptr::null()) };
}

unsafe fn sub(menu: isize, child: isize, label: &str) {
    let w = wide(label);
    unsafe { AppendMenuW(menu, MF_POPUP, child as usize, w.as_ptr()) };
}

unsafe fn build_edit_items(m: isize) {
    unsafe {
        item(m, CMD_UNDO, "Undo\tCtrl+Z");
        item(m, CMD_REDO, "Redo\tCtrl+Y");
        sep(m);
        item(m, CMD_CUT, "Cut\tCtrl+X");
        item(m, CMD_COPY, "Copy\tCtrl+C");
        item(m, CMD_PASTE, "Paste\tCtrl+V");
        item(m, CMD_DUPLICATE, "Duplicate\tCtrl+D");
        item(m, CMD_DELETE, "Delete\tDel");
        sep(m);
        item(m, CMD_SELECT_ALL, "Select all\tCtrl+A");
        item(m, CMD_GROUP, "Group\tCtrl+G");
        item(m, CMD_UNGROUP, "Ungroup\tCtrl+Shift+G");
        let order = CreatePopupMenu();
        item(order, CMD_FRONT, "Bring to front\tCtrl+Shift+]");
        item(order, CMD_FORWARD, "Bring forward\tCtrl+]");
        item(order, CMD_BACKWARD, "Send backward\tCtrl+[");
        item(order, CMD_BACK, "Send to back\tCtrl+Shift+[");
        sub(m, order, "Arrange");
        sep(m);
        item(m, CMD_FLATTEN, "Flatten annotations into image");
    }
}

unsafe fn build_selection_items(m: isize) {
    unsafe {
        item(m, CMD_SEL_INVERT, "Invert selection\tCtrl+Shift+I");
        item(m, CMD_SEL_FILL, "Delete selected pixels\tDel");
        item(m, CMD_SEL_CROP, "Crop to selection");
        let fx = CreatePopupMenu();
        item(fx, CMD_FX_BLUR, "Blur");
        item(fx, CMD_FX_PIXELATE, "Pixelate");
        item(fx, CMD_FX_GRAY, "Grayscale");
        item(fx, CMD_FX_INVERT, "Invert colours");
        item(fx, CMD_FX_BRIGHTEN, "Brighten");
        item(fx, CMD_FX_DARKEN, "Darken");
        sub(m, fx, "Effects");
        let bg = CreatePopupMenu();
        item(bg, CMD_BG_TRANSPARENT, "Transparent");
        item(bg, CMD_BG_AUTOFILL, "Auto-fill from surroundings");
        sub(m, bg, "Background fill");
    }
}

unsafe fn attach_menu(hwnd: HWND) {
    unsafe {
        let bar = CreateMenu();
        let file = CreatePopupMenu();
        item(file, CMD_NEW_CLIPBOARD, "New from clipboard\tCtrl+N");
        item(file, CMD_OPEN, "Open image or project…\tCtrl+O");
        sep(file);
        item(file, CMD_SAVE_PROJECT, "Save project (.frame)\tCtrl+S");
        item(file, CMD_SAVE_AS, "Save project as…\tCtrl+Shift+S");
        sep(file);
        item(file, CMD_EXPORT_PNG, "Export PNG…\tCtrl+E");
        item(file, CMD_EXPORT_JPG, "Export JPG…\tCtrl+Shift+E");
        item(file, CMD_COPY_IMAGE, "Copy image to clipboard\tCtrl+Shift+C");
        sep(file);
        item(file, CMD_CLOSE, "Close\tCtrl+W");
        sub(bar, file, "&File");

        let edit = CreatePopupMenu();
        build_edit_items(edit);
        sub(bar, edit, "&Edit");

        let image = CreatePopupMenu();
        build_selection_items(image);
        sub(bar, image, "&Image");

        let tools = CreatePopupMenu();
        for (i, t) in Tool::ALL.iter().enumerate() {
            item(tools, CMD_TOOL_BASE + i as u16, &format!("{}\t{}", t.name(), t.key() as char));
        }
        sub(bar, tools, "&Tools");

        let view = CreatePopupMenu();
        item(view, CMD_ZOOM_IN, "Zoom in\tCtrl+=");
        item(view, CMD_ZOOM_OUT, "Zoom out\tCtrl+-");
        item(view, CMD_ZOOM_FIT, "Fit to window\tCtrl+0");
        item(view, CMD_ZOOM_100, "Actual size\tCtrl+1");
        sub(bar, view, "&View");
        SetMenu(hwnd, bar);
    }
}

/// Grey out commands that don't apply and tick radio items.
fn update_menu_state(hwnd: HWND, state: &Rc<RefCell<EditorState>>) {
    let st = state.borrow();
    let c = &st.canvas;
    let menu = unsafe { GetMenu(hwnd) };
    apply_menu_state(menu, c);
}

fn apply_menu_state(menu: isize, c: &Canvas) {
    let has_sel = !c.selected.is_empty();
    let has_px = c.pixel_sel.is_some();
    let set = |id: u16, on: bool| unsafe {
        EnableMenuItem(menu, id as u32, MF_BYCOMMAND | if on { MF_ENABLED } else { MF_GRAYED });
    };
    let check = |id: u16, on: bool| unsafe {
        CheckMenuItem(menu, id as u32, MF_BYCOMMAND | if on { MF_CHECKED } else { MF_UNCHECKED });
    };
    set(CMD_UNDO, !c.history.undo.is_empty());
    set(CMD_REDO, !c.history.redo.is_empty());
    set(CMD_CUT, has_sel || has_px);
    set(CMD_COPY, has_sel || has_px);
    set(CMD_DUPLICATE, has_sel);
    set(CMD_DELETE, has_sel || has_px);
    set(CMD_GROUP, c.selected.len() > 1);
    set(CMD_UNGROUP, c.selected.iter().any(|id| c.doc.groups.contains_key(id)));
    for id in [CMD_FRONT, CMD_FORWARD, CMD_BACKWARD, CMD_BACK] {
        set(id, has_sel);
    }
    set(CMD_FLATTEN, !c.doc.annotations.is_empty());
    for id in [CMD_SEL_INVERT, CMD_SEL_FILL, CMD_SEL_CROP, CMD_FX_BLUR, CMD_FX_PIXELATE, CMD_FX_GRAY, CMD_FX_INVERT, CMD_FX_BRIGHTEN, CMD_FX_DARKEN] {
        set(id, has_px);
    }
    check(CMD_BG_TRANSPARENT, !c.style.sel_autofill);
    check(CMD_BG_AUTOFILL, c.style.sel_autofill);
    for (i, t) in Tool::ALL.iter().enumerate() {
        check(CMD_TOOL_BASE + i as u16, c.active_tool == *t);
    }
}

/// Right-click menu on the canvas: edit commands, plus selection operations
/// when a pixel selection exists.
pub fn show_context_menu(parent: HWND, x: i32, y: i32) {
    let Some(state) = get_state(parent) else { return };
    let cmd = unsafe {
        let m = CreatePopupMenu();
        if state.borrow().canvas.pixel_sel.is_some() {
            item(m, CMD_COPY, "Copy\tCtrl+C");
            item(m, CMD_CUT, "Cut\tCtrl+X");
            item(m, CMD_PASTE, "Paste\tCtrl+V");
            sep(m);
            build_selection_items(m);
        } else {
            build_edit_items(m);
        }
        apply_menu_state(m, &state.borrow().canvas);
        let c = TrackPopupMenu(m, TPM_RIGHTBUTTON | TPM_RETURNCMD, x, y, 0, parent, core::ptr::null());
        DestroyMenu(m);
        c as u16
    };
    if cmd != 0 {
        handle_command(parent, &state, cmd);
    }
}

fn save_project(hwnd: HWND, state: &Rc<RefCell<EditorState>>, force_dialog: bool) -> bool {
    let existing = state.borrow().current_path.clone();
    let path = match existing {
        Some(p) if !force_dialog => Some(p),
        _ => crate::win::dialog::save_file(hwnd, "Save project", "Frame project|*.frame|All files|*.*", "screenshot.frame").ok().flatten(),
    };
    let Some(mut p) = path else { return false };
    if !p.to_ascii_lowercase().ends_with(".frame") {
        p.push_str(".frame");
    }
    let ok = crate::io::project::save(&state.borrow().canvas.doc, &p).is_ok();
    if ok {
        let mut st = state.borrow_mut();
        st.current_path = Some(p);
        st.canvas.dirty = false;
    } else {
        message(hwnd, "Could not save the project.");
    }
    ok
}

fn message(hwnd: HWND, text: &str) {
    unsafe {
        MessageBoxW(hwnd, wide(text).as_ptr(), wide("frame").as_ptr(), MB_ICONWARNING);
    }
}

/// Decide how to treat reversible (blur/pixelate) redactions on export.
/// Returns None if the user cancelled, else Some(secure).
fn ask_secure(hwnd: HWND, state: &Rc<RefCell<EditorState>>) -> Option<bool> {
    if !state.borrow().canvas.has_soft_redactions() {
        return Some(false);
    }
    let r = unsafe {
        MessageBoxW(
            hwnd,
            wide("This image contains blurred or pixelated areas.\n\nBlur and pixelation can sometimes be reversed by third-party tools, so they are not safe for hiding sensitive information.\n\nExport them as solid (irreversible) redactions instead?").as_ptr(),
            wide("frame — redaction warning").as_ptr(),
            MB_YESNOCANCEL | MB_ICONWARNING,
        )
    };
    if r == IDCANCEL {
        None
    } else {
        Some(r == IDYES)
    }
}

fn export(hwnd: HWND, state: &Rc<RefCell<EditorState>>, jpg: bool) {
    let Some(secure) = ask_secure(hwnd, state) else { return };
    let (title, filter, def) = if jpg {
        ("Export JPG", "JPEG image|*.jpg;*.jpeg", "screenshot.jpg")
    } else {
        ("Export PNG", "PNG image|*.png", "screenshot.png")
    };
    let Some(mut p) = crate::win::dialog::save_file(hwnd, title, filter, def).ok().flatten() else { return };
    let lower = p.to_ascii_lowercase();
    if jpg && !(lower.ends_with(".jpg") || lower.ends_with(".jpeg")) {
        p.push_str(".jpg");
    } else if !jpg && !lower.ends_with(".png") {
        p.push_str(".png");
    }
    let bmp = state.borrow().canvas.flatten(secure);
    let r = bmp.and_then(|b| {
        if jpg {
            // JPEG has no alpha: composite transparent areas onto white.
            let mut b = b;
            for px in b.bgra.chunks_exact_mut(4) {
                let a = px[3] as u32;
                for c in 0..3 {
                    px[c] = ((px[c] as u32 * a + 255 * (255 - a)) / 255) as u8;
                }
                px[3] = 255;
            }
            crate::gfx::gdiplus::save_jpg(&b, &p, 90)
        } else {
            crate::gfx::gdiplus::save_png(&b, &p)
        }
    });
    if r.is_err() {
        message(hwnd, "Export failed.");
    }
}

fn open_file(hwnd: HWND, state: &Rc<RefCell<EditorState>>) {
    if !confirm_discard(hwnd, state) {
        return;
    }
    let Some(p) = crate::win::dialog::open_file(
        hwnd,
        "Open image or project",
        "Images and projects|*.png;*.jpg;*.jpeg;*.bmp;*.gif;*.frame|Frame project|*.frame|All files|*.*",
    )
    .ok()
    .flatten() else {
        return;
    };
    let is_project = p.to_ascii_lowercase().ends_with(".frame");
    let doc = if is_project {
        crate::io::project::load(&p).ok()
    } else {
        crate::gfx::gdiplus::decode_file(&p).ok().map(Document::new)
    };
    let Some(doc) = doc else {
        message(hwnd, "Could not open that file.");
        return;
    };
    {
        let mut st = state.borrow_mut();
        let _ = st.canvas.replace_document(doc);
        st.current_path = if is_project { Some(p) } else { None };
    }
}

pub fn handle_command(hwnd: HWND, state: &Rc<RefCell<EditorState>>, cmd: u16) {
    if (CMD_TOOL_BASE..CMD_TOOL_BASE + Tool::ALL.len() as u16).contains(&cmd) {
        state.borrow_mut().canvas.set_tool(Tool::ALL[(cmd - CMD_TOOL_BASE) as usize]);
    } else {
        match cmd {
            CMD_SAVE_PROJECT => {
                save_project(hwnd, state, false);
            }
            CMD_SAVE_AS => {
                save_project(hwnd, state, true);
            }
            CMD_OPEN => open_file(hwnd, state),
            CMD_NEW_CLIPBOARD => {
                let bm = crate::win::clipboard::read_image(hwnd);
                match bm {
                    Some(bm) => {
                        if confirm_discard(hwnd, state) {
                            let mut st = state.borrow_mut();
                            let _ = st.canvas.replace_document(Document::new(bm));
                            st.current_path = None;
                        }
                    }
                    None => message(hwnd, "The clipboard doesn't contain an image."),
                }
            }
            CMD_EXPORT_PNG => export(hwnd, state, false),
            CMD_EXPORT_JPG => export(hwnd, state, true),
            CMD_COPY_IMAGE => {
                if let Some(secure) = ask_secure(hwnd, state) {
                    let bmp = state.borrow().canvas.flatten(secure);
                    if let Ok(b) = bmp {
                        let _ = crate::editor::canvas::put_clipboard_image(hwnd, &b);
                    }
                }
            }
            CMD_CLOSE => unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(hwnd, WM_CLOSE, 0, 0);
            },
            _ => {
                let mut st = state.borrow_mut();
                let c = &mut st.canvas;
                match cmd {
                    CMD_UNDO => {
                        c.undo();
                    }
                    CMD_REDO => {
                        c.redo();
                    }
                    CMD_CUT => {
                        c.cut();
                    }
                    CMD_COPY => {
                        c.copy();
                    }
                    CMD_PASTE => {
                        c.paste();
                    }
                    CMD_DUPLICATE => {
                        c.duplicate();
                    }
                    CMD_DELETE => {
                        c.delete_selection();
                    }
                    CMD_SELECT_ALL => c.select_all(),
                    CMD_GROUP => {
                        c.group();
                    }
                    CMD_UNGROUP => {
                        c.ungroup();
                    }
                    CMD_FRONT => {
                        c.reorder(true, false);
                    }
                    CMD_FORWARD => {
                        c.reorder(true, true);
                    }
                    CMD_BACKWARD => {
                        c.reorder(false, true);
                    }
                    CMD_BACK => {
                        c.reorder(false, false);
                    }
                    CMD_FLATTEN => {
                        c.flatten_annotations();
                    }
                    CMD_SEL_INVERT => {
                        c.sel_invert();
                    }
                    CMD_SEL_FILL => {
                        let auto = c.style.sel_autofill;
                        c.sel_fill(auto);
                    }
                    CMD_SEL_CROP => {
                        c.sel_crop();
                    }
                    CMD_FX_BLUR => {
                        c.sel_effect(Effect::Blur);
                    }
                    CMD_FX_PIXELATE => {
                        c.sel_effect(Effect::Pixelate);
                    }
                    CMD_FX_GRAY => {
                        c.sel_effect(Effect::Grayscale);
                    }
                    CMD_FX_INVERT => {
                        c.sel_effect(Effect::Invert);
                    }
                    CMD_FX_BRIGHTEN => {
                        c.sel_effect(Effect::Brighten);
                    }
                    CMD_FX_DARKEN => {
                        c.sel_effect(Effect::Darken);
                    }
                    CMD_BG_TRANSPARENT => c.style.sel_autofill = false,
                    CMD_BG_AUTOFILL => c.style.sel_autofill = true,
                    CMD_ZOOM_IN => c.zoom_step(1.0),
                    CMD_ZOOM_OUT => c.zoom_step(-1.0),
                    CMD_ZOOM_FIT => c.zoom_fit(),
                    CMD_ZOOM_100 => c.zoom_to(1.0, c.size.0 as f32 * 0.5, c.size.1 as f32 * 0.5),
                    _ => {}
                }
            }
        }
    }
    let cv = state.borrow().canvas_hwnd;
    unsafe { InvalidateRect(cv, core::ptr::null(), 0) };
    refresh_panels(hwnd);
}
