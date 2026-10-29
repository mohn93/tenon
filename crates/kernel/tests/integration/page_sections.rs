//! Scenes that store section planes: refused when wrong, dropped with the plane, kept in data.

use std::collections::BTreeMap;

use glam::DVec3;
use kernel::{
    EntityId, KernelError, MeshId, Page, PageSections, Scene, SceneData, SectionId, SectionPlane,
    Step,
};

fn page(sections: Option<PageSections>) -> Page {
    Page {
        name: "A".into(),
        view: None,
        tags: None,
        positions: None,
        joints: None,
        sections,
    }
}

fn with_plane(scene: &mut Scene) -> SectionId {
    let change = scene
        .edit(MeshId::Root, |m| {
            m.add_section(SectionPlane::new(DVec3::ZERO, DVec3::Z), true)
        })
        .unwrap();
    change
        .steps
        .iter()
        .find_map(|s| match s {
            Step::Edit(_, c) => c.summary().created.iter().find_map(|e| match e {
                EntityId::Section(s) => Some(*s),
                _ => None,
            }),
            _ => None,
        })
        .expect("a plane")
}

fn storing(id: SectionId) -> PageSections {
    PageSections {
        active: BTreeMap::from([(MeshId::Root, id)]),
        ..PageSections::none()
    }
}

#[test]
fn a_page_can_store_only_planes_that_exist_in_their_context() {
    let mut s = Scene::new();
    let id = with_plane(&mut s);
    assert!(s.add_page(page(Some(storing(id)))).is_ok());
    let mut other = page(Some(storing(SectionId(999))));
    other.name = "B".into();
    assert!(matches!(s.add_page(other), Err(KernelError::Page(_))));
}

#[test]
fn erasing_a_plane_drops_it_from_the_page_and_undo_brings_it_back() {
    let mut s = Scene::new();
    let id = with_plane(&mut s);
    let mut custom = storing(id);
    custom.fill = false;
    let (p, _) = s.add_page(page(Some(custom))).unwrap();
    let change = s
        .edit(MeshId::Root, |m| Ok(m.erase_sections(&[id])))
        .unwrap();
    let stored = s.page(p).unwrap().sections.clone().unwrap();
    assert!(stored.active.is_empty(), "{stored:?}");
    assert!(!stored.fill, "the display stays");
    s.apply(&change.inverse());
    assert_eq!(
        s.page(p).unwrap().sections.as_ref().unwrap().active.len(),
        1
    );
}

#[test]
fn stored_planes_survive_a_data_round_trip_and_absent_ones_write_nothing() {
    let mut s = Scene::new();
    let id = with_plane(&mut s);
    let mut custom = storing(id);
    custom.cuts = false;
    custom.material_fill = true;
    s.add_page(page(Some(custom.clone()))).unwrap();
    let json = serde_json::to_string(&s.to_data()).unwrap();
    let back = Scene::from_data(&serde_json::from_str::<SceneData>(&json).unwrap()).unwrap();
    let (_, p) = back.pages().next().unwrap();
    assert_eq!(p.sections, Some(custom));

    let mut plain = Scene::new();
    plain.add_page(page(None)).unwrap();
    let json = serde_json::to_string(&plain.to_data()).unwrap();
    assert!(!json.contains("sections"), "{json}");
}
