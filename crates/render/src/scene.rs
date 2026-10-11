//! GPU vertex data built from a kernel model.

use std::collections::BTreeSet;

use bytemuck::{Pod, Zeroable};
use kernel::{EntityId, Model};

use crate::grid::LineVertex;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct FaceVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// 1 for selected faces, 0 otherwise.
    pub selected: f32,
}

pub const EDGE_COLOR: [f32; 4] = [0.08, 0.08, 0.08, 1.0];
pub const SELECTED_EDGE_COLOR: [f32; 4] = [0.10, 0.35, 0.95, 1.0];

/// Triangles for every face and line pairs for every edge.
#[derive(Debug, Default, PartialEq)]
pub struct ModelGeometry {
    pub faces: Vec<FaceVertex>,
    pub edges: Vec<LineVertex>,
}

/// Geometry for `model`, with the entities in `selection` highlighted.
pub fn model_geometry(model: &Model, selection: &BTreeSet<EntityId>) -> ModelGeometry {
    let mut g = ModelGeometry::default();
    for (id, face) in model.faces() {
        let normal = face.normal.as_vec3().to_array();
        let selected = if selection.contains(&EntityId::Face(id)) {
            1.0
        } else {
            0.0
        };
        for tri in model.triangulate(id).unwrap_or_default() {
            for p in tri {
                g.faces.push(FaceVertex {
                    pos: p.as_vec3().to_array(),
                    normal,
                    selected,
                });
            }
        }
    }
    for (id, edge) in model.edges() {
        let color = if selection.contains(&EntityId::Edge(id)) {
            SELECTED_EDGE_COLOR
        } else {
            EDGE_COLOR
        };
        for v in edge.v {
            if let Some(p) = model.position(v) {
                g.edges.push(LineVertex {
                    pos: p.as_vec3().to_array(),
                    color,
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

    fn square() -> Model {
        let mut m = Model::new();
        let pts = [DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
        m.add_polygon(&pts).unwrap();
        m
    }

    #[test]
    fn empty_model_has_no_geometry() {
        assert_eq!(
            model_geometry(&Model::new(), &BTreeSet::new()),
            ModelGeometry::default()
        );
    }

    #[test]
    fn square_gives_two_triangles_and_four_edges() {
        let g = model_geometry(&square(), &BTreeSet::new());
        assert_eq!(g.faces.len(), 6);
        assert!(
            g.faces
                .iter()
                .all(|v| v.normal == [0.0, 0.0, 1.0] && v.selected == 0.0)
        );
        assert_eq!(g.edges.len(), 8);
        assert!(g.edges.iter().all(|v| v.color == EDGE_COLOR));
    }

    #[test]
    fn selected_entities_are_marked() {
        let m = square();
        let face = m.faces().next().unwrap().0;
        let edge = m.edges().next().unwrap().0;
        let selection = BTreeSet::from([EntityId::Face(face), EntityId::Edge(edge)]);
        let g = model_geometry(&m, &selection);
        assert!(g.faces.iter().all(|v| v.selected == 1.0));
        let blue = g
            .edges
            .iter()
            .filter(|v| v.color == SELECTED_EDGE_COLOR)
            .count();
        assert_eq!(blue, 2, "one edge, two vertices");
    }
}
