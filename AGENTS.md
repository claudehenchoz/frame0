# AGENTS.md — AI Coding Agent Guide

This file is the companion to `CLAUDE.md` and provides the detailed reference an AI agent needs to make safe, correct edits across the `frame` codebase. Read `CLAUDE.md` first for high-level project context; use this for per-file patterns, naming conventions, and gotchas.

---

## Build & Run Nexus

### Commands

| Action | Command | Notes |
|--------|---------|-------|
| Type-check | `cargo +nightly check` | Fastest feedback loop; use this as your default |
| Debug build | `cargo +nightly build` | Produces `target/debug/frame.exe` |
| Release build | `cargo +nightly build --release` | Produces `target/x86_64-pc-windows-msvc/release/frame.exe` (~90 KB) |
| Run debug | `cargo +nightly run` | Launches the tray app |

### Critical gotcha

**Always exit the tray before rebuilding.** The running binary locks the `.exe`. Right-click the tray icon → **Exit**, then rebuild. Otherwise `cargo build` fails with `Access is denied`.

### Prerequisites

```bash
rustup toolchain install nightly
rustup component add rust-src --toolchain nightly
```

Nightly is mandatory. Stable will fail because `.cargo/config.toml` uses `build-std`, `panic=immediate-abort`, `location-detail=none`, `fmt-debug=none`.

---

## Module Map

```
src/main.rs                      → Entry point: init COM + DPI, create App, message loop
src/app.rs                       → App struct: tray, config, hotkey, editor lifecycle
src/config.rs                    → INI-based config (GetPrivateProfileStringW)
src/error.rs                     → Error enum (Hr/Win32/Generic), Result<T>
│
├── capture/                     → Screen capture
│   ├── mod.rs                   → CaptureMode enum, run() dispatcher
│   ├── bitblt.rs                → BitBlt-based screen/region capture + cursor overlay
│   ├── monitors.rs              → Monitor list, virtual screen bounds
│   ├── print_window.rs          → PrintWindow-based window capture
│   ├── windows.rs               → Window list (EnumWindows)
│   └── delay.rs                 → Stub; delayed capture deferred
│
├── editor/                      → Annotation editor
│   ├── mod.rs                   → Re-exports
│   ├── annotation.rs            → Annotation enum, Document, Point, Rect, Color
│   ├── canvas.rs                → GDI-based rendering, DIB upload, screen↔doc coords
│   ├── flatten.rs               → CPU rasterizer for export/clipboard
│   ├── history.rs               → Undo/redo via delta Commands (Insert/Remove/Mutate)
│   ├── tools/
│   │   └── mod.rs               → Tool enum, commit_drag(), ToolStyle
│   ├── toolbar.rs               → Tool list, palette, constants (TOOLBAR_HEIGHT=44)
│   └── window.rs                → Editor WndProc, toolbar paint, menus, keyboard dispatch
│
├── gfx/                         → Graphics infrastructure
│   ├── mod.rs                   → Gfx struct (D2D + DWrite + GDI+)
│   ├── bitmap.rs                → Owned BGRA Bitmap struct
│   ├── color.rs                 → Color struct with named constants
│   ├── d2d.rs                   → D2D1.1 + D3D11 + DXGI factories (wired but unused by canvas)
│   ├── dwrite.rs                → DWrite factory + cached text formats
│   └── gdiplus.rs               → GDI+ thin wrappers (PNG/JPG encode/decode, temp-file bridge)
│
├── io/                          → File I/O
│   ├── mod.rs                   → export_png, export_jpg, copy_to_clipboard (via flatten)
│   └── project.rs               → .frame binary format (magic "FRAM" + v1 + annotations)
│
├── overlay/                     → Fullscreen selection overlays
│   ├── mod.rs                   → (empty module file)
│   ├── region.rs                → Drag-rect overlay with dimensions readout
│   └── window_pick.rs           → Stub (returns foreground window only)
│
└── win/                         → Win32 wrappers
    ├── mod.rs                   → wide(), HWND_NULL
    ├── clipboard.rs             → CF_DIB + CF_DIBV5 + CF_PNG clipboard write/read
    ├── com.rs                   → CoInitializeEx/CoUninitialize
    ├── dialog.rs                → GetOpenFileNameW/GetSaveFileNameW
    ├── dpi.rs                   → SetProcessDpiAwarenessContext
    ├── hotkey.rs                → RegisterHotKey/UnregisterHotKey, HotkeySpec
    ├── tray.rs                  → NOTIFYICONDATAW tray icon + context menu
    └── window.rs                → RegisterClassExW, CreateWindowExW, message_loop helpers
```

