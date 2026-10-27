use egui::accesskit::Action;

use super::access::accesskit_action;
use super::*;
use crate::navigation::TrackpadScroll;
use crate::ui::theme::UiScale;

/// A crashed autosave of house.tnn, and an agent's discard of an untitled model whose copy
/// cannot be used.
fn rows() -> Vec<RecoverRow> {
    vec![
        RecoverRow {
            model: PathBuf::from("/r/house-1-2.autosave.tnn"),
            name: "house".into(),
            original: Some(PathBuf::from("/m/house.tnn")),
            saved: "5 min ago".into(),
            discarded: false,
            error: None,
        },
        RecoverRow {
            model: PathBuf::from("/r/untitled-3-4.tnn"),
            name: "Untitled".into(),
            original: None,
            saved: "2 h ago".into(),
            discarded: true,
            error: Some("the model file cannot be used: it is missing".into()),
        },
    ]
}

fn recover_fixture(rows: Vec<RecoverRow>) -> Fixture {
    Fixture {
        recover: Some(rows),
        ..Default::default()
    }
}

#[test]
fn the_recover_window_lists_each_model_with_open_and_discard() {
    let mut h = harness(recover_fixture(rows()));
    assert!(h.query_by_label("Recover Models").is_some(), "the window");
    let texts = drawn_texts(&h);
    for text in [
        "house",
        "/m/house.tnn",
        "Autosaved 5 min ago",
        "Untitled",
        "Discarded by an agent 2 h ago",
        "the model file cannot be used: it is missing",
    ] {
        assert!(texts.iter().any(|t| t == text), "{text} in {texts:?}");
    }
    // A model that cannot be opened can only be discarded.
    assert_eq!(h.query_all_by_label("Open").count(), 1);
    assert_eq!(h.query_all_by_label("Discard").count(), 2);
    assert_eq!(
        click(&mut h, "Open"),
        [UiAction::OpenRecovered(PathBuf::from(
            "/r/house-1-2.autosave.tnn"
        ))]
    );
    h.get_all_by_label("Discard").nth(1).unwrap().click();
    h.run_ok();
    assert_eq!(
        h.state_mut().take(),
        [UiAction::DiscardRecovered(Some(PathBuf::from(
            "/r/untitled-3-4.tnn"
        )))]
    );
    assert_eq!(
        click(&mut h, "Discard All"),
        [UiAction::DiscardRecovered(None)]
    );
    assert_eq!(
        click(&mut h, "Close window"),
        [UiAction::ShowRecover(false)]
    );
}

#[test]
fn the_recover_window_says_how_many_older_entries_it_left_out() {
    let h = harness(Fixture {
        recover_older: 3,
        ..recover_fixture(rows())
    });
    assert!(
        drawn_texts(&h)
            .iter()
            .any(|t| t == "3 older entries not shown")
    );
    let h = harness(recover_fixture(rows()));
    assert!(!drawn_texts(&h).iter().any(|t| t.contains("older")));
    let h = harness(Fixture {
        recover_older: 1,
        ..recover_fixture(Vec::new())
    });
    assert!(
        drawn_texts(&h)
            .iter()
            .any(|t| t == "1 older entry not shown")
    );
}

#[test]
fn an_empty_recover_window_says_so() {
    let h = harness(recover_fixture(Vec::new()));
    assert!(drawn_texts(&h).iter().any(|t| t == "No models to recover"));
    assert!(h.query_by_label("Discard All").is_none());
}

#[test]
fn no_recover_window_while_it_is_closed() {
    let h = harness(Fixture::default());
    assert!(h.query_by_label("Discard All").is_none());
    assert!(h.query_by_label("Recover Models").is_none());
}

#[test]
fn discarding_asks_first() {
    let sheet = |names: &[&str]| Fixture {
        sheet: Some(Sheet::DiscardRecovered {
            models: names
                .iter()
                .map(|n| PathBuf::from(format!("/r/{n}.tnn")))
                .collect(),
            names: names.iter().map(|n| n.to_string()).collect(),
        }),
        ..Default::default()
    };
    let mut h = harness(sheet(&["house"]));
    let texts = drawn_texts(&h);
    assert!(
        texts
            .iter()
            .any(|t| t == "Discard the recovered model house?"),
        "{texts:?}"
    );
    assert!(texts.iter().any(|t| t.starts_with("Its unsaved changes")));
    assert_eq!(
        click(&mut h, "Discard"),
        [UiAction::ConfirmDiscardRecovered]
    );
    let mut h = harness(sheet(&["house", "Untitled"]));
    let texts = drawn_texts(&h);
    assert!(
        texts.iter().any(|t| t == "Discard all 2 recovered models?"),
        "{texts:?}"
    );
    assert!(texts.iter().any(|t| t.starts_with("Their unsaved changes")));
    assert_eq!(click(&mut h, "Cancel"), [UiAction::CloseSheet]);
}

/// The Settings window open on its Files tab, autosaving every `minutes`.
fn files_tab(minutes: u32) -> H {
    let mut h = harness(Fixture {
        settings: Some(SettingsView {
            ui_scale: UiScale::P100,
            trackpad: TrackpadScroll::Orbit,
            autosave_minutes: minutes,
            transition: kernel::SceneTransition::DEFAULT,
            units: session::Units::default(),
        }),
        ..Default::default()
    });
    assert_eq!(click(&mut h, "Files"), []);
    assert_eq!(is_on(&h, "Files"), Some(true));
    h
}

#[test]
fn the_files_tab_sets_the_autosave_interval() {
    let mut h = files_tab(5);
    assert!(drawn_texts(&h).iter().any(|t| t == "Autosave every"));
    assert!(drawn_texts(&h).iter().any(|t| t == "5 min"));
    assert_eq!(
        accesskit_action(&mut h, "Autosave every value", Action::Increment),
        [UiAction::SetAutosave(6)]
    );
    let mut h = files_tab(1);
    assert_eq!(
        accesskit_action(&mut h, "Autosave every value", Action::Decrement),
        [UiAction::SetAutosave(0)]
    );
    let h = files_tab(0);
    assert!(
        drawn_texts(&h).iter().any(|t| t == "Off"),
        "0 is shown as Off"
    );
}
