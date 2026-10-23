//! Entity Info: what is selected, its tag, the Soft checkbox for edges, the Hidden checkbox and
//! the texture position for faces.

use egui::Ui;
use kernel::{DefinitionId, EntityId};

use super::widgets::{self, take_row};
use super::{LibrarySave, TexturePosition, UiAction, View, tags};
use crate::editor::RenameTarget;

/// Entity Info, the dock's first tab (the tab is its title).
pub(super) fn entity_info(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    match view.rename {
        // The first line ("1 component" or "1 group"), the Name field, then the rest.
        Some(target) => {
            let (first, rest) = view
                .info
                .split_first()
                .map_or((None, &[][..]), |(f, r)| (Some(f), r));
            if let Some(first) = first {
                ui.label(first);
            }
            name_field(ui, target, actions);
            for line in rest {
                ui.label(line);
            }
        }
        None => {
            // The field went away (the selection changed): a name being typed is dropped.
            ui.data_mut(|d| d.remove::<(DefinitionId, String)>(egui::Id::new(NAME_KEY)));
            for line in view.info {
                ui.label(line);
            }
        }
    }
    if let Some(instance) = view.component.filter(|_| view.features.library) {
        let enabled = view.library.folder.is_some();
        let button = ui.add_enabled(enabled, egui::Button::new("Save to Library"));
        let button = if enabled {
            button
        } else {
            button.on_disabled_hover_text("Choose a library folder in Settings > Files first")
        };
        if button.clicked() {
            actions.push(UiAction::SaveToLibrary(LibrarySave::Component(instance)));
        }
    }
    if let Some(current) = view.selection_tag {
        tags::selection_tag(ui, view, current, actions);
    }
    if let Some(soft) = view.edges_soft {
        let mut checked = soft;
        if ui
            .checkbox(&mut checked, "Soft")
            .on_hover_text("Hidden, with the faces beside it shaded as one surface")
            .changed()
        {
            actions.push(UiAction::SetSoft(checked));
        }
    }
    if let Some(hidden) = view.selection_hidden {
        let mut checked = hidden;
        if ui
            .checkbox(&mut checked, "Hidden")
            .on_hover_text("Not drawn or picked; shown ghosted while View > Hidden Geometry is on")
            .changed()
        {
            actions.push(UiAction::SetHidden(checked));
        }
    }
    match &view.texture_position {
        Some(t) => texture_position(ui, t, actions),
        None => end_texture_position_edit(ui, actions),
    }
}

/// The selected group's or component's Name field. The text being typed lives in egui's memory
/// until the field loses focus; then a changed name is reported, unless Escape discarded it or
/// it is blank for a component (which needs a name), when the field goes back to the name.
fn name_field(ui: &mut Ui, target: &RenameTarget, actions: &mut Vec<UiAction>) {
    let key = egui::Id::new(NAME_KEY);
    // A name typed for another definition (the selection changed meanwhile) is dropped.
    let mut text = ui
        .data(|d| d.get_temp::<(DefinitionId, String)>(key))
        .filter(|(d, _)| *d == target.definition)
        .map_or_else(|| target.name.clone(), |(_, t)| t);
    let edit = ui
        .horizontal(|ui| {
            let label = ui.label("Name");
            // Named by its label, which then is not a node of its own.
            ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY))
                .labelled_by(label.id)
        })
        .inner;
    edit.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Name");
        info.current_text_value = Some(text.clone());
        info
    });
    if edit.lost_focus() {
        let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let blank = target.kind == kernel::DefinitionKind::Component && text.trim().is_empty();
        if text != target.name && !escaped && !blank {
            actions.push(UiAction::RenameDefinition(target.definition, text));
        }
        ui.data_mut(|d| d.remove::<(DefinitionId, String)>(key));
    } else if edit.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, (target.definition, text)));
    } else {
        ui.data_mut(|d| d.remove::<(DefinitionId, String)>(key));
    }
}

/// The egui memory key of the name being typed in the Name field, with its definition.
const NAME_KEY: &str = "definition-name";

/// After the texture position fields go away (the selection is no longer only faces) with an
/// edit in progress: a value already taken (dragged, or stepped with the arrow keys) applies to
/// the faces the edit began with; text still being typed is dropped.
fn end_texture_position_edit(ui: &mut Ui, actions: &mut Vec<UiAction>) {
    let faces = ui.data_mut(|d| d.remove_temp::<Vec<EntityId>>(egui::Id::new(FACES_KEY)));
    let offset = take_row::<2>(ui, egui::Id::new(OFFSET_KEY));
    let rotation = take_row::<1>(ui, egui::Id::new(ROTATION_KEY));
    let scale = take_row::<2>(ui, egui::Id::new(SCALE_KEY));
    let Some(faces) = faces else { return };
    if let Some((start, offset)) = offset
        && offset.map(millimeters) != start.map(millimeters)
    {
        actions.push(UiAction::SetTextureOffset(
            faces.clone(),
            offset.map(millimeters),
        ));
    }
    if let Some(([start], [deg])) = rotation
        && degree_tenths(deg) != degree_tenths(start)
    {
        actions.push(UiAction::SetTextureRotation(
            faces.clone(),
            degree_tenths(deg),
        ));
    }
    if let Some((start, scale)) = scale
        && scale.map(thousandths) != start.map(thousandths)
    {
        actions.push(UiAction::SetTextureScale(faces, scale.map(thousandths)));
    }
}

