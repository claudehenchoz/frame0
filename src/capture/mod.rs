pub mod bitblt;
pub mod delay;
pub mod monitors;
pub mod print_window;
pub mod windows;

use crate::error::Result;
use crate::gfx::bitmap::Bitmap;
use windows_sys::Win32::Foundation::HWND;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CaptureMode {
    Fullscreen,
    Region,
    FixedRegion { x: i32, y: i32, w: i32, h: i32 },
    Monitor(usize),
    Window(HWND),
    InteractiveWindow,
}

#[derive(Clone)]
pub struct CaptureRequest {
    pub mode: CaptureMode,
    pub delay_secs: Option<u32>,
    pub include_cursor: bool,
}

pub fn run(req: &CaptureRequest) -> Result<Bitmap> {
    match req.mode {
        CaptureMode::Fullscreen => bitblt::virtual_screen(req.include_cursor),
        CaptureMode::Monitor(i) => {
            let mons = monitors::list();
            let m = mons.get(i).ok_or(crate::error::Error::msg("no such monitor"))?;
            bitblt::region(m.x, m.y, m.w, m.h, req.include_cursor)
        }
        CaptureMode::FixedRegion { x, y, w, h } => bitblt::region(x, y, w, h, req.include_cursor),
        CaptureMode::Region => {
            // Show the selection overlay and let the user drag a rectangle.
            let bg = bitblt::virtual_screen(false)?;
            let bounds = monitors::virtual_screen_bounds();
            match crate::overlay::region::pick(&bg, bounds)? {
                Some(rect) => bitblt::region(rect.0, rect.1, rect.2, rect.3, req.include_cursor),
                None => Err(crate::error::Error::msg("capture cancelled")),
            }
        }
        CaptureMode::InteractiveWindow => {
            let bg = bitblt::virtual_screen(false)?;
            let bounds = monitors::virtual_screen_bounds();
            match crate::overlay::window_pick::pick(&bg, bounds)? {
                Some(hwnd) => print_window::capture(hwnd),
                None => Err(crate::error::Error::msg("capture cancelled")),
            }
        }
        CaptureMode::Window(hwnd) => print_window::capture(hwnd),
    }
}
