//! Texture placements on faces: the placed and back-side texture positions, the
//! `position_texture` edit, what faces keep through splits, copies, groups and push/pull, and
//! plain data.

use glam::{DAffine3, DVec2, DVec3};
use kernel::{
    DefinitionKind, Face, FaceId, KernelError, MeshId, Model, PlacementEdit, Scene, SceneData,
    Side, TexturePlacement, back_texture_position, normal_angle,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A 2 x 1 m rectangle on the ground; its face (normal +z or -z, as the kernel turns it).
fn ground() -> (Model, FaceId) {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let f = m.faces().next().unwrap().0;
    (m, f)
}

fn placement(offset: [f64; 2], rotation_deg: f64, scale: [f64; 2]) -> TexturePlacement {
    TexturePlacement {
        offset,
        rotation_deg,
        scale,
    }
}

fn sets(t: TexturePlacement) -> PlacementEdit {
    PlacementEdit {
        offset: Some(t.offset),
        rotation_deg: Some(t.rotation_deg),
        scale: Some(t.scale),
        reset: false,
    }
}

#[test]
fn a_placement_moves_turns_and_scales_the_projection() {
    // Moved 1 m along u: the texture's origin is at u = 1.
    let moved = placement([1.0, 0.0], 0.0, [1.0, 1.0]);
    assert!(
        moved
            .place(DVec2::new(1.0, 0.0))
            .abs_diff_eq(DVec2::ZERO, 1e-12)
    );
    // Turned a quarter counter-clockwise: the image's width now runs along v.
    let turned = placement([0.0, 0.0], 90.0, [1.0, 1.0]);
    assert!(
        turned
            .place(DVec2::new(0.0, 1.0))
            .abs_diff_eq(DVec2::X, 1e-12)
    );
    assert!(
        turned
            .place(DVec2::new(-1.0, 0.0))
            .abs_diff_eq(DVec2::Y, 1e-12)
    );
    // Twice as wide: 2 m along u is one image width (in meters of the texture's own size).
    let wide = placement([0.0, 0.0], 0.0, [2.0, 1.0]);
    assert!(
        wide.place(DVec2::new(2.0, 3.0))
            .abs_diff_eq(DVec2::new(1.0, 3.0), 1e-12)
    );
    // The back side mirrors u.
    assert_eq!(
        back_texture_position(DVec2::new(1.5, 2.0)),
        DVec2::new(-1.5, 2.0)
    );
    assert_eq!(normal_angle(-90.0), 270.0);
    assert_eq!(normal_angle(450.0), 90.0);
    assert_eq!(normal_angle(-1e-20), 0.0);
    assert!(normal_angle(-0.0).is_sign_positive());
}

#[test]
fn turning_a_face_over_keeps_what_each_side_shows() {
    let t = placement([0.3, -0.7], 30.0, [2.0, 0.5]);
    // A level face and a sloping one.
    for normal in [DVec3::Z, p(0.0, -1.0, 1.0).normalize()] {
        let mut face = Face::new(Vec::new(), Vec::new(), normal);
        face.texture = Some(t);
        let flipped = face.reversed();
        for q in [p(0.0, 0.0, 0.0), p(1.0, 2.0, 0.5), p(-3.0, 0.25, 4.0)] {
            let (front, back) = (
                face.texture_position(q),
                back_texture_position(face.texture_position(q)),
            );
            let flipped_front = flipped.texture_position(q);
            assert!(flipped_front.abs_diff_eq(back, 1e-9), "{normal} {q}");
            assert!(
                back_texture_position(flipped_front).abs_diff_eq(front, 1e-9),
                "{normal} {q}"
            );
        }
        assert_eq!(flipped.reversed().texture, Some(t));
    }
}

#[test]
fn position_texture_sets_the_given_fields_and_undo_restores_them() {
    let (mut m, f) = ground();
    let before = m.clone();
    let edit = PlacementEdit {
        rotation_deg: Some(-90.0),
        ..PlacementEdit::default()
    };
    let (turn, n) = m.position_texture(&[f], edit).unwrap();
    assert_eq!(n, 1);
    assert_eq!(
        m.face(f).unwrap().texture,
        Some(placement([0.0, 0.0], 270.0, [1.0, 1.0]))
    );
    // Other fields are kept.
    let edit = PlacementEdit {
        offset: Some([0.5, 0.25]),
        ..PlacementEdit::default()
    };
    let (moved, _) = m.position_texture(&[f], edit).unwrap();
    assert_eq!(
        m.face(f).unwrap().texture,
        Some(placement([0.5, 0.25], 270.0, [1.0, 1.0]))
    );
    // The same again changes nothing.
    let (none, n) = m.position_texture(&[f], edit).unwrap();
    assert!(none.is_empty());
    assert_eq!(n, 0);
    // Reset, then the fields given.
    let reset = PlacementEdit {
        scale: Some([2.0, 2.0]),
        reset: true,
        ..PlacementEdit::default()
    };
    m.position_texture(&[f], reset).unwrap();
    assert_eq!(
        m.face(f).unwrap().texture,
        Some(placement([0.0, 0.0], 0.0, [2.0, 2.0]))
    );
    // A placement back at the default is stored as none.
    let (cleared, n) = m
        .position_texture(
            &[f],
            PlacementEdit {
                reset: true,
                ..PlacementEdit::default()
            },
        )
        .unwrap();
    assert_eq!((m.face(f).unwrap().texture, n), (None, 1));
    m.apply(&cleared.inverse());
    assert!(m.face(f).unwrap().texture.is_some());
    let mut undone = before.clone();
    undone.apply(&turn);
    undone.apply(&moved);
    undone.apply(&moved.inverse());
    undone.apply(&turn.inverse());
    assert_eq!(undone, before);
    m.validate().unwrap();
}

#[test]
fn out_of_range_placements_are_refused_and_change_nothing() {
    let (mut m, f) = ground();
    m.position_texture(&[f], sets(placement([1.0, 1.0], 45.0, [1.0, 1.0])))
        .unwrap();
    let before = m.clone();
    for bad in [
        PlacementEdit {
            offset: Some([f64::NAN, 0.0]),
            ..PlacementEdit::default()
        },
        PlacementEdit {
            offset: Some([0.0, 2.0e6]),
            ..PlacementEdit::default()
        },
        PlacementEdit {
            rotation_deg: Some(f64::INFINITY),
            ..PlacementEdit::default()
        },
        PlacementEdit {
            scale: Some([0.0, 1.0]),
            ..PlacementEdit::default()
        },
        PlacementEdit {
            scale: Some([1.0, -2.0]),
            ..PlacementEdit::default()
        },
        PlacementEdit {
            scale: Some([1.0, 1001.0]),
            ..PlacementEdit::default()
        },
    ] {
        let err = m.position_texture(&[f], bad).unwrap_err();
        assert!(
            matches!(&err, KernelError::TexturePlacement(why) if why.contains("texture")),
            "{bad:?}: {err:?}"
        );
        assert_eq!(m, before);
    }
}

#[test]
fn split_pieces_keep_the_placement_and_new_faces_have_none() {
    let (mut m, f) = ground();
    let t = placement([0.1, 0.2], 30.0, [0.5, 0.5]);
    m.position_texture(&[f], sets(t)).unwrap();
    // A line across splits the face in two; both keep the placement.
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    assert_eq!(m.faces().count(), 2);
    assert!(m.faces().all(|(_, g)| g.texture == Some(t)));
    // A new rectangle apart from it has none.
    m.add_polygon(&[
        p(5.0, 0.0, 0.0),
        p(6.0, 0.0, 0.0),
        p(6.0, 1.0, 0.0),
        p(5.0, 1.0, 0.0),
    ])
    .unwrap();
    assert_eq!(m.faces().filter(|(_, g)| g.texture.is_none()).count(), 1);
    m.validate().unwrap();
}

#[test]
fn copies_keep_the_placement_and_push_pull_caps_keep_it_too() {
    let (mut m, f) = ground();
    let t = placement([0.0, 0.0], 90.0, [1.0, 2.0]);
    m.position_texture(&[f], sets(t)).unwrap();
    let edges: Vec<_> = m.edges().map(|(id, _)| id).collect();
    m.copy(&edges, &[f], p(0.0, 5.0, 0.0), 2).unwrap();
    assert_eq!(m.faces().count(), 3);
    assert!(m.faces().all(|(_, g)| g.texture == Some(t)));

    // Pulled up into a box: the cap keeps the placement (turned over with the cap when the
    // sheet turns over), the sides have none.
    let (mut m, f) = ground();
    m.position_texture(&[f], sets(t)).unwrap();
    let face = m.face(f).unwrap().clone();
    m.push_pull(f, -face.normal.z.signum()).unwrap();
    let placed: Vec<&Face> = m
        .faces()
        .map(|(_, g)| g)
        .filter(|g| g.texture.is_some())
        .collect();
    assert_eq!(placed.len(), 2, "the base and the cap");
    assert_eq!(m.faces().filter(|(_, g)| g.texture.is_none()).count(), 4);
    // The cap is the pushed face moved: turned over with it when the sheet turned over.
    let expected = if face.normal.z < 0.0 {
        face.reversed().texture
    } else {
        face.texture
    };
    let cap = m
        .faces()
        .map(|(_, g)| g)
        .find(|g| g.texture.is_some() && cap_is_off_ground(&m, g))
        .expect("a placed cap on top");
    assert_eq!(cap.texture, expected);
    m.validate().unwrap();
}

/// True for a face that lies in the plane a meter off the ground, not on it.
fn cap_is_off_ground(m: &Model, g: &Face) -> bool {
    g.outer
        .iter()
        .all(|&v| (m.position(v).unwrap().z.abs() - 1.0).abs() < 1e-9)
}

#[test]
fn push_pull_sides_inside_a_placed_face_have_no_placement() {
    // A wall with a placement; a floor rectangle sharing its bottom edge pulled up 1 m: the
    // side that lies inside the wall's plane must not inherit the wall's placement.
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(4.0, 0.0, 0.0),
        p(4.0, 0.0, 3.0),
        p(0.0, 0.0, 3.0),
    ])
    .unwrap();
    let wall = m.faces().next().unwrap().0;
    m.position_texture(&[wall], sets(placement([0.0, 0.0], 30.0, [1.0, 1.0])))
        .unwrap();
    m.add_polygon(&[
        p(1.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 1.0, 0.0),
        p(1.0, 1.0, 0.0),
    ])
    .unwrap();
    let floor = m
        .faces()
        .find(|(_, g)| g.normal.z.abs() > 0.5)
        .map(|(f, _)| f)
        .unwrap();
    let sign = m.face(floor).unwrap().normal.z.signum();
    m.push_pull(floor, sign).unwrap();
    let side = m
        .faces()
        .map(|(_, g)| g)
        .find(|g| {
            g.normal.y.abs() > 0.5
                && g.outer.iter().all(|&v| {
                    let q = m.position(v).unwrap();
                    q.y.abs() < 1e-9
                        && (1.0 - 1e-9..=2.0 + 1e-9).contains(&q.x)
                        && q.z <= 1.0 + 1e-9
                })
        })
        .expect("the side face in the wall's plane");
    assert_eq!(side.texture, None);
    m.validate().unwrap();
}

