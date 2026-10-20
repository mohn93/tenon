//! The model as plain data, for files: every entity, definition and id as stored.

use std::collections::BTreeMap;
use std::sync::Arc;

use glam::{DAffine3, DVec3};
use serde::{Deserialize, Serialize};

use crate::ids::{
    CurveId, DefinitionId, EdgeId, FaceId, GuideId, InstanceId, MaterialId, TagId, VertexId,
};
use crate::material::Material;
use crate::model::{Edge, Face, Guide, Instance, Model, Vertex};
use crate::scene::{Definition, DefinitionKind, Scene};
use crate::tag::Tag;
use crate::texture::{Texture, TextureImage, TexturePlacement};

/// Ids in a file are below this, and its `next_id` is at most this, so every id is exact as a
/// JSON (double) number and a scene that loads also saves and loads again.
pub const MAX_ID: u64 = 1 << 53;

/// Instance transforms whose 3x3 part has a smaller determinant (in absolute value) cannot be
/// inverted reliably and are rejected on load.
pub const MIN_DETERMINANT: f64 = 1e-12;

/// A whole scene as plain data. Ids are the kernel's numbers (without the "v"/"e"/... prefix).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneData {
    /// The id the next new entity gets; never reused.
    pub next_id: u64,
    pub root: MeshData,
    pub definitions: Vec<DefinitionData>,
    /// Absent in files written before materials existed (and when there are none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<MaterialData>,
    /// Absent in files written before tags existed (and when there are none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<TagData>,
}

/// A tag: `color` is sRGB red, green and blue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagData {
    pub id: u64,
    pub name: String,
    pub color: [u8; 3],
    /// Absent when the tag is shown.
    #[serde(default = "shown", skip_serializing_if = "is_shown")]
    pub visible: bool,
}

fn shown() -> bool {
    true
}

fn is_shown(visible: &bool) -> bool {
    *visible
}

/// A material: `color` is sRGB red, green and blue; `opacity` is from 0 (invisible) to 1
/// (opaque); `emission` from 0 (no glow) to `MAX_EMISSION`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialData {
    pub id: u64,
    pub name: String,
    pub color: [u8; 3],
    /// Absent when 1, as in files written before opacity existed.
    #[serde(default = "opaque", skip_serializing_if = "is_opaque")]
    pub opacity: f32,
    /// Absent without a texture, as in files written before textures existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<TextureData>,
    /// Absent when 0, as in files written before emission existed.
    #[serde(default, skip_serializing_if = "is_dark")]
    pub emission: f32,
}

/// A material's texture: the file holding its image and the size it covers on the model, in
/// meters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureData {
    /// The image file's name in the model file's archive.
    pub file: String,
    pub width: f64,
    pub height: f64,
    /// The image: `to_data` fills it in, and a reader must before `from_data`. Not written.
    #[serde(skip)]
    pub image: Option<Arc<TextureImage>>,
}

fn opaque() -> f32 {
    1.0
}

fn is_opaque(opacity: &f32) -> bool {
    *opacity == 1.0
}

