//! The window's sheets: one modal dialog at a time over the whole window, drawn with
//! `egui::Modal`. Sheets are window chrome: what they hold while open lives in egui's memory,
//! and what they report goes through `UiAction`.

use std::path::{Path, PathBuf};

use egui::{Id, Modal, RichText, Ui};

use egui::text::{CCursor, CCursorRange};
use kernel::{DefinitionKind, MAX_NOTE_CHARS, MAX_NOTE_LINES, TagId};
use session::text3d::MIN_TEXT_HEIGHT;
use session::{MAX_COORDINATE, Units, format_length, parse_length};
use tools::{NoteRequest, TextSpec};

use super::{LibrarySave, UiAction, theme, widgets};
use crate::editor::{
    ImportApplies, ImportAs, ImportOptions, ImportUnits, PaperChoice, PrintOptions, PrintPurpose,
    RenameTarget, UpAxis,
};
use crate::platform::{UnsavedChoice, UnsavedPrompt};

/// The sheet open over the window.
#[derive(Clone, Debug, PartialEq)]
pub enum Sheet {
    /// Import Options, after a file was chosen and before it is read.
    Import {
        path: PathBuf,
        options: ImportOptions,
    },
    /// Make Component, asking for the new component's name (`suggested` filled in).
    NameComponent { suggested: String },
    /// Rename Component (or Group), with the current name filled in.
    Rename { target: RenameTarget },
    /// Field of View, with the camera's value (rounded when shown).
    FieldOfView { degrees: f64 },
    /// Delete Tag, for a tag something is on: where it goes, Untagged or one of `others`
    /// (each tag's id and name).
    DeleteTag {
        tag: TagId,
        name: String,
        count: usize,
        others: Vec<(TagId, String)>,
    },
    /// Discard Recovered, asking before the recovered copies at `models` are removed
    /// (`names`: the models' names, in the same order).
    DiscardRecovered {
        models: Vec<PathBuf>,
        names: Vec<String>,
    },
    /// A note's text: of the note the Text tool is placing, or of a note to change
    /// (`request`), starting with `text`. With `automatic` (a leader note's automatic text,
    /// shown while the field is empty) the text may be left blank; otherwise it is needed.
    NoteText {
        request: NoteRequest,
        text: String,
        automatic: Option<String>,
    },
    /// 3D Text: the text, its height and depth and whether it is filled, before the text is
    /// placed with the cursor.
    Text3d { spec: TextSpec },
    /// Save to Library: the name to save `save` under, `suggested` filled in.
    SaveToLibrary {
        save: LibrarySave,
        suggested: String,
    },
    /// The library already holds an item named `name` (the file `file`): replace it?
    OverwriteLibrary {
        save: LibrarySave,
        name: String,
        file: String,
    },
    /// Delete from Library, asking before the item's file is removed.
    DeleteLibraryItem { path: PathBuf, name: String },
    /// Export, where files have no path: the formats (name and extension, the one used last
    /// first) to choose from before the file is saved.
    Export {
        formats: Vec<(&'static str, &'static str)>,
    },
    /// The unsaved-changes question, where the window asks it itself.
    Unsaved { prompt: UnsavedPrompt },
    /// Help > About; `shadows_off` adds that light shadows are off in this browser.
    About { shadows_off: bool },
    /// Help > Check for Updates: what the check found, none while it runs.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Update {
        outcome: Option<crate::update::Outcome>,
    },
    /// Print: paper, orientation, Fit or 1:N and resolution, before the view is rendered to a
    /// PDF (`purpose`: a file chosen after it, or the system's viewer).
    Print {
        purpose: PrintPurpose,
        spec: PrintOptions,
    },
    /// File > Open on Phone: the QR code and link of the share, or why there is none.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Phone { view: PhoneView },
}

/// What the Open on Phone sheet shows; the window refreshes it every frame while it is open.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub enum PhoneView {
    /// A share is being served.
    Sharing(PhoneShare),
    /// No share could be made: what to tell the user.
    Unavailable(String),
}

/// A share being served.
#[derive(Clone, Debug, PartialEq)]
pub struct PhoneShare {
    /// The link, `http://<address>:<port>/#<fragment>`.
    pub url: String,
    /// The QR code's modules per side, and the modules row by row (true: dark), without the
    /// quiet zone.
    pub qr_width: usize,
    pub qr: Vec<bool>,
    /// How many phones fetched the model.
    pub sent: usize,
    /// Seconds until the share stops by itself.
    pub seconds_left: u64,
    /// The model changed after it was shared.
    pub stale: bool,
}

/// A sheet's width, in points.
const WIDTH: f32 = 340.0;

