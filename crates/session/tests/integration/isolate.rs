//! Isolating parts for viewing: hidden from every view, never part of the model.

use glam::DVec2;
use kernel::{InstanceId, MeshId};
use serde_json::{Value, json};
use session::{Command, Document};

fn run(d: &mut Document, v: Value) -> Value {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    serde_json::to_value(d.execute(cmd).unwrap()).unwrap()
}

/// Two 2 x 2 squares as separate groups, the second 10 m along x from the first.
fn two_groups() -> (Document, InstanceId, InstanceId) {
    let mut d = Document::new(DVec2::new(800.0, 600.0));
    let mut ids = Vec::new();
    for x in [0.0, 10.0] {
        let made = run(
            &mut d,
            json!({"type": "add_rectangle", "corner": [x, 0, 0], "opposite": [x + 2.0, 2, 0]}),
        );
        let new: Vec<Value> = made["changes"]["created"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|id| id.as_str().is_some_and(|s| s.starts_with(['e', 'f'])))
            .cloned()
            .collect();
        run(&mut d, json!({"type": "make_group", "ids": new}));
        ids.push(
            d.scene()
                .root()
                .instances()
                .map(|(id, _)| id)
                .max()
                .unwrap(),
        );
    }
    assert_eq!(d.scene().root().instances().count(), 2);
    (d, ids[0], ids[1])
}

#[test]
fn isolating_hides_the_others_and_changes_nothing_in_the_model() {
    let (mut d, a, b) = two_groups();
    assert_ne!(a, b);
    let saved = d.encode_bytes().unwrap();
    let modified = d.is_modified();
    let undo = d.undo_label().map(str::to_string);
    let all = d.scene().shown_bounds(MeshId::Root, &[]).unwrap();
    assert!(all[1].x > 11.9);

    let revision = d.revision();
    assert!(d.isolate(&[a]));
    assert!(d.isolated());
    assert!(d.revision() > revision, "what is derived is rebuilt");
    let only = d.scene().shown_bounds(MeshId::Root, &[]).unwrap();
    assert!(only[1].x < 2.1 && only[0].x > -0.1, "{only:?}");
    assert!(d.scene().view_hidden().contains(&b));
    assert!(!d.scene().view_hidden().contains(&a));

    // The model is the same: same bytes, no undo step, not modified.
    assert_eq!(d.encode_bytes().unwrap(), saved);
    assert_eq!(d.is_modified(), modified);
    assert_eq!(d.undo_label().map(str::to_string), undo);

    // The same isolation again changes nothing; another part replaces it.
    let revision = d.revision();
    assert!(!d.isolate(&[a]));
    assert_eq!(d.revision(), revision);
    assert!(d.isolate(&[b]));
    assert!(d.scene().view_hidden().contains(&a));

    assert!(d.show_all());
    assert!(!d.isolated());
    assert_eq!(d.scene().shown_bounds(MeshId::Root, &[]).unwrap(), all);
    assert!(!d.show_all(), "nothing was hidden");
}

#[test]
fn keeping_several_parts_and_unknown_ids() {
    let (mut d, a, b) = two_groups();
    let ghost = InstanceId(9999);
    let revision = d.revision();
    assert!(!d.isolate(&[]));
    assert!(!d.isolate(&[ghost]));
    assert_eq!(d.revision(), revision);
    assert!(!d.isolate(&[a, b]));
    assert!(!d.isolated(), "keeping every part hides none");
    assert!(d.isolate(&[a, ghost]));
    assert_eq!(d.scene().view_hidden().len(), 1);
}

#[test]
fn a_read_only_document_can_isolate_and_a_new_model_shows_everything() {
    let (mut d, a, _) = two_groups();
    d.set_read_only(true);
    assert!(d.isolate(&[a]));
    let bytes = d.encode_bytes().unwrap();
    d.open_bytes("again", &bytes, true).unwrap();
    assert!(
        !d.isolated(),
        "opening a model starts with everything shown"
    );
}
