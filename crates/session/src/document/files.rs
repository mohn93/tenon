//! New, open and save, and editing definitions' names and the definitions list.

use std::path::{Path, PathBuf};

use kernel::{DefinitionId, DefinitionKind, MeshId, Scene, SceneChange};

use super::{Document, SectionDisplay};
use crate::camera::{Camera, Lighting};
use crate::command::{CommandError, ErrorKind};
use crate::export::{self, ExportFormat, ExportReport};
use crate::file::{self, Access, ViewLook};
use crate::import::{ImportReport, MAX_IMPORT_BYTES};
use crate::skp_export;
use crate::sniff::{self, Kind};

/// What [`Document::export_bytes`] makes.
#[derive(Clone, Debug)]
pub struct ExportedBytes {
    /// The exported file's bytes.
    pub bytes: Vec<u8>,
    /// The files an OBJ names beside it, by file name: its material library (`<stem>.mtl`) and
    /// the texture images the library names. Empty for the other formats.
    pub side_files: Vec<(String, Vec<u8>)>,
    pub report: ExportReport,
}

impl Document {
    /// Writes the model, view and lighting as a `.skr` file at `path`, leaving the document's
    /// own file and its unsaved state as they are (a recovery copy). Only the user may read
    /// the copy (0600).
    pub fn write_copy(&self, path: &Path) -> Result<(), CommandError> {
        file::write(
            path,
            &self.scene,
            &self.camera,
            self.view_look(),
            Access::Private,
            &self.info_entries(),
        )
        .map_err(|e| file_error(&path.display().to_string(), e))
    }

    /// The extra archive entries a save writes: the `library.json` of the library item the
    /// model was opened from, if it was.
    fn info_entries(&self) -> Vec<(&str, &[u8])> {
        self.library_info
            .as_deref()
            .map(|bytes| (crate::library::INFO_ENTRY, bytes))
            .into_iter()
            .collect()
    }

    /// The lighting and coloring a file saves with the camera.
    pub(super) fn view_look(&self) -> ViewLook {
        ViewLook {
            lighting: self.lighting,
            color_by_tag: self.color_by_tag,
            section: self.section_display,
            units: self.units,
        }
    }

    /// Refuses with `unsaved_changes` when the model changed since it was opened, saved or
    /// started, unless `discard`: what `new`, `open` and quitting check first.
    pub fn check_unsaved(&self, discard: bool) -> Result<(), CommandError> {
        if self.is_modified() && !discard {
            return Err(CommandError::new(
                ErrorKind::UnsavedChanges,
                "the model has unsaved changes: save it first, or pass \"discard\": true",
            ));
        }
        Ok(())
    }

    /// An empty model with the default view, lit by day.
    pub(super) fn new_model(&mut self) {
        let camera = Camera::new(self.camera.viewport);
        self.replace(Scene::new(), camera);
        self.lighting = Lighting::Day;
        self.color_by_tag = false;
        self.section_display = SectionDisplay::default();
        self.path = None;
        self.library_info = None;
    }