/// The most the note sheet's text box grows to before it scrolls, in points.
const NOTE_HEIGHT: f32 = 160.0;

/// The id of the sheet's modal area, and the egui memory key of what it holds while open.
fn sheet_id() -> Id {
    Id::new("sheet")
}

/// What an open sheet holds between frames: the sheet it was opened as, and the values being
/// edited. A different sheet starts over from its own values.
#[derive(Clone)]
struct Editing {
    origin: Sheet,
    values: Values,
}

#[derive(Clone)]
enum Values {
    Import(ImportOptions),
    Name(String),
    Degrees(f64),
    /// Where what is on a deleted tag goes (None: Untagged).
    Destination(Option<TagId>),
    /// A note's text.
    Note(String),
    /// 3D text's text and sizes.
    Text3d(TextSpec),
    /// The print options being edited.
    Print(PrintOptions),
    /// The index of the format chosen in the Export sheet.
    Choice(usize),
    /// A sheet with nothing to edit.
    Nothing,
}

impl Values {
    /// What `sheet` starts with.
    fn of(sheet: &Sheet) -> Self {
        match sheet {
            Sheet::Import { options, .. } => Values::Import(*options),
            Sheet::NameComponent { suggested } => Values::Name(suggested.clone()),
            Sheet::Rename { target } => Values::Name(target.name.clone()),
            Sheet::FieldOfView { degrees } => Values::Degrees(degrees.round()),
            Sheet::DeleteTag { .. } => Values::Destination(None),
            Sheet::DiscardRecovered { .. }
            | Sheet::Unsaved { .. }
            | Sheet::About { .. }
            | Sheet::Update { .. } => Values::Nothing,
            Sheet::NoteText { text, .. } => Values::Note(text.clone()),
            Sheet::Text3d { spec } => Values::Text3d(spec.clone()),
            Sheet::SaveToLibrary { suggested, .. } => Values::Name(suggested.clone()),
            Sheet::OverwriteLibrary { .. } | Sheet::DeleteLibraryItem { .. } => Values::Nothing,
            Sheet::Print { spec, .. } => Values::Print(*spec),
            Sheet::Export { .. } => Values::Choice(0),
            Sheet::Phone { .. } => Values::Nothing,
        }
    }
}

/// Draws `sheet` over the window. Escape, a click on the backdrop and Cancel report
/// `CloseSheet`; the confirm button reports the sheet's confirm action.
pub(super) fn sheet(ui: &mut Ui, sheet: &Sheet, units: Units, actions: &mut Vec<UiAction>) {
    let id = sheet_id();
    let held = ui
        .data(|d| d.get_temp::<Editing>(id))
        .filter(|e| e.origin == *sheet);
    // The sheet's first frame: its name field takes focus.
    let fresh = held.is_none();
    let mut values = held.map_or_else(|| Values::of(sheet), |e| e.values);
    let modal = Modal::new(id)
        .area(Modal::default_area(id).fade_in(false))
        .show(ui.ctx(), |ui| {
            ui.set_width(WIDTH);
            match (sheet, &mut values) {
                (Sheet::Import { path, .. }, Values::Import(options)) => {
                    import_sheet(ui, path, options, actions);
                }
                (Sheet::NameComponent { .. }, Values::Name(name)) => {
                    name_sheet(
                        ui,
                        ("Make Component", "Create"),
                        name,
                        NameUse::new(false, fresh, UiAction::ConfirmName),
                        actions,
                    );
                }
                (Sheet::Rename { target }, Values::Name(name)) => {
                    let (title, allow_blank) = match target.kind {
                        DefinitionKind::Group => ("Rename Group", true),
                        DefinitionKind::Component => ("Rename Component", false),
                    };
                    name_sheet(
                        ui,
                        (title, "Rename"),
                        name,
                        NameUse::new(allow_blank, fresh, UiAction::ConfirmName),
                        actions,
                    );
                }
                (Sheet::FieldOfView { .. }, Values::Degrees(degrees)) => {
                    field_of_view_sheet(ui, degrees, actions);
                }
                (
                    Sheet::DeleteTag {
                        name,
                        count,
                        others,
                        ..
                    },
                    Values::Destination(to),
                ) => {
                    delete_tag_sheet(ui, name, *count, others, to, actions);
                }
                (Sheet::DiscardRecovered { names, .. }, Values::Nothing) => {
                    discard_recovered_sheet(ui, names, actions);
                }
                (
                    Sheet::NoteText {
                        request, automatic, ..
                    },
                    Values::Note(text),
                ) => {
                    note_sheet(ui, request, automatic.as_deref(), text, fresh, actions);
                }
                (Sheet::Text3d { .. }, Values::Text3d(spec)) => {
                    text_3d_sheet(ui, spec, fresh, &units, actions);
                }
                (Sheet::SaveToLibrary { save, .. }, Values::Name(name)) => {
                    let title = match save {
                        LibrarySave::Component(_) => "Save Component to Library",
                        LibrarySave::Material(_) => "Save Material to Library",
                        LibrarySave::Model => "Save Model to Library",
                    };
                    name_sheet(
                        ui,
                        (title, "Save"),
                        name,
                        NameUse::new(false, fresh, UiAction::ConfirmSaveToLibrary),
                        actions,
                    );
                }
                (Sheet::Export { formats }, Values::Choice(chosen)) => {
                    export_sheet(ui, formats, chosen, actions);
                }
                (Sheet::Unsaved { prompt }, Values::Nothing) => {
                    unsaved_sheet(ui, prompt, actions);
                }
                (Sheet::About { shadows_off }, Values::Nothing) => {
                    about_sheet(ui, *shadows_off, actions);
                }
                (Sheet::Update { outcome }, Values::Nothing) => {
                    update_sheet(ui, outcome.as_ref(), actions);
                }
                (Sheet::OverwriteLibrary { name, file, .. }, Values::Nothing) => {
                    overwrite_library_sheet(ui, name, file, actions);
                }
                (Sheet::DeleteLibraryItem { name, .. }, Values::Nothing) => {
                    delete_library_sheet(ui, name, actions);
                }
                (Sheet::Print { purpose, .. }, Values::Print(spec)) => {
                    print_sheet(ui, *purpose, spec, actions);
                }
                (Sheet::Phone { view }, Values::Nothing) => {
                    phone_sheet(ui, view, actions);
                }
                // `Values::of` matches each sheet with its values.
                _ => {}
            }
        });
    ui.data_mut(|d| {
        d.insert_temp(
            id,
            Editing {
                origin: sheet.clone(),
                values,
            },
        )
    });
    if modal.should_close() {
        actions.push(UiAction::CloseSheet);
    }
}

