//! The dock on the right: Entity Info, Materials, Tags, Scenes and Library as tabs over the Outliner,
//! split by a divider. Its width, split and tab are kept in the preferences.

use std::ops::RangeInclusive;

use egui::{Id, Sense, Stroke, Ui, Vec2, WidgetInfo, WidgetType};
use serde::{Deserialize, Serialize};

use super::{
    UiAction, View, joints, library, materials, outliner, right, scenes, shadows, styles, tags,
    theme, widgets,
};

/// The tab shown in the dock's top area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockTab {
    #[default]
    EntityInfo,
    Materials,
    Tags,
    Scenes,
    Joints,
    Shadows,
    Styles,
    Library,
}

impl DockTab {
    fn name(self) -> &'static str {
        match self {
            DockTab::EntityInfo => "Entity Info",
            DockTab::Materials => "Materials",
            DockTab::Tags => "Tags",
            DockTab::Scenes => "Scenes",
            DockTab::Joints => "Joints",
            DockTab::Shadows => "Shadows",
            DockTab::Styles => "Styles",
            DockTab::Library => "Library",
        }
    }
}

/// The dock's width in points, the share of its height the top area takes, and its tab.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockLayout {
    pub width: f32,
    pub split: f32,
    pub tab: DockTab,
}

impl Default for DockLayout {
    fn default() -> Self {
        Self {
            width: 280.0,
            split: 0.55,
            tab: DockTab::EntityInfo,
        }
    }
}

impl DockLayout {
    /// The width in `DOCK_WIDTH` and the split in `SPLIT`; a value that is not a number is the
    /// default's.
    pub fn clamped(self) -> Self {
        let default = Self::default();
        let clamp = |v: f32, range: RangeInclusive<f32>, or: f32| {
            if v.is_finite() {
                v.clamp(*range.start(), *range.end())
            } else {
                or
            }
        };
        Self {
            width: clamp(self.width, DOCK_WIDTH, default.width),
            split: clamp(self.split, SPLIT, default.split),
            tab: self.tab,
        }
    }
}

/// The dock's width, margins included, in points.
pub const DOCK_WIDTH: RangeInclusive<f32> = 240.0..=420.0;
/// The share of the height under the tabs the top area takes.
pub const SPLIT: RangeInclusive<f32> = 0.2..=0.8;
/// The least height of the top area, when both areas fit.
pub const AREA_MIN: f32 = 160.0;
/// The least height of the Outliner area, when both areas fit: AREA_MIN for its header and
/// list, and the filter row (a control and the spacing under it) on top.
pub const OUTLINER_MIN: f32 = AREA_MIN + theme::CONTROL_HEIGHT + theme::GRID;
/// The divider's height.
pub const DIVIDER: f32 = 8.0;
/// The dock's inner margin.
pub const DOCK_MARGIN: i8 = 12;

/// The top area's height for `split` of `height`, keeping the top area AREA_MIN and the
/// Outliner area OUTLINER_MIN when both fit.
pub fn top_height(split: f32, height: f32) -> f32 {
    if height < AREA_MIN + OUTLINER_MIN {
        split * height
    } else {
        (split * height).clamp(AREA_MIN, height - OUTLINER_MIN)
    }
}

/// The egui memory key of the split while the divider is dragged.
fn split_id() -> Id {
    Id::new("dock-split")
}

