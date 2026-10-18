# AGENTS.md — AI Coding Agent Guide

The single reference for AI agents working on **Frame Zero** (package / binary name `frame0`): what it is, how to build and test it, how the code is laid out, and the gotchas that cost time. The product spec is `PROMPT.md`.

---

## What this is

Frame Zero (`frame0`) is a single-binary Windows screenshot + annotation tool: a persistent tray app with a Direct2D editor, written in Rust (nightly), windows-only. v3 implements everything in `PROMPT.md`:

- **Capture.**
  - Modes: region (loupe, keyboard nudge), full screen / per monitor, fixed region (typed size, optional position), window pick (hover highlight), open-window list (PrintWindow, so covered and minimized windows work), scrolling capture with stitching.
  - Timing: delayed capture, interval capture, repeat last.
  - Input from the clipboard or a file.
  - Presets, include-cursor, effects (border, drop shadow).
  - Output to the editor, the clipboard or `Pictures\frame0-*.png`.
- **Editor tools.**
  - Move.
  - Selection: rect / ellipse / polygon / freehand, with delete (transparent or auto-fill), crop, invert, effects, copy, and lift-to-layer.
  - Arrow: end styles, cubic bezier with 2 handles, shadow, Shift = H/V.
  - Rectangle, Ellipse, Polygon.
  - Callout: multiple tails, corner radius.
  - Text.
  - Step: 1,2,3 or A,B,C; editable labels.
  - Highlighter.
  - Blur / Pixelate / Solid redaction.
  - Stamp: emoji, label badges, user PNGs; categories and search.
  - Magnify: editable source rect and zoom factor.
  - Cut-out: rows or columns; straight / zigzag / wave seam.
- **Object operations.** Select, multi-select (Shift-click, marquee), move / resize / rotate handles, delete, copy/paste/duplicate, group/ungroup, z-order, flatten, undo/redo, zoom/pan.
- **Output.** PNG/JPG/clipboard export goes through the *same* D2D renderer as the canvas (pixel-identical). Blur and pixelate are warned as reversible on export, with an option to convert them to solid. `.frame0` projects keep every layer editable.

**Not done:** DXGI Desktop Duplication (capture uses BitBlt / PrintWindow).

### Known gaps

- Scrolling-capture stitching keeps each frame's scrollbar column, which leaves small artefacts.
- Auto-fill is interpolation-based (inverse-distance from the four directions + blur), not texture synthesis.
- GDI+ PNG encode/decode goes through a temp file (`gfx/gdiplus.rs`); an in-memory IStream path was never added.
- A new capture *replaces* the open editor's document (after a save/discard prompt if dirty); there is only one editor window.

---

## Build & run

| Action | Command | Notes |
|--------|---------|-------|
| Type-check | `cargo +nightly check` | Fastest loop; use as default |
| Release build | `cargo +nightly build --release` | → `target/x86_64-pc-windows-msvc/release/frame0.exe` (~308 KB) |
| Debug run | `cargo +nightly run` | Launches the tray app |

- **Nightly is mandatory.** `rust-toolchain.toml` pins it; `.cargo/config.toml` uses `-Z build-std` with `panic=immediate-abort`, `location-detail=none`, `fmt-debug=none`, which stable can't satisfy. You need the `rust-src` component (`rustup component add rust-src --toolchain nightly`). The target is hardcoded to `x86_64-pc-windows-msvc`.
- **Exit the tray before rebuilding.** The running exe is locked, and cargo fails with `failed to remove file … Access is denied`. To exit, right-click the tray icon → Exit, or `frame0.exe --cmd 199`.
- **Resources.** `build.rs` embeds `res/frame0.rc`: the Common Controls v6 manifest (`res/frame0.manifest`, themed controls + the About box's SysLink) and `res/frame0.ico` (resource id 1) via `embed-resource` (needs the Windows SDK `rc.exe`); `win::app_icon(small)` loads it for the tray and the editor window class. The `.ico` is rendered from `res/frame0-small.svg` (a 16 px pixel-grid design, used for every size: 16–256 px) with ImageMagick, then packed as PNG-only entries by `python -I res/mkico.py res/frame0.ico 16.png 20.png … 256.png` (~16 KB; ImageMagick's own ICO writer stores BMPs and comes out larger).
- **Releases.** Pushing a `v*` tag runs `.github/workflows/release.yml`: it checks the tag against `Cargo.toml`'s version, builds, then publishes `frame0-<ver>-setup.exe` (NSIS, `installer/frame0.nsi`, per-user, icon `res/installer.ico`) and `frame0-<ver>-portable.zip`. Local installer build: `makensis /DVERSION=0.1.0 installer\frame0.nsi` after a release build.
- **There are no tests.** A test runner would have to cope with `build-std` + `panic=abort`.