/// Forgets what a sheet held, once none is open, so the next one opens fresh.
pub(super) fn forget(ui: &Ui) {
    ui.data_mut(|d| d.remove::<Editing>(sheet_id()));
}

/// The Import Options sheet: units, up axis, what the import makes and whether coplanar faces
/// merge, for the file at `path`. A SketchUp file has its own units, axes and faces: only
/// what the import makes is asked; a COLLADA file has its own units and axes.
fn import_sheet(
    ui: &mut Ui,
    path: &Path,
    options: &mut ImportOptions,
    actions: &mut Vec<UiAction>,
) {
    let t = theme::tokens(ui.ctx().theme());
    widgets::section_header(ui, "Import Options");
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    ui.label(RichText::new(name).color(t.text_secondary));
    ui.add_space(theme::GRID);
    let applies = ImportApplies::to(path);
    egui::Grid::new("import-options")
        .num_columns(2)
        .spacing([theme::GRID * 2.0, theme::GRID])
        .show(ui, |ui| {
            if applies.units {
                mesh_options(ui, options, t.text_secondary);
            }
            ui.label("Import as");
            ui.vertical(|ui| {
                ui.radio_value(&mut options.place, ImportAs::Group, "Group");
                ui.radio_value(&mut options.place, ImportAs::Component, "Component");
                ui.radio_value(&mut options.place, ImportAs::Context, "Loose geometry");
            });
            ui.end_row();
        });
    if applies.merge {
        ui.checkbox(&mut options.merge_coplanar, "Merge coplanar faces");
    }
    if buttons(ui, "Import", true, actions) {
        actions.push(UiAction::ConfirmImport(*options));
    }
}

/// The import sheet's rows for OBJ, STL and glTF: units and up axis.
fn mesh_options(ui: &mut Ui, options: &mut ImportOptions, secondary: egui::Color32) {
    let label = ui.label("Units");
    egui::ComboBox::from_id_salt("import-units")
        .selected_text(options.units.label())
        .show_ui(ui, |ui| {
            for units in ImportUnits::ALL {
                ui.selectable_value(&mut options.units, units, units.label());
            }
        })
        .response
        .labelled_by(label.id);
    ui.end_row();

    ui.label("Up axis");
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.radio_value(&mut options.up, None, "Automatic");
            ui.radio_value(&mut options.up, Some(UpAxis::Y), "Y up");
            ui.radio_value(&mut options.up, Some(UpAxis::Z), "Z up");
        });
        ui.label(
            RichText::new("Automatic: Y for OBJ and glTF, Z for STL")
                .small()
                .color(secondary),
        );
    });
    ui.end_row();
}

