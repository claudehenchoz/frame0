use crate::error::Result;
use crate::gfx::bitmap::Bitmap;
use windows_sys::Win32::Foundation::HWND;

/// Pick a top-level window by clicking on it. v1 minimal implementation: just
/// take the foreground window; full hover-highlight UX deferred to v2.
pub fn pick(_bg: &Bitmap, _virt_bounds: (i32, i32, i32, i32)) -> Result<Option<HWND>> {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        Ok(Some(GetForegroundWindow()))
    }
}
