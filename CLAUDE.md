# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`frame` is a single-binary Windows screenshot + annotation tool. The full product spec lives in `PROMPT.md`; the codebase implements an MVP of it (region capture + editor with Arrow/Shape/Text/Step/Highlighter/Blur/Selection/Move + undo/redo + PNG/JPG export + `.frame` project save/load). The spec lists many features (Callout, Magnify, Cut-out, Stamps, polygon/freehand selection, Bezier arrows, scrolling capture, content-aware fill, presets UI, DXGI Desktop Duplication) that are **deferred** — don't assume they exist.

## Build commands

```
cargo +nightly build --release    # produces target/x86_64-pc-windows-msvc/release/frame.exe
cargo +nightly check              # fast type-check
cargo +nightly run                # debug build + run
```

Nightly is mandatory (`rust-toolchain.toml` pins it). The build pulls in `-Z build-std` from `.cargo/config.toml` with `panic=immediate-abort`, `location-detail=none`, `fmt-debug=none`; stable can't satisfy those. `rust-src` component is required (`rustup component add rust-src --toolchain nightly`). Target is hardcoded to `x86_64-pc-windows-msvc` in `.cargo/config.toml`.

There are no tests. Adding one would need the test runner to play nicely with `build-std` and `panic=abort`.

### Running-binary gotcha

`frame.exe` is a persistent tray app — a launched instance keeps running until you right-click its tray icon and choose **Exit**. Building while it's running fails with `error: failed to remove file ... Access is denied`. Always exit the tray first before rebuilding.

### Binary-size discipline

Current release: ~90 KB. Plan target was <500 KB. Avoid `format!`/`Debug` outside cold paths and avoid proc-macro deps (`windows-implement`, `serde_derive`, `thiserror`); the build flags above strip a lot but the wins only show when those traps stay out of the hot code. Measure after non-trivial additions: `ls target/x86_64-pc-windows-msvc/release/frame.exe`.

## Architecture

### Process model

Single binary, single instance, persistent tray. `main.rs` creates `App` (`app.rs`) which owns a hidden `HWND_MESSAGE` window. That window receives:
- `WM_HOTKEY` (id=1) → `App::capture_and_save` → opens editor
- `WM_TRAY` (custom, `WM_APP+1`) → double-click capture / right-click context menu
- `WM_COMMAND` → tray menu commands
- `TaskbarCreated` registered message → re-add tray icon after Explorer restart

Control flows: hotkey/tray → `bitblt::virtual_screen` (snapshot) → `overlay::region::pick` (fullscreen drag-rect window) → `bitblt::region` (final shot) → `EditorWindow::create` (or `replace_image` if editor open).

### Crates: windows-sys + windows

Both are dependencies; they serve different roles:

- **`windows-sys 0.52`** for flat Win32 (GDI, GDI+, tray, hotkey, clipboard, menus, message loop). Used by most files.
- **`windows 0.61`** for COM interfaces (Direct2D, DirectWrite, D3D11, DXGI). Used by `gfx/d2d.rs` and `gfx/dwrite.rs`.

**Critical quirk:** in `windows-sys 0.52`, HWND/HMENU/HICON/HCURSOR/HBRUSH/HBITMAP/HDC/HMODULE are **`isize`**, not raw pointers. Null is `0`, not `core::ptr::null_mut()`. Checks are `h == 0` / `h != 0`, not `.is_null()`. Function args that take handles need `0` (or `0 as _`). This burns hours if you don't know it. The `windows` crate uses typed wrappers (e.g. `HWND(*mut c_void)`); a `From<windows::core::Error> for crate::error::Error` impl in `src/error.rs` lets `?` cross the boundary.

### Editor data flow

`editor/annotation.rs` defines `Annotation` (enum of Arrow/Shape/Text/Step/Highlighter/Blur), `Document` (base bitmap + `Vec<Annotation>` + `next_id` + `next_step_value`), `Point`, `Rect`, `ShapeKind`, `BlurMode`. Every annotation variant has an `id: AnnotationId` field — when pattern-matching, **always use `..`** to skip it (`Annotation::Arrow { start, end, .. }`); forgetting it is a common compile error.

