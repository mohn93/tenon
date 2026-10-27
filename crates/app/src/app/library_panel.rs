//! The window's side of the component library: the folder and its listing, the thumbnails
//! tiles asked for, and what the Library tab, the save sheets and Settings ask the window to
//! do.

use std::path::{Path, PathBuf};

use session::{LibraryItem, LibraryKind, LibraryRef, library as listing};

use super::*;
use crate::library::{self, Slot, ThumbKey, Thumbnails};
use crate::ui::{DockTab, LibraryRow, LibrarySave, Thumb};

/// What the window holds of the library folder.
#[derive(Default)]
pub(super) struct LibraryState {
    /// The folder's items as last listed, sorted by name.
    items: Vec<LibraryItem>,
    /// The folder holds more than were listed.
    more: bool,
    /// Why the folder could not be listed.
    error: Option<String>,
    /// The listing is out of date: it is made again when the tab is shown.
    stale: bool,
    thumbs: Thumbnails<egui::TextureHandle>,
    /// The tab's rows, built from `items` and `thumbs` when either changed.
    rows: Vec<LibraryRow>,
    /// `rows` no longer match `items` and `thumbs`.
    rows_stale: bool,
}

impl LibraryState {
    /// Nothing listed yet: the first time the tab is shown it lists the folder.
    pub(super) fn new() -> Self {
        Self {
            stale: true,
            rows_stale: true,
            ..Self::default()
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// The thumbnails belonged to the old panels' context.
    pub(super) fn forget_thumbnails(&mut self) {
        self.thumbs.clear();
        self.rows_stale = true;
    }

    /// The rows the tab shows (see `State::update_library_rows`).
    pub(super) fn rows(&self) -> &[LibraryRow] {
        &self.rows
    }

    pub(super) fn more(&self) -> bool {
        self.more
    }

    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl State {
    /// The library folder, when one is chosen.
    pub(super) fn library_folder(&self) -> Option<&Path> {
        self.prefs.library.as_deref()
    }

    fn library_shown(&self) -> bool {
        self.prefs.dock.tab == DockTab::Library
    }

    /// Marks the listing out of date (after a save or delete, a focus change or a new folder).
    pub(super) fn library_changed(&mut self) {
        self.library.stale = true;
    }

    /// Lists the folder again.
    fn refresh_library(&mut self) {
        self.library.stale = false;
        self.library.rows_stale = true;
        let Some(folder) = self.prefs.library.clone() else {
            self.library.items.clear();
            self.library.more = false;
            self.library.error = None;
            self.library.thumbs.clear();
            return;
        };
        match listing::list(&folder) {
            Ok(found) => {
                self.library.items = found.items;
                self.library.more = found.more;
                self.library.error = None;
            }
            Err(e) => {
                self.library.items.clear();
                self.library.more = false;
                self.library.error = Some(e.message);
            }
        }
        let listed: Vec<(PathBuf, (u64, u64))> = self
            .library
            .items
            .iter()
            .map(|i| (PathBuf::from(&i.path), (i.modified, i.size)))
            .collect();
        self.library.thumbs.keep_only(&listed);
    }

    /// Brings the Library tab's rows up to date: lists the folder first when it is out of date
    /// and the tab is shown; the rows are built again only when the listing or a thumbnail
    /// changed (`LibraryState::rows`).
    pub(super) fn update_library_rows(&mut self) {
        if !self.library_shown() {
            return;
        }
        if self.library.stale {
            self.refresh_library();
        }
        if !self.library.rows_stale {
            return;
        }
        self.library.rows_stale = false;
        let lib = &self.library;
        let rows = lib
            .items
            .iter()
            .map(|item| {
                let path = PathBuf::from(&item.path);
                let mut error = item.error.clone();
                let thumb = if error.is_some() {
                    Thumb::Failed
                } else {
                    match lib.thumbs.get(&path, (item.modified, item.size)) {
                        Some(Slot::Ready(handle)) => Thumb::Ready(handle.id()),
                        Some(Slot::Failed(why)) => {
                            // An item that cannot be read for use is shown as unusable.
                            error.clone_from(why);
                            Thumb::Failed
                        }
                        None => Thumb::Pending,
                    }
                };
                LibraryRow {
                    path,
                    name: item.name.clone(),
                    kind: item.kind,
                    error,
                    thumb,
                }
            })
            .collect();
        self.library.rows = rows;
    }

    /// Notes the tiles in view that have no thumbnail (a tile asks every frame until it has
    /// one). Paths the listing does not hold are ignored.
    pub(super) fn want_thumbnails(&mut self, paths: &[PathBuf]) {
        for path in paths {
            if let Some(item) = self.library_item(path) {
                let key = ThumbKey {
                    path: path.clone(),
                    stamp: (item.modified, item.size),
                };
                self.library.thumbs.want(key);
            }
        }
    }

    /// Makes the thumbnails asked for, at most `THUMBS_PER_FRAME` (the tiles ask again while
    /// they still show none, which brings the next frame).
    pub(super) fn make_thumbnails(&mut self) {
        for key in self.library.thumbs.next_batch() {
            let made = self.offscreen.get().map_or_else(
                || Err(None),
                |snapshotter| library::make(snapshotter, &key.path),
            );
            let texture = made.map(|pixels| {
                let side = library::THUMB_SIZE as usize;
                self.gui.ctx().load_texture(
                    format!("library-thumb-{}", key.path.display()),
                    egui::ColorImage::from_rgba_unmultiplied([side, side], &pixels.0),
                    egui::TextureOptions::LINEAR,
                )
            });
            self.library.thumbs.store(key, texture);
            self.library.rows_stale = true;
        }
    }

    fn library_item(&self, path: &Path) -> Option<&LibraryItem> {
        self.library
            .items
            .iter()
            .find(|i| Path::new(&i.path) == path)
    }

    /// Chooses the library folder in a dialog.
    pub(super) fn choose_library_folder(&mut self) {
        self.flow(|flows, env| flows.choose_folder(env));
    }

    pub(super) fn set_library_folder(&mut self, folder: Option<PathBuf>) {
        if self.prefs.library != folder {
            self.prefs.library = folder;
            self.save_prefs();
            self.library.thumbs.clear();
            self.library.items.clear();
            self.library.stale = true;
            self.library.rows_stale = true;
        }
    }

    /// A tile was clicked: a material is added and chosen, anything else placed with the Paste
    /// tool. A refusal is in the status.
    pub(super) fn library_use(&mut self, path: &Path) {
        match self.library_item(path).map(|i| i.kind) {
            Some(LibraryKind::Material) => {
                let _ = self.editor.insert_library_item(path);
            }
            Some(_) => {
                let _ = self.editor.place_library_item(path);
            }
            None => self
                .editor
                .show_error("The item is no longer in the library"),
        }
    }

    /// An item's Insert: at the open context's origin, or (a material) added and chosen.
    pub(super) fn library_insert(&mut self, path: &Path) {
        let _ = self.editor.insert_library_item(path);
    }

    /// An item's Open: its file becomes the open model.
    pub(super) fn library_open(&mut self, path: &Path) {
        // The caps every library read passes: a nesting bomb is refused here, not opened.
        if let Err(e) = session::library::read_item(path) {
            self.editor.show_error(&e.message);
            return;
        }
        self.settle_and(Then::OpenFile(path.to_path_buf()));
    }

    pub(super) fn library_ask_delete(&mut self, path: PathBuf) {
        let name = self
            .library_item(&path)
            .map_or_else(|| file_name(Some(&path)), |i| i.name.clone());
        self.open_sheet(Sheet::DeleteLibraryItem { path, name });
    }

    pub(super) fn library_delete(&mut self, path: &Path) {
        let Some(folder) = self.library_folder().map(Path::to_path_buf) else {
            return;
        };
        match library::delete_item(&folder, path) {
            Ok(()) => {
                self.editor.show_info("Deleted from the library");
                self.library_changed();
            }
            Err(why) => self.editor.show_error(&format!("Could not delete: {why}")),
        }
    }

    /// Opens the name sheet for saving `save` to the library.
    pub(super) fn library_ask_name(&mut self, save: LibrarySave) {
        if self.library_folder().is_none() {
            self.editor
                .show_error("Choose a library folder in Settings > Files first");
            return;
        }
        let doc = self.editor.doc();
        let suggested = match save {
            LibrarySave::Component(id) => doc
                .model()
                .instance(id)
                .and_then(|i| doc.scene().definition(i.definition))
                .map(|d| d.name.clone()),
            LibrarySave::Material(id) => doc.scene().material(id).map(|m| m.name.clone()),
            LibrarySave::Model => doc
                .path()
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().into_owned()),
        }
        .unwrap_or_else(|| "Untitled".into());
        self.open_sheet(Sheet::SaveToLibrary { save, suggested });
    }

    /// The save sheet's Save: writes `<folder>/<name>.tnn`, or asks before replacing a file.
    pub(super) fn library_save_named(&mut self, save: LibrarySave, name: &str) {
        let Some(folder) = self.library_folder().map(Path::to_path_buf) else {
            return;
        };
        let name = name.trim();
        let Some(file) = library::file_name_for(name) else {
            self.editor.show_error("Type a name for the item");
            return;
        };
        if std::fs::symlink_metadata(folder.join(&file)).is_ok() {
            self.open_sheet(Sheet::OverwriteLibrary {
                save,
                name: name.to_string(),
                file,
            });
        } else {
            self.library_write(save, name, &file, false);
        }
    }

    /// Writes `<folder>/<file>` holding `save` named `name`.
    pub(super) fn library_write(
        &mut self,
        save: LibrarySave,
        name: &str,
        file: &str,
        overwrite: bool,
    ) {
        let Some(folder) = self.library_folder().map(Path::to_path_buf) else {
            return;
        };
        let item = match save {
            LibrarySave::Component(id) => Some(LibraryRef::Entity(EntityId::Instance(id))),
            LibrarySave::Material(id) => Some(LibraryRef::Material(id)),
            LibrarySave::Model => None,
        };
        let saved =
            self.editor
                .save_to_library(&folder.join(file), save.kind(), item, name, overwrite);
        if saved.is_ok() {
            self.library_changed();
        }
    }
}
