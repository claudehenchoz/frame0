//! Window-pick overlay with hover highlight.
//!
//! Fullscreen transparent overlay showing the captured desktop. As the user
//! moves the mouse, the top-level window under the cursor is highlighted with
//! a green border and a title label. Left-click confirms, Escape/right-click
//! cancels.
//!
//! Architecture mirrors `region.rs`: modal message loop + double-buffered
//! GDI painting.

use crate::error::Result;
use crate::gfx::bitmap::Bitmap;
use crate::win::wide;
use std::cell::RefCell;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AlphaBlend, BLENDFUNCTION, BitBlt, CreateCompatibleDC, CreatePen, DeleteDC, DeleteObject,
    HBITMAP, HBRUSH, HPEN, InvalidateRect, PAINTSTRUCT, PS_SOLID, Polyline, BeginPaint, EndPaint,
    GetDC, ReleaseDC, SRCCOPY, SelectObject, CreateSolidBrush, FillRect, AC_SRC_OVER,
    SetBkMode, TRANSPARENT, SetTextColor, TextOutW, CreateFontW, FW_NORMAL, DEFAULT_CHARSET,
    OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, VARIABLE_PITCH,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    IDC_CROSS, LoadCursorW, MSG, PostMessageW, RegisterClassExW,
    SetCursor, ShowWindow, SW_SHOW, TranslateMessage, WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_MOUSEMOVE, WM_PAINT, WM_QUIT, WM_RBUTTONDOWN, WNDCLASSEXW, WS_EX_TOPMOST,
    WS_POPUP, WS_VISIBLE, GetAncestor, GA_ROOT, GetWindowRect, GetWindowTextW, IsWindowVisible,
    EnumWindows,
};


const CLASS_NAME: &str = "frame.overlay.windowpick";

struct WinEntry {
    hwnd: isize,
    rect: (i32, i32, i32, i32),
}

struct State {
    desktop: isize,
    desktop_bmp: isize,
    desktop_old: isize,
    width: i32,
    height: i32,
    origin_x: i32,
    origin_y: i32,
    windows: Vec<WinEntry>,
    hovered_hwnd: isize,
    hovered_rect: (i32, i32, i32, i32),
    confirmed: Option<isize>,
    cancelled: bool,
    hwnd: isize,
}

thread_local! {
    static STATE: RefCell<Option<*mut State>> = const { RefCell::new(None) };
    static ENUM_RESULT: RefCell<Option<*mut Vec<WinEntry>>> = const { RefCell::new(None) };
}

unsafe extern "system" fn enum_windows_proc(hwnd: isize, lparam: isize) -> i32 {
    unsafe {
        if hwnd == 0 || IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let root = GetAncestor(hwnd, GA_ROOT);
        if root == 0 || IsWindowVisible(root) == 0 {
            return 1;
        }
        // Skip the overlay itself (identified by lparam).
        if root == lparam {
            return 1;
        }
        let result = ENUM_RESULT.with(|r| *r.borrow());
        if let Some(windows) = result {
            // Check if already in list (avoid duplicates from child windows).
            if !(*windows).iter().any(|e| e.hwnd == root) {
                let mut wr: RECT = core::mem::zeroed();
                if GetWindowRect(root, &mut wr) != 0 {
                    (*windows).push(WinEntry {
                        hwnd: root,
                        rect: (wr.left, wr.top, wr.right, wr.bottom),
                    });
                }
            }
        }
        1
    }
}

