//! The Agent Activity, Keyboard Shortcuts and Soften Edges windows, and the viewport's
//! right-click menu.

use egui::{RichText, Ui};

use super::{Agents, UiAction, ViewportMenu, widgets};
use crate::menus::{FLIP_AXES, INTERSECT_SCOPES};
use crate::shortcuts::TABLE;
use session::FlipFrame;

/// The width of each of the Keyboard Shortcuts window's two columns, in points.
const SHORTCUT_COLUMN: f32 = 200.0;

/// The groups in each of the Keyboard Shortcuts window's columns: Tools and Camera on the left,
/// the menus' File, Edit and Group on the right. A test checks they cover `GROUPS` once.
const SHORTCUT_COLUMNS: [&[&str]; 2] = [&["Tools", "Camera"], &["File", "Edit", "Group"]];

/// The Keyboard Shortcuts window: every binding of the shortcut table under its group, in two
/// columns of about the same length, scrolling when the window is too short for them.
pub(super) fn shortcuts_window(ui: &mut Ui, actions: &mut Vec<UiAction>) {
    let mut open = true;
    let columns = SHORTCUT_COLUMNS;
    let room = (ui.ctx().content_rect().height() - 80.0).max(120.0);
    let title = "Keyboard Shortcuts";
    let shown = egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_height(room)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ui.ctx().content_rect().center())
        .constrain(true)
        .show(ui.ctx(), |ui| {
            let list = egui::ScrollArea::vertical()
                .max_height(room)
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        for column in columns {
                            ui.vertical(|ui| {
                                ui.set_width(SHORTCUT_COLUMN);
                                for group in column {
                                    shortcut_group(ui, group);
                                }
                            });
                        }
                    });
                });
            widgets::name_scroll_bars(ui.ctx(), list.id, title);
        });
    widgets::name_title_bar(ui.ctx(), &shown, title);
    if !open {
        actions.push(UiAction::ShowShortcuts(false));
    }
}

/// One group of the Keyboard Shortcuts window: its title and a label and key chip per binding.
fn shortcut_group(ui: &mut Ui, group: &str) {
    widgets::section_header(ui, group);
    egui::Grid::new(("shortcuts", group))
        .num_columns(2)
        .show(ui, |ui| {
            for s in TABLE.iter().filter(|s| s.bound.group() == group) {
                ui.label(s.bound.label());
                widgets::chip(ui, &s.chord.text());
                ui.end_row();
            }
        });
}

/// The Agent Activity window: how many agents are connected, the recovery files their
/// discards left, and what they asked for, newest first.
pub(super) fn agent_activity(
    ui: &mut Ui,
    agents: &Agents,
    log: &[String],
    actions: &mut Vec<UiAction>,
) {
    let mut open = true;
    let title = "Agent Activity";
    let shown = egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .default_width(420.0)
        .show(ui.ctx(), |ui| {
            ui.label(match agents.connected {
                0 => "No agents connected".to_string(),
                1 => "1 agent connected".to_string(),
                n => format!("{n} agents connected"),
            });
            if !agents.recovered.is_empty() {
                ui.label(
                    RichText::new("Unsaved changes agents discarded were kept in:")
                        .color(ui.visuals().warn_fg_color),
                );
                for path in agents.recovered {
                    ui.monospace(path.display().to_string());
                }
            }
            ui.separator();
            let list = egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    if log.is_empty() {
                        ui.label(RichText::new("Nothing yet").weak());
                    }
                    for line in log {
                        ui.monospace(line);
                    }
                });
            widgets::name_scroll_bars(ui.ctx(), list.id, "Agent requests");
        });
    widgets::name_title_bar(ui.ctx(), &shown, title);
    if !open {
        actions.push(UiAction::ShowAgentActivity(false));
    }
}

