//! Hidden tags and coloring faces by tag in what the renderer draws.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{
    DefinitionKind, EntityId, Guide, InstanceId, MeshId, Model, Scene, Side, Tag, TagEdit, TagId,
};
use render::Snapshotter;
use render::scene::{SceneBase, scene_base, scene_base_with};
use session::{Camera, Lighting};

/// A unit square at x from `x`, at height 0.
fn square_at(m: &mut Model, x: f64) {
    let p = |dx: f64, y: f64| DVec3::new(x + dx, y, 0.0);
    m.add_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)])
        .unwrap();
}

fn add_tag(s: &mut Scene, name: &str, color: [u8; 3]) -> TagId {
    s.add_tag(Tag {
        name: name.into(),
        color,
        visible: true,
    })
    .unwrap()
    .0
}

fn hide(s: &mut Scene, tag: TagId) {
    s.change_tag(
        tag,
        TagEdit {
            visible: Some(false),
            ..TagEdit::default()
        },
    )
    .unwrap();
}

/// The ids of a mesh's entities of the square whose corner is at `x`.
fn square_ids(m: &Model, x: f64) -> Vec<EntityId> {
    let on = |p: DVec3| (p.x - x) > -1e-9 && (p.x - x) < 1.0 + 1e-9;
    let faces = m
        .faces()
        .filter(|(_, f)| f.outer.iter().all(|v| m.position(*v).is_some_and(on)));
    let edges = m
        .edges()
        .filter(|(_, e)| e.v.iter().all(|v| m.position(*v).is_some_and(on)));
    faces
        .map(|(id, _)| EntityId::Face(id))
        .chain(edges.map(|(id, _)| EntityId::Edge(id)))
        .collect()
}

/// Two loose unit squares at x 0 and 3, and a guide point.
fn two_squares() -> Scene {
    let mut m = Model::new();
    square_at(&mut m, 0.0);
    square_at(&mut m, 3.0);
    m.add_guide(Guide::point(DVec3::new(5.0, 5.0, 0.0)))
        .unwrap();
    Scene::from_root(m)
}

/// The root's square at x 0 made a group, with the loose square at x 3 left beside it.
fn grouped() -> (Scene, InstanceId) {
    let mut s = two_squares();
    let ids = square_ids(s.root(), 0.0);
    let edges: Vec<_> = ids
        .iter()
        .filter_map(|id| match id {
            EntityId::Edge(e) => Some(*e),
            _ => None,
        })
        .collect();
    let faces: Vec<_> = ids
        .iter()
        .filter_map(|id| match id {
            EntityId::Face(f) => Some(*f),
            _ => None,
        })
        .collect();
    let (group, _) = s
        .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
        .unwrap();
    (s, group)
}

fn counts(b: &SceneBase) -> (usize, usize, usize) {
    (b.faces.len(), b.edges.len(), b.guides.len())
}

#[test]
fn a_hidden_tag_hides_its_faces_edges_and_guides() {
    let mut s = two_squares();
    assert_eq!(counts(&scene_base(&s, &[])), (12, 16, 1));
    let walls = add_tag(&mut s, "Walls", [200, 0, 0]);
    let face_only: Vec<EntityId> = square_ids(s.root(), 0.0)
        .into_iter()
        .filter(|id| matches!(id, EntityId::Face(_)))
        .collect();
    s.set_tags(MeshId::Root, &face_only, Some(walls)).unwrap();
    assert_eq!(
        counts(&scene_base(&s, &[])),
        (12, 16, 1),
        "a shown tag hides nothing"
    );
    hide(&mut s, walls);
    assert_eq!(
        counts(&scene_base(&s, &[])),
        (6, 16, 1),
        "the face goes, its untagged edges stay"
    );
    let guide = s.root().guides().next().unwrap().0;
    let mut tagged = square_ids(s.root(), 0.0);
    tagged.push(EntityId::Guide(guide));
    s.set_tags(MeshId::Root, &tagged, Some(walls)).unwrap();
    let base = scene_base(&s, &[]);
    assert_eq!(counts(&base), (6, 8, 0));
    assert!(base.faces.iter().all(|v| v.pos[0] >= 3.0));
    // The hidden face casts no shadow either; selecting hidden ids lights nothing.
    assert_eq!(base.casters.len(), 6);
    let flags = base.spans.flags(&tagged.iter().copied().collect());
    assert!(flags.faces.iter().chain(&flags.edges).all(|f| *f == 0.0));
}

#[test]
fn a_hidden_group_hides_everything_in_it_even_on_shown_tags() {
    let (mut s, group) = grouped();
    let (shown, hidden) = (
        add_tag(&mut s, "Shown", [0, 200, 0]),
        add_tag(&mut s, "Hidden", [0, 0, 200]),
    );
    let d = s.root().instance(group).unwrap().definition;
    let inside: Vec<EntityId> = square_ids(&s.definition(d).unwrap().mesh, 0.0);
    s.set_tags(MeshId::Definition(d), &inside, Some(shown))
        .unwrap();
    s.set_tags(MeshId::Root, &[EntityId::Instance(group)], Some(hidden))
        .unwrap();
    assert_eq!(counts(&scene_base(&s, &[])), (12, 16, 1));
    hide(&mut s, hidden);
    let base = scene_base(&s, &[]);
    assert_eq!(counts(&base), (6, 8, 1), "only the loose square and guide");
    assert!(base.faces.iter().all(|v| v.pos[0] >= 3.0));
    // Opened (by an agent, say), the group being edited stays drawn.
    assert_eq!(counts(&scene_base(&s, &[group])).0, 12);
    // A hidden tag inside a shown group hides just what is on it.
    let mut s2 = grouped().0;
    let inner = add_tag(&mut s2, "Inner", [9, 9, 9]);
    let g2 = s2.root().instances().next().unwrap().1.definition;
    let inside: Vec<EntityId> = square_ids(&s2.definition(g2).unwrap().mesh, 0.0)
        .into_iter()
        .filter(|id| matches!(id, EntityId::Face(_)))
        .collect();
    s2.set_tags(MeshId::Definition(g2), &inside, Some(inner))
        .unwrap();
    hide(&mut s2, inner);
    assert_eq!(counts(&scene_base(&s2, &[])), (6, 16, 1));
}

