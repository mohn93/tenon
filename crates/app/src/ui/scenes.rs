//! The Scenes tab: Add, Delete, Update and reordering of the selected scene, a row per scene
//! (a click goes to it, a double-click renames it) and what the selected scene stores. The
//! inline rename field the scene tabs share.

use egui::text::{CCursor, CCursorRange};
use egui::{Id, RichText, Ui, Vec2};
use kernel::PageId;

use super::widgets;
use super::{SceneRow, UiAction, View, icons};

/// Where a scene's name is shown and renamed: the scene tabs or the Scenes tab. Each keeps its
/// own rename in egui's memory.
#[derive(Clone, Copy)]
pub(super) enum Place {
    Tabs,
    Panel,
}

impl Place {
    fn salt(self) -> &'static str {
        match self {
            Place::Tabs => "scene-tabs",
            Place::Panel => "scenes-panel",
        }
    }
}

/// A scene name being typed: the scene, the text, and whether the field has yet to take focus.
type Renaming = (PageId, String, bool);

/// The egui memory key of the name being typed in `place`.
fn rename_key(place: Place) -> Id {
    Id::new(("scene-rename", place.salt()))
}

/// The egui memory key of the document count (`View::documents`) `place` last drew.
fn document_key(place: Place) -> Id {
    Id::new(("scenes-document", place.salt()))
}

/// Starts renaming `row` in `place`: its field shows from the next frame, with the name
/// selected.
pub(super) fn start_rename(ui: &Ui, place: Place, row: &SceneRow) {
    let renaming: Renaming = (row.id, row.name.clone(), true);
    ui.data_mut(|d| d.insert_temp(rename_key(place), renaming));
}

/// Forgets a name being typed in `place` for a scene no longer listed (deleted, or the model
/// replaced: after new or open, its id may name another scene).
pub(super) fn forget_gone(ui: &Ui, place: Place, view: &View) {
    let seen = ui.data(|d| d.get_temp::<u64>(document_key(place)));
    ui.data_mut(|d| d.insert_temp(document_key(place), view.documents));
    let replaced = seen.is_some_and(|n| n != view.documents);
    let renaming = ui.data(|d| d.get_temp::<Renaming>(rename_key(place)));
    if let Some((id, ..)) = renaming
        && (replaced || !view.scenes.iter().any(|r| r.id == id))
    {
        ui.data_mut(|d| d.remove::<Renaming>(rename_key(place)));
    }
}

/// While `row` is renamed in `place`, its name field, `width` wide (Enter or leaving it
/// renames once; Escape keeps the name). False when it is not being renamed.
pub(super) fn rename_field(
    ui: &mut Ui,
    place: Place,
    row: &SceneRow,
    width: f32,
    actions: &mut Vec<UiAction>,
) -> bool {
    let key = rename_key(place);
    let Some((_, mut text, fresh)) = ui
        .data(|d| d.get_temp::<Renaming>(key))
        .filter(|(id, ..)| *id == row.id)
    else {
        return false;
    };
    let field_id = Id::new(("scene-name", place.salt(), row.id.0));
    let output = egui::TextEdit::singleline(&mut text)
        .id(field_id)
        .desired_width(width)
        .show(ui);
    let edit = &output.response.response;
    edit.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Scene name");
        info.current_text_value = Some(text.clone());
        info
    });
    if fresh {
        // The field's first frame: it takes focus with the whole name selected.
        edit.request_focus();
        let mut state = output.state.clone();
        let all = CCursorRange::two(CCursor::new(0), CCursor::new(text.chars().count()));
        state.cursor.set_char_range(Some(all));
        state.store(ui.ctx(), field_id);
        ui.data_mut(|d| d.insert_temp(key, (row.id, text, false)));
    } else if edit.lost_focus() || !edit.has_focus() {
        let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if edit.lost_focus() && !escaped && text != row.name {
            actions.push(UiAction::RenameScene(row.id, text));
        }
        ui.data_mut(|d| d.remove::<Renaming>(key));
    } else {
        ui.data_mut(|d| d.insert_temp(key, (row.id, text, false)));
    }
    true
}