/// The Soften Edges window: an angle and the Soften and Harden buttons, which act on the
/// selection, or on the open context when nothing is selected.
pub(super) fn soften_window(ui: &mut Ui, angle: u8, actions: &mut Vec<UiAction>) {
    let mut open = true;
    let title = "Soften Edges";
    let shown = egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ui.ctx().content_rect().center())
        .show(ui.ctx(), |ui| {
            let mut value = angle;
            ui.horizontal(|ui| {
                let label = ui.label("Angle");
                if widgets::slider_row(ui, label.id, "Angle", &mut value, 0..=180, 1.0, |b| {
                    b.suffix("°")
                })
                .changed()
                {
                    actions.push(UiAction::SetSoftenAngle(value));
                }
            });
            ui.label("Edges between faces meeting at up to this angle");
            ui.horizontal(|ui| {
                if ui.button("Soften").clicked() {
                    actions.push(UiAction::Soften);
                }
                if ui.button("Harden").clicked() {
                    actions.push(UiAction::Harden);
                }
            });
            ui.label(RichText::new("Selection, or everything open when nothing is").weak());
        });
    widgets::name_title_bar(ui.ctx(), &shown, title);
    if !open {
        actions.push(UiAction::ShowSoften(false));
    }
}

/// The Check Interference window: the pairs found with their overlap volumes, each a button
/// that selects both parts and zooms to them, or "No interference".
pub(super) fn interference_window(
    ui: &mut Ui,
    report: &crate::editor::InterferenceReport,
    actions: &mut Vec<UiAction>,
) {
    let mut open = true;
    let title = "Check Interference";
    let shown = egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ui.ctx().content_rect().center())
        .show(ui.ctx(), |ui| {
            if report.rows.is_empty() {
                ui.label("No interference");
            }
            let list = egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for (i, row) in report.rows.iter().enumerate() {
                        if ui.button(&row.label).clicked() {
                            actions.push(UiAction::InterferencePair(i));
                        }
                    }
                });
            widgets::name_scroll_bars(ui.ctx(), list.id, "Interfering pairs");
            if report.skipped > 0 {
                ui.label(RichText::new(format!("{} not solid, skipped", report.skipped)).weak());
            }
        });
    widgets::name_title_bar(ui.ctx(), &shown, title);
    if !open {
        actions.push(UiAction::CloseInterference);
    }
}

/// The Model Health window: each part that is not a solid with its problems, a button that
/// selects the part and zooms to it, and Make Solid (enabled when a listed part can be closed);
/// or that every part checked is a solid.
pub(super) fn health_window(
    ui: &mut Ui,
    report: &crate::editor::HealthReport,
    actions: &mut Vec<UiAction>,
) {
    let mut open = true;
    let title = "Model Health";
    let shown = egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ui.ctx().content_rect().center())
        .show(ui.ctx(), |ui| {
            if report.rows.is_empty() {
                ui.label(match report.checked {
                    0 => "No parts to check".to_string(),
                    1 => "The part is a solid".to_string(),
                    n => format!("All {n} parts are solids"),
                });
            }
            let list = egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for (i, row) in report.rows.iter().enumerate() {
                        if ui.button(&row.label).clicked() {
                            actions.push(UiAction::HealthPart(i));
                        }
                    }
                });
            widgets::name_scroll_bars(ui.ctx(), list.id, "Parts that are not solids");
            if !report.rows.is_empty()
                && ui
                    .add_enabled(report.fixable(), egui::Button::new("Make Solid"))
                    .on_disabled_hover_text("Nothing listed can be closed safely")
                    .clicked()
            {
                actions.push(UiAction::MakeSolid);
            }
        });
    widgets::name_title_bar(ui.ctx(), &shown, title);
    if !open {
        actions.push(UiAction::CloseHealth);
    }
}

