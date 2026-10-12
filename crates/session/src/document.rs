use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use glam::{DAffine3, DVec2, DVec3};
use kernel::{
    ChangeSet, EdgeId, EntityId, FaceId, Guide, GuideId, Instance, InstanceId, KernelError, MeshId,
    Model, Scene, SceneChange, VertexId,
};

use crate::arc::{DEFAULT_ARC_SEGMENTS, arc_points};
use crate::camera::Camera;
use crate::circle::{DEFAULT_CIRCLE_SEGMENTS, circle_points};
use crate::command::{BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode};
use crate::query::{
    ChangeEvent, CurveInfo, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, GuideInfo, GuideKind,
    Query, QueryResult, VertexInfo,
};
use crate::rectangle::rectangle_corners;
use crate::rotate::rotation;

mod camera;
mod files;
mod groups;
mod materials;

/// Largest coordinate magnitude (meters) accepted from commands.
pub const MAX_COORDINATE: f64 = 1.0e6;

/// Most copies one `copy` or `rotate` command may make.
pub const MAX_COPIES: u32 = 100;

/// How many recent model changes `Query::Events` can return.
const EVENT_LOG_LEN: usize = 1024;

/// The open model plus everything needed to edit it: camera, selection, the open context and
/// undo history.
pub struct Document {
    scene: Scene,
    /// The instances being edited, from the root; empty when the root is open.
    context: Vec<InstanceId>,
    pub camera: Camera,
    selection: BTreeSet<EntityId>,
    undo: Vec<SceneChange>,
    redo: Vec<SceneChange>,
    revision: u64,
    events: VecDeque<ChangeEvent>,
    /// Changes up to this revision are missing from `events` (the model was replaced).
    log_start: u64,
    /// The file last opened or saved.
    path: Option<PathBuf>,
    /// The revision the model had when it was last opened, saved or started.
    clean_revision: u64,
}

impl Document {
    pub fn new(viewport: DVec2) -> Self {
        Self {
            scene: Scene::new(),
            context: Vec::new(),
            camera: Camera::new(viewport),
            selection: BTreeSet::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
            events: VecDeque::new(),
            log_start: 0,
            path: None,
            clean_revision: 0,
        }
    }

    /// The file the model was last opened from or saved to.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// True when the model changed since it was opened, saved or started.
    pub fn is_modified(&self) -> bool {
        self.revision != self.clean_revision
    }

