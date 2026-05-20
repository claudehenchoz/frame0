Implement `frame`, a super-tiny but fully featured screenshot+annotations tool for Windows, written in Rust.

* Use Rust nightly for smallest-possible binary size
* Support Windows only and use the windows-sys crate with only the features needed for this app
* Make the tool as quick as possible: Bringing up the screenshot ui and everything else should feel super-snappy
* Ask me if anything is unclear


Below is an implementation-style spec for a **screenshot + annotation tool*=
*.

## 1. Capture module

Support these capture modes:

| Mode                         | Required behavior                         =
                                                                          |
| ---------------------------- | ------------------------------------------=
------------------------------------------------------------------------- |
| Full screen                  | Capture entire display or selected monitor=
.                                                                         |
| Region                       | User drags crosshair rectangle over screen=
.                                                                         |
| Fixed region                 | User defines pixel width/height; optionall=
y fixed X/Y position.                                                     |
| Window                       | Detect windows under cursor, highlight can=
didate, capture selected window.                                          |
| Hidden/open window           | Present available windows and capture sele=
cted one.                                                                 |
| Delayed capture              | Start capture after N seconds.            =
                                                                          |
| Repeat last capture          | Reuse previous capture mode and settings. =
                                                                          |
| Clipboard / file input       | Import image from clipboard or file into e=
ditor.                                                                    |

## 2. Capture settings

Each screenshot capture should store:

```ts
type CaptureSettings =3D {
  mode:
    | "fullscreen"
    | "region"
    | "fixed-region"
    | "window"
    | "clipboard"
    | "file";

  monitorId?: string;
  region?: { x: number; y: number; width: number; height: number };
  fixedSize?: { width: number; height: number };
  fixedPosition?: { x: number; y: number };
  delaySeconds?: number;
  scheduleAt?: Date;
  intervalSeconds?: number;
  includeCursor?: boolean;
  outputDestination?: "editor" | "clipboard" | "file";
  effects?: ImageEffect[];
};
```

Support presets, so capture settings should be savable and reusable.

## 3. Editor canvas model

Use a non-destructive layered model:

```ts
type ScreenshotDocument =3D {
  baseImage: RasterImage;
  canvasSize: { width: number; height: number };
  objects: AnnotationObject[];
  history: Command[];
  quickStyles: QuickStyle[];
};
```

All annotations should be movable, resizable, editable, deletable, and sele=
ctable via a Move tool.

## 4. Annotation tools

### Arrow tool

Functional requirements:

```ts
type Arrow =3D {
  kind: "arrow";
  start: Point; // tail
  end: Point;   // head
  strokeColor: Color;
  strokeWidth: number;
  endStyle: "arrow" | "circle" | "square" | "none";
  shadow?: Shadow;
  bezier?: { enabled: boolean; controlPoints: Point[] };
};
```

Behavior: click-drag from tail to head; Shift constrains to straight horizo=
ntal/vertical; optional Bezier curve with draggable handles; properties inc=
lude color, shadow, and end style.

### Shape tool

Support rectangle, ellipse, polygon, square/circle via Shift constraint, tr=
ansparent fill for frames, resize handles, rotation handle.

```ts
type Shape =3D {
  kind: "shape";
  shape: "rectangle" | "ellipse" | "polygon";
  points?: Point[];
  bounds?: Rect;
  fill: Color | "transparent";
  outline: Color;
  outlineWidth: number;
  rotation: number;
  shadow?: Shadow;
};
```

Shapes are rectangles, ellipses, and polygons, with move, resize, rotate, t=
ransparent fill, and Shift-constrained square/circle behavior.

### Callout tool

Support speech-bubble/text-box annotations.

```ts
type Callout =3D {
  kind: "callout";
  text: string;
  bounds: Rect;
  fill: Color;
  outline: Color;
  textStyle: TextStyle;
  tails: { anchor: Point; tip: Point }[];
  rotation: number;
  cornerRadius?: number;
};
```

Behavior: drag to create, type text, resize body, move/resize tail, add/del=
ete extra tails, rotate, optionally adjust rounded corners.

### Step tool

Support sequential numbered/lettered markers.

```ts
type StepMarker =3D {
  kind: "step";
  value: string; // auto: 1,2,3 or A,B,C
  position: Point;
  size: number;
  fill: Color;
  textColor: Color;
};
```

Behavior: each click places next value; allow manual edit of label value; l=
ater values inherit current properties.

### Highlighter tool

Draw translucent rectangular highlights over image regions. This is draggin=
g a highlighted area to draw attention.

### Blur / redaction tool

Support blur, pixelate, and solid redaction.

```ts
type BlurRegion =3D {
  kind: "blur";
  bounds: Rect;
  mode: "blur" | "pixelate" | "solid";
  intensity?: number;
  fillColor?: Color;
};
```

Important: for secure redaction, flatten/export with solid shapes rather th=
an reversible blur/pixelation. Warn that blur and pixelation may be reversi=
ble by third-party tools.

### Stamp tool

Support predefined and downloadable/custom stamp assets.

```ts
type Stamp =3D {
  kind: "stamp";
  assetId: string;
  bounds: Rect;
  rotation?: number;
};
```

Behavior: select stamp category/search term, click canvas to place, drag to=
 move, handles to resize.

### Magnify tool

Support source region plus magnified copy offset elsewhere.

```ts
type Magnify =3D {
  kind: "magnify";
  sourceBounds: Rect;
  displayBounds: Rect;
  magnification: number;
  lineWidth: number;
};
```

Support selecting an area, resizing the magnify selection, offsetting the m=
agnified area, and adjusting magnification/line width.

### Cut-out tool

Remove unnecessary horizontal/vertical sections with configurable edge styl=
e. Behavior: choose direction/edge style, drag across image, collapse selec=
ted band.

### Selection tool

Support rectangle, ellipse, polygon, and freehand selection. Operations: mo=
ve, delete, copy/paste, invert selection, add effects, snap to object, and =
background fill.

```ts
type SelectionSettings =3D {
  shape: "rectangle" | "ellipse" | "polygon" | "freehand";
  backgroundFill: "transparent" | "auto-fill";
  snapToObject: boolean;
};
```

The selection tool can cut/copy/move/delete and auto-fill removed areas usi=
ng surrounding content.

## 5. Required editor operations

Support:

```ts
select(objectId)
multiSelect(objectIds)
move(dx, dy)
resize(handle, dx, dy)
rotate(angle)
delete()
copy()
paste()
undo()
redo()
group()
ungroup()
flatten()
export(format: "png" | "jpg")
```

For editable project files, preserve object layers. For exported PNG/JPG, f=
latten all annotations into the raster output.

## 6. Launch mode

The tool shall be running at all times in the system tray (double-click launches "screenshot taking", right click shows "Take screnshot", "Edit screenshot" or "Exit"). A hotkey can be configured to automatically invoke the "screenshot taking" feature (Ctrl-WIN-PgDn by default).

## 7. Minimum viable feature set

To create the core screenshot + annotation experience, implement:

1. Region, window, fullscreen, fixed-size, delayed, repeat-last, and scroll=
ing capture.
2. Editor with base image plus movable annotation layers.
3. Arrow, shape, callout, text, step, highlighter, blur/redact, stamp, sele=
ction, magnify, and cut-out tools.
4. Undo/redo and flattened export.
