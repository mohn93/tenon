//! GPU vertex data built from a kernel scene.

use std::collections::BTreeSet;

use bytemuck::{Pod, Zeroable};
use glam::{DAffine3, DVec3};
use kernel::{EntityId, Guide, InstanceId, MaterialId, MeshId, Scene};

use crate::grid::LineVertex;
use crate::guides::{FADED_GUIDE_COLOR, GUIDE_COLOR, GuideShape};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct FaceVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// 1 for selected faces, 0 otherwise.
    pub selected: f32,
    /// The colors of the front and back sides.
    pub front: [f32; 3],
    pub back: [f32; 3],
}

/// The default material's color on a face's front.
pub const DEFAULT_FRONT: [f32; 3] = [0.93, 0.93, 0.90];
/// The default material's color on a face's back.
pub const DEFAULT_BACK: [f32; 3] = [0.60, 0.65, 0.76];

/// The color a side shows: its material's (sRGB, as the targets store it), or `default`.
fn side_color(scene: &Scene, material: Option<MaterialId>, default: [f32; 3]) -> [f32; 3] {
    material
        .and_then(|m| scene.material(m))
        .map_or(default, |m| m.color.map(|c| f32::from(c) / 255.0))
}

pub const EDGE_COLOR: [f32; 4] = [0.08, 0.08, 0.08, 1.0];
pub const SELECTED_EDGE_COLOR: [f32; 4] = [0.10, 0.35, 0.95, 1.0];

/// Edges outside the open instance, while one is open.
pub const FADED_EDGE_COLOR: [f32; 4] = [0.62, 0.62, 0.62, 1.0];
/// The box around the open instance.
pub const CONTEXT_BOX_COLOR: [f32; 4] = [0.45, 0.45, 0.45, 1.0];

/// Triangles for every face, line pairs for every edge, and the guides (drawn per frame, see
/// `guides::guide_lines`).
#[derive(Debug, Default, PartialEq)]
pub struct ModelGeometry {
    pub faces: Vec<FaceVertex>,
    pub edges: Vec<LineVertex>,
    pub guides: Vec<GuideShape>,
}

/// Geometry for the whole scene in root coordinates, every instance placed by its transform.
/// `context` is the path of open instances (see `session::Document::context`) and `selection`
/// holds ids of the open context: selected entities there are highlighted, and a selected
/// instance highlights everything it places. While an instance is open, edges and guides
/// outside it are faded and its bounding box is outlined. Each face side gets its material's
/// color, or else that of the nearest painted instance placing it, or else the default.
pub fn scene_geometry(
    scene: &Scene,
    context: &[InstanceId],
    selection: &BTreeSet<EntityId>,
) -> ModelGeometry {
    let mut g = ModelGeometry::default();
    let mut walk = Walk {
        scene,
        context,
        selection,
        path: Vec::new(),
        g: &mut g,
    };
    walk.mesh(MeshId::Root, DAffine3::IDENTITY, false, None);
    if !context.is_empty()
        && let Some((mesh, t)) = scene.resolve(context)
        && let Some([lo, hi]) = scene.bounds(mesh)
    {
        let corner = |k: usize| {
            let c = DVec3::new(
                if k & 1 == 0 { lo.x } else { hi.x },
                if k & 2 == 0 { lo.y } else { hi.y },
                if k & 4 == 0 { lo.z } else { hi.z },
            );
            t.transform_point3(c).as_vec3().to_array()
        };
        for (a, b) in BOX_EDGES {
            for k in [a, b] {
                g.edges.push(LineVertex {
                    pos: corner(k),
                    color: CONTEXT_BOX_COLOR,
                });
            }
        }
    }
    g
}

/// Corner pairs of a box's edges; corner k has bit 0 for x, bit 1 for y, bit 2 for z.
const BOX_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

struct Walk<'a> {
    scene: &'a Scene,
    context: &'a [InstanceId],
    selection: &'a BTreeSet<EntityId>,
    /// Instances from the root down to the mesh being walked.
    path: Vec<InstanceId>,
    g: &'a mut ModelGeometry,
}

