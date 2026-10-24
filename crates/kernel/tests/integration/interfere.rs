//! Interference and clearance between parts.

use std::f64::consts::PI;

use glam::{DAffine3, DQuat, DVec3};
use kernel::bvh::triangle_distance;
use kernel::interfere::{Checker, Contact};
use kernel::{DefinitionKind, InstanceId, MeshId, Scene, Solid};

fn place(scene: &mut Scene, solid: &Solid, at: DAffine3) -> InstanceId {
    scene
        .add_solid(MeshId::Root, solid, DefinitionKind::Group, "P", at, None)
        .unwrap()
        .0
}

fn cube(size: f64) -> Solid {
    Solid::cuboid(DVec3::splat(size)).unwrap()
}

fn at(x: f64, y: f64, z: f64) -> DAffine3 {
    DAffine3::from_translation(DVec3::new(x, y, z))
}

fn contact(scene: &Scene, a: InstanceId, b: InstanceId) -> Contact {
    let mut c = Checker::new();
    let pa = c.part(scene, MeshId::Root, a).unwrap();
    let pb = c.part(scene, MeshId::Root, b).unwrap();
    c.contact(scene, &pa, &pb)
}

fn clearance(scene: &Scene, a: InstanceId, b: InstanceId) -> kernel::interfere::Clearance {
    let mut c = Checker::new();
    let pa = c.part(scene, MeshId::Root, a).unwrap();
    let pb = c.part(scene, MeshId::Root, b).unwrap();
    c.clearance(scene, &pa, &pb).unwrap()
}

#[test]
fn overlapping_boxes_share_their_common_volume() {
    let mut s = Scene::new();
    let a = place(&mut s, &cube(2.0), DAffine3::IDENTITY);
    let b = place(&mut s, &cube(2.0), at(1.0, 1.5, 1.0));
    let Contact::Overlap { volume, point } = contact(&s, a, b) else {
        panic!("expected an overlap");
    };
    assert!((volume - 0.5).abs() < 1e-9, "{volume}");
    assert!(point.cmpgt(DVec3::new(1.0, 1.5, 1.0)).all(), "{point}");
    assert!(point.cmplt(DVec3::splat(2.0)).all(), "{point}");
    let c = clearance(&s, a, b);
    assert_eq!(c.distance, 0.0);
    assert!(c.overlap);
}

#[test]
fn boxes_sharing_a_face_or_an_edge_touch() {
    let mut s = Scene::new();
    let a = place(&mut s, &cube(1.0), DAffine3::IDENTITY);
    let face = place(&mut s, &cube(1.0), at(1.0, 0.25, 0.0));
    let edge = place(&mut s, &cube(1.0), at(-1.0, 1.0, 0.0));
    let far = place(&mut s, &cube(1.0), at(3.0, 0.0, 0.0));
    assert!(matches!(contact(&s, a, face), Contact::Touching { .. }));
    assert!(matches!(contact(&s, a, edge), Contact::Touching { .. }));
    assert_eq!(contact(&s, a, far), Contact::Apart);
    let c = clearance(&s, a, face);
    assert_eq!((c.distance, c.overlap), (0.0, false));
    assert!((clearance(&s, a, far).distance - 2.0).abs() < 1e-12);

    let mut checker = Checker::new();
    let (parts, skipped) = checker.parts(&s, MeshId::Root, &[]);
    assert!(skipped.is_empty());
    let report = checker.interference(&s, &parts);
    assert!(report.overlaps.is_empty());
    let pairs: Vec<_> = report.touching.iter().map(|t| (t.a, t.b)).collect();
    assert_eq!(pairs, vec![(a, face), (a, edge)]);
}

#[test]
fn a_part_inside_another_overlaps_by_its_volume() {
    let mut s = Scene::new();
    let big = place(&mut s, &cube(4.0), DAffine3::IDENTITY);
    let small = place(&mut s, &cube(1.0), at(1.0, 1.0, 1.0));
    let flush = place(&mut s, &cube(1.0), at(0.0, 2.0, 2.0));
    let Contact::Overlap { volume, point } = contact(&s, big, small) else {
        panic!("expected an overlap");
    };
    assert!((volume - 1.0).abs() < 1e-9);
    assert!(point.cmpgt(DVec3::ONE).all() && point.cmplt(DVec3::splat(2.0)).all());
    // Inside and against one of its faces still overlaps by the whole volume.
    let Contact::Overlap { volume, .. } = contact(&s, big, flush) else {
        panic!("expected an overlap");
    };
    assert!((volume - 1.0).abs() < 1e-9);
    let c = clearance(&s, big, small);
    assert_eq!((c.distance, c.overlap), (0.0, true));
}