/// The viewport's right-click menu, at `menu.at`: for a selected section plane Flip and Active
/// Cut (checked while the plane is the active one), for faces and groups Intersect Faces with
/// the model, the selection or the context, and Hide, Unhide, Reverse Faces and Orient Faces
/// where they apply. It closes with a choice, Escape, or a press
/// elsewhere.
pub(super) fn viewport_menu(ui: &mut Ui, menu: ViewportMenu, actions: &mut Vec<UiAction>) {
    let area = egui::Area::new(egui::Id::new("viewport menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(menu.at)
        .constrain(true)
        .show(ui.ctx(), |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                // egui senses clicks on the area (its layer's id with "move") without naming it.
                widgets::name_egui_widget(
                    ui.ctx(),
                    ui.layer_id().id.with("move"),
                    egui::accesskit::Role::Menu,
                    || "Viewport menu".into(),
                );
                let entries = [
                    (menu.hide, "Hide", UiAction::Hide),
                    (menu.unhide, "Unhide", UiAction::UnhideSelected),
                    (menu.faces, "Reverse Faces", UiAction::ReverseFaces),
                    (
                        menu.faces,
                        "Orient Faces",
                        UiAction::OrientFaces(menu.orient_from),
                    ),
                ];
                for (shown, label, action) in entries {
                    if shown && widgets::menu_item(ui, label, None, true).clicked() {
                        actions.push(action);
                        actions.push(UiAction::CloseViewportMenu);
                    }
                }
                if menu.make_unique && widgets::menu_item(ui, "Make Unique", None, true).clicked() {
                    actions.push(UiAction::MakeUnique);
                    actions.push(UiAction::CloseViewportMenu);
                }
                if menu.flip {
                    ui.weak("Flip Along");
                    for (label, axis) in FLIP_AXES {
                        if widgets::menu_item(ui, label, None, true).clicked() {
                            actions.push(UiAction::Flip(axis, FlipFrame::Axes));
                            actions.push(UiAction::CloseViewportMenu);
                        }
                    }
                    if menu.flip_own {
                        for (label, axis) in FLIP_AXES {
                            let label = format!("Component's {label}");
                            if widgets::menu_item(ui, &label, None, true).clicked() {
                                actions.push(UiAction::Flip(axis, FlipFrame::Own));
                                actions.push(UiAction::CloseViewportMenu);
                            }
                        }
                    }
                }
                if menu.reset_axes && widgets::menu_item(ui, "Reset Axes", None, true).clicked() {
                    actions.push(UiAction::ResetAxes);
                    actions.push(UiAction::CloseViewportMenu);
                }
                if menu.intersect {
                    ui.weak("Intersect Faces");
                    for (label, scope) in INTERSECT_SCOPES {
                        if widgets::menu_item(ui, label, None, true).clicked() {
                            actions.push(UiAction::IntersectFaces(scope));
                            actions.push(UiAction::CloseViewportMenu);
                        }
                    }
                }
                let Some(active) = menu.section else { return };
                if widgets::menu_item(ui, "Flip", None, true).clicked() {
                    actions.push(UiAction::FlipSection);
                    actions.push(UiAction::CloseViewportMenu);
                }
                let button = egui::Button::selectable(active, "Active Cut");
                let response = ui.add(button);
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        true,
                        active,
                        "Active Cut",
                    )
                });
                if response.clicked() {
                    actions.push(UiAction::SetSectionActive(!active));
                    actions.push(UiAction::CloseViewportMenu);
                }
            });
        });
    let escaped = ui.ctx().input(|i| i.key_pressed(egui::Key::Escape));
    let elsewhere = ui.ctx().input(|i| i.pointer.any_pressed())
        && ui
            .ctx()
            .pointer_interact_pos()
            .is_some_and(|p| !area.response.rect.contains(p));
    if escaped || elsewhere {
        actions.push(UiAction::CloseViewportMenu);
    }
}

#[cfg(test)]
mod tests {
    use super::SHORTCUT_COLUMNS;
    use crate::shortcuts::GROUPS;

    #[test]
    fn the_columns_cover_every_group_once() {
        let mut listed: Vec<&str> = SHORTCUT_COLUMNS
            .iter()
            .flat_map(|c| c.iter().copied())
            .collect();
        listed.sort_unstable();
        let mut all = GROUPS.to_vec();
        all.sort_unstable();
        assert_eq!(listed, all);
    }
}
