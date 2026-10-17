//! The Phosphor glyphs the panels use.

use egui_phosphor::regular as ph;
use tools::ToolKind;

pub const UNDO: &str = ph::ARROW_U_UP_LEFT;
pub const REDO: &str = ph::ARROW_U_UP_RIGHT;

/// A tool's glyph on the rail.
pub fn tool(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Select => ph::CURSOR,
        ToolKind::Erase => ph::ERASER,
        ToolKind::Paint => ph::PAINT_BUCKET,
        ToolKind::Line => ph::LINE_SEGMENT,
        ToolKind::Rectangle => ph::RECTANGLE,
        ToolKind::Circle => ph::CIRCLE,
        ToolKind::Arc => ph::BEZIER_CURVE,
        ToolKind::PushPull => ph::ARROW_FAT_LINES_UP,
        ToolKind::Move => ph::ARROWS_OUT_CARDINAL,
        ToolKind::Rotate => ph::ARROW_CLOCKWISE,
        ToolKind::TapeMeasure => ph::RULER,
    }
}
