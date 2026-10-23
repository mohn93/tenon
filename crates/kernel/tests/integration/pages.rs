//! Pages (saved scenes): adding, changing, moving and deleting them, how deleting a tag
//! reaches them, and how they are kept in files.

use std::collections::BTreeSet;

use kernel::{KernelError, Page, PageId, PageView, Scene, SceneData, Tag, TagId};

fn view(x: f64) -> PageView {
    PageView {
        eye: [x, -10.0, 5.0],
        target: [0.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
        fov_y: 35f64.to_radians(),
        parallel: false,
        night: false,
        color_by_tag: false,
        x_ray: false,
        hidden_geometry: false,
        shadows: kernel::Shadows::default(),
        style: None,
    }
}

fn tag(s: &mut Scene, name: &str) -> TagId {
    s.add_tag(Tag {
        name: name.into(),
        color: [1, 2, 3],
        visible: true,
    })
    .unwrap()
    .0
}

fn page(name: &str, view: Option<PageView>, hidden: Option<&[TagId]>) -> Page {
    Page {
        name: name.into(),
        view,
        tags: hidden.map(|t| t.iter().copied().collect()),
    }
}

fn add(s: &mut Scene, p: Page) -> PageId {
    s.add_page(p).unwrap().0
}

fn names(s: &Scene) -> Vec<String> {
    s.pages().map(|(_, p)| p.name.clone()).collect()
}

#[test]
fn pages_are_added_in_order_with_ids_from_the_shared_counter() {
    let mut s = Scene::new();
    let t = tag(&mut s, "Walls");
    let (a, change) = s.add_page(page("A", Some(view(1.0)), Some(&[t]))).unwrap();
    assert_eq!(a, PageId(t.0 + 1));
    assert_eq!(a.to_string(), format!("p{}", t.0 + 1));
    assert_eq!(change.summary().scenes, vec![a]);
    let b = add(&mut s, page("B", None, None));
    assert_eq!(names(&s), ["A", "B"]);
    assert_eq!(s.page_index(b), Some(1));
    assert_eq!(s.page(a).unwrap().view, Some(view(1.0)));
    assert_eq!(s.page(a).unwrap().tags, Some(BTreeSet::from([t])));
    s.validate().unwrap();
    // Undo removes it again.
    s.apply(&change.inverse());
    assert!(s.page(a).is_none());
}

#[test]
fn default_names_take_the_lowest_free_number() {
    let mut s = Scene::new();
    assert_eq!(s.free_page_name(), "Scene 1");
    let one = add(&mut s, page("Scene 1", None, None));
    add(&mut s, page("Scene 2", None, None));
    assert_eq!(s.free_page_name(), "Scene 3");
    s.delete_page(one).unwrap();
    assert_eq!(s.free_page_name(), "Scene 1");
}

#[test]
fn page_names_must_be_valid_and_distinct() {
    let mut s = Scene::new();
    add(&mut s, page("Front", None, None));
    for bad in ["", "   ", "Front"] {
        let e = s.add_page(page(bad, None, None)).unwrap_err();
        assert!(matches!(e, KernelError::Page(_)), "{bad:?}: {e:?}");
    }
    let long = "x".repeat(257);
    assert!(matches!(
        s.add_page(page(&long, None, None)),
        Err(KernelError::Page(_))
    ));
    add(&mut s, page(&"x".repeat(256), None, None));
    // Case counts.
    add(&mut s, page("front", None, None));
    // Renaming to its own name is no change; to another's is refused.
    let back = add(&mut s, page("Back", None, None));
    assert!(
        s.change_page(back, page("Back", None, None))
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        s.change_page(back, page("Front", None, None)),
        Err(KernelError::Page(_))
    ));
    assert_eq!(
        s.change_page(PageId(999), page("X", None, None)),
        Err(KernelError::PageNotFound)
    );
}

#[test]
fn a_page_must_hide_existing_tags_and_have_a_usable_view() {
    let mut s = Scene::new();
    assert_eq!(
        s.add_page(page("A", None, Some(&[TagId(77)]))),
        Err(KernelError::TagNotFound)
    );
    let mut on_target = view(0.0);
    on_target.eye = on_target.target;
    let mut wide = view(1.0);
    wide.fov_y = 171f64.to_radians();
    let mut no_up = view(1.0);
    no_up.up = [0.0; 3];
    let mut nan = view(1.0);
    nan.eye[0] = f64::NAN;
    for v in [on_target, wide, no_up, nan] {
        assert!(matches!(
            s.add_page(page("A", Some(v), None)),
            Err(KernelError::Page(_))
        ));
    }
    assert_eq!(s.pages().count(), 0);
}

