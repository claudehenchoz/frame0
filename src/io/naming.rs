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

//! Capture timestamps (UTC FILETIME ticks) and the file names built from
//! them: `frame0 - YYYY-MM-DD HH-MM-SS`.

use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows_sys::Win32::Storage::FileSystem::FileTimeToLocalFileTime;
use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows_sys::Win32::System::Time::FileTimeToSystemTime;

/// 100 ns ticks between 1601-01-01 (FILETIME) and 1970-01-01 (Unix).
const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;

fn split(t: u64) -> FILETIME {
    FILETIME { dwLowDateTime: t as u32, dwHighDateTime: (t >> 32) as u32 }
}

pub fn now() -> u64 {
    let mut ft = split(0);
    unsafe { GetSystemTimeAsFileTime(&mut ft) };
    (ft.dwHighDateTime as u64) << 32 | ft.dwLowDateTime as u64
}

/// Last-write time of `path`, or now if it can't be read.
pub fn file_time(path: &str) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| UNIX_EPOCH_TICKS + d.as_secs() * 10_000_000 + d.subsec_nanos() as u64 / 100)
        .unwrap_or_else(now)
}

/// `frame0 - 2026-10-19 07-15-14` (local time; no colons, Windows forbids them).
pub fn stem(captured: u64) -> String {
    let mut st: SYSTEMTIME = unsafe { core::mem::zeroed() };
    unsafe {
        let mut local = split(0);
        FileTimeToLocalFileTime(&split(captured), &mut local);
        FileTimeToSystemTime(&local, &mut st);
    }
    let mut s = String::from("frame0 - ");
    let mut num = |v: u16, width: usize, sep: char| {
        let d = v.to_string();
        for _ in d.len()..width {
            s.push('0');
        }
        s.push_str(&d);
        if sep != '\0' {
            s.push(sep);
        }
    };
    num(st.wYear, 4, '-');
    num(st.wMonth, 2, '-');
    num(st.wDay, 2, ' ');
    num(st.wHour, 2, '-');
    num(st.wMinute, 2, '-');
    num(st.wSecond, 2, '\0');
    s
}

/// `dir\name`, without doubling a trailing backslash.
pub fn join(dir: &str, name: &str) -> String {
    let mut p = String::from(dir);
    if !p.is_empty() && !p.ends_with('\\') && !p.ends_with('/') {
        p.push('\\');
    }
    p.push_str(name);
    p
}