/// A tube of bore `bore` around a pin of radius `pin`, both 24-sided about Z; the pin is
/// longer than the tube.
fn pin_in_bore(pin: f64, bore: f64) -> (Scene, InstanceId, InstanceId) {
    let mut s = Scene::new();
    let tube = place(
        &mut s,
        &Solid::tube(0.02, bore, 0.03, 24).unwrap(),
        DAffine3::IDENTITY,
    );
    let pin = place(
        &mut s,
        &Solid::cylinder(pin, 0.05, 24).unwrap(),
        at(0.0, 0.0, -0.01),
    );
    (s, tube, pin)
}

#[test]
fn a_pin_with_clearance_in_its_bore_is_apart() {
    let (s, tube, pin) = pin_in_bore(0.01, 0.0101);
    assert_eq!(contact(&s, tube, pin), Contact::Apart);
    let c = clearance(&s, tube, pin);
    // The facets are parallel: the radial gap times the cosine of half a side's angle.
    let expected = 0.0001 * (PI / 24.0).cos();
    assert!((c.distance - expected).abs() < 1e-12, "{}", c.distance);
    assert!((c.a.distance(c.b) - c.distance).abs() < 1e-12);
    assert!(!c.overlap);
}

#[test]
fn a_pin_the_size_of_its_bore_touches() {
    let (s, tube, pin) = pin_in_bore(0.01, 0.01);
    assert!(matches!(contact(&s, tube, pin), Contact::Touching { .. }));
}

#[test]
fn an_oversized_pin_interferes_by_the_ring_volume() {
    let (s, tube, pin) = pin_in_bore(0.0102, 0.01);
    let Contact::Overlap { volume, point } = contact(&s, tube, pin) else {
        panic!("expected an overlap");
    };
    let polygon = |r: f64| 12.0 * r * r * (2.0 * PI / 24.0).sin();
    let expected = (polygon(0.0102) - polygon(0.01)) * 0.03;
    assert!((volume - expected).abs() < 1e-12, "{volume} vs {expected}");
    let r = point.truncate().length();
    assert!(
        r > 0.0099 && r < 0.0102 && point.z > 0.0 && point.z < 0.03,
        "{point}"
    );
}

#[test]
fn skew_cylinders_are_their_gap_apart() {
    let mut s = Scene::new();
    let r = 0.1;
    let cyl = Solid::cylinder(r, 2.0, 24).unwrap();
    // One along X at z = 0, one along Y at z = 0.5.
    let along_x = DAffine3::from_rotation_translation(
        DQuat::from_rotation_y(PI / 2.0),
        DVec3::new(-1.0, 0.0, 0.0),
    );
    let along_y = DAffine3::from_rotation_translation(
        DQuat::from_rotation_x(-PI / 2.0),
        DVec3::new(0.0, -1.0, 0.5),
    );
    let a = place(&mut s, &cyl, along_x);
    let b = place(&mut s, &cyl, along_y);
    let c = clearance(&s, a, b);
    let (lo, hi) = (0.5 - 2.0 * r, 0.5 - 2.0 * r * (PI / 24.0).cos());
    assert!(
        c.distance >= lo - 1e-12 && c.distance <= hi + 1e-12,
        "{}",
        c.distance
    );
    assert!((c.a.distance(c.b) - c.distance).abs() < 1e-12);
    // The tree agrees with every triangle pair.
    let tris = |id: InstanceId| {
        let inst = s.mesh(MeshId::Root).unwrap().instance(id).unwrap().clone();
        let m = &s.definition(inst.definition).unwrap().mesh;
        let mut out = Vec::new();
        for (f, _) in m.faces() {
            for t in m.triangulate(f).unwrap() {
                out.push(t.map(|p| inst.transform.transform_point3(p)));
            }
        }
        out
    };
    let (ta, tb) = (tris(a), tris(b));
    let brute = ta
        .iter()
        .flat_map(|t| tb.iter().map(|u| triangle_distance(*t, *u).distance))
        .fold(f64::INFINITY, f64::min);
    assert!(
        (c.distance - brute).abs() < 1e-12,
        "{} vs {brute}",
        c.distance
    );
    assert_eq!(contact(&s, a, b), Contact::Apart);
}

