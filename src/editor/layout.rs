//! Editor child-window layout. The parent owns three children: toolbar (top),
//! canvas (middle), property panel (right). Coordinates are in client space of
//! the parent window.

use windows_sys::Win32::Foundation::RECT;

pub const TOOLBAR_HEIGHT: i32 = 44;
/// Property panel is added in Phase 5; for now reserve 0 width.
pub const PROPERTY_WIDTH: i32 = 0;

pub fn toolbar_rect(client: &RECT) -> RECT {
    RECT {
        left: 0,
        top: 0,
        right: client.right,
        bottom: TOOLBAR_HEIGHT,
    }
}

pub fn canvas_rect(client: &RECT) -> RECT {
    RECT {
        left: 0,
        top: TOOLBAR_HEIGHT,
        right: (client.right - PROPERTY_WIDTH).max(0),
        bottom: client.bottom.max(TOOLBAR_HEIGHT),
    }
}

pub fn property_rect(client: &RECT) -> RECT {
    RECT {
        left: (client.right - PROPERTY_WIDTH).max(0),
        top: TOOLBAR_HEIGHT,
        right: client.right,
        bottom: client.bottom.max(TOOLBAR_HEIGHT),
    }
}
