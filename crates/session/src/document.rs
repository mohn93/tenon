use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use glam::{DAffine3, DVec2, DVec3};
use kernel::offset::{self, Polyline};
use kernel::{
    ChangeSet, DefinitionId, EdgeId, EntityId, FaceId, Guide, GuideId, Instance, InstanceId,
    KernelError, MaterialEdit, MaterialId, MeshId, Model, PageId, Parts, PlacementEdit, Scene,
    SceneChange, SoftenReport, TagEdit, VertexId,
};

use crate::arc::{DEFAULT_ARC_SEGMENTS, arc_points, arc_spec};
use crate::batch::BatchCommand;
use crate::camera::{Camera, Lighting};
use crate::circle::{DEFAULT_CIRCLE_SEGMENTS, circle_points};
use crate::command::{
    BatchError, Command, CommandError, ErrorKind, IntersectReport, Origin, Outcome, SelectMode,
};
use crate::import::ImportReport;
use crate::polygon::{DEFAULT_POLYGON_SIDES, polygon_points};
use crate::query::{
    ChangeEvent, CurveInfo, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, GuideInfo, GuideKind,
    Query, QueryResult, VertexInfo,
};
use crate::rectangle::rectangle_corners;
use crate::rotate::rotation;
use crate::text3d::Text3dReport;
use crate::units::Units;

mod annotations;
mod camera;
mod clip;
mod files;
mod follow_me;
mod groups;
mod import;
mod intersect;
mod library;
mod materials;
mod placement;
mod scenes;
mod sections;
mod soften;
mod tags;
mod text3d;

pub use annotations::{dimension_label, dimension_value, note_label};
pub use files::ExportedBytes;
pub use import::ImportOptions;
pub use sections::SectionDisplay;

/// Largest coordinate magnitude (meters) accepted from commands.
pub const MAX_COORDINATE: f64 = 1.0e6;

/// Most copies one `copy` or `rotate` command may make.
pub const MAX_COPIES: u32 = 1000;

/// How many recent model changes `Query::Events` can return.
const EVENT_LOG_LEN: usize = 1024;

/// One undo step: the model change and the label of what made it (see `Command::name`).
struct Step {
    label: String,
    origin: Origin,
    changes: SceneChange,
    /// The display units before and after, when the step changed them.
    units: Option<(Units, Units)>,
}

/// The open model plus everything needed to edit it: camera, selection, the open context and
/// undo history.
pub struct Document {
    scene: Scene,
    /// The instances being edited, from the root; empty when the root is open.
    context: Vec<InstanceId>,
    pub camera: Camera,
    /// How the view is lit; saved with the camera.
    pub lighting: Lighting,
    /// Faces are drawn in their tags' colors; saved with the camera.
    pub color_by_tag: bool,
    /// Whether section planes are drawn, cut and filled; saved with the camera.
    pub section_display: SectionDisplay,
    /// How labels write lengths; saved with the model, changed by undo steps.
    units: Units,
    selection: BTreeSet<EntityId>,
    /// The scene last added or gone to (see `selected_scene`); not saved.
    selected_scene: Option<PageId>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// Stamped on the steps recorded from now on; `new` and `open` keep it.
    origin: Origin,
    revision: u64,
    events: VecDeque<ChangeEvent>,
    /// Changes up to this revision are missing from `events` (the model was replaced).
    log_start: u64,
    /// The file last opened or saved.
    path: Option<PathBuf>,
    /// The SketchUp file the model was opened from, until it is saved.
    imported_from: Option<PathBuf>,
    /// The `library.json` entry of the library item the model was opened from, written back
    /// by every save so a saved item stays an item (`library::INFO_ENTRY`).
    library_info: Option<Vec<u8>>,
    /// The revision the model had when it was last opened, saved or started.
    clean_revision: u64,
    /// The bytes [`Document::import_bytes`] hands the `import` edit it runs, in place of a file.
    staged_import: Option<Vec<u8>>,
    /// What the last `import` edit made, until its outcome takes it.
    import_report: Option<ImportReport>,
    /// What the last soften edit did, until its outcome takes it.
    soften_report: Option<SoftenReport>,
    /// What the last intersect edit did, until its outcome takes it.
    intersect_report: Option<IntersectReport>,
    /// How many faces the last `position_texture` changed, until its outcome takes it.
    positioned: Option<usize>,
    /// How many definitions the last `purge_definitions` removed, until its outcome takes it.
    purged: Option<usize>,
    /// What the last `add_3d_text` made, until its outcome takes it.
    text_report: Option<Text3dReport>,
    /// The material an `insert_library_item` of a material added or reused, for its outcome.
    library_material: Option<MaterialId>,
    /// Names this document since it was created, opened or imported as a new model (see
    /// `token`); not saved.
    token: String,
    /// The definition each paste made, by the clip's source token and the clip definition it
    /// copied, so pasting the same clip again places it; not saved, forgotten with the model.
    pasted: BTreeMap<(String, DefinitionId), DefinitionId>,
    /// A viewer's document: commands that change the model are refused (see `set_read_only`).
    read_only: bool,
}

impl Document {
    pub fn new(viewport: DVec2) -> Self {
        Self {
            scene: Scene::new(),
            context: Vec::new(),
            camera: Camera::new(viewport),
            lighting: Lighting::Day,
            color_by_tag: false,
            section_display: SectionDisplay::default(),
            units: Units::default(),
            selection: BTreeSet::new(),
            selected_scene: None,
            undo: Vec::new(),
            redo: Vec::new(),
            origin: Origin::User,
            revision: 0,
            events: VecDeque::new(),
            log_start: 0,
            path: None,
            imported_from: None,
            library_info: None,
            clean_revision: 0,
            staged_import: None,
            import_report: None,
            soften_report: None,
            intersect_report: None,
            positioned: None,
            purged: None,
            text_report: None,
            library_material: None,
            token: clip::new_token(),
            pasted: BTreeMap::new(),
            read_only: false,
        }
    }