/// How a name sheet behaves: whether a blank name is taken, whether it is the sheet's first
/// frame (the field then takes focus with all of it selected, so typing replaces it) and which
/// action carries the name.
struct NameUse {
    allow_blank: bool,
    fresh: bool,
    confirm: fn(String) -> UiAction,
}

impl NameUse {
    fn new(allow_blank: bool, fresh: bool, confirm: fn(String) -> UiAction) -> Self {
        Self {
            allow_blank,
            fresh,
            confirm,
        }
    }
}

/// A sheet asking for a name: `title` and `button` (the confirm button), a "Name" field
/// holding `name`. A blank name is refused unless `how.allow_blank`: the button is then
/// disabled and Enter does nothing. Enter in the field or the button reports the name through
/// `how.confirm`.
fn name_sheet(
    ui: &mut Ui,
    (title, button): (&str, &str),
    name: &mut String,
    how: NameUse,
    actions: &mut Vec<UiAction>,
) {
    let NameUse {
        allow_blank,
        fresh,
        confirm,
    } = how;
    widgets::section_header(ui, title);
    ui.add_space(theme::GRID);
    let field_id = Id::new("sheet-name");
    let output = ui
        .horizontal(|ui| {
            let label = ui.label("Name");
            let output = egui::TextEdit::singleline(name)
                .id(field_id)
                .desired_width(f32::INFINITY)
                .show(ui);
            // Named by its label, which then is not a node of its own.
            output.response.response.clone().labelled_by(label.id);
            output
        })
        .inner;
    let field = &output.response;
    field.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Name");
        info.current_text_value = Some(name.clone());
        info
    });
    if fresh {
        field.request_focus();
        let mut state = output.state.clone();
        let all = CCursorRange::two(CCursor::new(0), CCursor::new(name.chars().count()));
        state.cursor.set_char_range(Some(all));
        state.store(ui.ctx(), field_id);
    }
    let valid = allow_blank || !name.trim().is_empty();
    let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if entered && !valid {
        // Enter ends the field's focus; it keeps it, so the name can still be typed.
        field.request_focus();
    }
    let clicked = buttons(ui, button, valid, actions);
    if valid && (entered || clicked) {
        actions.push(confirm(name.clone()));
    }
}

/// The overwrite sheet: the library holds an item in `file` already; Replace reports
/// `ConfirmOverwriteLibrary`.
fn overwrite_library_sheet(ui: &mut Ui, name: &str, file: &str, actions: &mut Vec<UiAction>) {
    widgets::section_header(ui, "Replace Library Item");
    ui.add_space(theme::GRID);
    ui.label(format!(
        "The library already holds {file}. Replace it with {name}?"
    ));
    if buttons(ui, "Replace", true, actions) {
        actions.push(UiAction::ConfirmOverwriteLibrary);
    }
}

/// The Delete from Library sheet: names the item, says the file goes, Delete reports
/// `ConfirmLibraryDelete`.
fn delete_library_sheet(ui: &mut Ui, name: &str, actions: &mut Vec<UiAction>) {
    widgets::section_header(ui, "Delete from Library");
    ui.add_space(theme::GRID);
    ui.label(format!("Delete {name} from the library?"));
    ui.label("Its file is deleted. This cannot be undone.");
    if buttons(ui, "Delete", true, actions) {
        actions.push(UiAction::ConfirmLibraryDelete);
    }
}

/// The Field of View sheet: a whole-degree drag field, 1 to 170 as `set_camera` allows, and
/// Apply, which reports the value rounded.
fn field_of_view_sheet(ui: &mut Ui, degrees: &mut f64, actions: &mut Vec<UiAction>) {
    widgets::section_header(ui, "Field of View");
    ui.add_space(theme::GRID);
    ui.horizontal(|ui| {
        let label = ui.label("Field of view");
        ui.add(
            egui::DragValue::new(degrees)
                .range(1.0..=170.0)
                .speed(0.5)
                .fixed_decimals(0)
                .suffix("°"),
        )
        .labelled_by(label.id);
    });
    if buttons(ui, "Apply", true, actions) {
        actions.push(UiAction::ConfirmFieldOfView(degrees.round()));
    }
}

