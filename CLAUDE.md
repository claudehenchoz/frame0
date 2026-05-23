# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`frame` is a single-binary Windows screenshot + annotation tool. The full product spec lives in `PROMPT.md`. v2 shipped: region capture, D2D-rendered editor with Arrow / Curve(Bezier) / Shape (rect+ellipse) / Text(in-place edit) / Step / Highlighter / Blur(real Gaussian) / Redact / Magnify / Stamp(4 bundled) / Selection(rect) / Move, undo/redo, PNG/JPG/clipboard export, `.frame` project save/load (v2 format), background pre-warm thread.

Still **deferred** from the spec — don't assume they exist: Callout (rounded body + tails), Cut-out (band removal from base), polygon/freehand selection shapes, selection operations (delete/fill/invert/content-aware-fill), scrolling capture, presets UI, DXGI Desktop Duplication, window-pick hover-highlight, dedicated property panel.

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

Current release: ~122 KB (v2). Plan ceiling was 1 MB; the original v1.5 target was <500 KB and v2 stays well under both. Avoid `format!`/`Debug` outside cold paths and avoid proc-macro deps (`windows-implement`, `serde_derive`, `thiserror`); the build flags above strip a lot but the wins only show when those traps stay out of the hot code. Measure after non-trivial additions: `ls target/x86_64-pc-windows-msvc/release/frame.exe`.

## Architecture

### Process model

Single binary, single instance, persistent tray. `main.rs` spawns `gfx::prewarm::spawn()` (background thread that LoadLibrarys d2d1/dwrite/d3d11/gdiplus and primes the DirectWrite font collection), then creates `App` (`app.rs`) which owns a hidden `HWND_MESSAGE` window. That window receives:
- `WM_HOTKEY` (id=1) → `App::capture_and_save` → opens editor
- `WM_TRAY` (custom, `WM_APP+1`) → double-click capture / right-click context menu
- `WM_COMMAND` → tray menu commands
- `TaskbarCreated` registered message → re-add tray icon after Explorer restart

Control flows: hotkey/tray → `bitblt::virtual_screen` (snapshot) → `overlay::region::pick` (fullscreen drag-rect window) → `bitblt::region` (final shot) → `EditorWindow::create` (or `replace_image` if editor open).

### Editor window topology

v2 split: the editor is a **parent window** (`EditorWindow`) with two child windows:
- `editor::toolbar` — child HWND, own window class, paints itself with GDI (double-buffered memory DC). Mouse clicks update the shared `EditorState` and invalidate the canvas.
- `editor::canvas` — child HWND, own window class, owns a DXGI flip-model swap chain bound to its HWND. WM_PAINT calls `Canvas::render()` which does D2D `BeginDraw` / `EndDraw` / `Present`. WM_ERASEBKGND returns 1.

Shared state (`EditorState` = `Canvas` + `History` + path + app weakref + child hwnds) lives in `thread_local! HashMap<HWND, Rc<RefCell<EditorState>>>` keyed by **every** editor HWND (parent + toolbar + canvas + edit-text sibling), so each child wndproc can look it up via `editor::window::get_state(GetParent(self))`. Don't add a child window without inserting into that map (and clearing on WM_DESTROY).

