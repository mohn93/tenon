//! Solids: whether a mesh is a closed, consistently oriented 2-manifold, and its volume.

use std::collections::BTreeMap;

use crate::ids::{EdgeId, VertexId};
use crate::model::{Instance, Model};
use crate::scene::Scene;

/// Why `m` is not closed (every edge bounds exactly two faces, one traversing it each way, and
/// no edge bounds no face), or None when it is. A mesh without faces is not closed.
pub fn open_reason(m: &Model) -> Option<String> {
    let mut sides: BTreeMap<(VertexId, VertexId), u32> = BTreeMap::new();
    for (_, f) in m.faces() {
        for l in std::iter::once(&f.outer).chain(&f.holes) {
            for i in 0..l.len() {
                *sides.entry((l[i], l[(i + 1) % l.len()])).or_default() += 1;
            }
        }
    }
    if sides.is_empty() {
        return Some("it has no faces".into());
    }
    let mut by_ends: BTreeMap<(VertexId, VertexId), EdgeId> = BTreeMap::new();
    for (id, e) in m.edges() {
        let [a, b] = e.v;
        by_ends.insert((a.min(b), a.max(b)), id);
    }
    let name = |a: VertexId, b: VertexId| match by_ends.get(&(a.min(b), a.max(b))) {
        Some(e) => format!("edge e{}", e.0),
        None => format!("side v{}-v{}", a.0, b.0),
    };
    for (id, e) in m.edges() {
        let [a, b] = e.v;
        let n = sides.get(&(a, b)).unwrap_or(&0) + sides.get(&(b, a)).unwrap_or(&0);
        if n == 0 {
            return Some(format!("edge e{} bounds no face", id.0));
        }
    }
    for (&(a, b), &n) in &sides {
        let back = sides.get(&(b, a)).copied().unwrap_or(0);
        let faces = n + back;
        if faces != 2 {
            let s = if faces == 1 { "" } else { "s" };
            return Some(format!("{} bounds {faces} face{s}", name(a, b)));
        }
        if n == 2 {
            return Some(format!(
                "{} is traversed twice the same way (faces disagree on orientation)",
                name(a, b)
            ));
        }
    }
    None
}

/// The signed volume enclosed by the faces of `m`, in its own coordinates: positive when the
/// face normals point outward. Faces with holes and several loops are triangulated. Only
/// meaningful for a closed mesh.
pub fn signed_volume(m: &Model) -> f64 {
    let mut six = 0.0;
    for (id, _) in m.faces() {
        for [a, b, c] in m.triangulate(id).unwrap_or_default() {
            six += a.dot(b.cross(c));
        }
    }
    six / 6.0
}

/// The volume of `instance` in the units of the context holding it (its definition's volume
/// times the absolute determinant of the placement), or why it is not a solid: its own
/// geometry must be closed (faces pointing in or out) and enclose a volume, with no nested
/// instances. The volume is positive either way.
pub fn instance_volume(scene: &Scene, instance: &Instance) -> Result<f64, String> {
    let d = scene
        .definition(instance.definition)
        .ok_or_else(|| "its definition is missing".to_string())?;
    if d.mesh.instances().next().is_some() {
        return Err("it contains groups or components".into());
    }
    if let Some(reason) = open_reason(&d.mesh) {
        return Err(reason);
    }
    // A closed mesh with every face pointing inward is still a solid; it has the same volume.
    let volume = signed_volume(&d.mesh).abs();
    if volume <= 1e-12 {
        return Err("it encloses no volume".into());
    }
    Ok(volume * instance.transform.matrix3.determinant().abs())
}

#[cfg(test)]
mod tests {
    use glam::{DAffine3, DVec3};

    use super::*;
    use crate::primitives::Solid;
    use crate::scene::{DefinitionKind, MeshId};

    fn cube() -> Model {
        Solid::cuboid(DVec3::new(2.0, 3.0, 4.0)).unwrap().to_model()
    }

    #[test]
    fn a_box_is_closed_with_its_volume() {
        let m = cube();
        assert_eq!(open_reason(&m), None);
        assert!((signed_volume(&m) - 24.0).abs() < 1e-9);
    }

