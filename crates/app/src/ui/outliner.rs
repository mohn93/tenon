//! The Outliner: the placed groups and components as a tree, filtered by name.

use std::collections::{HashMap, HashSet};

use egui::collapsing_header::CollapsingState;
use egui::{RichText, Ui, Vec2};
use kernel::{DefinitionId, DefinitionKind, EntityId};
use session::{OutlinerRow, OutlinerTable};

use super::{UiAction, View, icons, theme, widgets};

/// The most rows a filtered Outliner draws.
pub(super) const MAX_FILTERED_ROWS: usize = 500;

/// The Outliner's filter: rows whose shown name contains the query, ignoring case, and the
/// rows that hold them.
pub(super) struct Filter {
    /// The query, trimmed and lowercase.
    query: String,
    /// The definitions that place a matching row, at any depth.
    inside: HashSet<DefinitionId>,
}

impl Filter {
    /// None for a blank query.
    pub fn new(table: &OutlinerTable, query: &str) -> Option<Filter> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return None;
        }
        let mut filter = Filter {
            query,
            inside: HashSet::new(),
        };
        // Each definition is walked once, whatever number of instances place it.
        let mut seen = HashMap::new();
        for row in &table.roots {
            filter.walk(table, row.definition, &mut seen);
        }
        filter.inside = seen
            .into_iter()
            .filter_map(|(definition, inside)| inside.then_some(definition))
            .collect();
        Some(filter)
    }

    /// Whether something placed in `definition` matches, at any depth; memoized in `seen`.
    fn walk(
        &self,
        table: &OutlinerTable,
        definition: DefinitionId,
        seen: &mut HashMap<DefinitionId, bool>,
    ) -> bool {
        if let Some(&inside) = seen.get(&definition) {
            return inside;
        }
        // Not inside until shown otherwise; also ends a cycle, should a table have one.
        seen.insert(definition, false);
        let mut inside = false;
        for child in table.children(definition) {
            // Every child is walked, so each definition below is decided once.
            let below = self.walk(table, child.definition, seen);
            inside |= below || self.matches(child);
        }
        seen.insert(definition, inside);
        inside
    }

    /// The row's own name matches.
    pub fn matches(&self, row: &OutlinerRow) -> bool {
        display_name(row).to_lowercase().contains(&self.query)
    }

    /// The row shows: it matches, or something placed in it (at any depth) does.
    pub fn shows(&self, row: &OutlinerRow) -> bool {
        self.opens(row) || self.matches(row)
    }

    /// Something placed in the row's definition shows (draw it expanded).
    pub fn opens(&self, row: &OutlinerRow) -> bool {
        self.inside.contains(&row.definition)
    }
}

/// What drawing the tree with a filter needs: the filter and the rows drawn so far.
struct Filtering {
    filter: Filter,
    drawn: usize,
}

/// The Outliner: a filter field, a Model row (back to the root) and the tree of groups and
/// components, the selected ones highlighted and the open ones bold. A click selects a node, a
/// double-click opens it. Only expanded nodes' children are laid out. While a filter is typed,
/// only the matching rows and the rows holding them show, those drawn expanded, up to
/// `MAX_FILTERED_ROWS`. The list fills the height left and scrolls.
pub(super) fn outliner(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    widgets::section_header(ui, "Outliner");
    expand_open_path(ui.ctx(), view.context);
    #[cfg(test)]
    ui.data_mut(|d| d.remove::<Vec<super::tests::Recorded>>(super::tests::recorded_id()));
    let before = ui.data(|d| d.get_temp::<String>(filter_key()));
    let query = filter_field(ui);
    if before.as_deref().unwrap_or_default() != query {
        forget_filtered_states(ui.ctx());
    }
    let mut filtering =
        Filter::new(view.outliner, &query).map(|filter| Filtering { filter, drawn: 0 });
    let tree = egui::ScrollArea::vertical()
        .id_salt("outliner")
        .auto_shrink([false, false])
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
                outliner_node(ui, view, row, &mut path, &mut filtering, actions);
            }
            if let Some(f) = &filtering {
                if f.drawn == 0 {
                    ui.weak("No matches");
                } else if f.drawn > MAX_FILTERED_ROWS {
                    ui.weak("More matches: refine the filter");
                }
            }
        });
    widgets::name_scroll_bars(ui.ctx(), tree.id, "Outliner");
}

