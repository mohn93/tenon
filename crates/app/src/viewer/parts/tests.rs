use glam::{DAffine3, DQuat, DVec3};
use kernel::{DefinitionKind, InstanceId, MeshId, Model, Scene, Tag};
use session::{LengthUnit, Units};

use super::*;

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn close(a: [f64; 3], b: [f64; 3]) {
    for i in 0..3 {
        assert!((a[i] - b[i]).abs() < 1e-9, "{a:?} vs {b:?}");
    }
}

/// A scene whose root holds a polygon, made into one group or component.
fn one(corners: &[DVec3], extrude: f64, kind: DefinitionKind, name: &str) -> (Scene, InstanceId) {
    let mut m = Model::new();
    m.add_polygon(corners).unwrap();
    if extrude != 0.0 {
        let face = m.faces().next().unwrap().0;
        m.push_pull(face, extrude).unwrap();
    }
    let mut s = Scene::from_root(m);
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
    let (id, _) = s
        .make_group(MeshId::Root, &edges, &faces, &[], kind, name)
        .unwrap();
    (s, id)
}

fn place(s: &mut Scene, id: InstanceId, t: DAffine3) {
    s.edit(MeshId::Root, |m| Ok(m.transform_instances(&[id], t)))
        .unwrap();
}

fn rect(w: f64, d: f64) -> Vec<DVec3> {
    vec![
        p(0.0, 0.0, 0.0),
        p(w, 0.0, 0.0),
        p(w, d, 0.0),
        p(0.0, d, 0.0),
    ]
}

#[test]
fn a_group_is_listed_with_its_name_and_size() {
    let (s, id) = one(&rect(2.0, 1.0), 3.0, DefinitionKind::Group, "Shed");
    let parts = list(&s);
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].id, id);
    assert_eq!(parts[0].name, "Shed");
    assert_eq!(parts[0].kind, DefinitionKind::Group);
    close(parts[0].size.unwrap(), [2.0, 1.0, 3.0]);
    assert_eq!(parts[0].ordinal(), None, "groups have no 1 of N");
}

#[test]
fn unnamed_parts_are_called_group_and_component() {
    let (s, _) = one(&rect(1.0, 1.0), 0.0, DefinitionKind::Group, "");
    assert_eq!(list(&s)[0].name, "Group");
    let (s, _) = one(&rect(1.0, 1.0), 0.0, DefinitionKind::Component, "  ");
    assert_eq!(list(&s)[0].name, "Component");
}

#[test]
fn a_flat_part_has_no_height() {
    let (s, _) = one(&rect(2.0, 1.0), 0.0, DefinitionKind::Group, "Plate");
    close(list(&s)[0].size.unwrap(), [2.0, 1.0, 0.0]);
}

/// The 2 x 1 x 3 shed placed by `t`; its size as listed.
fn shed_size(t: DAffine3) -> [f64; 3] {
    let (mut s, id) = one(&rect(2.0, 1.0), 3.0, DefinitionKind::Group, "Shed");
    place(&mut s, id, t);
    list(&s)[0].size.unwrap()
}

#[test]
fn the_size_is_where_the_part_is_placed_not_the_definitions_box() {
    use std::f64::consts::FRAC_PI_2;
    // Moving changes no size.
    close(
        shed_size(DAffine3::from_translation(p(40.0, -7.0, 2.0))),
        [2.0, 1.0, 3.0],
    );
    // Turned a quarter about z, width and depth swap.
    close(
        shed_size(DAffine3::from_quat(DQuat::from_rotation_z(FRAC_PI_2))),
        [1.0, 2.0, 3.0],
    );
    // Laid on its side (a quarter about x): depth and height swap.
    close(
        shed_size(DAffine3::from_quat(DQuat::from_rotation_x(FRAC_PI_2))),
        [2.0, 3.0, 1.0],
    );
    // Laid on its other side (a quarter about y): width and height swap.
    close(
        shed_size(DAffine3::from_quat(DQuat::from_rotation_y(FRAC_PI_2))),
        [3.0, 1.0, 2.0],
    );
}

