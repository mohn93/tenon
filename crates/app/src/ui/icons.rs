//! The Phosphor glyphs the panels use, drawn in `fonts::icons()`.

use egui_phosphor::regular as ph;
use tools::ToolKind;

pub const UNDO: &str = ph::ARROW_U_UP_LEFT;
pub const REDO: &str = ph::ARROW_U_UP_RIGHT;
/// Make Group, Make Component and Explode.
pub const BOUNDING_BOX: &str = ph::BOUNDING_BOX;
pub const CUBE: &str = ph::CUBE;
/// Save to Library.
pub const BOOKMARK: &str = ph::BOOKMARK_SIMPLE;
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
/// A shown and a hidden tag's visibility button.
pub const EYE: &str = ph::EYE;
pub const EYE_SLASH: &str = ph::EYE_SLASH;
/// Adds and deletes a tag.
pub const PLUS: &str = ph::PLUS;
pub const MINUS: &str = ph::MINUS;
/// Updates the selected scene and moves it up or down the Scenes list.
pub const ARROWS_CLOCKWISE: &str = ph::ARROWS_CLOCKWISE;
pub const ARROW_UP: &str = ph::ARROW_UP;
pub const ARROW_DOWN: &str = ph::ARROW_DOWN;

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
        ToolKind::Dimension => ph::ARROWS_HORIZONTAL,
        ToolKind::Text => ph::TEXT_T,
        ToolKind::SectionPlane => ph::SQUARE_HALF,
        ToolKind::Axes => ph::CROSSHAIR_SIMPLE,
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
        BOOKMARK,
        ARROWS_OUT,
        SUN,
        MOON,
        ROBOT,
        CARET_RIGHT,
        X_CIRCLE,
        X,
        EYE,
        EYE_SLASH,
        PLUS,
        MINUS,
        ARROWS_CLOCKWISE,
        ARROW_UP,
        ARROW_DOWN,
    ];
    all.extend(ToolKind::ALL.map(tool));
    all
}
