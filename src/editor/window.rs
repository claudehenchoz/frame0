//! Editor parent window. Owns the menu, the layout of three children (toolbar,
//! canvas, property panel), and the shared `EditorState`. Child windows look
//! the state up via the thread-local map keyed by the parent's HWND.

use crate::app::App;
use crate::editor::annotation::Document;
use crate::editor::canvas::Canvas;
use crate::editor::history::{Command, History};
use crate::editor::layout::{TOOLBAR_HEIGHT, canvas_rect, toolbar_rect};
use crate::error::{Error, Result};
use crate::gfx::bitmap::Bitmap;
use crate::win::wide;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{InvalidateRect, UpdateWindow};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, BringWindowToTop, CW_USEDEFAULT, CreateMenu, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyWindow, GetClientRect, GetSystemMetrics, IDC_ARROW, LoadCursorW,
    MF_POPUP, MF_SEPARATOR, MF_STRING, RegisterClassExW, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE,
    SWP_NOZORDER, SetForegroundWindow, SetMenu, SetWindowPos, ShowWindow, SW_RESTORE, SW_SHOW,
    WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_ERASEBKGND, WM_SIZE, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE,
};

const CLASS_NAME: &str = "frame.editor";

pub struct EditorWindow {
    pub hwnd: HWND,
    pub state: Rc<RefCell<EditorState>>,
}

pub struct EditorState {
    pub app: Weak<App>,
    pub canvas: Canvas,
    pub history: History,
    pub current_path: Option<String>,
    pub canvas_hwnd: HWND,
    pub toolbar_hwnd: HWND,
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
            let (sw, sh) = (
                GetSystemMetrics(SM_CXSCREEN).max(1024),
                GetSystemMetrics(SM_CYSCREEN).max(720),
            );
            let init_w = ((doc.base.width as i32) + 80).min(sw).max(800);
            let init_h = ((doc.base.height as i32) + TOOLBAR_HEIGHT + 80).min(sh).max(600);
            let hwnd = CreateWindowExW(
                0,
                cname.as_ptr(),
                wide("frame").as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
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
                history: History::new(),
                current_path: path,
                canvas_hwnd: 0,
                toolbar_hwnd: 0,
            }));
            put_state(hwnd, state.clone());

            let mut client: RECT = core::mem::zeroed();
            GetClientRect(hwnd, &mut client);
            let tb_hwnd =
                crate::editor::toolbar::create_child(hwnd, toolbar_rect(&client))?;
            let cv_hwnd =
                crate::editor::canvas::create_child(hwnd, canvas_rect(&client))?;

            put_state(tb_hwnd, state.clone());
            put_state(cv_hwnd, state.clone());

            // Hand the canvas's HWND to the Canvas itself so it can build its
            // swap chain.
            {
                let cr = canvas_rect(&client);
                let mut st = state.borrow_mut();
                st.canvas_hwnd = cv_hwnd;
                st.toolbar_hwnd = tb_hwnd;
                st.canvas.attach(
                    cv_hwnd,
                    (cr.right - cr.left).max(1) as u32,
                    (cr.bottom - cr.top).max(1) as u32,
                )?;
            }

            ShowWindow(hwnd, SW_SHOW);
            UpdateWindow(hwnd);
            SetForegroundWindow(hwnd);

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
        let doc = Document::new(bmp);
        let cv_hwnd = {
            let mut st = self.state.borrow_mut();
            if let Err(_) = st.canvas.replace_document(doc) {
                return;
            }
            st.canvas_hwnd
        };
        unsafe {
            InvalidateRect(cv_hwnd, core::ptr::null(), 0);
        }
    }

    pub fn replace_document(&mut self, doc: Document) {
        let cv_hwnd = {
            let mut st = self.state.borrow_mut();
            if let Err(_) = st.canvas.replace_document(doc) {
                return;
            }
            st.canvas_hwnd
        };
        unsafe {
            InvalidateRect(cv_hwnd, core::ptr::null(), 0);
        }
    }
}