/// The tabs, the top area with the tab's content (scrolled when long), the divider and the
/// Outliner. Returns the layout with the tab or split changed this frame (the divider's split
/// once its drag ends); the caller reports it.
pub(super) fn dock(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) -> DockLayout {
    let mut layout = view.dock;
    // Fill the panel's width, so it keeps the width it was given.
    ui.set_min_width(ui.max_rect().width());
    let tabs: &[DockTab] = if view.features.library {
        &TABS
    } else {
        &TABS[..TABS.len() - 1]
    };
    // A Library tab kept in the preferences, where there is no library.
    let tab = if tabs.contains(&view.dock.tab) {
        view.dock.tab
    } else {
        DockTab::EntityInfo
    };
    tab_row(ui, tab, tabs, &mut layout.tab);
    let width = ui.available_width();
    // The areas share what the divider and the spacing after the top area and after the
    // divider leave.
    let height = (ui.available_height() - DIVIDER - 2.0 * ui.spacing().item_spacing.y).max(0.0);
    // The divider's drag, read before the areas are laid out so they follow the pointer in
    // the same frame.
    let divider_id = Id::new("dock-divider");
    let committed = top_height(view.dock.split, height);
    let mut split = ui
        .data(|d| d.get_temp::<f32>(split_id()))
        .unwrap_or(view.dock.split);
    let response = ui.ctx().read_response(divider_id);
    match response {
        Some(r) if r.dragged() || r.drag_stopped() => {
            // While dragged: the top area's committed height moved as far as the pointer has.
            // egui counts a press as a drag; the split is kept only once the pointer moved, so
            // a plain press and release leaves the saved split as it is.
            if r.dragged()
                && height > 0.0
                && let (Some(pos), Some(origin)) = (
                    r.interact_pointer_pos(),
                    ui.input(|i| i.pointer.press_origin()),
                )
                && (pos != origin || ui.data(|d| d.get_temp::<f32>(split_id())).is_some())
            {
                split =
                    ((committed + pos.y - origin.y) / height).clamp(*SPLIT.start(), *SPLIT.end());
                ui.data_mut(|d| d.insert_temp(split_id(), split));
            }
            // Released: the last split shown, in hundredths, is reported.
            if r.drag_stopped()
                && let Some(last) = ui.data_mut(|d| d.remove_temp::<f32>(split_id()))
            {
                split = last;
                layout.split = (last * 100.0).round() / 100.0;
            }
        }
        // Not dragged: a split left from a drag that ended without a release (the divider
        // gone, the window unfocused) is dropped.
        _ => {
            if ui.data_mut(|d| d.remove_temp::<f32>(split_id())).is_some() {
                split = view.dock.split;
            }
        }
    }
    // The tab's content is laid out in a part of the dock that does not grow with it: a row
    // wider than the dock is cut off at the dock's margin instead of widening the dock over
    // the 3D view.
    let area =
        egui::Rect::from_min_size(ui.cursor().min, Vec2::new(width, top_height(split, height)));
    let mut top_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("dock-top-area")
            .max_rect(area)
            .layout(*ui.layout()),
    );
    let mut clip = ui.clip_rect();
    let margin = f32::from(DOCK_MARGIN);
    (clip.min.x, clip.max.x) = (
        clip.min.x.max(area.min.x - margin),
        clip.max.x.min(area.max.x + margin),
    );
    top_ui.set_clip_rect(clip);
    let top = egui::ScrollArea::vertical()
        .id_salt("dock-top")
        .auto_shrink([false, false])
        .max_height(area.height())
        .show(&mut top_ui, |ui| match tab {
            DockTab::EntityInfo => right::entity_info(ui, view, actions),
            DockTab::Materials => materials::materials(ui, view, actions),
            DockTab::Tags => tags::tags(ui, view, actions),
            DockTab::Scenes => scenes::scenes(ui, view, actions),
            DockTab::Joints => joints::joints(ui, view, actions),
            DockTab::Shadows => shadows::shadows(ui, view, actions),
            DockTab::Styles => styles::styles(ui, view, actions),
            DockTab::Library => library::library(ui, view, actions),
        });
    ui.advance_cursor_after_rect(area);
    widgets::name_scroll_bars(ui.ctx(), top.id, tab.name());
    divider(ui, divider_id, width);
    outliner::outliner(ui, view, actions);
    layout
}

/// The tabs, in order.
const TABS: [DockTab; 8] = [
    DockTab::EntityInfo,
    DockTab::Materials,
    DockTab::Tags,
    DockTab::Scenes,
    DockTab::Joints,
    DockTab::Shadows,
    DockTab::Styles,
    DockTab::Library,
];

/// The gap between two tabs.
const TAB_GAP: f32 = 4.0;

/// The least padding at each side of a tab's name before the tabs go over two rows.
const TAB_PAD_MIN: f32 = 4.0;

/// The tabs across the dock's width: each its name's width and an equal share of what is left
/// (at the narrowest dock little is). Where they do not fit in one row even so, they go over
/// two balanced rows (3 and 3, or 3 and 2), each justified to the dock's width. A tab other
/// than `shown` clicked becomes `chosen`.
fn tab_row(ui: &mut Ui, shown: DockTab, tabs: &[DockTab], chosen: &mut DockTab) {
    let names: Vec<f32> = tabs
        .iter()
        .map(|tab| {
            egui::WidgetText::from(tab.name())
                .into_galley(
                    ui,
                    Some(egui::TextWrapMode::Extend),
                    f32::INFINITY,
                    egui::TextStyle::Button,
                )
                .size()
                .x
        })
        .collect();
    let gaps = |n: usize| TAB_GAP * (n - 1) as f32;
    let share = |names: &[f32], width: f32| {
        (width - gaps(names.len()) - names.iter().sum::<f32>()) / names.len() as f32
    };
    let width = ui.available_width();
    let rows = if share(&names, width) < 2.0 * TAB_PAD_MIN {
        2
    } else {
        1
    };
    let first = tabs.len().div_ceil(rows);
    for (row_tabs, row_names) in tabs.chunks(first).zip(names.chunks(first)) {
        let share = share(row_names, width);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = TAB_GAP;
            ui.spacing_mut().button_padding.x = (share / 2.0).clamp(0.0, 8.0);
            for (&tab, name) in row_tabs.iter().zip(row_names) {
                let on = shown == tab;
                let button = egui::Button::selectable(on, tab.name())
                    .truncate()
                    .min_size(Vec2::new((name + share.max(0.0)).floor(), 0.0));
                if ui.add(button).clicked() && !on {
                    *chosen = tab;
                }
            }
        });
    }
}

/// The divider between the top area and the Outliner: a line across, in the accent while
/// hovered or dragged. Named "Resize Outliner".
fn divider(ui: &mut Ui, id: Id, width: f32) {
    let t = theme::tokens(ui.ctx().theme());
    let (_, rect) = ui.allocate_space(Vec2::new(width, DIVIDER));
    let response = ui.interact(rect, id, Sense::drag());
    let color = if response.hovered() || response.dragged() {
        t.accent
    } else {
        t.border
    };
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, color));
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Resize Outliner"));
}