#[test]
fn colored_by_tag_faces_take_their_tag_or_the_nearest_tagged_instances() {
    let (mut s, group) = grouped();
    let (red, _) = s.add_material("Red", [255, 0, 0]).unwrap();
    let loose = square_ids(s.root(), 3.0);
    let loose_faces: Vec<_> = loose
        .iter()
        .filter_map(|id| match id {
            EntityId::Face(f) => Some(*f),
            _ => None,
        })
        .collect();
    s.paint(MeshId::Root, &loose_faces, &[], Side::Front, Some(red))
        .unwrap();
    let blue = add_tag(&mut s, "Blue", [0, 0, 255]);
    s.set_tags(MeshId::Root, &[EntityId::Instance(group)], Some(blue))
        .unwrap();
    let rgb = |c: [u8; 3]| c.map(|x| f32::from(x) / 255.0);
    let by_material = scene_base(&s, &[]);
    let by_tag = scene_base_with(&s, &[], true);
    assert_eq!(by_tag.edges, by_material.edges, "edges keep their colors");
    for v in &by_tag.faces {
        if v.pos[0] <= 1.0 {
            // The group's untagged face takes the group's tag, front and back.
            assert_eq!((v.front, v.back), (rgb([0, 0, 255]), rgb([0, 0, 255])));
        } else {
            // The loose untagged face keeps its material.
            assert_eq!(v.front, rgb([255, 0, 0]));
        }
    }
    // A face's own tag wins over the instance's.
    let green = add_tag(&mut s, "Green", [0, 255, 0]);
    let d = s.root().instance(group).unwrap().definition;
    let inside = square_ids(&s.definition(d).unwrap().mesh, 0.0);
    s.set_tags(MeshId::Definition(d), &inside, Some(green))
        .unwrap();
    let by_tag = scene_base_with(&s, &[], true);
    assert!(
        by_tag
            .faces
            .iter()
            .filter(|v| v.pos[0] <= 1.0)
            .all(|v| v.front == rgb([0, 255, 0]) && v.alpha == [1.0, 1.0])
    );
    // Off again, the materials show.
    assert_eq!(scene_base_with(&s, &[], false), scene_base(&s, &[]));
}

#[test]
fn textures_are_not_shown_while_coloring_by_tag() {
    let mut s = two_squares();
    let texture = kernel::Texture {
        image: std::sync::Arc::new(kernel::TextureImage {
            format: kernel::ImageFormat::Png,
            encoded: Vec::new(),
            width: 1,
            height: 1,
            pixels: vec![255; 4],
        }),
        width: 1.0,
        height: 1.0,
    };
    let (tiles, _) = s
        .add_material_with_texture("Tiles", [1, 2, 3], 1.0, Some(texture))
        .unwrap();
    let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
    s.paint(MeshId::Root, &faces, &[], Side::Front, Some(tiles))
        .unwrap();
    assert!(scene_base(&s, &[]).faces.iter().all(|v| v.layer[0] == 0.0));
    let by_tag = scene_base_with(&s, &[], true);
    assert!(
        by_tag
            .faces
            .iter()
            .all(|v| v.layer == [-1.0, -1.0] && v.uv == [0.0; 4])
    );
    assert!(
        by_tag
            .faces
            .iter()
            .all(|v| v.front == [1.0, 2.0, 3.0].map(|c| c / 255.0)),
        "untagged faces keep the material's color"
    );
}

/// A camera looking down at the unit square at the origin, its viewport the image's.
fn camera(w: u32, h: u32) -> Camera {
    let mut c = Camera::new(DVec2::new(w as f64, h as f64));
    c.eye = DVec3::new(0.5, -1.5, 3.0);
    c.target = DVec3::new(0.5, 0.5, 0.0);
    c
}

#[test]
fn hidden_faces_are_not_drawn_and_tag_colors_show() {
    let Some(mut snap) = Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let (w, h) = (64, 64);
    let cam = camera(w, h);
    let at = cam.project(DVec3::new(0.4, 0.6, 0.0)).unwrap();
    let mut capture = |scene: &Scene, by_tag: bool| {
        snap.set_color_by_tag(by_tag);
        let px = snap
            .capture(scene, &[], &BTreeSet::new(), &cam, Lighting::Day, w, h)
            .expect("capture");
        let i = ((at.y as u32 * w + at.x as u32) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };
    let mut m = Model::new();
    square_at(&mut m, 0.0);
    let mut s = Scene::from_root(m);
    let empty = capture(&Scene::new(), false);
    let plain = capture(&s, false);
    assert_ne!(plain, empty, "the face is drawn");
    let walls = add_tag(&mut s, "Walls", [250, 10, 10]);
    let all: Vec<EntityId> = square_ids(s.root(), 0.0);
    s.set_tags(MeshId::Root, &all, Some(walls)).unwrap();
    let colored = capture(&s, true);
    assert!(
        colored[0] as i32 > colored[1] as i32 + 80 && colored[0] as i32 > colored[2] as i32 + 80,
        "the face shows its tag's red, got {colored:?}"
    );
    assert_eq!(capture(&s, false), plain, "off again, as before");
    hide(&mut s, walls);
    assert_eq!(capture(&s, true), empty, "hidden, nothing is drawn there");
}
