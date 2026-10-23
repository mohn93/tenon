//! The model as plain data, for files: every entity, definition and id as stored.

use std::collections::BTreeMap;
use std::sync::Arc;

use glam::{DAffine3, DVec3};
use serde::{Deserialize, Serialize};

use crate::annotation::{Anchor, Dimension, Note, RadialAnchor};
use crate::axes::Axes;
use crate::ids::{
    CurveId, DefinitionId, DimensionId, EdgeId, FaceId, GuideId, InstanceId, MaterialId, NoteId,
    PageId, SectionId, TagId, VertexId,
};
use crate::location::Location;
use crate::material::Material;
use crate::model::{Edge, Face, Guide, Instance, Model, Vertex};
use crate::page::{Page, PageView};
use crate::scene::{Definition, DefinitionKind, Scene};
use crate::section::SectionPlane;
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
    /// The pages (saved scenes), in order. Absent in files written before scenes existed (and
    /// when there are none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<PageData>,
    /// The model axes. Absent in files written before axes existed (and when they are the
    /// world axes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axes: Option<AxesData>,
    /// Where the model stands. Absent in files written before locations existed (and when it
    /// is the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<LocationData>,
}

/// Where the model stands: see [`crate::Location`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationData {
    pub latitude: f64,
    pub longitude: f64,
    pub north_angle: f64,
    pub utc_offset: f64,
}

/// The model axes: an origin and three perpendicular unit vectors, right-handed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxesData {
    pub origin: [f64; 3],
    pub x: [f64; 3],
    pub y: [f64; 3],
    pub z: [f64; 3],
}

/// A page (saved scene). `view` is absent when the page does not store the camera, `tags`
/// when it does not store tag visibility; `tags` lists the tags hidden on it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageData {
    pub id: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<PageView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<u64>>,
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
    /// Absent in files written before dimensions existed (and when there are none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<DimensionData>,
    /// Absent in files written before notes existed (and when there are none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<NoteData>,
    /// Absent in files written before section planes existed (and when there are none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<SectionData>,
    /// The id of the plane cutting the mesh (absent: none, as when there are no planes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_section: Option<u64>,
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
    /// Absent (false) before hiding existed, and for entities that are shown.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
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
    /// Absent (false) before hiding existed, and for entities that are shown.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
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
    /// Absent (false) before hiding existed, and for entities that are shown.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
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
    /// Absent (false) before hiding existed, and for entities that are shown.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

/// A dimension end or a note's anchor: `{"vertex": 12}` or `{"point": [x, y, z]}`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorData {
    Vertex(u64),
    Point([f64; 3]),
}

/// The circle a radial dimension measures: `{"curve": 5}` or
/// `{"fixed": {"center": ..., "normal": ..., "radius": ...}}`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RadialAnchorData {
    Curve(u64),
    Fixed(CircleData),
}

/// A fixed circle: `normal` is a unit vector.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CircleData {
    pub center: [f64; 3],
    pub normal: [f64; 3],
    pub radius: f64,
}

/// A dimension (see `Dimension`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DimensionData {
    Linear {
        id: u64,
        a: AnchorData,
        b: AnchorData,
        offset: [f64; 3],
        /// The tag it is on (absent: Untagged).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u64>,
    },
    Radial {
        id: u64,
        curve: RadialAnchorData,
        /// Absent (false) for a radius.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        diameter: bool,
        leader: [f64; 3],
        /// The tag it is on (absent: Untagged).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u64>,
    },
}

/// A note (see `Note`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NoteData {
    Leader {
        id: u64,
        anchor: AnchorData,
        position: [f64; 3],
        /// Absent for the anchored point's coordinates.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        /// The tag it is on (absent: Untagged).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u64>,
    },
    Screen {
        id: u64,
        at: [f64; 2],
        text: String,
        /// The tag it is on (absent: Untagged).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u64>,
    },
}