/// The Delete Tag sheet: how many entities are on the tag, where they go (Untagged or another
/// tag) and Delete, which reports the destination.
fn delete_tag_sheet(
    ui: &mut Ui,
    name: &str,
    count: usize,
    others: &[(TagId, String)],
    to: &mut Option<TagId>,
    actions: &mut Vec<UiAction>,
) {
    widgets::section_header(ui, "Delete Tag");
    ui.add_space(theme::GRID);
    let what = match count {
        1 => "1 entity is".to_string(),
        n => format!("{n} entities are"),
    };
    ui.label(format!("{what} on {name}."));
    let name_of = |to: Option<TagId>| {
        to.and_then(|t| others.iter().find(|(o, _)| *o == t))
            .map_or("Untagged", |(_, n)| n.as_str())
    };
    ui.horizontal(|ui| {
        let label = ui.label("Move to");
        egui::ComboBox::from_id_salt("delete-tag-destination")
            .selected_text(name_of(*to))
            .show_ui(ui, |ui| {
                ui.selectable_value(to, None, "Untagged");
                for (t, n) in others {
                    ui.selectable_value(to, Some(*t), n);
                }
            })
            .response
            .labelled_by(label.id);
    });
    if buttons(ui, "Delete", true, actions) {
        actions.push(UiAction::ConfirmDeleteTag(*to));
    }
}

/// The Discard Recovered sheet: which recovered models go, that this cannot be undone, and
/// Discard.
fn discard_recovered_sheet(ui: &mut Ui, names: &[String], actions: &mut Vec<UiAction>) {
    widgets::section_header(ui, "Discard Recovered");
    ui.add_space(theme::GRID);
    let (question, whose) = match names {
        [name] => (format!("Discard the recovered model {name}?"), "Its"),
        names => (
            format!("Discard all {} recovered models?", names.len()),
            "Their",
        ),
    };
    ui.label(question);
    ui.label(format!(
        "{whose} unsaved changes will be lost. This cannot be undone."
    ));
    if buttons(ui, "Discard", true, actions) {
        actions.push(UiAction::ConfirmDiscardRecovered);
    }
}

/// The Export sheet: the formats as radio buttons, then Cancel and Export, which reports
/// `ConfirmExport` with the chosen format's extension.
fn export_sheet(
    ui: &mut Ui,
    formats: &[(&'static str, &'static str)],
    chosen: &mut usize,
    actions: &mut Vec<UiAction>,
) {
    widgets::section_header(ui, "Export Model");
    ui.add_space(theme::GRID);
    for (i, (name, _)) in formats.iter().enumerate() {
        ui.radio_value(chosen, i, *name);
    }
    if buttons(ui, "Export...", !formats.is_empty(), actions)
        && let Some((_, ext)) = formats.get(*chosen)
    {
        actions.push(UiAction::ConfirmExport((*ext).to_string()));
    }
}

/// The unsaved-changes question: Cancel, Don't Save and Save (rightmost). Escape and a click on
/// the backdrop report `CloseSheet`, which the window answers as Cancel.
fn unsaved_sheet(ui: &mut Ui, prompt: &UnsavedPrompt, actions: &mut Vec<UiAction>) {
    widgets::section_header(ui, "Unsaved Changes");
    ui.add_space(theme::GRID);
    ui.label(&prompt.question);
    ui.label(prompt.detail);
    ui.add_space(theme::GRID * 2.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (label, choice) in [
                ("Save", UnsavedChoice::Save),
                ("Don't Save", UnsavedChoice::DontSave),
                ("Cancel", UnsavedChoice::Cancel),
            ] {
                if ui.button(label).clicked() {
                    actions.push(UiAction::UnsavedAnswer(choice));
                }
            }
        });
    });
}

/// Help > About: the app's name, and what is off in this browser.
fn about_sheet(ui: &mut Ui, shadows_off: bool, actions: &mut Vec<UiAction>) {
    widgets::section_header(ui, "About tenon");
    ui.add_space(theme::GRID);
    ui.label(format!("tenon {}", crate::update::version_label()));
    if shadows_off {
        ui.label(SHADOWS_OFF);
    }
    ui.add_space(theme::GRID * 2.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Close").clicked() {
                actions.push(UiAction::CloseSheet);
            }
        });
    });
}

/// Help > Check for Updates: the result, with a button to the release page when there is a
/// newer version. Nothing is downloaded.
fn update_sheet(
    ui: &mut Ui,
    outcome: Option<&crate::update::Outcome>,
    actions: &mut Vec<UiAction>,
) {
    use crate::update::Outcome;

    widgets::section_header(ui, "Check for Updates");
    ui.add_space(theme::GRID);
    ui.label(format!("You have version {}.", crate::update::VERSION));
    let mut open = None;
    match outcome {
        None => {
            ui.label("Checking...");
        }
        Some(Outcome::Available { version, url }) => {
            ui.label(format!("Version {version} is available."));
            open = Some(url.clone());
        }
        Some(Outcome::UpToDate) => {
            ui.label("You're up to date.");
        }
        Some(Outcome::NoReleases) => {
            ui.label("No releases published yet.");
        }
        Some(Outcome::Failed(why)) => {
            ui.label(why);
        }
    }
    ui.add_space(theme::GRID * 2.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Close").clicked() {
                actions.push(UiAction::CloseSheet);
            }
            if let Some(url) = open
                && ui.button("Open Release Page").clicked()
            {
                actions.push(UiAction::OpenReleasePage(url));
            }
        });
    });
}

