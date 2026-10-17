//! The tool rail: a column of icon buttons at the window's left, in four groups.

use egui::{Stroke, Ui, Vec2};
use tools::ToolKind;

use super::{UiAction, View, theme, widgets};

/// The tools in rail order, top to bottom: selecting and painting, drawing, modifying, measuring.
pub const GROUPS: [&[ToolKind]; 4] = [
    &[ToolKind::Select, ToolKind::Erase, ToolKind::Paint],
    &[
        ToolKind::Line,
        ToolKind::Rectangle,
        ToolKind::Circle,
        ToolKind::Arc,
    ],
    &[ToolKind::PushPull, ToolKind::Move, ToolKind::Rotate],
    &[ToolKind::TapeMeasure],
];

pub(super) fn rail(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    let t = theme::tokens(ui.ctx().theme());
    ui.spacing_mut().item_spacing.y = 4.0;
    for (i, group) in GROUPS.iter().enumerate() {
        if i > 0 {
            ui.add_space(theme::RAIL_GROUP_GAP / 2.0);
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(theme::RAIL_BUTTON, 1.0), egui::Sense::hover());
            ui.painter()
                .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, t.border));
            ui.add_space(theme::RAIL_GROUP_GAP / 2.0);
        }
        for &kind in *group {
            if widgets::tool_button(ui, kind, view.tool == kind).clicked() {
                actions.push(UiAction::Tool(kind));
            }
        }
    }
}
