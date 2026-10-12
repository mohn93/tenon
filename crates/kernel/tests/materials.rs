//! Materials: adding, changing and deleting them, painting faces and instances, and how paint
//! follows geometry through splits, merges, push/pull, copies, groups, explodes and files.

use glam::DVec3;
use kernel::{
    DefinitionKind, EntityId, FaceId, InstanceId, KernelError, MaterialData, MaterialId, MeshId,
    Model, Scene, SceneData, Side,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

const RED: [u8; 3] = [200, 30, 30];
const BLUE: [u8; 3] = [30, 30, 200];

/// A 2 x 2 square on the ground (facing up), in a scene of its own.
fn square_scene() -> (Scene, FaceId) {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 2.0, 0.0),
        p(0.0, 2.0, 0.0),
    ])
    .unwrap();
    let face = m.faces().next().unwrap().0;
    (Scene::from_root(m), face)
}

fn red(s: &mut Scene) -> MaterialId {
    s.add_material("Red", RED).unwrap().0
}

#[test]
fn materials_are_added_changed_and_deleted_as_exact_undo_steps() {
    let (mut s, _) = square_scene();
    let empty = s.clone();
    let (id, added) = s.add_material("Brick", RED).unwrap();
    assert_eq!(s.material(id).unwrap().name, "Brick");
    assert_eq!(s.material(id).unwrap().color, RED);
    assert_eq!(added.summary().materials, vec![id]);
    assert!(added.summary().created.is_empty());
    // Material ids come from the entity id counter: never one an entity has.
    assert!(s.root().vertices().all(|(v, _)| v.0 != id.0));

    let before = s.clone();
    let changed = s
        .change_material(id, Some("Old brick"), Some(BLUE))
        .unwrap();
    assert_eq!(s.material(id).unwrap().name, "Old brick");
    assert_eq!(s.material(id).unwrap().color, BLUE);
    assert_eq!(changed.summary().materials, vec![id]);
    assert!(s.change_material(id, None, Some(BLUE)).unwrap().is_empty());
    s.apply(&changed.inverse());
    assert_eq!(s, before);

    let deleted = s.delete_material(id).unwrap();
    assert!(s.material(id).is_none());
    s.apply(&deleted.inverse());
    assert_eq!(s, before);
    s.apply(&added.inverse());
    assert_eq!(s, empty);
}

#[test]
fn material_names_must_be_present_short_and_distinct() {
    let (mut s, _) = square_scene();
    let id = red(&mut s);
    for bad in ["", "   ", "Red"] {
        assert!(
            matches!(s.add_material(bad, BLUE), Err(KernelError::MaterialName(_))),
            "{bad:?} accepted"
        );
    }
    let long = "x".repeat(kernel::MAX_MATERIAL_NAME + 1);
    assert!(matches!(
        s.add_material(&long, BLUE),
        Err(KernelError::MaterialName(_))
    ));
    let blue = s.add_material("Blue", BLUE).unwrap().0;
    assert!(matches!(
        s.change_material(blue, Some("Red"), None),
        Err(KernelError::MaterialName(_))
    ));
    // Keeping its own name is fine.
    assert!(s.change_material(id, Some("Red"), None).unwrap().is_empty());
    assert_eq!(s.free_material_name("Red"), "Red 2");
    assert_eq!(s.free_material_name("Green"), "Green");
    assert_eq!(
        s.change_material(MaterialId(999), None, None),
        Err(KernelError::MaterialNotFound)
    );
    assert_eq!(
        s.delete_material(MaterialId(999)),
        Err(KernelError::MaterialNotFound)
    );
}

