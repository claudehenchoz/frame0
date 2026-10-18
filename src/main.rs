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

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(non_snake_case, dead_code, unused_imports, unused_variables, unused_mut)]

mod app;
mod capture;
mod config;
mod editor;
mod error;
mod gfx;
mod io;
mod overlay;
mod win;

use error::Result;

fn main() -> Result<()> {
    if app::forward_command() {
        return Ok(());
    }
    // Kick off the COM/DLL warm-up before any synchronous setup. The bg thread
    // races App::new — every ms it gets before App's synchronous Gfx::create
    // shaves cold-start hotkey latency.
    gfx::prewarm::spawn();

    win::com::init_apartment()?;
    win::dpi::enable_per_monitor_v2();

    let app = app::App::new()?;
    app.run()?;
    Ok(())
}
