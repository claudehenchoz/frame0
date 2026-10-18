use crate::error::{Error, Result};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::HBRUSH;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HCURSOR, HICON,
    HMENU, MSG, PostQuitMessage, RegisterClassExW, TranslateMessage, WINDOW_EX_STYLE,
    WINDOW_STYLE, WNDCLASSEXW,
};

pub type WndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

pub struct WindowClass {
    pub atom: u16,
    pub name: Vec<u16>,
}

pub fn register_class(name: &str, wndproc: WndProc, cursor: HCURSOR, bg: HBRUSH) -> Result<WindowClass> {
    let wname = crate::win::wide(name);
    unsafe {
        let hinst = GetModuleHandleW(core::ptr::null()) as _;
        let wc = WNDCLASSEXW {
            cbSize: core::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: 0 as HICON,
            hCursor: cursor,
            hbrBackground: bg,
            lpszMenuName: core::ptr::null(),
            lpszClassName: wname.as_ptr(),
            hIconSm: 0 as HICON,
        };
        let atom = RegisterClassExW(&wc);
        if atom == 0 {
            return Err(Error::last_win32());
        }
        Ok(WindowClass { atom, name: wname })
    }
}

pub struct WindowOpts<'a> {
    pub class: &'a WindowClass,
    pub title: &'a str,
    pub ex_style: WINDOW_EX_STYLE,
    pub style: WINDOW_STYLE,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub parent: HWND,
    pub menu: HMENU,
    pub user_data: *mut core::ffi::c_void,
}

pub fn create_window(opts: &WindowOpts) -> Result<HWND> {
    let title = crate::win::wide(opts.title);
    unsafe {
        let hinst = GetModuleHandleW(core::ptr::null()) as _;
        let hwnd = CreateWindowExW(
            opts.ex_style,
            opts.class.name.as_ptr(),
            title.as_ptr(),
            opts.style,
            opts.x,
            opts.y,
            opts.w,
            opts.h,
            opts.parent,
            opts.menu,
            hinst,
            opts.user_data,
        );
        if hwnd == 0 {
            return Err(Error::last_win32());
        }
        Ok(hwnd)
    }
}

pub fn default_xy() -> (i32, i32) {
    (CW_USEDEFAULT, CW_USEDEFAULT)
}

pub fn message_loop() {
    unsafe {
        let mut msg: MSG = core::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

pub fn post_quit(code: i32) {
    unsafe { PostQuitMessage(code) }
}

pub unsafe extern "system" fn default_wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// Nested message loop used by the modal overlays and prompts. Runs until
/// `done()` returns true. A WM_QUIT received meanwhile is re-posted so the
/// outer loop still exits. If `dialog` is non-zero, IsDialogMessageW gives
/// it Tab / Enter / Esc navigation.
pub fn modal_loop(dialog: HWND, done: &dyn Fn() -> bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsDialogMessageW;
    unsafe {
        let mut msg: MSG = core::mem::zeroed();
        while !done() {
            if GetMessageW(&mut msg, 0, 0, 0) <= 0 {
                PostQuitMessage(msg.wParam as i32);
                break;
            }
            if dialog != 0 && IsDialogMessageW(dialog, &msg) != 0 {
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

pub fn sleep_ms(ms: u32) {
    unsafe { windows_sys::Win32::System::Threading::Sleep(ms) }
}
