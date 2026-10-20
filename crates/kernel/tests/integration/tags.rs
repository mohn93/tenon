//! Tags: adding, changing and deleting them, putting entities on them, and how tags follow
//! geometry through splits, copies, groups, explodes, push/pull and files.

use glam::DVec3;
use kernel::{
    DefinitionKind, EdgeId, EntityId, FaceId, Guide, InstanceId, KernelError, MeshId, Model, Scene,
    SceneData, TAG_COLORS, Tag, TagEdit, TagId,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

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

fn tag(s: &mut Scene, name: &str) -> TagId {
    s.add_tag(Tag {
        name: name.into(),
        color: [10, 20, 30],
        visible: true,
    })
    .unwrap()
    .0
}

fn edges(m: &Model) -> Vec<EdgeId> {
    m.edges().map(|(e, _)| e).collect()
}

fn all_ids(m: &Model) -> Vec<EntityId> {
    m.edges()
        .map(|(e, _)| EntityId::Edge(e))
        .chain(m.faces().map(|(f, _)| EntityId::Face(f)))
        .collect()
}

/// The square grouped, with a guide point inside the group: the scene and the instance.
fn grouped_square() -> (Scene, InstanceId) {
    let (mut s, face) = square_scene();
    let e = edges(s.root());
    let (group, _) = s
        .make_group(MeshId::Root, &e, &[face], &[], DefinitionKind::Group, "")
        .unwrap();
    let d = MeshId::Definition(s.root().instance(group).unwrap().definition);
    s.edit(d, |m| m.add_guide(Guide::point(p(1.0, 1.0, 1.0))))
        .unwrap();
    (s, group)
}

#[test]
fn tags_are_added_changed_and_deleted_as_exact_undo_steps() {
    let (mut s, _) = square_scene();
    let empty = s.clone();
    let (id, added) = s
        .add_tag(Tag {
            name: "Walls".into(),
            color: [1, 2, 3],
            visible: true,
        })
        .unwrap();
    assert_eq!(id.to_string(), format!("t{}", id.0));
    assert_eq!(s.tag(id).unwrap().name, "Walls");
    assert_eq!(added.summary().tags, vec![id]);
    assert!(added.summary().created.is_empty());
    // Tag ids come from the entity id counter: never one an entity has.
    assert!(s.root().vertices().all(|(v, _)| v.0 != id.0));

    let before = s.clone();
    let changed = s
        .change_tag(
            id,
            TagEdit {
                name: Some("Old walls"),
                color: Some([4, 5, 6]),
                visible: Some(false),
            },
        )
        .unwrap();
    assert_eq!(
        s.tag(id).unwrap(),
        &Tag {
            name: "Old walls".into(),
            color: [4, 5, 6],
            visible: false,
        }
    );
    assert_eq!(changed.summary().tags, vec![id]);
    let same = TagEdit {
        visible: Some(false),
        ..TagEdit::default()
    };
    assert!(s.change_tag(id, same).unwrap().is_empty());
    s.apply(&changed.inverse());
    assert_eq!(s, before);

    let deleted = s.delete_tag(id, None).unwrap();
    assert!(s.tag(id).is_none());
    assert_eq!(deleted.summary().tags, vec![id]);
    s.apply(&deleted.inverse());
    assert_eq!(s, before);
    s.apply(&added.inverse());
    assert_eq!(s, empty);
    assert_eq!(
        s.change_tag(id, TagEdit::default()),
        Err(KernelError::TagNotFound)
    );
}

#[test]
fn tag_names_must_be_present_short_distinct_and_not_untagged() {
    let (mut s, _) = square_scene();
    let walls = tag(&mut s, "Walls");
    let long = "x".repeat(kernel::MAX_TAG_NAME + 1);
    for bad in ["", "  ", "Walls", "Untagged", long.as_str()] {
        let added = s.add_tag(Tag {
            name: bad.into(),
            color: [0, 0, 0],
            visible: true,
        });
        assert!(
            matches!(added, Err(KernelError::TagName(_))),
            "{bad:?} accepted"
        );
    }
    // Case matters, as with materials.
    tag(&mut s, "walls");
    tag(&mut s, &"x".repeat(kernel::MAX_TAG_NAME));
    let renamed = s.change_tag(
        walls,
        TagEdit {
            name: Some("walls"),
            ..TagEdit::default()
        },
    );
    assert!(matches!(renamed, Err(KernelError::TagName(_))));
    // Keeping its own name is no clash.
    let kept = TagEdit {
        name: Some("Walls"),
        ..TagEdit::default()
    };
    assert!(s.change_tag(walls, kept).unwrap().is_empty());
    assert_eq!(s.free_tag_name("Walls"), "Walls 2");
    assert_eq!(s.free_tag_name("Tag"), "Tag");
}

#[test]
fn new_tags_take_palette_colors_in_order() {
    let (mut s, _) = square_scene();
    assert_eq!(s.next_tag_color(), TAG_COLORS[0]);
    let mut ids = Vec::new();
    for (k, _) in TAG_COLORS.iter().enumerate() {
        let color = s.next_tag_color();
        assert_eq!(color, TAG_COLORS[k]);
        let id = s
            .add_tag(Tag {
                name: format!("T{k}"),
                color,
                visible: true,
            })
            .unwrap()
            .0;
        ids.push(id);
    }
    // All twelve used: the next in turn.
    assert_eq!(s.next_tag_color(), TAG_COLORS[0]);
    // A freed color comes back first.
    s.delete_tag(ids[3], None).unwrap();
    assert_eq!(s.next_tag_color(), TAG_COLORS[3]);
}

#[test]
fn set_tags_skips_vertices_and_refuses_unknown_tags() {
    let (mut s, face) = square_scene();
    let walls = tag(&mut s, "Walls");
    let v = s.root().vertices().next().unwrap().0;
    let e = edges(s.root())[0];
    let ids = [EntityId::Face(face), EntityId::Edge(e), EntityId::Vertex(v)];
    let before = s.clone();
    let change = s.set_tags(MeshId::Root, &ids, Some(walls)).unwrap();
    assert_eq!(s.root().face(face).unwrap().tag, Some(walls));
    assert_eq!(s.root().edge(e).unwrap().tag, Some(walls));
    assert_eq!(
        change.summary().modified,
        vec![EntityId::Edge(e), EntityId::Face(face)]
    );
    assert_eq!(s.tag_count(walls), 2);
    // Already there: nothing to do.
    assert!(
        s.set_tags(MeshId::Root, &ids, Some(walls))
            .unwrap()
            .is_empty()
    );
    s.apply(&change.inverse());
    assert_eq!(s, before);
    assert_eq!(
        s.set_tags(MeshId::Root, &ids, Some(TagId(999))),
        Err(KernelError::TagNotFound)
    );
    s.validate().unwrap();
}

#[test]
fn deleting_a_tag_moves_its_entities_everywhere_and_undo_puts_them_back() {
    let (mut s, group) = grouped_square();
    let walls = tag(&mut s, "Walls");
    let roof = tag(&mut s, "Roof");
    let d = MeshId::Definition(s.root().instance(group).unwrap().definition);
    let inside: Vec<EntityId> = {
        let m = s.mesh(d).unwrap();
        let mut ids = all_ids(m);
        ids.extend(m.guides().map(|(g, _)| EntityId::Guide(g)));
        ids
    };
    s.set_tags(d, &inside, Some(walls)).unwrap();
    s.set_tags(MeshId::Root, &[EntityId::Instance(group)], Some(walls))
        .unwrap();
    assert_eq!(s.tag_count(walls), inside.len() + 1);

    for move_to in [None, Some(roof)] {
        let before = s.clone();
        let deleted = s.delete_tag(walls, move_to).unwrap();
        // One edit per mesh with something on the tag (root and the group's), then the
        // removal.
        assert_eq!(deleted.steps.len(), 3);
        s.validate().unwrap();
        assert!(s.tag(walls).is_none());
        assert_eq!(s.root().instance(group).unwrap().tag, move_to);
        let m = s.mesh(d).unwrap();
        assert!(inside.iter().all(|id| m.tag_of(*id) == Some(move_to)));
        assert_eq!(
            s.tag_count(roof),
            if move_to.is_some() {
                inside.len() + 1
            } else {
                0
            }
        );
        // Undo restores the tag and every entity's tag; redo moves them again.
        s.apply(&deleted.inverse());
        assert_eq!(s, before);
        s.apply(&deleted);
        assert_eq!(s.root().instance(group).unwrap().tag, move_to);
        s.apply(&deleted.inverse());
        assert_eq!(s, before);
    }
    assert!(matches!(
        s.delete_tag(walls, Some(walls)),
        Err(KernelError::TagName(_))
    ));
    assert_eq!(
        s.delete_tag(walls, Some(TagId(999))),
        Err(KernelError::TagNotFound)
    );
    assert_eq!(
        s.delete_tag(TagId(999), None),
        Err(KernelError::TagNotFound)
    );
}

#[test]
fn deleting_a_tag_edits_only_the_meshes_with_something_on_it() {
    let (mut s, group) = grouped_square();
    let walls = tag(&mut s, "Walls");
    s.set_tags(MeshId::Root, &[EntityId::Instance(group)], Some(walls))
        .unwrap();
    let deleted = s.delete_tag(walls, None).unwrap();
    // The group's definition has nothing on the tag: no step for it.
    assert!(matches!(
        &deleted.steps[..],
        [
            kernel::Step::Edit(MeshId::Root, _),
            kernel::Step::RemoveTag(..)
        ]
    ));
    let unused = tag(&mut s, "Unused");
    let deleted = s.delete_tag(unused, None).unwrap();
    assert!(matches!(&deleted.steps[..], [kernel::Step::RemoveTag(..)]));
}

#[test]
fn a_tagged_copy_drawn_over_an_existing_edge_puts_it_on_its_tag() {
    let mut m = Model::new();
    for x in [0.0, 4.0] {
        m.add_polygon(&[
            p(x, 0.0, 0.0),
            p(x + 2.0, 0.0, 0.0),
            p(x + 2.0, 2.0, 0.0),
            p(x, 2.0, 0.0),
        ])
        .unwrap();
    }
    let mut s = Scene::from_root(m);
    let walls = tag(&mut s, "Walls");
    let right: Vec<EntityId> = {
        let m = s.root();
        m.edges()
            .filter(|(_, e)| e.v.iter().all(|v| m.position(*v).unwrap().x >= 4.0))
            .map(|(e, _)| EntityId::Edge(e))
            .collect()
    };
    s.set_tags(MeshId::Root, &right, Some(walls)).unwrap();
    let edges: Vec<EdgeId> = right
        .iter()
        .map(|id| match id {
            EntityId::Edge(e) => *e,
            _ => unreachable!(),
        })
        .collect();
    // The copy lands between the squares: its left side lies on the left square's right edge.
    s.edit(MeshId::Root, |m| m.copy(&edges, &[], p(-2.0, 0.0, 0.0), 1))
        .unwrap();
    s.validate().unwrap();
    let m = s.root();
    let at_x = |x: f64| {
        m.edges()
            .find(|(_, e)| e.v.iter().all(|v| m.position(*v).unwrap().x == x))
            .unwrap()
            .1
            .tag
    };
    assert_eq!(at_x(2.0), Some(walls));
    assert_eq!(at_x(0.0), None);
}

#[test]
fn split_edges_and_faces_keep_their_tags_and_merges_keep_them_too() {
    let (mut s, _) = square_scene();
    let walls = tag(&mut s, "Walls");
    let ids = all_ids(s.root());
    s.set_tags(MeshId::Root, &ids, Some(walls)).unwrap();
    // A line across the square splits two of its edges and the face.
    s.edit(MeshId::Root, |m| {
        m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 2.0, 0.0))
    })
    .unwrap();
    s.validate().unwrap();
    assert_eq!(s.root().faces().count(), 2);
    assert!(s.root().faces().all(|(_, f)| f.tag == Some(walls)));
    let seam = s
        .root()
        .edges()
        .find(|(_, e)| e.v.iter().all(|v| s.root().position(*v).unwrap().x == 1.0))
        .unwrap()
        .0;
    // The new line is Untagged; the pieces of the old edges keep their tag.
    assert_eq!(s.root().edge(seam).unwrap().tag, None);
    assert!(
        s.root()
            .edges()
            .filter(|(e, _)| *e != seam)
            .all(|(_, e)| e.tag == Some(walls))
    );
    assert_eq!(s.root().edges().count(), 7);
    // Erasing the seam merges the faces and joins the split edges: all keep the tag.
    s.edit(MeshId::Root, |m| Ok(m.erase(&[seam], &[]))).unwrap();
    s.validate().unwrap();
    assert_eq!(s.root().faces().count(), 1);
    assert!(s.root().faces().all(|(_, f)| f.tag == Some(walls)));
    assert!(s.root().edges().all(|(_, e)| e.tag == Some(walls)));
}