    #[test]
    fn a_tube_with_hole_faces_has_the_ring_volume() {
        let m = Solid::tube(2.0, 1.0, 3.0, 32).unwrap().to_model();
        assert_eq!(open_reason(&m), None);
        let ring = |r: f64| 16.0 * r * r * (std::f64::consts::TAU / 32.0).sin();
        assert!((signed_volume(&m) - (ring(2.0) - ring(1.0)) * 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_missing_face_names_the_open_edge() {
        let mut m = cube();
        let f = m.faces().next().unwrap().0;
        m.erase(&[], &[f]);
        let reason = open_reason(&m).unwrap();
        assert!(reason.contains("bounds 1 face"), "{reason}");
        assert!(reason.starts_with("edge e"), "{reason}");
    }

    #[test]
    fn a_turned_face_disagrees_on_orientation() {
        let mut m = cube();
        let f = m.faces().next().unwrap().0;
        m.reverse_faces(&[f]);
        let reason = open_reason(&m).unwrap();
        assert!(reason.contains("orientation"), "{reason}");
    }

    #[test]
    fn a_stray_edge_is_named() {
        let mut m = cube();
        m.add_edge(DVec3::splat(10.0), DVec3::splat(11.0)).unwrap();
        let reason = open_reason(&m).unwrap();
        assert!(reason.contains("bounds no face"), "{reason}");
    }

    #[test]
    fn an_empty_mesh_is_not_closed() {
        assert!(open_reason(&Model::new()).is_some());
    }

    #[test]
    fn inward_faces_have_negative_volume() {
        let mut m = cube();
        let all: Vec<_> = m.faces().map(|(id, _)| id).collect();
        m.reverse_faces(&all);
        assert_eq!(open_reason(&m), None);
        assert!((signed_volume(&m) + 24.0).abs() < 1e-9);
    }

    fn placed(scene: &mut Scene, solid: &Solid, place: DAffine3) -> Instance {
        let (id, _) = scene
            .add_solid(MeshId::Root, solid, DefinitionKind::Group, "S", place, None)
            .unwrap();
        scene
            .mesh(MeshId::Root)
            .unwrap()
            .instance(id)
            .unwrap()
            .clone()
    }

    #[test]
    fn an_inward_solid_counts_with_a_positive_volume() {
        let mut scene = Scene::new();
        let (id, _) = scene
            .add_solid(
                MeshId::Root,
                &Solid::cuboid(DVec3::ONE).unwrap(),
                DefinitionKind::Group,
                "S",
                DAffine3::IDENTITY,
                None,
            )
            .unwrap();
        let inst = scene
            .mesh(MeshId::Root)
            .unwrap()
            .instance(id)
            .unwrap()
            .clone();
        let all: Vec<_> = scene
            .definition(inst.definition)
            .unwrap()
            .mesh
            .faces()
            .map(|(f, _)| f)
            .collect();
        scene
            .edit(MeshId::Definition(inst.definition), |m| {
                Ok(m.reverse_faces(&all).0)
            })
            .unwrap();
        assert!((instance_volume(&scene, &inst).unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn instance_volume_scales_with_the_placement() {
        let mut scene = Scene::new();
        let unit = Solid::cuboid(DVec3::ONE).unwrap();
        let big = placed(
            &mut scene,
            &unit,
            DAffine3::from_scale(DVec3::new(2.0, 3.0, 4.0)),
        );
        assert!((instance_volume(&scene, &big).unwrap() - 24.0).abs() < 1e-9);
        let mirror = placed(
            &mut scene,
            &unit,
            DAffine3::from_scale(DVec3::new(-2.0, 1.0, 1.0)),
        );
        assert!((instance_volume(&scene, &mirror).unwrap() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn an_instance_with_nested_instances_is_not_a_solid() {
        let mut scene = Scene::new();
        let unit = Solid::cuboid(DVec3::ONE).unwrap();
        let inner = placed(&mut scene, &unit, DAffine3::IDENTITY);
        let outer = placed(&mut scene, &unit, DAffine3::IDENTITY);
        scene
            .place_instance(
                MeshId::Definition(outer.definition),
                inner.definition,
                DAffine3::IDENTITY,
            )
            .unwrap();
        let err = instance_volume(&scene, &outer).unwrap_err();
        assert!(err.contains("contains"), "{err}");
    }
}
