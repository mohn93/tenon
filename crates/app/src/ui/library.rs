//! The Library tab: a search field and kind filter over a grid of the library folder's items,
//! each a thumbnail and a name. A click uses an item, a right-click opens its menu (Insert,
//! Open, Delete from Library). A tile in view without a thumbnail asks for one
//! (`UiAction::ThumbnailsWanted`); the window makes at most a few a frame.

use egui::{CornerRadius, RichText, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo, WidgetType};
use session::LibraryKind;

use super::{LibraryRow, Thumb, UiAction, View, icons, theme, widgets};

/// A tile's thumbnail side, in points.
pub(super) const LIBRARY_TILE: f32 = 88.0;

/// Which kinds of item the grid shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum KindFilter {
    #[default]
    All,
    Only(LibraryKind),
}

impl KindFilter {
    const ALL: [KindFilter; 4] = [
        KindFilter::All,
        KindFilter::Only(LibraryKind::Component),
        KindFilter::Only(LibraryKind::Material),
        KindFilter::Only(LibraryKind::Model),
    ];

    fn label(self) -> &'static str {
        match self {
            KindFilter::All => "All",
            KindFilter::Only(LibraryKind::Component) => "Components",
            KindFilter::Only(LibraryKind::Material) => "Materials",
            KindFilter::Only(LibraryKind::Model) => "Models",
        }
    }

    fn shows(self, kind: LibraryKind) -> bool {
        match self {
            KindFilter::All => true,
            KindFilter::Only(only) => only == kind,
        }
    }
}

/// The egui memory keys of the search text and the kind filter.
fn search_key() -> egui::Id {
    egui::Id::new("library-search")
}

fn kind_key() -> egui::Id {
    egui::Id::new("library-kind")
}

/// The rows whose name contains `query` (trimmed, ignoring case) and whose kind `kind` shows.
pub(super) fn filtered<'a>(
    rows: &'a [LibraryRow],
    query: &str,
    kind: impl Fn(LibraryKind) -> bool,
) -> Vec<&'a LibraryRow> {
    let query = query.trim().to_lowercase();
    rows.iter()
        .filter(|r| kind(r.kind) && r.name.to_lowercase().contains(&query))
        .collect()
}

/// How many tiles, `TILE_GAP` apart, fit `width` points (at least one).
fn columns(width: f32) -> usize {
    ((width + theme::TILE_GAP) / (LIBRARY_TILE + theme::TILE_GAP))
        .floor()
        .max(1.0) as usize
}

/// The Library tab (the tab is its title); the dock's top area scrolls it.
pub(super) fn library(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    let lib = &view.library;
    if lib.folder.is_none() {
        ui.label("Choose a library folder");
        ui.label(RichText::new("Components, materials and models saved there show up here").weak());
        if ui.button("Choose...").clicked() {
            actions.push(UiAction::ChooseLibraryFolder);
        }
        return;
    }
    let mut text = ui
        .data(|d| d.get_temp::<String>(search_key()))
        .unwrap_or_default();
    let mut kind = ui
        .data(|d| d.get_temp::<KindFilter>(kind_key()))
        .unwrap_or_default();
    ui.horizontal(|ui| {
        let combo = egui::ComboBox::from_id_salt("library-kind-box")
            .selected_text(kind.label())
            .width(96.0)
            .show_ui(ui, |ui| {
                for choice in KindFilter::ALL {
                    ui.selectable_value(&mut kind, choice, choice.label());
                }
            });
        combo
            .response
            .widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, "Kind"));
        let edit = ui.add(
            egui::TextEdit::singleline(&mut text)
                .hint_text("Search")
                .desired_width(f32::INFINITY),
        );
        edit.widget_info(|| {
            let mut info = WidgetInfo::labeled(WidgetType::TextEdit, true, "Search library");
            info.current_text_value = Some(text.clone());
            info
        });
    });
    ui.data_mut(|d| {
        d.insert_temp(search_key(), text.clone());
        d.insert_temp(kind_key(), kind);
    });
    if let Some(error) = lib.error {
        ui.label(RichText::new(error).color(theme::tokens(ui.ctx().theme()).text_secondary));
        return;
    }
    let shown = filtered(lib.rows, &text, |k| kind.shows(k));
    if lib.rows.is_empty() {
        ui.label(RichText::new("No items").weak());
    } else if shown.is_empty() {
        ui.label(RichText::new("No matches").weak());
    }
    let per_row = columns(ui.available_width());
    let mut wanted = Vec::new();
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(theme::TILE_GAP);
        for row in shown.chunks(per_row) {
            ui.horizontal_top(|ui| {
                for item in row {
                    tile(ui, item, &mut wanted, actions);
                }
            });
        }
    });
    if lib.more {
        ui.label(RichText::new("More items than are listed: search to narrow").weak());
    }
    if !wanted.is_empty() {
        actions.push(UiAction::ThumbnailsWanted(wanted));
    }
}