#[test]
fn copies_keep_tags() {
    let (mut s, face) = square_scene();
    let walls = tag(&mut s, "Walls");
    let ids = all_ids(s.root());
    s.set_tags(MeshId::Root, &ids, Some(walls)).unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_guide(Guide::point(p(1.0, 1.0, 0.0)))
    })
    .unwrap();
    let guide = s.root().guides().next().unwrap().0;
    s.set_tags(MeshId::Root, &[EntityId::Guide(guide)], Some(walls))
        .unwrap();
    let e = edges(s.root());
    s.edit(MeshId::Root, |m| m.copy(&e, &[face], p(5.0, 0.0, 0.0), 2))
        .unwrap();
    s.edit(MeshId::Root, |m| {
        Ok(m.copy_guides(
            &[guide],
            &[glam::DAffine3::from_translation(p(0.0, 5.0, 0.0))],
        ))
    })
    .unwrap();
    s.validate().unwrap();
    assert_eq!(s.root().faces().count(), 3);
    assert_eq!(s.root().edges().count(), 12);
    assert!(s.root().faces().all(|(_, f)| f.tag == Some(walls)));
    assert!(s.root().edges().all(|(_, e)| e.tag == Some(walls)));
    assert_eq!(s.root().guides().count(), 2);
    assert!(s.root().guides().all(|(_, g)| g.tag() == Some(walls)));

    // A copied instance keeps its tag.
    let (mut s, group) = grouped_square();
    let walls = tag(&mut s, "Walls");
    s.set_tags(MeshId::Root, &[EntityId::Instance(group)], Some(walls))
        .unwrap();
    s.copy_instances(MeshId::Root, &[group], p(0.0, 5.0, 0.0), 1)
        .unwrap();
    assert_eq!(s.root().instances().count(), 2);
    assert!(s.root().instances().all(|(_, i)| i.tag == Some(walls)));
}

