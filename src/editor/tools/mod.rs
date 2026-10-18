use crate::editor::Document;
use crate::editor::annotation::{
    Annotation, BlurMode, Color, EndStyle, Point, Rect, ShapeKind, Tail, default_bezier, step_label,
};
use crate::editor::pixels::{CutEdge, SelShape};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tool {
    Move,
    Select,
    Arrow,
    Rect,
    Ellipse,
    Polygon,
    Callout,
    Text,
    Step,
    Highlighter,
    Blur,
    Stamp,
    Magnify,
    CutOut,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Move => "Move",
            Tool::Select => "Selection",
            Tool::Arrow => "Arrow",
            Tool::Rect => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Polygon => "Polygon",
            Tool::Callout => "Callout",
            Tool::Text => "Text",
            Tool::Step => "Step",
            Tool::Highlighter => "Highlighter",
            Tool::Blur => "Blur / Redact",
            Tool::Stamp => "Stamp",
            Tool::Magnify => "Magnify",
            Tool::CutOut => "Cut-out",
        }
    }

    /// Single-key shortcut shown in the toolbar tooltip.
    pub fn key(self) -> u8 {
        match self {
            Tool::Move => b'V',
            Tool::Select => b'S',
            Tool::Arrow => b'A',
            Tool::Rect => b'R',
            Tool::Ellipse => b'E',
            Tool::Polygon => b'P',
            Tool::Callout => b'C',
            Tool::Text => b'T',
            Tool::Step => b'N',
            Tool::Highlighter => b'H',
            Tool::Blur => b'B',
            Tool::Stamp => b'K',
            Tool::Magnify => b'M',
            Tool::CutOut => b'X',
        }
    }

    pub const ALL: [Tool; 14] = [
        Tool::Move,
        Tool::Select,
        Tool::Arrow,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Polygon,
        Tool::Callout,
        Tool::Text,
        Tool::Step,
        Tool::Highlighter,
        Tool::Blur,
        Tool::Stamp,
        Tool::Magnify,
        Tool::CutOut,
    ];
}

/// Current "quick style": properties new annotations inherit.
#[derive(Clone, Debug)]
pub struct ToolStyle {
    pub color: Color,
    pub stroke: f32,
    pub font_size: f32,
    /// 0 = transparent, 1..254 = translucent, 255 = solid (shapes/callouts).
    pub fill_alpha: u8,
    pub shadow: bool,
    pub head: EndStyle,
    pub tail: EndStyle,
    pub curved: bool,
    pub blur_mode: BlurMode,
    pub blur_intensity: f32,
    pub stamp_asset: String,
    pub stamp_category: String,
    pub stamp_query: String,
    pub step_letters: bool,
    pub step_radius: f32,
    pub magnification: f32,
    pub corner_radius: f32,
    pub sel_shape: SelShape,
    pub sel_autofill: bool,
    pub snap: bool,
    pub cut_horizontal: bool,
    pub cut_edge: CutEdge,
}

impl Default for ToolStyle {
    fn default() -> Self {
        Self {
            color: Color::RED,
            stroke: 4.0,
            font_size: 20.0,
            fill_alpha: 0,
            shadow: false,
            head: EndStyle::Arrow,
            tail: EndStyle::None,
            curved: false,
            blur_mode: BlurMode::Blur,
            blur_intensity: 12.0,
            stamp_asset: crate::editor::stamps::DEFAULT.to_string(),
            stamp_category: "All".to_string(),
            stamp_query: String::new(),
            step_letters: false,
            step_radius: 16.0,
            magnification: 2.0,
            corner_radius: 12.0,
            sel_shape: SelShape::Rectangle,
            sel_autofill: true,
            snap: true,
            cut_horizontal: true,
            cut_edge: CutEdge::Zigzag,
        }
    }
}

impl ToolStyle {
    pub fn fill(&self) -> Color {
        Color { a: self.fill_alpha, ..self.color }
    }
}

/// Shift-constraint: arrows snap to horizontal/vertical, boxes to squares.
pub fn constrain(tool: Tool, start: Point, end: Point, shift: bool) -> Point {
    if !shift {
        return end;
    }
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    match tool {
        Tool::Arrow => {
            if dx.abs() >= dy.abs() {
                Point::new(end.x, start.y)
            } else {
                Point::new(start.x, end.y)
            }
        }
        Tool::Rect | Tool::Ellipse | Tool::Callout | Tool::Highlighter | Tool::Blur | Tool::Magnify | Tool::Select | Tool::Stamp => {
            let s = dx.abs().max(dy.abs());
            Point::new(start.x + s * dx.signum(), start.y + s * dy.signum())
        }
        _ => end,
    }
}

/// Next step value: one past the highest existing marker of the same style
/// (so deleting / undoing markers keeps the sequence sensible).
pub fn next_step(doc: &Document, letters: bool) -> String {
    let mut max = 0u32;
    for a in &doc.annotations {
        if let Annotation::Step { value, .. } = a {
            let n = if letters {
                if value.chars().all(|c| c.is_ascii_uppercase()) && !value.is_empty() {
                    value.bytes().fold(0u32, |acc, b| acc * 26 + (b - b'A' + 1) as u32)
                } else {
                    0
                }
            } else {
                value.parse::<u32>().unwrap_or(0)
            };
            max = max.max(n);
        }
    }
    step_label(max + 1, letters)
}

