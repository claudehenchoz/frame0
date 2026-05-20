use crate::error::Result;
use crate::gfx::bitmap::Bitmap;
use crate::win::wide;
use std::cell::RefCell;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AlphaBlend, BLENDFUNCTION, BitBlt, CreateCompatibleDC, CreatePen, DeleteDC, DeleteObject,
    HBITMAP, HBRUSH, HDC, HPEN, InvalidateRect, PS_SOLID, PatBlt, Polyline, PAINTSTRUCT, BeginPaint,
    EndPaint, GetDC, ReleaseDC, SRCCOPY, SelectObject, CreateSolidBrush, FillRect, AC_SRC_OVER,
    SetBkMode, TRANSPARENT, SetTextColor, TextOutW, CreateFontW, FW_NORMAL, DEFAULT_CHARSET,
    OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, VARIABLE_PITCH,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    HCURSOR, IDC_CROSS, LoadCursorW, MSG, PostMessageW, RegisterClassExW,
    SetCursor, ShowWindow, SW_SHOW, TranslateMessage, WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_QUIT, WM_RBUTTONDOWN, WNDCLASSEXW, WS_EX_TOPMOST,
    WS_POPUP, WS_VISIBLE,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};

const CLASS_NAME: &str = "frame.overlay.region";

struct State {
    desktop: HDC, // memory DC holding the captured desktop
    desktop_bmp: HBITMAP,
    desktop_old: isize,
    width: i32,
    height: i32,
    origin_x: i32,
    origin_y: i32,
    dragging: bool,
    start: (i32, i32),
    end: (i32, i32),
    confirmed_rect: Option<(i32, i32, i32, i32)>,
    cancelled: bool,
    hwnd: HWND,
}

thread_local! {
    static STATE: RefCell<Option<*mut State>> = const { RefCell::new(None) };
}

/// Show a fullscreen overlay covering the virtual screen, displaying `bg` as the backdrop;
/// return the user's selected rectangle in screen coordinates, or None if cancelled.
pub fn pick(bg: &Bitmap, virt_bounds: (i32, i32, i32, i32)) -> Result<Option<(i32, i32, i32, i32)>> {
    unsafe {
        register_class();

        // Build a memory DC pre-loaded with the desktop bitmap.
        let screen_dc = GetDC(0);
        let mem_dc = CreateCompatibleDC(screen_dc);
        let bmp = create_dib_from_bitmap(bg);
        let old = SelectObject(mem_dc, bmp as _);
        ReleaseDC(0, screen_dc);

        let cname = wide(CLASS_NAME);
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            cname.as_ptr(),
            wide("frame overlay").as_ptr(),
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
            dragging: false,
            start: (0, 0),
            end: (0, 0),
            confirmed_rect: None,
            cancelled: false,
            hwnd,
        });

        STATE.with(|s| *s.borrow_mut() = Some(&mut *st as *mut State));

        ShowWindow(hwnd, SW_SHOW);
        SetCursor(LoadCursorW(0, IDC_CROSS));

        // Modal message loop until window destroyed.
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
        Ok(st.confirmed_rect)
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