---

## Key Patterns & Conventions

### The `isize` handle gotcha (windows-sys 0.52)

In `windows-sys 0.52`, handle types like `HWND`, `HMENU`, `HICON`, `HCURSOR`, `HBRUSH`, `HBITMAP`, `HDC`, `HMODULE` are **`isize`**, not raw pointers. **Null is `0`**, not `core::ptr::null_mut()`. Checks are `h == 0` / `h != 0`. Function args passing null handles use `0` or `0 as _`.

The `windows 0.61` crate uses typed wrappers (e.g. `HWND(*mut c_void)`) — see `src/error.rs` for the `From<windows::core::Error>` bridge that lets `?` cross between them.

### Anti-flicker pattern (mandatory for all paint paths)

Both `overlay/region.rs` and `editor/window.rs` follow this pattern:

```
WM_ERASEBKGND → return 1   // suppress default erase

WM_PAINT:
  BeginPaint
  CreateCompatibleDC + CreateCompatibleBitmap  // memory DC
  SelectObject(mem_bmp) into mem_dc
  ... paint everything into mem_dc ...
  BitBlt from mem_dc to hdc  // single blit to screen
  SelectObject(old), DeleteObject(mem_bmp), DeleteDC(mem_dc)
  EndPaint
```

**Never paint multiple steps directly to the window HDC** — intermediate fills produce visible flicker on every `WM_MOUSEMOVE` invalidation.

### Editor canvas coordinate system

- The toolbar occupies the top 44 pixels. The canvas's logical origin (0,0) is at device `(0, TOOLBAR_HEIGHT)`.
- `editor/canvas.rs::render()` paints at logical (0,0). `editor/window.rs::wndproc` applies `SetViewportOrgEx(mem_dc, 0, TOOLBAR_HEIGHT, ...)` before calling render.
- `screen_to_doc(x, y)` converts device coords to document coords: subtract `view_offset`, divide by `view_scale`.

### Adding a new annotation tool

The 6-step checklist from CLAUDE.md:

1. **`editor/annotation.rs`** — Add variant to `Annotation` enum with `id: AnnotationId` field. Update `id()`, `bounding_box()`, `translate()`.
2. **`editor/tools/mod.rs`** — Add to `Tool` enum, add `commit_drag` match arm.
3. **`editor/canvas.rs`** — Add `draw_<kind>` method, call from `draw_annotation()` match.
4. **`editor/flatten.rs`** — Add rasterize arm (CPU path for export/clipboard).
5. **`io/project.rs`** — Pick new `TAG_*` byte, add to `write_ann()`/`read_ann()`.
6. **`editor/toolbar.rs`** — Add entry to `TOOLS` slice.

### Undo/redo system

`editor/history.rs` stores delta-based `Command` variants — not full snapshots:

- `Command::Insert { index, ann }` / `Command::Remove { index, ann }` / `Command::Mutate { id, before, after }`
- `History::undo()` pops from `undo` stack, applies reverse, pushes to `redo`.
- `History::redo()` pops from `redo`, applies forward, pushes to `undo`.
- `commit_drag()` returns `Option<Command>`; the editor pushes it to history on mouse-up.

### `.frame` binary format

See `io/project.rs` for the full layout:

```
0..4    magic "FRAM"
4..6    u16 version = 1
6..10   u32 base_png_len
10..    base PNG bytes
...     u32 next_id
...     u32 next_step_value
...     u32 annotation_count
...     annotation records (u8 TAG_* + variant payload)
```

Current tags: `TAG_ARROW=1`, `TAG_SHAPE=2`, `TAG_TEXT=3`, `TAG_STEP=4`, `TAG_HIGHLIGHT=5`, `TAG_BLUR=6`.

### D2D readiness

`Gfx` (in `gfx/mod.rs`) constructs the full D2D/DWrite/D3D11/DXGI stack at startup, but the editor canvas only renders through GDI. The D2D infrastructure is:

- `gfx/d2d.rs` — `D2D::create()` builds `ID2D1Factory1` + `ID3D11Device` + `ID2D1DeviceContext` + swap chain creation helpers
- `gfx/dwrite.rs` — `DWrite` with cached `IDWriteTextFormat` factory
- Canvas migration to D2D is a planned upgrade — don't tear out GDI paths until D2D is verified pixel-equivalent

### Binary-size discipline

Current release ~90 KB. Avoid:

- `format!` / `Debug` in hot paths (they pull in formatting machinery)
- Proc-macro deps (`windows-implement`, `serde_derive`, `thiserror`)
- `.cargo/config.toml` already strips aggressively: `panic=immediate-abort`, `location-detail=none`, `fmt-debug=none`

Measure after non-trivial additions: `ls target/x86_64-pc-windows-msvc/release/frame.exe`

---

## Hot Paths to Be Careful With

| File | Function | Concern |
|------|----------|---------|
| `editor/canvas.rs` | `render()` | Called on every WM_PAINT (mouse-move during drag); avoid allocations |
| `editor/window.rs` | `wndproc` WM_MOUSEMOVE | Invalidates the whole window each frame — keep `render()` lean |
| `overlay/region.rs` | `paint()` | Same — called per mouse-move during drag |
| `editor/flatten.rs` | `flatten()` | CPU path for export/clipboard; called on save/export/copy |
| `capture/bitblt.rs` | `region()` | Hot path on every capture; includes cursor overlay via GDI |

---

## Per-File Editing Guide

### `editor/annotation.rs`
- Every `Annotation` variant **must** have an `id: AnnotationId` field.
- When matching on variants, always use `..` to skip the id field: `Annotation::Arrow { start, end, .. }`.
- `Color` fields store pre-multiplied alpha values; `a == 0` means transparent.
- `ShapeKind::Rectangle` / `ShapeKind::Ellipse`; `BlurMode::Solid` / `BlurMode::Blur`.

### `editor/tools/mod.rs`
- `Tool::Move` and `Tool::SelectionRect` return `None` from `commit_drag()` (they handle clicks differently).
- `Tool::Text` allows single-click creation (tiny drag) — creates a default 200×`font_size*1.4` rect.
- `Tool::Step` auto-increments `doc.next_step_value` on creation.

### `editor/canvas.rs`
- Uses `unsafe` blocks extensively for GDI calls. Keep unsafe scoped tightly.
- `rgb_to_colorref()` converts `Color` to GDI's BGR: `(c.r) | (c.g<<8) | (c.b<<16)`.
- `to_screen()` applies `view_offset + p * view_scale` for rendering.
- `screen_to_doc()` inverts for mouse-hit calculations.
- The `_silence_unused` function at the bottom of `window.rs` and `region.rs` is a common pattern to suppress dead-code warnings on symbols imported only for assembly.

### `editor/flatten.rs`
- CPU rasterizer; every annotation type gets a `raster_*` function.
- Text is **placeholder only** (bordered rectangle). Text content round-trips via `.frame` files but doesn't render on export. Don't add glyph rasterization unless that's specifically tasked.
- `raster_box_blur()` is a naive CPU box blur (O(n²×r²)). Works for small regions.
- `blend()` handles alpha compositing: `dst = src*c.a + dst*(255-c.a)`.

### `editor/window.rs`
- The toolbar is painted pixel-by-pixel in `paint_toolbar()`. Buttons are 60px wide, swatches are 24px with 6px gaps.
- Command IDs for the File menu: `CMD_SAVE_PROJECT=200`, `CMD_OPEN_PROJECT=201`, `CMD_EXPORT_PNG=202`, `CMD_EXPORT_JPG=203`, `CMD_COPY=204`, `CMD_CLOSE=205`.
- Thread-local `EDITOR_STATE_BY_HWND` stores the editor state; the WndProc retrieves it via `state = EDITOR_STATE_BY_HWND.with(...)`.
- `SetViewportOrgEx` shift by `TOOLBAR_HEIGHT` is applied before `canvas.render()`.

### `editor/toolbar.rs`
- `TOOLS` order determines toolbar layout. `PALETTE` is 6 colors: RED, BLUE, GREEN, YELLOW, BLACK, WHITE.
- `TOOLBAR_HEIGHT = 44`, `SWATCH_SIZE = 24`.

