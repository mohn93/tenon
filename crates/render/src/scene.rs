//! GPU vertex data built from a kernel model.

use bytemuck::{Pod, Zeroable};
use kernel::Model;

use crate::grid::LineVertex;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct FaceVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
}

pub const EDGE_COLOR: [f32; 4] = [0.08, 0.08, 0.08, 1.0];

/// Triangles for every face and line pairs for every edge.
#[derive(Debug, Default, PartialEq)]
pub struct ModelGeometry {
    pub faces: Vec<FaceVertex>,
    pub edges: Vec<LineVertex>,
}

pub fn model_geometry(model: &Model) -> ModelGeometry {
    let mut g = ModelGeometry::default();
    for (id, face) in model.faces() {
        let normal = face.normal.as_vec3().to_array();
        for tri in model.triangulate(id).unwrap_or_default() {
            for p in tri {
                g.faces.push(FaceVertex {
                    pos: p.as_vec3().to_array(),
                    normal,
                });
            }
        }
    }
    for (_, edge) in model.edges() {
        for v in edge.v {
            if let Some(p) = model.position(v) {
                g.edges.push(LineVertex {
                    pos: p.as_vec3().to_array(),
                    color: EDGE_COLOR,
                });
            }
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn empty_model_has_no_geometry() {
        assert_eq!(model_geometry(&Model::new()), ModelGeometry::default());
    }

    #[test]
    fn square_gives_two_triangles_and_four_edges() {
        let mut m = Model::new();
        let pts = [DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
        for i in 0..4 {
            m.add_edge(pts[i], pts[(i + 1) % 4]).unwrap();
        }
        let g = model_geometry(&m);
        assert_eq!(g.faces.len(), 6);
        assert!(g.faces.iter().all(|v| v.normal == [0.0, 0.0, 1.0]));
        assert_eq!(g.edges.len(), 8);
        assert!(g.edges.iter().all(|v| v.color == EDGE_COLOR));
    }
}
