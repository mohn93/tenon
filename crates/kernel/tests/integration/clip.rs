use std::collections::BTreeSet;
use std::f64::consts::FRAC_PI_2;
use std::sync::Arc;

use glam::{DAffine3, DVec3};
use kernel::{
    DefinitionId, DefinitionKind, EdgeId, EntityId, FaceId, Guide, GuideId, ImageFormat, Inserted,
    InstanceId, KernelError, MAX_TEXTURES, Material, MeshId, Model, PlacementEdit, Scene, Side,
    Tag, Texture, TextureImage,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn texture() -> Texture {
    Texture {
        image: Arc::new(TextureImage {
            format: ImageFormat::Png,
            encoded: b"not decoded by the kernel".to_vec(),
            width: 2,
            height: 1,
            pixels: vec![255, 0, 0, 255, 0, 0, 255, 255],
        }),
        width: 1.0,
        height: 1.0,
    }
}

fn tag(name: &str) -> Tag {
    Tag {
        name: name.into(),
        color: [1, 2, 3],
        visible: true,
    }
}

fn square(lo: DVec3, size: f64) -> [DVec3; 4] {
    [
        lo,
        lo + p(size, 0.0, 0.0),
        lo + p(size, size, 0.0),
        lo + p(0.0, size, 0.0),
    ]
}

fn all(m: &Model) -> (Vec<EdgeId>, Vec<FaceId>) {
    (
        m.edges().map(|(id, _)| id).collect(),
        m.faces().map(|(id, _)| id).collect(),
    )
}

fn instances(m: &Model) -> Vec<InstanceId> {
    m.instances().map(|(id, _)| id).collect()
}

fn guides(m: &Model) -> Vec<GuideId> {
    m.guides().map(|(id, _)| id).collect()
}

/// A point rounded to micrometers, so it compares exactly.
fn r(v: DVec3) -> [i64; 3] {
    (v * 1e6).round().to_array().map(|c| c as i64)
}

fn material_name(s: &Scene, id: Option<kernel::MaterialId>) -> Option<String> {
    id.map(|m| s.material(m).unwrap().name.clone())
}

fn tag_name(s: &Scene, id: Option<kernel::TagId>) -> Option<String> {
    id.map(|t| s.tag(t).unwrap().name.clone())
}

/// A mesh described without ids: its edges, faces, instances and guides by position and by
/// the names of what they use.
fn describe(s: &Scene, mesh: MeshId) -> Vec<String> {
    let m = s.mesh(mesh).unwrap();
    let mut out = Vec::new();
    for (_, e) in m.edges() {
        let mut ends = [
            r(m.position(e.v[0]).unwrap()),
            r(m.position(e.v[1]).unwrap()),
        ];
        ends.sort();
        out.push(format!(
            "edge {ends:?} soft {} curve {} polygon {} tag {:?}",
            e.soft,
            e.curve.is_some(),
            e.polygon,
            tag_name(s, e.tag)
        ));
    }
    for (_, f) in m.faces() {
        let mut outer: Vec<[i64; 3]> = f.outer.iter().map(|v| r(m.position(*v).unwrap())).collect();
        outer.sort();
        let [front, back] = f.paint();
        out.push(format!(
            "face {outer:?} holes {} normal {:?} paint {:?} {:?} texture {:?} tag {:?}",
            f.holes.len(),
            r(f.normal),
            material_name(s, front),
            material_name(s, back),
            f.texture,
            tag_name(s, f.tag)
        ));
    }
    for (_, i) in m.instances() {
        let d = s.definition(i.definition).unwrap();
        out.push(format!(
            "instance {} {:?} at {:?} {:?} paint {:?} tag {:?} holding {:?}",
            d.name,
            d.kind,
            r(i.transform.translation),
            r(i.transform.matrix3.x_axis),
            material_name(s, i.material),
            tag_name(s, i.tag),
            describe(s, MeshId::Definition(i.definition))
        ));
    }
    for (_, g) in m.guides() {
        out.push(format!(
            "guide {:?} tag {:?}",
            r(g.anchor()),
            tag_name(s, g.tag())
        ));
    }
    out.sort();
    out
}

/// A scene whose root holds a 4 m square with a 2 m opening, painted with a textured material
/// whose placement is moved, on a tag, beside a soft curve on another tag; plus a material and
/// a tag nothing uses.
fn painted_scene() -> (Scene, FaceId) {
    let mut s = Scene::new();
    s.add_material("Unused", [9, 9, 9]).unwrap();
    let (red, _) = s
        .add_material_with_texture("Red", [200, 0, 0], 1.0, Some(texture()))
        .unwrap();
    s.add_tag(tag("Spare")).unwrap();
    let (walls, _) = s.add_tag(tag("Walls")).unwrap();
    let (curves, _) = s.add_tag(tag("Curves")).unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&square(p(0.0, 0.0, 0.0), 4.0))
    })
    .unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&square(p(1.0, 1.0, 0.0), 2.0))
    })
    .unwrap();
    let inner = s
        .root()
        .faces()
        .find(|(_, f)| f.holes.is_empty())
        .map(|(id, _)| id)
        .unwrap();
    s.edit(MeshId::Root, |m| Ok(m.erase(&[], &[inner])))
        .unwrap();
    let outer = s.root().faces().next().map(|(id, _)| id).unwrap();
    assert_eq!(s.root().face(outer).unwrap().holes.len(), 1);
    s.paint(MeshId::Root, &[outer], &[], Side::Front, Some(red))
        .unwrap();
    s.edit(MeshId::Root, |m| {
        let edit = PlacementEdit {
            offset: Some([0.25, 0.5]),
            rotation_deg: Some(30.0),
            ..PlacementEdit::default()
        };
        Ok(m.position_texture(&[outer], edit)?.0)
    })
    .unwrap();
    s.set_tags(MeshId::Root, &[EntityId::Face(outer)], Some(walls))
        .unwrap();
    let arc = [
        p(6.0, 0.0, 0.0),
        p(7.0, 1.0, 0.0),
        p(8.0, 1.0, 0.0),
        p(9.0, 0.0, 0.0),
    ];
    s.edit(MeshId::Root, |m| m.add_curve(&arc, false, false))
        .unwrap();
    let curve_edges: Vec<EdgeId> = s
        .root()
        .edges()
        .filter(|(_, e)| e.curve.is_some())
        .map(|(id, _)| id)
        .collect();
    assert_eq!(curve_edges.len(), 3);
    s.edit(MeshId::Root, |m| Ok(m.soften_edges(&curve_edges, true).0))
        .unwrap();
    let ids: Vec<EntityId> = curve_edges.iter().map(|e| EntityId::Edge(*e)).collect();
    s.set_tags(MeshId::Root, &ids, Some(curves)).unwrap();
    s.validate().unwrap();
    (s, outer)
}

