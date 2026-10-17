//! The menu bar as data: one table built from the panels' `View`, drawn as egui menus by
//! `ui::menu_bar` (and, later, as the native menu bar).

use session::{Lighting, Projection, StandardView};
use tools::ToolKind;

use crate::editor::ImportUnits;
use crate::shortcuts::Bound;
use crate::ui::theme::ThemeChoice;
use crate::ui::{FLY_HELP, UiAction, View, step_name};

/// The title of the native app menu (macOS), which holds Quit.
#[cfg_attr(not(test), allow(dead_code))]
pub const APP_MENU: &str = "sketchup-alt";

/// One menu of the bar.
pub struct Menu {
    pub title: &'static str,
    pub entries: Vec<Entry>,
}

pub enum Entry {
    Item(Item),
    Separator,
    Submenu {
        label: &'static str,
        entries: Vec<Entry>,
    },
}

pub struct Item {
    pub label: String,
    pub action: UiAction,
    pub bound: Option<Bound>,
    pub enabled: bool,
    /// Some for an item drawn with a check (a choice or a mode).
    pub checked: Option<bool>,
    pub tooltip: Option<&'static str>,
}

impl Item {
    fn new(label: impl Into<String>, action: UiAction) -> Self {
        Self {
            label: label.into(),
            action,
            bound: None,
            enabled: true,
            checked: None,
            tooltip: None,
        }
    }

    fn bound(mut self, bound: Bound) -> Self {
        self.bound = Some(bound);
        self
    }

    fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    fn tooltip(mut self, tooltip: &'static str) -> Self {
        self.tooltip = Some(tooltip);
        self
    }

    fn entry(self) -> Entry {
        Entry::Item(self)
    }
}

/// An item that shows the key bound to `bound` and does the binding's action.
fn bound_item(label: &str, bound: Bound, action: UiAction) -> Item {
    Item::new(label, action).bound(bound)
}

/// A tool's item, named by the tool.
fn tool_item(kind: ToolKind) -> Entry {
    bound_item(kind.label(), Bound::Tool(kind), UiAction::Tool(kind)).entry()
}

