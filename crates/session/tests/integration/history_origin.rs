//! Who made each undo step, and `undo` with `expect`.

use glam::DVec2;
use session::{Command, Document, ErrorKind, ExpectedStep, Origin, Query, QueryResult};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn edge(a: [f64; 3], b: [f64; 3]) -> Command {
    Command::AddEdge { start: a, end: b }
}

fn revision(d: &Document) -> u64 {
    match d.query(&Query::Stats {}).unwrap() {
        QueryResult::Stats { revision, .. } => revision,
        other => panic!("{other:?}"),
    }
}

#[test]
fn steps_carry_the_origin_they_were_made_with() {
    let mut doc = doc();
    doc.execute(edge([0., 0., 0.], [1., 0., 0.])).unwrap(); // user (default)
    doc.set_origin(Origin::Agent);
    doc.execute(edge([0., 0., 0.], [0., 1., 0.])).unwrap();
    let QueryResult::History {
        undo, undo_origins, ..
    } = doc.query(&Query::History {}).unwrap()
    else {
        panic!()
    };
    assert_eq!(undo, ["add_edge", "add_edge"]);
    assert_eq!(undo_origins, [Origin::Agent, Origin::User]);
    doc.execute(Command::Undo {
        count: None,
        expect: None,
    })
    .unwrap();
    let QueryResult::History { redo_origins, .. } = doc.query(&Query::History {}).unwrap() else {
        panic!()
    };
    assert_eq!(redo_origins, [Origin::Agent]);
}

#[test]
fn undo_refuses_an_unexpected_step() {
    // a user step on top; the agent expects its own add_edge
    let mut doc = doc();
    doc.set_origin(Origin::Agent);
    doc.execute(edge([0., 0., 0.], [1., 0., 0.])).unwrap();
    doc.set_origin(Origin::User);
    doc.execute(edge([0., 0., 0.], [0., 1., 0.])).unwrap();
    let undo = |label: &str, origin| Command::Undo {
        count: None,
        expect: Some(ExpectedStep {
            label: label.into(),
            origin,
        }),
    };
    let e = doc
        .execute(undo("add_edge", Some(Origin::Agent)))
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::UnexpectedStep);
    assert!(e.message.contains("add_edge") && e.message.contains("user"));
    let e = doc.execute(undo("push_pull", None)).unwrap_err();
    assert_eq!(e.kind, ErrorKind::UnexpectedStep);
    assert_eq!(revision(&doc), 2); // nothing undone
    doc.execute(undo("add_edge", Some(Origin::User))).unwrap();
    assert_eq!(revision(&doc), 3);
}

#[test]
fn undo_with_expect_and_nothing_to_undo_fails_as_such() {
    let mut doc = doc();
    let e = doc
        .execute(Command::Undo {
            count: None,
            expect: Some(ExpectedStep {
                label: "add_edge".into(),
                origin: None,
            }),
        })
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NothingToUndo);
}

#[test]
fn expect_parses_from_json() {
    let c: Command = serde_json::from_value(serde_json::json!(
        {"type": "undo", "expect": {"label": "push_pull", "origin": "agent"}}
    ))
    .unwrap();
    assert_eq!(
        c,
        Command::Undo {
            count: None,
            expect: Some(ExpectedStep {
                label: "push_pull".into(),
                origin: Some(Origin::Agent)
            })
        }
    );
}

#[test]
fn expect_refuses_an_unknown_key() {
    // a misspelled origin must not silently skip the origin check
    let r = serde_json::from_value::<Command>(serde_json::json!(
        {"type": "undo", "expect": {"label": "add_edge", "orgin": "agent"}}
    ));
    let e = r.unwrap_err().to_string();
    assert!(e.contains("orgin"), "{e}");
}

#[test]
fn new_and_open_keep_the_current_origin() {
    let mut doc = doc();
    doc.set_origin(Origin::Agent);
    doc.execute(Command::New { discard: true }).unwrap();
    assert_eq!(doc.origin(), Origin::Agent);
}

fn undo_depth(d: &Document) -> usize {
    match d.query(&Query::History {}).unwrap() {
        QueryResult::History { undo, .. } => undo.len(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn joining_and_dropping_steps_takes_only_the_expected_ones() {
    let user_edge = ExpectedStep {
        label: "add_edge".into(),
        origin: Some(Origin::User),
    };
    let mut doc = doc();
    doc.set_origin(Origin::Agent);
    doc.execute(edge([0., 0., 0.], [1., 0., 0.])).unwrap();
    doc.set_origin(Origin::User);
    doc.execute(edge([0., 0., 0.], [0., 1., 0.])).unwrap();
    doc.execute(edge([0., 0., 0.], [0., 0., 1.])).unwrap();
    // The agent's step is not the user's: nothing changes.
    assert!(!doc.join_last_steps(3, &user_edge));
    assert!(!doc.drop_last_steps(3, &user_edge));
    assert!(!doc.join_last_steps(0, &user_edge));
    assert_eq!(undo_depth(&doc), 3);
    let before = revision(&doc);
    assert!(doc.join_last_steps(2, &user_edge));
    assert_eq!(undo_depth(&doc), 2);
    assert_eq!(revision(&doc), before, "the model did not change");
    doc.execute(Command::Undo {
        count: None,
        expect: None,
    })
    .unwrap();
    assert_eq!(
        doc.scene().root().edges().count(),
        1,
        "one undo, both edges"
    );
    doc.execute(Command::Redo { count: None }).unwrap();
    assert_eq!(doc.scene().root().edges().count(), 3);
    assert!(doc.drop_last_steps(1, &user_edge));
    assert_eq!(doc.scene().root().edges().count(), 1);
    assert_eq!(undo_depth(&doc), 1);
    assert_eq!(doc.redo_label(), None, "a dropped step cannot be redone");
}