fn names<'a>(it: impl Iterator<Item = &'a str>) -> BTreeSet<String> {
    it.map(str::to_string).collect()
}

fn material_names(s: &Scene) -> BTreeSet<String> {
    names(s.materials().map(|(_, m)| m.name.as_str()))
}

fn tag_names(s: &Scene) -> BTreeSet<String> {
    names(s.tags().map(|(_, t)| t.name.as_str()))
}

fn extract_root(s: &Scene, to_world: DAffine3) -> Scene {
    let (edges, faces) = all(s.root());
    let clip = s
        .extract(
            MeshId::Root,
            &edges,
            &faces,
            &instances(s.root()),
            &guides(s.root()),
            to_world,
        )
        .unwrap();
    clip.validate().unwrap();
    clip
}

fn no_reuse(_: DefinitionId) -> Option<DefinitionId> {
    None
}

fn insert_root(target: &mut Scene, clip: &Scene, placement: DAffine3) -> Inserted {
    let before = target.clone();
    let (inserted, change) = target
        .insert(MeshId::Root, clip, placement, &no_reuse)
        .unwrap();
    target.validate().unwrap();
    let mut undone = target.clone();
    undone.apply(&change.inverse());
    assert!(undone == before, "undo restores the target");
    inserted
}

#[test]
fn a_painted_face_with_an_opening_and_a_soft_curve_round_trips() {
    let (s, _) = painted_scene();
    let clip = extract_root(&s, DAffine3::IDENTITY);
    assert_eq!(material_names(&clip), names(["Red"].into_iter()));
    assert_eq!(tag_names(&clip), names(["Walls", "Curves"].into_iter()));
    let red = clip.materials().next().unwrap().1;
    assert_eq!(red.texture, s.material(red_id(&s)).unwrap().texture);
    assert_eq!(describe(&clip, MeshId::Root), describe(&s, MeshId::Root));

    let mut target = Scene::new();
    let inserted = insert_root(&mut target, &clip, DAffine3::IDENTITY);
    assert_eq!(describe(&target, MeshId::Root), describe(&s, MeshId::Root));
    assert_eq!(material_names(&target), names(["Red"].into_iter()));
    assert_eq!(tag_names(&target), names(["Walls", "Curves"].into_iter()));
    let (edges, faces) = all(target.root());
    assert_eq!(inserted.edges, edges);
    assert_eq!(inserted.faces, faces);
    assert!(inserted.instances.is_empty() && inserted.guides.is_empty());
}

