//! The Styles tab: the Default row and a row per style (a click chooses it, a double-click
//! renames it), Add and Delete, and for the chosen style an editor of its edges, background and
//! fog. Each edit is one `change_style`; a slider, value box or color picker reports once, when
//! the drag ends, the value is entered or the picker closes. View > Edge Style toggles the
//! chosen style's edge effects.

use std::ops::RangeInclusive;

use egui::text::{CCursor, CCursorRange};
use egui::{Color32, Id, Popup, PopupCloseBehavior, RichText, Ui, Vec2};
use kernel::{
    ENDPOINT_LENGTH, EdgeColorMode, Fog, FogColor, Ground, PROFILE_WIDTH, SKETCHY_EXTENSION,
    SKETCHY_JITTER, Sketchy, StyleId, StyleSettings,
};
use session::{EdgesPatch, StylePatch, Units, format_length, parse_length};

use super::widgets::{self, Fill};
use super::{EdgeEffect, StyleRow, UiAction, View, icons};

/// What turning an effect on starts with.
const PROFILES_ON: f32 = 3.0;
const SKETCHY_ON: Sketchy = Sketchy {
    jitter: 2.0,
    extension: 8.0,
};
const ENDPOINTS_ON: f32 = 8.0;
const SKY_ON: [u8; 3] = [135, 180, 235];
const GROUND_ON: Ground = Ground {
    color: [150, 140, 120],
    opacity: 0.5,
    from_below: false,
};
const FOG_ON: Fog = Fog {
    start: 10.0,
    end: 100.0,
    color: FogColor::Background,
};

/// The furthest a fog distance goes, in meters.
const FOG_FAR: f64 = 100_000.0;
/// The least gap between the fog's start and end, in meters.
const FOG_GAP: f64 = 0.01;

/// The color pickers' names: what their swatches are called.
const COLOR_NAMES: [&str; 5] = [
    "Edge color",
    "Background color",
    "Sky color",
    "Ground color",
    "Fog color",
];

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// A style name being typed: the style, the text, and whether the field has yet to take focus.
type Renaming = (StyleId, String, bool);

fn rename_key() -> Id {
    Id::new("style-rename")
}

/// The egui memory key of the document count (`View::documents`) the tab last drew.
fn document_key() -> Id {
    Id::new("styles-document")
}

fn picking_key(name: &str) -> Id {
    Id::new(("style-picking", name))
}

fn picker_id(name: &str) -> Id {
    Id::new(("style-color", name)).with("popup")
}

/// An open color picker: its style, the color the style had when it opened, the color picked so
/// far, and the pass that last drew it.
#[derive(Clone, Copy)]
struct Picking {
    style: StyleId,
    opened: [u8; 3],
    color: Color32,
    pass: u64,
}

/// A patch of the edge settings.
fn edges_patch(f: impl FnOnce(&mut EdgesPatch)) -> StylePatch {
    let mut e = EdgesPatch::default();
    f(&mut e);
    StylePatch {
        edges: Some(e),
        ..Default::default()
    }
}

/// The patch View > Edge Style makes: `effect` on with its default size, or off.
pub(crate) fn edge_effect_patch(settings: &StyleSettings, effect: EdgeEffect) -> StylePatch {
    let e = &settings.edges;
    edges_patch(|p| match effect {
        EdgeEffect::Profiles => p.profiles = Some(e.profiles.is_none().then_some(PROFILES_ON)),
        EdgeEffect::Sketchy => p.sketchy = Some(e.sketchy.is_none().then_some(SKETCHY_ON)),
        EdgeEffect::Endpoints => p.endpoints = Some(e.endpoints.is_none().then_some(ENDPOINTS_ON)),
    })
}

