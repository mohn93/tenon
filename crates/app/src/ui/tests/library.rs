use session::LibraryKind;

use super::scenes::right_click;
use super::*;
use crate::ui::{LibrarySave, Thumb};
use kernel::InstanceId;

fn library_tab() -> DockLayout {
    DockLayout {
        tab: DockTab::Library,
        ..Default::default()
    }
}

fn row(name: &str, kind: LibraryKind, thumb: Thumb) -> LibraryRow {
    LibraryRow {
        path: PathBuf::from(format!("/lib/{name}.skr")),
        name: name.into(),
        kind,
        error: None,
        thumb,
    }
}

fn ready(name: &str, kind: LibraryKind) -> LibraryRow {
    row(name, kind, Thumb::Ready(egui::TextureId::default()))
}

/// A library of a chair, a table, a brick material and a house model, all with thumbnails.
fn library() -> Fixture {
    Fixture {
        dock: library_tab(),
        library_folder: Some("/lib".into()),
        library_rows: vec![
            ready("Chair", LibraryKind::Component),
            ready("Table", LibraryKind::Component),
            ready("Red Brick", LibraryKind::Material),
            ready("House", LibraryKind::Model),
        ],
        ..Default::default()
    }
}

fn path(name: &str) -> PathBuf {
    PathBuf::from(format!("/lib/{name}.skr"))
}

fn names_shown(h: &H) -> Vec<&'static str> {
    ["Chair", "Table", "Red Brick", "House"]
        .into_iter()
        .filter(|n| h.query_by_label(n).is_some())
        .collect()
}

#[test]
fn the_library_tab_follows_scenes_and_is_chosen_by_a_click() {
    let mut h = harness(Fixture::default());
    assert_eq!(click(&mut h, "Library"), [UiAction::SetDock(library_tab())]);
    let h = harness(library());
    assert_eq!(is_on(&h, "Library"), Some(true));
    assert!(rect_of(&h, "Scenes").max.x <= rect_of(&h, "Library").min.x);
}

#[test]
fn every_tab_stays_reachable_in_the_smallest_window() {
    let mut h = harness_sized(Fixture::default(), Vec2::new(960.0, 640.0));
    let tabs = ["Entity Info", "Materials", "Tags", "Scenes", "Library"];
    let rects: Vec<Rect> = tabs.iter().map(|t| rect_of(&h, t)).collect();
    let dock = h.state().central.max.x;
    for pair in rects.windows(2) {
        assert!(pair[0].max.x <= pair[1].min.x, "{rects:?}");
    }
    assert!(rects.iter().all(|r| r.min.x >= dock), "{rects:?} in {dock}");
    assert_eq!(click(&mut h, "Library"), [UiAction::SetDock(library_tab())]);
}

#[test]
fn with_no_folder_the_tab_asks_for_one() {
    let mut h = harness(Fixture {
        dock: library_tab(),
        ..Default::default()
    });
    assert!(
        drawn_texts(&h)
            .iter()
            .any(|t| t == "Choose a library folder")
    );
    assert!(h.query_by_label("Search library").is_none());
    assert_eq!(click(&mut h, "Choose..."), [UiAction::ChooseLibraryFolder]);
}

#[test]
fn the_tab_lists_the_items_and_says_when_there_are_none() {
    let h = harness(library());
    assert_eq!(names_shown(&h), ["Chair", "Table", "Red Brick", "House"]);
    let h = harness(Fixture {
        library_rows: Vec::new(),
        ..library()
    });
    assert!(drawn_texts(&h).iter().any(|t| t == "No items"));
    let h = harness(Fixture {
        library_rows: Vec::new(),
        library_error: Some("/lib: not a folder".into()),
        ..library()
    });
    assert!(drawn_texts(&h).iter().any(|t| t == "/lib: not a folder"));
    assert!(!drawn_texts(&h).iter().any(|t| t == "No items"));
    let h = harness(Fixture {
        library_more: true,
        ..library()
    });
    assert!(drawn_texts(&h).iter().any(|t| t.starts_with("More items")));
}