/// The quiet zone around a QR code, in modules (the standard's minimum).
const QR_QUIET: usize = 4;
/// The fewest physical pixels a QR module is drawn with.
const QR_MIN_MODULE_PX: f32 = 4.0;
/// About how wide a QR code is drawn, in points.
const QR_TARGET: f32 = 220.0;

/// The size of one QR module in points: a whole number of physical pixels (at
/// `pixels_per_point`), at least `QR_MIN_MODULE_PX`, so modules have sharp edges and scan.
pub(super) fn qr_module_size(width: usize, pixels_per_point: f32) -> f32 {
    let ppp = pixels_per_point.max(0.1);
    let px = (QR_TARGET * ppp / (width + 2 * QR_QUIET) as f32)
        .floor()
        .max(QR_MIN_MODULE_PX);
    px / ppp
}

/// The dark modules of a `width` x `width` QR code as rectangles, the code's top-left corner
/// (quiet zone included) at `origin`, each module `module` points square.
pub(super) fn qr_rects(
    origin: egui::Pos2,
    width: usize,
    modules: &[bool],
    module: f32,
) -> Vec<egui::Rect> {
    let at = |n: usize| (n + QR_QUIET) as f32 * module;
    modules
        .iter()
        .enumerate()
        .filter(|(i, dark)| **dark && *i < width * width)
        .map(|(i, _)| {
            let min = origin + egui::vec2(at(i % width), at(i / width));
            egui::Rect::from_min_size(min, egui::Vec2::splat(module))
        })
        .collect()
}

/// `seconds` as `m:ss`.
pub(super) fn clock(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// File > Open on Phone: the QR code (black on white with a quiet zone) and the link with a
/// Copy button, whether a phone has fetched the model, the time left and Stop. A model changed
/// since the share offers to share the new version.
fn phone_sheet(ui: &mut Ui, view: &PhoneView, actions: &mut Vec<UiAction>) {
    widgets::section_header(ui, "Open on Phone");
    ui.add_space(theme::GRID);
    match view {
        PhoneView::Unavailable(why) => {
            ui.label(why);
            ui.add_space(theme::GRID * 2.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    actions.push(UiAction::CloseSheet);
                }
            });
        }
        PhoneView::Sharing(share) => {
            ui.label("Scan with the phone's camera. The phone must be on the same Wi-Fi.");
            ui.add_space(theme::GRID);
            let ppp = ui.ctx().pixels_per_point();
            let module = qr_module_size(share.qr_width, ppp);
            let side = (share.qr_width + 2 * QR_QUIET) as f32 * module;
            ui.vertical_centered(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::Vec2::splat(side), egui::Sense::hover());
                let painter = ui.painter();
                painter.rect_filled(rect, 0.0, egui::Color32::WHITE);
                // On a whole pixel, as every module's size is.
                let snap = |v: f32| (v * ppp).round() / ppp;
                let origin = egui::pos2(snap(rect.min.x), snap(rect.min.y));
                for r in qr_rects(origin, share.qr_width, &share.qr, module) {
                    painter.rect_filled(r, 0.0, egui::Color32::BLACK);
                }
            });
            ui.add_space(theme::GRID);
            ui.label(&share.url);
            if ui.button("Copy").clicked() {
                ui.ctx().copy_text(share.url.clone());
            }
            ui.add_space(theme::GRID);
            ui.label(match share.sent {
                0 => "Waiting for the phone".to_string(),
                1 => "Sent to 1 phone".to_string(),
                n => format!("Sent to {n} phones"),
            });
            ui.label(format!("Stops in {}", clock(share.seconds_left)));
            if share.stale {
                ui.add_space(theme::GRID);
                ui.label("The model changed after it was shared.");
                if ui.button("Share the new version").clicked() {
                    actions.push(UiAction::ShareNewVersion);
                }
            }
            ui.add_space(theme::GRID * 2.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Stop").clicked() {
                    actions.push(UiAction::StopSharing);
                }
            });
        }
    }
}

/// What the status line and About say when the adapter cannot draw light shadows.
pub const SHADOWS_OFF: &str = "Light shadows are off in this browser (WebGL2)";