fn red_id(s: &Scene) -> kernel::MaterialId {
    s.materials().find(|(_, m)| m.name == "Red").unwrap().0
}

#[test]
fn only_the_selection_and_what_it_uses_goes_into_the_clip() {
    let (s, outer) = painted_scene();
    let clip = s
        .extract(MeshId::Root, &[], &[outer], &[], &[], DAffine3::IDENTITY)
        .unwrap();
    clip.validate().unwrap();
    assert_eq!(material_names(&clip), names(["Red"].into_iter()));
    assert_eq!(tag_names(&clip), names(["Walls"].into_iter()));
    assert_eq!(clip.root().faces().count(), 1);
    // The face's eight sides come with it; the curve does not.
    assert_eq!(clip.root().edges().count(), 8);
}

#[test]
fn an_empty_selection_makes_an_empty_clip() {
    let (s, _) = painted_scene();
    let clip = s
        .extract(MeshId::Root, &[], &[], &[], &[], DAffine3::IDENTITY)
        .unwrap();
    assert!(clip == Scene::new());
    assert_eq!(clip.materials().count(), 0);
    assert_eq!(clip.tags().count(), 0);
}

#[test]
fn extracting_from_a_missing_mesh_or_missing_instances_fails() {
    let (s, _) = painted_scene();
    let missing = MeshId::Definition(DefinitionId(999_999));
    assert_eq!(
        s.extract(missing, &[], &[], &[], &[], DAffine3::IDENTITY),
        Err(KernelError::DefinitionNotFound)
    );
    assert_eq!(
        s.extract(
            MeshId::Root,
            &[],
            &[],
            &[InstanceId(999_999)],
            &[],
            DAffine3::IDENTITY
        ),
        Err(KernelError::InstanceNotFound)
    );
}

/// A scene whose root holds two instances of a "Table" component, each holding two "Leg"
/// component instances and a "Top" group, with the tables on a tag and painted.
fn furniture() -> (Scene, [InstanceId; 2]) {
    let mut s = Scene::new();
    let (oak, _) = s.add_material("Oak", [150, 100, 50]).unwrap();
    let (furniture, _) = s.add_tag(tag("Furniture")).unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&square(p(0.0, 0.0, 0.0), 1.0))
    })
    .unwrap();
    let (e, f) = all(s.root());
    let (leg, _) = s
        .make_group(MeshId::Root, &e, &f, &[], DefinitionKind::Component, "Leg")
        .unwrap();
    s.copy_instances(MeshId::Root, &[leg], p(3.0, 0.0, 0.0), 1)
        .unwrap();
    let legs = instances(s.root());
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&square(p(0.0, 0.0, 2.0), 4.0))
    })
    .unwrap();
    let (e, f) = all(s.root());
    let (top, _) = s
        .make_group(MeshId::Root, &e, &f, &[], DefinitionKind::Group, "Top")
        .unwrap();
    let mut parts = legs.clone();
    parts.push(top);
    let (table, _) = s
        .make_group(
            MeshId::Root,
            &[],
            &[],
            &parts,
            DefinitionKind::Component,
            "Table",
        )
        .unwrap();
    s.paint(MeshId::Root, &[], &[table], Side::Front, Some(oak))
        .unwrap();
    s.set_tags(MeshId::Root, &[EntityId::Instance(table)], Some(furniture))
        .unwrap();
    s.copy_instances(MeshId::Root, &[table], p(10.0, 0.0, 0.0), 1)
        .unwrap();
    let tables = instances(s.root());
    s.validate().unwrap();
    (s, [tables[0], tables[1]])
}

