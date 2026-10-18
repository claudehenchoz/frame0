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

use core::fmt;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Clone, Copy)]
pub enum Error {
    Hr(i32),
    Win32(u32),
    Generic(&'static str),
}

impl Error {
    pub fn from_hr(hr: i32) -> Self {
        Error::Hr(hr)
    }
    pub fn last_win32() -> Self {
        unsafe { Error::Win32(windows_sys::Win32::Foundation::GetLastError()) }
    }
    pub fn msg(s: &'static str) -> Self {
        Error::Generic(s)
    }
}

impl From<windows::core::Error> for Error {
    fn from(e: windows::core::Error) -> Self {
        Error::Hr(e.code().0)
    }
}

pub fn hr_ok(hr: i32) -> Result<()> {
    if hr >= 0 {
        Ok(())
    } else {
        Err(Error::Hr(hr))
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Hr(h) => write!(f, "HRESULT 0x{:08X}", *h as u32),
            Error::Win32(e) => write!(f, "Win32 {}", e),
            Error::Generic(s) => f.write_str(s),
        }
    }
}
