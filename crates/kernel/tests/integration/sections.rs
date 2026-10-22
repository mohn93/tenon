use glam::{DAffine3, DQuat, DVec3};
use kernel::{
    DefinitionKind, EntityId, KernelError, MeshId, Model, Parts, Scene, SceneData, SectionId,
    SectionPlane, Tag, section_loops, triangulate_section,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn plane(point: DVec3, normal: DVec3) -> SectionPlane {
    SectionPlane::new(point, normal)
}

fn created(c: &kernel::ChangeSet) -> SectionId {
    let found: Vec<SectionId> = c
        .summary()
        .created
        .iter()
        .filter_map(|id| match id {
            EntityId::Section(s) => Some(*s),
            _ => None,
        })
        .collect();
    match found[..] {
        [s] => s,
        _ => panic!("expected one section plane, got {:?}", c.summary().created),
    }
}

fn add(m: &mut Model, point: DVec3, normal: DVec3, activate: bool) -> SectionId {
    created(&m.add_section(plane(point, normal), activate).unwrap())
}

/// A scene whose root holds a unit box.
fn box_scene() -> Scene {
    let mut m = Model::new();
    let c = m
        .add_polygon(&[
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
        ])
        .unwrap();
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
    Scene::from_root(m)
}

fn group_everything(
    s: &mut Scene,
    sections: Vec<SectionId>,
) -> (kernel::InstanceId, kernel::SceneChange) {
    let edges = s.root().edges().map(|(id, _)| id).collect();
    let faces = s.root().faces().map(|(id, _)| id).collect();
    let parts = Parts {
        edges,
        faces,
        sections,
        ..Parts::default()
    };
    s.make_group_with(MeshId::Root, &parts, DefinitionKind::Group, "Box")
        .unwrap()
}

// ---- model ----

#[test]
fn sections_are_added_activated_flipped_and_undone_exactly() {
    let mut m = Model::new();
    let empty = m.clone();
    let c = m
        .add_section(plane(p(0.0, 0.0, 1.0), DVec3::Z), true)
        .unwrap();
    let a = created(&c);
    assert_eq!(m.active_section(), Some(a));
    assert_eq!(m.section(a), Some(&plane(p(0.0, 0.0, 1.0), DVec3::Z)));
    m.validate().unwrap();
    let with_a = m.clone();

    let b = add(&mut m, DVec3::ZERO, DVec3::X, false);
    assert_eq!(m.active_section(), Some(a), "not activated");
    let with_b = m.clone();

    let act = m.set_active_section(Some(b)).unwrap();
    assert_eq!(m.active_section(), Some(b));
    let s = act.summary();
    assert!(
        s.modified.contains(&EntityId::Section(a)) && s.modified.contains(&EntityId::Section(b))
    );
    m.apply(&act.inverse());
    assert_eq!(m, with_b);
    assert_eq!(m.active_section(), Some(a));
    m.apply(&act);
    assert_eq!(m.active_section(), Some(b));
    let off = m.set_active_section(None).unwrap();
    assert_eq!(m.active_section(), None);
    m.apply(&off.inverse());
    assert_eq!(m.active_section(), Some(b));
    m.set_active_section(Some(a)).unwrap();

    let before = m.clone();
    let flip = m.flip_section(a).unwrap();
    assert_eq!(m.section(a).unwrap().normal, -DVec3::Z);
    assert_eq!(m.section(a).unwrap().point, p(0.0, 0.0, 1.0));
    assert_eq!(flip.summary().modified, vec![EntityId::Section(a)]);
    assert_eq!(m.active_section(), Some(a), "flipping keeps it active");
    m.validate().unwrap();
    m.apply(&flip.inverse());
    assert_eq!(m, before);

    m.apply(&c.inverse());
    let _ = (with_a, empty);
}

#[test]
fn sections_need_a_unit_normal_finite_numbers_and_known_ids() {
    let mut m = Model::new();
    assert!(
        m.add_section(plane(DVec3::ZERO, p(0.0, 0.0, 2.0)), true)
            .is_err()
    );
    assert!(
        m.add_section(plane(DVec3::ZERO, DVec3::ZERO), true)
            .is_err()
    );
    assert!(
        m.add_section(plane(p(f64::NAN, 0.0, 0.0), DVec3::Z), true)
            .is_err()
    );
    assert!(
        m.add_section(plane(DVec3::ZERO, p(f64::INFINITY, 0.0, 0.0)), true)
            .is_err()
    );
    assert_eq!(m, Model::new());
    assert!(matches!(
        m.set_active_section(Some(SectionId(9))),
        Err(KernelError::Section(_))
    ));
    assert!(matches!(
        m.flip_section(SectionId(9)),
        Err(KernelError::Section(_))
    ));
    assert!(m.take_changes().is_empty());
}

#[test]
fn erasing_the_active_plane_clears_it_in_the_same_change() {
    let mut m = Model::new();
    let a = add(&mut m, DVec3::ZERO, DVec3::Z, false);
    let b = add(&mut m, DVec3::X, DVec3::X, true);
    m.validate().unwrap();
    let before = m.clone();

    let gone = m.erase_sections(&[a, SectionId(999)]);
    assert_eq!(gone.summary().deleted, vec![EntityId::Section(a)]);
    assert_eq!(m.active_section(), Some(b), "another plane stays active");
    m.apply(&gone.inverse());
    assert_eq!(m, before);

    let gone = m.erase_sections(&[b]);
    assert_eq!(m.active_section(), None);
    m.validate().unwrap();
    assert!(gone.summary().deleted.contains(&EntityId::Section(b)));
    m.apply(&gone.inverse());
    assert_eq!(m, before);
    assert_eq!(m.active_section(), Some(b), "undo restores it active");
    m.apply(&gone);
    assert_eq!(m.active_section(), None);
}

#[test]
fn moving_a_plane_maps_its_point_and_normal() {
    let mut m = Model::new();
    let a = add(&mut m, p(1.0, 0.0, 0.0), DVec3::X, true);
    let before = m.clone();
    let t = DAffine3::from_rotation_translation(
        DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2),
        p(0.0, 0.0, 5.0),
    );
    let c = m.transform_sections(&[a], t);
    assert_eq!(c.summary().modified, vec![EntityId::Section(a)]);
    let s = m.section(a).unwrap();
    assert!(s.point.distance(p(0.0, 1.0, 5.0)) < 1e-9);
    assert!(s.normal.distance(DVec3::Y) < 1e-9);
    assert_eq!(m.active_section(), Some(a), "moving keeps it active");
    m.validate().unwrap();
    m.apply(&c.inverse());
    assert_eq!(m, before);

    // A stretch keeps the normal perpendicular to the plane: the plane x + y = 1 stretched
    // along x by 4 is x / 4 + y = 1.
    let mut m = Model::new();
    let a = add(
        &mut m,
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0).normalize(),
        false,
    );
    m.transform_sections(&[a], DAffine3::from_scale(p(4.0, 1.0, 1.0)));
    let s = m.section(a).unwrap();
    assert!(s.point.distance(p(4.0, 0.0, 0.0)) < 1e-9);
    assert!(s.normal.distance(p(1.0, 4.0, 0.0).normalize()) < 1e-9);
    assert!((s.normal.length() - 1.0).abs() < 1e-12);
    m.validate().unwrap();
}

