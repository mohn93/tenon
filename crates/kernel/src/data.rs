//! The model as plain data, for files: every entity, definition and id as stored.

use std::collections::BTreeMap;

use glam::{DAffine3, DVec3};
use serde::{Deserialize, Serialize};

use crate::ids::{DefinitionId, EdgeId, FaceId, GuideId, InstanceId, VertexId};
use crate::model::{Edge, Face, Guide, Instance, Model, Vertex};
use crate::scene::{Definition, DefinitionKind, Scene};

/// The largest id a file may use: every id up to it is exact as a JSON (double) number.
pub const MAX_ID: u64 = 1 << 53;

/// Instance transforms whose 3x3 part has a smaller determinant (in absolute value) cannot be
/// inverted reliably and are rejected on load.
const MIN_DETERMINANT: f64 = 1e-12;

/// A whole scene as plain data. Ids are the kernel's numbers (without the "v"/"e"/... prefix).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneData {
    /// The id the next new entity gets; never reused.
    pub next_id: u64,
    pub root: MeshData,
    pub definitions: Vec<DefinitionData>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshData {
    pub vertices: Vec<VertexData>,
    pub edges: Vec<EdgeData>,
    pub faces: Vec<FaceData>,
    pub instances: Vec<InstanceData>,
    /// Absent in files written before guides existed (and when there are none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guides: Vec<GuideData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VertexData {
    pub id: u64,
    pub position: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeData {
    pub id: u64,
    pub vertices: [u64; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceData {
    pub id: u64,
    pub outer: Vec<u64>,
    pub holes: Vec<Vec<u64>>,
    pub normal: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceData {
    pub id: u64,
    pub definition: u64,
    /// Column-major 3x4: x axis, y axis, z axis, translation.
    pub transform: [f64; 12],
}

/// A guide line through `point` along `direction` (a unit vector), or, without `direction`, a
/// guide point at `point`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideData {
    pub id: u64,
    pub point: [f64; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<[f64; 3]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefinitionData {
    pub id: u64,
    pub name: String,
    pub kind: DefinitionKind,
    pub mesh: MeshData,
}

impl Scene {
    /// The scene as plain data.
    pub fn to_data(&self) -> SceneData {
        SceneData {
            next_id: self.next_id(),
            root: mesh_data(self.root()),
            definitions: self
                .definitions()
                .map(|(id, d)| DefinitionData {
                    id: id.0,
                    name: d.name.clone(),
                    kind: d.kind,
                    mesh: mesh_data(&d.mesh),
                })
                .collect(),
        }
    }

    /// Rebuilds a scene from plain data, keeping every id. Fails, naming the problem, when an
    /// id is used twice or is over [`MAX_ID`], a number is not finite, an instance's transform
    /// cannot be inverted, or the result breaks a model invariant (see [`Scene::validate`]).
    pub fn from_data(data: &SceneData) -> Result<Scene, String> {
        if data.next_id > MAX_ID {
            return Err(format!(
                "next_id {} is too large (over {MAX_ID})",
                data.next_id
            ));
        }
        let root = mesh(&data.root)?;
        let mut definitions = BTreeMap::new();
        for d in &data.definitions {
            check_id("definition d", d.id)?;
            let definition = Definition {
                name: d.name.clone(),
                kind: d.kind,
                mesh: mesh(&d.mesh)?,
            };
            if definitions.insert(DefinitionId(d.id), definition).is_some() {
                return Err(format!("definition d{} appears twice", d.id));
            }
        }
        let scene = Scene::from_meshes(root, definitions, data.next_id);
        scene.validate()?;
        Ok(scene)
    }
}

fn mesh_data(m: &Model) -> MeshData {
    MeshData {
        vertices: m
            .vertices()
            .map(|(id, v)| VertexData {
                id: id.0,
                position: v.pos.to_array(),
            })
            .collect(),
        edges: m
            .edges()
            .map(|(id, e)| EdgeData {
                id: id.0,
                vertices: e.v.map(|v| v.0),
            })
            .collect(),
        faces: m
            .faces()
            .map(|(id, f)| FaceData {
                id: id.0,
                outer: f.outer.iter().map(|v| v.0).collect(),
                holes: f
                    .holes
                    .iter()
                    .map(|h| h.iter().map(|v| v.0).collect())
                    .collect(),
                normal: f.normal.to_array(),
            })
            .collect(),
        instances: m
            .instances()
            .map(|(id, i)| InstanceData {
                id: id.0,
                definition: i.definition.0,
                transform: i.transform.to_cols_array(),
            })
            .collect(),
        guides: m
            .guides()
            .map(|(id, g)| match *g {
                Guide::Line { point, direction } => GuideData {
                    id: id.0,
                    point: point.to_array(),
                    direction: Some(direction.to_array()),
                },
                Guide::Point { position } => GuideData {
                    id: id.0,
                    point: position.to_array(),
                    direction: None,
                },
            })
            .collect(),
    }
}

fn check_id(what: &str, id: u64) -> Result<(), String> {
    if id > MAX_ID {
        return Err(format!("{what}{id} has an id too large (over {MAX_ID})"));
    }
    Ok(())
}

fn mesh(data: &MeshData) -> Result<Model, String> {
    let finite = |xs: &[f64], what: String| {
        if xs.iter().all(|x| x.is_finite()) {
            Ok(())
        } else {
            Err(format!("{what} has a number that is not finite"))
        }
    };
    let mut vertices = BTreeMap::new();
    for v in &data.vertices {
        check_id("vertex v", v.id)?;
        finite(&v.position, format!("vertex v{}", v.id))?;
        let vertex = Vertex {
            pos: DVec3::from(v.position),
        };
        if vertices.insert(VertexId(v.id), vertex).is_some() {
            return Err(format!("vertex v{} appears twice", v.id));
        }
    }
    let mut edges = BTreeMap::new();
    for e in &data.edges {
        check_id("edge e", e.id)?;
        let edge = Edge {
            v: e.vertices.map(VertexId),
        };
        if edges.insert(EdgeId(e.id), edge).is_some() {
            return Err(format!("edge e{} appears twice", e.id));
        }
    }
    let mut faces = BTreeMap::new();
    for f in &data.faces {
        check_id("face f", f.id)?;
        finite(&f.normal, format!("face f{}", f.id))?;
        let ring = |l: &Vec<u64>| l.iter().map(|v| VertexId(*v)).collect();
        let face = Face {
            outer: ring(&f.outer),
            holes: f.holes.iter().map(ring).collect(),
            normal: DVec3::from(f.normal),
        };
        if faces.insert(FaceId(f.id), face).is_some() {
            return Err(format!("face f{} appears twice", f.id));
        }
    }
    let mut instances = BTreeMap::new();
    for i in &data.instances {
        check_id("instance i", i.id)?;
        finite(&i.transform, format!("instance i{}", i.id))?;
        let transform = DAffine3::from_cols_array(&i.transform);
        if transform.matrix3.determinant().abs() < MIN_DETERMINANT {
            return Err(format!(
                "instance i{} has a transform that is not invertible",
                i.id
            ));
        }
        let instance = Instance {
            definition: DefinitionId(i.definition),
            transform,
        };
        if instances.insert(InstanceId(i.id), instance).is_some() {
            return Err(format!("instance i{} appears twice", i.id));
        }
    }
    let mut guides = BTreeMap::new();
    for g in &data.guides {
        check_id("guide g", g.id)?;
        finite(&g.point, format!("guide g{}", g.id))?;
        let point = DVec3::from(g.point);
        let guide = match g.direction {
            None => Guide::Point { position: point },
            Some(d) => {
                finite(&d, format!("guide g{}", g.id))?;
                let direction = DVec3::from(d);
                if (direction.length() - 1.0).abs() > 1e-9 {
                    return Err(format!(
                        "guide g{} has a direction that is not a unit vector",
                        g.id
                    ));
                }
                Guide::Line { point, direction }
            }
        };
        if guides.insert(GuideId(g.id), guide).is_some() {
            return Err(format!("guide g{} appears twice", g.id));
        }
    }
    let ids = vertices.len() + edges.len() + faces.len() + instances.len() + guides.len();
    let distinct: std::collections::BTreeSet<u64> = vertices
        .keys()
        .map(|k| k.0)
        .chain(edges.keys().map(|k| k.0))
        .chain(faces.keys().map(|k| k.0))
        .chain(instances.keys().map(|k| k.0))
        .chain(guides.keys().map(|k| k.0))
        .collect();
    if distinct.len() != ids {
        return Err("two entities of different kinds share an id".into());
    }
    Ok(Model::from_parts(vertices, edges, faces, instances, guides))
}
