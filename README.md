# frame

A tiny Windows screenshot + annotation tool. Lives in the system tray, captures regions via a global hotkey, and opens a built-in editor with arrow / shape / text / step / highlighter / blur / redact tools.

Release binary is **~90 KB**, no installer, no runtime dependencies beyond what ships with Windows.

## Status

This is an MVP of the larger spec in `PROMPT.md`. What works:

- System tray with right-click menu (Take / Edit / Repeat / Exit) and double-click capture.
- Global hotkey for region capture (default `Ctrl+Win+PgDn`, configurable).
- Multi-monitor region selection with live dimensions readout.
- Editor with these tools: Move, Selection (rect), Arrow, Rectangle, Ellipse, Text, Step, Highlighter, Blur, Redact.
- Six-color palette on the toolbar.
- Undo / Redo with full history (`Ctrl+Z` / `Ctrl+Y`).
- File menu: Save Project (`.frame`), Open Project, Export PNG, Export JPG, Copy to Clipboard.
- Clipboard write produces `CF_DIB` + `CF_DIBV5` + `CF_PNG` so paste works in Paint, Word, browsers, Slack, etc.
- INI-based config at `%APPDATA%\frame\frame.ini`.

What's deferred (listed in the spec, not yet built): Callout, Magnify, Cut-out, Stamps, polygon/ellipse/freehand selection, Bezier arrows, scrolling capture, content-aware fill, presets UI, DXGI Desktop Duplication, window-pick UX with hover-highlight. Text annotations save and load but currently have no in-place editor — their content can only be set by editing a `.frame` file.

The editor canvas currently paints with GDI, so arrows and ellipses don't have anti-aliasing and the Blur tool draws an opaque dark rectangle rather than a real gaussian blur. The Direct2D / DirectWrite infrastructure is already linked in and ready for a future upgrade.

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