#[test]
fn copies_of_planes_are_inactive_and_new() {
    let mut m = Model::new();
    let a = add(&mut m, DVec3::ZERO, DVec3::Z, true);
    let before = m.clone();
    let c = m.copy_sections(
        &[a],
        &[
            DAffine3::from_translation(DVec3::X),
            DAffine3::from_translation(DVec3::X * 2.0),
        ],
    );
    assert_eq!(c.summary().created.len(), 2);
    assert_eq!(m.sections().count(), 3);
    assert_eq!(m.active_section(), Some(a));
    m.validate().unwrap();
    m.apply(&c.inverse());
    assert_eq!(m, before);
}

// ---- scene ----

#[test]
fn grouping_moves_a_plane_and_keeps_an_active_one_active() {
    let mut s = box_scene();
    let (a, b) = {
        let mut a = SectionId(0);
        let mut b = SectionId(0);
        s.edit(MeshId::Root, |m| {
            a = add(m, p(0.0, 0.0, 0.5), DVec3::Z, true);
            b = add(m, p(0.5, 0.0, 0.0), DVec3::X, false);
            Ok(kernel::ChangeSet::default())
        })
        .unwrap();
        (a, b)
    };
    s.validate().unwrap();
    let before = s.clone();
    let (i, change) = group_everything(&mut s, vec![a]);
    s.validate().unwrap();
    let def = MeshId::Definition(s.root().instance(i).unwrap().definition);
    assert!(s.root().section(a).is_none());
    assert_eq!(
        s.root().active_section(),
        None,
        "the parent has none active now"
    );
    assert!(s.root().section(b).is_some());
    let inside = s.mesh(def).unwrap();
    assert_eq!(inside.section(a), Some(&plane(p(0.0, 0.0, 0.5), DVec3::Z)));
    assert_eq!(inside.active_section(), Some(a));
    s.apply(&change.inverse());
    assert!(s == before, "undo restores both");
    assert_eq!(s.root().active_section(), Some(a));
}