    /// Replaces the model, view and lighting with a file's. A file whose view cannot be used
    /// (an eye on its target, say) opens with the default view. A SketchUp file (by its
    /// extension or its header) opens as a new model never saved, with its import report.
    pub(super) fn open_file(&mut self, path: &str) -> Result<Option<ImportReport>, CommandError> {
        let file = Path::new(path);
        check_nul(file)?;
        if file
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("skp"))
            || sniff::is_skp(file)
        {
            return self.open_skp(path).map(Some);
        }
        let opened = file::read(Path::new(path)).map_err(|e| file_error(path, e))?;
        self.apply_opened(opened);
        self.path = Some(PathBuf::from(path));
        self.library_info = crate::library::info_of(Path::new(path));
        Ok(None)
    }

    /// Replaces the model, view, lighting and units with those of an opened `.skr` file. A view
    /// that cannot be used opens as the default view. The file's path is for the caller to set.
    fn apply_opened(&mut self, opened: file::Opened) {
        let c = opened.camera;
        let camera = self
            .camera_for(c.eye, c.target, Some(c.fov_y_deg), c.up, Some(c.projection))
            .unwrap_or_else(|_| Camera::new(self.camera.viewport));
        self.replace(opened.scene, camera);
        self.units = opened.units;
        self.lighting = c.lighting;
        self.color_by_tag = c.color_by_tag;
        self.section_display = SectionDisplay {
            planes: c.section_planes,
            cuts: c.section_cuts,
            fill: c.section_fill,
        };
    }

    /// Opens a file given as bytes, as `open` does a path (this is the web's open, where there
    /// are no paths): a `.skr` file replaces the model, view, lighting and units, a SketchUp
    /// file (by its extension or its header) opens as a new model never saved, with its import
    /// report. `name` is the file's name, for messages and the extension. Refuses with
    /// `unsaved_changes` unless `discard`. The model has no path afterwards: save it with
    /// [`Document::save_bytes`]. Nothing changes on failure.
    pub fn open_bytes(
        &mut self,
        name: &str,
        bytes: &[u8],
        discard: bool,
    ) -> Result<Option<ImportReport>, CommandError> {
        self.check_unsaved(discard)?;
        check_nul(Path::new(name))?;
        let skp = Path::new(name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("skp"))
            || sniff::is_skp_bytes(bytes);
        if skp {
            if bytes.len() as u64 > MAX_IMPORT_BYTES {
                return Err(CommandError::new(
                    ErrorKind::File,
                    format!("{name}: the file is over {MAX_IMPORT_BYTES} bytes"),
                ));
            }
            let report = self.open_skp_bytes(name, bytes.to_vec())?;
            self.imported_from = None;
            return Ok(Some(report));
        }
        let opened = file::read_slice(bytes).map_err(|e| file_error(name, e))?;
        self.apply_opened(opened);
        self.path = None;
        self.library_info = None;
        Ok(None)
    }

    /// The model, view, lighting and units as the bytes of a `.skr` file, as `save` writes
    /// them, and marks the model as saved (no unsaved changes). The model's own path, if any,
    /// is left as it is: the caller decides where the bytes go.
    pub fn save_bytes(&mut self) -> Result<Vec<u8>, CommandError> {
        let bytes = file::encode(
            &self.scene,
            &self.camera,
            self.view_look(),
            &self.info_entries(),
        )
        .map_err(|e| file_error("the model", e))?;
        self.imported_from = None;
        self.clean_revision = self.revision;
        Ok(bytes)
    }

    /// Replaces the model, view and lighting with those of the `.skr` copy at `path` (a
    /// recovery file), as a model never saved: it has no file of its own and counts as
    /// modified until it is saved. Nothing changes on failure.
    pub fn open_recovered(&mut self, path: &Path) -> Result<(), CommandError> {
        let name = path.display().to_string();
        check_nul(path)?;
        let opened = file::read(path).map_err(|e| file_error(&name, e))?;
        self.apply_opened(opened);
        self.path = None;
        self.library_info = crate::library::info_of(path);
        // The revision before the model was replaced, which it never returns to.
        self.clean_revision = self.revision - 1;
        Ok(())
    }

    /// Replaces the model with a SketchUp file's: its root geometry and instances go into the
    /// root (no wrapper), the view zooms to them, lit by day, and the model has no file (so
    /// saving asks for a `.skr` path). Nothing changes on failure.
    fn open_skp(&mut self, path: &str) -> Result<ImportReport, CommandError> {
        let bytes = super::import::read_limited(Path::new(path), MAX_IMPORT_BYTES)
            .map_err(|e| file_error(path, e))?;
        let report = self.open_skp_bytes(path, bytes)?;
        self.imported_from = Some(PathBuf::from(path));
        Ok(report)
    }

    /// [`Document::open_skp`] for the file's bytes; `name` is for messages.
    fn open_skp_bytes(&mut self, name: &str, bytes: Vec<u8>) -> Result<ImportReport, CommandError> {
        let path = name;
        // Built aside, so a file that fails leaves the open model as it is.
        let mut fresh = Document::new(self.camera.viewport);
        let built = fresh.build_skp(path, bytes, 1.0, |scene, tree| {
            scene
                .import_tree_into_root(MeshId::Root, tree)
                .map(|(r, c)| (None, r, c))
        })?;
        let report = fresh.skp_report(&built);
        let mut camera = Camera::new(self.camera.viewport);
        if let Some([lo, hi]) = fresh.scene.shown_bounds(MeshId::Root, &[]) {
            camera.fit(lo, hi);
        }
        self.replace(fresh.scene, camera);
        self.lighting = Lighting::Day;
        self.color_by_tag = false;
        self.section_display = SectionDisplay::default();
        self.path = None;
        self.library_info = None;
        Ok(report)
    }

    /// Saves to `path`, or to the file last opened or saved.
    pub(super) fn save_file(
        &mut self,
        path: Option<&str>,
        overwrite: bool,
    ) -> Result<(), CommandError> {
        let target = path
            .map(PathBuf::from)
            .or_else(|| self.path.clone())
            .ok_or_else(|| {
                CommandError::new(
                    ErrorKind::InvalidInput,
                    "give a path: the model was never opened or saved",
                )
            })?;
        check_nul(&target)?;
        if !overwrite && !sniff::replaceable(&target, Kind::Skr) {
            return Err(sniff::refusal(&target, Kind::Skr));
        }
        file::write(
            &target,
            &self.scene,
            &self.camera,
            self.view_look(),
            Access::Default,
            &self.info_entries(),
        )
        .map_err(|e| file_error(&target.display().to_string(), e))?;
        self.path = Some(target);
        self.imported_from = None;
        self.clean_revision = self.revision;
        Ok(())
    }

    /// Writes every face of the whole model as triangles in `format`, or the format the
    /// path's extension names; a SketchUp file holds the whole scene instead (see
    /// `skp_export`).
    pub(super) fn export_file(
        &self,
        path: &str,
        format: Option<ExportFormat>,
        overwrite: bool,
        refuse_links: bool,
        scale: Option<f64>,
    ) -> Result<ExportReport, CommandError> {
        let Some(format) = format.or_else(|| ExportFormat::from_path(Path::new(path))) else {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "give a format, or a path ending in .obj, .stl, .glb, .gltf, .skp or .svg",
            ));
        };
        self.check_export_scale(format, scale)?;
        let target = Path::new(path);
        check_nul(target)?;
        if target
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("skr"))
        {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "a .skr path is a model file: use save, not export",
            ));
        }
        let own = |p: &Path| self.path.as_deref().is_some_and(|own| same_file(own, p));
        if own(target) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "that is the open model's own file: export to another path",
            ));
        }
        // A model opened from a SketchUp file has no file of its own; the original keeps
        // what this app cannot (scenes, curves, newer formats), so writing over it
        // needs `overwrite`.
        if !overwrite
            && self
                .imported_from
                .as_deref()
                .is_some_and(|original| same_file(original, target))
        {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "this is the SketchUp file the model was opened from; exporting would replace \
                 it with a 2017 copy that drops what this app cannot keep; pass overwrite to \
                 replace it",
            ));
        }
        if format == ExportFormat::Skp {
            return export_skp(&self.scene, target, overwrite, refuse_links);
        }
        if format == ExportFormat::Svg {
            return self.export_svg(target, overwrite, refuse_links, scale);
        }
        // An OBJ with painted faces names a material library beside it: same name, `.mtl`.
        let mtl_path = target.with_extension("mtl");
        let mtl_name = mtl_path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let (encoded, report) = self.encode_triangles(format, &mtl_name)?;
        if encoded.mtl.is_some() {
            // On a case-blind file system "x.MTL" is the library's own path.
            let same = |a: &Path, b: &Path| {
                a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
            };
            if same(&mtl_path, target) || own(&mtl_path) {
                return Err(CommandError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "an OBJ of painted faces writes its materials to {}, which is not \
                         free: export to another path",
                        mtl_path.display()
                    ),
                ));
            }
        }
        // The files written beside an OBJ: its library and the images the library names.
        let side: Vec<(PathBuf, Kind)> = match &encoded.mtl {
            Some(_) => std::iter::once((mtl_path.clone(), Kind::Mtl))
                .chain(
                    encoded
                        .images
                        .iter()
                        .map(|(name, _)| (mtl_path.with_file_name(name), Kind::Image)),
                )
                .collect(),
            None => Vec::new(),
        };
        if refuse_links {
            check_links(std::iter::once(target).chain(side.iter().map(|(p, _)| p.as_path())))?;
        }
        if !overwrite {
            let kind = match format {
                ExportFormat::Obj => Kind::Obj,
                ExportFormat::Stl => Kind::Stl,
                ExportFormat::Glb => Kind::Glb,
                ExportFormat::Gltf => Kind::Gltf,
                ExportFormat::Skp => Kind::Skp,
                ExportFormat::Svg => Kind::Svg,
            };
            let checks = std::iter::once((target.to_path_buf(), kind)).chain(side);
            if let Some((p, k)) = checks.into_iter().find(|(p, k)| !sniff::replaceable(p, *k)) {
                return Err(sniff::refusal(&p, k));
            }
        }
        if let Some(mtl) = &encoded.mtl {
            // Texture images go beside the library, before it, as it names them.
            for (name, bytes) in &encoded.images {
                let image = mtl_path.with_file_name(name);
                file::replace_file(&image, bytes, Access::Default)
                    .map_err(|e| file_error(&image.display().to_string(), e))?;
            }
            file::replace_file(&mtl_path, mtl, Access::Default)
                .map_err(|e| file_error(&mtl_path.display().to_string(), e))?;
        }
        file::replace_file(target, &encoded.bytes, Access::Default)
            .map_err(|e| file_error(path, e))?;
        Ok(report)
    }

    /// The scale checks of an export: a scale is for SVG in a parallel view, positive.
    fn check_export_scale(
        &self,
        format: ExportFormat,
        scale: Option<f64>,
    ) -> Result<(), CommandError> {
        let Some(n) = scale else {
            return Ok(());
        };
        if format != ExportFormat::Svg {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "a scale applies to SVG exports only",
            ));
        }
        if !(n.is_finite() && n > 0.0) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "the scale must be a positive number (1:scale)",
            ));
        }
        if !self.camera.is_parallel() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "Switch to parallel projection to export at scale",
            ));
        }
        Ok(())
    }

    /// Every face of the model as triangles encoded as an OBJ, STL or glTF (`mtl_name` is the
    /// material library an OBJ names), with the report. `invalid_input` when there is nothing
    /// to write.
    fn encode_triangles(
        &self,
        format: ExportFormat,
        mtl_name: &str,
    ) -> Result<(export::Encoded, ExportReport), CommandError> {
        let (triangles, skipped_faces) = self.scene.triangles_and_skipped();
        if triangles.is_empty() {
            let message = if skipped_faces > 0 {
                format!("nothing to export: {skipped_faces} faces could not be triangulated")
            } else if self.scene.hides_faces() {
                "nothing to export: no face is shown (hidden tags hide the rest)".to_string()
            } else {
                "nothing to export: the model has no faces".to_string()
            };
            return Err(CommandError::new(ErrorKind::InvalidInput, message));
        }
        let encoded = export::encode(&self.scene, &triangles, format, mtl_name)
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e))?;
        let report = ExportReport {
            triangles: triangles.len(),
            skipped_faces,
            warnings: export::annotation_warnings(&self.scene),
            ..ExportReport::default()
        };
        Ok((encoded, report))
    }

    /// Exports the model in `format` as bytes, as `export` writes a file (this is the web's
    /// export, where there are no paths). `stem` is the file name without extension, which an
    /// OBJ with painted faces uses to name its material library (`<stem>.mtl`); empty means "model", and a path separator is `invalid_input`. `scale` is for
    /// SVG as in `export`. The result holds the file's bytes, the files an OBJ names beside it
    /// (its `.mtl` and the texture images) and the report.
    pub fn export_bytes(
        &self,
        format: ExportFormat,
        stem: &str,
        scale: Option<f64>,
    ) -> Result<ExportedBytes, CommandError> {
        let stem = if stem.is_empty() { "model" } else { stem };
        if stem.contains(['/', '\\', '\0']) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "the file name must not contain a path separator",
            ));
        }
        self.check_export_scale(format, scale)?;
        match format {
            ExportFormat::Skp => {
                let (bytes, report) = skp_bytes(&self.scene)?;
                Ok(ExportedBytes {
                    bytes,
                    side_files: Vec::new(),
                    report,
                })
            }
            ExportFormat::Svg => {
                let (bytes, report) = self.svg_bytes(scale)?;
                Ok(ExportedBytes {
                    bytes,
                    side_files: Vec::new(),
                    report,
                })
            }
            _ => {
                let (encoded, report) = self.encode_triangles(format, &format!("{stem}.mtl"))?;
                let side_files = encoded
                    .mtl
                    .map(|mtl| (format!("{stem}.mtl"), mtl))
                    .into_iter()
                    .chain(encoded.images)
                    .collect();
                Ok(ExportedBytes {
                    bytes: encoded.bytes,
                    side_files,
                    report,
                })
            }
        }
    }

    /// Checks that `path` may be written with a PDF the way an export is: not a `.skr`, not the
    /// open model's own file, no link under `refuse_links`, and nothing but a PDF (or nothing)
    /// there unless `overwrite`. Returns the path to write.
    pub fn check_pdf_target(
        &self,
        path: &str,
        overwrite: bool,
        refuse_links: bool,
    ) -> Result<PathBuf, CommandError> {
        let target = Path::new(path);
        check_nul(target)?;
        if target
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("skr"))
        {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "a .skr path is a model file: use save, not export_pdf",
            ));
        }
        if self
            .path
            .as_deref()
            .is_some_and(|own| same_file(own, target))
        {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "that is the open model's own file: print to another path",
            ));
        }
        if !overwrite
            && self
                .imported_from
                .as_deref()
                .is_some_and(|original| same_file(original, target))
        {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "this is the SketchUp file the model was opened from; pass overwrite to replace it",
            ));
        }
        if refuse_links {
            check_links(std::iter::once(target))?;
        }
        if !overwrite && !sniff::replaceable(target, Kind::Pdf) {
            return Err(sniff::refusal(target, Kind::Pdf));
        }
        Ok(target.to_path_buf())
    }

    /// Writes `pdf` at `path` after the checks of [`Document::check_pdf_target`].
    pub fn write_pdf(
        &self,
        path: &str,
        pdf: &[u8],
        overwrite: bool,
        refuse_links: bool,
    ) -> Result<(), CommandError> {
        let target = self.check_pdf_target(path, overwrite, refuse_links)?;
        file::replace_file(&target, pdf, Access::Default).map_err(|e| file_error(path, e))
    }

    /// Writes the current view as a line drawing in SVG (see `drawing` and `svg`): in pixels at
    /// the viewport's size, or with `scale` (1:N, a parallel view) in millimetres.
    fn export_svg(
        &self,
        target: &Path,
        overwrite: bool,
        refuse_links: bool,
        scale: Option<f64>,
    ) -> Result<ExportReport, CommandError> {
        if refuse_links {
            check_links(std::iter::once(target))?;
        }
        if !overwrite && !sniff::replaceable(target, Kind::Svg) {
            return Err(sniff::refusal(target, Kind::Svg));
        }
        let (bytes, report) = self.svg_bytes(scale)?;
        file::replace_file(target, &bytes, Access::Default)
            .map_err(|e| file_error(&target.display().to_string(), e))?;
        Ok(report)
    }

    /// The current view as an SVG line drawing, with its report.
    fn svg_bytes(&self, scale: Option<f64>) -> Result<(Vec<u8>, ExportReport), CommandError> {
        let camera = &self.camera;
        let drawing = crate::drawing::line_drawing(self, camera, camera.viewport)?;
        if drawing.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "nothing to export: no edge is in view (hidden tags and section cuts hide the rest)",
            ));
        }
        // Millimetres on paper per pixel of the view: a pixel is `pixel_size` meters of the
        // model in a parallel view.
        let mm_per_px = scale.map(|n| camera.pixel_size(camera.target) * 1000.0 / n);
        let bytes = crate::svg::to_svg(&drawing, mm_per_px).into_bytes();
        let report = ExportReport {
            lines: drawing.lines.len() + drawing.cut_lines.len(),
            ..ExportReport::default()
        };
        Ok((bytes, report))
    }

    /// Swaps in another model: history, selection and the open context start over, and event
    /// subscribers are told to read the model again.
    /// The display units go back to the default; a file's are set after.
    fn replace(&mut self, scene: Scene, camera: Camera) {
        self.scene = scene;
        self.camera = camera;
        self.units = crate::units::Units::default();
        self.imported_from = None;
        self.context.clear();
        self.selection.clear();
        self.selected_scene = None;
        self.undo.clear();
        self.redo.clear();
        self.revision += 1;
        self.events.clear();
        self.log_start = self.revision;
        self.clean_revision = self.revision;
        self.token = super::clip::new_token();
        self.pasted.clear();
    }

    pub(super) fn rename_changes(
        &mut self,
        id: DefinitionId,
        name: &str,
    ) -> Result<SceneChange, CommandError> {
        let Some(d) = self.scene.definition(id) else {
            return Err(CommandError::new(
                ErrorKind::NotFound,
                format!("no definition {id}"),
            ));
        };
        if d.kind == DefinitionKind::Component && name.trim().is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "a component needs a name",
            ));
        }
        self.scene
            .rename_definition(id, name)
            .map_err(|e| CommandError::new(ErrorKind::NotFound, e.to_string()))
    }
}