#[test]
fn a_turned_part_is_boxed_by_its_geometry_not_by_its_turned_box() {
    // A diamond of side sqrt 2 (corners on the axes) has a 2 x 2 box; turned 45 degrees its
    // corners land on the diagonals, so its real box is sqrt 2 square. The turned 2 x 2 box
    // would be 2.83 wide.
    let diamond = vec![
        p(1.0, 0.0, 0.0),
        p(2.0, 1.0, 0.0),
        p(1.0, 2.0, 0.0),
        p(0.0, 1.0, 0.0),
    ];
    let (mut s, id) = one(&diamond, 0.0, DefinitionKind::Group, "Diamond");
    close(list(&s)[0].size.unwrap(), [2.0, 2.0, 0.0]);
    let turn = DAffine3::from_translation(p(1.0, 1.0, 0.0))
        * DAffine3::from_quat(DQuat::from_rotation_z(std::f64::consts::FRAC_PI_4))
        * DAffine3::from_translation(p(-1.0, -1.0, 0.0));
    place(&mut s, id, turn);
    let side = 2f64.sqrt();
    close(list(&s)[0].size.unwrap(), [side, side, 0.0]);
    // The kernel's own box of the instance is the looser one.
    let inst = s.root().instance(id).unwrap();
    let [lo, hi] = s.instance_bounds(inst).unwrap();
    assert!((hi - lo).x > 2.8, "the box of the box is {:?}", hi - lo);
}

#[test]
fn a_scaled_part_is_scaled_per_axis() {
    close(
        shed_size(DAffine3::from_scale(p(2.0, 3.0, 0.5))),
        [4.0, 3.0, 1.5],
    );
    // Scaled and turned together: x doubled (4 wide), then a quarter turn about z.
    let t = DAffine3::from_quat(DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2))
        * DAffine3::from_scale(p(2.0, 1.0, 1.0));
    close(shed_size(t), [1.0, 4.0, 3.0]);
    // Turned 45 degrees about z and scaled unevenly: the box of the placed corners.
    let t = DAffine3::from_quat(DQuat::from_rotation_z(std::f64::consts::FRAC_PI_4))
        * DAffine3::from_scale(p(1.0, 3.0, 1.0));
    let half = std::f64::consts::FRAC_1_SQRT_2;
    // Corners (0,0) (2,0) (2,3) (0,3) after scaling, then turned.
    let xs = [0.0, 2.0 * half, 2.0 * half - 3.0 * half, -3.0 * half];
    let ys = [0.0, 2.0 * half, 2.0 * half + 3.0 * half, 3.0 * half];
    let range = |v: [f64; 4]| {
        v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min)
    };
    close(shed_size(t), [range(xs), range(ys), 3.0]);
}

#[test]
fn a_mirrored_part_has_a_positive_size() {
    let (mut s, id) = one(&rect(2.0, 1.0), 3.0, DefinitionKind::Group, "Shed");
    place(&mut s, id, DAffine3::from_scale(p(-1.0, 1.0, 1.0)));
    close(list(&s)[0].size.unwrap(), [2.0, 1.0, 3.0]);
}

#[test]
fn nested_parts_count_toward_the_size_but_are_not_parts() {
    let (mut s, outer) = one(&rect(2.0, 1.0), 1.0, DefinitionKind::Group, "Outer");
    // A group inside the first, a square 5 m along x.
    let def = MeshId::Definition(s.root().instance(outer).unwrap().definition);
    let known: Vec<_> = s.mesh(def).unwrap().edges().map(|(id, _)| id).collect();
    s.edit(def, |m| {
        m.add_polygon(&[
            p(5.0, 0.0, 0.0),
            p(6.0, 0.0, 0.0),
            p(6.0, 1.0, 0.0),
            p(5.0, 1.0, 0.0),
        ])
    })
    .unwrap();
    let edges: Vec<_> = s
        .mesh(def)
        .unwrap()
        .edges()
        .map(|(id, _)| id)
        .filter(|id| !known.contains(id))
        .collect();
    let face = s
        .mesh(def)
        .unwrap()
        .faces()
        .map(|(id, _)| id)
        .last()
        .unwrap();
    s.make_group(def, &edges, &[face], &[], DefinitionKind::Group, "Inner")
        .unwrap();
    let parts = list(&s);
    assert_eq!(parts.len(), 1, "only the top level");
    assert_eq!(parts[0].name, "Outer");
    close(parts[0].size.unwrap(), [6.0, 1.0, 1.0]);
}