#[test]
fn painting_faces_and_instances_undoes_exactly() {
    let (mut s, face) = square_scene();
    let id = red(&mut s);
    let before = s.clone();
    let front = s
        .paint(MeshId::Root, &[face], &[], Side::Front, Some(id))
        .unwrap();
    assert_eq!(s.root().face(face).unwrap().paint(), [Some(id), None]);
    assert_eq!(front.summary().modified, vec![EntityId::Face(face)]);
    let back = s
        .paint(MeshId::Root, &[face], &[], Side::Back, Some(id))
        .unwrap();
    assert_eq!(s.root().face(face).unwrap().paint(), [Some(id), Some(id)]);
    assert!(
        s.paint(MeshId::Root, &[face], &[], Side::Both, Some(id))
            .unwrap()
            .is_empty(),
        "already painted"
    );
    let cleared = s
        .paint(MeshId::Root, &[face], &[], Side::Both, None)
        .unwrap();
    assert_eq!(s.root().face(face).unwrap().paint(), [None, None]);
    s.apply(&cleared.inverse());
    s.apply(&back.inverse());
    s.apply(&front.inverse());
    assert_eq!(s, before);
    assert_eq!(
        s.paint(
            MeshId::Root,
            &[face],
            &[],
            Side::Front,
            Some(MaterialId(999))
        ),
        Err(KernelError::MaterialNotFound)
    );

    let edges: Vec<_> = s.root().edges().map(|(e, _)| e).collect();
    let (group, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &[face],
            &[],
            DefinitionKind::Group,
            "",
        )
        .unwrap();
    let painted = s
        .paint(MeshId::Root, &[], &[group], Side::Back, Some(id))
        .unwrap();
    assert_eq!(s.root().instance(group).unwrap().material, Some(id));
    assert_eq!(painted.summary().modified, vec![EntityId::Instance(group)]);
    s.validate().unwrap();
}

#[test]
fn deleting_a_material_unpaints_everything_painted_with_it() {
    let (mut s, face) = square_scene();
    let id = red(&mut s);
    let blue = s.add_material("Blue", BLUE).unwrap().0;
    s.paint(MeshId::Root, &[face], &[], Side::Front, Some(id))
        .unwrap();
    s.paint(MeshId::Root, &[face], &[], Side::Back, Some(blue))
        .unwrap();
    let edges: Vec<_> = s.root().edges().map(|(e, _)| e).collect();
    let (group, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &[face],
            &[],
            DefinitionKind::Group,
            "",
        )
        .unwrap();
    s.paint(MeshId::Root, &[], &[group], Side::Front, Some(id))
        .unwrap();
    let before = s.clone();
    let change = s.delete_material(id).unwrap();
    s.validate().unwrap();
    assert_eq!(s.root().instance(group).unwrap().material, None);
    let d = s.root().instance(group).unwrap().definition;
    let inner = s.definition(d).unwrap().mesh.faces().next().unwrap().1;
    assert_eq!(
        inner.paint(),
        [None, Some(blue)],
        "the other side keeps blue"
    );
    assert!(change.summary().materials == vec![id]);
    s.apply(&change.inverse());
    assert_eq!(s, before);
    // Redo: the same change again unpaints once more.
    let after = {
        let mut a = before.clone();
        a.delete_material(id).unwrap();
        a
    };
    s.apply(&change);
    assert_eq!(s, after);
    assert!(s.material(id).is_none());
    assert_eq!(s.root().instance(group).unwrap().material, None);
    s.validate().unwrap();
}

#[test]
fn pieces_of_a_split_face_keep_its_paint_and_a_merge_keeps_it_too() {
    let (mut s, face) = square_scene();
    let id = red(&mut s);
    s.paint(MeshId::Root, &[face], &[], Side::Front, Some(id))
        .unwrap();
    let split = s
        .edit(MeshId::Root, |m| {
            m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 2.0, 0.0))
        })
        .unwrap();
    assert_eq!(s.root().faces().count(), 2);
    assert!(s.root().faces().all(|(_, f)| f.paint() == [Some(id), None]));
    let seam = split
        .summary()
        .created
        .iter()
        .find_map(|e| match e {
            EntityId::Edge(e)
                if s.root().edge(*e).is_some_and(|e| {
                    e.v.iter().all(|v| s.root().position(*v).unwrap().x == 1.0)
                }) =>
            {
                Some(*e)
            }
            _ => None,
        })
        .unwrap();
    s.edit(MeshId::Root, |m| Ok(m.erase(&[seam], &[]))).unwrap();
    assert_eq!(s.root().faces().count(), 1);
    assert_eq!(s.root().faces().next().unwrap().1.paint(), [Some(id), None]);
    s.validate().unwrap();
}