#[test]
fn search_keeps_the_names_that_contain_the_text_ignoring_case() {
    let mut h = harness(library());
    type_into(&mut h, "Search library", 0, "  BRI", egui::Key::Tab);
    assert_eq!(names_shown(&h), ["Red Brick"]);
    // The text stays while the tab is shown.
    h.run_ok();
    assert_eq!(names_shown(&h), ["Red Brick"]);
    let mut h = harness(library());
    type_into(&mut h, "Search library", 0, "zzz", egui::Key::Tab);
    assert_eq!(names_shown(&h), Vec::<&str>::new());
    assert!(drawn_texts(&h).iter().any(|t| t == "No matches"));
}

#[test]
fn the_kind_filter_shows_one_kind() {
    let mut h = harness(library());
    // Other panels use the same words: the entry is the node that appeared with the list.
    for (entry, shown) in [
        ("Components", vec!["Chair", "Table"]),
        ("Models", vec!["House"]),
        ("Materials", vec!["Red Brick"]),
        ("All", vec!["Chair", "Table", "Red Brick", "House"]),
    ] {
        let there = rects_named(&h, entry);
        assert_eq!(click(&mut h, "Kind"), []);
        new_node(&h, entry, &there).click();
        h.run_ok();
        assert_eq!(names_shown(&h), shown, "{entry}");
    }
    let there = rects_named(&h, "Components");
    click(&mut h, "Kind");
    new_node(&h, "Components", &there).click();
    h.run_ok();
    // The filter and the search together.
    type_into(&mut h, "Search library", 0, "zz", egui::Key::Tab);
    assert_eq!(names_shown(&h), Vec::<&str>::new());
}

#[test]
fn a_tile_click_uses_the_item() {
    let mut h = harness(library());
    assert_eq!(
        click(&mut h, "Chair"),
        [UiAction::LibraryUse(path("Chair"))]
    );
    assert_eq!(
        click(&mut h, "Red Brick"),
        [UiAction::LibraryUse(path("Red Brick"))]
    );
    assert_eq!(
        click(&mut h, "House"),
        [UiAction::LibraryUse(path("House"))]
    );
}

#[test]
fn the_context_menu_inserts_opens_and_deletes() {
    let mut h = harness(library());
    right_click(&mut h, "Table");
    assert_eq!(h.state_mut().take(), []);
    assert_eq!(
        click(&mut h, "Insert"),
        [UiAction::LibraryInsert(path("Table"))]
    );
    right_click(&mut h, "Table");
    assert_eq!(
        click(&mut h, "Open"),
        [UiAction::LibraryOpen(path("Table"))]
    );
    right_click(&mut h, "Chair");
    assert_eq!(
        click(&mut h, "Delete from Library"),
        [UiAction::LibraryDelete(path("Chair"))]
    );
}

#[test]
fn an_unusable_file_shows_an_icon_and_can_only_be_deleted() {
    let bad = LibraryRow {
        error: Some("not a model".into()),
        ..row("Broken", LibraryKind::Model, Thumb::Failed)
    };
    let mut h = harness(Fixture {
        library_rows: vec![bad],
        ..library()
    });
    assert_eq!(click(&mut h, "Broken"), []);
    right_click(&mut h, "Broken");
    assert!(h.get_by_label("Insert").accesskit_node().is_disabled());
    assert!(h.get_by_label("Open").accesskit_node().is_disabled());
    assert_eq!(
        click(&mut h, "Delete from Library"),
        [UiAction::LibraryDelete(path("Broken"))]
    );
}

#[test]
fn tiles_in_view_without_a_thumbnail_ask_for_one() {
    let f = Fixture {
        library_rows: vec![
            ready("Chair", LibraryKind::Component),
            row("Table", LibraryKind::Component, Thumb::Pending),
            row("Lamp", LibraryKind::Component, Thumb::Failed),
            row("Sofa", LibraryKind::Component, Thumb::Pending),
        ],
        reports_on_layout: true,
        ..library()
    };
    let mut h = harness(f);
    let actions = h.state_mut().take();
    let wanted = UiAction::ThumbnailsWanted(vec![path("Table"), path("Sofa")]);
    assert!(
        !actions.is_empty() && actions.iter().all(|a| *a == wanted),
        "{actions:?}"
    );
    // Nothing is asked for once they are made.
    let mut h = harness(library());
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
}