fn definition_count(s: &Scene, name: &str) -> usize {
    s.definitions().filter(|(_, d)| d.name == name).count()
}

#[test]
fn nested_components_and_groups_come_along_and_components_stay_shared() {
    let (s, tables) = furniture();
    let clip = s
        .extract(MeshId::Root, &[], &[], &tables, &[], DAffine3::IDENTITY)
        .unwrap();
    clip.validate().unwrap();
    assert_eq!(clip.definitions().count(), 3);
    let defs: BTreeSet<DefinitionId> = clip.root().instances().map(|(_, i)| i.definition).collect();
    assert_eq!(defs.len(), 1, "both tables place one definition");
    assert_eq!(material_names(&clip), names(["Oak"].into_iter()));
    assert_eq!(tag_names(&clip), names(["Furniture"].into_iter()));
    assert_eq!(describe(&clip, MeshId::Root), describe(&s, MeshId::Root));

    let mut target = Scene::new();
    let inserted = insert_root(&mut target, &clip, DAffine3::IDENTITY);
    assert_eq!(describe(&target, MeshId::Root), describe(&s, MeshId::Root));
    assert_eq!(inserted.instances, instances(target.root()));
    assert_eq!(target.definitions().count(), 3);
    let defs: BTreeSet<DefinitionId> = target
        .root()
        .instances()
        .map(|(_, i)| i.definition)
        .collect();
    assert_eq!(defs.len(), 1);

    // Pasting again without reuse makes new definitions of everything.
    insert_root(
        &mut target,
        &clip,
        DAffine3::from_translation(p(0.0, 20.0, 0.0)),
    );
    assert_eq!(definition_count(&target, "Table"), 2);
    assert_eq!(definition_count(&target, "Leg"), 2);
    assert_eq!(definition_count(&target, "Top"), 2);
}

#[test]
fn reuse_places_existing_component_definitions_but_groups_are_always_new() {
    let (mut s, tables) = furniture();
    let clip = s
        .extract(MeshId::Root, &[], &[], &tables, &[], DAffine3::IDENTITY)
        .unwrap();
    let before = s.clone();
    let table = s.root().instance(tables[0]).unwrap().definition;
    let reuse = |d: DefinitionId| {
        // Offer the definition itself for every clip definition, the group included.
        Some(d)
    };
    let (inserted, change) = s
        .insert(
            MeshId::Root,
            &clip,
            DAffine3::from_translation(p(0.0, 20.0, 0.0)),
            &reuse,
        )
        .unwrap();
    s.validate().unwrap();
    assert_eq!(inserted.instances.len(), 2);
    for id in &inserted.instances {
        assert_eq!(s.root().instance(*id).unwrap().definition, table);
    }
    assert_eq!(definition_count(&s, "Table"), 1);
    assert_eq!(definition_count(&s, "Leg"), 1);
    assert_eq!(definition_count(&s, "Top"), 1);
    s.apply(&change.inverse());
    assert!(s == before);

    // A group offered for reuse is copied all the same.
    let top = s
        .definitions()
        .find(|(_, d)| d.name == "Top")
        .map(|(id, _)| id)
        .unwrap();
    let group_clip = s
        .extract(
            MeshId::Definition(table),
            &[],
            &[],
            &instances(&s.definition(table).unwrap().mesh),
            &[],
            DAffine3::IDENTITY,
        )
        .unwrap();
    let (inserted, _) = s
        .insert(MeshId::Root, &group_clip, DAffine3::IDENTITY, &reuse)
        .unwrap();
    s.validate().unwrap();
    let placed: Vec<DefinitionId> = inserted
        .instances
        .iter()
        .map(|id| s.root().instance(*id).unwrap().definition)
        .collect();
    assert!(!placed.contains(&top));
    assert_eq!(definition_count(&s, "Top"), 2);
    assert_eq!(definition_count(&s, "Leg"), 1);
}

