//! The Materials list and its controls.

use egui::{Color32, RichText, Ui};
use session::MaterialId;

use super::right::{MATERIAL_ROWS_MAX, MATERIAL_ROWS_MIN, OUTLINER_ROWS_MIN, measured, row_pitch};
use super::widgets::{self, Fill, take_row};
use super::{MaterialRow, UiAction, View};
use crate::editor::MATERIAL_PALETTE;

/// The default material's swatch: its front color.
const DEFAULT_SWATCH: Color32 = Color32::from_rgb(237, 237, 230);

/// How a palette swatch is named for AccessKit: `Color #C45440`.
pub(super) fn palette_label(c: [u8; 3]) -> String {
    format!("Color #{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// The Materials list: the default material and every material, the one painted with
/// highlighted (clicking one chooses it and the Paint tool); Add and Delete; and for the
/// chosen material its name (renamed on Enter or leaving the field), colors to pick from and
/// its opacity (set when a drag of the slider ends). Returns the list's top and the top of what
/// follows it, in points.
pub(super) fn materials(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) -> (f32, f32) {
    end_texture_size_edit(ui, view.material, actions);
    widgets::section_header(ui, "Materials");
    // Whole rows only: as many as leave the controls below and the Outliner's least rows room.
    let pitch = row_pitch(ui.style());
    let below = measured(ui.ctx()).below_list + OUTLINER_ROWS_MIN as f32 * pitch;
    let fit = ((ui.available_height() - below) / pitch).floor().max(0.0) as usize;
    let shown = fit.clamp(MATERIAL_ROWS_MIN, MATERIAL_ROWS_MAX);
    let list_top = ui.cursor().top();
    egui::ScrollArea::vertical()
        .id_salt("materials")
        .max_height(shown as f32 * pitch - ui.spacing().item_spacing.y)
        .show(ui, |ui| {
            let rows = std::iter::once((None, "Default", Fill::Color(DEFAULT_SWATCH))).chain(
                view.materials.iter().map(|m| {
                    let fill = match &m.texture {
                        Some(t) => Fill::Image(t.swatch),
                        None => Fill::Color(rgb(m.color)),
                    };
                    (Some(m.id), m.name.as_str(), fill)
                }),
            );
            for (id, name, fill) in rows {
                ui.horizontal(|ui| {
                    let chosen = view.material == id;
                    let picked =
                        widgets::swatch(ui, fill, &format!("{name} color"), chosen).clicked();
                    // Cut short, not wider than the panel: a wide row would push it aside.
                    let label = ui.add(egui::Button::selectable(chosen, name).truncate());
                    if picked || label.clicked() {
                        actions.push(UiAction::ChooseMaterial(id));
                    }
                });
            }
        });
    let list_bottom = ui.cursor().top();
    material_controls(ui, view, actions);
    (list_top, list_bottom)
}

/// Add and Delete, and the chosen material's name, colors, sliders and texture.
fn material_controls(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        if ui.button("Add").clicked() {
            actions.push(UiAction::AddMaterial);
        }
        let current = view.material;
        if ui
            .add_enabled(current.is_some(), egui::Button::new("Delete"))
            .clicked()
            && let Some(id) = current
        {
            actions.push(UiAction::DeleteMaterial(id));
        }
    });
    let Some(m) = view
        .material
        .and_then(|id| view.materials.iter().find(|m| m.id == id))
    else {
        return;
    };
    // The name being typed lives in egui's memory until the field loses focus.
    let key = egui::Id::new(("material-name", m.id.0));
    let mut text = ui
        .data(|d| d.get_temp::<String>(key))
        .unwrap_or_else(|| m.name.clone());
    let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY));
    edit.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Material name");
        info.current_text_value = Some(text.clone());
        info
    });
    if edit.lost_focus() {
        // Escape discards the edit; so does deleting the material in the same frame.
        let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let deleted = actions.contains(&UiAction::DeleteMaterial(m.id));
        if text != m.name && !escaped && !deleted {
            actions.push(UiAction::RenameMaterial(m.id, text));
        }
        ui.data_mut(|d| d.remove::<String>(key));
    } else if edit.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, text));
    }
    ui.horizontal_wrapped(|ui| {
        for color in MATERIAL_PALETTE {
            let response = widgets::swatch(
                ui,
                Fill::Color(rgb(color)),
                &palette_label(color),
                color == m.color,
            );
            if response.clicked() && color != m.color {
                actions.push(UiAction::RecolorMaterial(m.id, color));
            }
        }
    });
    // The value being dragged lives in egui's memory until the drag ends, so a drag is one
    // change (one undo step).
    let key = egui::Id::new(("material-opacity", m.id.0));
    let shown = percent(m.opacity);
    let mut value = ui.data(|d| d.get_temp::<u8>(key)).unwrap_or(shown);
    let slider = ui
        .horizontal(|ui| {
            let label = ui.label("Opacity");
            // Fit the panel: the slider takes what the label and the number leave.
            ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(40.0);
            ui.add(egui::Slider::new(&mut value, 0..=100).suffix("%"))
                .labelled_by(label.id)
        })
        .inner;
    if slider.dragged() {
        ui.data_mut(|d| d.insert_temp(key, value));
    } else {
        if (slider.drag_stopped() || slider.changed()) && value != shown {
            actions.push(UiAction::SetMaterialOpacity(m.id, value));
        }
        ui.data_mut(|d| d.remove::<u8>(key));
    }
    glow_slider(ui, m, actions);
    texture_controls(ui, m, actions);
}