/// Show a fullscreen overlay covering the virtual screen, displaying `bg` as
/// the backdrop. The user hovers over windows and clicks to select one.
pub fn pick(_bg: &Bitmap, virt_bounds: (i32, i32, i32, i32)) -> Result<Option<isize>> {
    unsafe {
        register_class();

        let screen_dc = GetDC(0);
        let mem_dc = CreateCompatibleDC(screen_dc);
        let bmp = create_dib_from_bitmap(_bg);
        let old = SelectObject(mem_dc, bmp as _);
        ReleaseDC(0, screen_dc);

        let cname = wide(CLASS_NAME);
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            cname.as_ptr(),
            wide("frame window pick").as_ptr(),
            WS_POPUP | WS_VISIBLE,
            virt_bounds.0,
            virt_bounds.1,
            virt_bounds.2,
            virt_bounds.3,
            0,
            0,
            GetModuleHandleW(core::ptr::null()) as _,
            core::ptr::null(),
        );

        let mut st = Box::new(State {
            desktop: mem_dc,
            desktop_bmp: bmp,
            desktop_old: old,
            width: virt_bounds.2,
            height: virt_bounds.3,
            origin_x: virt_bounds.0,
            origin_y: virt_bounds.1,
            windows: Vec::new(),
            hovered_hwnd: 0,
            hovered_rect: (0, 0, 0, 0),
            confirmed: None,
            cancelled: false,
            hwnd,
        });

        // Enumerate visible top-level windows, excluding our overlay.
        ENUM_RESULT.with(|r| *r.borrow_mut() = Some(&mut st.windows));
        EnumWindows(Some(enum_windows_proc), hwnd);
        ENUM_RESULT.with(|r| *r.borrow_mut() = None);

        STATE.with(|s| *s.borrow_mut() = Some(&mut *st as *mut State));

        ShowWindow(hwnd, SW_SHOW);
        SetCursor(LoadCursorW(0, IDC_CROSS));

        let mut msg: MSG = core::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
            if msg.message == WM_QUIT {
                break;
            }
        }

        STATE.with(|s| *s.borrow_mut() = None);

        SelectObject(mem_dc, st.desktop_old);
        DeleteObject(st.desktop_bmp as _);
        DeleteDC(mem_dc);

        if st.cancelled {
            return Ok(None);
        }
        Ok(st.confirmed)
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
            hInstance: GetModuleHandleW(core::ptr::null()) as _,
            hIcon: 0,
            hCursor: LoadCursorW(0, IDC_CROSS),
            hbrBackground: 0,
            lpszMenuName: core::ptr::null(),
            lpszClassName: cname.as_ptr(),
            hIconSm: 0,
        };
        RegisterClassExW(&wc);
    });
}

unsafe fn create_dib_from_bitmap(bg: &Bitmap) -> isize {
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS,
    };
    unsafe {
        let mut bi: BITMAPINFO = core::mem::zeroed();
        bi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = bg.width as i32;
        bi.bmiHeader.biHeight = -(bg.height as i32);
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB as u32;
        let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
        let hbmp = CreateDIBSection(0, &bi, DIB_RGB_COLORS, &mut bits, 0, 0);
        if !bits.is_null() {
            core::ptr::copy_nonoverlapping(bg.bgra.as_ptr(), bits as *mut u8, bg.bgra.len());
        }
        hbmp
    }
}

fn get_window_title(hwnd: isize) -> String {
    unsafe {
        let mut buf = [0u16; 256];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if n > 0 {
            String::from_utf16_lossy(&buf[..n as usize])
        } else {
            String::new()
        }
    }
}

fn hit_test_windows(windows: &[WinEntry], x: i32, y: i32) -> Option<(isize, (i32, i32, i32, i32))> {
    // Iterate in reverse (last in Vec is bottom-most in Z-order since EnumWindows
    // returns topmost first, but we push in that order, so reverse gives Z-order).
    for entry in windows.iter().rev() {
        let (l, t, r, b) = entry.rect;
        if x >= l && x < r && y >= t && y < b {
            return Some((entry.hwnd, entry.rect));
        }
    }
    None
}