### Binary-size discipline

The ceiling is 1 MB and the target is <500 KB; v3 is ~308 KB.
- Keep `format!` / `Debug` out of hot paths.
- No proc-macro crates (`windows-implement`, `serde_derive`, `thiserror`).
- No new crates without a reason.
- Measure after non-trivial changes: `ls -la target/x86_64-pc-windows-msvc/release/frame0.exe`.

### Testing the GUI (lessons learned)

There's no test harness; verify by driving the real app:

- **Driving the tray.** `frame0.exe --cmd <id> [--pid <pid>]` posts a tray command to a running instance. IDs are the `CMD_*` consts in `app.rs`: e.g. 100 region, 101 full screen, 199 exit.
- **Isolated config.** `FRAME0_INI=<absolute path>` uses a separate ini (and hotkey), so a test instance doesn't clash with the user's.
- **Driving windows.** Send messages straight to the HWNDs (`WM_LBUTTONDOWN/MOVE/UP` with client coordinates to `frame0.editor.canvas`, `WM_KEYDOWN` for tool letters, `WM_COMMAND` with the editor's `CMD_*` to `frame0.editor`). Avoid global synthetic input on the user's desktop: it can land in their terminal.
- **Finding windows.** From PowerShell, ANSI `FindWindow` failed to find frame0's windows; enumerate with `EnumChildWindows(IntPtr.Zero, …)` + `GetClassName`. `EnumChildWindows` is recursive: the property panel has its own `Edit` (stamp search), so check `GetParent` when looking for the in-place text EDIT.
- **Sandbox.** The tool sandbox hides other processes' windows; UI-automation scripts must run unsandboxed. Screenshots: `System.Drawing` `CopyFromScreen` after `SetProcessDPIAware`.
- **Cleanup.** Kill test instances **by PID only**, never `taskkill /IM frame0.exe`.

---

## Module map

```
src/main.rs                 Entry: --cmd forwarding, prewarm thread, COM + DPI init, App::run
src/app.rs                  App: tray message window, tray menu (Menu builder), CMD_* ids,
                            run_capture pipeline (delay timer → grab → effects → output),
                            presets, interval timer, hotkey, editor lifecycle
src/config.rs               frame0.ini (GetPrivateProfileStringW), options + presets, FRAME0_INI
src/error.rs                Error (Hr / Win32 / Generic), Result; From<windows::core::Error>
│
├── capture/
│   ├── mod.rs              CaptureSettings, Mode, Output, FX_* flags, ScreenRect
│   ├── bitblt.rs           BitBlt screen/region grab (+ cursor composite)
│   ├── monitors.rs         Monitor list, virtual-screen bounds
│   ├── print_window.rs     PrintWindow(PW_RENDERFULLCONTENT) window grab
│   ├── windows.rs          Capturable top-level window list (filters cloaked/tool/etc.)
│   ├── effects.rs          Border / drop-shadow post effects
│   └── scrolling.rs        Scrolling capture: wheel-scroll + overlap detection + stitch
│
├── overlay/
│   ├── mod.rs              Shared Backdrop (frozen, pre-dimmed snapshot DCs), helpers
│   ├── region.rs           Region / fixed-size picker (loupe, nudge, F = monitor, W = window)
│   └── window_pick.rs      Hover-highlight window picker (DWM frame bounds, top-most hit)
│
├── editor/
│   ├── annotation.rs       Annotation enum, Document, Point, Rect, geometry/hit-test helpers
│   ├── history.rs          Command (Insert/Remove/Mutate/Base/Groups/Batch), History, execute
│   ├── tools/mod.rs        Tool enum (+ALL, name, key), ToolStyle (quick style), creation logic
│   ├── canvas.rs           Canvas child window: swap chain, view, Gesture state machine, ops
│   ├── render.rs           Renderer (D2D drawing of all annotations) + flatten() for export
│   ├── pixels.rs           CPU raster ops: selection masks, fill/auto-fill, effects, cut_band
│   ├── stamps.rs           Stamp catalogue (emoji/label built-ins, user folder, search)
│   ├── text_edit.rs        In-place Win32 EDIT for Text / Callout / Step labels
│   ├── toolbar.rs          Toolbar child (GDI): Fluent-icon tool buttons, palette, undo/redo
│   ├── property.rs         Property panel child (GDI): declarative rows, stamp grid
│   ├── layout.rs           Child rects: TOOLBAR_HEIGHT = 44, PROPERTY_WIDTH = 224
│   └── window.rs           EditorWindow parent: menus, CMD_* ids, accelerators, save/export
│
├── gfx/
│   ├── mod.rs              Gfx (D2D + DWrite + GDI+ token), created once on App
│   ├── d2d.rs              D3D11/DXGI/D2D1.1 device context, swap chains, upload_bitmap
│   ├── dwrite.rs           DWrite factory + cached text formats
│   ├── gdiplus.rs          Flat Gdip* API: PNG/JPG encode/decode (temp-file bridge)
│   ├── prewarm.rs          Background LoadLibrary of d2d1/dwrite/d3d11/gdiplus + font cache
│   ├── bitmap.rs           Owned BGRA Bitmap (straight alpha, top-down)
│   └── color.rs            Color (+ palette and UI theme constants, COLORREF conversion)
│
├── io/project.rs           .frame0 read/write (v1)
│
└── win/
    ├── tray.rs             Shell_NotifyIcon, balloon, tooltip, TaskbarCreated re-add
    ├── prompt.rs           prompt::ask(owner, title, &[(label, init)]) modal input dialog
    ├── about.rs            Help → About box (in-memory dialog template, SysLink to GitHub)
    ├── hotkey.rs           HotkeySpec, parse("Ctrl+Win+PgDn"), Register/UnregisterHotKey
    ├── clipboard.rs        write CF_DIBV5 + CF_DIB + "PNG"; read PNG then CF_DIB
    ├── dialog.rs           GetOpenFileNameW / GetSaveFileNameW
    ├── window.rs           register_class / create_window / message_loop helpers
    ├── com.rs, dpi.rs      COM apartment, per-monitor-v2 DPI awareness
    └── mod.rs              wide()
```

---

## Architecture

### Process model

Single binary, single instance, persistent tray. `main.rs` first handles `--cmd` forwarding, then spawns `gfx::prewarm::spawn()`. Then it creates `App`, which owns a hidden `HWND_MESSAGE` window that receives:
- `WM_HOTKEY` (id 1) → region capture.
- `WM_TRAY` (`WM_APP+1`) → double-click capture / right-click menu.
- `WM_COMMAND` → tray commands.
- `WM_TIMER` → delay countdown / interval capture.
- `TaskbarCreated` → re-add the tray icon.

**Capture flow.** Every capture (hotkey, menu, preset, repeat, interval) builds a `CaptureSettings` and calls `App::run_capture`:
1. Optional delay timer, with the countdown in the tray tooltip.
2. `grab`: interactive modes snapshot the screen after the delay and show an overlay on the frozen image.
3. Record the resolved rect / window as "last capture".
4. `capture::effects::apply`.
5. `output` → editor (`open_editor` / `open_document`), clipboard, or file.

Hotkey and tray input are ignored while an overlay or prompt is open (`busy`). Hotkey registration failure is non-fatal (warning balloon).

### Editor window topology

`EditorWindow` is a top-level parent with three children:
- **Toolbar** (`frame0.editor.toolbar`). GDI, double-buffered. Tool icons are Segoe Fluent Icons glyphs, falling back to Segoe MDL2 Assets on Windows 10 (detected with `GetTextFaceW`). Palette clicks recolour the selection too.
- **Canvas** (`frame0.editor.canvas`). Owns a DXGI flip-model swap chain and the whole interaction state machine. Has `WS_CLIPSIBLINGS` so the text EDIT sibling shows above it. It has keyboard focus; `window::accelerator` maps Ctrl-shortcuts to menu `CMD_*` ids, which are posted to the parent as `WM_COMMAND`. Plain letters switch tools (`Tool::key`).
- **Property panel** (`frame0.editor.property`). GDI. Built from a declarative `Row` list (`rows()`), laid out by `layout()`, clicks dispatched by `apply()`. It shows the selected annotation's properties, or the tool's when nothing is selected. Edits change the quick style (`ToolStyle`) **and** the selection, so later annotations inherit them. The stamp grid is rendered through `render::flatten` (so emoji are in colour) and cached.

**Shared state.** `EditorState` holds the `Canvas` (which owns `History`), the current path, a weak ref to the app, and the child hwnds. It lives in `thread_local! HashMap<HWND, Rc<RefCell<EditorState>>>`, keyed by the parent **and every child**. Child wndprocs look it up via `window::get_state(GetParent(hwnd))`. If you add a child window, insert it into that map and clear it on `WM_DESTROY`. Use `try_borrow` in paint paths: a modal dialog can re-enter while a borrow is held.

**Text editing.** `text_edit::spawn_for(parent, canvas, TextEditReq)` creates a Win32 `EDIT` as a **sibling of the canvas under the parent** (not a child of the canvas: DXGI swap chains don't compose with overlapping child windows), placed `HWND_TOP`. It's subclassed with `SetWindowLongPtrW(GWLP_WNDPROC)`: Enter / `WM_KILLFOCUS` commit, Esc cancels, Ctrl+Enter inserts a newline. **Read the text before removing the subclass context.** `WM_GETTEXT` is routed through the subclass, which needs the context to reach the original proc; removing it first made every commit read empty text.

### Editor data model

- **`annotation.rs`**
  - Types:
    - `Annotation` variants: Arrow, Shape (Rectangle / Ellipse / Polygon), Callout, Text, Step, Highlighter, Blur (Blur / Pixelate / Solid), Magnify, Stamp (string asset id), Image (`Rc<Bitmap>` raster layer).
    - `Document`: `base: Rc<Bitmap>` plus `base_gen` (bump via `set_base` so renderers re-upload), `annotations`, `next_id`, and a `groups: HashMap<AnnotationId, u32>` side table.
  - Geometry helpers:
    - `frame()`: the unrotated handle frame.
    - `bounding_box()`: everything drawn, axis-aligned.
    - `hit_test(p, tol)` and `part_at`: Magnify's source and display drag separately.
    - `translate[_part]`, `set_frame(old, new)` (resize), `rotation` / `set_rotation` / `rotatable`.
    - `edit_points()` / `set_edit_point()`: arrow endpoints + bezier handles, polygon vertices, callout tail tips, magnifier source corners.
  - Every variant has an `id` field, so **always use `..`** when pattern-matching.
- **`history.rs`.** Delta commands, not snapshots. `history::execute` applies and records. Drag gestures mutate the document live, then push a `Batch` of `Mutate`s on mouse-up (`cancel_gesture` restores the originals). `remove_many` builds index-safe Remove batches. `Base` swaps the raster (selection ops, cut-out, flatten).
- **`tools/mod.rs`.** `create_from_drag(tool, &style, start, end, &doc)` returns an annotation with id 0 (the caller allocates the id). The canvas preview reuses it, so it must not mutate the doc. `constrain()` implements Shift. `next_step()` derives the next marker from existing markers, so it survives undo/delete.

### Rendering (`render.rs`)

- **`Renderer`** owns the device-bound resources: brush LRU, Gaussian / Shadow effects, round stroke style, uploaded base / image / stamp bitmaps, and a pixelate cache. `draw_document(gfx, ctx, doc, world, opts)` paints the base + annotations. Rotation is applied as `rotation_around(center) * world` (D2D multiplies row vectors, local first).
- **`flatten(gfx, doc, opts)`** renders the same code into an offscreen `TARGET` bitmap, in 4096-px tiles, copies into a `CPU_READ` bitmap, maps it, and un-premultiplies. This is the **only** export path: export / clipboard / copy all use it. Don't reintroduce a CPU rasterizer.
- **Shadows.** The annotation is recorded into an `ID2D1CommandList`, fed through `CLSID_D2D1Shadow` (drawn offset), then drawn plain.
- **Blur** = Gaussian effect on the base (border mode HARD), drawn clipped to the bounds. **Pixelate** = a CPU block-average bitmap (`pixels::pixelate_blocks`), cached and drawn scaled with nearest-neighbour. `RenderOpts { secure: true }` draws blur/pixelate as solid fills (the export prompt offers this).
- **Highlighter** = translucent fill. MIN blending vanishes on dark screenshots, and `D2D1_PRIMITIVE_BLEND_MULTIPLY` doesn't exist in windows 0.61 (only SOURCE_OVER / COPY / MIN / MAX / ADD), so don't go looking for it.
- **Stamps.** `e:<emoji>` is drawn with DirectWrite `D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT` (Segoe UI Emoji); `l:<TEXT>` is a vector rounded badge; `f:<path>` is a user image from `%APPDATA%\frame0\stamps\[category\]`. User stamp pixels are embedded in `.frame0` files and re-registered with `stamps::register` on load.
- **Uploads.** `gfx::d2d::upload_bitmap` premultiplies when the bitmap has any alpha < 255 (cut-out / transparent fill / pasted PNGs). The canvas draws a checkerboard behind the page.
- **Device loss.** On `D2DERR_RECREATE_TARGET`, `Canvas::render` drops the swap chain, target, checker / dash and `Renderer` resources, then re-attaches. The `ID2D1DeviceContext` is shared process-wide; always `SetTarget(None)` after `EndDraw`.

### Canvas interaction (`canvas.rs`)

- **`Gesture`** enum: Pan, Create, Polygon, Freehand, Marquee, Move, Resize, Rotate, EditPoint, SelRect, Band (cut-out), PendingLift (drag inside a pixel selection lifts it into an Image layer, filling the hole).
- **Handles** are computed in screen space (`handles()`): 8 frame handles (rotated with the object), a rotate handle, and edit points (which win ties). Resizing a rotated object keeps the opposite handle fixed on screen. With a multi-selection you resize the union frame.
- **Drawing tools** can still grab an already-selected object; otherwise they draw. New annotations become selected. Text/Callout creation returns a `TextEditReq`, which the wndproc spawns *after* releasing the borrow.
- **Clipboard.** Copy puts the annotations on an internal clipboard *and* a flattened crop on the system clipboard. Paste uses the internal annotations if `GetClipboardSequenceNumber` is unchanged, otherwise pastes a system image as an Image layer.
- **View.** Fit to the window until the user zooms or pans (`user_view`). Ctrl+wheel zooms at the cursor, wheel / Shift+wheel scroll, Space- or middle-drag pans. `clamp_view` centres small content.

### `.frame0` format (`io/project.rs`)

Little-endian:
1. `b"FRM0"` magic.
2. `u16` version (1).
3. `u32` length + PNG of the base image.
4. `next_id`, `next_step_value`.
5. `u32` count + records (`u8 TAG_*`, `u32 id`, payload).
6. `u32` count + (id, group) pairs.

Tags 1–10: Arrow, Shape, Text, Step, Highlight, Blur, Magnify, Stamp, Callout, Image. Only the current `VERSION` is read; if the layout changes, bump `VERSION` (older files are then rejected unless you add gated reads on `version >= N`).

### Persistence (`config.rs`)

- **Ini path.** `%APPDATA%\frame0\frame0.ini` (or `FRAME0_INI`), written with a UTF-16 BOM so non-ASCII preset names survive. A pre-rename `%APPDATA%\frame` folder (with `frame.ini`) is moved there on first run.
- **Sections.**
  - `[hotkey] capture`: default `Ctrl+Win+PgDn`.
  - `[capture]`: `include_cursor`, `delay`, `interval`, `output` (`editor` / `clipboard` / `file`), `effects` (bitflags), `fixed_w`, `fixed_h`.
  - `[ui] last_export_dir`.
  - `[preset.N]`: name + all settings. An interactively dragged region is stored as a fixed region.
- **Image codecs.** Go through flat GDI+ (`gfx/gdiplus.rs`). WIC was tried and dropped for size.

---

## Key patterns & gotchas

- **windows-sys 0.52 handles are `isize`** (HWND, HMENU, HICON, HCURSOR, HBRUSH, HBITMAP, HDC, HMODULE). Null is `0`; check with `h == 0` / `h != 0`, never `.is_null()`; pass `0` or `0 as _`. This burns hours if you forget.
- **Three FFI crates.**
  - `windows-sys` for flat Win32.
  - `windows 0.61` for COM (D2D / DWrite / D3D11 / DXGI); typed handles like `HWND(*mut c_void)`.
  - `windows-numerics` for `Vector2 { X, Y }` (capital fields) and `Matrix3x2 { M11..M32 }`.
  - `?` crosses between them via `From<windows::core::Error>` in `error.rs`.
- **windows 0.61 Factory1 shadowing.** `ID2D1Factory1` methods shadow the base ones: `CreateStrokeStyle` takes `D2D1_STROKE_STYLE_PROPERTIES1` and returns `ID2D1StrokeStyle1`, and `CreatePathGeometry` returns `ID2D1PathGeometry1`. Use `.into()` / `.cast()`.
- **D2D rotation units.** `Matrix3x2::rotation_around` takes degrees, clockwise (matches `Point::rotated`).
- **Anti-flicker (mandatory for GDI windows).** `WM_ERASEBKGND` → 1. Paint everything into a `CreateCompatibleDC` + `CreateCompatibleBitmap` and `BitBlt` once. Never paint several steps straight to the window DC. The overlays share a pre-dimmed `Backdrop` so they don't AlphaBlend every frame.
- **Re-entrancy.** Modal loops (menus, dialogs, MessageBox) re-enter wndprocs. Don't hold a `RefCell` borrow across them; paint paths use `try_borrow`.
- **Capture alpha.** GDI zeroes alpha on capture, so `bitblt` / `print_window` force `px[3] = 0xFF`.

### Adding a new tool / annotation variant

1. `annotation.rs`: add the variant (with `id`) and handle it in `id`/`set_id`/`name`/`frame`/`bounding_box`/`hit_test`/`translate_part`/`set_frame` (and rotation / edit points if relevant).
2. `tools/mod.rs`: add it to `Tool`, `Tool::ALL`, `name`, `key`, plus creation logic in `create_from_drag`. Add a glyph in `toolbar::glyph`.
3. `render.rs`: draw it in `Renderer::draw_kind`. Canvas *and* export pick it up automatically.
4. `io/project.rs`: new `TAG_*` in `write_ann` / `read_ann`.
5. `property.rs`: rows in `rows()`, handling in `apply()`, and `apply_color` if it's colourable.

### Hot paths

| Where | Concern |
|-------|---------|
| `Canvas::render_inner` / `Renderer::draw_document` | Every paint (each mouse-move during drags); no per-frame CPU image work. Pixelate is cached; blur runs on the GPU |
| `canvas.rs` `WM_MOUSEMOVE` | Only invalidates while a gesture is active |
| `overlay/region.rs` paint | Per mouse-move; uses the shared pre-dimmed backdrop |
| `render::flatten` | Export / copy / stamp grid; tiled GPU render + readback |
| `pixels.rs` | Runs once per op on the full base; keep it O(pixels) |
