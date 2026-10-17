//! The Outliner: the placed groups and components as a tree.

use egui::collapsing_header::CollapsingState;
use egui::{RichText, Ui, Vec2};
use kernel::{DefinitionKind, EntityId};
use session::OutlinerRow;

use super::{UiAction, View};

/// The least height the Outliner's list keeps when the panel is short, in points.
pub(super) const OUTLINER_MIN_HEIGHT: f32 = 120.0;

/// The Outliner: a Model row (back to the root) and the tree of groups and components, the
/// selected ones highlighted and the open ones bold. A click selects a node, a double-click
/// opens it. Only expanded nodes' children are laid out.
pub(super) fn outliner(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    ui.heading("Outliner");
    expand_open_path(ui.ctx(), view.context);
    #[cfg(test)]
    ui.data_mut(|d| d.remove::<Vec<super::tests::Recorded>>(super::tests::recorded_id()));
    egui::ScrollArea::vertical()
        .id_salt("outliner")
        .auto_shrink([false, true])
        .min_scrolled_height(OUTLINER_MIN_HEIGHT)
        .show(ui, |ui| {
            let at_root = view.context.is_empty();
            let mut model = RichText::new("Model");
            if at_root {
                model = model.strong();
            }
            let row = row_button(ui, false, model);
            #[cfg(test)]
            super::tests::record(ui, &[], row.rect, false, false, at_root);
            // Already at the root: nothing to do (and the tool keeps its operation).
            if row.clicked() && !at_root {
                actions.push(UiAction::OpenPath(Vec::new()));
            }
            let mut path = Vec::new();
            for row in &view.outliner.roots {
                outliner_node(ui, view, row, &mut path, actions);
            }
        });
}

/// The collapsing state's id of the node at `path`.
fn outliner_id(path: &[EntityId]) -> egui::Id {
    egui::Id::new(("outliner", path))
}

/// Expands the nodes along the open path when it changed since the last frame, so the open
/// instance and its contents show.
fn expand_open_path(ctx: &egui::Context, context: &[EntityId]) {
    let key = egui::Id::new("outliner-context");
    if ctx.data(|d| d.get_temp::<Vec<EntityId>>(key)).as_deref() == Some(context) {
        return;
    }
    for end in 1..=context.len() {
        let mut state =
            CollapsingState::load_with_default_open(ctx, outliner_id(&context[..end]), true);
        state.set_open(true);
        state.store(ctx);
    }
    ctx.data_mut(|d| d.insert_temp(key, context.to_vec()));
}

/// One node and, when expanded, its children. `path` leads to the node's parent.
fn outliner_node(
    ui: &mut Ui,
    view: &View,
    row: &OutlinerRow,
    path: &mut Vec<EntityId>,
    actions: &mut Vec<UiAction>,
) {
    let selected = path.as_slice() == view.context && view.selection.contains(&row.id);
    path.push(row.id);
    let open = view.context.starts_with(path);
    let children = view.outliner.children(row.definition);
    if children.is_empty() {
        let clicked = ui
            .horizontal(|ui| {
                // Lined up with the names of nodes that have an expand button.
                ui.add_space(ui.spacing().indent);
                outliner_row(ui, row, path, selected, open)
            })
            .inner;
        actions.extend(clicked);
    } else {
        // Nodes on the open path start expanded.
        let state = CollapsingState::load_with_default_open(ui.ctx(), outliner_id(path), open);
        let (_toggle, header, _) = state
            .show_header(ui, |ui| outliner_row(ui, row, path, selected, open))
            .body(|ui| {
                for child in children {
                    outliner_node(ui, view, child, path, actions);
                }
            });
        #[cfg(test)]
        super::tests::record(ui, path, _toggle.rect, true, false, false);
        actions.extend(header.inner);
    }
    path.pop();
}

/// A node's name and what clicking it asks for. `path` leads to the node.
fn outliner_row(
    ui: &mut Ui,
    row: &OutlinerRow,
    path: &[EntityId],
    selected: bool,
    open: bool,
) -> Option<UiAction> {
    let mut text = RichText::new(outliner_label(row));
    if open {
        text = text.strong();
    }
    let response = row_button(ui, selected, text);
    #[cfg(test)]
    super::tests::record(ui, path, response.rect, false, selected, open);
    if response.double_clicked() {
        Some(UiAction::OpenPath(path.to_vec()))
    } else if response.clicked() {
        // Shift, or Cmd on macOS and Ctrl elsewhere. Ctrl is accepted on macOS too, though
        // the system may turn a Ctrl-click into a secondary click there.
        let toggle = ui.input(|i| i.modifiers.shift || i.modifiers.command || i.modifiers.ctrl);
        Some(UiAction::OutlinerSelect {
            path: path.to_vec(),
            toggle,
        })
    } else {
        None
    }
}

/// A selectable row as wide as the space left, its text on the left (cut short if too long),
/// named by its text alone for AccessKit.
fn row_button(ui: &mut Ui, selected: bool, text: RichText) -> egui::Response {
    let width = ui.available_width();
    let name = text.text().to_owned();
    let response = ui.add(
        egui::Button::selectable(selected, text)
            .right_text("")
            .truncate()
            .min_size(Vec2::new(width, 0.0)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            response.enabled(),
            selected,
            &name,
        )
    });
    response
}

/// How the Outliner names a node: a component as `<Name>`, a group by its name or as `Group`.
pub(super) fn outliner_label(row: &OutlinerRow) -> String {
    match row.kind {
        DefinitionKind::Component => format!("<{}>", row.name),
        DefinitionKind::Group if row.name.is_empty() => "Group".to_string(),
        DefinitionKind::Group => row.name.clone(),
    }
}