/// The Scenes tab (the tab is its title); the dock's top area scrolls it.
pub(super) fn scenes(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    forget_gone(ui, Place::Panel, view);
    let count = view.scenes.len();
    let selected = view.scenes.iter().position(|r| r.selected);
    let chosen = selected.map(|i| &view.scenes[i]);
    ui.horizontal(|ui| {
        let button = |ui: &mut Ui, icon, label, tip, enabled| {
            widgets::icon_button(ui, icon, label, tip, None, enabled).clicked()
        };
        if button(
            ui,
            icons::PLUS,
            "Add scene",
            "Save the view and tag visibility as a scene",
            true,
        ) {
            actions.push(UiAction::AddScene);
        }
        let some = chosen.is_some();
        if button(
            ui,
            icons::MINUS,
            "Delete scene",
            "Delete the selected scene",
            some,
        ) && let Some(row) = chosen
        {
            actions.push(UiAction::DeleteScene(row.id));
        }
        if button(
            ui,
            icons::ARROWS_CLOCKWISE,
            "Update scene",
            "Store the view and tag visibility in the selected scene again",
            some,
        ) && let Some(row) = chosen
        {
            actions.push(UiAction::UpdateScene(row.id));
        }
        let up = selected.is_some_and(|i| i > 0);
        if button(
            ui,
            icons::ARROW_UP,
            "Move scene up",
            "Move the selected scene up the list",
            up,
        ) && let (Some(row), Some(i)) = (chosen, selected)
        {
            actions.push(UiAction::MoveScene(row.id, i - 1));
        }
        let down = selected.is_some_and(|i| i + 1 < count);
        if button(
            ui,
            icons::ARROW_DOWN,
            "Move scene down",
            "Move the selected scene down the list",
            down,
        ) && let (Some(row), Some(i)) = (chosen, selected)
        {
            actions.push(UiAction::MoveScene(row.id, i + 1));
        }
    });
    if view.scenes.is_empty() {
        ui.label(RichText::new("No scenes").weak());
    }
    for row in view.scenes {
        scene_row(ui, row, actions);
    }
    ui.separator();
    stores(ui, chosen, actions);
}

/// A scene's row: its name, highlighted while it is the selected scene (a click goes to the
/// scene, a double-click renames it), or while it is renamed a field.
fn scene_row(ui: &mut Ui, row: &SceneRow, actions: &mut Vec<UiAction>) {
    let width = ui.available_width();
    if rename_field(ui, Place::Panel, row, width, actions) {
        return;
    }
    let response = ui.add(
        egui::Button::selectable(row.selected, &row.name)
            .truncate()
            .min_size(Vec2::new(width, 0.0)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, row.selected, &row.name)
    });
    if response.double_clicked() {
        start_rename(ui, Place::Panel, row);
    } else if response.clicked() {
        actions.push(UiAction::GoToScene(row.id));
    }
}

/// What the selected scene stores: the Camera, Tag visibility, Positions and Joint values checkboxes (off and disabled
/// with no scene selected). Ticking one stores that part from now; unticking drops it.
fn stores(ui: &mut Ui, chosen: Option<&SceneRow>, actions: &mut Vec<UiAction>) {
    ui.label(RichText::new("The selected scene stores").weak());
    let enabled = chosen.is_some();
    let mut camera = chosen.is_some_and(|r| r.camera);
    let response = ui
        .add_enabled(enabled, egui::Checkbox::new(&mut camera, "Camera"))
        .on_hover_text("The view: camera, projection, lighting and color by tag");
    if response.changed()
        && let Some(row) = chosen
    {
        actions.push(UiAction::StoreSceneCamera(row.id, camera));
    }
    let mut tags = chosen.is_some_and(|r| r.tags);
    let response = ui
        .add_enabled(enabled, egui::Checkbox::new(&mut tags, "Tag visibility"))
        .on_hover_text("Which tags are hidden");
    if response.changed()
        && let Some(row) = chosen
    {
        actions.push(UiAction::StoreSceneTags(row.id, tags));
    }
    let mut positions = chosen.is_some_and(|r| r.positions);
    let response = ui
        .add_enabled(enabled, egui::Checkbox::new(&mut positions, "Positions"))
        .on_hover_text(
            "Where instances are: the selected instances, or all of them when none is selected",
        );
    if response.changed()
        && let Some(row) = chosen
    {
        actions.push(UiAction::StoreScenePositions(row.id, positions));
    }
    let mut joints = chosen.is_some_and(|r| r.joints);
    let response = ui
        .add_enabled(enabled, egui::Checkbox::new(&mut joints, "Joint values"))
        .on_hover_text("The value of every joint: going to the scene drives them");
    if response.changed()
        && let Some(row) = chosen
    {
        actions.push(UiAction::StoreSceneJoints(row.id, joints));
    }
}