#[test]
fn a_face_drawn_in_an_opening_is_not_painted() {
    // A loop drawn on a painted face splits it: the inner piece keeps the paint. Erasing that
    // piece and drawing inside the opening gives a new, unpainted face.
    let (mut s, face) = square_scene();
    let id = red(&mut s);
    s.paint(MeshId::Root, &[face], &[], Side::Front, Some(id))
        .unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&[
            p(0.5, 0.5, 0.0),
            p(1.5, 0.5, 0.0),
            p(1.5, 1.5, 0.0),
            p(0.5, 1.5, 0.0),
        ])
    })
    .unwrap();
    assert!(s.root().faces().all(|(_, f)| f.front == Some(id)));
    let inner = s
        .root()
        .faces()
        .find(|(_, f)| f.holes.is_empty())
        .unwrap()
        .0;
    s.edit(MeshId::Root, |m| Ok(m.erase(&[], &[inner])))
        .unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&[
            p(0.75, 0.75, 0.0),
            p(1.25, 0.75, 0.0),
            p(1.25, 1.25, 0.0),
            p(0.75, 1.25, 0.0),
        ])
    })
    .unwrap();
    let fresh = s
        .root()
        .faces()
        .find(|(_, f)| f.holes.is_empty())
        .unwrap()
        .1;
    assert_eq!(fresh.paint(), [None, None]);
}

#[test]
fn pulled_faces_give_their_paint_to_the_cap_and_sides() {
    let (mut s, face) = square_scene();
    let id = red(&mut s);
    let blue = s.add_material("Blue", BLUE).unwrap().0;
    s.paint(MeshId::Root, &[face], &[], Side::Front, Some(id))
        .unwrap();
    s.paint(MeshId::Root, &[face], &[], Side::Back, Some(blue))
        .unwrap();
    s.edit(MeshId::Root, |m| m.push_pull(face, 1.0)).unwrap();
    s.validate().unwrap();
    let m = s.root();
    assert_eq!(m.faces().count(), 6);
    // The base turned over to face down: each material stays on its side, so blue (what was
    // below) is now its front.
    let base = m.face(face).unwrap();
    assert_eq!(base.normal, -DVec3::Z);
    assert_eq!(base.paint(), [Some(blue), Some(id)]);
    for (fid, f) in m.faces().filter(|(f, _)| *f != face) {
        assert_eq!(f.paint(), [Some(id), Some(blue)], "{fid:?}");
    }

    // A recess pushed into the painted top: its floor and walls take the top's paint.
    let top = m.faces().find(|(_, f)| f.normal == DVec3::Z).unwrap().0;
    let green = s.add_material("Green", [0, 200, 0]).unwrap().0;
    s.paint(MeshId::Root, &[top], &[], Side::Both, Some(green))
        .unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&[
            p(0.5, 0.5, 1.0),
            p(1.5, 0.5, 1.0),
            p(1.5, 1.5, 1.0),
            p(0.5, 1.5, 1.0),
        ])
    })
    .unwrap();
    let inner = s
        .root()
        .faces()
        .find(|(_, f)| f.normal == DVec3::Z && f.holes.is_empty() && f.outer.len() == 4)
        .filter(|(id, _)| *id != top)
        .unwrap()
        .0;
    s.edit(MeshId::Root, |m| m.push_pull(inner, -0.5)).unwrap();
    s.validate().unwrap();
    let m = s.root();
    let recess: Vec<_> = m
        .faces()
        .filter(|(_, f)| {
            f.outer
                .iter()
                .all(|v| m.position(*v).unwrap().z >= 0.5 - 1e-9)
                && f.outer
                    .iter()
                    .any(|v| m.position(*v).unwrap().z < 1.0 - 1e-9)
        })
        .collect();
    assert_eq!(recess.len(), 5, "floor and four walls");
    assert!(
        recess
            .iter()
            .all(|(_, f)| f.paint() == [Some(green), Some(green)])
    );
}

