use crate::editor::annotation::{Annotation, BlurMode, Color, Point, Rect, ShapeKind};
use crate::editor::history::Command;
use crate::editor::Document;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tool {
    Move,
    SelectionRect,
    Arrow,
    BezierArrow,
    ShapeRect,
    ShapeEllipse,
    Text,
    Step,
    Highlighter,
    BlurGaussian,
    BlurSolid,
    Magnify,
    Stamp,
}

#[derive(Clone, Copy, Debug)]
pub struct ToolStyle {
    pub color: Color,
    pub stroke: f32,
    pub font_size: f32,
    pub fill_alpha: u8, // 0 means transparent fill
    pub blur_intensity: f32,
    pub stamp_asset: u8,
}

impl Default for ToolStyle {
    fn default() -> Self {
        Self {
            color: Color::RED,
            stroke: 4.0,
            font_size: 20.0,
            fill_alpha: 0,
            blur_intensity: 12.0,
            stamp_asset: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Drag {
    pub start: Point,
    pub current: Point,
}

/// Commit the drag into a new annotation. Returns an Insert command if the drag
/// is large enough to be meaningful, else None.
pub fn commit_drag(
    tool: Tool,
    style: ToolStyle,
    drag: Drag,
    doc: &mut Document,
) -> Option<Command> {
    let bounds = Rect::from_points(drag.start, drag.current);
    let tiny = bounds.w < 2.0 || bounds.h < 2.0;
    let id = doc.alloc_id();
    let ann = match tool {
        Tool::Arrow => Annotation::Arrow {
            id,
            start: drag.start,
            end: drag.current,
            color: style.color,
            stroke: style.stroke,
            bezier: None,
        },
        Tool::BezierArrow => {
            // Default control point: 1/4 of the shaft's length offset
            // perpendicular from its midpoint. Gives a noticeable but tasteful
            // bend; user can adjust via property-panel edits (v2.1).
            let dx = drag.current.x - drag.start.x;
            let dy = drag.current.y - drag.start.y;
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let mid = Point::new(
                (drag.start.x + drag.current.x) * 0.5,
                (drag.start.y + drag.current.y) * 0.5,
            );
            let nx = -dy / len;
            let ny = dx / len;
            let bend = (len * 0.25).min(80.0);
            Annotation::Arrow {
                id,
                start: drag.start,
                end: drag.current,
                color: style.color,
                stroke: style.stroke,
                bezier: Some(Point::new(mid.x + nx * bend, mid.y + ny * bend)),
            }
        }
        Tool::ShapeRect => {
            if tiny {
                return None;
            }
            Annotation::Shape {
                id,
                kind: ShapeKind::Rectangle,
                bounds,
                outline: style.color,
                fill: Color {
                    a: style.fill_alpha,
                    ..style.color
                },
                stroke: style.stroke,
                rotation: 0.0,
            }
        }
        Tool::ShapeEllipse => {
            if tiny {
                return None;
            }
            Annotation::Shape {
                id,
                kind: ShapeKind::Ellipse,
                bounds,
                outline: style.color,
                fill: Color {
                    a: style.fill_alpha,
                    ..style.color
                },
                stroke: style.stroke,
                rotation: 0.0,
            }
        }
        Tool::Text => {
            if tiny {
                // Allow a single-click create with a default-size box
                let bounds = Rect {
                    x: drag.start.x,
                    y: drag.start.y,
                    w: 200.0,
                    h: style.font_size * 1.4,
                };
                Annotation::Text {
                    id,
                    bounds,
                    text: String::new(),
                    color: style.color,
                    size: style.font_size,
                }
            } else {
                Annotation::Text {
                    id,
                    bounds,
                    text: String::new(),
                    color: style.color,
                    size: style.font_size,
                }
            }
        }
        Tool::Step => {
            let val = doc.next_step_value;
            doc.next_step_value += 1;
            Annotation::Step {
                id,
                center: drag.start,
                radius: 16.0,
                value: format!("{}", val),
                fill: style.color,
                text: Color::WHITE,
            }
        }
        Tool::Highlighter => {
            if tiny {
                return None;
            }
            Annotation::Highlighter {
                id,
                bounds,
                color: Color::rgba(style.color.r, style.color.g, style.color.b, 100),
            }
        }
        Tool::BlurGaussian => {
            if tiny {
                return None;
            }
            Annotation::Blur {
                id,
                bounds,
                mode: BlurMode::Blur,
                intensity: style.blur_intensity,
                fill: Color::BLACK,
            }
        }
        Tool::BlurSolid => {
            if tiny {
                return None;
            }
            Annotation::Blur {
                id,
                bounds,
                mode: BlurMode::Solid,
                intensity: 0.0,
                fill: Color::BLACK,
            }
        }
        Tool::Magnify => {
            if tiny {
                return None;
            }
            // Source defaults to the centre of the display rect, half the size
            // — a 2× zoom. User-editable in v2.1.
            let cx = bounds.x + bounds.w * 0.5;
            let cy = bounds.y + bounds.h * 0.5;
            let sw = bounds.w * 0.5;
            let sh = bounds.h * 0.5;
            let source = Rect {
                x: cx - sw * 0.5,
                y: cy - sh * 0.5,
                w: sw,
                h: sh,
            };
            // Display sits offset to the right of the source so the connector
            // line is visible.
            let display = Rect {
                x: bounds.x + bounds.w * 0.6,
                y: bounds.y,
                w: bounds.w,
                h: bounds.h,
            };
            Annotation::Magnify {
                id,
                source,
                display,
                line_width: style.stroke,
                color: style.color,
            }
        }
        Tool::Stamp => {
            let size = if tiny { 48.0 } else { bounds.w.max(bounds.h).max(24.0) };
            let stamp_bounds = Rect {
                x: drag.start.x - size * 0.5,
                y: drag.start.y - size * 0.5,
                w: size,
                h: size,
            };
            Annotation::Stamp {
                id,
                asset: style.stamp_asset,
                bounds: stamp_bounds,
                rotation: 0.0,
            }
        }
        Tool::Move | Tool::SelectionRect => return None,
    };
    let index = doc.annotations.len();
    doc.annotations.push(ann.clone());
    Some(Command::Insert { index, ann })
}
