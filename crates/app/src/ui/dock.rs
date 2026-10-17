//! The dock on the right: Entity Info and Materials as tabs over the Outliner, split by a
//! divider. Its width, split and tab are kept in the preferences.

use std::ops::RangeInclusive;

use egui::{Id, Sense, Stroke, Ui, Vec2, WidgetInfo, WidgetType};
use serde::{Deserialize, Serialize};

use super::{UiAction, View, materials, outliner, right, theme};

/// The tab shown in the dock's top area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockTab {
    #[default]
    EntityInfo,
    Materials,
}

impl DockTab {
    fn name(self) -> &'static str {
        match self {
            DockTab::EntityInfo => "Entity Info",
            DockTab::Materials => "Materials",
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
/// The least height of the top area and of the Outliner area, when both fit.
pub const AREA_MIN: f32 = 160.0;
/// The divider's height.
pub const DIVIDER: f32 = 8.0;
/// The dock's inner margin.
pub const DOCK_MARGIN: i8 = 12;

/// The top area's height for `split` of `height`, keeping both areas AREA_MIN when they fit.
pub fn top_height(split: f32, height: f32) -> f32 {
    if height < 2.0 * AREA_MIN {
        split * height
    } else {
        (split * height).clamp(AREA_MIN, height - AREA_MIN)
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
    ui.horizontal(|ui| {
        for tab in [DockTab::EntityInfo, DockTab::Materials] {
            let on = view.dock.tab == tab;
            if ui.add(egui::Button::selectable(on, tab.name())).clicked() && !on {
                layout.tab = tab;
            }
        }
    });
    let width = ui.available_width();
    let height = (ui.available_height() - DIVIDER).max(0.0);
    // The divider's drag, read before the areas are laid out so they follow the pointer in
    // the same frame.
    let divider_id = Id::new("dock-divider");
    let committed = top_height(view.dock.split, height);
    let mut split = ui
        .data(|d| d.get_temp::<f32>(split_id()))
        .unwrap_or(view.dock.split);
    if let Some(r) = ui.ctx().read_response(divider_id) {
        // While dragged: the top area's committed height moved as far as the pointer has.
        if r.dragged()
            && height > 0.0
            && let (Some(pos), Some(origin)) = (
                r.interact_pointer_pos(),
                ui.input(|i| i.pointer.press_origin()),
            )
        {
            split = ((committed + pos.y - origin.y) / height).clamp(*SPLIT.start(), *SPLIT.end());
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
    egui::ScrollArea::vertical()
        .id_salt("dock-top")
        .auto_shrink([false, false])
        .max_height(top_height(split, height))
        .show(ui, |ui| match view.dock.tab {
            DockTab::EntityInfo => right::entity_info(ui, view, actions),
            DockTab::Materials => materials::materials(ui, view, actions),
        });
    divider(ui, divider_id, width);
    outliner::outliner(ui, view, actions);
    layout
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
