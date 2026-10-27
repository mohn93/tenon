//! The menu bar as data: one table built from the panels' `View`, drawn as egui menus by
//! `ui::menu_bar` and, on macOS, as the native menu bar by `native_menu`.

use std::path::{Path, PathBuf};

use session::{FlipAxis, FlipFrame, IntersectScope, Lighting, Projection, StandardView};
use tools::ToolKind;

use crate::editor::SolidOp;
use crate::shortcuts::Bound;
use crate::ui::theme::ThemeChoice;
use crate::ui::{EdgeEffect, FLY_HELP, LibrarySave, UiAction, View, step_name};

/// The title of the native app menu (macOS), which holds Settings... and Quit.
pub const APP_MENU: &str = "Tenon";

/// The File submenu that lists the recent files: the one submenu whose items change.
pub const OPEN_RECENT: &str = "Open Recent";

/// The axes Flip Along offers, by label.
pub const FLIP_AXES: [(&str, FlipAxis); 3] = [
    ("Red", FlipAxis::Red),
    ("Green", FlipAxis::Green),
    ("Blue", FlipAxis::Blue),
];

/// The scopes Intersect Faces offers, in the Edit menu and the viewport's right-click menu.
pub const INTERSECT_SCOPES: [(&str, IntersectScope); 3] = [
    ("With Model", IntersectScope::Model),
    ("With Selection", IntersectScope::Selection),
    ("With Context", IntersectScope::Context),
];

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
    let features = view.features;
    let mut file = vec![
        bound_item("New", Bound::NewModel, UiAction::NewModel).entry(),
        bound_item("Open...", Bound::OpenFile, UiAction::OpenFile).entry(),
    ];
    if features.recent {
        file.push(Entry::Submenu {
            label: OPEN_RECENT,
            entries: recent_entries(view.recent),
        });
    }
    if features.autosave {
        file.push(Item::new("Recover Models...", UiAction::ShowRecover(true)).entry());
    }
    file.extend([
        bound_item("Save", Bound::SaveFile, UiAction::SaveFile).entry(),
        bound_item("Save As...", Bound::SaveFileAs, UiAction::SaveFileAs).entry(),
        Entry::Separator,
        bound_item("Import...", Bound::ImportFile, UiAction::ImportFile).entry(),
        bound_item("Export...", Bound::ExportFile, UiAction::ExportFile).entry(),
    ]);
    if features.print {
        file.push(Item::new("Export PDF...", UiAction::ExportPdf).entry());
    }
    if features.library {
        file.push(
            Item::new(
                "Save Model to Library...",
                UiAction::SaveToLibrary(LibrarySave::Model),
            )
            .enabled(view.library.folder.is_some())
            .tooltip("Needs a library folder: Settings > Files")
            .entry(),
        );
    }
    if features.share {
        file.push(Entry::Separator);
        file.push(Item::new("Open on Phone...", UiAction::OpenOnPhone).entry());
    }
    if features.print {
        file.push(Entry::Separator);
        file.push(Item::new("Print...", UiAction::Print).entry());
    }
    if !native && features.quit {
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
        Item::new("Hide", UiAction::Hide)
            .enabled(view.selection_hidden == Some(false))
            .entry(),
        Entry::Submenu {
            label: "Unhide",
            entries: vec![
                Item::new("Selected", UiAction::UnhideSelected)
                    .enabled(view.has_hidden_selected)
                    .tooltip("Hidden entities can be selected while View > Hidden Geometry is on")
                    .entry(),
                Item::new("Last", UiAction::UnhideLast)
                    .enabled(view.has_last_hidden)
                    .entry(),
                Item::new("All", UiAction::UnhideAll)
                    .enabled(view.has_hidden)
                    .entry(),
            ],
        },
        Item::new("Reverse Faces", UiAction::ReverseFaces)
            .enabled(view.has_face_selected)
            .entry(),
        Item::new("Orient Faces", UiAction::OrientFaces(None))
            .enabled(view.has_face_selected)
            .tooltip("Turn joined faces to agree with the lowest-numbered selected one")
            .entry(),
        Entry::Separator,
        Item::new("Soften Edges...", UiAction::ShowSoften(true)).entry(),
        Entry::Submenu {
            label: "Intersect Faces",
            entries: INTERSECT_SCOPES
                .map(|(label, scope)| {
                    Item::new(label, UiAction::IntersectFaces(scope))
                        .enabled(view.has_selection)
                        .entry()
                })
                .into(),
        },
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
        ToolKind::Freehand,
        ToolKind::RotatedRectangle,
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
        ToolKind::Dimension,
        ToolKind::Text,
        ToolKind::SectionPlane,
        ToolKind::Axes,
        ToolKind::Erase,
        ToolKind::Paint,
        ToolKind::Protractor,
    ]
    .map(tool_item)
    .into_iter()
    .chain([
        Entry::Separator,
        Item::new("3D Text...", UiAction::Show3dText).entry(),
        Entry::Submenu {
            label: "Solid Tools",
            entries: SolidOp::ALL
                .map(|op| {
                    let state = view.solid_tools.get(op);
                    let item =
                        Item::new(op.label(), UiAction::SolidTool(op)).enabled(state.enabled);
                    match &state.tooltip {
                        Some(tip) => item.tooltip(tip.clone()),
                        None => item,
                    }
                    .entry()
                })
                .into(),
        },
        Item::new("Check Interference", UiAction::CheckInterference)
            .tooltip("Find selected parts that overlap (all parts when none is selected)")
            .entry(),
        Item::new("Cut List", UiAction::ShowCutList)
            .tooltip("List the parts with their sizes, materials and quantities")
            .entry(),
        Item::new("Model Health", UiAction::CheckHealth)
            .tooltip(
                "List selected parts that are not solids and why (all parts when none is selected)",
            )
            .entry(),
    ])
    .collect();

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
        Item::new("Make Unique", UiAction::MakeUnique)
            .enabled(view.can_make_unique)
            .entry(),
        Entry::Submenu {
            label: "Flip Along",
            entries: FLIP_AXES
                .iter()
                .map(|&(label, axis)| {
                    Item::new(label, UiAction::Flip(axis, FlipFrame::Axes))
                        .enabled(view.has_selection)
                        .entry()
                })
                .chain([Entry::Separator])
                .chain(FLIP_AXES.iter().map(|&(label, axis)| {
                    Item::new(
                        format!("Component's {label}"),
                        UiAction::Flip(axis, FlipFrame::Own),
                    )
                    .enabled(view.can_flip_own)
                    .entry()
                }))
                .collect(),
        },
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
    // Each lighting item toggles its own mode; unchecking one goes back to day.
    let lighting_item = |label: &str, mode: Lighting| {
        let on = view.lighting == mode;
        let target = if on { Lighting::Day } else { mode };
        Item::new(label, UiAction::SetLighting(target))
            .checked(on)
            .entry()
    };
    camera.push(Entry::Separator);
    camera.push(lighting_item("Night Lighting", Lighting::Night));
    camera.push(lighting_item("Studio Lighting", Lighting::Studio));
    camera.push(Entry::Separator);
    camera.extend([ToolKind::LookAround, ToolKind::PositionCamera].map(tool_item));
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
    // The chosen style's edge effects; the default look has none to toggle.
    let chosen = view
        .style
        .and_then(|id| view.styles.iter().find(|r| r.id == id))
        .map(|r| r.settings.edges);
    let edge_style: Vec<Entry> = [
        (
            "Profiles",
            EdgeEffect::Profiles,
            chosen.is_some_and(|e| e.profiles.is_some()),
        ),
        (
            "Sketchy",
            EdgeEffect::Sketchy,
            chosen.is_some_and(|e| e.sketchy.is_some()),
        ),
        (
            "Endpoints",
            EdgeEffect::Endpoints,
            chosen.is_some_and(|e| e.endpoints.is_some()),
        ),
    ]
    .into_iter()
    .map(|(label, effect, on)| {
        Item::new(label, UiAction::ToggleEdgeEffect(effect))
            .checked(on)
            .enabled(chosen.is_some())
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
        Item::new(
            "Section Planes",
            UiAction::SetSectionPlanes(!view.section_display.planes),
        )
        .checked(view.section_display.planes)
        .entry(),
        Item::new(
            "Section Cuts",
            UiAction::SetSectionCuts(!view.section_display.cuts),
        )
        .checked(view.section_display.cuts)
        .entry(),
        Item::new(
            "Section Fill",
            UiAction::SetSectionFill(!view.section_display.fill),
        )
        .checked(view.section_display.fill)
        .entry(),
        Entry::Separator,
        Item::new("X-ray", UiAction::SetXRay(!view.x_ray))
            .checked(view.x_ray)
            .entry(),
        Item::new(
            "Hidden Geometry",
            UiAction::SetHiddenGeometry(!view.hidden_geometry),
        )
        .checked(view.hidden_geometry)
        .entry(),
        Item::new("Shadows", UiAction::SetShadowsOn(!view.shadows.on))
            .checked(view.shadows.on)
            .entry(),
        Entry::Submenu {
            label: "Edge Style",
            entries: edge_style,
        },
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

    let mut help = vec![Item::new("Keyboard Shortcuts", UiAction::ShowShortcuts(true)).entry()];
    if view.features.update_check {
        help.push(Item::new("Check for Updates...", UiAction::CheckForUpdates).entry());
    }
    if view.features.in_app_dialogs {
        help.push(Item::new("About", UiAction::ShowAbout).entry());
    }

    let mut menus = Vec::new();
    if native {
        menus.push(Menu {
            title: APP_MENU,
            // Settings..., then Quit, which macOS names with the app, as the predefined About
            // does. `native_menu` puts its standard items around them.
            entries: vec![
                settings(),
                bound_item("Quit Tenon", Bound::Quit, UiAction::Quit).entry(),
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

/// A stable id for the item: its action's Debug text, except for the lighting toggles, which
/// are told apart by their labels (see `item_id`).
#[cfg(any(target_os = "macos", test))]
pub fn item_key(item: &Item) -> String {
    match item.action {
        UiAction::SetLighting(_) if item.label == "Studio Lighting" => {
            "ToggleStudioLighting".into()
        }
        _ => item_id(&item.action),
    }
}

/// A stable id for an action: its Debug text. The lighting toggle's action flips between its
/// mode and day, so it has one id for both (the Night Lighting item's; `item_key` gives the
/// Studio Lighting item its own); so do the section, X-ray and Hidden Geometry toggles.
#[cfg(any(target_os = "macos", test))]
pub fn item_id(action: &UiAction) -> String {
    match action {
        UiAction::SetLighting(_) => "ToggleNightLighting".into(),
        UiAction::SetSectionPlanes(_) => "ToggleSectionPlanes".into(),
        UiAction::SetSectionCuts(_) => "ToggleSectionCuts".into(),
        UiAction::SetSectionFill(_) => "ToggleSectionFill".into(),
        UiAction::SetXRay(_) => "ToggleXRay".into(),
        UiAction::SetHiddenGeometry(_) => "ToggleHiddenGeometry".into(),
        UiAction::SetShadowsOn(_) => "ToggleShadows".into(),
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