#[test]
fn grouping_an_inactive_plane_keeps_the_active_one_in_the_parent() {
    let mut s = box_scene();
    let mut ids = (SectionId(0), SectionId(0));
    s.edit(MeshId::Root, |m| {
        ids = (
            add(m, DVec3::ZERO, DVec3::Z, true),
            add(m, DVec3::X, DVec3::X, false),
        );
        Ok(kernel::ChangeSet::default())
    })
    .unwrap();
    let (i, _) = group_everything(&mut s, vec![ids.1]);
    s.validate().unwrap();
    let def = MeshId::Definition(s.root().instance(i).unwrap().definition);
    assert_eq!(s.root().active_section(), Some(ids.0));
    assert_eq!(s.mesh(def).unwrap().active_section(), None);
    assert!(s.mesh(def).unwrap().section(ids.1).is_some());
}

#[test]
fn exploding_a_group_brings_its_planes_out_inactive_and_leaves_the_parents_active() {
    let mut s = box_scene();
    let mut parent_plane = SectionId(0);
    let mut inner = SectionId(0);
    s.edit(MeshId::Root, |m| {
        inner = add(m, p(0.0, 0.0, 0.5), DVec3::Z, true);
        Ok(kernel::ChangeSet::default())
    })
    .unwrap();
    let (i, _) = group_everything(&mut s, vec![inner]);
    s.edit(MeshId::Root, |m| {
        parent_plane = add(m, p(9.0, 0.0, 0.0), DVec3::X, true);
        Ok(kernel::ChangeSet::default())
    })
    .unwrap();
    s.edit(MeshId::Root, |m| {
        Ok(m.move_instances(&[i], p(10.0, 0.0, 0.0)))
    })
    .unwrap();
    s.validate().unwrap();
    let before = s.clone();
    let change = s.explode(MeshId::Root, i).unwrap();
    s.validate().unwrap();
    assert_eq!(s.root().active_section(), Some(parent_plane));
    let out: Vec<_> = s
        .root()
        .sections()
        .filter(|(id, _)| *id != parent_plane)
        .collect();
    assert_eq!(out.len(), 1);
    assert!(
        out[0].1.point.distance(p(10.0, 0.0, 0.5)) < 1e-9,
        "placed by the instance"
    );
    s.apply(&change.inverse());
    assert!(s == before);
}

#[test]
fn copying_a_group_keeps_the_active_plane_inside_its_copy() {
    let mut s = box_scene();
    let mut inner = SectionId(0);
    s.edit(MeshId::Root, |m| {
        inner = add(m, p(0.0, 0.0, 0.5), DVec3::Z, true);
        Ok(kernel::ChangeSet::default())
    })
    .unwrap();
    let (i, _) = group_everything(&mut s, vec![inner]);
    let before = s.clone();
    let change = s
        .copy_instances(MeshId::Root, &[i], DVec3::X * 3.0, 1)
        .unwrap();
    s.validate().unwrap();
    let copy = s
        .root()
        .instances()
        .map(|(_, inst)| inst.definition)
        .find(|d| *d != s.root().instance(i).unwrap().definition)
        .unwrap();
    let m = s.mesh(MeshId::Definition(copy)).unwrap();
    assert_eq!(m.sections().count(), 1);
    let (copied, _) = m.sections().next().unwrap();
    assert_eq!(
        m.active_section(),
        Some(copied),
        "the content is copied as is"
    );
    s.apply(&change.inverse());
    assert!(s == before);
}

#[test]
fn a_pasted_group_keeps_its_active_plane_but_a_pasted_plane_is_inactive() {
    let mut s = box_scene();
    let mut inner = SectionId(0);
    s.edit(MeshId::Root, |m| {
        inner = add(m, p(0.0, 0.0, 0.5), DVec3::Z, true);
        Ok(kernel::ChangeSet::default())
    })
    .unwrap();
    let (i, _) = group_everything(&mut s, vec![inner]);
    let loose = {
        let mut id = SectionId(0);
        s.edit(MeshId::Root, |m| {
            id = add(m, DVec3::ZERO, DVec3::X, true);
            Ok(kernel::ChangeSet::default())
        })
        .unwrap();
        id
    };
    let parts = Parts {
        instances: vec![i],
        sections: vec![loose],
        ..Parts::default()
    };
    let clip = s
        .extract_with(MeshId::Root, &parts, DAffine3::IDENTITY)
        .unwrap();
    let mut target = Scene::new();
    let (inserted, _) = target
        .insert(MeshId::Root, &clip, DAffine3::IDENTITY, &|_| None)
        .unwrap();
    target.validate().unwrap();
    assert_eq!(target.root().active_section(), None);
    assert_eq!(inserted.sections.len(), 1);
    let def = target.root().instances().next().unwrap().1.definition;
    let m = target.mesh(MeshId::Definition(def)).unwrap();
    assert_eq!(m.sections().count(), 1);
    assert!(m.active_section().is_some());
}

