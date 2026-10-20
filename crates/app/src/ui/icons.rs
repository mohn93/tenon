//! The Phosphor glyphs the panels use, drawn in `fonts::icons()`.

use egui_phosphor::regular as ph;
use tools::ToolKind;

pub const UNDO: &str = ph::ARROW_U_UP_LEFT;
pub const REDO: &str = ph::ARROW_U_UP_RIGHT;
/// Make Group, Make Component and Explode.
pub const BOUNDING_BOX: &str = ph::BOUNDING_BOX;
pub const CUBE: &str = ph::CUBE;
pub const ARROWS_OUT: &str = ph::ARROWS_OUT;
/// The day/night toggle by day and at night.
pub const SUN: &str = ph::SUN;
pub const MOON: &str = ph::MOON;
/// The agents indicator.
pub const ROBOT: &str = ph::ROBOT;
/// Between the breadcrumb's crumbs.
pub const CARET_RIGHT: &str = ph::CARET_RIGHT;
/// Clears the Outliner's filter.
pub const X_CIRCLE: &str = ph::X_CIRCLE;
/// Dismisses the start card.
pub const X: &str = ph::X;

/// A tool's glyph on the rail.
pub fn tool(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Select => ph::CURSOR,
        ToolKind::Erase => ph::ERASER,
        ToolKind::Paint => ph::PAINT_BUCKET,
        ToolKind::Line => ph::LINE_SEGMENT,
        ToolKind::Rectangle => ph::RECTANGLE,
        ToolKind::Circle => ph::CIRCLE,
        ToolKind::Polygon => ph::HEXAGON,
        ToolKind::Arc => ph::BEZIER_CURVE,
        ToolKind::PushPull => ph::ARROW_FAT_LINES_UP,
        ToolKind::Move => ph::ARROWS_OUT_CARDINAL,
        ToolKind::Rotate => ph::ARROW_CLOCKWISE,
        ToolKind::Scale => ph::ARROWS_OUT,
        ToolKind::Offset => ph::FRAME_CORNERS,
        ToolKind::FollowMe => ph::PATH,
        ToolKind::TapeMeasure => ph::RULER,
    }
}

/// Every glyph the panels draw: the constants above, then each tool's.
#[cfg(test)]
pub fn all() -> Vec<&'static str> {
    let mut all = vec![
        UNDO,
        REDO,
        BOUNDING_BOX,
        CUBE,
        ARROWS_OUT,
        SUN,
        MOON,
        ROBOT,
        CARET_RIGHT,
        X_CIRCLE,
        X,
    ];
    all.extend(ToolKind::ALL.map(tool));
    all
}