#[test]
fn copies_groups_and_explodes_carry_paint() {
    let (mut s, face) = square_scene();
    let id = red(&mut s);
    s.paint(MeshId::Root, &[face], &[], Side::Front, Some(id))
        .unwrap();
    let edges: Vec<_> = s.root().edges().map(|(e, _)| e).collect();
    s.edit(MeshId::Root, |m| {
        m.copy(&edges, &[face], p(5.0, 0.0, 0.0), 1)
    })
    .unwrap();
    assert_eq!(s.root().faces().count(), 2);
    assert!(s.root().faces().all(|(_, f)| f.front == Some(id)));

    // Group the copy (the face stays painted inside), paint the group blue, copy the group:
    // the copy is blue too.
    let copy = s
        .root()
        .faces()
        .map(|(f, _)| f)
        .find(|f| *f != face)
        .unwrap();
    let copy_edges: Vec<_> = {
        let m = s.root();
        m.edges()
            .filter(|(_, e)| e.v.iter().all(|v| m.position(*v).unwrap().x >= 5.0))
            .map(|(e, _)| e)
            .collect()
    };
    let (group, _) = s
        .make_group(
            MeshId::Root,
            &copy_edges,
            &[copy],
            &[],
            DefinitionKind::Group,
            "",
        )
        .unwrap();
    let d = s.root().instance(group).unwrap().definition;
    assert!(
        s.definition(d)
            .unwrap()
            .mesh
            .faces()
            .all(|(_, f)| f.front == Some(id))
    );
    let blue = s.add_material("Blue", BLUE).unwrap().0;
    s.paint(MeshId::Root, &[], &[group], Side::Front, Some(blue))
        .unwrap();
    s.copy_instances(MeshId::Root, &[group], p(0.0, 5.0, 0.0), 1)
        .unwrap();
    let copied: InstanceId = s
        .root()
        .instances()
        .map(|(i, _)| i)
        .find(|i| *i != group)
        .unwrap();
    assert_eq!(s.root().instance(copied).unwrap().material, Some(blue));

    // Exploding the copy: its face keeps red on the front and takes the group's blue on the
    // back, which had no material.
    s.explode(MeshId::Root, copied).unwrap();
    s.validate().unwrap();
    let exploded = s
        .root()
        .faces()
        .find(|(_, f)| {
            f.outer
                .iter()
                .all(|v| s.root().position(*v).unwrap().y >= 5.0)
        })
        .unwrap()
        .1;
    assert_eq!(exploded.paint(), [Some(id), Some(blue)]);
}

#[test]
fn exploding_a_mirrored_painted_instance_keeps_each_material_on_its_physical_side() {
    let (mut s, face) = square_scene();
    let id = red(&mut s);
    let blue = s.add_material("Blue", BLUE).unwrap().0;
    s.paint(MeshId::Root, &[face], &[], Side::Front, Some(id))
        .unwrap();
    s.paint(MeshId::Root, &[face], &[], Side::Back, Some(blue))
        .unwrap();
    let edges: Vec<_> = s.root().edges().map(|(e, _)| e).collect();
    let (group, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &[face],
            &[],
            DefinitionKind::Group,
            "",
        )
        .unwrap();
    // Mirrored in z, the red side that faced up now faces down.
    let mirror = glam::DAffine3::from_scale(DVec3::new(1.0, 1.0, -1.0));
    s.edit(
        MeshId::Root,
        |m| Ok(m.transform_instances(&[group], mirror)),
    )
    .unwrap();
    s.explode(MeshId::Root, group).unwrap();
    s.validate().unwrap();
    let (_, f) = s.root().faces().next().unwrap();
    assert_eq!(f.normal, -DVec3::Z);
    assert_eq!(f.paint(), [Some(id), Some(blue)], "red faces down, blue up");
}