#[test]
fn clips_carry_planes_inactive() {
    let mut s = box_scene();
    let mut a = SectionId(0);
    s.edit(MeshId::Root, |m| {
        a = add(m, p(0.0, 0.0, 0.5), DVec3::Z, true);
        Ok(kernel::ChangeSet::default())
    })
    .unwrap();
    let parts = Parts {
        sections: vec![a],
        ..Parts::default()
    };
    let shift = DAffine3::from_translation(p(1.0, 0.0, 0.0));
    let clip = s.extract_with(MeshId::Root, &parts, shift).unwrap();
    clip.validate().unwrap();
    assert_eq!(clip.root().sections().count(), 1);
    assert_eq!(clip.root().active_section(), None);

    let mut target = Scene::new();
    let place = DAffine3::from_translation(p(0.0, 20.0, 0.0));
    let (inserted, change) = target
        .insert(MeshId::Root, &clip, place, &|_| None)
        .unwrap();
    target.validate().unwrap();
    assert_eq!(inserted.sections.len(), 1);
    let pasted = target.root().section(inserted.sections[0]).unwrap();
    assert!(pasted.point.distance(p(1.0, 20.0, 0.5)) < 1e-9);
    assert_eq!(target.root().active_section(), None);
    let mut undone = target.clone();
    undone.apply(&change.inverse());
    assert!(undone == Scene::new());
}

#[test]
fn planes_go_on_tags_and_move_off_deleted_ones() {
    let mut s = Scene::new();
    let (tag, _) = s
        .add_tag(Tag {
            name: "Cuts".into(),
            color: [1, 2, 3],
            visible: true,
        })
        .unwrap();
    let mut a = SectionId(0);
    s.edit(MeshId::Root, |m| {
        a = add(m, DVec3::ZERO, DVec3::Z, true);
        Ok(kernel::ChangeSet::default())
    })
    .unwrap();
    s.set_tags(MeshId::Root, &[EntityId::Section(a)], Some(tag))
        .unwrap();
    assert_eq!(s.root().section(a).unwrap().tag, Some(tag));
    assert_eq!(s.root().tag_of(EntityId::Section(a)), Some(Some(tag)));
    assert_eq!(s.tag_count(tag), 1);
    s.validate().unwrap();
    let before = s.clone();
    let deleted = s.delete_tag(tag, None).unwrap();
    assert_eq!(s.root().section(a).unwrap().tag, None);
    assert_eq!(s.root().active_section(), Some(a));
    s.apply(&deleted.inverse());
    assert!(s == before);
}

#[test]
fn entity_ids_of_planes_are_s_numbers() {
    assert_eq!(EntityId::Section(SectionId(4)).to_string(), "s4");
    assert_eq!(
        "s4".parse::<EntityId>(),
        Ok(EntityId::Section(SectionId(4)))
    );
    assert!("s04".parse::<EntityId>().is_err());
    let parts = Parts::of(&[
        EntityId::Section(SectionId(2)),
        EntityId::Section(SectionId(2)),
    ]);
    assert_eq!(parts.sections, vec![SectionId(2)]);
}

// ---- files ----

#[test]
fn planes_and_the_active_one_survive_a_data_round_trip() {
    let mut s = box_scene();
    s.edit(MeshId::Root, |m| {
        add(m, p(0.0, 0.0, 0.5), DVec3::Z, false);
        let b = add(m, p(0.5, 0.0, 0.0), DVec3::X, true);
        let _ = b;
        Ok(kernel::ChangeSet::default())
    })
    .unwrap();
    let data = s.to_data();
    assert_eq!(data.root.sections.len(), 2);
    assert!(data.root.active_section.is_some());
    let json = serde_json::to_string(&data).unwrap();
    let back = Scene::from_data(&serde_json::from_str::<SceneData>(&json).unwrap()).unwrap();
    assert!(back == s);
    assert_eq!(back.root().active_section(), s.root().active_section());
}

#[test]
fn a_scene_without_planes_writes_no_section_fields() {
    let json = serde_json::to_string(&box_scene().to_data()).unwrap();
    assert!(!json.contains("section"), "{json}");
    let data: SceneData = serde_json::from_str(&json).unwrap();
    assert!(data.root.sections.is_empty() && data.root.active_section.is_none());
}