When the Text tool commits or the user double-clicks a Text annotation, `editor::text_edit::spawn` creates a Win32 `EDIT` control **as a sibling of the canvas under the editor parent** (not a child of the canvas — DXGI swap chains don't compose with overlapping child windows). It's sub-classed via `SetWindowLongPtrW(GWLP_WNDPROC, ...)`; `VK_RETURN` and `WM_KILLFOCUS` commit, `VK_ESCAPE` cancels.

### Crates: windows-sys + windows + windows-numerics

Three FFI deps; they serve different roles:

- **`windows-sys 0.52`** for flat Win32 (GDI, GDI+, tray, hotkey, clipboard, menus, message loop, EDIT subclass). Used by most files.
- **`windows 0.61`** for COM interfaces (Direct2D, DirectWrite, D3D11, DXGI). Used by `gfx/d2d.rs`, `gfx/dwrite.rs`, `gfx/prewarm.rs`, `editor/canvas.rs`.
- **`windows-numerics 0.2`** for `Matrix3x2` and `Vector2`. In windows 0.61, D2D point arguments are `Vector2 { X, Y }` (note capital fields), not the old `D2D_POINT_2F { x, y }`. Transforms use `Matrix3x2` with `M11/M12/M21/M22/M31/M32` (not `Dx/Dy`).

**Critical quirk:** in `windows-sys 0.52`, HWND/HMENU/HICON/HCURSOR/HBRUSH/HBITMAP/HDC/HMODULE are **`isize`**, not raw pointers. Null is `0`, not `core::ptr::null_mut()`. Checks are `h == 0` / `h != 0`, not `.is_null()`. Function args that take handles need `0` (or `0 as _`). This burns hours if you don't know it. The `windows` crate uses typed wrappers (e.g. `HWND(*mut c_void)`); a `From<windows::core::Error> for crate::error::Error` impl in `src/error.rs` lets `?` cross the boundary.

**Another quirk:** `D2D1_PRIMITIVE_BLEND_MULTIPLY` does not exist in the windows 0.61 enum (only SOURCE_OVER / COPY / MIN / MAX / ADD). True multiply-blend for the Highlighter is unavailable; the canvas uses a translucent alpha-blended fill instead, which looks close. Don't go hunting for a `MULTIPLY` constant — it's not there.

### Editor data flow

`editor/annotation.rs` defines `Annotation` (enum of Arrow/Shape/Text/Step/Highlighter/Blur/Magnify/Stamp), `Document` (base bitmap + `Vec<Annotation>` + `next_id` + `next_step_value`), `Point`, `Rect`, `ShapeKind`, `BlurMode`. Every annotation variant has an `id: AnnotationId` field — when pattern-matching, **always use `..`** to skip it (`Annotation::Arrow { start, end, .. }`); forgetting it is a common compile error. `Annotation::Arrow` has an optional `bezier: Option<Point>` (single quadratic control point).

`editor/history.rs` holds `History` with `undo`/`redo` stacks of `Command` (Insert/Remove/Mutate deltas — not full snapshots).

`editor/canvas.rs` owns the `Document`, the current `Tool`, viewport (scale/offset), and renders via **Direct2D** through `App.gfx.d2d.context`. It owns a `IDXGISwapChain1` per editor and caches an LRU of `ID2D1SolidColorBrush` (8 slots) + an `ID2D1Effect` for `CLSID_D2D1GaussianBlur` + lazy `ID2D1Bitmap1` per bundled stamp. On `D2DERR_RECREATE_TARGET` the canvas drops all device-bound resources and skips the frame; a perfect in-place recovery is a v2.1 task.

`editor/window.rs` is the **parent** wndproc — menu, child layout (`SetWindowPos` on toolbar + canvas in `WM_SIZE`), and File-menu commands. Toolbar / canvas wndprocs live in `editor/toolbar.rs` / `editor/canvas.rs` respectively.

`editor/text_edit.rs` hosts the Win32 `EDIT` control for in-place text editing. `spawn(parent, canvas, annotation_id, parent_rect, initial_text, font_size)` creates the EDIT as a sibling of the canvas; the subclassed wndproc commits on Enter / focus-loss (with a `finishing` flag to prevent re-entrant commits during destroy).

`editor/stamps.rs` generates the 4 bundled stamp bitmaps programmatically (vector geometry → BGRA buffer) — cheaper than embedding PNGs and decoding them. The canvas lazy-decodes via `gfx::d2d::upload_bitmap` on first draw, then caches.

`editor/flatten.rs` is a **CPU rasterizer** (`fn flatten(doc) -> Bitmap`) used for export and clipboard. It walks each annotation and writes pixels directly. Text annotations rasterize as placeholder rectangles, Bezier arrows export as straight, Magnify exports as outlines + connector only. These are export-side limitations; the on-screen D2D canvas renders correctly.

### Adding a new tool / annotation variant

1. Add variant to `Annotation` enum (`editor/annotation.rs`) with `id: AnnotationId` field. Update `id()`, `bounding_box()`, `translate()`.
2. Add to `Tool` enum (`editor/tools/mod.rs`) and add a `commit_drag` match arm that builds the annotation.
3. Add a `draw_<kind>` method in `editor/canvas.rs::draw_annotation` (D2D path) and call it from the match arm there. If the new variant uses a special drag preview, also extend `draw_preview`.
4. Add a render arm in `editor/flatten.rs::rasterize` (CPU path for export).
5. Add (de)serialization in `io/project.rs`: pick a new `TAG_*` byte, add to `write_ann`/`read_ann`. If the variant's data layout changes for an existing tag, bump `VERSION` and gate the new fields on `version >= N` in `read_ann`.
6. Add an entry to `TOOLS` in `editor/toolbar.rs` so it appears in the toolbar.

### Anti-flicker pattern (mandatory for new windows)

Both the editor and the region overlay handle **`WM_ERASEBKGND` → 1** and double-buffer through a memory DC: paint everything into `CreateCompatibleDC` + `CreateCompatibleBitmap`, then `BitBlt` to the window once. Painting directly to the window's HDC across multiple steps causes visible flicker on every `WM_MOUSEMOVE` invalidation. The editor's WM_PAINT additionally uses `SetViewportOrgEx` to shift the canvas's logical origin past the toolbar — `Canvas::render` paints at logical (0,0) and the offset is applied by the host.

### Persistence

- `frame.ini` in `%APPDATA%\frame\` via `GetPrivateProfileStringW`/`WritePrivateProfileStringW` (`src/config.rs`). Default hotkey `Ctrl+Win+PgDn` (`win/hotkey.rs::HotkeySpec::default_capture`).
- `.frame` project files: little-endian binary, `b"FRAM"` magic + `u16` version + length-prefixed PNG of base image + annotation records keyed by `TAG_*` byte. See `io/project.rs`.
- Image encode/decode goes through `gfx/gdiplus.rs` (flat `Gdip*` C API, not COM). WIC was attempted and dropped to keep the binary small.

### Where things are NOT yet wired (v2.1 work)

- No dedicated property panel — stroke width / font size / blur intensity have working defaults set at commit time but can't be changed from the UI (`ToolStyle::default` in `editor/tools/mod.rs` is the only way to influence them right now).
- Bezier arrow's control point isn't user-draggable yet — `commit_drag` sets it to a perpendicular midpoint with a sensible default bend.
- Magnify defaults to a 2× zoom from a centered sub-rect of the drag — no UI to adjust source bounds or factor.
- Stamp tool always places asset 0; no picker.
- Callout, Cut-out, polygon / freehand selection, scrolling capture, DXGI Desktop Duplication, presets UI — all still on the spec wish-list.
- `overlay::window_pick` is a stub that just returns the foreground window.
- CPU flatten doesn't match D2D canvas: exported PNG/JPG misses text glyph rendering (placeholder rect), Bezier curves (straight), and Magnify's scaled bitmap (outlines + connector only).

If you're touching the editor render loop, beware: the canvas uses `ID2D1DeviceContext` with `D2D1_BITMAP_OPTIONS_TARGET` on its swap chain's back buffer. Brushes are cached per-color on `Canvas.brushes` (8-slot LRU). Stamps cache per-asset. All of those must be cleared on device loss (`recover` path in `Canvas::render`).
