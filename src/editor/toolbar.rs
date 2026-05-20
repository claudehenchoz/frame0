use crate::editor::tools::Tool;
use crate::gfx::color::Color;

#[derive(Clone, Copy, Debug)]
pub struct ToolEntry {
    pub tool: Tool,
    pub label: &'static str,
}

pub const TOOLS: &[ToolEntry] = &[
    ToolEntry { tool: Tool::Move, label: "Move" },
    ToolEntry { tool: Tool::SelectionRect, label: "Select" },
    ToolEntry { tool: Tool::Arrow, label: "Arrow" },
    ToolEntry { tool: Tool::ShapeRect, label: "Rect" },
    ToolEntry { tool: Tool::ShapeEllipse, label: "Ellipse" },
    ToolEntry { tool: Tool::Text, label: "Text" },
    ToolEntry { tool: Tool::Step, label: "Step" },
    ToolEntry { tool: Tool::Highlighter, label: "Hilite" },
    ToolEntry { tool: Tool::BlurGaussian, label: "Blur" },
    ToolEntry { tool: Tool::BlurSolid, label: "Redact" },
];

pub const PALETTE: &[Color] = &[
    Color::RED,
    Color::BLUE,
    Color::GREEN,
    Color::YELLOW,
    Color::BLACK,
    Color::WHITE,
];

pub const TOOLBAR_HEIGHT: i32 = 44;
pub const SWATCH_SIZE: i32 = 24;