/// The Styles tab (the tab is its title); the dock's top area scrolls it.
pub(super) fn styles(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    forget_gone(ui, view);
    let chosen = view
        .style
        .and_then(|id| view.styles.iter().find(|r| r.id == id));
    ui.horizontal(|ui| {
        if widgets::icon_button(
            ui,
            icons::PLUS,
            "Add style",
            "Add a style: a copy of the chosen one, or the default settings",
            None,
            true,
        )
        .clicked()
        {
            actions.push(UiAction::AddStyle(chosen.map(|r| r.id)));
        }
        let delete = widgets::icon_button(
            ui,
            icons::MINUS,
            "Delete style",
            "Delete the chosen style",
            None,
            chosen.is_some(),
        );
        if delete.clicked()
            && let Some(row) = chosen
        {
            actions.push(UiAction::DeleteStyle(row.id));
        }
    });
    let default = ui.add(
        egui::Button::selectable(chosen.is_none(), "Default")
            .min_size(Vec2::new(ui.available_width(), 0.0)),
    );
    default.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, chosen.is_none(), "Default")
    });
    if default.clicked() && chosen.is_some() {
        actions.push(UiAction::ChooseStyle(None));
    }
    for row in view.styles {
        name(ui, row, chosen.is_some_and(|c| c.id == row.id), actions);
    }
    let Some(row) = chosen else {
        ui.add(
            egui::Label::new(RichText::new("Default draws the model as the app does").weak())
                .wrap(),
        );
        return;
    };
    editor(ui, row, view.units, actions);
}

/// Forgets what was held for styles no longer listed (deleted, or the model replaced): the name
/// being typed. A color picker its row stopped drawing (the tab left) closes with its color
/// dropped.
fn forget_gone(ui: &Ui, view: &View) {
    let seen = ui.data(|d| d.get_temp::<u64>(document_key()));
    ui.data_mut(|d| d.insert_temp(document_key(), view.documents));
    let replaced = seen.is_some_and(|n| n != view.documents);
    let renaming = ui.data(|d| d.get_temp::<Renaming>(rename_key()));
    if let Some((id, ..)) = renaming
        && (replaced || !view.styles.iter().any(|r| r.id == id))
    {
        ui.data_mut(|d| d.remove::<Renaming>(rename_key()));
    }
    for name in COLOR_NAMES {
        let picking = ui.data(|d| d.get_temp::<Picking>(picking_key(name)));
        if let Some(p) = picking
            && (replaced
                || !view.styles.iter().any(|r| r.id == p.style)
                || p.pass + 1 < ui.ctx().cumulative_pass_nr())
        {
            ui.data_mut(|d| d.remove::<Picking>(picking_key(name)));
            Popup::close_id(ui.ctx(), picker_id(name));
        }
    }
}

/// The style's name: a selectable row filling the width (a click chooses it, a double-click
/// renames it), or while it is renamed a field (Enter or leaving it renames; Escape keeps the
/// name).
fn name(ui: &mut Ui, row: &StyleRow, chosen: bool, actions: &mut Vec<UiAction>) {
    let renaming = ui
        .data(|d| d.get_temp::<Renaming>(rename_key()))
        .filter(|(s, ..)| *s == row.id);
    if let Some((_, mut text, fresh)) = renaming {
        let field_id = Id::new(("style-name", row.id.0));
        let output = egui::TextEdit::singleline(&mut text)
            .id(field_id)
            .desired_width(ui.available_width())
            .show(ui);
        let edit = &output.response.response;
        edit.widget_info(|| {
            let mut info =
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Style name");
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
            ui.data_mut(|d| d.insert_temp::<Renaming>(rename_key(), (row.id, text, false)));
        } else if edit.lost_focus() || !edit.has_focus() {
            let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if edit.lost_focus() && !escaped && text != row.name {
                actions.push(UiAction::RenameStyle(row.id, text));
            }
            ui.data_mut(|d| d.remove::<Renaming>(rename_key()));
        } else {
            ui.data_mut(|d| d.insert_temp::<Renaming>(rename_key(), (row.id, text, false)));
        }
        return;
    }
    let response = ui.add(
        egui::Button::selectable(chosen, &row.name)
            .truncate()
            .min_size(Vec2::new(ui.available_width(), 0.0)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, chosen, &row.name)
    });
    if response.double_clicked() {
        ui.data_mut(|d| d.insert_temp::<Renaming>(rename_key(), (row.id, row.name.clone(), true)));
    } else if response.clicked() && !chosen {
        actions.push(UiAction::ChooseStyle(Some(row.id)));
    }
}