    /// Makes the document read-only (or not): every command that `changes_model` (editing,
    /// undo and redo, new, open, import) and Save are refused with `invalid_input`; camera, selection
    /// and export commands still run. Opening bytes (`open_bytes`) is not a command and still
    /// replaces the model, which is how a viewer loads what it shows.
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    /// Whether commands that change the model are refused.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn refuse_if_read_only(&self, command: &Command) -> Result<(), CommandError> {
        if self.read_only
            && (command.changes_model()
                || matches!(
                    command,
                    Command::Save { .. } | Command::SaveToLibrary { .. }
                ))
        {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "this model is open for viewing only",
            ));
        }
        Ok(())
    }

    /// Who the steps recorded from now on are attributed to.
    pub fn origin(&self) -> Origin {
        self.origin
    }

    pub fn set_origin(&mut self, origin: Origin) {
        self.origin = origin;
    }

    /// The file the model was last opened from or saved to.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The SketchUp file the model was opened from, until the model is first saved (until
    /// then it has no path of its own, so saving asks for one).
    pub fn imported_from(&self) -> Option<&Path> {
        self.imported_from.as_deref()
    }

    /// True when the model changed since it was opened, saved or started.
    pub fn is_modified(&self) -> bool {
        self.revision != self.clean_revision
    }

    /// The whole model: the root mesh and every definition.
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The scene, for changes made outside `document` (an import's materials).
    pub(crate) fn scene_mut(&mut self) -> &mut Scene {
        &mut self.scene
    }

    /// The mesh being edited: the root, or the definition of the open instance.
    pub fn model(&self) -> &Model {
        self.scene
            .mesh(self.active())
            .expect("the open context exists")
    }

    /// The instances being edited, from the root; empty when the root is open.
    pub fn context(&self) -> &[InstanceId] {
        &self.context
    }

    /// Maps the open context's coordinates to the root's.
    pub fn context_transform(&self) -> DAffine3 {
        self.scene
            .resolve(&self.context)
            .expect("the open context exists")
            .1
    }

    fn active(&self) -> MeshId {
        self.scene
            .resolve(&self.context)
            .expect("the open context exists")
            .0
    }

    /// Runs one kernel edit on the open context's mesh.
    fn edit_mesh(
        &mut self,
        f: impl FnOnce(&mut Model) -> Result<ChangeSet, KernelError>,
    ) -> Result<SceneChange, KernelError> {
        let mesh = self.active();
        self.scene.edit(mesh, f)
    }

    /// The selected edges, faces and instances. Entities leave it when they stop existing.
    pub fn selection(&self) -> &BTreeSet<EntityId> {
        &self.selection
    }

    /// Increases every time the model changes (edits, undo, redo) or `isolate` and `show_all`
    /// change what is shown. Clients compare it to decide
    /// whether to rebuild anything derived from the model. Opening or closing an instance and
    /// selection changes do not bump it; compare `context()` too.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Hides every part but `keep` (root groups and components) from every view of the model:
    /// drawing, picking, snapping and Zoom Extents. This is for looking only: the model, the
    /// undo history and the saved file are untouched, but the revision moves so what is derived
    /// from the model is rebuilt. False (nothing changes) when none of `keep` is a root
    /// instance, or when the document is not read only (only a viewer hides parts).
    pub fn isolate(&mut self, keep: &[kernel::InstanceId]) -> bool {
        if !self.read_only {
            return false;
        }
        let root = self.scene.root();
        if !keep.iter().any(|id| root.instance(*id).is_some()) {
            return false;
        }
        let hidden: BTreeSet<kernel::InstanceId> = root
            .instances()
            .map(|(id, _)| id)
            .filter(|id| !keep.contains(id))
            .collect();
        if hidden == *self.scene.view_hidden() {
            return false;
        }
        self.scene.set_view_hidden(hidden);
        self.view_changed();
        true
    }

    /// Shows every part again after [`Document::isolate`]. False when none was hidden.
    pub fn show_all(&mut self) -> bool {
        if self.scene.view_hidden().is_empty() {
            return false;
        }
        self.scene.set_view_hidden(BTreeSet::new());
        self.view_changed();
        true
    }

    /// Moves the revision for a change of what is shown, not of the model: a document that was
    /// unmodified stays so.
    fn view_changed(&mut self) {
        let clean = !self.is_modified();
        self.revision += 1;
        if clean {
            self.clean_revision = self.revision;
        }
    }

    /// Whether some parts are hidden by [`Document::isolate`].
    pub fn isolated(&self) -> bool {
        !self.scene.view_hidden().is_empty()
    }

    /// Model changes after `since`, oldest first, and whether older ones were dropped.
    pub fn events_since(&self, since: u64) -> (Vec<ChangeEvent>, bool) {
        let truncated = since < self.log_start
            || self
                .events
                .front()
                .is_some_and(|first| first.revision > since.saturating_add(1));
        let events = self
            .events
            .iter()
            .filter(|e| e.revision > since)
            .cloned()
            .collect();
        (events, truncated)
    }

    /// Imports an OBJ, STL or SketchUp file given as bytes, as the `import` command does a path
    /// (this is the web's import, where there are no paths): one undo step, the new group,
    /// component or geometry selected, the outcome's `import` saying what was made. `name` is the
    /// file's name: its extension gives the format unless `options.format` does, and its stem
    /// names the group or component. An OBJ's material libraries are not read (there is no
    /// folder to find them in), so its materials the model lacks are not made.
    pub fn import_bytes(
        &mut self,
        name: &str,
        bytes: Vec<u8>,
        options: &ImportOptions,
    ) -> Result<Outcome, CommandError> {
        self.staged_import = Some(bytes);
        let outcome = self.execute(Command::Import {
            path: name.to_string(),
            format: options.format,
            place: options.place,
            scale: options.scale,
            up: options.up,
            merge_coplanar: options.merge_coplanar,
            library_roots: None,
        });
        self.staged_import = None;
        outcome
    }

    /// Runs one command. Each modeling command is one undo step; camera commands are not recorded.
    pub fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
        // The undo step of a library insert reads "insert" wherever it came from.
        let label = match command {
            Command::InsertLibraryItem { .. } => "insert".to_string(),
            _ => command.name(),
        };
        self.execute_as(command, label)
    }

    /// Runs one command as `execute` does, its undo step (if it makes one) labelled `label`
    /// instead of by the command's name: the window's Cut is an `erase` labelled `cut`.
    pub fn execute_as(
        &mut self,
        command: Command,
        label: impl Into<String>,
    ) -> Result<Outcome, CommandError> {
        self.refuse_if_read_only(&command)?;
        let units_before = self.units;
        if let Some(changes) = self.edit(&command) {
            let changes = changes?;
            let units = self.units_change(units_before);
            let mut outcome = self.record(label.into(), changes, units);
            outcome.import = self.import_report.take();
            outcome.soften = self.soften_report.take();
            outcome.intersect = self.intersect_report.take();
            outcome.positioned = self.positioned.take();
            outcome.purged = self.purged.take();
            outcome.text_3d = self.text_report.take();
            outcome.material = self.library_material.take();
            return Ok(outcome);
        }
        match command {
            Command::Undo { count, expect } => {
                let n = step_count(count)?;
                if self.undo.len() < n {
                    return Err(CommandError::new(
                        ErrorKind::NothingToUndo,
                        too_few("undo", self.undo.len()),
                    ));
                }
                if let (Some(expect), Some(top)) = (&expect, self.undo.last())
                    && (expect.label != top.label || expect.origin.is_some_and(|o| o != top.origin))
                {
                    return Err(CommandError::new(
                        ErrorKind::UnexpectedStep,
                        format!(
                            "the next step to undo is \"{}\" (by the {}), not \"{}\"",
                            top.label,
                            top.origin.as_str(),
                            expect.label
                        ),
                    ));
                }
                let mut done = Vec::new();
                let mut units = None;
                for _ in 0..n {
                    let step = self.undo.pop().expect("counted");
                    let inverse = step.changes.inverse();
                    self.scene.apply(&inverse);
                    let restored = step.units.map(|(from, _)| from);
                    if let Some(from) = restored {
                        self.units = from;
                        units = restored;
                    }
                    self.redo.push(step);
                    self.changed(&inverse, restored);
                    done.push(inverse);
                }
                Ok(Outcome {
                    changes: SceneChange::join(done).summary(),
                    units,
                    ..Outcome::default()
                })
            }
            Command::Redo { count } => {
                let n = step_count(count)?;
                if self.redo.len() < n {
                    return Err(CommandError::new(
                        ErrorKind::NothingToRedo,
                        too_few("redo", self.redo.len()),
                    ));
                }
                let mut done = Vec::new();
                let mut units = None;
                for _ in 0..n {
                    let step = self.redo.pop().expect("counted");
                    self.scene.apply(&step.changes);
                    let restored = step.units.map(|(_, to)| to);
                    if let Some(to) = restored {
                        self.units = to;
                        units = restored;
                    }
                    self.changed(&step.changes, restored);
                    done.push(step.changes.clone());
                    self.undo.push(step);
                }
                Ok(Outcome {
                    changes: SceneChange::join(done).summary(),
                    units,
                    ..Outcome::default()
                })
            }
            Command::SetCamera { .. }
            | Command::Orbit { .. }
            | Command::Pan { .. }
            | Command::Zoom { .. }
            | Command::ZoomExtents {}
            | Command::StandardView { .. }
            | Command::SetProjection { .. } => {
                self.camera = self.moved_camera(&command)?;
                Ok(Outcome::default())
            }
            Command::SetLighting { mode } => {
                self.lighting = mode;
                Ok(Outcome::default())
            }
            Command::SetColorByTag { on } => {
                self.color_by_tag = on;
                Ok(Outcome::default())
            }
            Command::SetSectionDisplay { planes, cuts, fill } => {
                self.set_section_display_changes(planes, cuts, fill);
                Ok(Outcome::default())
            }
            Command::OpenInstance { id } => {
                self.open_instance(id)?;
                Ok(Outcome::default())
            }
            Command::CloseInstance {} => {
                self.close_instance()?;
                Ok(Outcome::default())
            }
            Command::OpenPath { path } => {
                self.open_path(&path)?;
                Ok(Outcome::default())
            }
            Command::New { discard } => {
                self.check_unsaved(discard)?;
                self.new_model();
                Ok(Outcome::default())
            }
            Command::Open { path, discard } => {
                self.check_unsaved(discard)?;
                let import = self.open_file(&path)?;
                Ok(Outcome {
                    import,
                    ..Outcome::default()
                })
            }
            Command::Save { path, overwrite } => {
                self.save_file(path.as_deref(), overwrite)?;
                Ok(Outcome::default())
            }
            Command::SaveToLibrary {
                path,
                kind,
                item,
                name,
                overwrite,
            } => {
                self.save_to_library(&path, kind, item, name.as_deref(), overwrite)?;
                Ok(Outcome::default())
            }
            Command::Export {
                path,
                format,
                overwrite,
                refuse_links,
                scale,
            } => {
                let report = self.export_file(&path, format, overwrite, refuse_links, scale)?;
                Ok(Outcome {
                    export: Some(report),
                    ..Outcome::default()
                })
            }
            Command::Select { ids, mode, all } => {
                let ids: BTreeSet<EntityId> = if all {
                    if !ids.is_empty() {
                        return Err(CommandError::new(
                            ErrorKind::InvalidInput,
                            "select takes ids or all, not both",
                        ));
                    }
                    // What hidden tags hide is left out, as in SketchUp; so are section planes while
                    // they are not shown.
                    let m = self.model();
                    let shown = |tag| self.scene.tag_shown(tag);
                    m.edges()
                        .filter(|(_, e)| shown(e.tag))
                        .map(|(id, _)| EntityId::Edge(id))
                        .chain(
                            m.faces()
                                .filter(|(_, f)| shown(f.tag))
                                .map(|(id, _)| EntityId::Face(id)),
                        )
                        .chain(
                            m.instances()
                                .filter(|(_, i)| shown(i.tag))
                                .map(|(id, _)| EntityId::Instance(id)),
                        )
                        .chain(
                            m.guides()
                                .filter(|(_, g)| shown(g.tag()))
                                .map(|(id, _)| EntityId::Guide(id)),
                        )
                        .chain(
                            m.dimensions()
                                .filter(|(_, a)| shown(a.tag()))
                                .map(|(id, _)| EntityId::Dimension(id)),
                        )
                        .chain(
                            m.notes()
                                .filter(|(_, n)| shown(n.tag()))
                                .map(|(id, _)| EntityId::Note(id)),
                        )
                        .chain(
                            m.sections()
                                .filter(|(_, p)| self.section_display.planes && shown(p.tag()))
                                .map(|(id, _)| EntityId::Section(id)),
                        )
                        .collect()
                } else {
                    self.parts(&ids)?;
                    ids.into_iter().collect()
                };
                match mode {
                    SelectMode::Replace => self.selection = ids,
                    SelectMode::Add => self.selection.extend(ids),
                    SelectMode::Remove => self.selection.retain(|id| !ids.contains(id)),
                    SelectMode::Toggle => {
                        self.selection =
                            self.selection.symmetric_difference(&ids).copied().collect();
                    }
                }
                Ok(Outcome::default())
            }
            Command::AddEdge { .. }
            | Command::AddRectangle { .. }
            | Command::AddCircle { .. }
            | Command::AddPolygon { .. }
            | Command::AddArc { .. }
            | Command::AddGuideLine { .. }
            | Command::AddGuidePoint { .. }
            | Command::DeleteGuides {}
            | Command::AddDimension { .. }
            | Command::AddEdgeDimension { .. }
            | Command::AddRadialDimension { .. }
            | Command::AddNote { .. }
            | Command::AddScreenNote { .. }
            | Command::ChangeNote { .. }
            | Command::AddSectionPlane { .. }
            | Command::FlipSectionPlane { .. }
            | Command::ActivateSectionPlane { .. }
            | Command::DeactivateSection {}
            | Command::Add3dText { .. }
            | Command::SetUnits { .. }
            | Command::PushPull { .. }
            | Command::Erase { .. }
            | Command::Move { .. }
            | Command::Copy { .. }
            | Command::Rotate { .. }
            | Command::Scale { .. }
            | Command::Offset { .. }
            | Command::FollowMe { .. }
            | Command::MakeGroup { .. }
            | Command::MakeComponent { .. }
            | Command::Explode { .. }
            | Command::RenameDefinition { .. }
            | Command::PurgeDefinitions {}
            | Command::AddMaterial { .. }
            | Command::ChangeMaterial { .. }
            | Command::DeleteMaterial { .. }
            | Command::Paint { .. }
            | Command::AddTag { .. }
            | Command::ChangeTag { .. }
            | Command::DeleteTag { .. }
            | Command::SetTag { .. }
            | Command::AddScene { .. }
            | Command::UpdateScene { .. }
            | Command::ChangeScene { .. }
            | Command::MoveScene { .. }
            | Command::DeleteScene { .. }
            | Command::GoToScene { .. }
            | Command::SoftenEdges { .. }
            | Command::SoftenByAngle { .. }
            | Command::IntersectFaces { .. }
            | Command::PositionTexture { .. }
            | Command::Import { .. }
            | Command::Paste { .. }
            | Command::InsertLibraryItem { .. } => {
                unreachable!("edits are handled by Document::edit")
            }
        }
    }

    /// Applies an editing command to the model and returns its changes, not yet recorded in
    /// history. None for commands that are not edits (undo, redo, camera, selection, context).
    /// Every editing command goes through here, so `execute` and atomic batches stay in step.
    fn edit(&mut self, command: &Command) -> Option<Result<SceneChange, CommandError>> {
        Some(match command {
            Command::AddEdge { start, end } => self.add_edge_changes(*start, *end),
            Command::AddRectangle {
                corner,
                opposite,
                normal,
            } => self.add_rectangle_changes(*corner, *opposite, *normal),
            Command::AddCircle {
                center,
                radius,
                normal,
                segments,
            } => self.add_circle_changes(*center, *radius, *normal, *segments),
            Command::AddPolygon {
                center,
                radius,
                sides,
                normal,
                circumscribed,
                direction,
            } => self.add_polygon_changes(
                *center,
                *radius,
                *sides,
                *normal,
                *circumscribed,
                *direction,
            ),
            Command::AddArc {
                center,
                start,
                angle,
                end,
                through,
                bulge,
                normal,
                segments,
            } => {
                let points: Vec<[f64; 3]> = [Some(*start), *center, *end, *through]
                    .into_iter()
                    .flatten()
                    .collect();
                check_coordinates(&points).and_then(|()| {
                    let (center, start, angle, normal) =
                        arc_spec(*center, *start, *angle, *end, *through, *bulge, *normal)?;
                    self.add_arc_changes(center, start, angle, normal, *segments)
                })
            }
            Command::AddGuideLine { point, direction } => {
                self.add_guide_line_changes(*point, *direction)
            }
            Command::AddGuidePoint { position } => check_coordinates(&[*position]).and_then(|()| {
                self.add_guide(Guide::Point {
                    position: DVec3::from(*position),
                    tag: None,
                })
            }),
            Command::DeleteGuides {} => {
                let all: Vec<GuideId> = self.model().guides().map(|(id, _)| id).collect();
                Ok(self
                    .edit_mesh(|m| Ok(m.erase_guides(&all)))
                    .expect("erasing guides cannot fail"))
            }
            Command::AddDimension { a, b, offset } => self.add_dimension_changes(*a, *b, *offset),
            Command::AddEdgeDimension { edge, offset } => {
                self.add_edge_dimension_changes(*edge, *offset)
            }
            Command::AddRadialDimension {
                edge,
                diameter,
                leader,
            } => self.add_radial_dimension_changes(*edge, *diameter, *leader),
            Command::AddNote {
                anchor,
                position,
                text,
            } => self.add_note_changes(*anchor, *position, text.clone()),
            Command::AddScreenNote { at, text } => self.add_screen_note_changes(*at, text),
            Command::ChangeNote { note, text } => self.change_note_changes(*note, text.clone()),
            Command::AddSectionPlane {
                point,
                normal,
                activate,
            } => self.add_section_plane_changes(*point, *normal, activate.unwrap_or(true)),
            Command::FlipSectionPlane { plane } => self.flip_section_plane_changes(*plane),
            Command::ActivateSectionPlane { plane } => self.activate_section_plane_changes(*plane),
            Command::DeactivateSection {} => Ok(self.deactivate_section_changes()),
            Command::Add3dText {
                text,
                position,
                height,
                depth,
                filled,
            } => self.add_3d_text_changes(text, *position, *height, *depth, *filled),
            Command::SetUnits { length, precision } => self.set_units_changes(*length, *precision),
            Command::PushPull { face, distance } => self.push_pull_changes(*face, *distance),
            Command::Erase { ids } => self.erase_changes(ids),
            Command::Move { ids, offset } => self.move_changes(ids, *offset),
            Command::Copy { ids, offset, count } => {
                self.copy_changes(ids, *offset, count.unwrap_or(1))
            }
            Command::Rotate {
                ids,
                center,
                axis,
                angle,
                copies,
            } => self.rotate_changes(ids, *center, *axis, *angle, *copies),
            Command::Scale {
                ids,
                origin,
                factors,
            } => self.scale_changes(ids, *origin, *factors),
            Command::Offset {
                ids,
                distance,
                toward,
            } => self.offset_changes(ids, *distance, *toward),
            Command::FollowMe { face, path } => self.follow_me_changes(*face, path),
            Command::MakeGroup { ids } => self.group_changes(ids, None),
            Command::MakeComponent { ids, name } => self.group_changes(ids, Some(name)),
            Command::Explode { ids } => self.explode_changes(ids),
            Command::RenameDefinition { definition, name } => {
                self.rename_changes(*definition, name)
            }
            Command::PurgeDefinitions {} => {
                let change = self.scene.purge();
                // Each step is one RemoveDefinition.
                self.purged = Some(change.steps.len());
                Ok(change)
            }
            Command::AddMaterial {
                name,
                color,
                opacity,
                texture,
                emission,
            } => self.add_material_changes(
                name.as_deref(),
                *color,
                *opacity,
                texture.as_ref(),
                *emission,
            ),
            Command::ChangeMaterial {
                material,
                name,
                color,
                opacity,
                texture,
                emission,
            } => self.change_material_changes(
                *material,
                MaterialEdit {
                    name: name.as_deref(),
                    color: *color,
                    opacity: *opacity,
                    texture: None,
                    emission: *emission,
                },
                texture.as_ref().map(Option::as_ref),
            ),
            Command::DeleteMaterial { material } => self.delete_material_changes(*material),
            Command::Paint {
                ids,
                material,
                side,
            } => self.paint_changes(ids, *material, *side),
            Command::AddTag { name, color } => self.add_tag_changes(name.as_deref(), *color),
            Command::ChangeTag {
                tag,
                name,
                color,
                visible,
            } => self.change_tag_changes(
                *tag,
                TagEdit {
                    name: name.as_deref(),
                    color: *color,
                    visible: *visible,
                },
            ),
            Command::DeleteTag { tag, move_to } => self.delete_tag_changes(*tag, *move_to),
            Command::SetTag { ids, tag } => self.set_tag_changes(ids, *tag),
            Command::AddScene { name, camera, tags } => {
                self.add_scene_changes(name.as_deref(), *camera, *tags)
            }
            Command::UpdateScene {
                scene,
                camera,
                tags,
            } => self.update_scene_changes(*scene, *camera, *tags),
            Command::ChangeScene {
                scene,
                name,
                camera,
                tags,
            } => self.change_scene_changes(*scene, name.as_deref(), *camera, *tags),
            Command::MoveScene { scene, index } => self.move_scene_changes(*scene, *index),
            Command::DeleteScene { scene } => self.delete_scene_changes(*scene),
            // Its view change is made here too, outside the returned change.
            Command::GoToScene { scene } => self.go_to_scene_changes(*scene),
            Command::SoftenEdges { ids, soft } => self.soften_changes(ids, *soft),
            Command::SoftenByAngle { ids, max_angle_deg } => {
                self.soften_by_angle_changes(ids, *max_angle_deg)
            }
            Command::IntersectFaces { ids, with } => self.intersect_faces_changes(ids, *with),
            Command::PositionTexture {
                ids,
                offset,
                rotation_deg,
                rotate_by,
                scale,
                reset,
            } => self.position_texture_changes(
                ids,
                PlacementEdit {
                    offset: *offset,
                    rotation_deg: *rotation_deg,
                    rotate_by: *rotate_by,
                    scale: *scale,
                    reset: *reset,
                },
            ),
            Command::Import {
                path,
                format,
                place,
                scale,
                up,
                merge_coplanar,
                library_roots,
            } => {
                let source = self
                    .staged_import
                    .take()
                    .map_or(import::Source::Path, import::Source::Bytes);
                self.import_changes(
                    path,
                    &ImportOptions {
                        format: *format,
                        place: *place,
                        scale: *scale,
                        up: *up,
                        merge_coplanar: *merge_coplanar,
                        library_roots: library_roots.clone(),
                    },
                    source,
                )
            }
            Command::Paste { clip, at } => self.paste_changes(clip, *at),
            Command::InsertLibraryItem { path, at } => self.insert_library_changes(path, *at),
            Command::New { .. }
            | Command::Open { .. }
            | Command::Save { .. }
            | Command::SaveToLibrary { .. }
            | Command::Export { .. } => return None,
            Command::Undo { .. }
            | Command::Redo { .. }
            | Command::SetCamera { .. }
            | Command::Orbit { .. }
            | Command::Pan { .. }
            | Command::Zoom { .. }
            | Command::ZoomExtents {}
            | Command::StandardView { .. }
            | Command::SetProjection { .. }
            | Command::SetLighting { .. }
            | Command::SetColorByTag { .. }
            | Command::SetSectionDisplay { .. }
            | Command::Select { .. }
            | Command::OpenInstance { .. }
            | Command::CloseInstance {}
            | Command::OpenPath { .. } => {
                return None;
            }
        })
    }

    /// Runs commands in order, stopping at the first error.
    ///
    /// A failed atomic batch still consumes entity ids: ids are never reused.
    ///
    /// Non-atomic: each command is its own undo step and earlier commands stay applied.
    /// Atomic: all modeling commands become one undo step, and on error everything (model,
    /// camera, selection and open context) is rolled back. Undo and redo are not allowed inside
    /// an atomic batch.
    ///
    /// A command's references to ids earlier commands made are resolved at its turn; one that
    /// cannot be resolved is that command's error.
    pub fn execute_batch<C: Into<BatchCommand>>(
        &mut self,
        commands: Vec<C>,
        atomic: bool,
    ) -> Result<Vec<Outcome>, BatchError> {
        let commands: Vec<BatchCommand> = commands.into_iter().map(Into::into).collect();
        let mut outcomes = Vec::new();
        for (index, command) in commands.iter().enumerate() {
            if let Err(error) = self.refuse_if_read_only(command.command()) {
                return Err(BatchError {
                    index,
                    error,
                    completed: Vec::new(),
                    rolled_back: true,
                });
            }
        }
        if !atomic {
            for (index, command) in commands.iter().enumerate() {
                match command
                    .resolve(&outcomes)
                    .and_then(|command| self.execute(command))
                {
                    Ok(o) => outcomes.push(o),
                    Err(error) => {
                        return Err(BatchError {
                            index,
                            error,
                            completed: outcomes,
                            rolled_back: false,
                        });
                    }
                }
            }
            return Ok(outcomes);
        }

        let camera_before = self.camera.clone();
        let lighting_before = self.lighting;
        let color_by_tag_before = self.color_by_tag;
        let section_display_before = self.section_display;
        let selected_scene_before = self.selected_scene;
        let selection_before = self.selection.clone();
        let context_before = self.context.clone();
        let units_before = self.units;
        let mut applied: Vec<SceneChange> = Vec::new();
        // The edits' names: the step is labeled by theirs when they share one.
        let mut names: Vec<String> = Vec::new();
        for (index, command) in commands.iter().enumerate() {
            let units_then = self.units;
            let result = match command.resolve(&outcomes) {
                Err(error) => Err(error),
                Ok(command) => match self.edit(&command) {
                    Some(changes) => changes.map(|c| {
                        let summary = c.summary();
                        applied.push(c);
                        names.push(command.name());
                        Outcome {
                            changes: summary,
                            import: self.import_report.take(),
                            export: None,
                            soften: self.soften_report.take(),
                            intersect: self.intersect_report.take(),
                            positioned: self.positioned.take(),
                            purged: self.purged.take(),
                            text_3d: self.text_report.take(),
                            material: self.library_material.take(),
                            units: (self.units != units_then).then_some(self.units),
                        }
                    }),
                    None if matches!(
                        command,
                        Command::Undo { .. }
                            | Command::Redo { .. }
                            | Command::New { .. }
                            | Command::Open { .. }
                            | Command::Save { .. }
                            | Command::SaveToLibrary { .. }
                            | Command::Export { .. }
                    ) =>
                    {
                        Err(CommandError::new(
                            ErrorKind::InvalidInput,
                            "undo, redo, new, open, save and export are not allowed in an atomic batch",
                        ))
                    }
                    None => self.execute(command),
                },
            };
            match result {
                Ok(o) => outcomes.push(o),
                Err(error) => {
                    for c in applied.iter().rev() {
                        self.scene.apply(&c.inverse());
                    }
                    self.camera = camera_before;
                    self.lighting = lighting_before;
                    self.color_by_tag = color_by_tag_before;
                    self.section_display = section_display_before;
                    self.selected_scene = selected_scene_before;
                    self.selection = selection_before;
                    self.context = context_before;
                    self.units = units_before;
                    return Err(BatchError {
                        index,
                        error,
                        completed: outcomes,
                        rolled_back: true,
                    });
                }
            }
        }
        let label = match names.first() {
            Some(first) if names.iter().all(|n| n == first) => first.clone(),
            _ => "batch".to_string(),
        };
        let units = self.units_change(units_before);
        self.record(label, SceneChange::join(applied), units);
        Ok(outcomes)
    }

    fn add_edge_changes(
        &mut self,
        start: [f64; 3],
        end: [f64; 3],
    ) -> Result<SceneChange, CommandError> {
        check_coordinates(&[start, end])?;
        self.edit_mesh(|m| m.add_edge(DVec3::from(start), DVec3::from(end)))
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
    }

    fn add_guide_line_changes(
        &mut self,
        point: [f64; 3],
        direction: [f64; 3],
    ) -> Result<SceneChange, CommandError> {
        check_coordinates(&[point])?;
        // The direction is normalized, so only its finiteness and length matter.
        let Some(direction) = unit_vector(DVec3::from(direction)) else {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "direction must be a finite, non-zero vector",
            ));
        };
        self.add_guide(Guide::Line {
            point: DVec3::from(point),
            direction,
            tag: None,
        })
    }

    fn add_guide(&mut self, guide: Guide) -> Result<SceneChange, CommandError> {
        self.edit_mesh(|m| m.add_guide(guide))
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
    }

    fn add_circle_changes(
        &mut self,
        center: [f64; 3],
        radius: f64,
        normal: Option<[f64; 3]>,
        segments: Option<u32>,
    ) -> Result<SceneChange, CommandError> {
        check_coordinates(&[center])?;
        let points = circle_points(
            DVec3::from(center),
            radius,
            normal.map(DVec3::from),
            segments.unwrap_or(DEFAULT_CIRCLE_SEGMENTS),
        )?;
        check_coordinates(&points.iter().map(|p| p.to_array()).collect::<Vec<_>>())?;
        self.edit_mesh(|m| m.add_curve(&points, true, false))
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
    }

    fn add_polygon_changes(
        &mut self,
        center: [f64; 3],
        radius: f64,
        sides: Option<u32>,
        normal: Option<[f64; 3]>,
        circumscribed: bool,
        direction: Option<[f64; 3]>,
    ) -> Result<SceneChange, CommandError> {
        check_coordinates(&[center])?;
        let points = polygon_points(
            DVec3::from(center),
            radius,
            sides.unwrap_or(DEFAULT_POLYGON_SIDES),
            normal.map(DVec3::from),
            circumscribed,
            direction.map(DVec3::from),
        )?;
        check_coordinates(&points.iter().map(|p| p.to_array()).collect::<Vec<_>>())?;
        self.edit_mesh(|m| m.add_curve(&points, true, true))
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
    }

    fn add_arc_changes(
        &mut self,
        center: DVec3,
        start: DVec3,
        angle: f64,
        normal: Option<DVec3>,
        segments: Option<u32>,
    ) -> Result<SceneChange, CommandError> {
        let points = arc_points(
            center,
            start,
            angle,
            normal,
            segments.unwrap_or(DEFAULT_ARC_SEGMENTS),
        )?;
        check_coordinates(&points.iter().map(|p| p.to_array()).collect::<Vec<_>>())?;
        self.edit_mesh(|m| m.add_curve(&points, false, false))
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
    }

    fn add_rectangle_changes(
        &mut self,
        corner: [f64; 3],
        opposite: [f64; 3],
        normal: Option<[f64; 3]>,
    ) -> Result<SceneChange, CommandError> {
        check_coordinates(&[corner, opposite])?;
        let corners = rectangle_corners(
            DVec3::from(corner),
            DVec3::from(opposite),
            normal.map(DVec3::from),
        )?;
        check_coordinates(&corners.map(|c| c.to_array()))?;
        self.edit_mesh(|m| m.add_polygon(&corners))
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
    }

    fn push_pull_changes(
        &mut self,
        face: EntityId,
        distance: f64,
    ) -> Result<SceneChange, CommandError> {
        let EntityId::Face(id) = face else {
            return Err(
                CommandError::new(ErrorKind::InvalidInput, "push_pull needs a face id")
                    .with_ids(vec![face]),
            );
        };
        let Some(f) = self.model().face(id) else {
            return Err(
                CommandError::new(ErrorKind::NotFound, format!("no entity {face}"))
                    .with_ids(vec![face]),
            );
        };
        if !distance.is_finite() || distance.abs() > MAX_COORDINATE {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("distance must be finite and within +/-{MAX_COORDINATE} m"),
            ));
        }
        let moved: Vec<[f64; 3]> = std::iter::once(&f.outer)
            .chain(&f.holes)
            .flatten()
            .map(|v| (self.model().position(*v).unwrap() + f.normal * distance).to_array())
            .collect();
        check_coordinates(&moved)?;
        self.edit_mesh(|m| m.push_pull(id, distance)).map_err(|e| {
            CommandError::new(ErrorKind::InvalidInput, e.to_string()).with_ids(vec![face])
        })
    }

    fn move_changes(
        &mut self,
        ids: &[EntityId],
        offset: [f64; 3],
    ) -> Result<SceneChange, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one vertex, edge, face, instance, guide, dimension, note or section plane",
            ));
        }
        let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
        if !missing.is_empty() {
            return Err(
                CommandError::new(ErrorKind::NotFound, "no such entities").with_ids(missing)
            );
        }
        check_coordinates(&[offset])?;
        let vertices = self.vertex_ids(ids);
        let parts = Parts::of(ids);
        let offset = DVec3::from(offset);
        let shift = DAffine3::from_translation(offset);
        self.check_placed(&vertices, &parts, &[shift], false)?;
        self.edit_mesh(|m| {
            // Held, so the dimensions that move too are judged once everything has moved.
            m.hold_dimensions(&parts.dimensions);
            let moved = m.move_vertices(&vertices, offset);
            let mut c = match moved {
                Ok(c) => c,
                Err(e) => {
                    m.hold_dimensions(&[]);
                    return Err(e);
                }
            };
            c.ops.extend(m.move_instances(&parts.instances, offset).ops);
            c.ops.extend(m.transform_guides(&parts.guides, shift).ops);
            c.ops.extend(
                m.transform_annotations(&parts.dimensions, &parts.notes, shift)
                    .ops,
            );
            c.ops
                .extend(m.transform_sections(&parts.sections, shift).ops);
            Ok(c)
        })
        .map_err(|e| self.kernel_error(e, ids))
    }

    fn rotate_changes(
        &mut self,
        ids: &[EntityId],
        center: [f64; 3],
        axis: Option<[f64; 3]>,
        angle: f64,
        copies: Option<u32>,
    ) -> Result<SceneChange, CommandError> {
        let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
        if ids.is_empty() {
            return Err(invalid(
                "ids must list at least one vertex, edge, face, instance, guide, dimension, note or section plane",
            ));
        }
        check_coordinates(&[center])?;
        let Some(axis) = unit_vector(DVec3::from(axis.unwrap_or([0.0, 0.0, 1.0]))) else {
            return Err(invalid("axis must be a finite, non-zero vector"));
        };
        if !angle.is_finite() {
            return Err(invalid("angle must be finite"));
        }
        // Whole turns are dropped first, so copies k * angle stay finite and exact.
        let angle = angle % 360.0;
        if angle.abs() < 1e-9 || 360.0 - angle.abs() < 1e-9 {
            return Err(invalid("angle must not be a whole number of turns"));
        }
        let center = DVec3::from(center);
        if let Some(count) = copies {
            let parts = self.copy_parts(ids, count)?;
            // A copy turned by a whole number of turns would land exactly on the original.
            let whole = |k: u32| {
                let r = (angle * k as f64).rem_euclid(360.0);
                r < 1e-9 || 360.0 - r < 1e-9
            };
            if let Some(k) = (1..=count).find(|k| whole(*k)) {
                return Err(invalid(&format!(
                    "copies would complete a whole turn and land on the original (at most {})",
                    k - 1
                )));
            }
            let places: Vec<DAffine3> = (1..=count)
                .map(|k| rotation(center, axis, angle * k as f64))
                .collect();
            self.check_placed(&self.vertex_ids(ids), &parts, &places, true)?;
            return self.copy_placed_changes(ids, &parts, &places);
        }
        let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
        if !missing.is_empty() {
            return Err(
                CommandError::new(ErrorKind::NotFound, "no such entities").with_ids(missing)
            );
        }
        let vertices = self.vertex_ids(ids);
        let parts = Parts::of(ids);
        let turn = rotation(center, axis, angle);
        self.check_placed(&vertices, &parts, &[turn], false)?;
        self.transform_changes(ids, &vertices, &parts, turn)
    }

    fn scale_changes(
        &mut self,
        ids: &[EntityId],
        origin: [f64; 3],
        factors: [f64; 3],
    ) -> Result<SceneChange, CommandError> {
        let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
        if ids.is_empty() {
            return Err(invalid(
                "ids must list at least one vertex, edge, face, instance, guide, dimension, note or section plane",
            ));
        }
        check_coordinates(&[origin])?;
        if !factors
            .iter()
            .all(|f| f.is_finite() && (1e-4..=1e4).contains(&f.abs()))
        {
            return Err(invalid(
                "scale factors must be finite and between 1e-4 and 1e4 in size",
            ));
        }
        if factors == [1.0; 3] {
            return Err(invalid("nothing to scale: every factor is 1"));
        }
        let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
        if !missing.is_empty() {
            return Err(
                CommandError::new(ErrorKind::NotFound, "no such entities").with_ids(missing)
            );
        }
        let origin = DVec3::from(origin);
        let scale = DAffine3::from_translation(origin)
            * DAffine3::from_scale(DVec3::from(factors))
            * DAffine3::from_translation(-origin);
        let vertices = self.vertex_ids(ids);
        let parts = Parts::of(ids);
        // An instance squeezed this flat could not be opened again once saved.
        let flat: Vec<EntityId> = parts
            .instances
            .iter()
            .filter(|i| {
                self.model().instance(**i).is_some_and(|i| {
                    (scale * i.transform).matrix3.determinant().abs() < kernel::MIN_DETERMINANT
                })
            })
            .map(|i| EntityId::Instance(*i))
            .collect();
        if !flat.is_empty() {
            return Err(
                invalid("scaling would squeeze an instance too flat to keep").with_ids(flat),
            );
        }
        self.check_placed(&vertices, &parts, &[scale], false)?;
        self.transform_changes(ids, &vertices, &parts, scale)
    }

    /// Transforms `vertices` and the instances, guides and annotations of `parts` by `t` as
    /// one change. The geometry goes first, so a radial dimension whose circle the change
    /// bends is frozen to the circle it had and then transformed with the rest.
    fn transform_changes(
        &mut self,
        ids: &[EntityId],
        vertices: &BTreeSet<VertexId>,
        parts: &Parts,
        t: DAffine3,
    ) -> Result<SceneChange, CommandError> {
        self.edit_mesh(|m| {
            // Held, so the dimensions that move too are judged once everything has moved.
            m.hold_dimensions(&parts.dimensions);
            let moved = m.transform_vertices(vertices, t);
            let mut c = match moved {
                Ok(c) => c,
                Err(e) => {
                    m.hold_dimensions(&[]);
                    return Err(e);
                }
            };
            c.ops.extend(m.transform_instances(&parts.instances, t).ops);
            c.ops.extend(m.transform_guides(&parts.guides, t).ops);
            c.ops.extend(
                m.transform_annotations(&parts.dimensions, &parts.notes, t)
                    .ops,
            );
            c.ops.extend(m.transform_sections(&parts.sections, t).ops);
            Ok(c)
        })
        .map_err(|e| self.kernel_error(e, ids))
    }

    fn offset_changes(
        &mut self,
        ids: &[EntityId],
        distance: f64,
        toward: Option<[f64; 3]>,
    ) -> Result<SceneChange, CommandError> {
        let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
        if ids.is_empty() {
            return Err(invalid(
                "ids must list one face, or edges forming one chain",
            ));
        }
        let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
        if !missing.is_empty() {
            return Err(
                CommandError::new(ErrorKind::NotFound, "no such entities").with_ids(missing)
            );
        }
        let least = 10.0 * kernel::geom::LENGTH_TOL;
        if !distance.is_finite() || distance.abs() < least || distance.abs() > MAX_COORDINATE {
            return Err(invalid(&format!(
                "distance must be finite, at least {least} m and at most {MAX_COORDINATE} m in size"
            )));
        }
        if let Some(t) = toward {
            check_coordinates(&[t])?;
        }
        let ids: Vec<EntityId> = ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let lines = match ids[..] {
            [EntityId::Face(face)] => {
                if toward.is_some() {
                    return Err(invalid(
                        "toward is only for edges: for a face the sign of distance picks the side (positive: inward)",
                    ));
                }
                self.face_offset(face, distance)
            }
            _ => {
                let others: Vec<EntityId> = ids
                    .iter()
                    .copied()
                    .filter(|id| !matches!(id, EntityId::Edge(_)))
                    .collect();
                if !others.is_empty() {
                    return Err(
                        invalid("offset takes one face, or edges forming one chain")
                            .with_ids(others),
                    );
                }
                if distance < 0.0 {
                    return Err(invalid(
                        "distance must be positive for edges: toward picks the side",
                    ));
                }
                let Some(toward) = toward else {
                    return Err(invalid("toward is required for edges: it picks the side"));
                };
                let edges: Vec<EdgeId> = ids
                    .iter()
                    .filter_map(|id| match id {
                        EntityId::Edge(e) => Some(*e),
                        _ => None,
                    })
                    .collect();
                self.chain_offset(&edges, distance, DVec3::from(toward))
            }
        }
        .map_err(|e| e.with_ids(ids.clone()))?;
        let points: Vec<[f64; 3]> = lines
            .iter()
            .flat_map(|l| l.points.iter().map(|p| p.to_array()))
            .collect();
        check_coordinates(&points)?;
        self.edit_mesh(|m| m.add_polylines(&lines))
            .map_err(|e| self.kernel_error(e, &ids))
    }

    /// The new loops of a face offset by `distance` (positive: inward).
    fn face_offset(&self, face: FaceId, distance: f64) -> Result<Vec<Polyline>, CommandError> {
        let m = self.model();
        let f = m.face(face).expect("checked");
        let loops: Vec<&Vec<VertexId>> = std::iter::once(&f.outer).chain(&f.holes).collect();
        let rings: Vec<Vec<DVec3>> = loops
            .iter()
            .map(|l| l.iter().map(|v| m.position(*v).unwrap()).collect())
            .collect();
        // Outward only the outer loop is offset; the holes stay as they are.
        let new = offset::offset_face(&rings, f.normal, distance).map_err(|_| too_far())?;
        Ok(loops
            .iter()
            .zip(new)
            .map(|(l, points)| {
                let n = l.len();
                let edges: Vec<EdgeId> = (0..n)
                    .filter_map(|i| m.edge_between(l[i], l[(i + 1) % n]))
                    .collect();
                Polyline {
                    points,
                    closed: true,
                    curve: one_curve(m, &edges),
                }
            })
            .collect())
    }

    /// The new chain of edges offset by `distance` to the side of `toward`.
    fn chain_offset(
        &self,
        edges: &[EdgeId],
        distance: f64,
        toward: DVec3,
    ) -> Result<Vec<Polyline>, CommandError> {
        let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
        let m = self.model();
        let Some((vertices, closed)) = m.edge_chain(edges) else {
            return Err(invalid(
                "the edges must form one connected chain without branches",
            ));
        };
        let points: Vec<DVec3> = vertices.iter().map(|v| m.position(*v).unwrap()).collect();
        let tol = kernel::geom::LENGTH_TOL;
        let normal = match offset::plane_normal(&points) {
            Some(n) => {
                if points.iter().any(|p| (*p - points[0]).dot(n).abs() >= tol) {
                    return Err(invalid("the edges must lie in one plane"));
                }
                if (toward - points[0]).dot(n).abs() >= tol {
                    return Err(invalid("toward must lie in the plane of the edges"));
                }
                n
            }
            // A straight chain: its plane is the one through toward.
            None => (points[1] - points[0])
                .cross(toward - points[0])
                .try_normalize()
                .ok_or_else(|| invalid("toward must lie to one side of the edges"))?,
        };
        let Some(side) = offset::side_of(&points, closed, normal, toward) else {
            return Err(invalid("toward must lie to one side of the edges"));
        };
        let curve = one_curve(m, edges);
        // An arc's ends move along its radii, so its offset is an arc too.
        let center = curve
            .and_then(|_| m.curve_of(edges[0])?.circle)
            .map(|c| c.center);
        let new = match (closed, center) {
            (true, _) => offset::offset_loop(&points, normal, side * distance),
            (false, Some(c)) => offset::offset_arc(&points, normal, side * distance, c),
            (false, None) => offset::offset_chain(&points, normal, side * distance),
        }
        .map_err(|_| too_far())?;
        Ok(vec![Polyline {
            points: new,
            closed,
            curve,
        }])
    }

    fn copy_changes(
        &mut self,
        ids: &[EntityId],
        offset: [f64; 3],
        count: u32,
    ) -> Result<SceneChange, CommandError> {
        let parts = self.copy_parts(ids, count)?;
        check_coordinates(&[offset])?;
        let offset = DVec3::from(offset);
        let places: Vec<DAffine3> = (1..=count)
            .map(|k| DAffine3::from_translation(offset * k as f64))
            .collect();
        self.check_placed(&self.vertex_ids(ids), &parts, &places, true)?;
        if offset.length() < kernel::geom::LENGTH_TOL {
            return Err(self.kernel_error(KernelError::ZeroDistance, ids));
        }
        self.copy_placed_changes(ids, &parts, &places)
    }

    /// Checks the ids and count of a copy and splits the ids into edges, faces and instances.
    fn copy_parts(&self, ids: &[EntityId], count: u32) -> Result<Parts, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one edge, face, instance, guide, dimension, note or section plane",
            ));
        }
        if !(1..=MAX_COPIES).contains(&count) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("count must be between 1 and {MAX_COPIES}"),
            ));
        }
        self.parts(ids)
    }

    /// Copies `parts` (split from `ids`) once per rigid motion in `places`.
    fn copy_placed_changes(
        &mut self,
        ids: &[EntityId],
        parts: &Parts,
        places: &[DAffine3],
    ) -> Result<SceneChange, CommandError> {
        // Worked out before the copy, which may split the copied edges.
        let copied = self.model().vertices_of(&parts.edges, &parts.faces);
        let geometry = if parts.edges.is_empty() && parts.faces.is_empty() {
            SceneChange::default()
        } else {
            self.edit_mesh(|m| m.copy_placed(&parts.edges, &parts.faces, places))
                .map_err(|e| self.kernel_error(e, ids))?
        };
        let placed = if parts.instances.is_empty() {
            SceneChange::default()
        } else {
            let mesh = self.active();
            match self
                .scene
                .copy_instances_placed(mesh, &parts.instances, places)
            {
                Ok(c) => c,
                Err(e) => {
                    self.scene.apply(&geometry.inverse());
                    return Err(self.kernel_error(e, ids));
                }
            }
        };
        let guides = self
            .edit_mesh(|m| {
                let mut c = m.copy_guides(&parts.guides, places);
                c.ops.extend(
                    m.copy_annotations(&parts.dimensions, &parts.notes, places, &copied)
                        .ops,
                );
                c.ops.extend(m.copy_sections(&parts.sections, places).ops);
                Ok(c)
            })
            .expect("copying guides, annotations and planes cannot fail");
        Ok(SceneChange::join([geometry, placed, guides]))
    }

    /// Rejects moves, rotations or copies that would put a vertex, an instance's bounding
    /// box (its origin when it has nothing in it), a guide's point or an annotation's points
    /// (see `annotation_points_placed`) beyond `MAX_COORDINATE` under any of `places`.
    /// `copies`: the annotations are copied, so every anchor of theirs is placed.
    fn check_placed(
        &self,
        vertices: &BTreeSet<VertexId>,
        parts: &Parts,
        places: &[DAffine3],
        copies: bool,
    ) -> Result<(), CommandError> {
        let m = self.model();
        let mut moved: Vec<[f64; 3]> = Vec::new();
        for t in places {
            let at = |p: DVec3| t.transform_point3(p).to_array();
            moved.extend(vertices.iter().map(|v| at(m.position(*v).unwrap())));
            moved.extend(
                parts
                    .guides
                    .iter()
                    .filter_map(|g| m.guide(*g))
                    .map(|g| at(g.anchor())),
            );
            moved.extend(
                parts
                    .sections
                    .iter()
                    .filter_map(|p| m.section(*p))
                    .map(|p| at(p.point)),
            );
            let follows = |v: VertexId| copies || vertices.contains(&v);
            moved.extend(
                self.annotation_points_placed(&parts.dimensions, &parts.notes, *t, &follows)
                    .into_iter()
                    .map(|p| p.to_array()),
            );
            for id in &parts.instances {
                let Some(i) = m.instance(*id) else { continue };
                let placed = Instance::new(i.definition, *t * i.transform);
                let mut points: Vec<[f64; 3]> = match self.scene.instance_bounds(&placed) {
                    Some(b) => b.map(|p| p.to_array()).to_vec(),
                    None => vec![placed.transform.translation.to_array()],
                };
                // Bounds leave guides out, so check the definition's guides too.
                points.extend(
                    self.scene
                        .instance_guide_anchors(&placed)
                        .iter()
                        .map(|p| p.to_array()),
                );
                check_coordinates(&points)
                    .map_err(|e| e.with_ids(vec![EntityId::Instance(*id)]))?;
            }
        }
        check_coordinates(&moved)
    }

    /// The box around an instance of the open context, in the context's coordinates.
    fn instance_box(&self, id: InstanceId) -> Option<[DVec3; 2]> {
        self.scene.instance_bounds(self.model().instance(id)?)
    }

    /// The vertices that make up the entities (vertices themselves, edge ends, face loops).
    fn vertex_ids(&self, ids: &[EntityId]) -> BTreeSet<VertexId> {
        let m = self.model();
        let mut out = BTreeSet::new();
        for id in ids {
            match *id {
                EntityId::Vertex(v) => {
                    out.insert(v);
                }
                // Annotations are anchored to vertices but are not made of them; planes
                // have no vertices.
                EntityId::Instance(_)
                | EntityId::Guide(_)
                | EntityId::Dimension(_)
                | EntityId::Note(_)
                | EntityId::Section(_) => {}
                EntityId::Edge(e) => out.extend(m.edge(e).map(|e| e.v).into_iter().flatten()),
                EntityId::Face(f) => {
                    if let Some(f) = m.face(f) {
                        out.extend(f.outer.iter().chain(f.holes.iter().flatten()));
                    }
                }
            }
        }
        out
    }

    /// A kernel error from moving or copying `ids`, naming the bent face when there is one.
    fn kernel_error(&self, e: KernelError, ids: &[EntityId]) -> CommandError {
        let culprits = match e {
            KernelError::Bend(f) => vec![EntityId::Face(f)],
            _ => ids.to_vec(),
        };
        CommandError::new(ErrorKind::InvalidInput, e.to_string()).with_ids(culprits)
    }

    fn erase_changes(&mut self, ids: &[EntityId]) -> Result<SceneChange, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one edge, face, instance, guide, dimension, note or section plane",
            ));
        }
        let parts = self.parts(ids)?;
        let erased = self
            .edit_mesh(|m| {
                // Annotations first, so erasing their geometry does not freeze them.
                let mut c = m.erase_annotations(&parts.dimensions, &parts.notes);
                c.ops.extend(m.erase(&parts.edges, &parts.faces).ops);
                c.ops.extend(m.erase_guides(&parts.guides).ops);
                c.ops.extend(m.erase_sections(&parts.sections).ops);
                Ok(c)
            })
            .expect("erase cannot fail");
        let mesh = self.active();
        let removed = self.scene.erase_instances(mesh, &parts.instances);
        Ok(SceneChange::join([erased, removed]))
    }

    /// Splits ids (each counted once) into edges, faces, instances, guides, dimensions and
    /// notes of the open context.
    /// Vertex ids are invalid input and unknown ids are not found; both errors list the
    /// offending ids.
    fn parts(&self, ids: &[EntityId]) -> Result<Parts, CommandError> {
        let vertices: Vec<EntityId> = ids
            .iter()
            .copied()
            .filter(|id| matches!(id, EntityId::Vertex(_)))
            .collect();
        if !vertices.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "only edges, faces, instances, guides, dimensions and notes are accepted here, not vertices",
            )
            .with_ids(vertices));
        }
        let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
        if !missing.is_empty() {
            return Err(
                CommandError::new(ErrorKind::NotFound, "no such entities").with_ids(missing)
            );
        }
        Ok(Parts::of(ids))
    }

    /// Whether the open context holds the entity.
    fn exists(&self, id: EntityId) -> bool {
        let m = self.model();
        match id {
            EntityId::Vertex(v) => m.vertex(v).is_some(),
            EntityId::Edge(e) => m.edge(e).is_some(),
            EntityId::Face(f) => m.face(f).is_some(),
            EntityId::Instance(i) => m.instance(i).is_some(),
            EntityId::Guide(g) => m.guide(g).is_some(),
            EntityId::Dimension(d) => m.dimension(d).is_some(),
            EntityId::Note(n) => m.note(n).is_some(),
            EntityId::Section(p) => m.section(p).is_some(),
        }
    }

    /// Records `changes` and the change of display `units` (before, after) as an undo step
    /// labeled `label`, unless there are none.
    fn record(
        &mut self,
        label: String,
        changes: SceneChange,
        units: Option<(Units, Units)>,
    ) -> Outcome {
        if changes.is_empty() && units.is_none() {
            return Outcome::default();
        }
        let outcome = self.changed(&changes, units.map(|(_, after)| after));
        self.undo.push(Step {
            label,
            origin: self.origin,
            changes,
            units,
        });
        self.redo.clear();
        outcome
    }

    /// Bumps the revision, logs the change (with the display `units` it set, if any), closes
    /// open instances that no longer exist and drops entities that are no longer in the open
    /// context from the selection.
    fn changed(&mut self, changes: &SceneChange, units: Option<Units>) -> Outcome {
        self.revision += 1;
        while self.scene.resolve(&self.context).is_none() {
            self.context.pop();
        }
        self.selection = self
            .selection
            .iter()
            .copied()
            .filter(|id| self.exists(*id))
            .collect();
        let summary = changes.summary();
        self.events.push_back(ChangeEvent {
            revision: self.revision,
            changes: summary.clone(),
            units,
        });
        if self.events.len() > EVENT_LOG_LEN {
            self.events.pop_front();
        }
        Outcome {
            changes: summary,
            units,
            ..Outcome::default()
        }
    }

    /// The label of the step `undo` would undo next (the `history` query's first `undo`).
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|s| s.label.as_str())
    }

    /// The label of the step `redo` would redo next.
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }

    pub fn query(&self, query: &Query) -> Result<QueryResult, CommandError> {
        let m = self.model();
        Ok(match query {
            Query::Entities { ids, kinds, within } => {
                let wanted = |id: EntityId, kind: EntityKind| {
                    (ids.is_empty() || ids.contains(&id))
                        && (kinds.is_empty() || kinds.contains(&kind))
                        && within.is_none_or(|b| {
                            let points = self.vertices_of(id);
                            !points.is_empty() && points.into_iter().all(|p| b.contains(p))
                        })
                };
                QueryResult::Entities {
                    vertices: m
                        .vertices()
                        .filter(|(id, _)| wanted(EntityId::Vertex(*id), EntityKind::Vertex))
                        .map(|(id, _)| self.vertex_info(id))
                        .collect(),
                    edges: m
                        .edges()
                        .filter(|(id, _)| wanted(EntityId::Edge(*id), EntityKind::Edge))
                        .map(|(id, _)| self.edge_info(id))
                        .collect(),
                    faces: m
                        .faces()
                        .filter(|(id, _)| wanted(EntityId::Face(*id), EntityKind::Face))
                        .map(|(id, _)| self.face_info(id))
                        .collect(),
                    instances: m
                        .instances()
                        .filter(|(id, _)| wanted(EntityId::Instance(*id), EntityKind::Instance))
                        .map(|(id, _)| self.instance_info(id))
                        .collect(),
                    guides: m
                        .guides()
                        .filter(|(id, _)| wanted(EntityId::Guide(*id), EntityKind::Guide))
                        .map(|(id, _)| self.guide_info(id))
                        .collect(),
                    revision: self.revision,
                }
            }
            Query::Entity { id } => {
                let missing = || {
                    CommandError::new(ErrorKind::NotFound, format!("no entity {id}"))
                        .with_ids(vec![*id])
                };
                match *id {
                    EntityId::Vertex(v) => QueryResult::Vertex(
                        m.vertex(v)
                            .map(|_| self.vertex_info(v))
                            .ok_or_else(missing)?,
                    ),
                    EntityId::Edge(e) => {
                        QueryResult::Edge(m.edge(e).map(|_| self.edge_info(e)).ok_or_else(missing)?)
                    }
                    EntityId::Face(f) => {
                        QueryResult::Face(m.face(f).map(|_| self.face_info(f)).ok_or_else(missing)?)
                    }
                    EntityId::Instance(i) => QueryResult::Instance(
                        m.instance(i)
                            .map(|_| self.instance_info(i))
                            .ok_or_else(missing)?,
                    ),
                    EntityId::Guide(g) => QueryResult::Guide(
                        m.guide(g).map(|_| self.guide_info(g)).ok_or_else(missing)?,
                    ),
                    EntityId::Dimension(r) => QueryResult::Dimension(
                        m.dimension(r)
                            .map(|_| self.dimension_info(r))
                            .ok_or_else(missing)?,
                    ),
                    EntityId::Note(n) => {
                        QueryResult::Note(m.note(n).map(|_| self.note_info(n)).ok_or_else(missing)?)
                    }
                    EntityId::Section(p) => QueryResult::SectionPlane(
                        m.section(p)
                            .map(|_| self.section_plane_info(p))
                            .ok_or_else(missing)?,
                    ),
                }
            }
            Query::Curve { id } => QueryResult::Curve(self.curve_info(*id)?),
            Query::Camera {} => QueryResult::Camera {
                eye: self.camera.eye.to_array(),
                target: self.camera.target.to_array(),
                up: self.camera.up.to_array(),
                fov_y_deg: self.camera.fov_y.to_degrees(),
                projection: self.camera.projection,
                view_height: self.camera.view_height(),
                lighting: self.lighting,
                color_by_tag: self.color_by_tag,
            },
            Query::Stats {} => QueryResult::Stats {
                vertices: m.vertices().count(),
                edges: m.edges().count(),
                faces: m.faces().count(),
                instances: m.instances().count(),
                definitions: self.scene.definitions().count(),
                revision: self.revision,
            },
            Query::History {} => QueryResult::History {
                undo_depth: self.undo.len(),
                redo_depth: self.redo.len(),
                undo: self.undo.iter().rev().map(|s| s.label.clone()).collect(),
                redo: self.redo.iter().rev().map(|s| s.label.clone()).collect(),
                undo_origins: self.undo.iter().rev().map(|s| s.origin).collect(),
                redo_origins: self.redo.iter().rev().map(|s| s.origin).collect(),
                revision: self.revision,
            },
            Query::Selection {} => QueryResult::Selection {
                ids: self.selection.iter().copied().collect(),
                revision: self.revision,
            },
            Query::Definitions {} => QueryResult::Definitions {
                definitions: self
                    .scene
                    .definitions()
                    .map(|(id, d)| DefinitionInfo {
                        id,
                        name: d.name.clone(),
                        kind: d.kind,
                        instances: self.scene.instance_count(id),
                        edges: d.mesh.edges().count(),
                        faces: d.mesh.faces().count(),
                    })
                    .collect(),
                revision: self.revision,
            },
            Query::Materials {} => QueryResult::Materials {
                materials: self.material_infos(),
                revision: self.revision,
            },
            Query::Tags {} => QueryResult::Tags {
                tags: self.tag_infos(),
                revision: self.revision,
            },
            Query::Scenes {} => QueryResult::Scenes {
                scenes: self.scene_infos(),
                revision: self.revision,
            },
            Query::Scene { scene } => self.scene_detail(*scene)?,
            Query::Context {} => QueryResult::Context {
                path: self
                    .context
                    .iter()
                    .map(|i| EntityId::Instance(*i))
                    .collect(),
                definition: match self.active() {
                    MeshId::Root => None,
                    MeshId::Definition(d) => Some(d),
                },
            },
            Query::Outliner { depth } => QueryResult::Outliner {
                instances: self.outliner(*depth),
                revision: self.revision,
            },
            Query::File {} => QueryResult::File {
                path: self.path.as_ref().map(|p| p.display().to_string()),
                modified: self.is_modified(),
                revision: self.revision,
            },
            Query::Events { since } => {
                let (events, truncated) = self.events_since(*since);
                QueryResult::Events {
                    revision: self.revision,
                    events,
                    truncated,
                }
            }
            Query::Annotations {} => QueryResult::Annotations {
                dimensions: m
                    .dimensions()
                    .map(|(id, _)| self.dimension_info(id))
                    .collect(),
                notes: m.notes().map(|(id, _)| self.note_info(id)).collect(),
                revision: self.revision,
            },
            Query::Units {} => QueryResult::Units(self.units),
            Query::Library { folder } => {
                QueryResult::Library(crate::library::list(std::path::Path::new(folder))?)
            }
            Query::SectionPlanes {} => QueryResult::SectionPlanes {
                planes: m
                    .sections()
                    .map(|(id, _)| self.section_plane_info(id))
                    .collect(),
                revision: self.revision,
            },
            Query::SectionDisplay {} => QueryResult::SectionDisplay {
                planes: self.section_display.planes,
                cuts: self.section_display.cuts,
                fill: self.section_display.fill,
            },
        })
    }

    /// Positions of the vertices an entity is made of; for an instance, the corners of its
    /// bounding box; for a guide point, its position (none for a guide line).
    fn vertices_of(&self, id: EntityId) -> Vec<[f64; 3]> {
        let m = self.model();
        let ids: Vec<VertexId> = match id {
            EntityId::Vertex(v) => vec![v],
            EntityId::Instance(i) => {
                return self
                    .instance_box(i)
                    .into_iter()
                    .flatten()
                    .map(|p| p.to_array())
                    .collect();
            }
            EntityId::Guide(g) => {
                return match m.guide(g) {
                    Some(Guide::Point { position, .. }) => vec![position.to_array()],
                    _ => Vec::new(),
                };
            }
            EntityId::Dimension(d) => {
                let points = match m.dimension_points(d) {
                    Some((a, b)) => vec![a, b],
                    None => m.radial_circle(d).map(|c| c.center).into_iter().collect(),
                };
                return points.into_iter().map(|p| p.to_array()).collect();
            }
            EntityId::Note(n) => {
                return m.note_point(n).into_iter().map(|p| p.to_array()).collect();
            }
            EntityId::Section(p) => {
                return m
                    .section(p)
                    .map(|p| p.point.to_array())
                    .into_iter()
                    .collect();
            }
            EntityId::Edge(e) => m.edge(e).map(|e| e.v.to_vec()).unwrap_or_default(),
            EntityId::Face(f) => m
                .face(f)
                .map(|f| {
                    f.outer
                        .iter()
                        .chain(f.holes.iter().flatten())
                        .copied()
                        .collect()
                })
                .unwrap_or_default(),
        };
        ids.into_iter()
            .filter_map(|v| m.position(v).map(|p| p.to_array()))
            .collect()
    }

    fn vertex_info(&self, id: VertexId) -> VertexInfo {
        VertexInfo {
            id: EntityId::Vertex(id),
            position: self.model().position(id).unwrap().to_array(),
        }
    }

    fn edge_info(&self, id: EdgeId) -> EdgeInfo {
        let e = self.model().edge(id).unwrap();
        let [a, b] = e.v;
        EdgeInfo {
            id: EntityId::Edge(id),
            vertices: [EntityId::Vertex(a), EntityId::Vertex(b)],
            start: self.model().position(a).unwrap().to_array(),
            end: self.model().position(b).unwrap().to_array(),
            curve: e.curve.map(|c| c.to_string()),
            soft: e.soft,
            tag: e.tag,
        }
    }

    /// The curve an edge of the open context is part of.
    fn curve_info(&self, id: EntityId) -> Result<CurveInfo, CommandError> {
        let EntityId::Edge(e) = id else {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "the curve query needs an edge id",
            )
            .with_ids(vec![id]));
        };
        if self.model().edge(e).is_none() {
            return Err(
                CommandError::new(ErrorKind::NotFound, format!("no entity {id}"))
                    .with_ids(vec![id]),
            );
        }
        let Some(curve) = self.model().curve_of(e) else {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("{id} is not part of a curve"),
            )
            .with_ids(vec![id]));
        };
        let kind = curve.kind();
        let circle = curve.circle;
        Ok(CurveInfo {
            id: curve.id.to_string(),
            kind,
            edges: curve.edges.iter().map(|e| EntityId::Edge(*e)).collect(),
            vertices: curve
                .vertices
                .iter()
                .map(|v| EntityId::Vertex(*v))
                .collect(),
            segments: curve.segments,
            length: curve.length,
            center: circle.map(|c| c.center.to_array()),
            normal: circle.map(|c| c.normal.to_array()),
            radius: circle.map(|c| c.radius),
            angle: circle.map(|c| c.angle.to_degrees()),
        })
    }

    fn guide_info(&self, id: GuideId) -> GuideInfo {
        let guide = *self.model().guide(id).unwrap();
        let (kind, point, direction) = match guide {
            Guide::Line {
                point, direction, ..
            } => (GuideKind::Line, point, Some(direction)),
            Guide::Point { position, .. } => (GuideKind::Point, position, None),
        };
        GuideInfo {
            id: EntityId::Guide(id),
            kind,
            point: point.to_array(),
            direction: direction.map(|d| d.to_array()),
            tag: guide.tag(),
        }
    }

    fn face_info(&self, id: FaceId) -> FaceInfo {
        let f = self.model().face(id).unwrap();
        let ids = |l: &Vec<VertexId>| l.iter().map(|v| EntityId::Vertex(*v)).collect();
        FaceInfo {
            id: EntityId::Face(id),
            outer: ids(&f.outer),
            holes: f.holes.iter().map(ids).collect(),
            normal: f.normal.to_array(),
            area: self.model().area(id).unwrap(),
            front_material: f.front,
            back_material: f.back,
            texture_position: f.texture,
            tag: f.tag,
        }
    }
}