#[test]
fn bad_plane_data_is_rejected() {
    let mut s = box_scene();
    s.edit(MeshId::Root, |m| {
        add(m, DVec3::ZERO, DVec3::Z, true);
        Ok(kernel::ChangeSet::default())
    })
    .unwrap();
    let good = s.to_data();
    let id = good.root.sections[0].id;

    let mut d = good.clone();
    d.root.sections.push(d.root.sections[0].clone());
    assert!(Scene::from_data(&d).is_err(), "duplicate id");

    let mut d = good.clone();
    d.root.sections[0].normal = [0.0, 0.0, 2.0];
    assert!(Scene::from_data(&d).is_err(), "normal not unit");

    let mut d = good.clone();
    d.root.sections[0].point = [f64::NAN, 0.0, 0.0];
    assert!(Scene::from_data(&d).is_err(), "not finite");

    let mut d = good.clone();
    d.root.active_section = Some(id + 100);
    assert!(Scene::from_data(&d).is_err(), "active names no plane");

    let mut d = good.clone();
    d.root.sections[0].id = good.root.vertices[0].id;
    d.root.active_section = Some(d.root.sections[0].id);
    assert!(Scene::from_data(&d).is_err(), "id shared with a vertex");

    let mut d = good;
    d.root.sections[0].tag = Some(77);
    assert!(Scene::from_data(&d).is_err(), "missing tag");
}

// ---- section loops ----

/// The 12 triangles of the box `lo..hi`.
fn box_tris(lo: DVec3, hi: DVec3) -> Vec<[DVec3; 3]> {
    let c = |i: usize| {
        p(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        )
    };
    let quads = [
        [0, 1, 3, 2],
        [4, 6, 7, 5],
        [0, 4, 5, 1],
        [2, 3, 7, 6],
        [0, 2, 6, 4],
        [1, 5, 7, 3],
    ];
    quads
        .iter()
        .flat_map(|q| [[c(q[0]), c(q[1]), c(q[2])], [c(q[0]), c(q[2]), c(q[3])]])
        .collect()
}

fn area(points: &[DVec3], normal: DVec3) -> f64 {
    let mut sum = DVec3::ZERO;
    for i in 0..points.len() {
        sum += points[i].cross(points[(i + 1) % points.len()]);
    }
    0.5 * sum.dot(normal)
}

fn fill_area(tris: &[[DVec3; 3]], normal: DVec3) -> f64 {
    tris.iter()
        .map(|t| 0.5 * (t[1] - t[0]).cross(t[2] - t[0]).dot(normal))
        .sum()
}

#[test]
fn a_box_cut_through_its_middle_is_one_square_loop() {
    let tris = box_tris(DVec3::ZERO, p(2.0, 3.0, 1.0));
    let r = section_loops(&tris, p(0.0, 0.0, 0.5), DVec3::Z);
    assert!(r.open.is_empty());
    assert_eq!(r.closed.len(), 1);
    assert_eq!(r.closed[0].len(), 1, "no holes");
    let outer = &r.closed[0][0];
    assert_eq!(
        outer.len(),
        4,
        "diagonal crossings are not corners: {outer:?}"
    );
    for q in outer {
        assert!((q.z - 0.5).abs() < 1e-9);
    }
    assert!(
        (area(outer, DVec3::Z) - 6.0).abs() < 1e-9,
        "outer winds counter-clockwise"
    );
    let fill = triangulate_section(&r.closed, DVec3::Z);
    assert!((fill_area(&fill, DVec3::Z) - 6.0).abs() < 1e-9);
}

#[test]
fn the_cut_does_not_depend_on_which_way_the_plane_faces() {
    let tris = box_tris(DVec3::ZERO, p(2.0, 3.0, 1.0));
    let r = section_loops(&tris, p(1.0, 0.0, 0.0), -DVec3::X);
    assert_eq!((r.closed.len(), r.open.len()), (1, 0));
    let fill = triangulate_section(&r.closed, -DVec3::X);
    assert!((fill_area(&fill, -DVec3::X).abs() - 3.0).abs() < 1e-9);
    assert!(
        fill_area(&fill, -DVec3::X) > 0.0,
        "fill winds around the normal"
    );
}

#[test]
fn a_wall_with_a_hole_cuts_to_an_outer_loop_and_a_hole() {
    // An outer box and a smaller box inside it, as a hollow solid.
    let mut tris = box_tris(DVec3::ZERO, p(4.0, 4.0, 4.0));
    tris.extend(box_tris(p(1.0, 1.0, 1.0), p(3.0, 3.0, 3.0)));
    let r = section_loops(&tris, p(0.0, 0.0, 2.0), DVec3::Z);
    assert!(r.open.is_empty());
    assert_eq!(r.closed.len(), 1);
    assert_eq!(r.closed[0].len(), 2, "outer and one hole");
    assert!((area(&r.closed[0][0], DVec3::Z) - 16.0).abs() < 1e-9);
    assert!(
        (area(&r.closed[0][1], DVec3::Z) + 4.0).abs() < 1e-9,
        "the hole winds clockwise"
    );
    let fill = triangulate_section(&r.closed, DVec3::Z);
    assert!((fill_area(&fill, DVec3::Z) - 12.0).abs() < 1e-9);
}