#[test]
fn nested_instances_count_as_parts_of_their_root_instance() {
    let mut s = Scene::new();
    // A box group whose definition also places a box at x = 3.
    let assembly = place(&mut s, &cube(1.0), DAffine3::IDENTITY);
    let inner = place(&mut s, &cube(1.0), at(10.0, 0.0, 0.0));
    let root = s.mesh(MeshId::Root).unwrap();
    let outer = root.instance(assembly).unwrap().definition;
    let def = root.instance(inner).unwrap().definition;
    s.place_instance(MeshId::Definition(outer), def, at(3.0, 0.0, 0.0))
        .unwrap();
    s.erase_instances(MeshId::Root, &[inner]);
    let probe = place(&mut s, &cube(1.0), at(3.5, 0.5, 0.5));
    let Contact::Overlap { volume, point } = contact(&s, assembly, probe) else {
        panic!("expected an overlap");
    };
    assert!((volume - 0.125).abs() < 1e-9);
    assert!(point.x > 3.5 && point.x < 4.0);
}

#[test]
fn an_open_part_is_skipped_with_its_reason() {
    let mut s = Scene::new();
    let a = place(&mut s, &cube(1.0), DAffine3::IDENTITY);
    let b = place(&mut s, &cube(1.0), at(0.5, 0.0, 0.0));
    let def = s
        .mesh(MeshId::Root)
        .unwrap()
        .instance(b)
        .unwrap()
        .definition;
    let f = s.definition(def).unwrap().mesh.faces().next().unwrap().0;
    s.edit(MeshId::Definition(def), |m| {
        m.erase(&[], &[f]);
        Ok(m.take_changes())
    })
    .unwrap();
    let mut checker = Checker::new();
    let (parts, skipped) = checker.parts(&s, MeshId::Root, &[]);
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].id, a);
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].0, b);
    assert!(skipped[0].1.contains("bounds 1 face"), "{}", skipped[0].1);
}

#[test]
fn the_checker_reuses_its_trees_across_poses() {
    let mut s = Scene::new();
    let a = place(&mut s, &cube(1.0), DAffine3::IDENTITY);
    let b = place(&mut s, &cube(1.0), at(2.0, 0.0, 0.0));
    let mut checker = Checker::new();
    let pa = checker.part(&s, MeshId::Root, a).unwrap();
    let mut pb = checker.part(&s, MeshId::Root, b).unwrap();
    assert!((checker.clearance(&s, &pa, &pb).unwrap().distance - 1.0).abs() < 1e-12);
    // Move the second part by its placement alone: no scene change needed.
    pb.shells[0].place = at(0.5, 0.0, 0.0);
    assert!(matches!(
        checker.contact(&s, &pa, &pb),
        Contact::Overlap { .. }
    ));
}

#[test]
fn turned_boxes_against_each_other_touch_and_mirrors_overlap() {
    let mut s = Scene::new();
    let turn = DQuat::from_rotation_z(0.523) * DQuat::from_rotation_x(0.2);
    let a = place(
        &mut s,
        &cube(1.0),
        DAffine3::from_rotation_translation(turn, DVec3::new(0.3, 0.1, 0.2)),
    );
    // The same turn, one box length further along its own x and half a box along its y.
    let b = place(
        &mut s,
        &cube(1.0),
        DAffine3::from_rotation_translation(
            turn,
            DVec3::new(0.3, 0.1, 0.2) + turn * DVec3::new(1.0, 0.5, 0.0),
        ),
    );
    assert!(matches!(contact(&s, a, b), Contact::Touching { .. }));
    // A mirrored box reaching a quarter into the first one.
    let m = place(
        &mut s,
        &cube(1.0),
        DAffine3::from_rotation_translation(
            turn,
            DVec3::new(0.3, 0.1, 0.2) + turn * DVec3::new(0.75, 0.0, 0.0),
        ) * DAffine3::from_scale(DVec3::new(-1.0, 1.0, 1.0))
            * at(-1.0, 0.0, 0.0),
    );
    let Contact::Overlap { volume, .. } = contact(&s, a, m) else {
        panic!("expected an overlap");
    };
    assert!((volume - 0.25).abs() < 1e-9, "{volume}");
}