#[test]
fn reuse_of_a_missing_definition_makes_a_new_one() {
    let (s, tables) = furniture();
    let clip = s
        .extract(MeshId::Root, &[], &[], &tables, &[], DAffine3::IDENTITY)
        .unwrap();
    let mut target = Scene::new();
    let reuse = |d: DefinitionId| Some(d);
    target
        .insert(MeshId::Root, &clip, DAffine3::IDENTITY, &reuse)
        .unwrap();
    target.validate().unwrap();
    assert_eq!(target.definitions().count(), 3);
}

#[test]
fn guides_come_along_on_their_tags() {
    let mut s = Scene::new();
    let (t, _) = s.add_tag(tag("Lines")).unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_guide(Guide::point(p(1.0, 2.0, 3.0)))
    })
    .unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_guide(Guide::line(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)).with_tag(Some(t)))
    })
    .unwrap();
    let clip = extract_root(&s, DAffine3::IDENTITY);
    assert_eq!(clip.root().guides().count(), 2);
    assert_eq!(tag_names(&clip), names(["Lines"].into_iter()));
    let mut target = Scene::new();
    let inserted = insert_root(
        &mut target,
        &clip,
        DAffine3::from_translation(p(0.0, 0.0, 1.0)),
    );
    assert_eq!(inserted.guides.len(), 2);
    let anchors: BTreeSet<[i64; 3]> = target.root().guides().map(|(_, g)| r(g.anchor())).collect();
    assert_eq!(
        anchors,
        [r(p(1.0, 2.0, 4.0)), r(p(0.0, 0.0, 1.0))]
            .into_iter()
            .collect()
    );
}

fn vertex_positions(m: &Model) -> BTreeSet<[i64; 3]> {
    m.vertices().map(|(_, v)| r(v.pos)).collect()
}

#[test]
fn extraction_takes_the_selection_into_world_coordinates() {
    let mut s = Scene::new();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&square(p(0.0, 0.0, 0.0), 1.0))
    })
    .unwrap();
    let (e, f) = all(s.root());
    let (g, _) = s
        .make_group(MeshId::Root, &e, &f, &[], DefinitionKind::Group, "G")
        .unwrap();
    let d = s.root().instance(g).unwrap().definition;
    let mesh = s.definition(d).unwrap().mesh.clone();
    let (e, f) = all(&mesh);
    let to_world = DAffine3::from_translation(p(5.0, 0.0, 0.0));
    let clip = s
        .extract(MeshId::Definition(d), &e, &f, &[], &[], to_world)
        .unwrap();
    assert_eq!(
        vertex_positions(clip.root()),
        square(p(5.0, 0.0, 0.0), 1.0)
            .iter()
            .map(|v| r(*v))
            .collect()
    );
}

#[test]
fn inserting_translates_and_rotates_the_clip() {
    let mut s = Scene::new();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&square(p(0.0, 0.0, 0.0), 1.0))
    })
    .unwrap();
    let clip = extract_root(&s, DAffine3::IDENTITY);

    let mut moved = Scene::new();
    insert_root(
        &mut moved,
        &clip,
        DAffine3::from_translation(p(10.0, 0.0, 0.0)),
    );
    assert_eq!(
        vertex_positions(moved.root()),
        square(p(10.0, 0.0, 0.0), 1.0)
            .iter()
            .map(|v| r(*v))
            .collect()
    );

    let mut turned = Scene::new();
    insert_root(&mut turned, &clip, DAffine3::from_rotation_z(FRAC_PI_2));
    assert_eq!(
        vertex_positions(turned.root()),
        [
            p(0.0, 0.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(-1.0, 1.0, 0.0),
            p(-1.0, 0.0, 0.0)
        ]
        .iter()
        .map(|v| r(*v))
        .collect()
    );
    assert_eq!(turned.root().faces().count(), 1);
}

