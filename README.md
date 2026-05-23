# frame

A tiny Windows screenshot + annotation tool. Lives in the system tray, captures regions via a global hotkey, and opens a built-in editor.

Release binary is **~122 KB**, no installer, no runtime dependencies beyond what ships with Windows.

## Status

This is an MVP of the larger spec in `PROMPT.md`. What works:

- System tray with right-click menu (Take / Edit / Repeat / Exit) and double-click capture.
- Global hotkey for region capture (default `Ctrl+Win+PgDn`, configurable).
- Multi-monitor region selection with live dimensions readout.
- Background pre-warm thread loads D2D / DirectWrite / D3D11 / GDI+ DLLs and primes the system font cache so the first hotkey press is fast.
- Editor with these tools: Move, Selection (rect), Arrow, Curve (Bezier arrow), Rectangle, Ellipse, Text (with in-place editing), Step, Highlighter, Blur (real Gaussian via Direct2D effect), Redact (opaque solid), Magnify (scaled inset from a source region), Stamp (4 bundled icons: ✓ ✗ ● ★).
- Canvas renders via **Direct2D** with DirectWrite text — anti-aliased arrows, ellipses, smooth gaussian blur.
- Six-color palette on the toolbar.
- Undo / Redo with full history (`Ctrl+Z` / `Ctrl+Y`).
- File menu: Save Project (`.frame`), Open Project, Export PNG, Export JPG, Copy to Clipboard.
- Clipboard write produces `CF_DIB` + `CF_DIBV5` + `CF_PNG` so paste works in Paint, Word, browsers, Slack, etc.
- INI-based config at `%APPDATA%\frame\frame.ini`.

Deferred (listed in the spec, still pending): Callout (rounded body + tails), Cut-out (band removal from the base image), polygon / ellipse / freehand selection shapes, the operations on a selection (delete / fill / invert / content-aware fill), scrolling capture, presets UI, DXGI Desktop Duplication, window-pick UX with hover-highlight, dedicated property panel for tool-specific options (stroke width / font size / blur intensity — defaults are usable but not yet editable from the UI).

Known editor / export caveats:
- The canvas (on-screen) is D2D anti-aliased; **export and clipboard flatten via the CPU rasterizer**, which doesn't anti-alias and doesn't render text glyphs (text annotations show as outlined placeholder rectangles in exported PNG/JPG). Text content still round-trips through `.frame` save/load.
- Bezier arrows look curved in the editor and in `.frame` round-trips, but the exported PNG renders them as straight lines.
- Magnify's exported PNG shows only the outlines + connector line (not the scaled-up bitmap).
- Bundled stamps (4 of them) are generated programmatically from vector geometry to keep the binary small.

## Install / build

Requires Rust nightly with the `rust-src` component:

```
rustup toolchain install nightly
rustup component add rust-src --toolchain nightly
```

Then:

```
cargo +nightly build --release
```

The output is `target/x86_64-pc-windows-msvc/release/frame.exe`. Run it directly — it sits in the tray on first launch.

The build pulls `std` from source with `panic=immediate-abort` and aggressive size flags from `.cargo/config.toml`. Stable Rust can't build this — `cargo build` without `+nightly` will fail.

## Usage

Once `frame.exe` is running, look for the tray icon.

- **Take a screenshot:** press `Ctrl+Win+PgDn`, or double-click the tray icon, or right-click the tray icon → Take screenshot. Drag a rectangle. `Esc` or right-click cancels.
- The editor opens with the capture as the base image.

In the editor:

| Action | Input |
| --- | --- |
| Pick a tool | Click its button in the toolbar |
| Pick a color | Click a swatch |
| Draw | Click and drag on the canvas |
| Edit text in place | Pick the Text tool, click — an EDIT box appears; type, press `Enter` to commit or `Esc` to cancel |
| Re-edit text | Double-click an existing text annotation |
| Select an existing annotation | Switch to Move tool, click |
| Move selection | Drag with Move tool |
| Delete selection | `Del` or `Backspace` |
| Deselect | `Esc` |
| Undo / Redo | `Ctrl+Z` / `Ctrl+Y` |
| Save project | File → Save Project |
| Export | File → Export PNG / Export JPG |
| Copy to clipboard | File → Copy to Clipboard |

### Configuration

`%APPDATA%\frame\frame.ini`. Created on first run with defaults. Recognized keys:

```ini
[hotkey]
capture = Ctrl+Win+PgDn

[capture]
include_cursor = false

[ui]
last_export_dir =
```

Hotkey modifiers: `Ctrl`, `Alt`, `Shift`, `Win`. Keys: letters/digits, `F1`–`F12`, `PgUp`, `PgDn`, `Home`, `End`, `Insert`, `Space`, arrow keys, `PrintScreen`.

### `.frame` project files

Custom little-endian binary format. Round-trips the base image (as embedded PNG) plus all annotation layers. Open with File → Open Project.

PNG / JPG export is flattened (annotations are rasterized into the base image). The `.frame` file is the only way to come back and edit layers.

## License

No license has been declared. Treat this as all-rights-reserved unless and until a license file is added.