impl Drop for EditorWindow {
    fn drop(&mut self) {
        // Cleanup handled by WM_DESTROY (which fires on the parent and clears
        // entries for parent + children). Nothing to do here.
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
                let tb = toolbar_rect(&client);
                let cv = canvas_rect(&client);
                let (tb_hwnd, cv_hwnd) = {
                    let st = state.borrow();
                    (st.toolbar_hwnd, st.canvas_hwnd)
                };
                if tb_hwnd != 0 {
                    SetWindowPos(
                        tb_hwnd,
                        0,
                        tb.left,
                        tb.top,
                        tb.right - tb.left,
                        tb.bottom - tb.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                if cv_hwnd != 0 {
                    SetWindowPos(
                        cv_hwnd,
                        0,
                        cv.left,
                        cv.top,
                        cv.right - cv.left,
                        cv.bottom - cv.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                0
            }
            WM_COMMAND => {
                handle_command(hwnd, &state, (w & 0xFFFF) as u16);
                0
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                let (tb, cv) = {
                    let st = state.borrow();
                    (st.toolbar_hwnd, st.canvas_hwnd)
                };
                if tb != 0 {
                    clear_state(tb);
                }
                if cv != 0 {
                    clear_state(cv);
                }
                clear_state(hwnd);
                if let Some(app) = state.borrow().app.upgrade() {
                    app.editor_closed();
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

const CMD_SAVE_PROJECT: u16 = 200;
const CMD_OPEN_PROJECT: u16 = 201;
const CMD_EXPORT_PNG: u16 = 202;
const CMD_EXPORT_JPG: u16 = 203;
const CMD_COPY: u16 = 204;
const CMD_CLOSE: u16 = 205;

unsafe fn attach_menu(hwnd: HWND) {
    unsafe {
        let bar = CreateMenu();
        let file = CreatePopupMenu();
        let s_save = wide("Save Project (.frame)\tCtrl+S");
        let s_open = wide("Open Project (.frame)…\tCtrl+O");
        let s_png = wide("Export PNG…");
        let s_jpg = wide("Export JPG…");
        let s_copy = wide("Copy to Clipboard\tCtrl+C");
        let s_close = wide("Close\tCtrl+W");
        AppendMenuW(file, MF_STRING, CMD_SAVE_PROJECT as usize, s_save.as_ptr());
        AppendMenuW(file, MF_STRING, CMD_OPEN_PROJECT as usize, s_open.as_ptr());
        AppendMenuW(file, MF_SEPARATOR, 0, core::ptr::null());
        AppendMenuW(file, MF_STRING, CMD_EXPORT_PNG as usize, s_png.as_ptr());
        AppendMenuW(file, MF_STRING, CMD_EXPORT_JPG as usize, s_jpg.as_ptr());
        AppendMenuW(file, MF_STRING, CMD_COPY as usize, s_copy.as_ptr());
        AppendMenuW(file, MF_SEPARATOR, 0, core::ptr::null());
        AppendMenuW(file, MF_STRING, CMD_CLOSE as usize, s_close.as_ptr());
        let s_file = wide("&File");
        AppendMenuW(bar, MF_POPUP, file as usize, s_file.as_ptr());
        SetMenu(hwnd, bar);
    }
}

fn handle_command(hwnd: HWND, state: &Rc<RefCell<EditorState>>, cmd: u16) {
    match cmd {
        CMD_SAVE_PROJECT => {
            let path = crate::win::dialog::save_file(
                hwnd,
                "Save project",
                "Frame project|*.frame|All files|*.*",
                "untitled.frame",
            )
            .ok()
            .flatten();
            if let Some(p) = path {
                let st = state.borrow();
                let _ = crate::io::project::save(&st.canvas.doc, &p);
            }
        }
        CMD_OPEN_PROJECT => {
            let path = crate::win::dialog::open_file(
                hwnd,
                "Open project",
                "Frame project|*.frame|All files|*.*",
            )
            .ok()
            .flatten();
            if let Some(p) = path {
                if let Ok(doc) = crate::io::project::load(&p) {
                    let cv = {
                        let mut st = state.borrow_mut();
                        if st.canvas.replace_document(doc).is_err() {
                            return;
                        }
                        st.canvas_hwnd
                    };
                    unsafe {
                        InvalidateRect(cv, core::ptr::null(), 0);
                    }
                }
            }
        }
        CMD_EXPORT_PNG => {
            let path = crate::win::dialog::save_file(
                hwnd,
                "Export PNG",
                "PNG image|*.png",
                "frame.png",
            )
            .ok()
            .flatten();
            if let Some(p) = path {
                let st = state.borrow();
                let _ = crate::io::export_png(&st.canvas.doc, &p);
            }
        }
        CMD_EXPORT_JPG => {
            let path = crate::win::dialog::save_file(
                hwnd,
                "Export JPG",
                "JPEG image|*.jpg;*.jpeg",
                "frame.jpg",
            )
            .ok()
            .flatten();
            if let Some(p) = path {
                let st = state.borrow();
                let _ = crate::io::export_jpg(&st.canvas.doc, &p);
            }
        }
        CMD_COPY => {
            let st = state.borrow();
            let _ = crate::io::copy_to_clipboard(hwnd, &st.canvas.doc);
        }
        CMD_CLOSE => unsafe {
            DestroyWindow(hwnd);
        },
        _ => {}
    }
}