unsafe fn create_dib_from_bitmap(bg: &Bitmap) -> HBITMAP {
    use windows_sys::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS};
    unsafe {
        let mut bi: BITMAPINFO = core::mem::zeroed();
        bi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = bg.width as i32;
        bi.bmiHeader.biHeight = -(bg.height as i32);
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB as u32;
        let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
        let hbmp = CreateDIBSection(
            0,
            &bi,
            DIB_RGB_COLORS,
            &mut bits,
            0,
            0,
        );
        if !bits.is_null() {
            core::ptr::copy_nonoverlapping(bg.bgra.as_ptr(), bits as *mut u8, bg.bgra.len());
        }
        hbmp
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let p = STATE.with(|s| *s.borrow());
    let st = match p {
        Some(p) => unsafe { &mut *p },
        None => return unsafe { DefWindowProcW(hwnd, msg, w, l) },
    };
    unsafe {
        match msg {
            WM_LBUTTONDOWN => {
                let x = (l & 0xFFFF) as i16 as i32;
                let y = ((l >> 16) & 0xFFFF) as i16 as i32;
                st.dragging = true;
                st.start = (x, y);
                st.end = (x, y);
                SetCapture(hwnd);
                InvalidateRect(hwnd, core::ptr::null(), 0);
                0
            }
            WM_MOUSEMOVE => {
                if st.dragging {
                    let x = (l & 0xFFFF) as i16 as i32;
                    let y = ((l >> 16) & 0xFFFF) as i16 as i32;
                    st.end = (x, y);
                    InvalidateRect(hwnd, core::ptr::null(), 0);
                }
                0
            }
            WM_LBUTTONUP => {
                if st.dragging {
                    st.dragging = false;
                    ReleaseCapture();
                    let (x0, y0) = st.start;
                    let (x1, y1) = st.end;
                    let x = x0.min(x1);
                    let y = y0.min(y1);
                    let w = (x1 - x0).abs();
                    let h = (y1 - y0).abs();
                    if w >= 2 && h >= 2 {
                        st.confirmed_rect = Some((
                            st.origin_x + x,
                            st.origin_y + y,
                            w,
                            h,
                        ));
                    } else {
                        st.cancelled = true;
                    }
                    DestroyWindow(hwnd);
                }
                0
            }
            WM_RBUTTONDOWN | WM_KEYDOWN => {
                if msg == WM_KEYDOWN && w as i32 != 27 /* VK_ESCAPE */ {
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
            WM_DESTROY => {
                PostMessageW(0, WM_QUIT, 0, 0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

unsafe fn paint(hwnd: HWND, st: &State) {
    unsafe {
        let mut ps: PAINTSTRUCT = core::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut client: RECT = core::mem::zeroed();
        GetClientRect(hwnd, &mut client);

        // Blit desktop background.
        BitBlt(hdc, 0, 0, st.width, st.height, st.desktop, 0, 0, SRCCOPY);

        // Tint everything outside the selection with semi-transparent dark overlay.
        let tint_dc = CreateCompatibleDC(hdc);
        let one_w = 1;
        let one_h = 1;
        use windows_sys::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS};
        let mut bi: BITMAPINFO = core::mem::zeroed();
        bi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = one_w;
        bi.bmiHeader.biHeight = -one_h;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB as u32;
        let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
        let tint_bmp = CreateDIBSection(
            0,
            &bi,
            DIB_RGB_COLORS,
            &mut bits,
            0,
            0,
        );
        if !bits.is_null() {
            // BGRA: dark with alpha 140
            *(bits as *mut u32) = 0x8C_00_00_00;
        }
        let old_tint = SelectObject(tint_dc, tint_bmp as _);

        let blend = BLENDFUNCTION {
            BlendOp: 0,        // AC_SRC_OVER
            BlendFlags: 0,
            SourceConstantAlpha: 140,
            AlphaFormat: 0,
        };
        AlphaBlend(hdc, 0, 0, st.width, st.height, tint_dc, 0, 0, 1, 1, blend);

        SelectObject(tint_dc, old_tint);
        DeleteObject(tint_bmp as _);
        DeleteDC(tint_dc);

        if st.dragging || st.start != st.end {
            let (x0, y0) = st.start;
            let (x1, y1) = st.end;
            let x = x0.min(x1);
            let y = y0.min(y1);
            let w = (x1 - x0).abs();
            let h = (y1 - y0).abs();
            if w > 0 && h > 0 {
                // Re-blit the unobscured selection (no tint).
                BitBlt(hdc, x, y, w, h, st.desktop, x, y, SRCCOPY);
                // Draw the rectangle outline.
                let pen = CreatePen(PS_SOLID as i32, 2, 0x00FF66 /* greenish */);
                let old_pen = SelectObject(hdc, pen as _);
                let pts = [
                    windows_sys::Win32::Foundation::POINT { x, y },
                    windows_sys::Win32::Foundation::POINT { x: x + w, y },
                    windows_sys::Win32::Foundation::POINT { x: x + w, y: y + h },
                    windows_sys::Win32::Foundation::POINT { x, y: y + h },
                    windows_sys::Win32::Foundation::POINT { x, y },
                ];
                Polyline(hdc, pts.as_ptr(), pts.len() as i32);
                SelectObject(hdc, old_pen);
                DeleteObject(pen as _);

                // Dimensions label.
                let dim_text = format!("{}×{}", w, h);
                let label = wide(&dim_text);
                let font = CreateFontW(
                    -14,
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
                    wide("Segoe UI").as_ptr(),
                );
                let old_font = SelectObject(hdc, font as _);
                SetBkMode(hdc, TRANSPARENT as i32);
                SetTextColor(hdc, 0x00FFFFFF);
                let label_x = (x + 4).max(0);
                let label_y = if y + h + 4 < st.height { y + h + 4 } else { y - 18 };
                TextOutW(hdc, label_x, label_y, label.as_ptr(), (label.len() - 1) as i32);
                SelectObject(hdc, old_font);
                DeleteObject(font as _);
            }
        }

        EndPaint(hwnd, &ps);
    }
}

#[allow(dead_code)]
fn _silence_unused() {
    let _ = AC_SRC_OVER;
    let _: HBRUSH = 0;
    let _: HPEN = 0;
    let _ = (CreateSolidBrush, FillRect, PatBlt);
}
