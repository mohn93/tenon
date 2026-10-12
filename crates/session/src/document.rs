use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use glam::{DAffine3, DVec2, DVec3};
use kernel::{
    ChangeSet, EdgeId, EntityId, FaceId, InstanceId, KernelError, MeshId, Model, Scene,
    SceneChange, VertexId,
};

use crate::camera::Camera;
use crate::command::{BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode};
use crate::query::{
    ChangeEvent, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, Query, QueryResult, VertexInfo,
};
use crate::rectangle::rectangle_corners;

mod files;
mod groups;

/// Largest coordinate magnitude (meters) accepted from commands.
pub const MAX_COORDINATE: f64 = 1.0e6;

/// Most copies one `copy` command may make.
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
            Command::SetCamera {
                eye,
                target,
                fov_y_deg,
            } => {
                self.camera = self.camera_for(eye, target, fov_y_deg)?;
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
            | Command::PushPull { .. }
            | Command::Erase { .. }
            | Command::Move { .. }
            | Command::Copy { .. }
            | Command::MakeGroup { .. }
            | Command::MakeComponent { .. }
            | Command::Explode { .. }
            | Command::RenameDefinition { .. }
            | Command::PurgeDefinitions => {
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
            Command::PushPull { face, distance } => self.push_pull_changes(*face, *distance),
            Command::Erase { ids } => self.erase_changes(ids),
            Command::Move { ids, offset } => self.move_changes(ids, *offset),
            Command::Copy { ids, offset, count } => {
                self.copy_changes(ids, *offset, count.unwrap_or(1))
            }
            Command::MakeGroup { ids } => self.group_changes(ids, None),
            Command::MakeComponent { ids, name } => self.group_changes(ids, Some(name)),
            Command::Explode { ids } => self.explode_changes(ids),
            Command::RenameDefinition { definition, name } => {
                self.rename_changes(*definition, name)
            }
            Command::PurgeDefinitions => Ok(self.scene.purge()),
            Command::New { .. } | Command::Open { .. } | Command::Save { .. } => return None,
            Command::Undo
            | Command::Redo
            | Command::SetCamera { .. }
            | Command::Select { .. }
            | Command::OpenInstance { .. }
            | Command::CloseInstance => {
                return None;
            }
        })
    }

    /// The current camera moved to `eye`/`target` (and `fov_y_deg` if given), after the same
    /// checks `SetCamera` applies. Does not change the document.
    pub fn camera_for(
        &self,
        eye: [f64; 3],
        target: [f64; 3],
        fov_y_deg: Option<f64>,
    ) -> Result<Camera, CommandError> {
        check_coordinates(&[eye, target])?;
        let (eye, target) = (DVec3::from(eye), DVec3::from(target));
        let fov = fov_y_deg.unwrap_or(self.camera.fov_y.to_degrees());
        if eye.distance(target) < 1e-6 {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "eye and target must be distinct points",
            ));
        }
        if (target - eye).normalize().cross(DVec3::Z).length() < 1e-9 {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "view direction must not be vertical",
            ));
        }
        if !(1.0..=170.0).contains(&fov) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "fov_y_deg must be between 1 and 170",
            ));
        }
        let mut camera = self.camera.clone();
        camera.eye = eye;
        camera.target = target;
        camera.fov_y = fov.to_radians();
        Ok(camera)
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
                ) =>
                {
                    Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        "undo, redo, new, open and save are not allowed in an atomic batch",
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
                "ids must list at least one vertex, edge, face or instance",
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
        let instances: Vec<InstanceId> = ids
            .iter()
            .filter_map(|id| match id {
                EntityId::Instance(i) => Some(*i),
                _ => None,
            })
            .collect();
        let offset = DVec3::from(offset);
        self.check_moved(&vertices, &instances, &[offset])?;
        self.edit_mesh(|m| {
            let mut c = m.move_vertices(&vertices, offset)?;
            c.ops.extend(m.move_instances(&instances, offset).ops);
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
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one edge, face or instance",
            ));
        }
        if !(1..=MAX_COPIES).contains(&count) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("count must be between 1 and {MAX_COPIES}"),
            ));
        }
        let parts = self.parts(ids)?;
        check_coordinates(&[offset])?;
        let offset = DVec3::from(offset);
        self.check_moved(
            &self.vertex_ids(ids),
            &parts.instances,
            &[offset * count as f64],
        )?;
        let geometry = if parts.edges.is_empty() && parts.faces.is_empty() {
            SceneChange::default()
        } else {
            self.edit_mesh(|m| m.copy(&parts.edges, &parts.faces, offset, count))
                .map_err(|e| self.kernel_error(e, ids))?
        };
        let placed = if parts.instances.is_empty() {
            SceneChange::default()
        } else {
            let mesh = self.active();
            match self
                .scene
                .copy_instances(mesh, &parts.instances, offset, count)
            {
                Ok(c) => c,
                Err(e) => {
                    self.scene.apply(&geometry.inverse());
                    return Err(self.kernel_error(e, ids));
                }
            }
        };
        Ok(SceneChange::join([geometry, placed]))
    }

    /// Rejects moves or copies that would put a vertex or an instance's bounding box (its
    /// origin when it has nothing in it) beyond `MAX_COORDINATE`.
    fn check_moved(
        &self,
        vertices: &BTreeSet<VertexId>,
        instances: &[InstanceId],
        offsets: &[DVec3],
    ) -> Result<(), CommandError> {
        let mut points: Vec<DVec3> = vertices
            .iter()
            .map(|v| self.model().position(*v).unwrap())
            .collect();
        for i in instances {
            match self.instance_box(*i) {
                Some(b) => points.extend(b),
                None => points.extend((self.model().instance(*i)).map(|i| i.transform.translation)),
            }
        }
        let moved: Vec<[f64; 3]> = points
            .iter()
            .flat_map(|p| offsets.iter().map(move |o| (*p + *o).to_array()))
            .collect();
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
                EntityId::Instance(_) => {}
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
                "ids must list at least one edge, face or instance",
            ));
        }
        let parts = self.parts(ids)?;
        let erased = self
            .edit_mesh(|m| Ok(m.erase(&parts.edges, &parts.faces)))
            .expect("erase cannot fail");
        let mesh = self.active();
        let removed = self.scene.erase_instances(mesh, &parts.instances);
        Ok(SceneChange::join([erased, removed]))
    }

    /// Splits ids (each counted once) into edges, faces and instances of the open context.
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
                "only edges, faces and instances are accepted here, not vertices",
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
                }
            }
            Query::Camera => QueryResult::Camera {
                eye: self.camera.eye.to_array(),
                target: self.camera.target.to_array(),
                fov_y_deg: self.camera.fov_y.to_degrees(),
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
    /// bounding box.
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
        let [a, b] = self.model().edge(id).unwrap().v;
        EdgeInfo {
            id: EntityId::Edge(id),
            vertices: [EntityId::Vertex(a), EntityId::Vertex(b)],
            start: self.model().position(a).unwrap().to_array(),
            end: self.model().position(b).unwrap().to_array(),
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
        }
    }
}

/// Edges, faces and instances named by a command.
#[derive(Default)]
struct Parts {
    edges: Vec<EdgeId>,
    faces: Vec<FaceId>,
    instances: Vec<InstanceId>,
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
