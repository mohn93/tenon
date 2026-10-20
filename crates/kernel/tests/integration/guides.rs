use glam::{DAffine3, DVec3};
use kernel::{
    DefinitionKind, EntityId, Guide, GuideData, GuideId, MeshId, Model, Scene, SceneData,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn line(point: DVec3, direction: DVec3) -> Guide {
    Guide::Line {
        point,
        direction,
        tag: None,
    }
}

/// The id of the one guide a change created.
fn created_guide(created: &[EntityId]) -> GuideId {
    match created {
        [EntityId::Guide(g)] => *g,
        other => panic!("expected one guide, got {other:?}"),
    }
}

#[test]
fn guides_are_added_erased_moved_and_copied_as_undoable_changes() {
    let mut m = Model::new();
    let c = m
        .add_guide(line(p(1.0, 0.0, 0.0), p(0.0, 3.0, 0.0)))
        .unwrap();
    let g = created_guide(&c.summary().created);
    assert_eq!(
        m.guide(g),
        Some(&line(p(1.0, 0.0, 0.0), DVec3::Y)),
        "the direction is normalized"
    );
    let point = created_guide(
        &m.add_guide(Guide::Point {
            position: p(2.0, 2.0, 0.0),
            tag: None,
        })
        .unwrap()
        .summary()
        .created,
    );
    m.validate().unwrap();
    let before = m.clone();

    let moved = m.transform_guides(&[g, point], DAffine3::from_translation(p(0.0, 0.0, 1.0)));
    let s = moved.summary();
    assert_eq!(s.modified, vec![EntityId::Guide(g), EntityId::Guide(point)]);
    assert!(s.created.is_empty() && s.deleted.is_empty());
    assert_eq!(m.guide(g), Some(&line(p(1.0, 0.0, 1.0), DVec3::Y)));
    m.apply(&moved.inverse());
    assert_eq!(m, before);

    let copies = m.copy_guides(
        &[point],
        &[
            DAffine3::from_translation(DVec3::X),
            DAffine3::from_translation(DVec3::X * 2.0),
        ],
    );
    assert_eq!(copies.summary().created.len(), 2);
    assert_eq!(m.guides().count(), 4);
    m.apply(&copies.inverse());
    assert_eq!(m, before);

    let erased = m.erase_guides(&[g, GuideId(999)]);
    assert_eq!(erased.summary().deleted, vec![EntityId::Guide(g)]);
    m.apply(&erased.inverse());
    assert_eq!(m, before);
}

#[test]
fn guides_need_finite_numbers_and_a_direction() {
    let mut m = Model::new();
    assert!(m.add_guide(line(DVec3::ZERO, DVec3::ZERO)).is_err());
    assert!(m.add_guide(line(p(f64::NAN, 0.0, 0.0), DVec3::X)).is_err());
    assert!(
        m.add_guide(Guide::Point {
            position: p(0.0, f64::INFINITY, 0.0),
            tag: None
        })
        .is_err()
    );
    assert_eq!(m.guides().count(), 0);
}

/// A root holding a square and a guide line along its front side, offset 1 m.
fn scene_with_guides() -> (Scene, GuideId) {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 2.0, 0.0),
        p(0.0, 2.0, 0.0),
    ])
    .unwrap();
    let mut s = Scene::from_root(m);
    let g = s
        .edit(MeshId::Root, |m| {
            m.add_guide(line(p(0.0, -1.0, 0.0), DVec3::X))
        })
        .unwrap()
        .summary()
        .created;
    (s, created_guide(&g))
}

#[test]
fn guides_round_trip_through_data_and_old_data_has_none() {
    let (s, g) = scene_with_guides();
    let json = serde_json::to_string(&s.to_data()).unwrap();
    assert!(json.contains("\"guides\""), "{json}");
    let back = Scene::from_data(&serde_json::from_str(&json).unwrap()).unwrap();
    assert_eq!(back, s);
    assert_eq!(back.root().guide(g), s.root().guide(g));

    // Data written before guides existed has no "guides" key, and none are written when
    // there are none.
    let old = Scene::from_root(Model::new());
    let json = serde_json::to_string(&old.to_data()).unwrap();
    assert!(!json.contains("guides"), "{json}");
    let data: SceneData = serde_json::from_str(&json).unwrap();
    assert_eq!(Scene::from_data(&data).unwrap(), old);
}

#[test]
fn broken_guides_are_rejected() {
    let (s, _) = scene_with_guides();
    let rejects = |f: &dyn Fn(&mut SceneData), expected: &str| {
        let mut data = s.to_data();
        f(&mut data);
        let err = Scene::from_data(&data).unwrap_err();
        assert!(err.contains(expected), "{err:?} lacks {expected:?}");
    };
    rejects(
        &|d| d.root.guides[0].direction = Some([0.0, 2.0, 0.0]),
        "not a unit vector",
    );
    rejects(
        &|d| d.root.guides[0].point = [f64::NAN, 0.0, 0.0],
        "not finite",
    );
    rejects(
        &|d| {
            let v = d.root.vertices[0].id;
            d.root.guides.push(GuideData {
                id: v,
                point: [0.0; 3],
                direction: None,
                tag: None,
            });
        },
        "share an id",
    );
}

#[test]
fn grouping_keeps_guides_and_copies_and_explodes_carry_them() {
    let (mut s, g) = scene_with_guides();
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
    let (i, _) = s
        .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
        .unwrap();
    let d = MeshId::Definition(s.root().instance(i).unwrap().definition);
    let moved = s.move_guides(MeshId::Root, d, &[g]);
    assert_eq!(s.root().guides().count(), 0);
    assert!(
        s.mesh(d).unwrap().guide(g).is_some(),
        "the guide keeps its id"
    );
    s.validate().unwrap();
    s.apply(&moved.inverse());
    assert!(s.root().guide(g).is_some());
    s.apply(&moved);

    // A copied group gets a copy of the guide; exploding it brings the guide out, placed.
    s.copy_instances(MeshId::Root, &[i], p(5.0, 0.0, 0.0), 1)
        .unwrap();
    let copy = s
        .root()
        .instances()
        .map(|(id, _)| id)
        .find(|c| *c != i)
        .unwrap();
    let inner = s
        .mesh(MeshId::Definition(
            s.root().instance(copy).unwrap().definition,
        ))
        .unwrap();
    assert_eq!(inner.guides().count(), 1);
    assert!(inner.guide(g).is_none(), "copies get new ids");
    s.explode(MeshId::Root, copy).unwrap();
    let out: Vec<_> = s.root().guides().map(|(_, g)| *g).collect();
    assert_eq!(out, vec![line(p(5.0, -1.0, 0.0), DVec3::X)]);
    s.validate().unwrap();
}