#[test]
fn materials_survive_files_and_bad_material_data_is_rejected() {
    let (mut s, face) = square_scene();
    let id = red(&mut s);
    s.paint(MeshId::Root, &[face], &[], Side::Back, Some(id))
        .unwrap();
    let edges: Vec<_> = s.root().edges().map(|(e, _)| e).collect();
    let (group, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &[face],
            &[],
            DefinitionKind::Group,
            "",
        )
        .unwrap();
    s.paint(MeshId::Root, &[], &[group], Side::Front, Some(id))
        .unwrap();
    let json = serde_json::to_string(&s.to_data()).unwrap();
    let data: SceneData = serde_json::from_str(&json).unwrap();
    let back = Scene::from_data(&data).unwrap();
    assert_eq!(back, s);
    // New ids continue after the material's.
    let mut next = back.clone();
    let (later, _) = next.add_material("Blue", BLUE).unwrap();
    assert!(later.0 > id.0);

    let rejects = |f: &dyn Fn(&mut SceneData), expected: &str| {
        let mut d = s.to_data();
        f(&mut d);
        let err = Scene::from_data(&d).unwrap_err();
        assert!(err.contains(expected), "{err:?} lacks {expected:?}");
    };
    rejects(
        &|d| d.root.instances[0].material = Some(12345),
        "missing material",
    );
    rejects(
        &|d| d.definitions[0].mesh.faces[0].front = Some(12345),
        "missing material",
    );
    rejects(&|d| d.materials[0].name = " ".into(), "needs a name");
    rejects(
        &|d| {
            let m = d.materials[0].clone();
            d.materials.push(MaterialData { id: 777, ..m });
        },
        "two materials are named",
    );
    rejects(
        &|d| {
            let m = d.materials[0].clone();
            d.materials.push(MaterialData {
                name: "Other".into(),
                ..m
            });
        },
        "appears twice",
    );
    rejects(&|d| d.materials[0].id = u64::MAX, "too large");
    // A color component over 255 is not a material color.
    let bad = json.replace("[200,30,30]", "[256,30,30]");
    assert!(serde_json::from_str::<SceneData>(&bad).is_err());
}

#[test]
fn a_sheet_pushed_against_its_normal_keeps_the_painted_side_outside() {
    // A vertical rectangle in the plane y = 0 (normal +Y), painted red on its back (the -Y side).
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 0.0, 2.0),
        p(0.0, 0.0, 2.0),
    ])
    .unwrap();
    let face = m.faces().next().unwrap().0;
    let mut s = Scene::from_root(m);
    let id = red(&mut s);
    assert_eq!(s.root().face(face).unwrap().normal, DVec3::Y);
    s.paint(MeshId::Root, &[face], &[], Side::Back, Some(id))
        .unwrap();
    s.edit(MeshId::Root, |m| m.push_pull(face, -1.0)).unwrap();
    s.validate().unwrap();
    let m = s.root();
    assert_eq!(m.faces().count(), 6);
    let cap = m
        .faces()
        .find(|(_, f)| {
            f.normal == -DVec3::Y && f.outer.iter().all(|v| m.position(*v).unwrap().y < -0.5)
        })
        .unwrap()
        .1;
    assert_eq!(cap.front, Some(id));
    // Every face of the solid except the base shows red on its outward (front) side.
    for (fid, f) in m.faces().filter(|(f, _)| *f != face) {
        assert_eq!(f.front, Some(id), "{fid:?}");
    }
}