/// The editor of the chosen style: Edges, Background and Fog.
fn editor(ui: &mut Ui, row: &StyleRow, units: Units, actions: &mut Vec<UiAction>) {
    let s = &row.settings;
    let id = row.id;
    let mut send = |patch: StylePatch| actions.push(UiAction::ChangeStyle(id, patch));
    widgets::section_header(ui, "Edges");
    let mut show = s.edges.show_edges;
    if ui
        .checkbox(&mut show, "Show edges")
        .on_hover_text("Off draws only the profiles")
        .changed()
    {
        send(edges_patch(|e| e.show_edges = Some(show)));
    }
    ui.horizontal_wrapped(|ui| {
        for (label, mode) in [
            ("Fixed", EdgeColorMode::Fixed),
            ("By material", EdgeColorMode::ByMaterial),
            ("By axis", EdgeColorMode::ByAxis),
        ] {
            let on = s.edges.color_mode == mode;
            let b = ui.add(egui::Button::selectable(on, label));
            b.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, label));
            if b.clicked() && !on {
                send(edges_patch(|e| e.color_mode = Some(mode)));
            }
        }
        if let Some(c) = color(ui, id, "Edge color", s.edges.color) {
            send(edges_patch(|e| e.color = Some(c)));
        }
    });
    let mut on = s.edges.profiles.is_some();
    if ui.checkbox(&mut on, "Profiles").changed() {
        send(edges_patch(|e| {
            e.profiles = Some(on.then_some(PROFILES_ON))
        }));
    }
    ui.add_enabled_ui(on, |ui| {
        let shown = s.edges.profiles.unwrap_or(PROFILES_ON);
        if let Some(w) = slider(ui, "Profile width", shown, PROFILE_WIDTH, 0.5, |b| {
            b.suffix(" px")
        }) {
            send(edges_patch(|e| e.profiles = Some(Some(w))));
        }
    });
    let mut on = s.edges.sketchy.is_some();
    if ui.checkbox(&mut on, "Sketchy edges").changed() {
        send(edges_patch(|e| e.sketchy = Some(on.then_some(SKETCHY_ON))));
    }
    ui.add_enabled_ui(on, |ui| {
        let now = s.edges.sketchy.unwrap_or(SKETCHY_ON);
        if let Some(jitter) = slider(ui, "Jitter", now.jitter, SKETCHY_JITTER, 0.5, |b| {
            b.suffix(" px")
        }) {
            send(edges_patch(|e| {
                e.sketchy = Some(Some(Sketchy { jitter, ..now }))
            }));
        }
        if let Some(extension) = slider(
            ui,
            "Extension",
            now.extension,
            SKETCHY_EXTENSION,
            0.5,
            |b| b.suffix(" px"),
        ) {
            send(edges_patch(|e| {
                e.sketchy = Some(Some(Sketchy { extension, ..now }))
            }));
        }
    });
    let mut on = s.edges.endpoints.is_some();
    if ui.checkbox(&mut on, "Endpoints").changed() {
        send(edges_patch(|e| {
            e.endpoints = Some(on.then_some(ENDPOINTS_ON))
        }));
    }
    ui.add_enabled_ui(on, |ui| {
        let shown = s.edges.endpoints.unwrap_or(ENDPOINTS_ON);
        if let Some(l) = slider(ui, "Endpoint length", shown, ENDPOINT_LENGTH, 0.5, |b| {
            b.suffix(" px")
        }) {
            send(edges_patch(|e| e.endpoints = Some(Some(l))));
        }
    });

    widgets::section_header(ui, "Background");
    ui.horizontal(|ui| {
        ui.label("Color");
        if let Some(c) = color(ui, id, "Background color", s.background) {
            send(StylePatch {
                background: Some(c),
                ..Default::default()
            });
        }
    });
    let sky_patch = |sky| StylePatch {
        sky: Some(sky),
        ..Default::default()
    };
    ui.horizontal(|ui| {
        let mut on = s.sky.is_some();
        if ui.checkbox(&mut on, "Sky").changed() {
            send(sky_patch(on.then_some(SKY_ON)));
        }
        if let Some(now) = s.sky
            && let Some(c) = color(ui, id, "Sky color", now)
        {
            send(sky_patch(Some(c)));
        }
    });
    let ground_patch = |g| StylePatch {
        ground: Some(Some(g)),
        ..Default::default()
    };
    let now = s.ground.unwrap_or(GROUND_ON);
    ui.horizontal(|ui| {
        let mut on = s.ground.is_some();
        if ui.checkbox(&mut on, "Ground").changed() {
            send(StylePatch {
                ground: Some(on.then_some(GROUND_ON)),
                ..Default::default()
            });
        }
        ui.add_enabled_ui(on, |ui| {
            if let Some(c) = color(ui, id, "Ground color", now.color) {
                send(ground_patch(Ground { color: c, ..now }));
            }
        });
    });
    ui.add_enabled_ui(s.ground.is_some(), |ui| {
        let percent = (now.opacity * 100.0).round();
        if let Some(p) = slider(ui, "Ground opacity", percent, 0.0..=100.0, 1.0, |b| {
            b.suffix("%")
        }) {
            send(ground_patch(Ground {
                opacity: p / 100.0,
                ..now
            }));
        }
        let mut below = now.from_below;
        if ui
            .checkbox(&mut below, "Ground from below")
            .on_hover_text("Also draw the ground when the eye is under it")
            .changed()
        {
            send(ground_patch(Ground {
                from_below: below,
                ..now
            }));
        }
    });

    widgets::section_header(ui, "Distance fog");
    let mut on = s.fog.is_some();
    if ui.checkbox(&mut on, "Fog").changed() {
        send(StylePatch {
            fog: Some(on.then_some(FOG_ON)),
            ..Default::default()
        });
    }
    let fog = s.fog.unwrap_or(FOG_ON);
    let fog_patch = |f: Fog| StylePatch {
        fog: Some(Some(f)),
        ..Default::default()
    };
    ui.add_enabled_ui(on, |ui| {
        let (start, end) = (f64::from(fog.start), f64::from(fog.end));
        if let Some(v) = length(ui, "Fog start", start, 0.0..=end - FOG_GAP, units) {
            send(fog_patch(Fog {
                start: v as f32,
                ..fog
            }));
        }
        if let Some(v) = length(ui, "Fog end", end, start + FOG_GAP..=FOG_FAR, units) {
            send(fog_patch(Fog {
                end: v as f32,
                ..fog
            }));
        }
        ui.horizontal_wrapped(|ui| {
            let custom = matches!(fog.color, FogColor::Rgb(_));
            for (label, is_custom) in [("Same as background", false), ("Custom", true)] {
                let chosen = custom == is_custom;
                let b = ui.add(egui::Button::selectable(chosen, label));
                b.widget_info(|| {
                    egui::WidgetInfo::selected(egui::WidgetType::Button, true, chosen, label)
                });
                if b.clicked() && !chosen {
                    let color = if is_custom {
                        FogColor::Rgb(s.background)
                    } else {
                        FogColor::Background
                    };
                    send(fog_patch(Fog { color, ..fog }));
                }
            }
            if let FogColor::Rgb(now) = fog.color
                && let Some(c) = color(ui, id, "Fog color", now)
            {
                send(fog_patch(Fog {
                    color: FogColor::Rgb(c),
                    ..fog
                }));
            }
        });
    });
}