#[test]
fn groups_explode_and_definitions_keep_placements() {
    let (m, f) = ground();
    let mut s = Scene::from_root(m);
    let t = placement([0.5, 0.0], 0.0, [3.0, 3.0]);
    s.edit(MeshId::Root, |m| Ok(m.position_texture(&[f], sets(t))?.0))
        .unwrap();
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let (instance, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &[f],
            &[],
            DefinitionKind::Component,
            "Tile",
        )
        .unwrap();
    let (_, d) = s.definitions().next().unwrap();
    assert!(d.mesh.faces().all(|(_, g)| g.texture == Some(t)));
    s.explode(MeshId::Root, instance).unwrap();
    assert!(s.root().faces().all(|(_, g)| g.texture == Some(t)));
    s.validate().unwrap();
}

#[test]
fn placements_are_kept_in_plain_data_and_checked_on_load() {
    let (m, f) = ground();
    let mut s = Scene::from_root(m);
    let t = placement([0.25, -1.5], 22.5, [0.5, 4.0]);
    s.edit(MeshId::Root, |m| Ok(m.position_texture(&[f], sets(t))?.0))
        .unwrap();
    let data = s.to_data();
    let json = serde_json::to_string(&data).unwrap();
    assert!(
        json.contains(
            r#""texture_position":{"offset":[0.25,-1.5],"rotation_deg":22.5,"scale":[0.5,4.0]}"#
        ),
        "{json}"
    );
    let back: SceneData = serde_json::from_str(&json).unwrap();
    assert_eq!(Scene::from_data(&back).unwrap(), s);

    // Faces written before placements existed have none; a default placement reads as none.
    let mut old = data.clone();
    old.root.faces[0].texture_position = None;
    let plain = Scene::from_data(&old).unwrap();
    assert!(plain.root().faces().all(|(_, g)| g.texture.is_none()));
    let mut default = data.clone();
    default.root.faces[0].texture_position = Some(TexturePlacement::default());
    let read = Scene::from_data(&default).unwrap();
    assert!(read.root().faces().all(|(_, g)| g.texture.is_none()));

    for bad in [
        placement([0.0, 0.0], 360.0, [1.0, 1.0]),
        placement([0.0, 0.0], 0.0, [0.0, 1.0]),
        placement([f64::INFINITY, 0.0], 0.0, [1.0, 1.0]),
    ] {
        let mut d = data.clone();
        d.root.faces[0].texture_position = Some(bad);
        assert!(
            Scene::from_data(&d).unwrap_err().contains("texture"),
            "{bad:?}"
        );
    }
}

