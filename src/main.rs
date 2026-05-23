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