/// A section plane (see `SectionPlane`): `normal` is a unit vector.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectionData {
    pub id: u64,
    pub point: [f64; 3],
    pub normal: [f64; 3],
    /// The tag it is on (absent: Untagged).
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
            pages: self
                .pages()
                .map(|(id, p)| PageData {
                    id: id.0,
                    name: p.name.clone(),
                    view: p.view,
                    tags: p.tags.as_ref().map(|t| t.iter().map(|t| t.0).collect()),
                })
                .collect(),
            location: (!self.location().is_default()).then(|| {
                let l = self.location();
                LocationData {
                    latitude: l.latitude,
                    longitude: l.longitude,
                    north_angle: l.north_angle,
                    utc_offset: l.utc_offset,
                }
            }),
            axes: (!self.axes().is_world()).then(|| {
                let a = self.axes();
                AxesData {
                    origin: a.origin.to_array(),
                    x: a.x.to_array(),
                    y: a.y.to_array(),
                    z: a.z.to_array(),
                }
            }),
        }
    }

    /// Rebuilds a scene from plain data, keeping every id. Fails, naming the problem, when an
    /// id is used twice (by things of any kind) or is not below [`MAX_ID`], `next_id` is over
    /// it, a number is not finite, an instance's transform
    /// cannot be inverted, or the result breaks a model invariant (see [`Scene::validate`]:
    /// that includes painting with a missing material, an entity on a missing tag, a page
    /// hiding a missing tag, an invalid page view, and blank, overlong or repeated material,
    /// tag and page names).
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
        let mut pages: Vec<(PageId, Page)> = Vec::new();
        for p in &data.pages {
            check_id("scene p", p.id)?;
            if pages.iter().any(|(id, _)| id.0 == p.id) {
                return Err(format!("scene p{} appears twice", p.id));
            }
            let page = Page {
                name: p.name.clone(),
                view: p.view,
                tags: p
                    .tags
                    .as_ref()
                    .map(|t| t.iter().map(|t| TagId(*t)).collect()),
            };
            pages.push((PageId(p.id), page));
        }
        let mut scene = Scene::from_meshes(root, definitions, materials, tags, pages, data.next_id);
        if let Some(a) = &data.axes {
            scene.axes = Axes::from_parts(a.origin.into(), a.x.into(), a.y.into(), a.z.into())
                .map_err(|e| e.to_string())?;
        }
        if let Some(l) = &data.location {
            let location = Location {
                latitude: l.latitude,
                longitude: l.longitude,
                north_angle: l.north_angle,
                utc_offset: l.utc_offset,
            };
            location.check().map_err(|e| e.to_string())?;
            scene.location = location;
        }
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
                hidden: e.hidden,
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
                hidden: f.hidden,
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
                hidden: i.hidden,
            })
            .collect(),
        guides: m
            .guides()
            .map(|(id, g)| match *g {
                Guide::Line {
                    point,
                    direction,
                    tag,
                    hidden,
                } => GuideData {
                    id: id.0,
                    point: point.to_array(),
                    direction: Some(direction.to_array()),
                    tag: tag.map(|t| t.0),
                    hidden,
                },
                Guide::Point {
                    position,
                    tag,
                    hidden,
                } => GuideData {
                    id: id.0,
                    point: position.to_array(),
                    direction: None,
                    tag: tag.map(|t| t.0),
                    hidden,
                },
            })
            .collect(),
        dimensions: m
            .dimensions()
            .map(|(id, d)| dimension_data(id, d))
            .collect(),
        notes: m.notes().map(|(id, n)| note_data(id, n)).collect(),
        sections: m
            .sections()
            .map(|(id, p)| SectionData {
                id: id.0,
                point: p.point.to_array(),
                normal: p.normal.to_array(),
                tag: p.tag.map(|t| t.0),
            })
            .collect(),
        active_section: m.active_section().map(|id| id.0),
    }
}

fn anchor_data(a: &Anchor) -> AnchorData {
    match a {
        Anchor::Vertex(v) => AnchorData::Vertex(v.0),
        Anchor::Point(p) => AnchorData::Point(p.to_array()),
    }
}

fn dimension_data(id: DimensionId, d: &Dimension) -> DimensionData {
    match *d {
        Dimension::Linear { a, b, offset, tag } => DimensionData::Linear {
            id: id.0,
            a: anchor_data(&a),
            b: anchor_data(&b),
            offset: offset.to_array(),
            tag: tag.map(|t| t.0),
        },
        Dimension::Radial {
            curve,
            diameter,
            leader,
            tag,
        } => DimensionData::Radial {
            id: id.0,
            curve: match curve {
                RadialAnchor::Curve(c) => RadialAnchorData::Curve(c.0),
                RadialAnchor::Fixed {
                    center,
                    normal,
                    radius,
                } => RadialAnchorData::Fixed(CircleData {
                    center: center.to_array(),
                    normal: normal.to_array(),
                    radius,
                }),
            },
            diameter,
            leader: leader.to_array(),
            tag: tag.map(|t| t.0),
        },
    }
}