/// `v` scaled to unit length, or None when it is zero or not finite. Very long and very
/// short vectors work too (their squared length would overflow or underflow). Vectors of
/// ordinary length are normalized directly, so a unit vector comes back bit for bit.
pub fn unit_vector(v: DVec3) -> Option<DVec3> {
    let largest = v.abs().max_element();
    if !v.is_finite() || largest == 0.0 {
        return None;
    }
    v.try_normalize().or_else(|| (v / largest).try_normalize())
}

/// The refusal of an offset that the shape is too small for.
fn too_far() -> CommandError {
    CommandError::new(
        ErrorKind::InvalidInput,
        "the offset distance is too large for this shape",
    )
}

/// Some(polygon flag) when every one of `edges` is on one curve, so their offset is a curve too.
fn one_curve(m: &Model, edges: &[EdgeId]) -> Option<bool> {
    let first = m.edge(*edges.first()?)?;
    let curve = first.curve?;
    edges
        .iter()
        .all(|e| m.edge(*e).is_some_and(|e| e.curve == Some(curve)))
        .then_some(first.polygon)
}

/// Rejects non-finite or out-of-range coordinates before they reach the kernel or camera.
pub fn check_coordinates(points: &[[f64; 3]]) -> Result<(), CommandError> {
    let ok = points
        .iter()
        .flatten()
        .all(|c| c.is_finite() && c.abs() <= MAX_COORDINATE);
    if ok {
        Ok(())
    } else {
        Err(CommandError::new(
            ErrorKind::InvalidInput,
            format!("coordinates must be finite and within +/-{MAX_COORDINATE} m"),
        ))
    }
}

/// The steps an `undo` or `redo` of `count` takes: 1 by default, never 0.
fn step_count(count: Option<u32>) -> Result<usize, CommandError> {
    match count.unwrap_or(1) {
        0 => Err(CommandError::new(
            ErrorKind::InvalidInput,
            "count must be at least 1",
        )),
        n => Ok(n as usize),
    }
}

/// Why an `undo` or `redo` of more steps than there are fails.
fn too_few(what: &str, left: usize) -> String {
    match left {
        0 => format!("nothing to {what}"),
        1 => format!("only 1 step to {what}"),
        n => format!("only {n} steps to {what}"),
    }
}