`editor/history.rs` holds `History` with `undo`/`redo` stacks of `Command` (Insert/Remove/Mutate deltas — not full snapshots).

`editor/canvas.rs` owns the `Document`, the current `Tool`, viewport (scale/offset), and renders via **GDI** (`StretchBlt` for the base, `Polygon`/`Rectangle`/`Ellipse`/`TextOutW` for annotations). The D2D infrastructure exists (`App.gfx.d2d`/`dwrite` are real, fully constructed) but the canvas does not use it — a v2 task is to swap `draw_*` to use `self.gfx.d2d.context` for anti-aliased rendering and the gaussian blur effect.

`editor/window.rs` is the editor's WndProc — toolbar paint, mouse dispatch (`commit_drag` from `editor/tools/mod.rs` builds annotations), keyboard (Ctrl+Z/Y, Delete, Esc), and the File menu (Save Project, Open Project, Export PNG, Export JPG, Copy, Close).

`editor/flatten.rs` is a **CPU rasterizer** (`fn flatten(doc) -> Bitmap`) used for export and clipboard. It walks each annotation and writes pixels directly. Text annotations rasterize as placeholder rectangles — text content survives `.frame` save/load but doesn't appear in exported PNG/JPG yet.

### Adding a new tool

1. Add variant to `Annotation` enum (`editor/annotation.rs`) with `id: AnnotationId` field. Update `id()`, `bounding_box()`, `translate()`.
2. Add to `Tool` enum (`editor/tools/mod.rs`) and add a `commit_drag` match arm that builds the annotation.
3. Add a `draw_<kind>` method in `editor/canvas.rs::draw_annotation` and call it from the match arm there.
4. Add a render arm in `editor/flatten.rs::rasterize` (CPU path for export).
5. Add (de)serialization in `io/project.rs`: pick a new `TAG_*` byte, add to `write_ann`/`read_ann`.
6. Add an entry to `TOOLS` in `editor/toolbar.rs` so it appears in the toolbar.

### Anti-flicker pattern (mandatory for new windows)

Both the editor and the region overlay handle **`WM_ERASEBKGND` → 1** and double-buffer through a memory DC: paint everything into `CreateCompatibleDC` + `CreateCompatibleBitmap`, then `BitBlt` to the window once. Painting directly to the window's HDC across multiple steps causes visible flicker on every `WM_MOUSEMOVE` invalidation. The editor's WM_PAINT additionally uses `SetViewportOrgEx` to shift the canvas's logical origin past the toolbar — `Canvas::render` paints at logical (0,0) and the offset is applied by the host.

### Persistence

- `frame.ini` in `%APPDATA%\frame\` via `GetPrivateProfileStringW`/`WritePrivateProfileStringW` (`src/config.rs`). Default hotkey `Ctrl+Win+PgDn` (`win/hotkey.rs::HotkeySpec::default_capture`).
- `.frame` project files: little-endian binary, `b"FRAM"` magic + `u16` version + length-prefixed PNG of base image + annotation records keyed by `TAG_*` byte. See `io/project.rs`.
- Image encode/decode goes through `gfx/gdiplus.rs` (flat `Gdip*` C API, not COM). WIC was attempted and dropped to keep the binary small.

### Where things are NOT yet wired

- D2D / DirectWrite factories construct successfully but the canvas doesn't render through them.
- `gfx::Gfx::create()` runs synchronously inside `App::new()` — there's no pre-warm thread yet (adds ~100–200 ms to cold start).
- No in-place text editing for Text annotations; Text content can only enter through `.frame` load.
- No DXGI Desktop Duplication (BitBlt only).
- `overlay::window_pick` is a stub that just returns the foreground window.

If you're adding D2D rendering, the canvas must keep working under GDI as a fallback path during the migration; don't tear out GDI draw routines until D2D is verified pixel-equivalent.