fn note_data(id: NoteId, n: &Note) -> NoteData {
    match n {
        Note::Leader {
            anchor,
            position,
            text,
            tag,
        } => NoteData::Leader {
            id: id.0,
            anchor: anchor_data(anchor),
            position: position.to_array(),
            text: text.clone(),
            tag: tag.map(|t| t.0),
        },
        Note::Screen { at, text, tag } => NoteData::Screen {
            id: id.0,
            at: *at,
            text: text.clone(),
            tag: tag.map(|t| t.0),
        },
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
            hidden: e.hidden,
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
            hidden: f.hidden,
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
            hidden: i.hidden,
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
                hidden: g.hidden,
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
                    hidden: g.hidden,
                }
            }
        };
        if guides.insert(GuideId(g.id), guide).is_some() {
            return Err(format!("guide g{} appears twice", g.id));
        }
    }
    let anchor = |a: &AnchorData, what: &str| -> Result<Anchor, String> {
        Ok(match *a {
            AnchorData::Vertex(v) => {
                if !vertices.contains_key(&VertexId(v)) {
                    return Err(format!(
                        "{what} is anchored to vertex v{v}, which the mesh does not have"
                    ));
                }
                Anchor::Vertex(VertexId(v))
            }
            AnchorData::Point(p) => {
                finite(&p, what.to_string())?;
                Anchor::Point(DVec3::from(p))
            }
        })
    };
    let mut dimensions = BTreeMap::new();
    for d in &data.dimensions {
        let (id, dimension) = match d {
            DimensionData::Linear {
                id,
                a,
                b,
                offset,
                tag,
            } => {
                let what = format!("dimension r{id}");
                check_id("dimension r", *id)?;
                finite(offset, what.clone())?;
                let dimension = Dimension::Linear {
                    a: anchor(a, &what)?,
                    b: anchor(b, &what)?,
                    offset: DVec3::from(*offset),
                    tag: tag.map(TagId),
                };
                (*id, dimension)
            }
            DimensionData::Radial {
                id,
                curve,
                diameter,
                leader,
                tag,
            } => {
                let what = format!("dimension r{id}");
                check_id("dimension r", *id)?;
                finite(leader, what.clone())?;
                let curve = match curve {
                    RadialAnchorData::Curve(c) => {
                        check_id("curve c", *c)?;
                        RadialAnchor::Curve(CurveId(*c))
                    }
                    RadialAnchorData::Fixed(c) => {
                        finite(&c.center, what.clone())?;
                        finite(&c.normal, what.clone())?;
                        finite(&[c.radius], what.clone())?;
                        RadialAnchor::Fixed {
                            center: DVec3::from(c.center),
                            normal: DVec3::from(c.normal),
                            radius: c.radius,
                        }
                    }
                };
                let dimension = Dimension::Radial {
                    curve,
                    diameter: *diameter,
                    leader: DVec3::from(*leader),
                    tag: tag.map(TagId),
                };
                (*id, dimension)
            }
        };
        if dimensions.insert(DimensionId(id), dimension).is_some() {
            return Err(format!("dimension r{id} appears twice"));
        }
    }
    let mut notes = BTreeMap::new();
    for n in &data.notes {
        let (id, note) = match n {
            NoteData::Leader {
                id,
                anchor: a,
                position,
                text,
                tag,
            } => {
                let what = format!("note n{id}");
                check_id("note n", *id)?;
                finite(position, what.clone())?;
                let note = Note::Leader {
                    anchor: anchor(a, &what)?,
                    position: DVec3::from(*position),
                    text: text.clone(),
                    tag: tag.map(TagId),
                };
                (*id, note)
            }
            NoteData::Screen { id, at, text, tag } => {
                check_id("note n", *id)?;
                finite(at, format!("note n{id}"))?;
                let note = Note::Screen {
                    at: *at,
                    text: text.clone(),
                    tag: tag.map(TagId),
                };
                (*id, note)
            }
        };
        if notes.insert(NoteId(id), note).is_some() {
            return Err(format!("note n{id} appears twice"));
        }
    }
    let mut sections = BTreeMap::new();
    for p in &data.sections {
        let what = format!("section plane s{}", p.id);
        check_id("section plane s", p.id)?;
        finite(&p.point, what.clone())?;
        finite(&p.normal, what.clone())?;
        let plane = SectionPlane {
            point: DVec3::from(p.point),
            normal: DVec3::from(p.normal),
            tag: p.tag.map(TagId),
        };
        plane.check().map_err(|e| format!("{what}: {e}"))?;
        if sections.insert(SectionId(p.id), plane).is_some() {
            return Err(format!("section plane s{} appears twice", p.id));
        }
    }
    let active = match data.active_section {
        None => None,
        Some(id) if sections.contains_key(&SectionId(id)) => Some(SectionId(id)),
        Some(id) => {
            return Err(format!(
                "the active section plane s{id} is not a plane of the mesh"
            ));
        }
    };
    let ids = vertices.len()
        + edges.len()
        + faces.len()
        + instances.len()
        + guides.len()
        + dimensions.len()
        + notes.len()
        + sections.len();
    let distinct: std::collections::BTreeSet<u64> = vertices
        .keys()
        .map(|k| k.0)
        .chain(edges.keys().map(|k| k.0))
        .chain(faces.keys().map(|k| k.0))
        .chain(instances.keys().map(|k| k.0))
        .chain(guides.keys().map(|k| k.0))
        .chain(dimensions.keys().map(|k| k.0))
        .chain(notes.keys().map(|k| k.0))
        .chain(sections.keys().map(|k| k.0))
        .collect();
    if distinct.len() != ids {
        return Err("two entities of different kinds share an id".into());
    }
    Ok(
        Model::from_parts(vertices, edges, faces, instances, guides, dimensions, notes)
            .with_sections(sections, active),
    )
}