unsafe extern "system" fn wndproc(hwnd: isize, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let p = STATE.with(|s| *s.borrow());
    let st = match p {
        Some(p) => unsafe { &mut *p },
        None => return unsafe { DefWindowProcW(hwnd, msg, w, l) },
    };
    unsafe {
        match msg {
            WM_MOUSEMOVE => {
                let x = (l & 0xFFFF) as i16 as i32;
                let y = ((l >> 16) & 0xFFFF) as i16 as i32;
                let screen_x = st.origin_x + x;
                let screen_y = st.origin_y + y;
                if let Some((candidate, rect)) = hit_test_windows(&st.windows, screen_x, screen_y) {
                    if candidate != st.hovered_hwnd {
                        st.hovered_hwnd = candidate;
                        st.hovered_rect = rect;
                        InvalidateRect(hwnd, core::ptr::null(), 0);
                    }
                } else if st.hovered_hwnd != 0 {
                    st.hovered_hwnd = 0;
                    InvalidateRect(hwnd, core::ptr::null(), 0);
                }
                0
            }
            WM_LBUTTONDOWN => {
                if st.hovered_hwnd != 0 {
                    st.confirmed = Some(st.hovered_hwnd);
                } else {
                    st.cancelled = true;
                }
                DestroyWindow(hwnd);
                0
            }
            WM_RBUTTONDOWN | WM_KEYDOWN => {
                if msg == WM_KEYDOWN && w as i32 != 27 {
                    return 0;
                }
                st.cancelled = true;
                DestroyWindow(hwnd);
                0
            }
            WM_PAINT => {
                paint(hwnd, st);
                0
            }
            windows_sys::Win32::UI::WindowsAndMessaging::WM_ERASEBKGND => 1,
            WM_DESTROY => {
                PostMessageW(0, WM_QUIT, 0, 0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

unsafe fn paint(hwnd: isize, st: &State) {
    unsafe {
        let mut ps: PAINTSTRUCT = core::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);

        use windows_sys::Win32::Graphics::Gdi::CreateCompatibleBitmap;
        let mem_dc = CreateCompatibleDC(hdc);
        let mem_bmp = CreateCompatibleBitmap(hdc, st.width, st.height);
        let old_bmp = SelectObject(mem_dc, mem_bmp as _);

        BitBlt(mem_dc, 0, 0, st.width, st.height, st.desktop, 0, 0, SRCCOPY);

        // Tint via AlphaBlend.
        let tint_dc = CreateCompatibleDC(mem_dc);
        let mut bi: windows_sys::Win32::Graphics::Gdi::BITMAPINFO = core::mem::zeroed();
        bi.bmiHeader.biSize = core::mem::size_of::<windows_sys::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = 1;
        bi.bmiHeader.biHeight = -1;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = windows_sys::Win32::Graphics::Gdi::BI_RGB as u32;
        let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
        let tint_bmp = windows_sys::Win32::Graphics::Gdi::CreateDIBSection(0, &bi, windows_sys::Win32::Graphics::Gdi::DIB_RGB_COLORS, &mut bits, 0, 0);
        if !bits.is_null() {
            *(bits as *mut u32) = 0x8C_00_00_00;
        }
        let old_tint = SelectObject(tint_dc, tint_bmp as _);
        let blend = BLENDFUNCTION {
            BlendOp: 0,
            BlendFlags: 0,
            SourceConstantAlpha: 140,
            AlphaFormat: 0,
        };
        AlphaBlend(mem_dc, 0, 0, st.width, st.height, tint_dc, 0, 0, 1, 1, blend);
        SelectObject(tint_dc, old_tint);
        DeleteObject(tint_bmp as _);
        DeleteDC(tint_dc);

        // Highlight hovered window.
        if st.hovered_hwnd != 0 {
            let (l, t, r, b) = st.hovered_rect;
            let x = l - st.origin_x;
            let y = t - st.origin_y;
            let w = r - l;
            let h = b - t;

            // Re-blit clear desktop under highlighted window.
            BitBlt(mem_dc, x, y, w, h, st.desktop, x, y, SRCCOPY);

            // Thick green outline.
            let pen = CreatePen(PS_SOLID as i32, 3, 0x00FF66);
            let old_pen = SelectObject(mem_dc, pen as _);
            let pts = [
                windows_sys::Win32::Foundation::POINT { x, y },
                windows_sys::Win32::Foundation::POINT { x: x + w, y },
                windows_sys::Win32::Foundation::POINT { x: x + w, y: y + h },
                windows_sys::Win32::Foundation::POINT { x, y: y + h },
                windows_sys::Win32::Foundation::POINT { x, y },
            ];
            Polyline(mem_dc, pts.as_ptr(), pts.len() as i32);
            SelectObject(mem_dc, old_pen);
            DeleteObject(pen as _);

            // Window title label.
            let title = get_window_title(st.hovered_hwnd);
            if !title.is_empty() {
                let label = wide(&title);
                let font = CreateFontW(
                    -14, 0, 0, 0,
                    FW_NORMAL as i32,
                    0, 0, 0,
                    DEFAULT_CHARSET as u32,
                    OUT_DEFAULT_PRECIS as u32,
                    CLIP_DEFAULT_PRECIS as u32,
                    CLEARTYPE_QUALITY as u32,
                    VARIABLE_PITCH as u32,
                    wide("Segoe UI").as_ptr(),
                );
                let old_font = SelectObject(mem_dc, font as _);
                SetBkMode(mem_dc, TRANSPARENT as i32);
                SetTextColor(mem_dc, 0x00FFFFFF);
                let label_y = if y + h + 4 < st.height { y + h + 4 } else { y - 18 };
                TextOutW(mem_dc, x + 4, label_y, label.as_ptr(), (label.len() - 1) as i32);
                SelectObject(mem_dc, old_font);
                DeleteObject(font as _);
            }
        }

        BitBlt(hdc, 0, 0, st.width, st.height, mem_dc, 0, 0, SRCCOPY);

        SelectObject(mem_dc, old_bmp);
        DeleteObject(mem_bmp as _);
        DeleteDC(mem_dc);
        EndPaint(hwnd, &ps);
    }
}

#[allow(dead_code)]
fn _silence_unused() {
    let _ = AC_SRC_OVER;
    let _: HBRUSH = 0;
    let _: HPEN = 0;
    let _ = (CreateSolidBrush, FillRect);
}
