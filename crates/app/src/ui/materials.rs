//! The Materials list and its controls.

use egui::{Color32, RichText, Ui, Vec2};
use session::MaterialId;

use super::widgets::{self, Fill, take_row};
use super::{LibrarySave, MaterialRow, UiAction, View, theme};
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

/// How many `TILE` columns, `TILE_GAP` apart, fit `width` points (at least one).
pub(super) fn grid_columns(width: f32) -> usize {
    ((width + theme::TILE_GAP) / (theme::TILE + theme::TILE_GAP))
        .floor()
        .max(1.0) as usize
}

/// The Materials grid: a tile for the default material and each material, the one painted
/// with ringed. A click chooses a material, keeping the tool; a double-click also takes the
/// Paint tool. Then Add and Delete, and under "Edit material" the chosen material's name
/// (renamed on Enter or leaving the field), colors to pick from and its opacity (set when a
/// drag of the slider ends). The dock's Materials tab (the tab is its title); the dock's top
/// area scrolls it.
pub(super) fn materials(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    end_texture_size_edit(ui, view.material, actions);
    let tiles: Vec<(Option<MaterialId>, &str, Fill)> =
        std::iter::once((None, "Default", Fill::Color(DEFAULT_SWATCH)))
            .chain(view.materials.iter().map(|m| {
                let fill = match &m.texture {
                    Some(t) => Fill::Image(t.swatch),
                    None => Fill::Color(rgb(m.color)),
                };
                (Some(m.id), m.name.as_str(), fill)
            }))
            .collect();
    let columns = grid_columns(ui.available_width());
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(theme::TILE_GAP);
        for row in tiles.chunks(columns) {
            ui.horizontal(|ui| {
                for &(id, name, fill) in row {
                    let tile = widgets::material_tile(ui, fill, name, view.material == id);
                    if tile.double_clicked() {
                        actions.push(UiAction::PaintWithMaterial(id));
                    } else if tile.clicked() {
                        actions.push(UiAction::ChooseMaterial(id));
                    }
                }
            });
        }
    });
    material_controls(ui, view, actions);
}

/// Add and Delete, and under "Edit material" the chosen material's name, colors, sliders and
/// texture.
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
    let enabled = view.library.folder.is_some();
    let save = ui.add_enabled(enabled, egui::Button::new("Save to Library"));
    let save = if enabled {
        save
    } else {
        save.on_disabled_hover_text("Choose a library folder in Settings > Files first")
    };
    if save.clicked() {
        actions.push(UiAction::SaveToLibrary(LibrarySave::Material(m.id)));
    }
    widgets::section_header(ui, "Edit material");
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
            widgets::slider_row(ui, label.id, "Opacity", &mut value, 0..=100, 1.0, |b| {
                b.suffix("%")
            })
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
            widgets::slider_row(
                ui,
                label.id,
                "Glow",
                &mut value,
                0.0..=kernel::MAX_EMISSION,
                0.1,
                |b| b.max_decimals(1),
            )
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
