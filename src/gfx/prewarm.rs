//! Background-thread pre-warm for the COM stacks the editor uses.
//!
//! When the user presses the global hotkey for the first time, the path is:
//! 1. `BitBlt` desktop  → 2. overlay → 3. `EditorWindow::create` → 4.
//! `Canvas::attach` → 5. swap-chain create → 6. D2D bitmap upload.
//!
//! Cold-start that path is dominated by **DLL loads** (d2d1, dwrite, d3d11,
//! gdiplus — ~40 ms together) and **`GetSystemFontCollection`** the first
//! time DirectWrite enumerates installed fonts (~50–150 ms). We spawn a
//! detached thread at `main` entry that touches all of these. By the time
//! the user's first hotkey fires, the DLLs are mapped and DWrite's font
//! cache is warm; `App::new`'s synchronous `Gfx::create()` reuses both.
//!
//! Note: we deliberately do **not** share factories across threads here.
//! `Gfx::create()` still constructs its own on the UI thread; the warmth
//! lives in the DLL image cache and DWrite's process-wide font cache.

use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection,
};
use windows::core::Interface;
use windows_sys::Win32::System::LibraryLoader::LoadLibraryW;

pub fn spawn() {
    std::thread::spawn(|| unsafe {
        warm();
    });
}

unsafe fn warm() {
    unsafe {
        // Force-load the DLLs the editor will need. These are delay-loaded by
        // the windows / windows-sys link stubs, so without this step the
        // first call to any D2D / DWrite / D3D11 function on the UI thread
        // pays the load cost synchronously.
        for dll in ["d2d1.dll", "dwrite.dll", "d3d11.dll", "gdiplus.dll"] {
            let mut w: Vec<u16> = dll.encode_utf16().collect();
            w.push(0);
            let _ = LoadLibraryW(w.as_ptr());
        }

        // Prime DirectWrite's font collection. `GetSystemFontCollection(false)`
        // scans the system font directories on first call and caches the
        // result process-wide; the second call (from App::new's DWrite
        // factory) is cheap.
        if let Ok(factory) = DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) {
            let mut collection: Option<IDWriteFontCollection> = None;
            let _ = factory.GetSystemFontCollection(&mut collection, false);
            drop(factory);
        }
    }
}