#[test]
fn tiles_scrolled_out_of_view_do_not_ask() {
    let rows: Vec<LibraryRow> = (0..60)
        .map(|n| {
            row(
                &format!("Part {n:02}"),
                LibraryKind::Component,
                Thumb::Pending,
            )
        })
        .collect();
    let h = harness(Fixture {
        library_rows: rows,
        reports_on_layout: true,
        ..library()
    });
    let actions = &h.state().actions;
    let [.., UiAction::ThumbnailsWanted(wanted)] = actions.as_slice() else {
        panic!("{actions:?}")
    };
    assert!(!wanted.is_empty() && wanted.len() < 40, "{}", wanted.len());
    assert_eq!(wanted[0], path("Part 00"));
}

#[test]
fn the_save_sheet_asks_for_a_name() {
    let fixture = || Fixture {
        sheet: Some(Sheet::SaveToLibrary {
            save: LibrarySave::Component(InstanceId(4)),
            suggested: "Chair".into(),
        }),
        ..Default::default()
    };
    let mut h = harness(fixture());
    assert!(h.query_by_label("Save Component to Library").is_some());
    h.event(egui::Event::Text("Armchair".into()));
    h.run_ok();
    assert_eq!(
        click(&mut h, "Save"),
        [UiAction::ConfirmSaveToLibrary("Armchair".into())]
    );
    let mut h = harness(fixture());
    h.event(egui::Event::Text("  ".into()));
    h.run_ok();
    assert!(h.get_by_label("Save").accesskit_node().is_disabled());
    assert_eq!(click(&mut h, "Cancel"), [UiAction::CloseSheet]);
    for (save, title) in [
        (
            LibrarySave::Material(MaterialId(2)),
            "Save Material to Library",
        ),
        (LibrarySave::Model, "Save Model to Library"),
    ] {
        let h = harness(Fixture {
            sheet: Some(Sheet::SaveToLibrary {
                save,
                suggested: "x".into(),
            }),
            ..Default::default()
        });
        assert!(h.query_by_label(title).is_some(), "{title}");
    }
}

#[test]
fn an_existing_name_asks_before_replacing() {
    let mut h = harness(Fixture {
        sheet: Some(Sheet::OverwriteLibrary {
            save: LibrarySave::Model,
            name: "House".into(),
            file: "House.skr".into(),
        }),
        ..Default::default()
    });
    assert!(
        drawn_texts(&h)
            .iter()
            .any(|t| t.contains("House.skr") && t.contains("Replace"))
    );
    assert_eq!(
        click(&mut h, "Replace"),
        [UiAction::ConfirmOverwriteLibrary]
    );
}

#[test]
fn deleting_from_the_library_asks_first() {
    let mut h = harness(Fixture {
        sheet: Some(Sheet::DeleteLibraryItem {
            path: path("Chair"),
            name: "Chair".into(),
        }),
        ..Default::default()
    });
    assert!(
        drawn_texts(&h)
            .iter()
            .any(|t| t == "Delete Chair from the library?")
    );
    assert_eq!(click(&mut h, "Cancel"), [UiAction::CloseSheet]);
    let mut h = harness(Fixture {
        sheet: Some(Sheet::DeleteLibraryItem {
            path: path("Chair"),
            name: "Chair".into(),
        }),
        ..Default::default()
    });
    assert_eq!(click(&mut h, "Delete"), [UiAction::ConfirmLibraryDelete]);
}