#[test]
fn make_group_keeps_tags_and_explode_passes_the_instance_tag_to_untagged_contents() {
    let (mut s, face) = square_scene();
    let walls = tag(&mut s, "Walls");
    let roof = tag(&mut s, "Roof");
    s.set_tags(MeshId::Root, &[EntityId::Face(face)], Some(walls))
        .unwrap();
    let e = edges(s.root());
    let (group, _) = s
        .make_group(MeshId::Root, &e, &[face], &[], DefinitionKind::Group, "")
        .unwrap();
    // The new instance is Untagged; what went inside keeps its tags.
    assert_eq!(s.root().instance(group).unwrap().tag, None);
    let d = MeshId::Definition(s.root().instance(group).unwrap().definition);
    assert!(
        s.mesh(d)
            .unwrap()
            .faces()
            .all(|(_, f)| f.tag == Some(walls))
    );
    assert!(s.mesh(d).unwrap().edges().all(|(_, e)| e.tag.is_none()));
    // A guide and a nested group inside, both Untagged, plus a guide on Walls.
    s.edit(d, |m| m.add_guide(Guide::point(p(1.0, 1.0, 1.0))))
        .unwrap();
    s.edit(d, |m| {
        m.add_guide(Guide::Point {
            position: p(1.0, 1.0, 2.0),
            tag: Some(walls),
        })
    })
    .unwrap();
    let inner_edges = edges(s.mesh(d).unwrap());
    s.make_group(d, &inner_edges[..1], &[], &[], DefinitionKind::Group, "")
        .unwrap();
    s.set_tags(MeshId::Root, &[EntityId::Instance(group)], Some(roof))
        .unwrap();
    s.validate().unwrap();

    s.explode(MeshId::Root, group).unwrap();
    s.validate().unwrap();
    let m = s.root();
    // The face keeps its own tag; the Untagged edges, guide and nested instance take Roof.
    assert!(m.faces().all(|(_, f)| f.tag == Some(walls)));
    assert!(m.edges().all(|(_, e)| e.tag == Some(roof)));
    let guide_tags: Vec<Option<TagId>> = m.guides().map(|(_, g)| g.tag()).collect();
    assert_eq!(guide_tags.len(), 2);
    assert!(guide_tags.contains(&Some(roof)) && guide_tags.contains(&Some(walls)));
    assert!(m.instances().all(|(_, i)| i.tag == Some(roof)));
    assert_eq!(m.instances().count(), 1);
}

