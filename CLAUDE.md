# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`frame` is a single-binary Windows screenshot + annotation tool. The full product spec lives in `PROMPT.md`, and v3 implements all of it:
- **Capture:** region, full screen / per monitor, fixed region, window pick (hover highlight), open-window list (PrintWindow, also for covered or minimized windows), scrolling capture, delayed and interval capture, repeat last, clipboard and file input, presets, effects (border, shadow), and output to the editor, the clipboard or a file.
- **Editor tools:** Move, Selection (rect/ellipse/polygon/freehand with delete, auto-fill, crop, invert, effects, copy and lift-to-layer), Arrow (end styles, curved with 2 bezier handles, shadow), Rectangle, Ellipse, Polygon, Callout (tails, corner radius), Text, Step (1,2,3 or A,B,C, editable labels), Highlighter, Blur/Pixelate/Solid redaction, Stamp (emoji, labels, user PNGs; categories and search), Magnify and Cut-out.
- **Object operations:** move, resize and rotate handles, multi-select and marquee, group/ungroup, copy/paste/duplicate, z-order, flatten, undo/redo.
- **Output:** PNG/JPG/clipboard export through the same D2D renderer as the canvas, and `.frame` projects (v3).

Not done: DXGI Desktop Duplication (capture uses BitBlt/PrintWindow).

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

Current release: ~308 KB (v3). Plan ceiling was 1 MB; the original v1.5 target was <500 KB and v3 stays under both. Avoid `format!`/`Debug` outside cold paths and avoid proc-macro deps (`windows-implement`, `serde_derive`, `thiserror`); the build flags above strip a lot but the wins only show when those traps stay out of the hot code. Measure after non-trivial additions: `ls target/x86_64-pc-windows-msvc/release/frame.exe`.

## Architecture

### Process model

Single binary, single instance, persistent tray. `main.rs` spawns `gfx::prewarm::spawn()` (background thread that LoadLibrarys d2d1/dwrite/d3d11/gdiplus and primes the DirectWrite font collection), then creates `App` (`app.rs`) which owns a hidden `HWND_MESSAGE` window. That window receives:
- `WM_HOTKEY` (id=1) → `App::run_capture` (region) → editor / clipboard / file
- `WM_TRAY` (custom, `WM_APP+1`) → double-click capture / right-click context menu
- `WM_COMMAND` → tray menu commands
- `TaskbarCreated` registered message → re-add tray icon after Explorer restart

Control flows: hotkey/tray command → `App::run_capture(CaptureSettings)` (`app.rs`; optional delay timer → resolve target via `overlay::region` / `overlay::window_pick` / `capture::*` → effects → output routing) → `App::open_editor` / `open_document`. `frame.exe --cmd <id> [--pid <pid>]` posts a tray command to a running instance (handy for testing). `FRAME_INI` overrides the ini path.

### Editor window topology

The editor is a **parent window** (`EditorWindow`) with three child windows (toolbar, canvas, property panel):
- `editor::toolbar` — child HWND, own window class, paints itself with GDI (double-buffered memory DC). Mouse clicks update the shared `EditorState` and invalidate the canvas.
- `editor::canvas` — child HWND (`WS_CLIPSIBLINGS` so the text EDIT can sit above it), owns a DXGI flip-model swap chain and the whole interaction state machine (`Gesture`): selection, handles, drawing tools, pixel selection, cut-out, zoom/pan. Keyboard accelerators are mapped in `window::accelerator` and posted to the parent as WM_COMMAND.
- `editor::property` — child HWND, GDI panel built from a declarative `Row` list (steppers, choices, toggles, buttons, notes, stamp grid). Edits change the quick style (`ToolStyle`) **and** the selection, so later annotations inherit the current properties.

Shared state (`EditorState` = `Canvas` (which owns the `History`) + path + app weakref + child hwnds) lives in `thread_local! HashMap<HWND, Rc<RefCell<EditorState>>>` keyed by **every** editor HWND (parent + toolbar + canvas + edit-text sibling), so each child wndproc can look it up via `editor::window::get_state(GetParent(self))`. Don't add a child window without inserting into that map (and clearing on WM_DESTROY).