#[test]
fn entity_info_saves_a_selected_component() {
    let f = |folder: Option<&str>, component| Fixture {
        info: vec!["1 component".into()],
        library_folder: folder.map(PathBuf::from),
        component,
        ..Default::default()
    };
    let mut h = harness(f(Some("/lib"), Some(InstanceId(7))));
    assert_eq!(
        click(&mut h, "Save to Library"),
        [UiAction::SaveToLibrary(LibrarySave::Component(InstanceId(
            7
        )))]
    );
    // Without a folder the button is there but off; without a component it is not there.
    let h = harness(f(None, Some(InstanceId(7))));
    assert!(
        h.get_by_label("Save to Library")
            .accesskit_node()
            .is_disabled()
    );
    let h = harness(f(Some("/lib"), None));
    assert!(h.query_by_label("Save to Library").is_none());
}

#[test]
fn the_materials_panel_saves_the_chosen_material() {
    let f = |folder: Option<&str>| Fixture {
        materials: vec![brick()],
        material: Some(MaterialId(31)),
        library_folder: folder.map(PathBuf::from),
        dock: super::layout::materials_tab(),
        ..Default::default()
    };
    let mut h = harness(f(Some("/lib")));
    assert_eq!(
        click(&mut h, "Save to Library"),
        [UiAction::SaveToLibrary(LibrarySave::Material(MaterialId(
            31
        )))]
    );
    let h = harness(f(None));
    assert!(
        h.get_by_label("Save to Library")
            .accesskit_node()
            .is_disabled()
    );
    // No chosen material, nothing to save.
    let h = harness(Fixture {
        material: None,
        ..f(Some("/lib"))
    });
    assert!(h.query_by_label("Save to Library").is_none());
}

#[test]
fn the_file_menu_saves_the_model_to_the_library() {
    let mut h = harness(Fixture {
        library_folder: Some("/lib".into()),
        ..Default::default()
    });
    assert_eq!(
        click_menu(&mut h, &["File", "Save Model to Library..."]),
        [UiAction::SaveToLibrary(LibrarySave::Model)]
    );
    let bar = crate::menus::menu_bar(&Fixture::default().view(), false);
    let item = crate::menus::items(&bar)
        .into_iter()
        .find(|i| i.label == "Save Model to Library...")
        .expect("the item");
    assert!(!item.enabled, "no folder, no item");
}

#[test]
fn settings_chooses_and_clears_the_library_folder() {
    let mut f = super::settings::settings_fixture();
    f.library_folder = Some("/lib/parts".into());
    let mut h = harness(f);
    click(&mut h, "Files");
    assert!(drawn_texts(&h).iter().any(|t| t == "Library folder"));
    assert!(drawn_texts(&h).iter().any(|t| t == "/lib/parts"));
    assert_eq!(click(&mut h, "Choose..."), [UiAction::ChooseLibraryFolder]);
    assert_eq!(click(&mut h, "Clear"), [UiAction::ClearLibraryFolder]);
    let mut h = harness(super::settings::settings_fixture());
    click(&mut h, "Files");
    assert!(drawn_texts(&h).iter().any(|t| t == "Not set"));
    assert!(h.get_by_label("Clear").accesskit_node().is_disabled());
}

#[test]
fn the_context_bar_saves_a_selected_component() {
    let f = |folder: Option<&str>, component| Fixture {
        library_folder: folder.map(PathBuf::from),
        component,
        ..Default::default()
    };
    let mut h = harness(f(Some("/lib"), Some(InstanceId(7))));
    assert_eq!(
        click(&mut h, "Save Component to Library"),
        [UiAction::SaveToLibrary(LibrarySave::Component(InstanceId(
            7
        )))]
    );
    let h = harness(f(None, Some(InstanceId(7))));
    assert!(
        h.get_by_label("Save Component to Library")
            .accesskit_node()
            .is_disabled()
    );
    let h = harness(f(Some("/lib"), None));
    assert!(h.query_by_label("Save Component to Library").is_none());
}

#[test]
fn a_material_tile_says_a_click_adds_it_and_paints_with_it() {
    let mut h = harness(library());
    h.get_by_label("Red Brick").hover();
    for _ in 0..30 {
        h.step();
    }
    let texts = drawn_texts(&h);
    assert!(
        texts
            .iter()
            .any(|t| t.contains("click to add to the model and paint with it")),
        "{texts:?}"
    );
}