impl Walk<'_> {
    /// Adds a mesh placed by `t`; `lit` highlights all of it (it is inside a selected instance)
    /// and `inherited` is the material of the nearest painted instance placing it.
    fn mesh(&mut self, id: MeshId, t: DAffine3, lit: bool, inherited: Option<MaterialId>) {
        let Some(model) = self.scene.mesh(id) else {
            return;
        };
        // Selection ids refer to the open context's placement only, not to other placements
        // of the same component.
        let open = self.path == self.context;
        let inside = self.path.starts_with(self.context);
        let selected = |e: EntityId| lit || (open && self.selection.contains(&e));
        // Normals follow the inverse transpose, right for any linear map (placements turn and
        // move, so it is the rotation itself for now).
        let normal_map = t.matrix3.inverse().transpose();
        for (fid, face) in model.faces() {
            let normal = (normal_map * face.normal).normalize().as_vec3().to_array();
            let flag = if selected(EntityId::Face(fid)) {
                1.0
            } else {
                0.0
            };
            let front = side_color(self.scene, face.front.or(inherited), DEFAULT_FRONT);
            let back = side_color(self.scene, face.back.or(inherited), DEFAULT_BACK);
            for tri in model.triangulate(fid).unwrap_or_default() {
                for p in tri {
                    self.g.faces.push(FaceVertex {
                        pos: t.transform_point3(p).as_vec3().to_array(),
                        normal,
                        selected: flag,
                        front,
                        back,
                    });
                }
            }
        }
        // Soft edges are hidden, selected or not: the faces around them read as one surface.
        for (eid, edge) in model.edges().filter(|(_, e)| !e.soft) {
            let color = if selected(EntityId::Edge(eid)) {
                SELECTED_EDGE_COLOR
            } else if inside {
                EDGE_COLOR
            } else {
                FADED_EDGE_COLOR
            };
            for v in edge.v {
                if let Some(p) = model.position(v) {
                    self.g.edges.push(LineVertex {
                        pos: t.transform_point3(p).as_vec3().to_array(),
                        color,
                    });
                }
            }
        }
        for (gid, guide) in model.guides() {
            let color = if selected(EntityId::Guide(gid)) {
                SELECTED_EDGE_COLOR
            } else if inside {
                GUIDE_COLOR
            } else {
                FADED_GUIDE_COLOR
            };
            let (point, direction) = match guide.transformed(t) {
                Guide::Line { point, direction } => (point, Some(direction)),
                Guide::Point { position } => (position, None),
            };
            self.g.guides.push(GuideShape {
                point,
                direction,
                color,
            });
        }
        for (iid, instance) in model.instances() {
            let lit = selected(EntityId::Instance(iid));
            self.path.push(iid);
            self.mesh(
                MeshId::Definition(instance.definition),
                t * instance.transform,
                lit,
                instance.material.or(inherited),
            );
            self.path.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::{DefinitionKind, Model};

    fn square() -> Model {
        let mut m = Model::new();
        let pts = [DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
        m.add_polygon(&pts).unwrap();
        m
    }

    fn geometry(m: Model, selection: &BTreeSet<EntityId>) -> ModelGeometry {
        scene_geometry(&Scene::from_root(m), &[], selection)
    }

    /// The square made a component, copied 3 m along x. Returns the scene and both instances.
    fn two_squares() -> (Scene, InstanceId, InstanceId) {
        let mut s = Scene::from_root(square());
        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
        let (first, _) = s
            .make_group(
                MeshId::Root,
                &edges,
                &faces,
                &[],
                DefinitionKind::Component,
                "Tile",
            )
            .unwrap();
        s.copy_instances(MeshId::Root, &[first], DVec3::new(3.0, 0.0, 0.0), 1)
            .unwrap();
        let second = s
            .root()
            .instances()
            .map(|(id, _)| id)
            .find(|id| *id != first)
            .unwrap();
        (s, first, second)
    }

    #[test]
    fn empty_model_has_no_geometry() {
        assert_eq!(
            geometry(Model::new(), &BTreeSet::new()),
            ModelGeometry::default()
        );
    }

    #[test]
    fn square_gives_two_triangles_and_four_edges() {
        let g = geometry(square(), &BTreeSet::new());
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
    fn soft_edges_are_not_drawn() {
        // A triangle pulled up: three side faces. Its sides made a curve first, so the
        // swept edges between them are soft.
        let mut m = Model::new();
        let pts = [DVec3::ZERO, DVec3::X, DVec3::new(0.5, 0.8, 0.0)];
        let c = m.add_curve(&pts, true).unwrap();
        let face = c
            .summary()
            .created
            .iter()
            .find_map(|id| match id {
                EntityId::Face(f) => Some(*f),
                _ => None,
            })
            .unwrap();
        m.push_pull(face, 1.0).unwrap();
        assert_eq!(m.edges().filter(|(_, e)| e.soft).count(), 3);
        let all: BTreeSet<EntityId> = m.edges().map(|(e, _)| EntityId::Edge(e)).collect();
        let g = geometry(m, &all);
        assert_eq!(g.edges.len(), 2 * 6, "the two rims, selected or not");
    }

    #[test]
    fn faces_take_their_material_colors_or_the_nearest_painted_instance() {
        let (mut s, first, second) = two_squares();
        let (red, _) = s.add_material("Red", [255, 0, 0]).unwrap();
        let (blue, _) = s.add_material("Blue", [0, 0, 255]).unwrap();
        let g = scene_geometry(&s, &[], &BTreeSet::new());
        assert!(
            g.faces
                .iter()
                .all(|v| v.front == DEFAULT_FRONT && v.back == DEFAULT_BACK)
        );
        // The first square's instance is blue; the component's face is red on its back only.
        s.paint(MeshId::Root, &[], &[first], kernel::Side::Front, Some(blue))
            .unwrap();
        let d = s.root().instance(first).unwrap().definition;
        let face = s.definition(d).unwrap().mesh.faces().next().unwrap().0;
        s.paint(
            MeshId::Definition(d),
            &[face],
            &[],
            kernel::Side::Back,
            Some(red),
        )
        .unwrap();
        let g = scene_geometry(&s, &[], &BTreeSet::new());
        let (one, two): (Vec<&FaceVertex>, Vec<&FaceVertex>) =
            g.faces.iter().partition(|v| v.pos[0] <= 1.0);
        assert!(
            one.iter()
                .all(|v| v.front == [0.0, 0.0, 1.0] && v.back == [1.0, 0.0, 0.0])
        );
        assert!(
            two.iter()
                .all(|v| v.front == DEFAULT_FRONT && v.back == [1.0, 0.0, 0.0]),
            "the second placement shares the face but not the instance's blue"
        );
        let _ = second;
    }

    #[test]
    fn selected_entities_are_marked() {
        let m = square();
        let face = m.faces().next().unwrap().0;
        let edge = m.edges().next().unwrap().0;
        let selection = BTreeSet::from([EntityId::Face(face), EntityId::Edge(edge)]);
        let g = geometry(m, &selection);
        assert!(g.faces.iter().all(|v| v.selected == 1.0));
        let blue = g
            .edges
            .iter()
            .filter(|v| v.color == SELECTED_EDGE_COLOR)
            .count();
        assert_eq!(blue, 2, "one edge, two vertices");
    }

    #[test]
    fn every_instance_is_drawn_where_it_is_placed() {
        let (s, _, _) = two_squares();
        let g = scene_geometry(&s, &[], &BTreeSet::new());
        assert_eq!(g.faces.len(), 12);
        assert_eq!(g.edges.len(), 16);
        let max_x = g.faces.iter().map(|v| v.pos[0]).fold(f32::MIN, f32::max);
        assert_eq!(max_x, 4.0);
    }

    #[test]
    fn a_turned_instance_is_drawn_turned_with_turned_normals() {
        let (mut s, _, second) = two_squares();
        // A third of a quarter turn about X through the second square's corner (3, 0, 0).
        let turn = DAffine3::from_translation(DVec3::new(3.0, 0.0, 0.0))
            * DAffine3::from_rotation_x(30f64.to_radians())
            * DAffine3::from_translation(DVec3::new(-3.0, 0.0, 0.0));
        s.edit(MeshId::Root, |m| Ok(m.transform_instances(&[second], turn)))
            .unwrap();
        let g = scene_geometry(&s, &[second], &BTreeSet::new());
        let (sin, cos) = 30f32.to_radians().sin_cos();
        let turned: Vec<_> = g.faces.iter().filter(|v| v.pos[0] >= 3.0).collect();
        assert_eq!(turned.len(), 6);
        for v in turned {
            let n = v.normal;
            assert!((n[0]).abs() < 1e-6 && (n[1] + sin).abs() < 1e-6 && (n[2] - cos).abs() < 1e-6);
        }
        assert!(
            g.faces.iter().any(|v| (v.pos[2] - sin).abs() < 1e-6),
            "the far side is lifted"
        );
        let lifted_box = g
            .edges
            .iter()
            .filter(|v| v.color == CONTEXT_BOX_COLOR)
            .any(|v| (v.pos[2] - sin).abs() < 1e-6);
        assert!(lifted_box, "the open instance's box turns with it");
    }

    #[test]
    fn guides_are_placed_and_lit_when_selected() {
        let (mut s, first, _) = two_squares();
        let d = s.root().instance(first).unwrap().definition;
        let inner = s
            .edit(MeshId::Definition(d), |m| {
                m.add_guide(Guide::Point {
                    position: DVec3::new(0.5, 0.5, 0.0),
                })
            })
            .unwrap()
            .summary()
            .created[0];
        let root = s
            .edit(MeshId::Root, |m| {
                m.add_guide(Guide::Line {
                    point: DVec3::ZERO,
                    direction: DVec3::Y,
                })
            })
            .unwrap()
            .summary()
            .created[0];
        let g = scene_geometry(&s, &[], &BTreeSet::from([root]));
        // The component is placed twice, so its guide point is drawn twice.
        assert_eq!(g.guides.len(), 3);
        let line = g.guides.iter().find(|s| s.direction.is_some()).unwrap();
        assert_eq!(line.color, SELECTED_EDGE_COLOR);
        let mut points: Vec<f64> = g
            .guides
            .iter()
            .filter(|s| s.direction.is_none())
            .map(|s| s.point.x)
            .collect();
        points.sort_by(f64::total_cmp);
        assert_eq!(points, vec![0.5, 3.5]);
        // Inside the first square, the root's guide line is faded.
        let open = scene_geometry(&s, &[first], &BTreeSet::from([inner]));
        let faded = open.guides.iter().filter(|s| s.color == FADED_GUIDE_COLOR);
        assert_eq!(
            faded.count(),
            2,
            "the root's line and the other square's point"
        );
        assert!(open.guides.iter().any(|s| s.color == SELECTED_EDGE_COLOR));
    }

    #[test]
    fn a_selected_instance_lights_up_what_it_places() {
        let (s, first, _) = two_squares();
        let selection = BTreeSet::from([EntityId::Instance(first)]);
        let g = scene_geometry(&s, &[], &selection);
        let lit: Vec<_> = g.faces.iter().filter(|v| v.selected == 1.0).collect();
        assert_eq!(lit.len(), 6);
        assert!(lit.iter().all(|v| v.pos[0] <= 1.0), "only the first square");
        let blue = g
            .edges
            .iter()
            .filter(|v| v.color == SELECTED_EDGE_COLOR)
            .count();
        assert_eq!(blue, 8);
    }

    #[test]
    fn an_open_instance_fades_the_rest_and_is_boxed() {
        let (s, first, _) = two_squares();
        let d = s.root().instance(first).unwrap().definition;
        let face = s.definition(d).unwrap().mesh.faces().next().unwrap().0;
        let selection = BTreeSet::from([EntityId::Face(face)]);
        let g = scene_geometry(&s, &[first], &selection);
        let lit: Vec<_> = g.faces.iter().filter(|v| v.selected == 1.0).collect();
        assert_eq!(lit.len(), 6, "the face is lit in the open placement only");
        assert!(lit.iter().all(|v| v.pos[0] <= 1.0));
        let count = |c: [f32; 4]| g.edges.iter().filter(|v| v.color == c).count();
        assert_eq!(count(EDGE_COLOR), 8, "the open square's edges");
        assert_eq!(count(FADED_EDGE_COLOR), 8, "the other square's edges");
        assert_eq!(count(CONTEXT_BOX_COLOR), 24, "twelve box edges");
    }
}
