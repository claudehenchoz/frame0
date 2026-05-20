use crate::app::App;
use crate::editor::annotation::{Document, Point};
use crate::editor::canvas::Canvas;
use crate::editor::history::{Command, History};
use crate::editor::tools::{Drag, Tool, ToolStyle, commit_drag};
use crate::editor::toolbar::{PALETTE, SWATCH_SIZE, TOOLBAR_HEIGHT, TOOLS, ToolEntry};
use crate::error::{Error, Result};
use crate::gfx::bitmap::Bitmap;
use crate::gfx::color::Color;
use crate::win::wide;
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HBRUSH, PAINTSTRUCT,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CW_USEDEFAULT, CreateMenu, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyWindow, GetClientRect, IDC_ARROW, LoadCursorW, MF_POPUP, MF_SEPARATOR, MF_STRING,
    PostMessageW, RegisterClassExW, SetMenu, SetWindowLongPtrW, SetWindowTextW, ShowWindow,
    SW_RESTORE, SW_SHOW, GWLP_USERDATA, GetWindowLongPtrW, WM_CLOSE, WM_COMMAND, WM_DESTROY,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDOWN,
    WM_SIZE, WNDCLASSEXW, WS_OVERLAPPEDWINDOW, WS_VISIBLE, GetSystemMetrics, SM_CXSCREEN,
    SM_CYSCREEN, SetForegroundWindow, BringWindowToTop, MoveWindow,
};
use windows_sys::Win32::Graphics::Gdi::{InvalidateRect, UpdateWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_BACK, VK_CONTROL, VK_DELETE, VK_ESCAPE, VK_RETURN, VK_Y, VK_Z, GetKeyState,
    SetCapture, ReleaseCapture,
};

const CLASS_NAME: &str = "frame.editor";
const COLOR_BG: Color = Color::rgb(0x1A, 0x1B, 0x1D);

pub struct EditorWindow {
    pub hwnd: HWND,
    pub state: Rc<RefCell<EditorState>>,
}

pub struct EditorState {
    pub app: Weak<App>,
    pub canvas: Canvas,
    pub history: History,
    pub current_path: Option<String>,
}

thread_local! {
    static EDITOR_STATE_BY_HWND: RefCell<Option<(HWND, Rc<RefCell<EditorState>>)>> = const { RefCell::new(None) };
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
            }));
            EDITOR_STATE_BY_HWND.with(|s| *s.borrow_mut() = Some((hwnd, state.clone())));

            // Attach D2D swap chain.
            let mut client = core::mem::zeroed();
            GetClientRect(hwnd, &mut client);
            state.borrow_mut().canvas.attach(
                hwnd,
                (client.right as u32).max(1),
                ((client.bottom - TOOLBAR_HEIGHT) as u32).max(1),
            )?;

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
        if let Err(_) = self.state.borrow_mut().canvas.replace_document(doc) {
            return;
        }
        unsafe {
            InvalidateRect(self.hwnd, core::ptr::null(), 0);
        }
    }

    pub fn replace_document(&mut self, doc: Document) {
        if let Err(_) = self.state.borrow_mut().canvas.replace_document(doc) {
            return;
        }
        unsafe {
            InvalidateRect(self.hwnd, core::ptr::null(), 0);
        }
    }
}