/// The note sheet: a box for the note's text, several lines long, then Cancel and Add (a
/// new note) or Change. Blank text is taken only for a leader note (`automatic`, shown in the
/// empty box, is what it then shows); text over the length or line limits is not taken, and
/// the sheet says why. Add or Change reports `ConfirmNote`.
fn note_sheet(
    ui: &mut Ui,
    request: &NoteRequest,
    automatic: Option<&str>,
    text: &mut String,
    fresh: bool,
    actions: &mut Vec<UiAction>,
) {
    let t = theme::tokens(ui.ctx().theme());
    let (title, button) = match request {
        NoteRequest::Edit(_) => ("Edit Text", "Change"),
        NoteRequest::Leader { .. } | NoteRequest::Screen { .. } => ("Add Text", "Add"),
    };
    widgets::section_header(ui, title);
    ui.add_space(theme::GRID);
    let field_id = Id::new("sheet-note");
    let label = ui.label("Note text");
    let mut field = egui::TextEdit::multiline(text)
        .id(field_id)
        .desired_rows(4)
        .desired_width(f32::INFINITY);
    if let Some(automatic) = automatic {
        field = field.hint_text(automatic);
    }
    // Long text scrolls inside the sheet.
    let output = egui::ScrollArea::vertical()
        .max_height(NOTE_HEIGHT)
        .show(ui, |ui| field.show(ui))
        .inner;
    output.response.response.clone().labelled_by(label.id);
    output.response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Note text");
        info.current_text_value = Some(text.clone());
        info
    });
    if fresh {
        output.response.request_focus();
        let mut state = output.state.clone();
        let all = CCursorRange::two(CCursor::new(0), CCursor::new(text.chars().count()));
        state.cursor.set_char_range(Some(all));
        state.store(ui.ctx(), field_id);
    }
    let problem = if text.trim().is_empty() {
        automatic
            .is_none()
            .then_some("Type the text the note shows".to_string())
    } else if text.chars().count() > MAX_NOTE_CHARS {
        Some(format!("At most {MAX_NOTE_CHARS} characters"))
    } else if text.lines().count() > MAX_NOTE_LINES {
        Some(format!("At most {MAX_NOTE_LINES} lines"))
    } else {
        None
    };
    let hint = match (&problem, automatic) {
        (Some(problem), _) => Some(problem.as_str()),
        (None, Some(_)) => Some("Leave it empty to show the point's coordinates"),
        (None, None) => None,
    };
    if let Some(hint) = hint {
        ui.label(RichText::new(hint).small().color(t.text_secondary));
    }
    if buttons(ui, button, problem.is_none(), actions) {
        actions.push(UiAction::ConfirmNote(text.clone()));
    }
}

/// The 3D Text sheet: a box for the text (several lines), its height, how deep (filled text
/// only) and whether it is filled, then Cancel and Place, which reports `Confirm3dText` (a
/// depth of 0 for text that is not filled). Text that is blank or over the length or line
/// limits is not taken, and the sheet says why.
fn text_3d_sheet(
    ui: &mut Ui,
    spec: &mut TextSpec,
    fresh: bool,
    units: &Units,
    actions: &mut Vec<UiAction>,
) {
    let t = theme::tokens(ui.ctx().theme());
    widgets::section_header(ui, "3D Text");
    ui.add_space(theme::GRID);
    let field_id = Id::new("sheet-3d-text");
    let label = ui.label("Text to draw");
    let output = egui::ScrollArea::vertical()
        .max_height(NOTE_HEIGHT)
        .show(ui, |ui| {
            egui::TextEdit::multiline(&mut spec.text)
                .id(field_id)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .show(ui)
        })
        .inner;
    output.response.response.clone().labelled_by(label.id);
    output.response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Text to draw");
        info.current_text_value = Some(spec.text.clone());
        info
    });
    if fresh {
        output.response.request_focus();
        let mut state = output.state.clone();
        let all = CCursorRange::two(CCursor::new(0), CCursor::new(spec.text.chars().count()));
        state.cursor.set_char_range(Some(all));
        state.store(ui.ctx(), field_id);
    }
    ui.add_space(theme::GRID);
    egui::Grid::new("text-3d-sizes")
        .num_columns(2)
        .spacing([theme::GRID * 2.0, theme::GRID])
        .show(ui, |ui| {
            let label = ui.label("Height");
            ui.add(
                egui::DragValue::new(&mut spec.height)
                    .range(MIN_TEXT_HEIGHT..=MAX_COORDINATE)
                    .speed(0.01)
                    .custom_formatter(|v, _| format_length(v, units))
                    .custom_parser(|text| parse_length(text, units).ok()),
            )
            .labelled_by(label.id);
            ui.end_row();
            let label = ui.label("Depth");
            ui.add_enabled(
                spec.filled,
                egui::DragValue::new(&mut spec.depth)
                    .range(0.0..=MAX_COORDINATE)
                    .speed(0.01)
                    .custom_formatter(|v, _| format_length(v, units))
                    .custom_parser(|text| parse_length(text, units).ok()),
            )
            .labelled_by(label.id);
            ui.end_row();
        });
    ui.checkbox(&mut spec.filled, "Filled");
    let problem = if spec.text.trim().is_empty() {
        Some("Type the text to draw".to_string())
    } else if spec.text.chars().count() > MAX_NOTE_CHARS {
        Some(format!("At most {MAX_NOTE_CHARS} characters"))
    } else if spec.text.lines().count() > MAX_NOTE_LINES {
        Some(format!("At most {MAX_NOTE_LINES} lines"))
    } else {
        None
    };
    let hint = problem
        .as_deref()
        .unwrap_or("Capital letters are this tall; depth 0 draws flat letters");
    ui.label(RichText::new(hint).small().color(t.text_secondary));
    if buttons(ui, "Place", problem.is_none(), actions) {
        let mut spec = spec.clone();
        if !spec.filled {
            spec.depth = 0.0;
        }
        actions.push(UiAction::Confirm3dText(spec));
    }
}