/// A color swatch named `name` (a click opens a picker). While the picker is open the color
/// being picked lives in egui's memory (and shows in the swatch); when it closes, a color
/// other than the one it opened with is returned once.
fn color(ui: &mut Ui, style: StyleId, name: &str, shown: [u8; 3]) -> Option<[u8; 3]> {
    let picking = ui
        .data(|d| d.get_temp::<Picking>(picking_key(name)))
        .filter(|p| p.style == style);
    // Read before the popup, which takes the key when it closes.
    let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let mut color = picking.map_or(rgb(shown), |p| p.color);
    let swatch = widgets::swatch(ui, Fill::Color(color), name, false);
    let open = Popup::menu(&swatch)
        .id(picker_id(name))
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            egui::color_picker::color_picker_color32(
                ui,
                &mut color,
                egui::color_picker::Alpha::Opaque,
            );
        });
    if open.is_some() && escaped {
        // The popup closes after this frame: the pick is dropped now, as it is cancelled.
        ui.data_mut(|d| d.remove::<Picking>(picking_key(name)));
        None
    } else if open.is_some() {
        let held = Picking {
            style,
            opened: picking.map_or(shown, |p| p.opened),
            color,
            pass: ui.ctx().cumulative_pass_nr(),
        };
        ui.data_mut(|d| d.insert_temp(picking_key(name), held));
        None
    } else if let Some(p) = picking {
        ui.data_mut(|d| d.remove::<Picking>(picking_key(name)));
        let picked = [p.color.r(), p.color.g(), p.color.b()];
        (picked != p.opened).then_some(picked)
    } else {
        None
    }
}

