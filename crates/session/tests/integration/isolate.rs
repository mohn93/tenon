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
    d.set_read_only(true);
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
    d.set_read_only(true);
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

#[test]
fn isolating_a_freshly_opened_document_leaves_it_unmodified() {
    let (d, a, _) = two_groups();
    let bytes = d.encode_bytes().unwrap();
    let mut d = Document::new(DVec2::new(800.0, 600.0));
    d.open_bytes("model", &bytes, true).unwrap();
    assert!(!d.is_modified());
    let a = d
        .scene()
        .root()
        .instances()
        .map(|(id, _)| id)
        .next()
        .unwrap_or(a);
    d.set_read_only(true);
    assert!(d.isolate(&[a]));
    assert!(!d.is_modified(), "isolate is not an edit");
    assert!(d.show_all());
    assert!(!d.is_modified(), "nor is Show all");
    // A document that was modified stays modified.
    d.set_read_only(false);
    run(
        &mut d,
        json!({"type": "add_edge", "start": [0, 0, 5], "end": [1, 0, 5]}),
    );
    assert!(d.is_modified());
    d.set_read_only(true);
    d.isolate(&[a]);
    d.show_all();
    assert!(d.is_modified());
}

#[test]
fn only_a_read_only_document_can_isolate() {
    let (mut d, a, _) = two_groups();
    let revision = d.revision();
    assert!(!d.isolate(&[a]), "an editor never hides parts");
    assert!(!d.isolated());
    assert_eq!(d.revision(), revision);
}

/// The root's notes, by text.
fn note_ids(d: &Document) -> std::collections::BTreeMap<String, kernel::NoteId> {
    d.scene()
        .root()
        .notes()
        .filter_map(|(id, n)| match n {
            kernel::Note::Leader { text: Some(t), .. } => Some((t.clone(), id)),
            _ => None,
        })
        .collect()
}

#[test]
fn isolating_hides_the_notes_pointing_at_hidden_parts() {
    let (mut d, a, b) = two_groups();
    run(
        &mut d,
        json!({"type": "add_label", "target": format!("i{}", b.0), "text": "B label"}),
    );
    // A point just off the second square, nearer it than anything else.
    run(
        &mut d,
        json!({"type": "add_note", "anchor": [11.0, 1.0, 0.2], "position": [11, 3, 1], "text": "B near"}),
    );
    run(
        &mut d,
        json!({"type": "add_note", "anchor": [1.0, 1.0, 0.0], "position": [1, 3, 1], "text": "A"}),
    );
    // Root geometry of its own, nearer than either group.
    run(
        &mut d,
        json!({"type": "add_edge", "start": [5, 0, 0], "end": [5, 2, 0]}),
    );
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [5, 5, 0], "opposite": [7, 7, 0]}),
    );
    run(
        &mut d,
        json!({"type": "add_note", "anchor": [6.0, 6.0, 0.1], "position": [6, 8, 1], "text": "Root"}),
    );
    let notes = note_ids(&d);
    assert_eq!(notes.len(), 4, "{notes:?}");
    d.set_read_only(true);

    assert!(d.isolate(&[a]));
    let hidden = d.scene().view_hidden_notes().clone();
    assert_eq!(
        hidden,
        std::collections::BTreeSet::from([notes["B label"], notes["B near"]])
    );
    let root = d.scene().root();
    for (text, id) in &notes {
        let note = root.note(*id).unwrap();
        assert_eq!(
            d.scene().note_shown(*id, note),
            !hidden.contains(id),
            "{text}"
        );
    }
    // Keeping the other part hides the other notes instead.
    assert!(d.isolate(&[b]));
    assert_eq!(
        *d.scene().view_hidden_notes(),
        std::collections::BTreeSet::from([notes["A"]])
    );
    // Show all brings every note back, and nothing of it is saved.
    assert!(d.show_all());
    assert!(d.scene().view_hidden_notes().is_empty());
    assert!(d.isolate(&[a]));
    let bytes = d.encode_bytes().unwrap();
    d.open_bytes("again", &bytes, true).unwrap();
    assert!(d.scene().view_hidden_notes().is_empty());
}
