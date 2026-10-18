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

//! Capture settings model (PROMPT.md §2) plus the low-level grabbers.
//!
//! `CaptureSettings` is the single description of "what to capture and where
//! to send it". The tray / hotkey / presets / repeat-last all build one and
//! hand it to `App::run_capture`, which resolves interactive modes (overlay
//! picks) into a concrete rect or window and records that for Repeat last.

pub mod bitblt;
pub mod effects;
pub mod monitors;
pub mod print_window;
pub mod scrolling;
pub mod windows;

use windows_sys::Win32::Foundation::HWND;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// All monitors (the whole virtual screen).
    Fullscreen,
    /// One monitor, `CaptureSettings::monitor` indexes `monitors::list()`.
    Monitor,
    /// Interactive drag rectangle.
    Region,
    /// `fixed_size`, at `fixed_pos` if given, else placed interactively.
    FixedRegion,
    /// Interactive hover-pick of a top-level window.
    Window,
    /// A specific window (from the "Open window" list), via PrintWindow.
    WindowHandle,
    /// Scrolling capture of a picked region / window.
    Scrolling,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Fullscreen => "fullscreen",
            Mode::Monitor => "monitor",
            Mode::Region => "region",
            Mode::FixedRegion => "fixed-region",
            Mode::Window => "window",
            Mode::WindowHandle => "window-handle",
            Mode::Scrolling => "scrolling",
        }
    }
    pub fn parse(s: &str) -> Option<Mode> {
        Some(match s {
            "fullscreen" => Mode::Fullscreen,
            "monitor" => Mode::Monitor,
            "region" => Mode::Region,
            "fixed-region" => Mode::FixedRegion,
            "window" => Mode::Window,
            "window-handle" => Mode::WindowHandle,
            "scrolling" => Mode::Scrolling,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Editor,
    Clipboard,
    File,
}

impl Output {
    pub fn as_str(self) -> &'static str {
        match self {
            Output::Editor => "editor",
            Output::Clipboard => "clipboard",
            Output::File => "file",
        }
    }
    pub fn parse(s: &str) -> Output {
        match s {
            "clipboard" => Output::Clipboard,
            "file" => Output::File,
            _ => Output::Editor,
        }
    }
}

/// Effect bitflags.
pub const FX_BORDER: u32 = 1;
pub const FX_SHADOW: u32 = 2;

/// Screen-space rectangle: x, y, width, height.
pub type ScreenRect = (i32, i32, i32, i32);

#[derive(Clone)]
pub struct CaptureSettings {
    pub mode: Mode,
    pub monitor: u32,
    /// The resolved rectangle of the last execution (region / window /
    /// fixed placement). Repeat-last reuses it instead of asking again.
    pub region: Option<ScreenRect>,
    pub fixed_size: Option<(i32, i32)>,
    pub fixed_pos: Option<(i32, i32)>,
    /// Runtime-only: the window chosen for Window / WindowHandle modes.
    pub hwnd: HWND,
    pub delay: u32,
    pub interval: u32,
    pub include_cursor: bool,
    pub output: Output,
    pub effects: u32,
}

impl CaptureSettings {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            monitor: 0,
            region: None,
            fixed_size: None,
            fixed_pos: None,
            hwnd: 0,
            delay: 0,
            interval: 0,
            include_cursor: false,
            output: Output::Editor,
            effects: 0,
        }
    }

    /// Settings suitable for storing as a preset: an interactively-picked
    /// region becomes a fixed rectangle; window picks stay interactive
    /// (window handles don't survive restarts).
    pub fn to_preset(&self) -> Self {
        let mut p = self.clone();
        p.hwnd = 0;
        match self.mode {
            Mode::Region | Mode::FixedRegion => {
                if let Some((x, y, w, h)) = self.region {
                    p.mode = Mode::FixedRegion;
                    p.fixed_size = Some((w, h));
                    p.fixed_pos = Some((x, y));
                }
            }
            Mode::WindowHandle => p.mode = Mode::Window,
            _ => {}
        }
        if p.mode != Mode::FixedRegion {
            p.region = None;
        }
        p
    }
}