impl Drop for EditorWindow {
    fn drop(&mut self) {
        EDITOR_STATE_BY_HWND.with(|s| *s.borrow_mut() = None);
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
    let state = EDITOR_STATE_BY_HWND.with(|s| s.borrow().as_ref().and_then(|(h, st)| if *h == hwnd { Some(st.clone()) } else { None }));
    if state.is_none() {
        return unsafe { DefWindowProcW(hwnd, msg, w, l) };
    }
    let state = state.unwrap();
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = core::mem::zeroed();
                BeginPaint(hwnd, &mut ps);
                // Draw toolbar via GDI.
                paint_toolbar(hwnd, &state.borrow());
                // Render canvas via D2D (full client minus toolbar).
                let hdc = ps.hdc;
                state.borrow().canvas.render(hwnd, hdc);
                EndPaint(hwnd, &ps);
                0
            }
            WM_SIZE => {
                let cw = (l & 0xFFFF) as u16 as u32;
                let ch = ((l >> 16) & 0xFFFF) as u16 as u32;
                let _ = state
                    .borrow_mut()
                    .canvas
                    .resize(cw.max(1), ch.saturating_sub(TOOLBAR_HEIGHT as u32).max(1));
                InvalidateRect(hwnd, core::ptr::null(), 0);
                0
            }
            WM_LBUTTONDOWN => {
                let x = (l & 0xFFFF) as i16 as i32;
                let y = ((l >> 16) & 0xFFFF) as i16 as i32;
                if y < TOOLBAR_HEIGHT {
                    handle_toolbar_click(&state, x, y);
                    InvalidateRect(hwnd, core::ptr::null(), 0);
                    return 0;
                }
                SetCapture(hwnd);
                let mut st = state.borrow_mut();
                let p = st.canvas.screen_to_doc(x as f32, (y - TOOLBAR_HEIGHT) as f32);
                if st.canvas.active_tool == Tool::Move {
                    st.canvas.selected = st.canvas.doc.topmost_hit(p);
                    st.canvas.move_anchor = Some(p);
                } else {
                    st.canvas.drag = Some(Drag { start: p, current: p });
                }
                drop(st);
                InvalidateRect(hwnd, core::ptr::null(), 0);
                0
            }
            WM_MOUSEMOVE => {
                let x = (l & 0xFFFF) as i16 as i32;
                let y = ((l >> 16) & 0xFFFF) as i16 as i32;
                let mut st = state.borrow_mut();
                let p = st.canvas.screen_to_doc(x as f32, (y - TOOLBAR_HEIGHT) as f32);
                if let Some(drag) = st.canvas.drag.as_mut() {
                    drag.current = p;
                    drop(st);
                    InvalidateRect(hwnd, core::ptr::null(), 0);
                    return 0;
                }
                if st.canvas.active_tool == Tool::Move {
                    if let (Some(sel), Some(anchor)) = (st.canvas.selected, st.canvas.move_anchor) {
                        if let Some(i) = st.canvas.doc.find_index(sel) {
                            let dx = p.x - anchor.x;
                            let dy = p.y - anchor.y;
                            st.canvas.doc.annotations[i].translate(dx, dy);
                            st.canvas.move_anchor = Some(p);
                            drop(st);
                            InvalidateRect(hwnd, core::ptr::null(), 0);
                        }
                    }
                }
                0
            }
            WM_LBUTTONUP => {
                ReleaseCapture();
                let mut st = state.borrow_mut();
                if let Some(drag) = st.canvas.drag.take() {
                    let tool = st.canvas.active_tool;
                    let style = st.canvas.style;
                    if let Some(cmd) = commit_drag(tool, style, drag, &mut st.canvas.doc) {
                        st.history.push(cmd);
                    }
                }
                st.canvas.move_anchor = None;
                drop(st);
                InvalidateRect(hwnd, core::ptr::null(), 0);
                0
            }
            WM_KEYDOWN => {
                let ctrl = (GetKeyState(VK_CONTROL as i32) as u16 & 0x8000) != 0;
                let mut st = state.borrow_mut();
                match w as u32 {
                    v if v == VK_ESCAPE as u32 => {
                        st.canvas.selected = None;
                        drop(st);
                        InvalidateRect(hwnd, core::ptr::null(), 0);
                    }
                    v if v == VK_DELETE as u32 || v == VK_BACK as u32 => {
                        if let Some(sel) = st.canvas.selected {
                            if let Some(idx) = st.canvas.doc.find_index(sel) {
                                let ann = st.canvas.doc.annotations.remove(idx);
                                st.history.push(Command::Remove { index: idx, ann });
                                st.canvas.selected = None;
                            }
                        }
                        drop(st);
                        InvalidateRect(hwnd, core::ptr::null(), 0);
                    }
                    v if ctrl && v == VK_Z as u32 => {
                        let s = &mut *st;
                        let (doc, history) = (&mut s.canvas.doc, &mut s.history);
                        history.undo(doc);
                        drop(st);
                        InvalidateRect(hwnd, core::ptr::null(), 0);
                    }
                    v if ctrl && v == VK_Y as u32 => {
                        let s = &mut *st;
                        let (doc, history) = (&mut s.canvas.doc, &mut s.history);
                        history.redo(doc);
                        drop(st);
                        InvalidateRect(hwnd, core::ptr::null(), 0);
                    }
                    _ => {}
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
                if let Some(app) = state.borrow().app.upgrade() {
                    app.editor_closed();
                }
                EDITOR_STATE_BY_HWND.with(|s| *s.borrow_mut() = None);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

unsafe fn paint_toolbar(hwnd: HWND, st: &EditorState) {
    unsafe {
        let hdc = windows_sys::Win32::Graphics::Gdi::GetDC(hwnd);
        let mut rc: windows_sys::Win32::Foundation::RECT = core::mem::zeroed();
        GetClientRect(hwnd, &mut rc);
        let toolbar_rc = windows_sys::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: rc.right,
            bottom: TOOLBAR_HEIGHT,
        };
        let bg = CreateSolidBrush(0x00222426);
        FillRect(hdc, &toolbar_rc, bg);
        DeleteObject(bg as _);

        let mut x = 8;
        for entry in TOOLS {
            draw_tool_button(hdc, x, 8, entry, st.canvas.active_tool == entry.tool);
            x += 64;
        }
        x += 12;
        for c in PALETTE {
            let r = windows_sys::Win32::Foundation::RECT {
                left: x,
                top: (TOOLBAR_HEIGHT - SWATCH_SIZE) / 2,
                right: x + SWATCH_SIZE,
                bottom: (TOOLBAR_HEIGHT - SWATCH_SIZE) / 2 + SWATCH_SIZE,
            };
            let bgr = ((c.b as u32) | ((c.g as u32) << 8) | ((c.r as u32) << 16)) as u32;
            let br = CreateSolidBrush(bgr);
            FillRect(hdc, &r, br);
            DeleteObject(br as _);
            if *c == st.canvas.style.color {
                let outline = CreateSolidBrush(0x00FFFFFF);
                let or = windows_sys::Win32::Foundation::RECT {
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

        windows_sys::Win32::Graphics::Gdi::ReleaseDC(hwnd, hdc);
    }
}

unsafe fn draw_tool_button(
    hdc: windows_sys::Win32::Graphics::Gdi::HDC,
    x: i32,
    y: i32,
    entry: &ToolEntry,
    active: bool,
) {
    unsafe {
        let r = windows_sys::Win32::Foundation::RECT {
            left: x,
            top: y,
            right: x + 60,
            bottom: y + TOOLBAR_HEIGHT - 16,
        };
        let color = if active { 0x00505964 } else { 0x002E3236 };
        let bg = CreateSolidBrush(color);
        FillRect(hdc, &r, bg);
        DeleteObject(bg as _);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, 0x00E0E0E0);
        let label = wide(entry.label);
        windows_sys::Win32::Graphics::Gdi::TextOutW(hdc, x + 8, y + 4, label.as_ptr(), (label.len() - 1) as i32);
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
                    if let Err(_) = state.borrow_mut().canvas.replace_document(doc) {
                        return;
                    }
                    unsafe {
                        InvalidateRect(hwnd, core::ptr::null(), 0);
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

fn handle_toolbar_click(state: &Rc<RefCell<EditorState>>, x: i32, _y: i32) {
    let mut st = state.borrow_mut();
    let mut cx = 8;
    for entry in TOOLS {
        if x >= cx && x < cx + 60 {
            st.canvas.active_tool = entry.tool;
            return;
        }
        cx += 64;
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

#[allow(dead_code)]
fn _silence_unused() {
    let _ = (Point::new(0.0, 0.0), VK_RETURN, SetWindowLongPtrW, SetWindowTextW, GetWindowLongPtrW, GWLP_USERDATA, PostMessageW, MoveWindow, ToolStyle::default());
    let _: HBRUSH = 0;
}