/// One tile: the thumbnail (an icon when it could not be made) and the name under it, cut
/// short with "…". Named by the item's name. A tile in view with no thumbnail yet adds its path
/// to `wanted`.
fn tile(
    ui: &mut Ui,
    row: &LibraryRow,
    wanted: &mut Vec<std::path::PathBuf>,
    actions: &mut Vec<UiAction>,
) {
    let t = theme::tokens(ui.ctx().theme());
    let mut job = egui::text::LayoutJob::single_section(
        row.name.clone(),
        egui::TextFormat::simple(
            egui::FontId::proportional(theme::SMALL_SIZE),
            t.text_secondary,
        ),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: LIBRARY_TILE,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let caption = ui.fonts_mut(|f| f.layout_job(job));
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(LIBRARY_TILE, LIBRARY_TILE + caption.size().y),
        Sense::click(),
    );
    let square = egui::Rect::from_min_size(rect.min, Vec2::splat(LIBRARY_TILE));
    let radius = CornerRadius::same(theme::SWATCH_RADIUS);
    match row.thumb {
        Thumb::Ready(image) => egui::Image::new((image, square.size()))
            .corner_radius(radius)
            .paint_at(ui, square),
        Thumb::Pending | Thumb::Failed => {
            ui.painter().rect_filled(square, radius, t.bg_panel);
            if row.thumb == Thumb::Failed || row.error.is_some() {
                ui.painter().text(
                    square.center(),
                    egui::Align2::CENTER_CENTER,
                    icons::CUBE,
                    egui::FontId::new(24.0, super::fonts::icons()),
                    t.text_secondary,
                );
            } else if ui.is_rect_visible(square) {
                wanted.push(row.path.clone());
            }
        }
    }
    ui.painter().rect_stroke(
        square,
        radius,
        Stroke::new(1.0, t.border),
        StrokeKind::Inside,
    );
    ui.painter()
        .galley(square.left_bottom(), caption, t.text_secondary);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &row.name));
    let response = match &row.error {
        Some(why) => response.on_hover_text(why),
        None if row.kind == LibraryKind::Material => response.on_hover_text(format!(
            "{}: click to add to the model and paint with it",
            row.name
        )),
        None => response.on_hover_text(format!("{}: click to place", row.name)),
    };
    if response.clicked() && row.error.is_none() {
        actions.push(UiAction::LibraryUse(row.path.clone()));
    }
    response.context_menu(|ui| {
        widgets::name_egui_widget(
            ui.ctx(),
            ui.layer_id().id.with("move"),
            egui::accesskit::Role::Menu,
            || format!("{} menu", row.name),
        );
        let item = |ui: &mut Ui, label: &str, enabled: bool| {
            widgets::menu_item(ui, label, None, enabled).clicked()
        };
        let usable = row.error.is_none();
        if item(ui, "Insert", usable) {
            actions.push(UiAction::LibraryInsert(row.path.clone()));
        }
        if item(ui, "Open", usable) {
            actions.push(UiAction::LibraryOpen(row.path.clone()));
        }
        if item(ui, "Delete from Library", true) {
            actions.push(UiAction::LibraryDelete(row.path.clone()));
        }
    });
}