#[test]
fn insertion_matches_materials_and_tags_by_name_and_adds_missing_ones() {
    let (s, _) = painted_scene();
    let clip = extract_root(&s, DAffine3::IDENTITY);
    let mut target = Scene::new();
    let (red, _) = target.add_material("Red", [0, 0, 255]).unwrap();
    let (walls, _) = target.add_tag(tag("Walls")).unwrap();
    insert_root(&mut target, &clip, DAffine3::IDENTITY);
    assert_eq!(material_names(&target), names(["Red"].into_iter()));
    assert_eq!(tag_names(&target), names(["Walls", "Curves"].into_iter()));
    // The existing material is used as it is (not replaced by the clip's).
    assert_eq!(target.material(red).unwrap().color, [0, 0, 255]);
    let (_, face) = target.root().faces().next().unwrap();
    assert_eq!(face.paint()[0], Some(red));
    assert_eq!(face.tag, Some(walls));
}

#[test]
fn pasting_onto_existing_geometry_merges_and_undo_restores_it() {
    let (s, _) = painted_scene();
    let clip = extract_root(&s, DAffine3::IDENTITY);
    let mut target = Scene::new();
    target
        .edit(MeshId::Root, |m| {
            m.add_polygon(&square(p(-2.0, -2.0, 0.0), 4.0))
        })
        .unwrap();
    let inserted = insert_root(&mut target, &clip, DAffine3::IDENTITY);
    // The pasted square's sides crossing the existing square are split where they cross.
    assert!(target.root().faces().count() > 2);
    // The pieces of the pasted face are the faces pasted; the existing square's pieces and
    // the opening's are not.
    let red: Vec<FaceId> = target
        .root()
        .faces()
        .filter(|(_, f)| f.paint()[0].is_some())
        .map(|(id, _)| id)
        .collect();
    assert_eq!(red.len(), 2);
    assert_eq!(inserted.faces, red);
    for e in &inserted.edges {
        assert!(target.root().edge(*e).is_some());
    }
}

#[test]
fn a_failed_insert_changes_nothing() {
    let mut s = Scene::new();
    let (plain, _) = s.add_material("Plain", [1, 1, 1]).unwrap();
    let (tex, _) = s
        .add_material_from(Material {
            name: "Tex".into(),
            color: [2, 2, 2],
            opacity: 1.0,
            texture: Some(texture()),
            emission: 0.0,
        })
        .unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&square(p(0.0, 0.0, 0.0), 1.0))
    })
    .unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&square(p(2.0, 0.0, 0.0), 1.0))
    })
    .unwrap();
    let faces: Vec<FaceId> = all(s.root()).1;
    s.paint(MeshId::Root, &faces[..1], &[], Side::Front, Some(plain))
        .unwrap();
    s.paint(MeshId::Root, &faces[1..], &[], Side::Front, Some(tex))
        .unwrap();
    let clip = extract_root(&s, DAffine3::IDENTITY);

    let mut full = Scene::new();
    for k in 0..MAX_TEXTURES {
        full.add_material_with_texture(&format!("T{k}"), [0, 0, 0], 1.0, Some(texture()))
            .unwrap();
    }
    let before = full.clone();
    assert!(matches!(
        full.insert(MeshId::Root, &clip, DAffine3::IDENTITY, &no_reuse),
        Err(KernelError::MaterialTexture(_))
    ));
    assert!(full == before);

    let missing = MeshId::Definition(DefinitionId(999_999));
    assert_eq!(
        full.insert(missing, &clip, DAffine3::IDENTITY, &no_reuse)
            .map(|_| ()),
        Err(KernelError::DefinitionNotFound)
    );
    let nan = DAffine3::from_translation(p(f64::NAN, 0.0, 0.0));
    assert_eq!(
        full.insert(MeshId::Root, &clip, nan, &no_reuse).map(|_| ()),
        Err(KernelError::NotFinite)
    );
    assert!(full == before);
}

