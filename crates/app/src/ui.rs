//! The window's panels: toolbar, status bar with the measurements box, and Entity Info.

use egui::{Align, Layout, Panel, RichText, Ui};
use session::{Projection, StandardView};
use tools::ToolKind;

use crate::editor::Status;

/// What the user asked for through the panels this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    Tool(ToolKind),
    Undo,
    Redo,
    EraseSelection,
    MakeGroup,
    MakeComponent,
    Explode,
    CloseInstance,
    NewModel,
    OpenFile,
    SaveFile,
    SaveFileAs,
    ExportFile,
    DeleteGuides,
    ZoomExtents,
    StandardView(StandardView),
    SetProjection(Projection),
}

/// What the panels show.
pub struct View<'a> {
    pub tool: ToolKind,
    pub status: &'a Status,
    pub info: &'a [String],
    pub has_selection: bool,
    /// Whether a group or component is selected.
    pub has_instance_selected: bool,
    /// Whether a group or component is open for editing.
    pub inside_instance: bool,
    /// Whether the open context has guides to delete.
    pub has_guides: bool,
    /// The camera's projection, checked in the Camera menu.
    pub projection: Projection,
}

/// Lays out the panels for one frame. Returns the actions clicked and the rectangle left for
/// the 3D view, in points.
pub fn draw(ui: &mut Ui, view: &View) -> (Vec<UiAction>, egui::Rect) {
    let mut actions = Vec::new();
    Panel::top("toolbar").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.menu_button("File", |ui| {
                let items = [
                    ("New", "Cmd+N", UiAction::NewModel),
                    ("Open...", "Cmd+O", UiAction::OpenFile),
                    ("Save", "Cmd+S", UiAction::SaveFile),
                    ("Save As...", "Shift+Cmd+S", UiAction::SaveFileAs),
                    ("Export...", "Shift+Cmd+E", UiAction::ExportFile),
                ];
                for (label, key, action) in items {
                    if ui
                        .add(egui::Button::new(label).shortcut_text(key))
                        .clicked()
                    {
                        actions.push(action);
                    }
                }
            });
            ui.menu_button("Edit", |ui| {
                let erase = egui::Button::new("Erase Selection").shortcut_text("Delete");
                if ui.add_enabled(view.has_selection, erase).clicked() {
                    actions.push(UiAction::EraseSelection);
                }
                if ui
                    .add_enabled(view.has_guides, egui::Button::new("Delete Guides"))
                    .clicked()
                {
                    actions.push(UiAction::DeleteGuides);
                }
            });
            ui.menu_button("Camera", |ui| {
                if ui
                    .add(egui::Button::new("Zoom Extents").shortcut_text("Shift+Z"))
                    .clicked()
                {
                    actions.push(UiAction::ZoomExtents);
                }
                ui.separator();
                for v in StandardView::ALL {
                    if ui.button(v.label()).clicked() {
                        actions.push(UiAction::StandardView(v));
                    }
                }
                ui.separator();
                for (projection, label) in [
                    (Projection::Perspective, "Perspective"),
                    (Projection::Parallel, "Parallel Projection"),
                ] {
                    if ui
                        .selectable_label(view.projection == projection, label)
                        .clicked()
                    {
                        actions.push(UiAction::SetProjection(projection));
                    }
                }
            });
            ui.separator();
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
            ui.separator();
            let buttons = [
                (view.has_selection, "Make group", "G", UiAction::MakeGroup),
                (
                    view.has_selection,
                    "Make component",
                    "Shift+G",
                    UiAction::MakeComponent,
                ),
                (view.has_instance_selected, "Explode", "", UiAction::Explode),
                (
                    view.inside_instance,
                    "Close group",
                    "Escape",
                    UiAction::CloseInstance,
                ),
            ];
            for (enabled, label, key, action) in buttons {
                let mut button = ui.add_enabled(enabled, egui::Button::new(label));
                if !key.is_empty() {
                    button = button.on_hover_text(key);
                }
                if button.clicked() {
                    actions.push(action);
                }
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

    /// The window's default size (see `app.rs`); the toolbar needs about 1200 points.
    fn screen() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
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

    /// One frame with a primary click (press, then release in the next frame) at `pos`.
    fn click(ctx: &egui::Context, view: &View, pos: Pos2) -> Vec<UiAction> {
        let pointer = |pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let mut press = screen();
        press.events = vec![egui::Event::PointerMoved(pos), pointer(true)];
        let mut actions = run(ctx, press, view).0;
        let mut release = screen();
        release.events = vec![pointer(false)];
        actions.extend(run(ctx, release, view).0);
        actions
    }

    #[test]
    fn the_file_menu_reports_its_items() {
        let ctx = egui::Context::default();
        let status = status();
        let info = Vec::new();
        let view = View {
            tool: ToolKind::Line,
            status: &status,
            info: &info,
            has_selection: false,
            has_instance_selected: false,
            inside_instance: false,
            has_guides: false,
            projection: Projection::Perspective,
        };
        let (_, central) = run(&ctx, screen(), &view);
        let file = Pos2::new(14.0, central.min.y / 2.0);
        let mut clicked = Vec::new();
        // Open the menu, then click one spot below it; repeat down the menu.
        for y in (0..160).step_by(6) {
            click(&ctx, &view, file);
            let _ = run(&ctx, screen(), &view);
            clicked.extend(click(
                &ctx,
                &view,
                Pos2::new(40.0, central.min.y + y as f32),
            ));
            // Close the menu if the click missed every item.
            let mut escape = screen();
            escape.events = vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }];
            let _ = run(&ctx, escape, &view);
        }
        for action in [
            UiAction::NewModel,
            UiAction::OpenFile,
            UiAction::SaveFile,
            UiAction::SaveFileAs,
            UiAction::ExportFile,
        ] {
            assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
        }
    }

    /// The actions produced by opening the menu under each x in `xs` on the toolbar row and
    /// clicking one spot below it, down the menu.
    fn menu_actions(view: &View, xs: std::ops::Range<i32>) -> Vec<UiAction> {
        let ctx = egui::Context::default();
        let (_, central) = run(&ctx, screen(), view);
        let row = central.min.y / 2.0;
        let mut clicked = Vec::new();
        for x in xs.step_by(12) {
            for y in (0..300).step_by(8) {
                click(&ctx, view, Pos2::new(x as f32, row));
                let _ = run(&ctx, screen(), view);
                clicked.extend(click(
                    &ctx,
                    view,
                    Pos2::new(x as f32 + 10.0, central.min.y + y as f32),
                ));
                let mut escape = screen();
                escape.events = vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }];
                let _ = run(&ctx, escape, view);
            }
        }
        clicked
    }

    #[test]
    fn the_camera_menu_has_views_zoom_extents_and_projections() {
        let status = status();
        let info = Vec::new();
        let mut view = View {
            tool: ToolKind::Line,
            status: &status,
            info: &info,
            has_selection: false,
            has_instance_selected: false,
            inside_instance: false,
            has_guides: false,
            projection: Projection::Perspective,
        };
        // The Camera menu sits right after File and Edit.
        let clicked = menu_actions(&view, 80..140);
        assert!(clicked.contains(&UiAction::ZoomExtents), "{clicked:?}");
        for v in StandardView::ALL {
            assert!(
                clicked.contains(&UiAction::StandardView(v)),
                "{v:?} in {clicked:?}"
            );
        }
        for p in [Projection::Perspective, Projection::Parallel] {
            assert!(
                clicked.contains(&UiAction::SetProjection(p)),
                "{p:?} in {clicked:?}"
            );
        }
        view.projection = Projection::Parallel;
        let clicked = menu_actions(&view, 80..140);
        assert!(clicked.contains(&UiAction::SetProjection(Projection::Perspective)));
    }

    fn edit_menu_actions(has_selection: bool, has_guides: bool) -> Vec<UiAction> {
        let status = status();
        let info = Vec::new();
        let view = View {
            tool: ToolKind::Line,
            status: &status,
            info: &info,
            has_selection,
            has_instance_selected: false,
            inside_instance: false,
            has_guides,
            projection: Projection::Perspective,
        };
        // The Edit button sits right after File (which ends before x = 40).
        menu_actions(&view, 40..80)
    }

    #[test]
    fn the_edit_menu_erases_the_selection_and_deletes_guides() {
        let clicked = edit_menu_actions(true, true);
        assert!(clicked.contains(&UiAction::EraseSelection), "{clicked:?}");
        assert!(clicked.contains(&UiAction::DeleteGuides), "{clicked:?}");
    }

    #[test]
    fn the_edit_menu_items_need_a_selection_and_guides() {
        let clicked = edit_menu_actions(false, false);
        assert!(!clicked.contains(&UiAction::EraseSelection), "{clicked:?}");
        assert!(!clicked.contains(&UiAction::DeleteGuides), "{clicked:?}");
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
            has_instance_selected: false,
            inside_instance: false,
            has_guides: false,
            projection: Projection::Perspective,
        };
        let (actions, rect) = run(&ctx, screen(), &view);
        assert!(actions.is_empty());
        assert!(
            rect.min.x == 0.0 && rect.min.y > 0.0,
            "toolbar above: {rect:?}"
        );
        assert!(rect.max.x < 1280.0, "info panel on the right: {rect:?}");
        assert!(rect.max.y < 800.0, "status bar below: {rect:?}");
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
            has_instance_selected: true,
            inside_instance: true,
            has_guides: true,
            projection: Projection::Perspective,
        };
        let (_, central) = run(&ctx, screen(), &view);
        let row = central.min.y / 2.0;
        // Click along the toolbar row until a press lands on the Rectangle button.
        let mut clicked = Vec::new();
        for x in (2..1280).step_by(4) {
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
        for kind in [ToolKind::Rectangle, ToolKind::Arc] {
            assert!(clicked.contains(&UiAction::Tool(kind)), "{clicked:?}");
        }
        assert!(clicked.contains(&UiAction::Undo));
        for action in [
            UiAction::MakeGroup,
            UiAction::MakeComponent,
            UiAction::Explode,
            UiAction::CloseInstance,
        ] {
            assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
        }
    }
}