When the Text/Callout tool commits or the user double-clicks a Text/Callout/Step, `editor::text_edit::spawn_for` creates a Win32 `EDIT` control **as a sibling of the canvas under the editor parent** (not a child of the canvas — DXGI swap chains don't compose with overlapping child windows). It's sub-classed via `SetWindowLongPtrW(GWLP_WNDPROC, ...)`; `VK_RETURN` and `WM_KILLFOCUS` commit, `VK_ESCAPE` cancels.

### Crates: windows-sys + windows + windows-numerics

Three FFI deps; they serve different roles:

- **`windows-sys 0.52`** for flat Win32 (GDI, GDI+, tray, hotkey, clipboard, menus, message loop, EDIT subclass). Used by most files.
- **`windows 0.61`** for COM interfaces (Direct2D, DirectWrite, D3D11, DXGI). Used by `gfx/d2d.rs`, `gfx/dwrite.rs`, `gfx/prewarm.rs`, `editor/canvas.rs`.
- **`windows-numerics 0.2`** for `Matrix3x2` and `Vector2`. In windows 0.61, D2D point arguments are `Vector2 { X, Y }` (note capital fields), not the old `D2D_POINT_2F { x, y }`. Transforms use `Matrix3x2` with `M11/M12/M21/M22/M31/M32` (not `Dx/Dy`).

**Critical quirk:** in `windows-sys 0.52`, HWND/HMENU/HICON/HCURSOR/HBRUSH/HBITMAP/HDC/HMODULE are **`isize`**, not raw pointers. Null is `0`, not `core::ptr::null_mut()`. Checks are `h == 0` / `h != 0`, not `.is_null()`. Function args that take handles need `0` (or `0 as _`). This burns hours if you don't know it. The `windows` crate uses typed wrappers (e.g. `HWND(*mut c_void)`); a `From<windows::core::Error> for crate::error::Error` impl in `src/error.rs` lets `?` cross the boundary.

**Another quirk:** `D2D1_PRIMITIVE_BLEND_MULTIPLY` does not exist in the windows 0.61 enum (only SOURCE_OVER / COPY / MIN / MAX / ADD). True multiply-blend for the Highlighter is unavailable; the canvas uses a translucent alpha-blended fill instead, which looks close. Don't go hunting for a `MULTIPLY` constant — it's not there.

### Editor data flow

`editor/annotation.rs` defines `Annotation` (Arrow/Shape/Callout/Text/Step/Highlighter/Blur/Magnify/Stamp/Image), `Document` (base `Rc<Bitmap>` + `base_gen` + `Vec<Annotation>` + `groups` map), `Point`, `Rect` and the geometry helpers. These are `frame()` (the unrotated handle frame), `bounding_box()`, `hit_test()`, `translate[_part]()`, `set_frame()` (resize), `rotation()`, and `edit_points()` / `set_edit_point()` for kind-specific handles. Every variant has an `id` field, so **always use `..`** when pattern-matching.

`editor/history.rs`: `Command` = Insert / Remove / Mutate / Base (swap the base raster) / Groups / Batch. `history::execute` applies and records a command; drag gestures mutate live, then push a Batch of Mutates on mouse-up.

`editor/render.rs`: the `Renderer` (brush LRU, Gaussian/Shadow effects, uploaded bitmaps, pixelate cache). The canvas draws through `draw_document`, and `render::flatten` runs the same code into an offscreen target with tiled CPU readback, so **export is pixel-identical to the canvas**. There is no separate CPU rasterizer any more. Shadows are drawn by recording the annotation into a command list and feeding it to the Shadow effect. Stamps are drawn as colour emoji (DirectWrite `ENABLE_COLOR_FONT`), vector label badges, or user PNGs (`editor/stamps.rs`).

`editor/pixels.rs`: CPU raster ops for the Selection and Cut-out tools (polygon masks, transparent/auto-fill, effects, extract/crop, `cut_band` with straight/zigzag/wave seams). All functions are pure, so callers wrap them in `Command::Base`.

`editor/text_edit.rs` hosts the in-place Win32 `EDIT` for Text, Callout and Step labels (`spawn_for(parent, canvas, TextEditReq)`). It must read the text **before** removing its subclass context, because WM_GETTEXT goes through the subclass.

### Adding a new tool / annotation variant

1. Add the variant to `Annotation` and handle it in the geometry helpers in `annotation.rs`.
2. Add it to `Tool` (+ `Tool::ALL`, `name`, `key`), creation logic in `tools::create_from_drag`, and a glyph in `toolbar::glyph`.
3. Draw it in `render::Renderer::draw_kind` (canvas and export get it automatically).
4. (De)serialize it in `io/project.rs`: new `TAG_*`; bump `VERSION` and gate new fields on `version >= N` if an existing layout changes.
5. Add property rows in `editor/property.rs::rows` / `apply`.

### Anti-flicker pattern (mandatory for new windows)

Both the editor and the region overlay handle **`WM_ERASEBKGND` → 1** and double-buffer through a memory DC: paint everything into `CreateCompatibleDC` + `CreateCompatibleBitmap`, then `BitBlt` to the window once. Painting directly to the window's HDC across multiple steps causes visible flicker on every `WM_MOUSEMOVE` invalidation. The editor's WM_PAINT additionally uses `SetViewportOrgEx` to shift the canvas's logical origin past the toolbar — `Canvas::render` paints at logical (0,0) and the offset is applied by the host.

### Persistence

- `frame.ini` in `%APPDATA%\frame\` via `GetPrivateProfileStringW`/`WritePrivateProfileStringW` (`src/config.rs`). Default hotkey `Ctrl+Win+PgDn` (`win/hotkey.rs::HotkeySpec::default_capture`).
- `.frame` project files: little-endian binary, `b"FRAM"` magic + `u16` version + length-prefixed PNG of base image + annotation records keyed by `TAG_*` byte. See `io/project.rs`.
- Image encode/decode goes through `gfx/gdiplus.rs` (flat `Gdip*` C API, not COM). WIC was attempted and dropped to keep the binary small.

### Known gaps

- Scrolling-capture stitching keeps each frame's scrollbar column, which leaves small artefacts.
- The prompt dialogs use classic (non-themed) Win32 controls: there's no comctl v6 manifest.
- Auto-fill is interpolation-based, not texture synthesis.

If you're touching the editor render loop, beware: the canvas uses `ID2D1DeviceContext` with `D2D1_BITMAP_OPTIONS_TARGET` on its swap chain's back buffer, and the device context is shared process-wide. Device-bound resources live in `Renderer` (plus the canvas checker brush / dash style) and are dropped on `D2DERR_RECREATE_TARGET` in `Canvas::render`, which then re-attaches.