#[test]
fn an_island_inside_a_hole_is_a_second_solid_region() {
    let mut tris = box_tris(DVec3::ZERO, p(6.0, 6.0, 6.0));
    tris.extend(box_tris(p(1.0, 1.0, 1.0), p(5.0, 5.0, 5.0)));
    tris.extend(box_tris(p(2.0, 2.0, 2.0), p(4.0, 4.0, 4.0)));
    let r = section_loops(&tris, p(0.0, 0.0, 3.0), DVec3::Z);
    assert_eq!(r.closed.len(), 2);
    let sizes: Vec<usize> = r.closed.iter().map(Vec::len).collect();
    assert!(sizes.contains(&2) && sizes.contains(&1), "{sizes:?}");
    let fill = triangulate_section(&r.closed, DVec3::Z);
    assert!((fill_area(&fill, DVec3::Z) - (36.0 - 16.0 + 4.0)).abs() < 1e-9);
}

#[test]
fn two_separate_solids_give_two_loops() {
    let mut tris = box_tris(DVec3::ZERO, DVec3::ONE);
    tris.extend(box_tris(p(3.0, 0.0, 0.0), p(4.0, 1.0, 1.0)));
    let r = section_loops(&tris, p(0.0, 0.0, 0.5), DVec3::Z);
    assert_eq!(r.closed.len(), 2);
    assert!(r.closed.iter().all(|g| g.len() == 1));
}

#[test]
fn a_cut_through_an_open_surface_is_an_open_chain() {
    // A vertical quad, as two triangles.
    let tris = vec![
        [p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0), p(2.0, 0.0, 2.0)],
        [p(0.0, 0.0, 0.0), p(2.0, 0.0, 2.0), p(0.0, 0.0, 2.0)],
    ];
    let r = section_loops(&tris, p(0.0, 0.0, 1.0), DVec3::Z);
    assert!(r.closed.is_empty());
    assert_eq!(r.open.len(), 1);
    let chain = &r.open[0];
    assert_eq!(
        chain.len(),
        2,
        "the diagonal crossing is not a corner: {chain:?}"
    );
    assert!((chain[0].distance(chain[1]) - 2.0).abs() < 1e-9);
    assert!(triangulate_section(&r.closed, DVec3::Z).is_empty());
}

#[test]
fn a_plane_through_the_top_face_fills_only_when_the_box_is_kept() {
    let tris = box_tris(DVec3::ZERO, p(2.0, 3.0, 1.0));
    // Keeping what is above, no material is kept: the outline stays as cut edges.
    let r = section_loops(&tris, p(0.0, 0.0, 1.0), DVec3::Z);
    assert!(r.closed.is_empty(), "{r:?}");
    assert_eq!(r.open.len(), 1);
    // Keeping the box: its top is the cap.
    let (r, area) = fill_of(&tris, p(0.0, 0.0, 1.0), -DVec3::Z);
    assert_eq!((r.closed.len(), r.open.len()), (1, 0), "{r:?}");
    assert!((area - 6.0).abs() < 1e-9, "{area}");
}

#[test]
fn a_plane_through_vertices_gives_closed_loops_without_repeats() {
    // x = y through the opposite corners of a box.
    let tris = box_tris(DVec3::ZERO, p(1.0, 1.0, 1.0));
    let n = p(1.0, -1.0, 0.0).normalize();
    let r = section_loops(&tris, DVec3::ZERO, n);
    assert!(r.open.is_empty());
    assert_eq!(r.closed.len(), 1);
    let outer = &r.closed[0][0];
    assert_eq!(outer.len(), 4);
    for i in 0..outer.len() {
        assert!(
            outer[i].distance(outer[(i + 1) % outer.len()]) > 1e-6,
            "zero-length side"
        );
    }
    assert!((area(outer, n).abs() - 2f64.sqrt()).abs() < 1e-9);
}

#[test]
fn a_plane_touching_only_an_edge_or_a_corner_or_missing_is_harmless() {
    let tris = box_tris(DVec3::ZERO, DVec3::ONE);
    let n = p(1.0, 1.0, 0.0).normalize();
    for (point, normal) in [
        (DVec3::ZERO, n),
        (DVec3::ZERO, DVec3::ONE.normalize()),
        (p(0.0, 0.0, 9.0), DVec3::Z),
        (p(0.0, 0.0, -1.0), DVec3::Z),
    ] {
        let r = section_loops(&tris, point, normal);
        assert!(r.closed.is_empty() && r.open.is_empty(), "{r:?}");
    }
}

