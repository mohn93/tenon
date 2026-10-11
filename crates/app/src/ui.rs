//! The window's panels: toolbar, status bar with the measurements box, and Entity Info.

use egui::{Align, Layout, Panel, RichText, Ui};
use tools::ToolKind;

use crate::editor::Status;

/// What the user asked for through the panels this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    Tool(ToolKind),
    Undo,
    Redo,
    EraseSelection,
}

/// What the panels show.
pub struct View<'a> {
    pub tool: ToolKind,
    pub status: &'a Status,
    pub info: &'a [String],
    pub has_selection: bool,
}

/// Lays out the panels for one frame. Returns the actions clicked and the rectangle left for
/// the 3D view, in points.
pub fn draw(ui: &mut Ui, view: &View) -> (Vec<UiAction>, egui::Rect) {
    let mut actions = Vec::new();
    Panel::top("toolbar").show(ui, |ui| {
        ui.horizontal(|ui| {
            for kind in ToolKind::ALL {
                let key = match kind.shortcut() {
                    ' ' => "Space".to_string(),
                    c => c.to_ascii_uppercase().to_string(),
                };
                let button = ui
                    .selectable_label(view.tool == kind, kind.label())
                    .on_hover_text(format!("{} ({key})", kind.label()));
                if button.clicked() {
                    actions.push(UiAction::Tool(kind));
                }
            }
            ui.separator();
            if ui.button("Undo").on_hover_text("Cmd+Z").clicked() {
                actions.push(UiAction::Undo);
            }
            if ui.button("Redo").on_hover_text("Shift+Cmd+Z").clicked() {
                actions.push(UiAction::Redo);
            }
            let erase = ui
                .add_enabled(view.has_selection, egui::Button::new("Erase selection"))
                .on_hover_text("Delete");
            if erase.clicked() {
                actions.push(UiAction::EraseSelection);
            }
        });
    });
    Panel::bottom("status").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(view.status.prompt);
            if let Some(lock) = view.status.lock {
                ui.label(RichText::new(lock).strong());
            }
            if let Some(message) = &view.status.message {
                ui.label(RichText::new(message).color(ui.visuals().warn_fg_color));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // The measurements box: typed text, or the tool's live measurement.
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_width(120.0);
                    ui.monospace(view.status.value.clone().unwrap_or_default());
                });
                ui.label(view.status.label);
            });
        });
    });
    Panel::right("entity_info")
        .resizable(false)
        .default_size(180.0)
        .show(ui, |ui| {
            ui.heading("Entity Info");
            for line in view.info {
                ui.label(line);
            }
        });
    (actions, ui.available_rect_before_wrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect, Vec2};

    fn run(ctx: &egui::Context, input: egui::RawInput, view: &View) -> (Vec<UiAction>, Rect) {
        let mut out = None;
        ctx.run_ui(input, |ui| out = Some(draw(ui, view)))
            .drop_without_applying_deltas();
        out.unwrap()
    }

    fn screen() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
            ..Default::default()
        }
    }

    fn status() -> Status {
        Status {
            tool: "Line",
            prompt: "Click to start a line",
            label: "Length",
            value: Some("2.5_".into()),
            message: Some("Not a length: x".into()),
            lock: None,
        }
    }

    #[test]
    fn panels_leave_a_central_view_inside_the_window() {
        let ctx = egui::Context::default();
        let status = status();
        let info = vec!["Nothing selected".to_string()];
        let view = View {
            tool: ToolKind::Line,
            status: &status,
            info: &info,
            has_selection: false,
        };
        let (actions, rect) = run(&ctx, screen(), &view);
        assert!(actions.is_empty());
        assert!(
            rect.min.x == 0.0 && rect.min.y > 0.0,
            "toolbar above: {rect:?}"
        );
        assert!(rect.max.x < 1000.0, "info panel on the right: {rect:?}");
        assert!(rect.max.y < 700.0, "status bar below: {rect:?}");
        assert!(rect.width() > 600.0 && rect.height() > 500.0, "{rect:?}");
    }

    #[test]
    fn clicking_a_tool_button_reports_it() {
        let ctx = egui::Context::default();
        let status = status();
        let info = Vec::new();
        let view = View {
            tool: ToolKind::Line,
            status: &status,
            info: &info,
            has_selection: true,
        };
        let (_, central) = run(&ctx, screen(), &view);
        let row = central.min.y / 2.0;
        // Click along the toolbar row until a press lands on the Rectangle button.
        let mut clicked = Vec::new();
        for x in (2..600).step_by(4) {
            let pos = Pos2::new(x as f32, row);
            let pointer = |pressed: bool| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            let mut press = screen();
            press.events = vec![egui::Event::PointerMoved(pos), pointer(true)];
            let _ = run(&ctx, press, &view);
            let mut release = screen();
            release.events = vec![pointer(false)];
            clicked.extend(run(&ctx, release, &view).0);
        }
        assert!(
            clicked.contains(&UiAction::Tool(ToolKind::Rectangle)),
            "{clicked:?}"
        );
        assert!(clicked.contains(&UiAction::Undo));
        assert!(clicked.contains(&UiAction::EraseSelection));
    }
}