fn is_dark(emission: &f32) -> bool {
    *emission == 0.0
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
    /// The curve the edge is part of (absent before curves existed, and for other edges).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<u64>,
    /// Absent (false) before soft edges existed, and for edges that are not soft.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub soft: bool,
    /// Absent (false) before polygons existed, and for edges that are not polygon sides.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub polygon: bool,
    /// The tag it is on (absent: Untagged, as in files written before tags existed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceData {
    pub id: u64,
    pub outer: Vec<u64>,
    pub holes: Vec<Vec<u64>>,
    pub normal: [f64; 3],
    /// The materials of the front and back sides (absent: the default material).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back: Option<u64>,
    /// The texture placement (absent: none, as in files written before placements existed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_position: Option<TexturePlacement>,
    /// The tag it is on (absent: Untagged, as in files written before tags existed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceData {
    pub id: u64,
    pub definition: u64,
    /// Column-major 3x4: x axis, y axis, z axis, translation.
    pub transform: [f64; 12],
    /// Absent when the instance is not painted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<u64>,
    /// The tag it is on (absent: Untagged, as in files written before tags existed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<u64>,
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
    /// The tag it is on (absent: Untagged, as in files written before tags existed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<u64>,
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
            materials: self
                .materials()
                .map(|(id, m)| MaterialData {
                    id: id.0,
                    name: m.name.clone(),
                    color: m.color,
                    opacity: m.opacity,
                    texture: m.texture.as_ref().map(|t| TextureData {
                        file: format!("textures/m{}.{}", id.0, t.image.format.extension()),
                        width: t.width,
                        height: t.height,
                        image: Some(t.image.clone()),
                    }),
                    emission: m.emission,
                })
                .collect(),
            tags: self
                .tags()
                .map(|(id, t)| TagData {
                    id: id.0,
                    name: t.name.clone(),
                    color: t.color,
                    visible: t.visible,
                })
                .collect(),
        }
    }

    /// Rebuilds a scene from plain data, keeping every id. Fails, naming the problem, when an
    /// id is used twice (by things of any kind) or is not below [`MAX_ID`], `next_id` is over
    /// it, a number is not finite, an instance's transform
    /// cannot be inverted, or the result breaks a model invariant (see [`Scene::validate`]:
    /// that includes painting with a missing material, an entity on a missing tag, and blank,
    /// overlong or repeated material and tag names).
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
        let mut materials = BTreeMap::new();
        for m in &data.materials {
            check_id("material m", m.id)?;
            let texture = match &m.texture {
                None => None,
                Some(t) => Some(Texture {
                    image: t
                        .image
                        .clone()
                        .ok_or_else(|| format!("material m{}'s texture has no image", m.id))?,
                    width: t.width,
                    height: t.height,
                }),
            };
            let material = Material {
                name: m.name.clone(),
                color: m.color,
                opacity: m.opacity,
                texture,
                emission: m.emission,
            };
            if materials.insert(MaterialId(m.id), material).is_some() {
                return Err(format!("material m{} appears twice", m.id));
            }
        }
        let mut tags = BTreeMap::new();
        for t in &data.tags {
            check_id("tag t", t.id)?;
            let tag = Tag {
                name: t.name.clone(),
                color: t.color,
                visible: t.visible,
            };
            if tags.insert(TagId(t.id), tag).is_some() {
                return Err(format!("tag t{} appears twice", t.id));
            }
        }
        let scene = Scene::from_meshes(root, definitions, materials, tags, data.next_id);
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
                curve: e.curve.map(|c| c.0),
                soft: e.soft,
                polygon: e.polygon,
                tag: e.tag.map(|t| t.0),
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
                front: f.front.map(|m| m.0),
                back: f.back.map(|m| m.0),
                texture_position: f.texture,
                tag: f.tag.map(|t| t.0),
            })
            .collect(),
        instances: m
            .instances()
            .map(|(id, i)| InstanceData {
                id: id.0,
                definition: i.definition.0,
                transform: i.transform.to_cols_array(),
                material: i.material.map(|m| m.0),
                tag: i.tag.map(|t| t.0),
            })
            .collect(),
        guides: m
            .guides()
            .map(|(id, g)| match *g {
                Guide::Line {
                    point,
                    direction,
                    tag,
                } => GuideData {
                    id: id.0,
                    point: point.to_array(),
                    direction: Some(direction.to_array()),
                    tag: tag.map(|t| t.0),
                },
                Guide::Point { position, tag } => GuideData {
                    id: id.0,
                    point: position.to_array(),
                    direction: None,
                    tag: tag.map(|t| t.0),
                },
            })
            .collect(),
    }
}

fn check_id(what: &str, id: u64) -> Result<(), String> {
    if id >= MAX_ID {
        return Err(format!(
            "{what}{id} has an id too large (ids must be below {MAX_ID})"
        ));
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
        if let Some(c) = e.curve {
            check_id("curve c", c)?;
        }
        let edge = Edge {
            v: e.vertices.map(VertexId),
            curve: e.curve.map(CurveId),
            soft: e.soft,
            polygon: e.polygon,
            tag: e.tag.map(TagId),
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
            front: f.front.map(MaterialId),
            back: f.back.map(MaterialId),
            // A default placement is the same as none.
            texture: f.texture_position.filter(|t| !t.is_default()),
            tag: f.tag.map(TagId),
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
            material: i.material.map(MaterialId),
            tag: i.tag.map(TagId),
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
        let tag = g.tag.map(TagId);
        let guide = match g.direction {
            None => Guide::Point {
                position: point,
                tag,
            },
            Some(d) => {
                finite(&d, format!("guide g{}", g.id))?;
                let direction = DVec3::from(d);
                if (direction.length() - 1.0).abs() > 1e-9 {
                    return Err(format!(
                        "guide g{} has a direction that is not a unit vector",
                        g.id
                    ));
                }
                Guide::Line {
                    point,
                    direction,
                    tag,
                }
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
