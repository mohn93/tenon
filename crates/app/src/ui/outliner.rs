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
    // Only the rows in view are laid out; the flat list is rebuilt only when an input changes.
    let flat = flat_list(ui.ctx(), view, &query);
    let items = &flat.items;
    let row_height = ui.spacing().interact_size.y;
    let tree = egui::ScrollArea::vertical()
        .id_salt("outliner")
        .auto_shrink([false, false])
        .show_rows(ui, row_height, items.len(), |ui, range| {
            for index in range {
                match &items[index] {
                    Item::Model => {
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
                    }
                    Item::Note(text) => {
                        ui.weak(*text);
                    }
                    Item::Node(node) => {
                        let path = node_path(items, index);
                        let row = node.row.get(view.outliner);
                        draw_node(ui, view, node, row, &path, actions);
                    }
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

/// The flattened list and what it was built from.
struct FlatList {
    /// The table's stamp (`View::outliner_stamp`).
    stamp: (u64, u64),
    context: Vec<EntityId>,
    query: String,
    /// Bumped when a node is expanded or collapsed.
    toggles: u64,
    /// How many times a list was built, for tests.
    builds: u64,
    items: Vec<Item>,
}

/// Where the cached list lives in egui's memory.
fn flat_key() -> egui::Id {
    egui::Id::new("outliner-flat")
}

/// Where the count of expands and collapses lives in egui's memory.
fn toggles_key() -> egui::Id {
    egui::Id::new("outliner-toggles")
}

/// The flat list for `view` and `query`: the one kept from an earlier frame while the table,
/// the open path, the filter and the expanded nodes are as they were.
fn flat_list(ctx: &egui::Context, view: &View, query: &str) -> std::sync::Arc<FlatList> {
    let toggles = ctx.data(|d| d.get_temp::<u64>(toggles_key())).unwrap_or(0);
    let kept = ctx.data(|d| d.get_temp::<std::sync::Arc<FlatList>>(flat_key()));
    if let Some(kept) = &kept
        && kept.stamp == view.outliner_stamp
        && kept.toggles == toggles
        && kept.query == query
        && kept.context == view.context
    {
        return kept.clone();
    }
    let mut filtering =
        Filter::new(view.outliner, query).map(|filter| Filtering { filter, drawn: 0 });
    let list = std::sync::Arc::new(FlatList {
        stamp: view.outliner_stamp,
        context: view.context.to_vec(),
        query: query.to_owned(),
        toggles,
        builds: kept.map_or(0, |k| k.builds) + 1,
        items: flatten(ctx, view, &mut filtering),
    });
    ctx.data_mut(|d| d.insert_temp(flat_key(), list.clone()));
    list
}

/// How many flat lists were built, for tests.
#[cfg(test)]
pub(super) fn builds(ctx: &egui::Context) -> u64 {
    ctx.data(|d| d.get_temp::<std::sync::Arc<FlatList>>(flat_key()))
        .map_or(0, |k| k.builds)
}

/// A row of the table: of the roots (no definition) or of a definition's children.
#[derive(Clone, Copy)]
struct RowRef(Option<DefinitionId>, usize);

impl RowRef {
    fn get(self, table: &OutlinerTable) -> &OutlinerRow {
        match self.0 {
            None => &table.roots[self.1],
            Some(definition) => &table.children(definition)[self.1],
        }
    }
}

/// A line of the flattened Outliner.
enum Item {
    /// The Model row.
    Model,
    /// A node.
    Node(Node),
    /// A weak line of text ("No matches", ...).
    Note(&'static str),
}

/// A node of the flattened tree.
struct Node {
    row: RowRef,
    id: EntityId,
    /// Nodes above it.
    depth: usize,
    /// The index of the parent's item.
    parent: Option<usize>,
    /// Its own flag or tag, or one above it, hides it.
    hidden: bool,
    /// On the open path.
    open: bool,
    /// For a node with children shown: the state's id, its default and whether it is expanded.
    toggle: Option<(egui::Id, bool, bool)>,
}

/// The tree as drawn: the Model row, then every visible node in order, expanded nodes
/// followed by their children. With a filter, a node that does not show is skipped, one
/// holding rows that show is expanded (under its own state, leaving the one kept without a
/// filter), and nothing is listed past `MAX_FILTERED_ROWS` rows (the count goes one past it to
/// say so).
fn flatten(ctx: &egui::Context, view: &View, filtering: &mut Option<Filtering>) -> Vec<Item> {
    let mut items = Vec::with_capacity(view.outliner.roots.len() + 2);
    items.push(Item::Model);
    let mut flat = Flat {
        ctx,
        view,
        filtering,
        items,
    };
    let mut path = Vec::new();
    for (i, row) in view.outliner.roots.iter().enumerate() {
        flat.node(row, RowRef(None, i), &mut path, None, false);
    }
    if let Some(f) = flat.filtering.as_ref() {
        if f.drawn == 0 {
            flat.items.push(Item::Note("No matches"));
        } else if f.drawn > MAX_FILTERED_ROWS {
            flat.items
                .push(Item::Note("More matches: refine the filter"));
        }
    }
    flat.items
}

/// What flattening the tree works with.
struct Flat<'a, 'b> {
    ctx: &'b egui::Context,
    view: &'a View<'a>,
    filtering: &'b mut Option<Filtering>,
    items: Vec<Item>,
}

impl Flat<'_, '_> {
    /// One node and, when expanded, its children. `path` leads to the node's parent.
    fn node(
        &mut self,
        row: &OutlinerRow,
        at: RowRef,
        path: &mut Vec<EntityId>,
        parent: Option<usize>,
        hidden: bool,
    ) {
        let view = self.view;
        let mut opens = false;
        if let Some(f) = self.filtering.as_mut() {
            if !f.filter.shows(row) || f.drawn > MAX_FILTERED_ROWS {
                return;
            }
            f.drawn += 1;
            if f.drawn > MAX_FILTERED_ROWS {
                return;
            }
            opens = f.filter.opens(row);
        }
        let filtered = self.filtering.is_some();
        let hidden = hidden || row.hidden;
        path.push(row.id);
        let open = view.context.starts_with(path);
        let children = view.outliner.children(row.definition);
        // A filtered node shows its children only when some of them show.
        let leaf = children.is_empty() || (filtered && !opens);
        let mut toggle = None;
        if !leaf {
            // Nodes on the open path start expanded; with a filter, every node shown with
            // children does.
            let (id, default) = if filtered {
                (filtered_id(self.ctx, path), true)
            } else {
                (outliner_id(path), open)
            };
            let expanded = CollapsingState::load_with_default_open(self.ctx, id, default).is_open();
            toggle = Some((id, default, expanded));
        }
        let index = self.items.len();
        self.items.push(Item::Node(Node {
            row: at,
            id: row.id,
            depth: path.len() - 1,
            parent,
            hidden,
            open,
            toggle,
        }));
        if toggle.is_some_and(|(_, _, expanded)| expanded) {
            for (i, child) in children.iter().enumerate() {
                self.node(
                    child,
                    RowRef(Some(row.definition), i),
                    path,
                    Some(index),
                    hidden,
                );
            }
        }
        path.pop();
    }
}

/// The path to the node at `index`.
fn node_path(items: &[Item], index: usize) -> Vec<EntityId> {
    let mut path = Vec::new();
    let mut at = Some(index);
    while let Some(i) = at {
        let Item::Node(node) = &items[i] else { break };
        path.push(node.id);
        at = node.parent;
    }
    path.reverse();
    path
}

/// One node's line: indent, expand button or the space for one, and the name.
fn draw_node(
    ui: &mut Ui,
    view: &View,
    node: &Node,
    row: &OutlinerRow,
    path: &[EntityId],
    actions: &mut Vec<UiAction>,
) {
    let selected = path[..path.len() - 1] == *view.context && view.selection.contains(&row.id);
    let clicked = ui
        .horizontal(|ui| {
            let indent = ui.spacing().indent;
            // As `CollapsingState::show_header` did: the name follows the button flush.
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(indent * node.depth as f32);
            match node.toggle {
                Some((id, default, expanded)) => {
                    let (_, toggle) = ui.allocate_exact_size(
                        Vec2::new(indent, ui.spacing().icon_width),
                        egui::Sense::click(),
                    );
                    let openness = if expanded { 1.0 } else { 0.0 };
                    egui::collapsing_header::paint_default_icon(ui, openness, &toggle);
                    let verb = if expanded { "Collapse" } else { "Expand" };
                    toggle.widget_info(|| {
                        let name = format!("{verb} {}", display_name(row));
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name)
                    });
                    #[cfg(test)]
                    super::tests::record(ui, path, toggle.rect, true, false, false);
                    if toggle.clicked() {
                        let mut state =
                            CollapsingState::load_with_default_open(ui.ctx(), id, default);
                        state.set_open(!expanded);
                        state.store(ui.ctx());
                        ui.ctx().data_mut(|d| {
                            *d.get_temp_mut_or_default::<u64>(toggles_key()) += 1;
                        });
                        ui.ctx().request_repaint();
                    }
                }
                // Lined up with the names of nodes that have an expand button.
                None => {
                    ui.add_space(indent);
                }
            }
            outliner_row(ui, row, path, selected, node.open, node.hidden)
        })
        .inner;
    actions.extend(clicked);
}

/// A node's name and what clicking it asks for. `path` leads to the node.
fn outliner_row(
    ui: &mut Ui,
    row: &OutlinerRow,
    path: &[EntityId],
    selected: bool,
    open: bool,
    hidden: bool,
) -> Option<UiAction> {
    let mut text = RichText::new(outliner_label(row));
    if open {
        text = text.strong();
    }
    if hidden {
        text = text.weak();
    }
    let response = row_button(ui, selected, text);
    #[cfg(test)]
    super::tests::record(ui, path, response.rect, false, selected, open);
    let mut menu = None;
    response.context_menu(|ui| {
        let (label, hide) = if row.own_hidden {
            ("Unhide", false)
        } else {
            ("Hide", true)
        };
        if widgets::menu_item(ui, label, None, true).clicked() {
            menu = Some(UiAction::OutlinerSetHidden {
                path: path.to_vec(),
                hidden: hide,
            });
            ui.close();
        }
    });
    if menu.is_some() {
        menu
    } else if response.double_clicked() {
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
