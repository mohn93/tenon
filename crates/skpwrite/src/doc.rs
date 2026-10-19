//! What the writer takes: a whole document, in inches, with no ties to the modelling
//! session. Indices are into the lists of the same document (materials) or the same
//! geometry (vertices).

/// A whole document: its materials, its component and group definitions, and the root.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Doc {
    pub materials: Vec<Material>,
    pub definitions: Vec<Definition>,
    pub root: Geometry,
}

/// A material: a colour, an opacity and an optional texture.
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub name: String,
    pub rgba: [u8; 4],
    /// 1.0 is opaque.
    pub opacity: f64,
    pub texture: Option<Texture>,
}

/// A texture image as stored, and the size one tile of it covers.
#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    /// PNG or JPEG, as stored.
    pub bytes: Vec<u8>,
    /// Inches.
    pub width: f64,
    /// Inches.
    pub height: f64,
}

/// A component or group definition.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Definition {
    pub name: String,
    pub is_group: bool,
    pub geometry: Geometry,
}

/// The entities of the root or of one definition, in its own coordinates (inches).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Geometry {
    pub vertices: Vec<[f64; 3]>,
    /// Every edge, including those faces use. The writer finds a face's edges here by
    /// their vertex pair; an edge no face uses is written as a loose edge.
    pub edges: Vec<Edge>,
    pub faces: Vec<Face>,
    pub instances: Vec<Instance>,
    pub guides: Vec<Guide>,
}

/// An edge between two vertices of the same geometry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Edge {
    pub a: u32,
    pub b: u32,
    pub soft: bool,
    pub smooth: bool,
    pub hidden: bool,
}

/// One texture pin: a point on the face and the texture coordinate, in tiles, it takes.
pub type UvPin = ([f64; 3], [f64; 2]);

/// A planar face.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Face {
    /// Vertex rings: the outer loop first, counter-clockwise around the face's normal;
    /// then the holes, clockwise.
    pub loops: Vec<Vec<u32>>,
    /// Index into `Doc::materials`.
    pub front: Option<usize>,
    /// Index into `Doc::materials`.
    pub back: Option<usize>,
    /// Three pins fixing the front texture's placement; `None` for the default projection.
    pub front_uv: Option<[UvPin; 3]>,
    /// Three pins fixing the back texture's placement.
    pub back_uv: Option<[UvPin; 3]>,
}

/// A placement of a definition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instance {
    /// Index into `Doc::definitions`.
    pub definition: usize,
    /// Row-major, as SketchUp stores it.
    pub rotation: [[f64; 3]; 3],
    /// Inches.
    pub translation: [f64; 3],
    /// Index into `Doc::materials`.
    pub material: Option<usize>,
}

/// A guide line (infinite) or a guide point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Guide {
    Line {
        point: [f64; 3],
        direction: [f64; 3],
    },
    Point {
        position: [f64; 3],
    },
}