/// A row of `name`, a slider over `range` and its value box. The value being dragged lives in
/// egui's memory until the drag ends, so a drag is one change; returns the value then, or when
/// one is entered.
fn slider(
    ui: &mut Ui,
    name: &str,
    shown: f32,
    range: RangeInclusive<f32>,
    step: f64,
    value_box: impl for<'v> FnOnce(egui::DragValue<'v>) -> egui::DragValue<'v>,
) -> Option<f32> {
    let key = Id::new(("style-slider", name));
    let mut value = ui.data(|d| d.get_temp::<f32>(key)).unwrap_or(shown);
    let response = ui
        .horizontal(|ui| {
            let label = ui.label(name);
            ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(40.0);
            widgets::slider_row(ui, label.id, name, &mut value, range, step, |b| {
                value_box(b).update_while_editing(false)
            })
        })
        .inner;
    if response.dragged() {
        ui.data_mut(|d| d.insert_temp(key, value));
        None
    } else {
        ui.data_mut(|d| d.remove::<f32>(key));
        ((response.drag_stopped() || response.changed()) && value != shown).then_some(value)
    }
}

/// A row of `name` and a length field in the model's units over `range` (meters); returns the
/// length when a drag ends or a value is entered. The field is named "`name` value".
fn length(
    ui: &mut Ui,
    name: &str,
    shown: f64,
    range: RangeInclusive<f64>,
    units: Units,
) -> Option<f64> {
    let key = Id::new(("style-length", name));
    let mut value = ui.data(|d| d.get_temp::<f64>(key)).unwrap_or(shown);
    let response = ui
        .horizontal(|ui| {
            let label = ui.label(name);
            let field = ui.add(
                egui::DragValue::new(&mut value)
                    .range(range)
                    .clamp_existing_to_range(false)
                    .speed(0.1)
                    .update_while_editing(false)
                    .custom_formatter(|v, _| format_length(v, &units))
                    .custom_parser(|text| parse_length(text, &units).ok()),
            );
            field.clone().labelled_by(label.id);
            ui.ctx().accesskit_node_builder(field.id, |node| {
                node.set_label(format!("{name} value"));
            });
            field
        })
        .inner;
    if response.dragged() {
        ui.data_mut(|d| d.insert_temp(key, value));
        None
    } else {
        ui.data_mut(|d| d.remove::<f64>(key));
        // The field shows millimeters at most: a value that reads back the same is no edit.
        ((response.drag_stopped() || response.changed()) && (value - shown).abs() > 0.0005)
            .then_some(value)
    }
}