/// The menu bar. `native`: macOS's, with an app menu first holding Quit (File then has none).
/// The same items exist in every state of `view`; only labels, checks and enabled flags change.
pub fn menu_bar(view: &View, native: bool) -> Vec<Menu> {
    let mut file = vec![
        bound_item("New", Bound::NewModel, UiAction::NewModel).entry(),
        bound_item("Open...", Bound::OpenFile, UiAction::OpenFile).entry(),
        bound_item("Save", Bound::SaveFile, UiAction::SaveFile).entry(),
        bound_item("Save As...", Bound::SaveFileAs, UiAction::SaveFileAs).entry(),
        Entry::Separator,
        bound_item("Import...", Bound::ImportFile, UiAction::ImportFile).entry(),
        Entry::Submenu {
            label: "Import Units",
            entries: ImportUnits::ALL
                .into_iter()
                .map(|units| {
                    Item::new(units.label(), UiAction::SetImportUnits(units))
                        .checked(view.import_units == units)
                        .entry()
                })
                .collect(),
        },
        bound_item("Export...", Bound::ExportFile, UiAction::ExportFile).entry(),
    ];
    if !native {
        file.push(Entry::Separator);
        file.push(bound_item("Quit", Bound::Quit, UiAction::Quit).entry());
    }

    let step = |verb: &str, label: Option<&str>| match label {
        Some(l) => format!("{verb} {}", step_name(l)),
        None => verb.to_string(),
    };
    let edit = vec![
        bound_item(&step("Undo", view.undo), Bound::Undo, UiAction::Undo)
            .enabled(view.undo.is_some())
            .entry(),
        bound_item(&step("Redo", view.redo), Bound::Redo, UiAction::Redo)
            .enabled(view.redo.is_some())
            .entry(),
        Entry::Separator,
        bound_item("Erase", Bound::EraseSelection, UiAction::EraseSelection)
            .enabled(view.has_selection)
            .entry(),
        Item::new("Delete Guides", UiAction::DeleteGuides)
            .enabled(view.has_guides)
            .entry(),
        Entry::Separator,
        Item::new("Soften Edges...", UiAction::ShowSoften(true)).entry(),
    ];

    let draw = [
        ToolKind::Line,
        ToolKind::Rectangle,
        ToolKind::Circle,
        ToolKind::Arc,
    ]
    .map(tool_item)
    .into();
    let tools = [
        ToolKind::Select,
        ToolKind::PushPull,
        ToolKind::Move,
        ToolKind::Rotate,
        ToolKind::TapeMeasure,
        ToolKind::Erase,
        ToolKind::Paint,
    ]
    .map(tool_item)
    .into();

    let group = vec![
        bound_item("Make Group", Bound::MakeGroup, UiAction::MakeGroup)
            .enabled(view.has_selection)
            .entry(),
        bound_item(
            "Make Component",
            Bound::MakeComponent,
            UiAction::MakeComponent,
        )
        .enabled(view.has_selection)
        .entry(),
        Item::new("Explode", UiAction::Explode)
            .enabled(view.has_instance_selected)
            .entry(),
        bound_item("Close Group", Bound::CloseInstance, UiAction::CloseInstance)
            .enabled(view.inside_instance)
            .entry(),
    ];

    let mut camera = vec![
        bound_item("Zoom Extents", Bound::ZoomExtents, UiAction::ZoomExtents).entry(),
        Entry::Separator,
    ];
    camera.extend(
        StandardView::ALL
            .into_iter()
            .map(|v| Item::new(v.label(), UiAction::StandardView(v)).entry()),
    );
    camera.push(Entry::Separator);
    for (projection, label) in [
        (Projection::Perspective, "Perspective"),
        (Projection::Parallel, "Parallel Projection"),
    ] {
        camera.push(
            Item::new(label, UiAction::SetProjection(projection))
                .checked(view.projection == projection)
                .entry(),
        );
    }
    let night = view.lighting == Lighting::Night;
    let lighting = if night {
        Lighting::Day
    } else {
        Lighting::Night
    };
    camera.push(Entry::Separator);
    camera.push(
        Item::new("Night Lighting", UiAction::SetLighting(lighting))
            .checked(night)
            .entry(),
    );
    camera.push(Entry::Separator);
    camera.push(
        bound_item("Fly Mode", Bound::ToggleFly, UiAction::ToggleFly)
            .checked(view.flying)
            .tooltip(FLY_HELP)
            .entry(),
    );

    let theme = ThemeChoice::ALL
        .into_iter()
        .map(|choice| {
            Item::new(choice.label(), UiAction::SetTheme(choice))
                .checked(view.theme == choice)
                .entry()
        })
        .collect();
    let view_menu = vec![
        Entry::Submenu {
            label: "Theme",
            entries: theme,
        },
        Item::new("Agent Activity", UiAction::ShowAgentActivity(true)).entry(),
    ];

    let help = vec![Item::new("Keyboard Shortcuts", UiAction::ShowShortcuts(true)).entry()];

    let mut menus = Vec::new();
    if native {
        menus.push(Menu {
            title: APP_MENU,
            entries: vec![bound_item("Quit", Bound::Quit, UiAction::Quit).entry()],
        });
    }
    menus.extend([
        Menu {
            title: "File",
            entries: file,
        },
        Menu {
            title: "Edit",
            entries: edit,
        },
        Menu {
            title: "Draw",
            entries: draw,
        },
        Menu {
            title: "Tools",
            entries: tools,
        },
        Menu {
            title: "Group",
            entries: group,
        },
        Menu {
            title: "Camera",
            entries: camera,
        },
        Menu {
            title: "View",
            entries: view_menu,
        },
    ]);
    menus.push(Menu {
        title: "Help",
        entries: help,
    });
    menus
}

/// Every item, depth first.
#[cfg_attr(not(test), allow(dead_code))]
pub fn items(menus: &[Menu]) -> Vec<&Item> {
    fn walk<'a>(entries: &'a [Entry], out: &mut Vec<&'a Item>) {
        for entry in entries {
            match entry {
                Entry::Item(item) => out.push(item),
                Entry::Separator => {}
                Entry::Submenu { entries, .. } => walk(entries, out),
            }
        }
    }
    let mut out = Vec::new();
    for menu in menus {
        walk(&menu.entries, &mut out);
    }
    out
}

/// A stable id for the item: its action's Debug text. The lighting toggle's action flips
/// between day and night, so it has one id for both.
#[cfg_attr(not(test), allow(dead_code))]
pub fn item_id(action: &UiAction) -> String {
    match action {
        UiAction::SetLighting(_) => "ToggleNightLighting".into(),
        _ => format!("{action:?}"),
    }
}