/// egui memory keys of the texture position fields' edits and the faces they apply to.
const FACES_KEY: &str = "texture-position-faces";
const OFFSET_KEY: &str = "texture-offset";
const ROTATION_KEY: &str = "texture-rotation";
const SCALE_KEY: &str = "texture-scale";

/// Meters in whole millimeters.
fn millimeters(m: f64) -> i32 {
    (m * 1000.0).round() as i32
}

/// Degrees in whole tenths.
fn degree_tenths(deg: f64) -> i32 {
    (deg * 10.0).round() as i32
}

/// A scale in whole thousandths.
fn thousandths(s: f64) -> u32 {
    (s * 1000.0).round() as u32
}

/// Entity Info's texture position for selected faces: offset, rotation and scale fields, each
/// applied when a drag ends or a typed value is entered, and the Rotate 90 and Reset buttons.
/// A field applies to the faces selected when its edit began, from the values shown then, even
/// if the selection changed before the value was entered.
fn texture_position(ui: &mut Ui, t: &TexturePosition, actions: &mut Vec<UiAction>) {
    ui.label("Texture position")
        .on_hover_text("Where the texture lies on the selected faces");
    let faces_key = egui::Id::new(FACES_KEY);
    let mut done = Vec::new();
    let mut editing = false;
    let offset_range = -kernel::MAX_TEXTURE_OFFSET..=kernel::MAX_TEXTURE_OFFSET;
    let mm = millimeters;
    let p = &t.placement;
    let row = widgets::numeric_row(
        ui,
        egui::Id::new(OFFSET_KEY),
        "Offset",
        p.offset,
        offset_range,
        0.01,
    );
    editing |= row.editing;
    if let Some((start, offset)) = row.done
        && offset.map(mm) != start.map(mm)
    {
        done.push(Field::Offset(offset.map(mm)));
    }
    let tenths = degree_tenths;
    let row = widgets::numeric_row(
        ui,
        egui::Id::new(ROTATION_KEY),
        "Rotation",
        [p.rotation_deg],
        0.0..=360.0,
        0.5,
    );
    editing |= row.editing;
    if let Some(([start], [deg])) = row.done
        && tenths(deg) != tenths(start)
    {
        done.push(Field::Rotation(tenths(deg)));
    }
    let scale_range = kernel::MIN_TEXTURE_SCALE..=kernel::MAX_TEXTURE_SCALE;
    let row = widgets::numeric_row(
        ui,
        egui::Id::new(SCALE_KEY),
        "Scale",
        p.scale,
        scale_range,
        0.01,
    );
    editing |= row.editing;
    if let Some((start, scale)) = row.done
        && scale.map(thousandths) != start.map(thousandths)
    {
        done.push(Field::Scale(scale.map(thousandths)));
    }
    // The faces an edit applies to: those selected when it began.
    let faces = ui
        .data(|d| d.get_temp::<Vec<EntityId>>(faces_key))
        .unwrap_or_else(|| t.faces.clone());
    if editing {
        ui.data_mut(|d| d.insert_temp(faces_key, faces.clone()));
    } else {
        ui.data_mut(|d| d.remove::<Vec<EntityId>>(faces_key));
    }
    for field in done {
        actions.push(match field {
            Field::Offset(v) => UiAction::SetTextureOffset(faces.clone(), v),
            Field::Rotation(v) => UiAction::SetTextureRotation(faces.clone(), v),
            Field::Scale(v) => UiAction::SetTextureScale(faces.clone(), v),
        });
    }
    ui.horizontal(|ui| {
        if ui
            .button("Rotate 90")
            .on_hover_text("Turn the texture a quarter counter-clockwise")
            .clicked()
        {
            actions.push(UiAction::RotateTexture90);
        }
        if ui
            .button("Reset")
            .on_hover_text("Lay the texture as projected")
            .clicked()
        {
            actions.push(UiAction::ResetTexturePosition);
        }
    });
}

/// A texture position field's entered value.
enum Field {
    Offset([i32; 2]),
    Rotation(i32),
    Scale([u32; 2]),
}