### `capture/bitblt.rs`
- Uses `CreateCompatibleBitmap` + `BitBlt(SRCCOPY|CAPTUREBLT)` for pixel capture.
- Cursor overlay via `GetCursorInfo` + `DrawIconEx`, offset by the hot spot.
- After capture, forces alpha to 0xFF: `px[3] = 0xFF` (GDI DCs zero the alpha channel).

### `overlay/region.rs`
- Fullscreen `WS_POPUP | WS_EX_TOPMOST` overlay on the virtual screen.
- Double-buffers into a memory DC: paints desktop → dark tint via `AlphaBlend` → clears selection rect → draws green outline + dimension label → single `BitBlt` to screen.
- Selection confirmed when `w >= 2 && h >= 2`; cancelled on right-click or Escape.

### `io/project.rs`
- `Reader` struct with `take()`, `u8()`, `u16()`, `u32()`, `f32()` methods simplifies deserialization.
- Strings: `u32` length prefix + UTF-8 bytes. Colors: 4 bytes RGBA.
- PNG encoding for the base image goes through a temp file (GDI+ limitation). The comment at `encode_png_bytes` says in-memory IStream path is deferred to v1.5.

### `gfx/gdiplus.rs`
- GDI+ startup/shutdown via `GdiplusStartup`/`GdiplusShutdown`.
- PNG/JPG encode uses `GdipSaveImageToFile` (file path, not IStream).
- `encode_png_bytes()` writes to a temp file, reads back, deletes temp.
- `decode_file()` / `decode_bytes()` reverse the process.
- `BitmapData` locking reads pixels from a loaded image.
- The `PixelFormat32bppARGB` constant is hardcoded as `2498570` (0x0026200A) to avoid pulling in GDI+ enum definitions.

### `win/clipboard.rs`
- Sets `CF_DIB` (BITMAPINFOHEADER + pixels), `CF_DIBV5` (BITMAPV5HEADER with masks + sRGB), and `CF_PNG` (registered format).
- `set_global()` allocates global memory, locks, copies, unlocks, and calls `SetClipboardData`.
- Clipboard read prefers `CF_PNG` first, falls back to `CF_DIB`.

### `win/hotkey.rs`
- `HotkeySpec` holds `mods` (bitfield of `MOD_CONTROL|MOD_ALT|MOD_SHIFT|MOD_WIN|MOD_NOREPEAT`) and `vk` (virtual key code).
- `parse()` decodes strings like `"Ctrl+Win+PgDn"` — handles `ctrl/control`, `alt`, `shift`, `win/windows/super/meta`, F-keys, navigation keys, `printscreen`, single alphanumeric chars.

### `win/tray.rs`
- Shell_NotifyIconW with `NIF_ICON | NIF_MESSAGE | NIF_TIP`. Callback message is `WM_TRAY` (`WM_APP+1`).
- Context menu uses `CreatePopupMenu` + `AppendMenuW` + `TrackPopupMenu(TPM_RETURNCMD)`.
- `taskbar_created_msg` is registered via `RegisterWindowMessageW("TaskbarCreated")` — re-adds tray icon after Explorer restart.

---

## File I/O & Persistence

| Data | Format | Path | Code |
|------|--------|------|------|
| Config | INI | `%APPDATA%\frame\frame.ini` | `config.rs` via `GetPrivateProfileStringW` |
| Projects | `.frame` binary | User-chosen | `io/project.rs` |
| Exports | PNG / JPG | User-chosen | `io/mod.rs` → `gfx/gdiplus.rs` |

---

## Known Deferred Features (Don't Assume They Exist)

From the full spec in `PROMPT.md`; **not yet implemented**:

- Callout annotation
- Magnify annotation
- Cut-out annotation
- Stamps (predefined shapes)
- Polygon / freehand selection
- Bezier arrows
- Scrolling capture (window scroll capture)
- Content-aware fill
- Presets UI
- DXGI Desktop Duplication (uses BitBlt instead)
- Window-pick UX with hover highlight (returns foreground window only)
- In-place text editing (Text annotations are placeholder rectangles)
- Delayed capture (`capture/delay.rs` is an empty stub)
- D2D-based canvas rendering (GDI is the only path)
- In-memory IStream for GDI+ encode/decode (uses temp files)
