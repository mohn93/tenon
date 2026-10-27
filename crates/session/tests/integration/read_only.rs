//! A read-only document (the phone viewer's): commands that change the model are refused.

use glam::DVec2;
use session::{Command, Document, ErrorKind, SelectMode};

fn doc() -> Document {
    let mut d = Document::new(DVec2::new(800.0, 600.0));
    d.execute(Command::AddEdge {
        start: [0., 0., 0.],
        end: [1., 0., 0.],
    })
    .unwrap();
    d
}

fn edge() -> Command {
    Command::AddEdge {
        start: [0., 0., 0.],
        end: [0., 1., 0.],
    }
}

#[test]
fn editing_commands_are_refused() {
    let mut d = doc();
    let revision = d.revision();
    d.set_read_only(true);
    assert!(d.is_read_only());
    for command in [
        edge(),
        Command::Undo {
            count: None,
            expect: None,
        },
        Command::Redo { count: None },
        Command::New { discard: true },
        Command::Save {
            path: None,
            overwrite: true,
        },
    ] {
        let error = d.execute(command).unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidInput);
    }
    assert_eq!(d.revision(), revision);
}

#[test]
fn batches_are_refused_whole() {
    let mut d = doc();
    d.set_read_only(true);
    for atomic in [true, false] {
        let error = d
            .execute_batch(
                vec![
                    Command::Select {
                        ids: vec![],
                        mode: SelectMode::Replace,
                        all: true,
                    },
                    edge(),
                ],
                atomic,
            )
            .unwrap_err();
        assert_eq!(error.index, 1);
        assert_eq!(error.error.kind, ErrorKind::InvalidInput);
    }
}

#[test]
fn camera_and_selection_still_work_and_bytes_still_open() {
    let mut d = doc();
    let bytes = d.encode_bytes().unwrap();
    d.set_read_only(true);
    d.execute(Command::Orbit {
        yaw: 10.0,
        pitch: 5.0,
        about: None,
    })
    .unwrap();
    d.execute(Command::ZoomExtents {}).unwrap();
    d.execute(Command::Select {
        ids: vec![],
        mode: SelectMode::Replace,
        all: true,
    })
    .unwrap();
    d.open_bytes("m.tnn", &bytes, true).unwrap();
    assert!(d.is_read_only());
}