/// A material's Glow slider, its emission from 0 to 10 in tenths; like the opacity, one change
/// when a drag ends.
fn glow_slider(ui: &mut Ui, m: &MaterialRow, actions: &mut Vec<UiAction>) {
    let key = egui::Id::new(("material-emission", m.id.0));
    let shown = tenths(m.emission);
    let mut value = ui
        .data(|d| d.get_temp::<f32>(key))
        .unwrap_or(f32::from(shown) / 10.0);
    let slider = ui
        .horizontal(|ui| {
            let label = ui.label("Glow").on_hover_text(
                "Faces painted with a glowing material are drawn unshaded and light their \
                 surroundings (Camera > Night lighting shows them best)",
            );
            ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(40.0);
            ui.add(
                egui::Slider::new(&mut value, 0.0..=kernel::MAX_EMISSION)
                    .step_by(0.1)
                    .max_decimals(1),
            )
            .labelled_by(label.id)
        })
        .inner;
    if slider.dragged() {
        ui.data_mut(|d| d.insert_temp(key, value));
    } else {
        if (slider.drag_stopped() || slider.changed()) && tenths(value) != shown {
            actions.push(UiAction::SetMaterialEmission(m.id, tenths(value)));
        }
        ui.data_mut(|d| d.remove::<f32>(key));
    }
}

/// An emission (0 to 10) in whole tenths.
fn tenths(emission: f32) -> u8 {
    (emission.clamp(0.0, kernel::MAX_EMISSION) * 10.0).round() as u8
}

/// The egui memory key of material `id`'s texture size edit.
fn texture_size_key(id: MaterialId) -> egui::Id {
    egui::Id::new(("texture-size", id.0))
}

/// Ends a texture size edit left in progress when another material is chosen: a size already
/// taken (dragged, or stepped with the arrow keys) applies to the material it was edited for.
fn end_texture_size_edit(ui: &mut Ui, chosen: Option<MaterialId>, actions: &mut Vec<UiAction>) {
    let owner_key = egui::Id::new("texture-size-material");
    let owner = ui.data(|d| d.get_temp::<MaterialId>(owner_key));
    if let Some(old) = owner
        && Some(old) != chosen
        && let Some((start, size)) = take_row::<2>(ui, texture_size_key(old))
    {
        let mm = |m: f64| (m * 1000.0).round() as u32;
        if size.map(mm) != start.map(mm) {
            actions.push(UiAction::SetTextureSize(old, size.map(mm)));
        }
    }
    ui.data_mut(|d| match chosen {
        Some(id) => {
            d.insert_temp(owner_key, id);
        }
        None => d.remove::<MaterialId>(owner_key),
    });
}

/// A material's texture controls: Texture... to load an image and Remove, then the size one copy
/// covers (set when a drag ends or a typed value is entered).
fn texture_controls(ui: &mut Ui, m: &MaterialRow, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        if ui
            .button("Texture...")
            .on_hover_text("Show a PNG or JPEG image on faces painted with this material")
            .clicked()
        {
            actions.push(UiAction::LoadTexture(m.id));
        }
        if m.texture.is_some() && ui.button("Remove").clicked() {
            actions.push(UiAction::RemoveTexture(m.id));
        }
    });
    let Some(t) = &m.texture else { return };
    ui.label(RichText::new(format!("{} x {} px", t.pixels[0], t.pixels[1])).weak());
    // One change per drag or entered value, like the texture position fields.
    let row = widgets::numeric_row(
        ui,
        texture_size_key(m.id),
        "Size (m)",
        [t.width, t.height],
        kernel::MIN_TEXTURE_SIZE..=kernel::MAX_TEXTURE_SIZE,
        0.01,
    );
    let mm = |m: f64| (m * 1000.0).round() as u32;
    if let Some((start, size)) = row.done
        && size.map(mm) != start.map(mm)
    {
        actions.push(UiAction::SetTextureSize(m.id, size.map(mm)));
    }
}

/// An opacity (0 to 1) in whole percent.
fn percent(opacity: f32) -> u8 {
    (opacity.clamp(0.0, 1.0) * 100.0).round() as u8
}