#[test]
fn pulling_a_tagged_face_makes_untagged_geometry() {
    for distance in [1.0, -1.0] {
        let (mut s, face) = square_scene();
        let walls = tag(&mut s, "Walls");
        let ids = all_ids(s.root());
        s.set_tags(MeshId::Root, &ids, Some(walls)).unwrap();
        s.edit(MeshId::Root, |m| m.push_pull(face, distance))
            .unwrap();
        s.validate().unwrap();
        let m = s.root();
        assert_eq!(m.faces().count(), 6);
        assert_eq!(m.edges().count(), 12);
        // The source face and its edges keep their tag; the cap, the sides and the new edges
        // are new geometry, Untagged.
        assert_eq!(m.face(face).unwrap().tag, Some(walls));
        for (id, f) in m.faces() {
            assert_eq!(f.tag.is_some(), id == face, "{id:?}");
        }
        for (id, e) in m.edges() {
            let old = ids.contains(&EntityId::Edge(id));
            assert_eq!(e.tag, old.then_some(walls), "{id:?}");
        }
    }
}

#[test]
fn an_import_tree_with_a_guide_on_a_missing_tag_is_refused() {
    let (mut s, _) = square_scene();
    let walls = tag(&mut s, "Walls");
    let tree = |t: TagId| kernel::ImportTree {
        root: kernel::ImportGeometry {
            guides: vec![Guide::point(p(1.0, 1.0, 1.0)).with_tag(Some(t))],
            ..kernel::ImportGeometry::default()
        },
        definitions: Vec::new(),
    };
    let before = s.clone();
    let refused = s.import_tree(MeshId::Root, &tree(TagId(999)), DefinitionKind::Group, "x");
    assert_eq!(refused.unwrap_err(), KernelError::TagNotFound);
    assert_eq!(s, before);
    let (placed, _, _) = s
        .import_tree(MeshId::Root, &tree(walls), DefinitionKind::Group, "x")
        .unwrap();
    s.validate().unwrap();
    let d = MeshId::Definition(s.root().instance(placed).unwrap().definition);
    assert!(
        s.mesh(d)
            .unwrap()
            .guides()
            .all(|(_, g)| g.tag() == Some(walls))
    );
}

