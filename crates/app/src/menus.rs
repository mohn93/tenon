//! The menu bar as data: one table built from the panels' `View`, drawn as egui menus by
//! `ui::menu_bar` and, on macOS, as the native menu bar by `native_menu`.

use std::path::{Path, PathBuf};

use session::{Lighting, Projection, StandardView};
use tools::ToolKind;

use crate::shortcuts::Bound;
use crate::ui::theme::ThemeChoice;
use crate::ui::{FLY_HELP, UiAction, View, step_name};

/// The title of the native app menu (macOS), which holds Settings... and Quit.
pub const APP_MENU: &str = "sketchup-alt";

/// The File submenu that lists the recent files: the one submenu whose items change.
pub const OPEN_RECENT: &str = "Open Recent";

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
    pub tooltip: Option<String>,
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

    fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
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

/// The menu bar. `native`: macOS's, with an app menu first holding Settings... and Quit (File
/// and Edit then have neither); otherwise Settings... ends Edit and Quit ends File.
/// The same items exist in every state of `view`; only labels, checks and enabled flags change.
pub fn menu_bar(view: &View, native: bool) -> Vec<Menu> {
    let mut file = vec![
        bound_item("New", Bound::NewModel, UiAction::NewModel).entry(),
        bound_item("Open...", Bound::OpenFile, UiAction::OpenFile).entry(),
        Entry::Submenu {
            label: OPEN_RECENT,
            entries: recent_entries(view.recent),
        },
        bound_item("Save", Bound::SaveFile, UiAction::SaveFile).entry(),
        bound_item("Save As...", Bound::SaveFileAs, UiAction::SaveFileAs).entry(),
        Entry::Separator,
        bound_item("Import...", Bound::ImportFile, UiAction::ImportFile).entry(),
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
    let settings =
        || bound_item("Settings...", Bound::Settings, UiAction::ShowSettings(true)).entry();
    let mut edit = vec![
        bound_item(&step("Undo", view.undo), Bound::Undo, UiAction::Undo)
            .enabled(view.undo.is_some())
            .entry(),
        bound_item(&step("Redo", view.redo), Bound::Redo, UiAction::Redo)
            .enabled(view.redo.is_some())
            .entry(),
        Entry::Separator,
        bound_item("Cut", Bound::Cut, UiAction::Cut)
            .enabled(view.has_selection)
            .entry(),
        bound_item("Copy", Bound::Copy, UiAction::Copy)
            .enabled(view.has_selection)
            .entry(),
        bound_item("Paste", Bound::Paste, UiAction::Paste)
            .enabled(view.has_clip)
            .entry(),
        Item::new("Paste In Place", UiAction::PasteInPlace)
            .enabled(view.has_clip)
            .entry(),
        Entry::Separator,
        bound_item("Select All", Bound::SelectAll, UiAction::SelectAll).entry(),
        bound_item("Select None", Bound::SelectNone, UiAction::SelectNone)
            .enabled(view.has_selection)
            .entry(),
        bound_item("Duplicate", Bound::Duplicate, UiAction::Duplicate)
            .enabled(view.has_selection)
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
    if !native {
        edit.push(Entry::Separator);
        edit.push(settings());
    }

    let draw = [
        ToolKind::Line,
        ToolKind::Rectangle,
        ToolKind::Circle,
        ToolKind::Polygon,
        ToolKind::Arc,
    ]
    .map(tool_item)
    .into();
    let tools = [
        ToolKind::Select,
        ToolKind::PushPull,
        ToolKind::Move,
        ToolKind::Rotate,
        ToolKind::Scale,
        ToolKind::Offset,
        ToolKind::FollowMe,
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
            "Make Component...",
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
        Entry::Separator,
        bound_item(
            "Rename Component...",
            Bound::RenameComponent,
            UiAction::RenameComponent,
        )
        .enabled(view.rename.is_some())
        .entry(),
        // Always enabled: knowing whether anything is unused would mean counting instances
        // every frame.
        Item::new("Purge Unused", UiAction::PurgeUnused).entry(),
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
    // In a parallel view the field of view only sizes the view: changing it would zoom.
    camera.push(
        Item::new("Field of View...", UiAction::ShowFieldOfView)
            .enabled(view.projection == Projection::Perspective)
            .entry(),
    );
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
        Entry::Separator,
        Item::new("Add Scene", UiAction::AddScene).entry(),
        bound_item(
            "Previous Scene",
            Bound::PreviousScene,
            UiAction::PreviousScene,
        )
        .enabled(!view.scenes.is_empty())
        .entry(),
        bound_item("Next Scene", Bound::NextScene, UiAction::NextScene)
            .enabled(!view.scenes.is_empty())
            .entry(),
    ];

    let help = vec![Item::new("Keyboard Shortcuts", UiAction::ShowShortcuts(true)).entry()];

    let mut menus = Vec::new();
    if native {
        menus.push(Menu {
            title: APP_MENU,
            // Settings..., then Quit, which macOS names with the app, as the predefined About
            // does. `native_menu` puts its standard items around them.
            entries: vec![
                settings(),
                bound_item("Quit sketchup-alt", Bound::Quit, UiAction::Quit).entry(),
            ],
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

/// The Open Recent rows, newest first, each with its full path as tooltip, then Clear Menu
/// (disabled when there is nothing to clear).
fn recent_entries(recent: &[PathBuf]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = recent
        .iter()
        .map(|p| {
            Item::new(recent_label(p, recent), UiAction::OpenRecent(p.clone()))
                .tooltip(p.display().to_string())
                .entry()
        })
        .collect();
    if !entries.is_empty() {
        entries.push(Entry::Separator);
    }
    entries.push(
        Item::new("Clear Menu", UiAction::ClearRecent)
            .enabled(!recent.is_empty())
            .entry(),
    );
    entries
}

/// A recent file's menu label: its file name, with " — <parent folder>" when another
/// entry in `recent` has the same file name.
pub fn recent_label(path: &Path, recent: &[PathBuf]) -> String {
    let name = path.file_name().unwrap_or(path.as_os_str());
    let label = name.to_string_lossy().into_owned();
    let shared = recent
        .iter()
        .any(|p| p != path && p.file_name() == Some(name));
    match path.parent().and_then(Path::file_name) {
        Some(folder) if shared => format!("{label} \u{2014} {}", folder.to_string_lossy()),
        _ => label,
    }
}

/// The entries of the Open Recent submenu in `bar`.
#[cfg(any(target_os = "macos", test))]
pub fn recent_entries_in(bar: &[Menu]) -> Option<&[Entry]> {
    fn find(entries: &[Entry]) -> Option<&[Entry]> {
        entries.iter().find_map(|entry| match entry {
            Entry::Submenu { label, entries } if *label == OPEN_RECENT => Some(entries.as_slice()),
            Entry::Submenu { entries, .. } => find(entries),
            _ => None,
        })
    }
    bar.iter().find_map(|m| find(&m.entries))
}

/// The paths the Open Recent submenu lists in `bar`, in order.
#[cfg(any(target_os = "macos", test))]
pub fn recent_paths(bar: &[Menu]) -> Vec<PathBuf> {
    recent_paths_of(recent_entries_in(bar).unwrap_or_default())
}

/// The paths of the Open Recent rows among `entries`, in order.
#[cfg(any(target_os = "macos", test))]
pub fn recent_paths_of(entries: &[Entry]) -> Vec<PathBuf> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Item(Item {
                action: UiAction::OpenRecent(path),
                ..
            }) => Some(path.clone()),
            _ => None,
        })
        .collect()
}

/// Every item, depth first.
#[cfg(any(target_os = "macos", test))]
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
#[cfg(any(target_os = "macos", test))]
pub fn item_id(action: &UiAction) -> String {
    match action {
        UiAction::SetLighting(_) => "ToggleNightLighting".into(),
        _ => format!("{action:?}"),
    }
}

/// Whether a native item is enabled: as in the table, except those a text field takes while it
/// has focus (`yields_to_text`), so Cmd+Z and Cmd+A reach the field. While a sheet is open only
/// Quit is.
#[cfg(any(target_os = "macos", test))]
pub fn native_enabled(item: &Item, text_focused: bool, sheet_open: bool) -> bool {
    if sheet_open {
        return item.action == UiAction::Quit;
    }
    item.enabled && !(text_focused && yields_to_text(&item.action))
}

/// Whether a menu event raised by a held key acts: only for bindings that repeat.
#[cfg(any(target_os = "macos", test))]
pub fn accepts_repeat(action: &UiAction) -> bool {
    crate::shortcuts::TABLE
        .iter()
        .any(|s| s.repeats && s.bound.action().as_ref() == Some(action))
}

/// Whether a native menu event for `action` acts. `repeat`: a held key raised it.
/// `text_focused`: a text field has focus now, so Undo, Redo, Select All and Select None are
/// the field's (the item may have been enabled when the key went down). `sheet_open`: a sheet
/// is open, so only Quit acts (an event raced past the items being disabled).
#[cfg(any(target_os = "macos", test))]
pub fn native_event_acts(
    action: &UiAction,
    repeat: bool,
    text_focused: bool,
    sheet_open: bool,
) -> bool {
    if sheet_open && *action != UiAction::Quit {
        return false;
    }
    !(repeat && !accepts_repeat(action)) && !(text_focused && yields_to_text(action))
}

/// Whether a focused text field takes the action's chord for itself: undo and redo its
/// typing, select all or none of its text, cut, copy and paste text. Duplicate is not a
/// field's (it has no Cmd+D); Paste In Place has no key but is the field's too, so no item
/// pastes geometry while a field is being typed in.
#[cfg(any(target_os = "macos", test))]
fn yields_to_text(action: &UiAction) -> bool {
    matches!(
        action,
        UiAction::Undo
            | UiAction::Redo
            | UiAction::SelectAll
            | UiAction::SelectNone
            | UiAction::Cut
            | UiAction::Copy
            | UiAction::Paste
            | UiAction::PasteInPlace
    )
}