#[test]
fn copying_inside_a_turned_group_and_pasting_in_place_at_the_root_keeps_world_positions() {
    let mut s = Scene::new();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&square(p(0.0, 0.0, 0.0), 1.0))
    })
    .unwrap();
    let (e, f) = all(s.root());
    let (g, _) = s
        .make_group(MeshId::Root, &e, &f, &[], DefinitionKind::Group, "G")
        .unwrap();
    let t = DAffine3::from_translation(p(3.0, 1.0, 0.0))
        * DAffine3::from_rotation_z(0.7)
        * DAffine3::from_scale(p(2.0, 2.0, 2.0));
    s.edit(MeshId::Root, |m| Ok(m.transform_instances(&[g], t)))
        .unwrap();
    let to_world = s.root().instance(g).unwrap().transform;
    let d = s.root().instance(g).unwrap().definition;
    let mesh = s.definition(d).unwrap().mesh.clone();
    let (e, f) = all(&mesh);
    let clip = s
        .extract(MeshId::Definition(d), &e, &f, &[], &[], to_world)
        .unwrap();
    let world: BTreeSet<[i64; 3]> = mesh
        .vertices()
        .map(|(_, v)| r(to_world.transform_point3(v.pos)))
        .collect();
    let inserted = insert_root(&mut s, &clip, DAffine3::IDENTITY);
    assert_eq!(inserted.faces.len(), 1);
    assert_eq!(vertex_positions(s.root()), world);

    // And back: pasted in place inside the group, it lands on the original.
    let back = insert_root_in(&mut s, MeshId::Definition(d), &clip, to_world.inverse());
    assert_eq!(back.faces.len(), 1);
    assert_eq!(
        vertex_positions(&s.definition(d).unwrap().mesh),
        vertex_positions(&mesh)
    );
}

fn insert_root_in(
    target: &mut Scene,
    parent: MeshId,
    clip: &Scene,
    placement: DAffine3,
) -> Inserted {
    let (inserted, _) = target.insert(parent, clip, placement, &no_reuse).unwrap();
    target.validate().unwrap();
    inserted
}

#[test]
fn a_component_pasted_into_itself_gets_a_copy_instead() {
    let (mut s, tables) = furniture();
    let clip = s
        .extract(
            MeshId::Root,
            &[],
            &[],
            &tables[..1],
            &[],
            DAffine3::IDENTITY,
        )
        .unwrap();
    let table = s.root().instance(tables[0]).unwrap().definition;
    let leg = s
        .definitions()
        .find(|(_, d)| d.name == "Leg")
        .map(|(id, _)| id)
        .unwrap();
    let reuse = |d: DefinitionId| Some(d);
    let (inserted, _) = s
        .insert(MeshId::Definition(table), &clip, DAffine3::IDENTITY, &reuse)
        .unwrap();
    s.validate().unwrap();
    let placed = s
        .definition(table)
        .unwrap()
        .mesh
        .instance(inserted.instances[0])
        .unwrap()
        .definition;
    assert_ne!(placed, table);
    assert_eq!(definition_count(&s, "Table"), 2);
    // The legs inside the copy still share the existing Leg.
    assert_eq!(definition_count(&s, "Leg"), 1);
    assert!(s.instance_count(leg) > 2);
}

#[test]
fn pasting_onto_the_original_selects_the_faces_it_lands_on() {
    let mut s = Scene::new();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&square(p(0.0, 0.0, 0.0), 1.0))
    })
    .unwrap();
    let clip = extract_root(&s, DAffine3::IDENTITY);
    let before = s.clone();
    let inserted = insert_root(&mut s, &clip, DAffine3::IDENTITY);
    assert!(s == before);
    let (edges, faces) = all(s.root());
    assert_eq!((inserted.edges, inserted.faces), (edges, faces));
}
