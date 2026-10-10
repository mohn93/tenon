use glam::{DVec2, DVec3};
use kernel::{ChangeSet, EdgeId, EntityId, FaceId, Model, VertexId};

use crate::camera::Camera;
use crate::command::{Command, CommandError, ErrorKind, Outcome};
use crate::query::{EdgeInfo, FaceInfo, Query, QueryResult, VertexInfo};

/// The open model plus everything needed to edit it: camera and undo history.
pub struct Document {
    pub model: Model,
    pub camera: Camera,
    undo: Vec<ChangeSet>,
    redo: Vec<ChangeSet>,
}

impl Document {
    pub fn new(viewport: DVec2) -> Self {
        Self {
            model: Model::new(),
            camera: Camera::new(viewport),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Runs one command. Each modeling command is one undo step; camera commands are not recorded.
    pub fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
        match command {
            Command::AddEdge { start, end } => {
                let changes = self
                    .model
                    .add_edge(DVec3::from(start), DVec3::from(end))
                    .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))?;
                Ok(self.record(changes))
            }
            Command::Undo => {
                let changes = self.undo.pop().ok_or_else(|| {
                    CommandError::new(ErrorKind::NothingToUndo, "nothing to undo")
                })?;
                let inverse = changes.inverse();
                self.model.apply(&inverse);
                self.redo.push(changes);
                Ok(Outcome {
                    changes: inverse.summary(),
                })
            }
            Command::Redo => {
                let changes = self.redo.pop().ok_or_else(|| {
                    CommandError::new(ErrorKind::NothingToRedo, "nothing to redo")
                })?;
                self.model.apply(&changes);
                let summary = changes.summary();
                self.undo.push(changes);
                Ok(Outcome { changes: summary })
            }
            Command::SetCamera {
                eye,
                target,
                fov_y_deg,
            } => {
                let (eye, target) = (DVec3::from(eye), DVec3::from(target));
                let fov = fov_y_deg.unwrap_or(self.camera.fov_y.to_degrees());
                if !eye.is_finite() || !target.is_finite() || eye.distance(target) < 1e-6 {
                    return Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        "eye and target must be distinct finite points",
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
                self.camera.eye = eye;
                self.camera.target = target;
                self.camera.fov_y = fov.to_radians();
                Ok(Outcome::default())
            }
        }
    }

    fn record(&mut self, changes: ChangeSet) -> Outcome {
        let summary = changes.summary();
        if !changes.is_empty() {
            self.undo.push(changes);
            self.redo.clear();
        }
        Outcome { changes: summary }
    }

    pub fn query(&self, query: &Query) -> Result<QueryResult, CommandError> {
        let m = &self.model;
        Ok(match query {
            Query::Entities => QueryResult::Entities {
                vertices: m.vertices().map(|(id, _)| self.vertex_info(id)).collect(),
                edges: m.edges().map(|(id, _)| self.edge_info(id)).collect(),
                faces: m.faces().map(|(id, _)| self.face_info(id)).collect(),
            },
            Query::Entity { id } => {
                let missing = || CommandError::new(ErrorKind::NotFound, format!("no entity {id}"));
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
            },
            Query::History => QueryResult::History {
                undo_depth: self.undo.len(),
                redo_depth: self.redo.len(),
            },
        })
    }

    fn vertex_info(&self, id: VertexId) -> VertexInfo {
        VertexInfo {
            id: EntityId::Vertex(id),
            position: self.model.pos(id).to_array(),
        }
    }

    fn edge_info(&self, id: EdgeId) -> EdgeInfo {
        let [a, b] = self.model.edge(id).unwrap().v;
        EdgeInfo {
            id: EntityId::Edge(id),
            vertices: [EntityId::Vertex(a), EntityId::Vertex(b)],
            start: self.model.pos(a).to_array(),
            end: self.model.pos(b).to_array(),
        }
    }

    fn face_info(&self, id: FaceId) -> FaceInfo {
        let f = self.model.face(id).unwrap();
        let ids = |l: &Vec<VertexId>| l.iter().map(|v| EntityId::Vertex(*v)).collect();
        FaceInfo {
            id: EntityId::Face(id),
            outer: ids(&f.outer),
            holes: f.holes.iter().map(ids).collect(),
            normal: f.normal.to_array(),
            area: self.model.face_area(id),
        }
    }
}