#[test]
fn triangles_carry_placed_texture_positions() {
    let (m, f) = ground();
    let mut s = Scene::from_root(m);
    let t = placement([1.0, 0.0], 90.0, [1.0, 1.0]);
    s.edit(MeshId::Root, |m| Ok(m.position_texture(&[f], sets(t))?.0))
        .unwrap();
    let face = s.root().face(f).unwrap().clone();
    for tri in s.triangles() {
        for (c, uv) in tri.corners.iter().zip(tri.uvs) {
            assert!(uv.abs_diff_eq(face.texture_position(*c), 1e-12));
        }
    }
    // On a level face u runs along x: the corner at x = 2, y = 0 sits 1 m past the moved
    // origin along u, which a quarter turn of the texture puts 1 m down its height.
    let corner = p(2.0, 0.0, 0.0);
    assert!(
        face.texture_position(corner)
            .abs_diff_eq(DVec2::new(0.0, -1.0), 1e-12)
    );
}

/// Explodes the one instance of `m`'s face `f`, with placement `t`, after moving the instance
/// by `moved`, and checks each corner of the exploded face shows the texture where the instance
/// showed it.
fn explode_keeps_texture_in_place(m: Model, f: FaceId, t: TexturePlacement, moved: DAffine3) {
    let mut s = Scene::from_root(m);
    s.edit(MeshId::Root, |m| Ok(m.position_texture(&[f], sets(t))?.0))
        .unwrap();
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let (instance, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &[f],
            &[],
            DefinitionKind::Group,
            "Tile",
        )
        .unwrap();
    // The texture shows only on painted faces; the instance's material passes to them.
    let (plain, _) = s.add_material("Plain", [1, 2, 3]).unwrap();
    s.paint(MeshId::Root, &[], &[instance], Side::Front, Some(plain))
        .unwrap();
    s.edit(MeshId::Root, |m| {
        Ok(m.transform_instances(&[instance], moved))
    })
    .unwrap();
    let placed = s.root().instance(instance).unwrap().clone();
    let definition = &s.definition(placed.definition).unwrap().mesh;
    let (_, face) = definition.faces().next().unwrap();
    let before: Vec<(DVec3, DVec2)> = face
        .outer
        .iter()
        .map(|&v| {
            let q = definition.position(v).unwrap();
            (
                placed.transform.transform_point3(q),
                face.texture_position(q),
            )
        })
        .collect();
    s.explode(MeshId::Root, instance).unwrap();
    let (_, face) = s.root().faces().next().unwrap();
    for (world, shown) in before {
        let now = face.texture_position(world);
        assert!(now.abs_diff_eq(shown, 1e-9), "{world}: {now} not {shown}");
    }
    s.validate().unwrap();
}

#[test]
fn explode_keeps_a_moved_and_turned_instance_texture_in_place() {
    let t = placement([0.5, -0.25], 15.0, [2.0, 3.0]);
    let moved = DAffine3::from_translation(p(0.3, -1.2, 0.0))
        * DAffine3::from_rotation_z(30f64.to_radians());
    let (m, f) = ground();
    explode_keeps_texture_in_place(m, f, t, moved);
    // A wall, moved and turned about z: u turns with it.
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 0.0, 1.0),
        p(0.0, 0.0, 1.0),
    ])
    .unwrap();
    let f = m.faces().next().unwrap().0;
    explode_keeps_texture_in_place(m, f, t, moved);
    // A turn about x stands the ground face up; an even scale enlarges the texture with it.
    let (m, f) = ground();
    let tilted = DAffine3::from_translation(p(1.0, 2.0, 0.5))
        * DAffine3::from_rotation_x(90f64.to_radians())
        * DAffine3::from_scale(DVec3::splat(2.0));
    explode_keeps_texture_in_place(m, f, t, tilted);
}