    /// The whole model: the root mesh and every definition.
    pub fn scene(&self) -> &Scene {
        &self.scene
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

    /// Increases every time the model changes (edits, undo, redo). Clients compare it to decide
    /// whether to rebuild anything derived from the model. Opening or closing an instance and
    /// selection changes do not bump it; compare `context()` too.
    pub fn revision(&self) -> u64 {
        self.revision
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

    /// Runs one command. Each modeling command is one undo step; camera commands are not recorded.
    pub fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
        if let Some(changes) = self.edit(&command) {
            return Ok(self.record(changes?));
        }
        match command {
            Command::Undo => {
                let changes = self.undo.pop().ok_or_else(|| {
                    CommandError::new(ErrorKind::NothingToUndo, "nothing to undo")
                })?;
                let inverse = changes.inverse();
                self.scene.apply(&inverse);
                self.redo.push(changes);
                Ok(self.changed(&inverse))
            }
            Command::Redo => {
                let changes = self.redo.pop().ok_or_else(|| {
                    CommandError::new(ErrorKind::NothingToRedo, "nothing to redo")
                })?;
                self.scene.apply(&changes);
                let outcome = self.changed(&changes);
                self.undo.push(changes);
                Ok(outcome)
            }
            Command::SetCamera { .. }
            | Command::Orbit { .. }
            | Command::Pan { .. }
            | Command::Zoom { .. }
            | Command::ZoomExtents
            | Command::StandardView { .. }
            | Command::SetProjection { .. } => {
                self.camera = self.moved_camera(&command)?;
                Ok(Outcome::default())
            }
            Command::OpenInstance { id } => {
                self.open_instance(id)?;
                Ok(Outcome::default())
            }
            Command::CloseInstance => {
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
                self.open_file(&path)?;
                Ok(Outcome::default())
            }
            Command::Save { path } => {
                self.save_file(path.as_deref())?;
                Ok(Outcome::default())
            }
            Command::Export { path, format } => {
                self.export_file(&path, format)?;
                Ok(Outcome::default())
            }
            Command::Select { ids, mode } => {
                self.parts(&ids)?;
                let ids: BTreeSet<EntityId> = ids.into_iter().collect();
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
            | Command::AddArc { .. }
            | Command::AddGuideLine { .. }
            | Command::AddGuidePoint { .. }
            | Command::DeleteGuides
            | Command::PushPull { .. }
            | Command::Erase { .. }
            | Command::Move { .. }
            | Command::Copy { .. }
            | Command::Rotate { .. }
            | Command::MakeGroup { .. }
            | Command::MakeComponent { .. }
            | Command::Explode { .. }
            | Command::RenameDefinition { .. }
            | Command::PurgeDefinitions
            | Command::AddMaterial { .. }
            | Command::ChangeMaterial { .. }
            | Command::DeleteMaterial { .. }
            | Command::Paint { .. } => {
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
            Command::AddArc {
                center,
                start,
                angle,
                normal,
                segments,
            } => self.add_arc_changes(*center, *start, *angle, *normal, *segments),
            Command::AddGuideLine { point, direction } => {
                self.add_guide_line_changes(*point, *direction)
            }
            Command::AddGuidePoint { position } => check_coordinates(&[*position]).and_then(|()| {
                self.add_guide(Guide::Point {
                    position: DVec3::from(*position),
                })
            }),
            Command::DeleteGuides => {
                let all: Vec<GuideId> = self.model().guides().map(|(id, _)| id).collect();
                Ok(self
                    .edit_mesh(|m| Ok(m.erase_guides(&all)))
                    .expect("erasing guides cannot fail"))
            }
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
            Command::MakeGroup { ids } => self.group_changes(ids, None),
            Command::MakeComponent { ids, name } => self.group_changes(ids, Some(name)),
            Command::Explode { ids } => self.explode_changes(ids),
            Command::RenameDefinition { definition, name } => {
                self.rename_changes(*definition, name)
            }
            Command::PurgeDefinitions => Ok(self.scene.purge()),
            Command::AddMaterial { name, color } => {
                self.add_material_changes(name.as_deref(), *color)
            }
            Command::ChangeMaterial {
                material,
                name,
                color,
            } => self.change_material_changes(*material, name.as_deref(), *color),
            Command::DeleteMaterial { material } => self.delete_material_changes(*material),
            Command::Paint {
                ids,
                material,
                side,
            } => self.paint_changes(ids, *material, *side),
            Command::New { .. }
            | Command::Open { .. }
            | Command::Save { .. }
            | Command::Export { .. } => return None,
            Command::Undo
            | Command::Redo
            | Command::SetCamera { .. }
            | Command::Orbit { .. }
            | Command::Pan { .. }
            | Command::Zoom { .. }
            | Command::ZoomExtents
            | Command::StandardView { .. }
            | Command::SetProjection { .. }
            | Command::Select { .. }
            | Command::OpenInstance { .. }
            | Command::CloseInstance
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
    pub fn execute_batch(
        &mut self,
        commands: Vec<Command>,
        atomic: bool,
    ) -> Result<Vec<Outcome>, BatchError> {
        let mut outcomes = Vec::new();
        if !atomic {
            for (index, command) in commands.into_iter().enumerate() {
                match self.execute(command) {
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
        let selection_before = self.selection.clone();
        let context_before = self.context.clone();
        let mut applied: Vec<SceneChange> = Vec::new();
        for (index, command) in commands.into_iter().enumerate() {
            let result = match self.edit(&command) {
                Some(changes) => changes.map(|c| {
                    let summary = c.summary();
                    applied.push(c);
                    Outcome { changes: summary }
                }),
                None if matches!(
                    command,
                    Command::Undo
                        | Command::Redo
                        | Command::New { .. }
                        | Command::Open { .. }
                        | Command::Save { .. }
                        | Command::Export { .. }
                ) =>
                {
                    Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        "undo, redo, new, open, save and export are not allowed in an atomic batch",
                    ))
                }
                None => self.execute(command),
            };
            match result {
                Ok(o) => outcomes.push(o),
                Err(error) => {
                    for c in applied.iter().rev() {
                        self.scene.apply(&c.inverse());
                    }
                    self.camera = camera_before;
                    self.selection = selection_before;
                    self.context = context_before;
                    return Err(BatchError {
                        index,
                        error,
                        completed: outcomes,
                        rolled_back: true,
                    });
                }
            }
        }
        self.record(SceneChange::join(applied));
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
        let direction = DVec3::from(direction);
        let Some(direction) = direction
            .is_finite()
            .then(|| direction.try_normalize())
            .flatten()
        else {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "direction must be a finite, non-zero vector",
            ));
        };
        self.add_guide(Guide::Line {
            point: DVec3::from(point),
            direction,
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
        self.edit_mesh(|m| m.add_curve(&points, true))
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
    }

    fn add_arc_changes(
        &mut self,
        center: [f64; 3],
        start: [f64; 3],
        angle: f64,
        normal: Option<[f64; 3]>,
        segments: Option<u32>,
    ) -> Result<SceneChange, CommandError> {
        check_coordinates(&[center, start])?;
        let points = arc_points(
            DVec3::from(center),
            DVec3::from(start),
            angle,
            normal.map(DVec3::from),
            segments.unwrap_or(DEFAULT_ARC_SEGMENTS),
        )?;
        check_coordinates(&points.iter().map(|p| p.to_array()).collect::<Vec<_>>())?;
        self.edit_mesh(|m| m.add_curve(&points, false))
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
                "ids must list at least one vertex, edge, face, instance or guide",
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
        let (instances, guides) = (instance_ids(ids), guide_ids(ids));
        let offset = DVec3::from(offset);
        let shift = DAffine3::from_translation(offset);
        self.check_placed(&vertices, &instances, &guides, &[shift])?;
        self.edit_mesh(|m| {
            let mut c = m.move_vertices(&vertices, offset)?;
            c.ops.extend(m.move_instances(&instances, offset).ops);
            c.ops.extend(m.transform_guides(&guides, shift).ops);
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
                "ids must list at least one vertex, edge, face, instance or guide",
            ));
        }
        check_coordinates(&[center, axis.unwrap_or([0.0, 0.0, 1.0])])?;
        let Some(axis) = DVec3::from(axis.unwrap_or([0.0, 0.0, 1.0])).try_normalize() else {
            return Err(invalid("axis must be a non-zero vector"));
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
            self.check_placed(
                &self.vertex_ids(ids),
                &parts.instances,
                &parts.guides,
                &places,
            )?;
            return self.copy_placed_changes(ids, &parts, &places);
        }
        let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
        if !missing.is_empty() {
            return Err(
                CommandError::new(ErrorKind::NotFound, "no such entities").with_ids(missing)
            );
        }
        let vertices = self.vertex_ids(ids);
        let (instances, guides) = (instance_ids(ids), guide_ids(ids));
        let turn = rotation(center, axis, angle);
        self.check_placed(&vertices, &instances, &guides, &[turn])?;
        self.edit_mesh(|m| {
            let mut c = m.transform_vertices(&vertices, turn)?;
            c.ops.extend(m.transform_instances(&instances, turn).ops);
            c.ops.extend(m.transform_guides(&guides, turn).ops);
            Ok(c)
        })
        .map_err(|e| self.kernel_error(e, ids))
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
        self.check_placed(
            &self.vertex_ids(ids),
            &parts.instances,
            &parts.guides,
            &places,
        )?;
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
                "ids must list at least one edge, face, instance or guide",
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
            .edit_mesh(|m| Ok(m.copy_guides(&parts.guides, places)))
            .expect("copying guides cannot fail");
        Ok(SceneChange::join([geometry, placed, guides]))
    }

    /// Rejects moves, rotations or copies that would put a vertex, an instance's bounding
    /// box (its origin when it has nothing in it) or a guide's point beyond `MAX_COORDINATE`
    /// under any of `places`.
    fn check_placed(
        &self,
        vertices: &BTreeSet<VertexId>,
        instances: &[InstanceId],
        guides: &[GuideId],
        places: &[DAffine3],
    ) -> Result<(), CommandError> {
        let m = self.model();
        let mut moved: Vec<[f64; 3]> = Vec::new();
        for t in places {
            let at = |p: DVec3| t.transform_point3(p).to_array();
            moved.extend(vertices.iter().map(|v| at(m.position(*v).unwrap())));
            moved.extend(
                guides
                    .iter()
                    .filter_map(|g| m.guide(*g))
                    .map(|g| at(g.anchor())),
            );
            for id in instances {
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
                EntityId::Instance(_) | EntityId::Guide(_) => {}
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
                "ids must list at least one edge, face, instance or guide",
            ));
        }
        let parts = self.parts(ids)?;
        let erased = self
            .edit_mesh(|m| {
                let mut c = m.erase(&parts.edges, &parts.faces);
                c.ops.extend(m.erase_guides(&parts.guides).ops);
                Ok(c)
            })
            .expect("erase cannot fail");
        let mesh = self.active();
        let removed = self.scene.erase_instances(mesh, &parts.instances);
        Ok(SceneChange::join([erased, removed]))
    }

    /// Splits ids (each counted once) into edges, faces, instances and guides of the open
    /// context.
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
                "only edges, faces, instances and guides are accepted here, not vertices",
            )
            .with_ids(vertices));
        }
        let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
        if !missing.is_empty() {
            return Err(
                CommandError::new(ErrorKind::NotFound, "no such entities").with_ids(missing)
            );
        }
        let mut parts = Parts::default();
        for id in ids.iter().collect::<BTreeSet<_>>() {
            match *id {
                EntityId::Edge(e) => parts.edges.push(e),
                EntityId::Face(f) => parts.faces.push(f),
                EntityId::Instance(i) => parts.instances.push(i),
                EntityId::Guide(g) => parts.guides.push(g),
                EntityId::Vertex(_) => {}
            }
        }
        Ok(parts)
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
        }
    }

    fn record(&mut self, changes: SceneChange) -> Outcome {
        if changes.is_empty() {
            return Outcome::default();
        }
        let outcome = self.changed(&changes);
        self.undo.push(changes);
        self.redo.clear();
        outcome
    }

    /// Bumps the revision, logs the change, closes open instances that no longer exist and
    /// drops entities that are no longer in the open context from the selection.
    fn changed(&mut self, changes: &SceneChange) -> Outcome {
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
        });
        if self.events.len() > EVENT_LOG_LEN {
            self.events.pop_front();
        }
        Outcome { changes: summary }
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
                }
            }
            Query::Curve { id } => QueryResult::Curve(self.curve_info(*id)?),
            Query::Camera => QueryResult::Camera {
                eye: self.camera.eye.to_array(),
                target: self.camera.target.to_array(),
                up: self.camera.up.to_array(),
                fov_y_deg: self.camera.fov_y.to_degrees(),
                projection: self.camera.projection,
                view_height: self.camera.view_height(),
            },
            Query::Stats => QueryResult::Stats {
                vertices: m.vertices().count(),
                edges: m.edges().count(),
                faces: m.faces().count(),
                instances: m.instances().count(),
                definitions: self.scene.definitions().count(),
                revision: self.revision,
            },
            Query::History => QueryResult::History {
                undo_depth: self.undo.len(),
                redo_depth: self.redo.len(),
                revision: self.revision,
            },
            Query::Selection => QueryResult::Selection {
                ids: self.selection.iter().copied().collect(),
                revision: self.revision,
            },
            Query::Definitions => QueryResult::Definitions {
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
            Query::Materials => QueryResult::Materials {
                materials: self.material_infos(),
                revision: self.revision,
            },
            Query::Context => QueryResult::Context {
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
            Query::Outliner => QueryResult::Outliner {
                instances: self.outliner(),
                revision: self.revision,
            },
            Query::File => QueryResult::File {
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
                    Some(Guide::Point { position }) => vec![position.to_array()],
                    _ => Vec::new(),
                };
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
        let (kind, point, direction) = match *self.model().guide(id).unwrap() {
            Guide::Line { point, direction } => (GuideKind::Line, point, Some(direction)),
            Guide::Point { position } => (GuideKind::Point, position, None),
        };
        GuideInfo {
            id: EntityId::Guide(id),
            kind,
            point: point.to_array(),
            direction: direction.map(|d| d.to_array()),
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
        }
    }
}

/// Edges, faces, instances and guides named by a command.
#[derive(Default)]
struct Parts {
    edges: Vec<EdgeId>,
    faces: Vec<FaceId>,
    instances: Vec<InstanceId>,
    guides: Vec<GuideId>,
}

/// The instances among `ids`.
fn instance_ids(ids: &[EntityId]) -> Vec<InstanceId> {
    ids.iter()
        .filter_map(|id| match id {
            EntityId::Instance(i) => Some(*i),
            _ => None,
        })
        .collect()
}

/// The guides among `ids`.
fn guide_ids(ids: &[EntityId]) -> Vec<GuideId> {
    ids.iter()
        .filter_map(|id| match id {
            EntityId::Guide(g) => Some(*g),
            _ => None,
        })
        .collect()
}

/// Rejects non-finite or out-of-range coordinates before they reach the kernel or camera.
fn check_coordinates(points: &[[f64; 3]]) -> Result<(), CommandError> {
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