#[test]
fn tags_survive_files_and_bad_tag_data_is_rejected() {
    let (mut s, group) = grouped_square();
    let walls = tag(&mut s, "Walls");
    let hidden = s
        .add_tag(Tag {
            name: "Hidden".into(),
            color: [9, 8, 7],
            visible: false,
        })
        .unwrap()
        .0;
    let d = MeshId::Definition(s.root().instance(group).unwrap().definition);
    let inside: Vec<EntityId> = {
        let m = s.mesh(d).unwrap();
        let mut ids = all_ids(m);
        ids.extend(m.guides().map(|(g, _)| EntityId::Guide(g)));
        ids
    };
    s.set_tags(d, &inside, Some(walls)).unwrap();
    s.set_tags(MeshId::Root, &[EntityId::Instance(group)], Some(hidden))
        .unwrap();
    let json = serde_json::to_string(&s.to_data()).unwrap();
    let data: SceneData = serde_json::from_str(&json).unwrap();
    assert_eq!(data.tags.len(), 2);
    // A shown tag writes no `visible`.
    assert!(json.contains(r#""visible":false"#));
    assert_eq!(json.matches(r#""visible""#).count(), 1);
    let back = Scene::from_data(&data).unwrap();
    assert_eq!(back, s);
    assert!(!back.tag(hidden).unwrap().visible);

    // An entity on a tag the file lacks is refused.
    let mut bad = s.to_data();
    bad.tags.retain(|t| t.id != walls.0);
    let err = Scene::from_data(&bad).unwrap_err();
    assert!(err.contains("missing tag"), "{err}");
    // A tag named "Untagged", or two tags named the same, are refused.
    let mut bad = s.to_data();
    bad.tags[0].name = "Untagged".into();
    assert!(Scene::from_data(&bad).is_err());
    let mut bad = s.to_data();
    bad.tags[1].name = bad.tags[0].name.clone();
    assert!(Scene::from_data(&bad).is_err());
    // A tag sharing an id with an entity is refused.
    let mut bad = s.to_data();
    bad.tags[0].id = bad.root.instances[0].id;
    assert!(Scene::from_data(&bad).is_err());

    // Data written before tags (no `tags`, no `tag` fields) loads Untagged.
    let (plain, _) = grouped_square();
    let json = serde_json::to_string(&plain.to_data()).unwrap();
    assert!(!json.contains("tag"), "{json}");
    let back = Scene::from_data(&serde_json::from_str(&json).unwrap()).unwrap();
    assert_eq!(back.tags().count(), 0);
}