/// Build the annotation for a completed drag (or click, when start≈end).
/// Returns None when the gesture is too small to mean anything.
/// The returned annotation has id 0; the caller allocates a real id.
pub fn create_from_drag(tool: Tool, style: &ToolStyle, start: Point, end: Point, doc: &Document) -> Option<Annotation> {
    let bounds = Rect::from_points(start, end);
    let tiny = bounds.w < 3.0 || bounds.h < 3.0;
    let ann = match tool {
        Tool::Arrow => {
            if start.dist(end) < 4.0 {
                return None;
            }
            Annotation::Arrow {
                id: 0,
                start,
                end,
                color: style.color,
                stroke: style.stroke,
                bezier: if style.curved { Some(default_bezier(start, end)) } else { None },
                head: style.head,
                tail: style.tail,
                shadow: style.shadow,
            }
        }
        Tool::Rect | Tool::Ellipse => {
            if tiny {
                return None;
            }
            Annotation::Shape {
                id: 0,
                kind: if tool == Tool::Rect { ShapeKind::Rectangle } else { ShapeKind::Ellipse },
                bounds,
                points: Vec::new(),
                outline: style.color,
                fill: style.fill(),
                stroke: style.stroke,
                rotation: 0.0,
                shadow: style.shadow,
            }
        }
        Tool::Callout => {
            // Click → default body to the upper-right of the click with the
            // tail pointing at the click. Drag → body is the dragged box and
            // the tail points down-left from it.
            let (body, tip) = if tiny {
                (Rect { x: start.x + 40.0, y: start.y - 110.0, w: 220.0, h: 80.0 }, start)
            } else {
                (bounds, Point::new(bounds.x - bounds.w * 0.15, bounds.bottom() + bounds.h * 0.6))
            };
            Annotation::Callout {
                id: 0,
                bounds: body,
                text: String::new(),
                fill: if style.fill_alpha == 0 { Color::WHITE } else { style.fill() },
                outline: style.color,
                text_color: if style.fill_alpha == 255 { Color::WHITE } else { Color::rgb(0x20, 0x20, 0x20) },
                size: style.font_size,
                stroke: (style.stroke * 0.6).max(1.0),
                tails: vec![Tail { anchor: Point::new(0.5, 0.5), tip }],
                rotation: 0.0,
                radius: style.corner_radius,
                shadow: style.shadow,
            }
        }
        Tool::Text => {
            let b = if tiny {
                Rect { x: start.x, y: start.y, w: 240.0, h: style.font_size * 1.5 }
            } else {
                Rect { h: bounds.h.max(style.font_size * 1.5), ..bounds }
            };
            Annotation::Text { id: 0, bounds: b, text: String::new(), color: style.color, size: style.font_size, rotation: 0.0 }
        }
        Tool::Step => Annotation::Step {
            id: 0,
            center: start,
            radius: style.step_radius,
            value: next_step(doc, style.step_letters),
            fill: style.color,
            text: if style.color == Color::WHITE || style.color == Color::YELLOW { Color::BLACK } else { Color::WHITE },
        },
        Tool::Highlighter => {
            if tiny {
                return None;
            }
            Annotation::Highlighter { id: 0, bounds, color: style.color }
        }
        Tool::Blur => {
            if tiny {
                return None;
            }
            Annotation::Blur { id: 0, bounds, mode: style.blur_mode, intensity: style.blur_intensity, fill: Color::BLACK }
        }
        Tool::Magnify => {
            if tiny {
                return None;
            }
            // The drag picks the source; the magnified copy is placed beside
            // it (right if there's room, else left), scaled by the factor.
            let m = style.magnification.max(1.1);
            let (dw, dh) = (bounds.w * m, bounds.h * m);
            let cw = doc.base.width as f32;
            let x = if bounds.right() + 24.0 + dw <= cw || bounds.x - 24.0 - dw < 0.0 {
                bounds.right() + 24.0
            } else {
                bounds.x - 24.0 - dw
            };
            let y = (bounds.center().y - dh * 0.5).max(0.0);
            Annotation::Magnify {
                id: 0,
                source: bounds,
                display: Rect { x, y, w: dw, h: dh },
                line_width: (style.stroke * 0.5).max(1.0),
                color: style.color,
            }
        }
        Tool::Stamp => {
            let asp = crate::editor::stamps::aspect(&style.stamp_asset);
            let (w, h) = if tiny {
                (56.0 * asp, 56.0)
            } else {
                let h = bounds.h.max(bounds.w / asp).max(16.0);
                (h * asp, h)
            };
            let b = if tiny { Rect { x: start.x - w * 0.5, y: start.y - h * 0.5, w, h } } else { Rect { x: bounds.x, y: bounds.y, w, h } };
            Annotation::Stamp { id: 0, asset: style.stamp_asset.clone(), bounds: b, rotation: 0.0 }
        }
        Tool::Polygon | Tool::Move | Tool::Select | Tool::CutOut => return None,
    };
    Some(ann)
}

pub fn create_polygon(style: &ToolStyle, points: Vec<Point>) -> Option<Annotation> {
    if points.len() < 3 {
        return None;
    }
    let bounds = Rect::bounding(&points);
    if bounds.w < 3.0 && bounds.h < 3.0 {
        return None;
    }
    Some(Annotation::Shape {
        id: 0,
        kind: ShapeKind::Polygon,
        bounds,
        points,
        outline: style.color,
        fill: style.fill(),
        stroke: style.stroke,
        rotation: 0.0,
        shadow: style.shadow,
    })
}