#[test]
fn degenerate_input_never_panics() {
    let tri = [DVec3::ZERO, DVec3::X, DVec3::Y];
    assert!(section_loops(&[], DVec3::ZERO, DVec3::Z).closed.is_empty());
    for (point, normal) in [
        (DVec3::ZERO, DVec3::ZERO),
        (DVec3::ZERO, p(f64::NAN, 0.0, 1.0)),
        (p(f64::INFINITY, 0.0, 0.0), DVec3::Z),
    ] {
        let r = section_loops(&[tri], point, normal);
        assert!(r.closed.is_empty() && r.open.is_empty());
    }
    let nan = [
        [DVec3::NAN, DVec3::X, DVec3::Y],
        [DVec3::ZERO, DVec3::ZERO, DVec3::ZERO],
    ];
    let r = section_loops(&nan, p(0.0, 0.0, 0.0), DVec3::Y);
    assert!(r.closed.is_empty());
    // A triangle lying in the plane contributes nothing.
    let r = section_loops(&[tri], DVec3::ZERO, DVec3::Z);
    assert!(r.closed.is_empty() && r.open.is_empty());
    // Zero-area triangles crossing the plane.
    let line = [p(0.0, 0.0, -1.0), p(0.0, 0.0, 1.0), p(0.0, 0.0, 0.0)];
    let _ = section_loops(&[line, line], DVec3::ZERO, DVec3::Z);
    // Filling nothing, or a short or non-finite ring.
    assert!(triangulate_section(&[], DVec3::Z).is_empty());
    assert!(triangulate_section(&[vec![vec![DVec3::ZERO, DVec3::X]]], DVec3::Z).is_empty());
    assert!(triangulate_section(&[vec![vec![DVec3::NAN; 4]]], DVec3::Z).is_empty());
    assert!(triangulate_section(&[vec![]], DVec3::Z).is_empty());
}

fn fill_of(tris: &[[DVec3; 3]], point: DVec3, normal: DVec3) -> (kernel::SectionLoops, f64) {
    let r = section_loops(tris, point, normal);
    let fill = triangulate_section(&r.closed, normal);
    let area = fill_area(&fill, normal);
    (r, area)
}

#[test]
fn a_plane_on_the_joint_of_stacked_boxes_fills_the_material_it_keeps() {
    let mut tris = box_tris(DVec3::ZERO, p(4.0, 4.0, 1.0));
    tris.extend(box_tris(p(1.0, 1.0, 1.0), p(3.0, 3.0, 2.0)));
    // Keeping the upper box: its 2 x 2 foot.
    let (r, area) = fill_of(&tris, p(0.0, 0.0, 1.0), DVec3::Z);
    assert_eq!(r.closed.len(), 1, "{r:?}");
    assert!((area - 4.0).abs() < 1e-9, "{area}");
    // Keeping the lower box: its 4 x 4 top.
    let (r, area) = fill_of(&tris, p(0.0, 0.0, 1.0), -DVec3::Z);
    assert!(r.open.is_empty(), "{r:?}");
    assert!((area - 16.0).abs() < 1e-9, "{area}");
}

#[test]
fn a_plane_through_the_step_of_an_l_shape_fills_the_material_it_keeps() {
    // A 4 x 4 slab with a 2 x 4 block on half of it.
    let mut tris = box_tris(DVec3::ZERO, p(4.0, 4.0, 1.0));
    tris.extend(box_tris(p(0.0, 0.0, 1.0), p(2.0, 4.0, 2.0)));
    let (r, area) = fill_of(&tris, p(0.0, 0.0, 1.0), DVec3::Z);
    assert!((area - 8.0).abs() < 1e-9, "{area}");
    assert_eq!(r.closed.len(), 1, "{r:?}");
    assert_eq!(r.open.len(), 1, "the slab's top beside the block: {r:?}");
    let (r, area) = fill_of(&tris, p(0.0, 0.0, 1.0), -DVec3::Z);
    assert!(r.open.is_empty(), "{r:?}");
    assert!((area - 16.0).abs() < 1e-9, "{area}");
}

#[test]
fn a_cut_across_a_face_two_boxes_share_fills_both() {
    let mut tris = box_tris(DVec3::ZERO, p(2.0, 2.0, 1.0));
    tris.extend(box_tris(p(0.0, 0.0, 1.0), p(2.0, 2.0, 2.0)));
    let (r, area) = fill_of(&tris, p(1.0, 0.0, 0.0), DVec3::X);
    assert!(r.open.is_empty(), "{r:?}");
    assert_eq!(r.closed.len(), 2, "{r:?}");
    assert!((area - 4.0).abs() < 1e-9, "{area}");
}