#[test]
fn components_count_their_top_level_instances() {
    let (mut s, first) = one(&rect(1.0, 1.0), 1.0, DefinitionKind::Component, "Crate");
    s.copy_instances(MeshId::Root, &[first], p(3.0, 0.0, 0.0), 2)
        .unwrap();
    let parts = list(&s);
    assert_eq!(parts.len(), 3);
    for (i, part) in parts.iter().enumerate() {
        assert_eq!(part.name, "Crate");
        assert_eq!(part.number, i + 1);
        assert_eq!(part.count, 3);
        assert_eq!(part.ordinal().unwrap(), format!("{} of 3", i + 1));
    }
    // A lone component is "1 of 1".
    let (s, _) = one(&rect(1.0, 1.0), 1.0, DefinitionKind::Component, "Crate");
    assert_eq!(list(&s)[0].ordinal().unwrap(), "1 of 1");
}

#[test]
fn rows_group_a_components_instances_and_keep_groups_apart() {
    let (mut s, first) = one(&rect(1.0, 1.0), 1.0, DefinitionKind::Component, "Crate");
    s.copy_instances(MeshId::Root, &[first], p(3.0, 0.0, 0.0), 1)
        .unwrap();
    let parts = list(&s);
    let rows = rows(&parts);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "Crate");
    assert_eq!(rows[0].count, 2);
    assert_eq!(rows[0].ids, parts.iter().map(|p| p.id).collect::<Vec<_>>());
    close(rows[0].size.unwrap(), [1.0, 1.0, 1.0]);
}

#[test]
fn groups_are_rows_of_their_own() {
    let parts = vec![
        Part {
            id: InstanceId(1),
            definition: kernel::DefinitionId(1),
            kind: DefinitionKind::Group,
            name: "A".into(),
            size: None,
            number: 1,
            count: 1,
        },
        Part {
            id: InstanceId(2),
            definition: kernel::DefinitionId(1),
            kind: DefinitionKind::Group,
            name: "A".into(),
            size: None,
            number: 2,
            count: 2,
        },
    ];
    let rows = rows(&parts);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.count == 1));
}

#[test]
fn a_part_on_a_hidden_tag_is_not_listed() {
    let (mut s, id) = one(&rect(1.0, 1.0), 1.0, DefinitionKind::Group, "Shed");
    let (tag, _) = s
        .add_tag(Tag {
            name: "Hidden".into(),
            color: [1, 2, 3],
            visible: false,
        })
        .unwrap();
    s.set_tags(MeshId::Root, &[kernel::EntityId::Instance(id)], Some(tag))
        .unwrap();
    assert!(list(&s).is_empty());
}

#[test]
fn sizes_are_written_in_the_documents_units() {
    let size = [1.2, 0.6, 2.0];
    assert_eq!(size_text(size, &Units::default()), "1.2 m x 0.6 m x 2 m");
    let cm = Units {
        length: LengthUnit::Centimeters,
        precision: 1,
    };
    assert_eq!(size_text(size, &cm), "120 cm x 60 cm x 200 cm");
    let mm = Units {
        length: LengthUnit::Millimeters,
        precision: 0,
    };
    assert_eq!(size_text(size, &mm), "1200 mm x 600 mm x 2000 mm");
    let inches = Units {
        length: LengthUnit::Inches,
        precision: 2,
    };
    assert_eq!(
        size_text([0.0254, 0.0508, 0.0762], &inches),
        "1\" x 2\" x 3\""
    );
    let ft = Units {
        length: LengthUnit::FeetInches,
        precision: 4,
    };
    assert_eq!(size_text([0.3048, 0.0254, 0.6096], &ft), "1' x 1\" x 2'");
}
