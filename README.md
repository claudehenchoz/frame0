# frame

A tiny Windows screenshot + annotation tool. Lives in the system tray, captures regions via a global hotkey, and opens a built-in editor.

Release binary is **~308 KB**, no installer, no runtime dependencies beyond what ships with Windows.

## Features

- **Tray app.** Double-click or `Ctrl+Win+PgDn` (configurable) to capture. Right-click for Take screenshot, Edit screenshot, Capture modes, Presets, Options and Exit.
- **Capture modes.** Region (with loupe), full screen / per monitor, fixed region, window under the cursor, any open window (even covered or minimized), scrolling capture, delayed, interval, and repeat last. Capture settings can be saved as presets. Output goes to the editor, the clipboard or `Pictures`. Optional cursor, border and shadow effects.
- **Editor.** Direct2D canvas with zoom/pan.
  - Annotation tools: Arrow (end styles, curved, shadow), Rectangle, Ellipse, Polygon, Callout, Text, Step markers, Highlighter, Blur / Pixelate / Solid redaction, Stamps (emoji, label badges, your own PNGs in `%APPDATA%\frame\stamps`), Magnify, Cut-out.
  - Pixel Selection tool: rect, ellipse, polygon or freehand selections, with delete (transparent or auto-fill), crop, invert, effects, copy and move.
  - Every object is selectable, movable, resizable, rotatable and deletable. Also: multi-select, group/ungroup, copy/paste/duplicate, z-order, undo/redo, flatten.
- **Output.** PNG/JPG export and clipboard copy are rendered by the same D2D pipeline as the canvas, so what you see is what you get. Blur and pixelate are flagged as reversible on export, with an option to convert them to solid redactions. `.frame` project files keep every layer editable.

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