#[test]
fn a_cut_where_one_boxs_edge_ends_inside_anothers_still_closes() {
    // The upper box is half as wide: its bottom edge ends in the middle of the lower one's top.
    let mut tris = box_tris(DVec3::ZERO, p(2.0, 2.0, 1.0));
    tris.extend(box_tris(p(0.0, 0.0, 1.0), p(2.0, 1.0, 2.0)));
    let (r, area) = fill_of(&tris, p(1.0, 0.0, 0.0), DVec3::X);
    assert!(r.open.is_empty(), "{r:?}");
    assert!((area - 3.0).abs() < 1e-9, "{area}");
}

#[test]
fn two_regions_touching_at_a_point_are_two_fills() {
    let mut tris = box_tris(DVec3::ZERO, DVec3::ONE);
    tris.extend(box_tris(p(1.0, 1.0, 0.0), p(2.0, 2.0, 1.0)));
    let (r, area) = fill_of(&tris, p(0.0, 0.0, 0.5), DVec3::Z);
    assert!(r.open.is_empty(), "{r:?}");
    assert_eq!(r.closed.len(), 2, "{r:?}");
    assert!(r.closed.iter().all(|g| g.len() == 1 && g[0].len() == 4));
    assert!((area - 2.0).abs() < 1e-9, "{area}");
}

#[test]
fn a_cut_away_box_touching_the_plane_with_an_edge_gives_that_edge() {
    let tris = box_tris(DVec3::ZERO, DVec3::ONE);
    let n = -p(1.0, 1.0, 0.0).normalize();
    let r = section_loops(&tris, DVec3::ZERO, n);
    assert!(r.closed.is_empty(), "{r:?}");
    assert_eq!(r.open.len(), 1, "{r:?}");
    let mut ends = r.open[0].clone();
    ends.sort_by(|a, b| a.z.total_cmp(&b.z));
    assert_eq!(ends.len(), 2);
    assert!(ends[0].distance(DVec3::ZERO) < 1e-9 && ends[1].distance(DVec3::Z) < 1e-9);
}

#[test]
fn stacked_boxes_sharing_a_face_cut_vertically_fill_both() {
    // Two 2 x 2 x 1 boxes with the face they share once, as the lower box's top.
    let mut tris = box_tris(DVec3::ZERO, p(2.0, 2.0, 1.0));
    let upper = box_tris(p(0.0, 0.0, 1.0), p(2.0, 2.0, 2.0));
    tris.extend(upper.into_iter().filter(|t| !t.iter().all(|q| q.z == 1.0)));
    let (r, area) = fill_of(&tris, p(1.0, 0.0, 0.0), DVec3::X);
    assert_eq!(r.open.len(), 0, "{r:?}");
    assert!((area - 4.0).abs() < 1e-9, "{area}");
}

#[test]
fn a_partition_inside_a_box_does_not_split_the_fill() {
    // A 2 x 2 x 4 box with a face across it at z = 2, facing up.
    let mut tris = box_tris(DVec3::ZERO, p(2.0, 2.0, 4.0));
    tris.push([p(0.0, 0.0, 2.0), p(2.0, 0.0, 2.0), p(2.0, 2.0, 2.0)]);
    tris.push([p(0.0, 0.0, 2.0), p(2.0, 2.0, 2.0), p(0.0, 2.0, 2.0)]);
    let (r, area) = fill_of(&tris, p(1.0, 0.0, 0.0), DVec3::X);
    assert_eq!(r.open.len(), 0, "{r:?}");
    assert!((area - 8.0).abs() < 1e-9, "{area}");
}

/// `count` separate walls on a grid, as one soup of triangles.
fn wall_grid(count: usize) -> Vec<[DVec3; 3]> {
    let side = (count as f64).sqrt().ceil() as usize;
    (0..count)
        .flat_map(|k| {
            let (x, y) = ((k % side) as f64 * 3.0, (k / side) as f64 * 1.0);
            box_tris(p(x, y, 0.0), p(x + 2.5, y + 0.2, 2.5))
        })
        .collect()
}

fn time_cut(walls: usize) -> std::time::Duration {
    let tris = wall_grid(walls);
    let start = std::time::Instant::now();
    let r = section_loops(&tris, p(0.0, 0.0, 1.0), DVec3::Z);
    let took = start.elapsed();
    assert_eq!((r.closed.len(), r.open.len()), (walls, 0));
    took
}

#[test]
fn a_cut_through_many_walls_takes_a_moment() {
    // Debug builds are slow; the bound only catches a return to cost growing with
    // regions times triangles.
    let took = time_cut(840);
    assert!(took.as_secs_f64() < 5.0, "{took:?}");
}

#[test]
#[ignore = "timing; run with --release -- --ignored --nocapture"]
fn cut_timings_on_large_meshes() {
    for walls in [840, 1860, 4140] {
        println!(
            "{walls} walls ({} triangles): {:?}",
            walls * 12,
            time_cut(walls)
        );
    }
}
