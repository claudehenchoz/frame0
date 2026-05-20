use crate::editor::annotation::{Annotation, BlurMode, Color, Point, Rect, ShapeKind};
use crate::editor::history::Command;
use crate::editor::Document;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tool {
    Move,
    SelectionRect,
    Arrow,
    ShapeRect,
    ShapeEllipse,
    Text,
    Step,
    Highlighter,
    BlurGaussian,
    BlurSolid,
}

#[derive(Clone, Copy, Debug)]
pub struct ToolStyle {
    pub color: Color,
    pub stroke: f32,
    pub font_size: f32,
    pub fill_alpha: u8, // 0 means transparent fill
}

impl Default for ToolStyle {
    fn default() -> Self {
        Self {
            color: Color::RED,
            stroke: 4.0,
            font_size: 20.0,
            fill_alpha: 0,
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
        },
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
                intensity: 12.0,
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
        Tool::Move | Tool::SelectionRect => return None,
    };
    let index = doc.annotations.len();
    doc.annotations.push(ann.clone());
    Some(Command::Insert { index, ann })
}