/// Writes the whole scene as a SketchUp file at `target`, after the checks every export
/// makes: something to write, no symbolic link under `--api-root`, and no existing file
/// that is not a SketchUp file unless `overwrite`.
fn export_skp(
    scene: &Scene,
    target: &Path,
    overwrite: bool,
    refuse_links: bool,
) -> Result<ExportReport, CommandError> {
    let (bytes, report) = skp_bytes(scene)?;
    if refuse_links {
        check_links(std::iter::once(target))?;
    }
    if !overwrite && !sniff::replaceable(target, Kind::Skp) {
        return Err(sniff::refusal(target, Kind::Skp));
    }
    file::replace_file(target, &bytes, Access::Default)
        .map_err(|e| file_error(&target.display().to_string(), e))?;
    Ok(report)
}

/// The whole scene as the bytes of a SketchUp file, with the report.
fn skp_bytes(scene: &Scene) -> Result<(Vec<u8>, ExportReport), CommandError> {
    let (bytes, stats) = skp_export::encode_skp(scene)?;
    Ok((
        bytes,
        ExportReport {
            definitions: stats.definitions,
            instances: stats.instances,
            faces: stats.faces,
            edges: stats.edges,
            guides: stats.guides,
            materials: stats.materials,
            tags: stats.tags,
            warnings: stats.warnings,
            ..ExportReport::default()
        },
    ))
}

/// Refuses the first of `paths` that is a symbolic link (on Windows any reparse point, such as
/// a junction): under `--api-root` the API does not write through links.
fn check_links<'a>(mut paths: impl Iterator<Item = &'a Path>) -> Result<(), CommandError> {
    match paths.find(|p| std::fs::symlink_metadata(p).is_ok_and(|m| crate::fspath::is_link(&m))) {
        Some(link) => Err(CommandError::new(
            ErrorKind::InvalidInput,
            format!(
                "{} is a symbolic link or, on Windows, another reparse point (a junction, a cloud \
                 file placeholder): under --api-root the API does not write to links",
                link.display()
            ),
        )),
        None => Ok(()),
    }
}

/// True when `a` and `b` name the same file: compared after resolving both, or as given
/// when either cannot be resolved (a file not there yet).
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Refuses a path holding a NUL byte, which no file name can hold.
pub(super) fn check_nul(path: &Path) -> Result<(), CommandError> {
    if path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            format!(
                "{} contains a NUL byte, which no file name can hold",
                path.display().to_string().escape_debug()
            ),
        ));
    }
    Ok(())
}

fn file_error(path: &str, e: String) -> CommandError {
    CommandError::new(ErrorKind::File, format!("{path}: {e}"))
}