/// The filter field, named "Filter outliner", and a Clear filter button while it holds text.
/// Returns the text; it lives in egui's memory.
fn filter_field(ui: &mut Ui) -> String {
    let key = filter_key();
    let mut text = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_default();
    let had_text = !text.is_empty();
    ui.horizontal(|ui| {
        let button = if had_text {
            theme::CONTROL_HEIGHT + ui.spacing().item_spacing.x
        } else {
            0.0
        };
        let edit = ui.add(
            egui::TextEdit::singleline(&mut text)
                .hint_text("Filter")
                .desired_width((ui.available_width() - button).max(0.0)),
        );
        edit.widget_info(|| {
            let mut info =
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Filter outliner");
            info.current_text_value = Some(text.clone());
            info
        });
        if had_text
            && widgets::icon_button(
                ui,
                icons::X_CIRCLE,
                "Clear filter",
                "Clear filter",
                None,
                true,
            )
            .clicked()
        {
            text.clear();
        }
    });
    if text.is_empty() {
        ui.data_mut(|d| d.remove::<String>(key));
    } else {
        ui.data_mut(|d| d.insert_temp(key, text.clone()));
    }
    text
}

/// Where the filter text lives in egui's memory.
fn filter_key() -> egui::Id {
    egui::Id::new("outliner-filter")
}

/// Where the ids of the collapsing states used under a filter live in egui's memory.
fn filtered_ids_key() -> egui::Id {
    egui::Id::new("outliner-filtered-ids")
}

/// The collapsing state's id of the node at `path` while a filter is typed, noted so
/// `forget_filtered_states` can drop it.
fn filtered_id(ctx: &egui::Context, path: &[EntityId]) -> egui::Id {
    let id = egui::Id::new(("outliner-filtered", path));
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<HashSet<egui::Id>>(filtered_ids_key())
            .insert(id)
    });
    id
}

/// Drops the collapsing states used under a filter, so a node collapsed under one filter
/// shows expanded again under the next.
fn forget_filtered_states(ctx: &egui::Context) {
    let ids = ctx.data_mut(|d| d.remove_temp::<HashSet<egui::Id>>(filtered_ids_key()));
    for id in ids.unwrap_or_default() {
        if let Some(state) = CollapsingState::load(ctx, id) {
            state.remove(ctx);
        }
    }
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

/// One node and, when expanded, its children. `path` leads to the node's parent. With a
/// filter, a node that does not show is skipped, one holding rows that show is drawn expanded
/// (under its own state, leaving the one kept without a filter), and nothing is drawn past
/// `MAX_FILTERED_ROWS` rows (the count goes one past it to say so).
fn outliner_node(
    ui: &mut Ui,
    view: &View,
    row: &OutlinerRow,
    path: &mut Vec<EntityId>,
    filtering: &mut Option<Filtering>,
    actions: &mut Vec<UiAction>,
) {
    let mut opens = false;
    if let Some(f) = filtering {
        if !f.filter.shows(row) || f.drawn > MAX_FILTERED_ROWS {
            return;
        }
        f.drawn += 1;
        if f.drawn > MAX_FILTERED_ROWS {
            return;
        }
        opens = f.filter.opens(row);
    }
    let selected = path.as_slice() == view.context && view.selection.contains(&row.id);
    path.push(row.id);
    let open = view.context.starts_with(path);
    let children = view.outliner.children(row.definition);
    // A filtered node shows its children only when some of them show.
    let leaf = children.is_empty() || (filtering.is_some() && !opens);
    if leaf {
        let clicked = ui
            .horizontal(|ui| {
                // Lined up with the names of nodes that have an expand button.
                ui.add_space(ui.spacing().indent);
                outliner_row(ui, row, path, selected, open)
            })
            .inner;
        actions.extend(clicked);
    } else {
        // Nodes on the open path start expanded; with a filter, every node shown with
        // children does.
        let state = if filtering.is_some() {
            CollapsingState::load_with_default_open(ui.ctx(), filtered_id(ui.ctx(), path), true)
        } else {
            CollapsingState::load_with_default_open(ui.ctx(), outliner_id(path), open)
        };
        let expanded = state.is_open();
        let (toggle, header, _) = state
            .show_header(ui, |ui| outliner_row(ui, row, path, selected, open))
            .body(|ui| {
                for child in children {
                    outliner_node(ui, view, child, path, filtering, actions);
                }
            });
        let verb = if expanded { "Collapse" } else { "Expand" };
        let name = format!("{verb} {}", display_name(row));
        toggle.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
        #[cfg(test)]
        super::tests::record(ui, path, toggle.rect, true, false, false);
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
        DefinitionKind::Group => display_name(row).to_string(),
    }
}

/// A node's name without the Outliner's angle brackets: "Group" for an unnamed group.
pub(super) fn display_name(row: &OutlinerRow) -> &str {
    match row.kind {
        DefinitionKind::Group if row.name.is_empty() => "Group",
        _ => &row.name,
    }
}