#[test]
fn moving_a_page_reorders_and_undo_moves_it_back() {
    let mut s = Scene::new();
    let a = add(&mut s, page("A", None, None));
    add(&mut s, page("B", None, None));
    add(&mut s, page("C", None, None));
    let change = s.move_page(a, 2).unwrap();
    assert_eq!(names(&s), ["B", "C", "A"]);
    assert_eq!(change.summary().scenes, vec![a]);
    s.apply(&change.inverse());
    assert_eq!(names(&s), ["A", "B", "C"]);
    assert!(s.move_page(a, 0).unwrap().is_empty());
    assert!(matches!(s.move_page(a, 3), Err(KernelError::Page(_))));
    assert_eq!(s.move_page(PageId(999), 0), Err(KernelError::PageNotFound));
}

#[test]
fn undoing_a_delete_puts_the_page_back_where_it_was() {
    let mut s = Scene::new();
    let t = tag(&mut s, "Walls");
    add(&mut s, page("A", None, None));
    let b = add(&mut s, page("B", Some(view(4.0)), Some(&[t])));
    add(&mut s, page("C", None, None));
    let before = s.clone();
    let change = s.delete_page(b).unwrap();
    assert_eq!(names(&s), ["A", "C"]);
    assert_eq!(change.summary().scenes, vec![b]);
    s.apply(&change.inverse());
    assert!(s == before);
    assert_eq!(s.page(b).unwrap().view, Some(view(4.0)));
    assert_eq!(s.delete_page(PageId(999)), Err(KernelError::PageNotFound));
}

#[test]
fn deleting_a_tag_drops_it_from_every_page_and_undo_restores_it() {
    let mut s = Scene::new();
    let walls = tag(&mut s, "Walls");
    let roof = tag(&mut s, "Roof");
    let a = add(&mut s, page("A", None, Some(&[walls, roof])));
    let b = add(&mut s, page("B", None, Some(&[roof])));
    let c = add(&mut s, page("C", Some(view(1.0)), None));
    let before = s.clone();
    let change = s.delete_tag(walls, None).unwrap();
    assert_eq!(s.page(a).unwrap().tags, Some(BTreeSet::from([roof])));
    assert_eq!(s.page(b).unwrap().tags, Some(BTreeSet::from([roof])));
    assert_eq!(s.page(c).unwrap().tags, None);
    s.validate().unwrap();
    let summary = change.summary();
    assert_eq!(summary.scenes, vec![a]);
    assert_eq!(summary.tags, vec![walls]);
    s.apply(&change.inverse());
    assert!(s == before);
    s.validate().unwrap();
}

#[test]
fn pages_are_kept_in_files() {
    let mut s = Scene::new();
    let walls = tag(&mut s, "Walls");
    let mut night = view(2.0);
    night.parallel = true;
    night.night = true;
    night.color_by_tag = true;
    add(&mut s, page("Z first", Some(night), Some(&[walls])));
    add(&mut s, page("A second", None, Some(&[])));
    add(&mut s, page("Camera only", Some(view(3.0)), None));
    let data = s.to_data();
    let json = serde_json::to_string(&data).unwrap();
    let read: SceneData = serde_json::from_str(&json).unwrap();
    let back = Scene::from_data(&read).unwrap();
    assert!(back == s);
    assert_eq!(names(&back), ["Z first", "A second", "Camera only"]);
    // A page storing tags with none hidden differs from one not storing tags.
    let pages: Vec<_> = back.pages().map(|(_, p)| p.tags.clone()).collect();
    assert_eq!(
        pages,
        [Some(BTreeSet::from([walls])), Some(BTreeSet::new()), None]
    );
    // New ids continue after the pages'.
    let mut back = back;
    let next = tag(&mut back, "New");
    assert!(back.pages().all(|(id, _)| id.0 < next.0));
}

#[test]
fn files_with_broken_pages_are_refused() {
    let mut s = Scene::new();
    let walls = tag(&mut s, "Walls");
    add(&mut s, page("A", Some(view(1.0)), Some(&[walls])));
    add(&mut s, page("B", None, None));
    let good = s.to_data();
    let refused = |f: &dyn Fn(&mut SceneData), why: &str| {
        let mut data = good.clone();
        f(&mut data);
        let e = Scene::from_data(&data).unwrap_err();
        assert!(e.contains(why), "{e} lacks {why:?}");
    };
    refused(&|d| d.pages[0].tags = Some(vec![999]), "missing tag");
    refused(&|d| d.pages[1].name = "A".into(), "named the same");
    refused(&|d| d.pages[1].name = " ".into(), "needs a name");
    refused(&|d| d.pages[1].id = d.pages[0].id, "appears twice");
    refused(&|d| d.pages[1].id = walls.0, "share an id");
    refused(
        &|d| d.pages[0].view.as_mut().unwrap().fov_y = 0.5f64.to_radians(),
        "field of view",
    );
    refused(&|d| d.pages[0].id = kernel::MAX_ID, "p");
    // A file without pages opens with none.
    let mut none = good.clone();
    none.pages.clear();
    let json = serde_json::to_value(&none).unwrap();
    assert!(json.get("pages").is_none());
    assert_eq!(Scene::from_data(&none).unwrap().pages().count(), 0);
}