/// The Print sheet: paper, orientation, Fit or an exact 1:N scale (with N) and resolution.
/// Export PDF then asks for a file; Print opens the result.
fn print_sheet(
    ui: &mut Ui,
    purpose: PrintPurpose,
    spec: &mut PrintOptions,
    actions: &mut Vec<UiAction>,
) {
    let t = theme::tokens(ui.ctx().theme());
    let (title, button) = match purpose {
        PrintPurpose::Export => ("Export PDF", "Export..."),
        PrintPurpose::Print => ("Print Setup", "Print"),
    };
    widgets::section_header(ui, title);
    ui.add_space(theme::GRID);
    egui::Grid::new("print-options")
        .num_columns(2)
        .spacing([theme::GRID * 2.0, theme::GRID])
        .show(ui, |ui| {
            let label = ui.label("Paper");
            egui::ComboBox::from_id_salt("print-paper")
                .selected_text(spec.paper.label())
                .show_ui(ui, |ui| {
                    for paper in PaperChoice::ALL {
                        ui.selectable_value(&mut spec.paper, paper, paper.label());
                    }
                })
                .response
                .labelled_by(label.id);
            ui.end_row();

            ui.label("Orientation");
            ui.horizontal(|ui| {
                ui.radio_value(&mut spec.landscape, false, "Portrait");
                ui.radio_value(&mut spec.landscape, true, "Landscape");
            });
            ui.end_row();

            ui.label("Scale");
            ui.vertical(|ui| {
                if ui.radio(spec.scale.is_none(), "Fit to page").clicked() {
                    spec.scale = None;
                }
                ui.horizontal(|ui| {
                    let exact = spec.scale.is_some();
                    let radio = ui.radio(exact, "1 :");
                    if radio.clicked() && !exact {
                        spec.scale = Some(PrintOptions::DEFAULT_SCALE);
                    }
                    if let Some(n) = &mut spec.scale {
                        ui.add(
                            egui::DragValue::new(n)
                                .range(0.01..=1_000_000.0)
                                .speed(1.0)
                                .max_decimals(2),
                        )
                        .labelled_by(radio.id);
                    }
                });
                ui.label(
                    RichText::new("Exact scale needs a parallel view")
                        .small()
                        .color(t.text_secondary),
                );
            });
            ui.end_row();

            let label = ui.label("Resolution");
            ui.add(
                egui::DragValue::new(&mut spec.dpi)
                    .range(session::print::MIN_DPI..=session::print::MAX_DPI)
                    .suffix(" dpi"),
            )
            .labelled_by(label.id);
            ui.end_row();

            ui.label("Cut list");
            ui.checkbox(&mut spec.cut_list, "Add cut list pages");
            ui.end_row();
        });
    if buttons(ui, button, spec.valid(), actions) {
        actions.push(UiAction::ConfirmPrint(*spec));
    }
}

/// The sheet's buttons at the right: `confirm` (enabled when `can_confirm`) rightmost, Cancel
/// beside it. Cancel reports `CloseSheet`. True when `confirm` was clicked.
fn buttons(ui: &mut Ui, confirm: &str, can_confirm: bool, actions: &mut Vec<UiAction>) -> bool {
    ui.add_space(theme::GRID * 2.0);
    let mut confirmed = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            confirmed = ui
                .add_enabled(can_confirm, egui::Button::new(confirm))
                .clicked();
            if ui.button("Cancel").clicked() {
                actions.push(UiAction::CloseSheet);
            }
        });
    });
    confirmed
}
