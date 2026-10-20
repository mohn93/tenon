//! The Tags tab: the Color by tag toggle, Add and Delete, and a row per tag with its
//! visibility, color, name and how many entities are on it. Entity Info's Tag field.

use egui::text::{CCursor, CCursorRange};
use egui::{Color32, Id, Popup, PopupCloseBehavior, RichText, Ui, Vec2};
use kernel::TagId;

use super::widgets::{self, Fill};
use super::{TagRow, UiAction, View, icons};
use crate::editor::SelectionTag;

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// The egui memory key of the tag chosen in the list (the one Delete deletes).
fn chosen_key() -> Id {
    Id::new("tags-chosen")
}

/// The egui memory key of the name being typed for a tag, with whether the field has yet to
/// take focus.
fn rename_key() -> Id {
    Id::new("tag-rename")
}

/// The Tags tab (the tab is its title); the dock's top area scrolls it.
pub(super) fn tags(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    let mut on = view.color_by_tag;
    if ui
        .checkbox(&mut on, "Color by tag")
        .on_hover_text("Draw faces in their tags' colors")
        .changed()
    {
        actions.push(UiAction::SetColorByTag(on));
    }
    // A chosen tag that went away (deleted, or the model replaced) is forgotten.
    let mut chosen = ui
        .data(|d| d.get_temp::<TagId>(chosen_key()))
        .filter(|t| view.tags.iter().any(|r| r.id == *t));
    ui.horizontal(|ui| {
        if widgets::icon_button(ui, icons::PLUS, "Add tag", "Add a tag", None, true).clicked() {
            actions.push(UiAction::AddTag);
        }
        let delete = widgets::icon_button(
            ui,
            icons::MINUS,
            "Delete tag",
            "Delete the chosen tag",
            None,
            chosen.is_some(),
        );
        if delete.clicked()
            && let Some(tag) = chosen
        {
            actions.push(UiAction::DeleteTag(tag));
        }
    });
    if view.tags.is_empty() {
        ui.label(RichText::new("No tags").weak());
    }
    for row in view.tags {
        tag_row(ui, row, &mut chosen, actions);
    }
    ui.data_mut(|d| match chosen {
        Some(t) => {
            d.insert_temp(chosen_key(), t);
        }
        None => d.remove::<TagId>(chosen_key()),
    });
}

/// One tag's row: the eye (shows or hides it), the color swatch (a click opens a color
/// picker; the color is set once it closes), the name (a click chooses the tag, a
/// double-click renames it) and the count at the right.
fn tag_row(ui: &mut Ui, row: &TagRow, chosen: &mut Option<TagId>, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        let (icon, label, tip) = if row.visible {
            (
                icons::EYE,
                format!("Hide {}", row.name),
                "Hide what is on this tag",
            )
        } else {
            (
                icons::EYE_SLASH,
                format!("Show {}", row.name),
                "Show what is on this tag",
            )
        };
        if widgets::icon_button(ui, icon, &label, tip, None, true).clicked() {
            actions.push(UiAction::SetTagVisible(row.id, !row.visible));
        }
        color_swatch(ui, row, actions);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let tip = match row.count {
                1 => "1 entity is on this tag".to_string(),
                n => format!("{n} entities are on this tag"),
            };
            ui.label(RichText::new(row.count.to_string()).weak())
                .on_hover_text(tip);
            name(ui, row, chosen, actions);
        });
    });
}

/// The tag's color swatch. While its picker is open the color being picked lives in egui's
/// memory (and shows in the swatch); when the picker closes, a changed color is reported once.
fn color_swatch(ui: &mut Ui, row: &TagRow, actions: &mut Vec<UiAction>) {
    let key = Id::new(("tag-color", row.id.0));
    let picking = ui.data(|d| d.get_temp::<Color32>(key));
    let mut color = picking.unwrap_or(rgb(row.color));
    let swatch = widgets::swatch(
        ui,
        Fill::Color(color),
        &format!("Color of {}", row.name),
        false,
    );
    let open = Popup::menu(&swatch)
        .id(key.with("popup"))
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            egui::color_picker::color_picker_color32(
                ui,
                &mut color,
                egui::color_picker::Alpha::Opaque,
            );
        });
    if open.is_some() {
        ui.data_mut(|d| d.insert_temp(key, color));
    } else if let Some(picked) = picking {
        ui.data_mut(|d| d.remove::<Color32>(key));
        let picked = [picked.r(), picked.g(), picked.b()];
        if picked != row.color {
            actions.push(UiAction::RecolorTag(row.id, picked));
        }
    }
}

/// The tag's name: a selectable row filling the space left, or while it is renamed a field
/// (Enter or leaving it renames; Escape keeps the name).
fn name(ui: &mut Ui, row: &TagRow, chosen: &mut Option<TagId>, actions: &mut Vec<UiAction>) {
    let renaming = ui
        .data(|d| d.get_temp::<(TagId, String, bool)>(rename_key()))
        .filter(|(t, ..)| *t == row.id);
    if let Some((_, mut text, fresh)) = renaming {
        let field_id = Id::new(("tag-name", row.id.0));
        let output = egui::TextEdit::singleline(&mut text)
            .id(field_id)
            .desired_width(ui.available_width())
            .show(ui);
        let edit = &output.response.response;
        edit.widget_info(|| {
            let mut info = egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Tag name");
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
            ui.data_mut(|d| d.insert_temp(rename_key(), (row.id, text, false)));
        } else if edit.lost_focus() || !edit.has_focus() {
            let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if edit.lost_focus() && !escaped && text != row.name {
                actions.push(UiAction::RenameTag(row.id, text));
            }
            ui.data_mut(|d| d.remove::<(TagId, String, bool)>(rename_key()));
        } else {
            ui.data_mut(|d| d.insert_temp(rename_key(), (row.id, text, false)));
        }
        return;
    }
    let selected = *chosen == Some(row.id);
    let mut text = RichText::new(&row.name);
    if !row.visible {
        text = text.weak();
    }
    let response = ui.add(
        egui::Button::selectable(selected, text)
            .truncate()
            .min_size(Vec2::new(ui.available_width(), 0.0)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &row.name)
    });
    if response.double_clicked() {
        ui.data_mut(|d| d.insert_temp(rename_key(), (row.id, row.name.clone(), true)));
    } else if response.clicked() {
        *chosen = Some(row.id);
    }
}

/// Entity Info's Tag field for the selection: Untagged and every tag to choose from; "Mixed"
/// while the selected entities are on different tags.
pub(super) fn selection_tag(
    ui: &mut Ui,
    view: &View,
    current: SelectionTag,
    actions: &mut Vec<UiAction>,
) {
    let tag_name = |tag: Option<TagId>| match tag {
        None => "Untagged".to_string(),
        Some(t) => view
            .tags
            .iter()
            .find(|r| r.id == t)
            .map_or_else(|| "Untagged".to_string(), |r| r.name.clone()),
    };
    let shown = match current {
        SelectionTag::One(tag) => tag_name(tag),
        SelectionTag::Mixed => "Mixed".to_string(),
    };
    ui.horizontal(|ui| {
        let label = ui.label("Tag");
        egui::ComboBox::from_id_salt("selection-tag")
            .selected_text(shown)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                let options = std::iter::once(None).chain(view.tags.iter().map(|r| Some(r.id)));
                for tag in options {
                    let on = current == SelectionTag::One(tag);
                    if ui.selectable_label(on, tag_name(tag)).clicked() && !on {
                        actions.push(UiAction::SetSelectionTag(tag));
                    }
                }
            })
            .response
            .labelled_by(label.id);
    });
}
