use std::collections::BTreeSet;
use std::f64::consts::TAU;

use glam::{DAffine3, DQuat, DVec3};
use kernel::{
    Anchor, DefinitionKind, Dimension, DimensionId, EntityId, KernelError, MeshId, Model, Note,
    NoteId, Parts, RadialAnchor, Scene, SceneData, Tag, VertexId,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// The vertex at `at`.
fn vid(m: &Model, at: DVec3) -> VertexId {
    m.vertices()
        .find(|(_, v)| v.pos.distance(at) < 1e-9)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no vertex at {at}"))
}

fn linear(a: Anchor, b: Anchor, offset: DVec3) -> Dimension {
    Dimension::Linear {
        a,
        b,
        offset,
        tag: None,
    }
}

fn leader(anchor: Anchor, position: DVec3, text: Option<&str>) -> Note {
    Note::Leader {
        anchor,
        position,
        text: text.map(str::to_string),
        tag: None,
    }
}

fn screen(text: &str) -> Note {
    Note::Screen {
        at: [0.25, 0.75],
        text: text.to_string(),
        tag: None,
    }
}

fn created_dimension(created: &[EntityId]) -> DimensionId {
    let found: Vec<DimensionId> = created
        .iter()
        .filter_map(|id| match id {
            EntityId::Dimension(d) => Some(*d),
            _ => None,
        })
        .collect();
    match found[..] {
        [d] => d,
        _ => panic!("expected one dimension, got {created:?}"),
    }
}

fn created_note(created: &[EntityId]) -> NoteId {
    let found: Vec<NoteId> = created
        .iter()
        .filter_map(|id| match id {
            EntityId::Note(n) => Some(*n),
            _ => None,
        })
        .collect();
    match found[..] {
        [n] => n,
        _ => panic!("expected one note, got {created:?}"),
    }
}

/// A 4 x 2 rectangle on the ground with its corner at `at`.
fn rectangle(m: &mut Model, at: DVec3) {
    m.add_polygon(&[
        at,
        at + p(4.0, 0.0, 0.0),
        at + p(4.0, 2.0, 0.0),
        at + p(0.0, 2.0, 0.0),
    ])
    .unwrap();
}

/// A model with a rectangle and a dimension along its bottom edge.
fn dimensioned_rectangle() -> (Model, DimensionId) {
    let mut m = Model::new();
    rectangle(&mut m, DVec3::ZERO);
    let (a, b) = (vid(&m, DVec3::ZERO), vid(&m, p(4.0, 0.0, 0.0)));
    let d = created_dimension(
        &m.add_dimension(linear(
            Anchor::Vertex(a),
            Anchor::Vertex(b),
            p(0.0, -1.0, 0.0),
        ))
        .unwrap()
        .summary()
        .created,
    );
    m.validate().unwrap();
    (m, d)
}

/// The points of a circle of radius `r` around `c` on the ground, in `n` segments.
fn circle_points(c: DVec3, r: f64, n: usize) -> Vec<DVec3> {
    (0..n)
        .map(|k| {
            let a = TAU * k as f64 / n as f64;
            c + p(r * a.cos(), r * a.sin(), 0.0)
        })
        .collect()
}

fn all_edges(m: &Model) -> Vec<kernel::EdgeId> {
    m.edges().map(|(id, _)| id).collect()
}

fn all_faces(m: &Model) -> Vec<kernel::FaceId> {
    m.faces().map(|(id, _)| id).collect()
}

#[test]
fn annotations_are_added_moved_and_erased_as_undoable_changes() {
    let (m, d) = dimensioned_rectangle();
    assert_eq!(m.dimension_points(d), Some((DVec3::ZERO, p(4.0, 0.0, 0.0))));
    assert_eq!(
        m.dimension_line(d),
        Some((p(0.0, -1.0, 0.0), p(4.0, -1.0, 0.0)))
    );
    let top = vid(&m, p(4.0, 2.0, 0.0));
    // Notes go in through the scene (screen notes only into the root).
    let mut s = Scene::from_root(m);
    let added = s
        .add_note(
            MeshId::Root,
            leader(Anchor::Vertex(top), p(5.0, 3.0, 0.0), None),
        )
        .unwrap();
    let n2 = created_note(&added.summary().created);
    assert_eq!(s.root().note_point(n2), Some(p(4.0, 2.0, 0.0)));
    s.validate().unwrap();
    let before = s.clone();

    let t = DAffine3::from_translation(p(0.0, 0.0, 2.0));
    let moved = s
        .edit(MeshId::Root, |m| {
            Ok(m.transform_annotations(&[d], &[n2], t))
        })
        .unwrap();
    let summary = moved.summary();
    assert_eq!(summary.modified, vec![EntityId::Note(n2)]);
    // The dimension's ends are vertices and its offset is horizontal: a lift leaves it be.
    assert_eq!(s.root().dimension(d), before.root().dimension(d));
    assert_eq!(
        s.root().note(n2),
        Some(&leader(Anchor::Vertex(top), p(5.0, 3.0, 2.0), None))
    );
    s.apply(&moved.inverse());
    assert_eq!(s, before);

    let erased = s
        .edit(MeshId::Root, |m| Ok(m.erase_annotations(&[d], &[n2])))
        .unwrap();
    let summary = erased.summary();
    assert_eq!(
        summary.deleted,
        vec![EntityId::Dimension(d), EntityId::Note(n2)]
    );
    assert!(s.root().dimension(d).is_none() && s.root().note(n2).is_none());
    s.validate().unwrap();
    s.apply(&erased.inverse());
    assert_eq!(s, before);
}

#[test]
fn point_anchors_offsets_and_fixed_circles_move_with_a_transform() {
    let mut m = Model::new();
    let d = created_dimension(
        &m.add_dimension(linear(
            Anchor::Point(DVec3::ZERO),
            Anchor::Point(p(2.0, 0.0, 0.0)),
            p(0.0, 1.0, 0.0),
        ))
        .unwrap()
        .summary()
        .created,
    );
    let r = created_dimension(
        &m.add_dimension(Dimension::Radial {
            curve: RadialAnchor::Fixed {
                center: DVec3::ZERO,
                normal: p(0.0, 0.0, 3.0),
                radius: 1.0,
            },
            diameter: true,
            leader: p(2.0, 0.0, 0.0),
            tag: None,
        })
        .unwrap()
        .summary()
        .created,
    );
    let turn =
        DAffine3::from_rotation_translation(DQuat::from_rotation_z(TAU / 4.0), p(10.0, 0.0, 0.0));
    m.transform_annotations(&[d, r], &[], turn);
    m.validate().unwrap();
    let (a, b) = m.dimension_points(d).unwrap();
    assert!(a.distance(p(10.0, 0.0, 0.0)) < 1e-12);
    assert!(b.distance(p(10.0, 2.0, 0.0)) < 1e-12);
    let (la, _) = m.dimension_line(d).unwrap();
    assert!(la.distance(p(9.0, 0.0, 0.0)) < 1e-12);
    let c = m.radial_circle(r).unwrap();
    assert!(c.center.distance(p(10.0, 0.0, 0.0)) < 1e-12);
    assert!(
        c.normal.distance(DVec3::Z) < 1e-12,
        "the normal is kept a unit vector"
    );
    assert!((c.radius - 1.0).abs() < 1e-12);
    let scale = DAffine3::from_scale(DVec3::splat(2.0));
    assert!(m.transform_annotations(&[], &[], scale).is_empty());
    m.transform_annotations(&[r], &[], scale);
    assert!((m.radial_circle(r).unwrap().radius - 2.0).abs() < 1e-12);
}

#[test]
fn invalid_annotations_are_refused() {
    let (mut m, _) = dimensioned_rectangle();
    let a = vid(&m, DVec3::ZERO);
    let before = m.clone();
    assert_eq!(
        m.add_dimension(linear(
            Anchor::Vertex(a),
            Anchor::Point(DVec3::ZERO),
            DVec3::Y
        )),
        Err(KernelError::ZeroDistance),
        "ends that coincide"
    );
    assert_eq!(
        m.add_dimension(linear(Anchor::Vertex(a), Anchor::Vertex(a), DVec3::Y)),
        Err(KernelError::ZeroDistance)
    );
    assert!(matches!(
        m.add_dimension(linear(
            Anchor::Vertex(VertexId(999)),
            Anchor::Point(DVec3::Z),
            DVec3::Y
        )),
        Err(KernelError::Annotation(e)) if e.contains("v999")
    ));
    assert_eq!(
        m.add_dimension(linear(
            Anchor::Point(p(f64::NAN, 0.0, 0.0)),
            Anchor::Point(DVec3::Z),
            DVec3::Y
        )),
        Err(KernelError::NotFinite)
    );
    let edge_curve = m.edges().next().unwrap().1.curve;
    assert!(edge_curve.is_none());
    assert!(matches!(
        m.add_dimension(Dimension::Radial {
            curve: RadialAnchor::Curve(kernel::CurveId(5000)),
            diameter: false,
            leader: DVec3::X,
            tag: None,
        }),
        Err(KernelError::Annotation(_))
    ));
    assert_eq!(
        m.add_dimension(Dimension::Radial {
            curve: RadialAnchor::Fixed {
                center: DVec3::ZERO,
                normal: DVec3::ZERO,
                radius: 1.0
            },
            diameter: false,
            leader: DVec3::X,
            tag: None,
        }),
        Err(KernelError::NotFinite)
    );
    assert_eq!(m, before, "nothing was added");

    let mut s = Scene::from_root(m);
    let long = "x".repeat(kernel::MAX_NOTE_CHARS + 1);
    let lines = "x\n".repeat(kernel::MAX_NOTE_LINES + 1);
    for text in ["", "  \n ", long.as_str(), lines.as_str()] {
        assert!(
            matches!(
                s.add_note(MeshId::Root, screen(text)),
                Err(KernelError::Annotation(_))
            ),
            "{text:?} was taken"
        );
        assert!(matches!(
            s.add_note(
                MeshId::Root,
                leader(Anchor::Point(DVec3::ZERO), DVec3::X, Some(text))
            ),
            Err(KernelError::Annotation(_))
        ));
    }
    let longest = "y".repeat(kernel::MAX_NOTE_CHARS);
    s.add_note(MeshId::Root, screen(&longest)).unwrap();
    assert!(matches!(
        s.add_note(
            MeshId::Root,
            Note::Screen {
                at: [1.5, 0.0],
                text: "off the view".into(),
                tag: None
            }
        ),
        Err(KernelError::Annotation(_))
    ));
    s.validate().unwrap();
}

#[test]
fn note_text_changes_and_screen_notes_need_text() {
    let mut s = Scene::new();
    let leader_id = created_note(
        &s.add_note(
            MeshId::Root,
            leader(Anchor::Point(DVec3::ZERO), DVec3::X, Some("Door")),
        )
        .unwrap()
        .summary()
        .created,
    );
    let screen_id = created_note(
        &s.add_note(MeshId::Root, screen("Title"))
            .unwrap()
            .summary()
            .created,
    );
    let before = s.clone();
    let c = s
        .edit(MeshId::Root, |m| m.set_note_text(leader_id, None))
        .unwrap();
    assert_eq!(c.summary().modified, vec![EntityId::Note(leader_id)]);
    assert_eq!(s.root().note(leader_id).unwrap().text(), None);
    s.apply(&c.inverse());
    assert_eq!(s, before);
    let same = s
        .edit(MeshId::Root, |m| {
            m.set_note_text(leader_id, Some("Door".into()))
        })
        .unwrap();
    assert!(same.is_empty(), "unchanged text changes nothing");
    assert!(matches!(
        s.edit(MeshId::Root, |m| m.set_note_text(screen_id, None)),
        Err(KernelError::Annotation(_))
    ));
    s.edit(MeshId::Root, |m| {
        m.set_note_text(screen_id, Some("Plan".into()))
    })
    .unwrap();
    assert_eq!(s.root().note(screen_id).unwrap().text(), Some("Plan"));
    assert!(matches!(
        s.edit(MeshId::Root, |m| m.set_note_text(NoteId(999), None)),
        Err(KernelError::Annotation(_))
    ));
}

#[test]
fn an_erased_end_vertex_freezes_its_anchor_and_undo_attaches_it_again() {
    // Review focus 1: a dimension along an edge whose one end goes.
    let mut m = Model::new();
    let (a, b, c) = (DVec3::ZERO, p(3.0, 0.0, 0.0), p(3.0, 2.0, 0.0));
    m.add_edge(a, b).unwrap();
    m.add_edge(b, c).unwrap();
    let (va, vb) = (vid(&m, a), vid(&m, b));
    let d = created_dimension(
        &m.add_dimension(linear(Anchor::Vertex(va), Anchor::Vertex(vb), -DVec3::Y))
            .unwrap()
            .summary()
            .created,
    );
    let attached = m.clone();
    let ab = m.edge_between(va, vb).unwrap();
    let erased = m.erase(&[ab], &[]);
    m.validate().unwrap();
    assert!(
        m.position(va).is_none(),
        "the end vertex went with its edge"
    );
    assert_eq!(
        m.dimension(d),
        Some(&linear(Anchor::Point(a), Anchor::Vertex(vb), -DVec3::Y)),
        "frozen where the vertex was, in the same change set"
    );
    assert_eq!(m.dimension_points(d), Some((a, b)));
    assert!(erased.summary().modified.contains(&EntityId::Dimension(d)));
    let frozen = m.clone();

    m.apply(&erased.inverse());
    assert_eq!(m, attached, "undo attaches it again");
    m.apply(&erased);
    assert_eq!(m, frozen, "redo freezes it exactly as before");

    // Only the attached end follows the remaining vertex.
    m.move_vertices(&BTreeSet::from([vb]), p(0.0, 0.0, 1.0))
        .unwrap();
    assert_eq!(m.dimension_points(d), Some((a, p(3.0, 0.0, 1.0))));
    m.validate().unwrap();
}

#[test]
fn a_healed_vertex_freezes_a_leader_note() {
    // a-m-b in a line with a branch m-c: erasing the branch heals m away.
    let mut m = Model::new();
    let (a, mid, b, c) = (
        DVec3::ZERO,
        p(1.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
    );
    m.add_edge(a, b).unwrap();
    m.add_edge(mid, c).unwrap();
    let vm = vid(&m, mid);
    let mut s = Scene::from_root(m);
    let n = created_note(
        &s.add_note(
            MeshId::Root,
            leader(Anchor::Vertex(vm), p(1.0, -1.0, 0.0), None),
        )
        .unwrap()
        .summary()
        .created,
    );
    let branch = s.root().edge_between(vm, vid(s.root(), c)).unwrap();
    let before = s.clone();
    let erased = s
        .edit(MeshId::Root, |m| Ok(m.erase(&[branch], &[])))
        .unwrap();
    s.validate().unwrap();
    assert!(
        s.root().position(vm).is_none(),
        "the line healed over the vertex"
    );
    assert_eq!(
        s.root().note(n),
        Some(&leader(Anchor::Point(mid), p(1.0, -1.0, 0.0), None))
    );
    s.apply(&erased.inverse());
    assert_eq!(s, before);
}

#[test]
fn a_radial_dimension_follows_its_circle_and_keeps_the_last_circle_when_it_breaks() {
    let mut m = Model::new();
    let points = circle_points(DVec3::ZERO, 2.0, 24);
    m.add_curve(&points, true, false).unwrap();
    let edge = m.edges().next().unwrap().0;
    let curve = m.edge(edge).unwrap().curve.unwrap();
    let r = created_dimension(
        &m.add_dimension(Dimension::Radial {
            curve: RadialAnchor::Curve(curve),
            diameter: false,
            leader: p(3.0, 0.0, 0.0),
            tag: None,
        })
        .unwrap()
        .summary()
        .created,
    );
    assert!((m.radial_circle(r).unwrap().radius - 2.0).abs() < 1e-9);

    // The whole circle moves: the dimension follows.
    let every: BTreeSet<VertexId> = m.vertices().map(|(id, _)| id).collect();
    m.move_vertices(&every, p(5.0, 0.0, 0.0)).unwrap();
    let c = m.radial_circle(r).unwrap();
    assert!(c.center.distance(p(5.0, 0.0, 0.0)) < 1e-9);
    assert_eq!(
        m.dimension(r).map(|d| matches!(
            d,
            Dimension::Radial {
                curve: RadialAnchor::Curve(_),
                ..
            }
        )),
        Some(true)
    );

    // One vertex moves off the circle: the curve is no longer one, so the dimension keeps
    // the circle it had, in the same change set.
    let attached = m.clone();
    let first = vid(&m, points[0] + p(5.0, 0.0, 0.0));
    let bent = m
        .move_vertices(&BTreeSet::from([first]), p(0.5, 0.0, 0.0))
        .unwrap();
    m.validate().unwrap();
    match m.dimension(r) {
        Some(Dimension::Radial {
            curve: RadialAnchor::Fixed { center, radius, .. },
            ..
        }) => {
            assert!(center.distance(p(5.0, 0.0, 0.0)) < 1e-9);
            assert!((radius - 2.0).abs() < 1e-9);
        }
        other => panic!("not frozen: {other:?}"),
    }
    assert!(bent.summary().modified.contains(&EntityId::Dimension(r)));
    let frozen = m.clone();
    m.apply(&bent.inverse());
    assert_eq!(m, attached, "undo attaches it again");
    m.apply(&bent);
    assert_eq!(m, frozen, "redo is exact");
    m.apply(&bent.inverse());

    // Erasing the circle's edges freezes it too.
    let edges = all_edges(&m);
    m.erase(&edges, &[]);
    m.validate().unwrap();
    assert!(matches!(
        m.dimension(r),
        Some(Dimension::Radial {
            curve: RadialAnchor::Fixed { .. },
            ..
        })
    ));
}

#[test]
fn copies_anchor_to_the_copied_vertices() {
    // Review focus 2.
    let (mut m, d) = dimensioned_rectangle();
    let (a, b) = (vid(&m, DVec3::ZERO), vid(&m, p(4.0, 0.0, 0.0)));
    let (edges, faces) = (all_edges(&m), all_faces(&m));
    let copied = m.vertices_of(&edges, &faces);
    assert_eq!(copied.len(), 4);
    let places = [DAffine3::from_translation(p(0.0, 5.0, 0.0))];
    m.copy_placed(&edges, &faces, &places).unwrap();
    let c = m.copy_annotations(&[d], &[], &places, &copied);
    let copy = created_dimension(&c.summary().created);
    let (ca, cb) = (vid(&m, p(0.0, 5.0, 0.0)), vid(&m, p(4.0, 5.0, 0.0)));
    assert_eq!(
        m.dimension(copy),
        Some(&linear(
            Anchor::Vertex(ca),
            Anchor::Vertex(cb),
            p(0.0, -1.0, 0.0)
        ))
    );
    m.validate().unwrap();

    // Moving an original vertex moves only the original dimension's end.
    m.move_vertices(&BTreeSet::from([a]), p(-1.0, 0.0, 0.0))
        .unwrap();
    assert_eq!(
        m.dimension_points(d),
        Some((p(-1.0, 0.0, 0.0), p(4.0, 0.0, 0.0)))
    );
    assert_eq!(
        m.dimension_points(copy),
        Some((p(0.0, 5.0, 0.0), p(4.0, 5.0, 0.0)))
    );

    // Copied without its geometry, the copy's ends are points.
    let alone = created_dimension(
        &m.copy_annotations(&[d], &[], &places, &BTreeSet::new())
            .summary()
            .created,
    );
    assert_eq!(
        m.dimension(alone),
        Some(&linear(
            Anchor::Point(p(-1.0, 5.0, 0.0)),
            Anchor::Point(p(4.0, 5.0, 0.0)),
            p(0.0, -1.0, 0.0)
        ))
    );
    // Only one end copied: that end anchors to its copy.
    let half = BTreeSet::from([b]);
    let places = [DAffine3::from_translation(p(0.0, 10.0, 0.0))];
    m.copy_placed(&edges, &faces, &places).unwrap();
    let one = created_dimension(
        &m.copy_annotations(&[d], &[], &places, &half)
            .summary()
            .created,
    );
    assert_eq!(
        m.dimension(one),
        Some(&linear(
            Anchor::Point(p(-1.0, 10.0, 0.0)),
            Anchor::Vertex(vid(&m, p(4.0, 10.0, 0.0))),
            p(0.0, -1.0, 0.0)
        ))
    );
    m.validate().unwrap();
}

#[test]
fn a_copied_circle_takes_its_radial_dimension_along() {
    let mut m = Model::new();
    m.add_curve(&circle_points(DVec3::ZERO, 1.0, 12), true, false)
        .unwrap();
    let curve = m.edges().next().unwrap().1.curve.unwrap();
    let r = created_dimension(
        &m.add_dimension(Dimension::Radial {
            curve: RadialAnchor::Curve(curve),
            diameter: true,
            leader: p(2.0, 0.0, 0.0),
            tag: None,
        })
        .unwrap()
        .summary()
        .created,
    );
    let edges = all_edges(&m);
    let copied = m.vertices_of(&edges, &[]);
    let places = [DAffine3::from_translation(p(5.0, 0.0, 0.0))];
    m.copy_placed(&edges, &[], &places).unwrap();
    let copy = created_dimension(
        &m.copy_annotations(&[r], &[], &places, &copied)
            .summary()
            .created,
    );
    match m.dimension(copy) {
        Some(Dimension::Radial {
            curve: RadialAnchor::Curve(c),
            diameter: true,
            ..
        }) => assert_ne!(*c, curve),
        other => panic!("not on the copied curve: {other:?}"),
    }
    assert!(
        m.radial_circle(copy)
            .unwrap()
            .center
            .distance(p(5.0, 0.0, 0.0))
            < 1e-9
    );
    // Without its curve, the copy measures a fixed circle.
    let fixed = created_dimension(
        &m.copy_annotations(&[r], &[], &places, &BTreeSet::new())
            .summary()
            .created,
    );
    assert!(matches!(
        m.dimension(fixed),
        Some(Dimension::Radial {
            curve: RadialAnchor::Fixed { .. },
            ..
        })
    ));
    m.validate().unwrap();
}

#[test]
fn grouping_takes_annotations_into_the_definition_still_anchored() {
    // Review focus 3 (kernel part).
    let (m, d) = dimensioned_rectangle();
    let mut s = Scene::from_root(m);
    let far = s
        .add_note(
            MeshId::Root,
            leader(Anchor::Point(p(9.0, 9.0, 0.0)), p(9.0, 9.0, 1.0), None),
        )
        .unwrap();
    let far = created_note(&far.summary().created);
    let top = vid(s.root(), p(4.0, 2.0, 0.0));
    let n = created_note(
        &s.add_note(
            MeshId::Root,
            leader(Anchor::Vertex(top), p(5.0, 3.0, 0.0), Some("Top")),
        )
        .unwrap()
        .summary()
        .created,
    );
    let title = created_note(
        &s.add_note(MeshId::Root, screen("Title"))
            .unwrap()
            .summary()
            .created,
    );
    let before = s.clone();
    let root = s.root();
    // Group the bottom and right edges only, with the dimension and both notes.
    let bottom = root
        .edge_between(vid(root, DVec3::ZERO), vid(root, p(4.0, 0.0, 0.0)))
        .unwrap();
    let parts = Parts {
        edges: vec![bottom],
        dimensions: vec![d],
        notes: vec![n, title],
        ..Parts::default()
    };
    let (i, change) = s
        .make_group_with(MeshId::Root, &parts, DefinitionKind::Group, "")
        .unwrap();
    s.validate().unwrap();
    let def = s.root().instance(i).unwrap().definition;
    let inner = &s.definition(def).unwrap().mesh;
    assert!(s.root().dimension(d).is_none());
    assert_eq!(
        inner.dimension(d),
        Some(&linear(
            Anchor::Vertex(vid(inner, DVec3::ZERO)),
            Anchor::Vertex(vid(inner, p(4.0, 0.0, 0.0))),
            p(0.0, -1.0, 0.0)
        )),
        "same id, anchored to the group's vertices"
    );
    assert_eq!(
        inner.note(n),
        Some(&leader(
            Anchor::Point(p(4.0, 2.0, 0.0)),
            p(5.0, 3.0, 0.0),
            Some("Top")
        )),
        "a vertex left outside becomes a point"
    );
    assert!(
        s.root().note(title).is_some(),
        "screen notes stay in the root"
    );
    assert!(s.root().note(far).is_some());
    // Moved between meshes under one id, the dimension counts as changed.
    assert!(change.summary().modified.contains(&EntityId::Dimension(d)));
    s.apply(&change.inverse());
    assert_eq!(s, before);
    s.apply(&change);

    // Moving the group moves the dimension with it; exploding brings it back out with a
    // new id, anchored to the parent's vertices where the group put them.
    let t =
        DAffine3::from_rotation_translation(DQuat::from_rotation_z(TAU / 4.0), p(10.0, 0.0, 0.0));
    s.edit(MeshId::Root, |m| Ok(m.transform_instances(&[i], t)))
        .unwrap();
    let exploded = s.explode(MeshId::Root, i).unwrap();
    s.validate().unwrap();
    let out = created_dimension(&exploded.summary().created);
    assert_ne!(out, d);
    let root = s.root();
    let (pa, pb) = (
        t.transform_point3(DVec3::ZERO),
        t.transform_point3(p(4.0, 0.0, 0.0)),
    );
    match root.dimension(out) {
        Some(Dimension::Linear {
            a: Anchor::Vertex(a),
            b: Anchor::Vertex(b),
            offset,
            ..
        }) => {
            assert!(root.position(*a).unwrap().distance(pa) < 1e-9);
            assert!(root.position(*b).unwrap().distance(pb) < 1e-9);
            assert!(
                offset.distance(p(1.0, 0.0, 0.0)) < 1e-9,
                "the offset turned"
            );
        }
        other => panic!("not anchored: {other:?}"),
    }
    assert!(root.notes().any(|(_, n)| n.text() == Some("Top")));
}

#[test]
fn copied_groups_copy_their_annotations() {
    let (m, d) = dimensioned_rectangle();
    let mut s = Scene::from_root(m);
    let (edges, faces) = (all_edges(s.root()), all_faces(s.root()));
    let parts = Parts {
        edges,
        faces,
        dimensions: vec![d],
        ..Parts::default()
    };
    let (i, _) = s
        .make_group_with(MeshId::Root, &parts, DefinitionKind::Group, "")
        .unwrap();
    s.copy_instances(MeshId::Root, &[i], p(0.0, 10.0, 0.0), 1)
        .unwrap();
    s.validate().unwrap();
    let copies: Vec<_> = s.root().instances().map(|(_, i)| i.definition).collect();
    assert_eq!(copies.len(), 2);
    for def in copies {
        let mesh = &s.definition(def).unwrap().mesh;
        let (id, dim) = mesh.dimensions().next().expect("a dimension in each");
        assert_eq!(
            mesh.dimension_points(id),
            Some((DVec3::ZERO, p(4.0, 0.0, 0.0)))
        );
        assert!(matches!(
            dim,
            Dimension::Linear {
                a: Anchor::Vertex(_),
                b: Anchor::Vertex(_),
                ..
            }
        ));
    }
}

#[test]
fn clips_carry_annotations_and_their_tags() {
    let (m, d) = dimensioned_rectangle();
    let mut s = Scene::from_root(m);
    let (tag, _) = s
        .add_tag(Tag {
            name: "Dims".into(),
            color: [1, 2, 3],
            visible: true,
        })
        .unwrap();
    s.set_tags(MeshId::Root, &[EntityId::Dimension(d)], Some(tag))
        .unwrap();
    let title = created_note(
        &s.add_note(MeshId::Root, screen("Title"))
            .unwrap()
            .summary()
            .created,
    );
    let (edges, faces) = (all_edges(s.root()), all_faces(s.root()));
    let parts = Parts {
        edges,
        faces,
        dimensions: vec![d],
        notes: vec![title],
        ..Parts::default()
    };
    let shift = DAffine3::from_translation(p(1.0, 0.0, 0.0));
    let clip = s.extract_with(MeshId::Root, &parts, shift).unwrap();
    clip.validate().unwrap();
    let root = clip.root();
    let (cd, dim) = root.dimensions().next().unwrap();
    assert_eq!(
        root.dimension_points(cd),
        Some((p(1.0, 0.0, 0.0), p(5.0, 0.0, 0.0)))
    );
    assert!(matches!(
        dim,
        Dimension::Linear {
            a: Anchor::Vertex(_),
            b: Anchor::Vertex(_),
            ..
        }
    ));
    assert_eq!(clip.tags().count(), 1, "the dimension's tag comes along");
    assert_eq!(root.notes().count(), 1);

    // Pasting into another scene: the dimension anchors to the pasted vertices and its tag is
    // matched by name.
    let mut target = Scene::new();
    let place = DAffine3::from_translation(p(0.0, 20.0, 0.0));
    let (inserted, change) = target
        .insert(MeshId::Root, &clip, place, &|_| None)
        .unwrap();
    target.validate().unwrap();
    assert_eq!(inserted.dimensions.len(), 1);
    assert_eq!(inserted.notes.len(), 1);
    let pasted = inserted.dimensions[0];
    let m = target.root();
    assert_eq!(
        m.dimension_points(pasted),
        Some((p(1.0, 20.0, 0.0), p(5.0, 20.0, 0.0)))
    );
    assert!(matches!(
        m.dimension(pasted),
        Some(Dimension::Linear {
            a: Anchor::Vertex(_),
            b: Anchor::Vertex(_),
            ..
        })
    ));
    let pasted_tag = m.dimension(pasted).unwrap().tag().unwrap();
    assert_eq!(target.tag(pasted_tag).unwrap().name, "Dims");
    assert!(
        change
            .summary()
            .created
            .contains(&EntityId::Dimension(pasted))
    );

    // Into a group, screen notes are left out.
    let mut nested = Scene::from_root(Model::new());
    nested
        .edit(MeshId::Root, |m| {
            m.add_edge(p(50.0, 0.0, 0.0), p(51.0, 0.0, 0.0))
        })
        .unwrap();
    let edges = all_edges(nested.root());
    let (i, _) = nested
        .make_group(MeshId::Root, &edges, &[], &[], DefinitionKind::Group, "")
        .unwrap();
    let def = MeshId::Definition(nested.root().instance(i).unwrap().definition);
    let (inside, _) = nested
        .insert(def, &clip, DAffine3::IDENTITY, &|_| None)
        .unwrap();
    nested.validate().unwrap();
    assert_eq!((inside.dimensions.len(), inside.notes.len()), (1, 0));
}

#[test]
fn annotations_go_on_tags_and_are_counted_and_moved_off_deleted_tags() {
    let (m, d) = dimensioned_rectangle();
    let mut s = Scene::from_root(m);
    let n = created_note(
        &s.add_note(MeshId::Root, screen("Title"))
            .unwrap()
            .summary()
            .created,
    );
    let (tag, _) = s
        .add_tag(Tag {
            name: "Notes".into(),
            color: [9, 9, 9],
            visible: true,
        })
        .unwrap();
    let before = s.clone();
    let c = s
        .set_tags(
            MeshId::Root,
            &[EntityId::Dimension(d), EntityId::Note(n)],
            Some(tag),
        )
        .unwrap();
    assert_eq!(
        c.summary().modified,
        vec![EntityId::Dimension(d), EntityId::Note(n)]
    );
    assert_eq!(s.root().tag_of(EntityId::Note(n)), Some(Some(tag)));
    assert_eq!(
        s.root().tagged(tag).collect::<Vec<_>>(),
        vec![EntityId::Dimension(d), EntityId::Note(n)]
    );
    assert_eq!(s.tag_count(tag), 2);
    s.validate().unwrap();
    let deleted = s.delete_tag(tag, None).unwrap();
    assert_eq!(s.root().dimension(d).unwrap().tag(), None);
    assert_eq!(s.root().note(n).unwrap().tag(), None);
    s.apply(&deleted.inverse());
    s.apply(&c.inverse());
    assert_eq!(s, before);
}

#[test]
fn screen_notes_go_only_into_the_root() {
    let mut s = Scene::new();
    s.edit(MeshId::Root, |m| m.add_edge(DVec3::ZERO, DVec3::X))
        .unwrap();
    let edges = all_edges(s.root());
    let (i, _) = s
        .make_group(MeshId::Root, &edges, &[], &[], DefinitionKind::Group, "")
        .unwrap();
    let def = MeshId::Definition(s.root().instance(i).unwrap().definition);
    assert!(matches!(
        s.add_note(def, screen("Inside")),
        Err(KernelError::Annotation(_))
    ));
    s.add_note(
        def,
        leader(Anchor::Point(DVec3::ZERO), DVec3::Y, Some("Inside")),
    )
    .unwrap();
    s.validate().unwrap();

    // A file with a screen note inside a definition is refused.
    let mut data = s.to_data();
    data.definitions[0]
        .mesh
        .notes
        .push(kernel::NoteData::Screen {
            id: 9000,
            at: [0.5, 0.5],
            text: "Inside".into(),
            tag: None,
        });
    let e = Scene::from_data(&data).unwrap_err();
    assert!(e.contains("n9000") && e.contains("screen note"), "{e}");
}

/// A scene with every kind of annotation: linear on vertices and on points, radial on a curve
/// and on a fixed circle, leaders with and without text, a screen note, one tagged.
fn annotated_scene() -> Scene {
    let (mut m, _) = dimensioned_rectangle();
    m.add_curve(&circle_points(p(10.0, 0.0, 0.0), 1.0, 16), true, false)
        .unwrap();
    let curve = m.edges().find_map(|(_, e)| e.curve).unwrap();
    m.add_dimension(linear(
        Anchor::Vertex(vid(&m, p(4.0, 2.0, 0.0))),
        Anchor::Point(p(4.0, 8.0, 0.0)),
        p(1.0, 0.0, 0.0),
    ))
    .unwrap();
    m.add_dimension(Dimension::Radial {
        curve: RadialAnchor::Curve(curve),
        diameter: true,
        leader: p(2.0, 2.0, 0.0),
        tag: None,
    })
    .unwrap();
    m.add_dimension(Dimension::Radial {
        curve: RadialAnchor::Fixed {
            center: p(20.0, 0.0, 0.0),
            normal: DVec3::Y,
            radius: 0.5,
        },
        diameter: false,
        leader: p(1.0, 0.0, 0.0),
        tag: None,
    })
    .unwrap();
    let corner = vid(&m, p(0.0, 2.0, 0.0));
    let mut s = Scene::from_root(m);
    s.add_note(
        MeshId::Root,
        leader(Anchor::Vertex(corner), p(-1.0, 3.0, 0.0), None),
    )
    .unwrap();
    s.add_note(
        MeshId::Root,
        leader(
            Anchor::Point(p(2.0, 1.0, 0.0)),
            p(2.0, 1.0, 2.0),
            Some("Floor\nOak"),
        ),
    )
    .unwrap();
    s.add_note(MeshId::Root, screen("Plan")).unwrap();
    let (tag, _) = s
        .add_tag(Tag {
            name: "Annotations".into(),
            color: [0, 0, 0],
            visible: false,
        })
        .unwrap();
    let first = s.root().dimensions().next().unwrap().0;
    s.set_tags(MeshId::Root, &[EntityId::Dimension(first)], Some(tag))
        .unwrap();
    s.validate().unwrap();
    s
}

#[test]
fn annotations_round_trip_through_data_and_json() {
    let s = annotated_scene();
    let data = s.to_data();
    assert_eq!(data.root.dimensions.len(), 4);
    assert_eq!(data.root.notes.len(), 3);
    let json = serde_json::to_string(&data).unwrap();
    let back: SceneData = serde_json::from_str(&json).unwrap();
    assert_eq!(back, data);
    let loaded = Scene::from_data(&back).unwrap();
    assert_eq!(loaded, s);

    // A frozen anchor round-trips too.
    let mut frozen = s.clone();
    let corner = vid(frozen.root(), p(4.0, 2.0, 0.0));
    let edges: Vec<_> = frozen.root().edges_at(corner);
    frozen
        .edit(MeshId::Root, |m| Ok(m.erase(&edges, &[])))
        .unwrap();
    frozen.validate().unwrap();
    assert!(frozen.root().dimensions().any(|(_, d)| matches!(
        d,
        Dimension::Linear {
            a: Anchor::Point(_),
            b: Anchor::Point(_),
            ..
        }
    )));
    let again = Scene::from_data(&frozen.to_data()).unwrap();
    assert_eq!(again, frozen);
}

#[test]
fn meshes_without_annotations_write_no_annotation_fields() {
    let mut m = Model::new();
    rectangle(&mut m, DVec3::ZERO);
    let json = serde_json::to_string(&Scene::from_root(m).to_data()).unwrap();
    assert!(
        !json.contains("dimensions") && !json.contains("notes"),
        "{json}"
    );
    // And files written before annotations existed load.
    let back: SceneData = serde_json::from_str(&json).unwrap();
    Scene::from_data(&back).unwrap();
}

#[test]
fn annotation_data_that_breaks_the_model_is_refused_clearly() {
    let s = annotated_scene();
    let data = s.to_data();
    let refuse = |change: &dyn Fn(&mut SceneData), want: &str| {
        let mut bad = data.clone();
        change(&mut bad);
        let e = Scene::from_data(&bad).unwrap_err();
        assert!(e.contains(want), "{want:?} not in {e:?}");
    };
    refuse(
        &|d| {
            if let kernel::DimensionData::Linear { a, .. } = &mut d.root.dimensions[0] {
                *a = kernel::AnchorData::Vertex(999);
            }
        },
        "vertex v999",
    );
    refuse(
        &|d| {
            if let kernel::NoteData::Leader { anchor, .. } = &mut d.root.notes[0] {
                *anchor = kernel::AnchorData::Vertex(998);
            }
        },
        "vertex v998",
    );
    refuse(
        &|d| {
            let n = d.root.notes.len();
            if let kernel::NoteData::Screen { text, .. } = &mut d.root.notes[n - 1] {
                *text = " ".into();
            }
        },
        "blank",
    );
    refuse(
        &|d| {
            let first = d.root.dimensions[0].clone();
            d.root.dimensions.push(first);
        },
        "appears twice",
    );
    refuse(
        &|d| {
            for dim in &mut d.root.dimensions {
                if let kernel::DimensionData::Radial {
                    curve: kernel::RadialAnchorData::Curve(c),
                    ..
                } = dim
                {
                    *c = 4321;
                }
            }
        },
        "not a circle or an arc",
    );
    refuse(
        &|d| {
            let n = d.root.notes.len();
            if let kernel::NoteData::Screen { id, .. } = &mut d.root.notes[n - 1] {
                *id = d.root.vertices[0].id;
            }
        },
        "share an id",
    );
    refuse(&|d| d.tags.clear(), "missing tag");
    // Unknown fields are refused.
    let json = serde_json::to_string(&data).unwrap();
    let odd = json.replacen("\"offset\"", "\"bogus\":1,\"offset\"", 1);
    assert!(serde_json::from_str::<SceneData>(&odd).is_err());
}

#[test]
fn edits_that_leave_a_circle_alone_leave_its_radial_dimension_attached() {
    let mut m = Model::new();
    m.add_curve(&circle_points(DVec3::ZERO, 1.0, 16), true, false)
        .unwrap();
    let curve = m.edges().next().unwrap().1.curve.unwrap();
    let r = created_dimension(
        &m.add_dimension(Dimension::Radial {
            curve: RadialAnchor::Curve(curve),
            diameter: false,
            leader: p(2.0, 0.0, 0.0),
            tag: None,
        })
        .unwrap()
        .summary()
        .created,
    );
    let on_curve = |m: &Model| {
        matches!(
            m.dimension(r),
            Some(Dimension::Radial {
                curve: RadialAnchor::Curve(c),
                ..
            }) if *c == curve
        )
    };
    // Unrelated edits: a separate line drawn, then moved, then erased.
    let drawn = m.add_edge(p(5.0, 0.0, 0.0), p(6.0, 0.0, 0.0)).unwrap();
    assert!(on_curve(&m));
    assert!(!drawn.summary().modified.contains(&EntityId::Dimension(r)));
    let far = vid(&m, p(6.0, 0.0, 0.0));
    let moved = m
        .move_vertices(&BTreeSet::from([far]), p(0.0, 1.0, 0.0))
        .unwrap();
    assert!(on_curve(&m));
    assert!(!moved.summary().modified.contains(&EntityId::Dimension(r)));
    let line: Vec<_> = m.edges_at(far);
    m.erase(&line, &[]);
    assert!(on_curve(&m));
    m.validate().unwrap();
    // An edit that touches the circle and breaks it still freezes the dimension.
    let first = vid(&m, p(1.0, 0.0, 0.0));
    m.move_vertices(&BTreeSet::from([first]), p(0.3, 0.0, 0.0))
        .unwrap();
    assert!(matches!(
        m.dimension(r),
        Some(Dimension::Radial {
            curve: RadialAnchor::Fixed { .. },
            ..
        })
    ));
    m.validate().unwrap();
}

#[test]
fn a_non_uniform_scale_keeps_a_fixed_circles_area_and_scales_point_ends() {
    let mut m = Model::new();
    let d = created_dimension(
        &m.add_dimension(linear(
            Anchor::Point(DVec3::ZERO),
            Anchor::Point(p(0.0, 2.0, 0.0)),
            p(1.0, 0.0, 0.0),
        ))
        .unwrap()
        .summary()
        .created,
    );
    let r = created_dimension(
        &m.add_dimension(Dimension::Radial {
            curve: RadialAnchor::Fixed {
                center: p(1.0, 1.0, 0.0),
                normal: DVec3::Z,
                radius: 1.0,
            },
            diameter: false,
            leader: p(1.0, 1.0, 0.0),
            tag: None,
        })
        .unwrap()
        .summary()
        .created,
    );
    let stretch = DAffine3::from_scale(p(2.0, 3.0, 5.0));
    m.transform_annotations(&[d, r], &[], stretch);
    m.validate().unwrap();
    assert_eq!(
        m.dimension(d),
        Some(&linear(
            Anchor::Point(DVec3::ZERO),
            Anchor::Point(p(0.0, 6.0, 0.0)),
            p(2.0, 0.0, 0.0)
        ))
    );
    let c = m.radial_circle(r).unwrap();
    assert!(c.center.distance(p(2.0, 3.0, 0.0)) < 1e-12);
    assert!(c.normal.distance(DVec3::Z) < 1e-12);
    assert!(
        (c.radius - 6.0_f64.sqrt()).abs() < 1e-12,
        "the radius keeps the ellipse's area: sqrt(2 * 3)"
    );
    match m.dimension(r) {
        Some(Dimension::Radial { leader, .. }) => assert_eq!(*leader, p(2.0, 3.0, 0.0)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn exploding_a_stretched_group_places_its_annotations_by_the_stretch() {
    let (m, d) = dimensioned_rectangle();
    let mut s = Scene::from_root(m);
    let (edges, faces) = (all_edges(s.root()), all_faces(s.root()));
    let parts = Parts {
        edges,
        faces,
        dimensions: vec![d],
        ..Parts::default()
    };
    let (i, _) = s
        .make_group_with(MeshId::Root, &parts, DefinitionKind::Group, "")
        .unwrap();
    let stretch = DAffine3::from_scale(p(2.0, 3.0, 1.0));
    s.edit(MeshId::Root, |m| Ok(m.transform_instances(&[i], stretch)))
        .unwrap();
    let exploded = s.explode(MeshId::Root, i).unwrap();
    s.validate().unwrap();
    let out = created_dimension(&exploded.summary().created);
    let root = s.root();
    assert_eq!(
        root.dimension(out),
        Some(&linear(
            Anchor::Vertex(vid(root, DVec3::ZERO)),
            Anchor::Vertex(vid(root, p(8.0, 0.0, 0.0))),
            p(0.0, -3.0, 0.0)
        ))
    );
}

#[test]
fn moving_a_point_end_onto_the_other_end_removes_the_dimension() {
    let (mut m, _) = dimensioned_rectangle();
    let a = vid(&m, DVec3::ZERO);
    let d = created_dimension(
        &m.add_dimension(linear(
            Anchor::Vertex(a),
            Anchor::Point(p(0.0, -3.0, 0.0)),
            DVec3::X,
        ))
        .unwrap()
        .summary()
        .created,
    );
    let c = m.transform_annotations(&[d], &[], DAffine3::from_translation(p(0.0, 3.0, 0.0)));
    assert!(m.dimension(d).is_none());
    assert_eq!(c.summary().deleted, vec![EntityId::Dimension(d)]);
    m.validate().unwrap();
    m.apply(&c.inverse());
    assert!(m.dimension(d).is_some());

    // Moving the vertex end onto the point end removes it too, with the move.
    let moved = m
        .move_vertices(&BTreeSet::from([a]), p(0.0, -3.0, 0.0))
        .unwrap();
    assert!(moved.summary().deleted.contains(&EntityId::Dimension(d)));
    assert!(m.dimension(d).is_none());
    m.validate().unwrap();
}

#[test]
fn anchors_are_written_tagged_and_coincident_ends_are_refused_on_load() {
    let s = annotated_scene();
    let json = serde_json::to_string(&s.to_data()).unwrap();
    assert!(json.contains("{\"vertex\":"), "{json}");
    assert!(json.contains("{\"point\":["), "{json}");
    assert!(json.contains("{\"curve\":"), "{json}");
    assert!(json.contains("{\"fixed\":{"), "{json}");
    let bad = json.replacen("{\"vertex\":", "{\"vertexx\":", 1);
    let e = serde_json::from_str::<SceneData>(&bad)
        .unwrap_err()
        .to_string();
    assert!(e.contains("vertexx"), "{e}");

    let mut data = s.to_data();
    data.root.dimensions.push(kernel::DimensionData::Linear {
        id: 9100,
        a: kernel::AnchorData::Point([1.0, 1.0, 1.0]),
        b: kernel::AnchorData::Point([1.0, 1.0, 1.0]),
        offset: [0.0, 0.0, 1.0],
        tag: None,
    });
    let e = Scene::from_data(&data).unwrap_err();
    assert!(e.contains("r9100") && e.contains("coincide"), "{e}");
}
