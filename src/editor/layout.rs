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

//! Editor child-window layout. The parent owns three children: toolbar (top),
//! canvas (middle), property panel (right). Coordinates are in client space of
//! the parent window.

use windows_sys::Win32::Foundation::RECT;

pub const TOOLBAR_HEIGHT: i32 = 44;
pub const PROPERTY_WIDTH: i32 = 224;

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
