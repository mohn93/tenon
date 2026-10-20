//! Batch commands that refer to ids earlier commands of the batch made ("$0:f0").

use glam::DVec2;
use serde_json::{Value, json};
use session::{BatchCommand, Command, Document, ErrorKind, Query, check_references};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn batch(values: Vec<Value>) -> Vec<BatchCommand> {
    values
        .into_iter()
        .enumerate()
        .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
        .collect()
}

fn stats(d: &Document, key: &str) -> u64 {
    ask(d, json!({"type": "stats"}))[key].as_u64().unwrap()
}

fn rectangle() -> Value {
    json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 1, 0]})
}

#[test]
fn a_rectangle_then_push_pull_of_its_face_makes_a_box() {
    for atomic in [true, false] {
        let mut d = doc();
        let commands = batch(vec![
            rectangle(),
            json!({"type": "push_pull", "face": "$0:f0", "distance": 1}),
        ]);
        let outcomes = d.execute_batch(commands, atomic).unwrap();
        assert_eq!(outcomes.len(), 2);
        assert_eq!(stats(&d, "faces"), 6, "atomic {atomic}");
        if atomic {
            assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
        }
    }
}

#[test]
fn star_expands_in_place_in_id_lists() {
    let mut d = doc();
    let commands = batch(vec![
        rectangle(),
        json!({"type": "erase", "ids": ["$0:e*"]}),
    ]);
    d.execute_batch(commands, true).unwrap();
    assert_eq!(stats(&d, "edges"), 0);
    assert_eq!(stats(&d, "faces"), 0);
}

#[test]
fn group_everything_a_command_made_then_paint_it() {
    let mut d = doc();
    let commands = batch(vec![
        rectangle(),
        json!({"type": "push_pull", "face": "$0:f0", "distance": 1}),
        json!({"type": "make_group", "ids": ["$0:*", "$1:*"]}),
        json!({"type": "add_material", "color": [200, 0, 0]}),
        json!({"type": "paint", "ids": ["$2:i0"], "material": "$3:m0"}),
    ]);
    let outcomes = d.execute_batch(commands, true).unwrap();
    let all = ask(&d, json!({"type": "entities"}));
    let instances = all["instances"].as_array().unwrap();
    assert_eq!(instances.len(), 1, "{all}");
    assert_eq!(stats(&d, "faces"), 0);
    let id = instances[0]["id"].clone();
    let material = serde_json::to_value(outcomes[3].changes.materials[0]).unwrap();
    let entity = ask(&d, json!({"type": "entity", "id": id}));
    assert_eq!(entity["material"], material, "{entity}");
}

#[test]
fn a_reference_past_what_was_made_fails_that_command_and_rolls_back() {
    let mut d = doc();
    d.execute(Command::AddEdge {
        start: [5.0, 5.0, 0.0],
        end: [6.0, 5.0, 0.0],
    })
    .unwrap();
    let edge = ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"][0]["id"].clone();
    d.execute(serde_json::from_value(json!({"type": "select", "ids": [edge]})).unwrap())
        .unwrap();
    d.execute(Command::Orbit {
        yaw: 30.0,
        pitch: 10.0,
        about: None,
    })
    .unwrap();
    let camera = ask(&d, json!({"type": "camera"}));
    let selection = ask(&d, json!({"type": "selection"}));
    let context = ask(&d, json!({"type": "context"}));
    let commands = batch(vec![
        rectangle(),
        json!({"type": "push_pull", "face": "$0:f1", "distance": 1}),
    ]);
    let e = d.execute_batch(commands, true).unwrap_err();
    assert_eq!(e.index, 1);
    assert!(e.rolled_back);
    assert_eq!(e.error.kind, ErrorKind::InvalidInput);
    assert_eq!(e.error.message, "$0:f1: command 0 made 1 face");
    assert_eq!(stats(&d, "faces"), 0);
    assert_eq!(stats(&d, "edges"), 1);
    assert_eq!(ask(&d, json!({"type": "camera"})), camera);
    assert_eq!(ask(&d, json!({"type": "selection"})), selection);
    assert_eq!(ask(&d, json!({"type": "context"})), context);

    let commands = batch(vec![
        rectangle(),
        json!({"type": "push_pull", "face": "$0:f1", "distance": 1}),
    ]);
    let e = d.execute_batch(commands, false).unwrap_err();
    assert_eq!(e.index, 1);
    assert!(!e.rolled_back);
    assert_eq!(e.completed.len(), 1);
    assert_eq!(e.error.message, "$0:f1: command 0 made 1 face");
    assert_eq!(stats(&d, "faces"), 1);
}

#[test]
fn parse_rejects_bad_references() {
    let cases = [
        (
            json!({"type": "push_pull", "face": "$1:f0", "distance": 1}),
            1,
            vec!["$1:f0", "earlier"],
        ),
        (
            json!({"type": "push_pull", "face": "$0:x0", "distance": 1}),
            1,
            vec!["invalid reference"],
        ),
        (
            json!({"type": "push_pull", "face": "$00:f0", "distance": 1}),
            1,
            vec!["invalid reference"],
        ),
        (
            json!({"type": "push_pull", "face": "$0:f*", "distance": 1}),
            1,
            vec!["list"],
        ),
        (
            json!({"type": "delete_material", "material": "$0:f0"}),
            1,
            vec!["material"],
        ),
        (
            json!({"type": "erase", "ids": ["$0:m0"]}),
            1,
            vec!["material"],
        ),
        (
            json!({"type": "erase", "ids": ["$0:f0", "f4611686018427387904"]}),
            1,
            vec!["cannot be used with references"],
        ),
        (
            json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0], "id": "$0:f0"}),
            1,
            vec!["unknown field `id`"],
        ),
    ];
    for (value, index, texts) in cases {
        let e = BatchCommand::parse(value.clone(), index).unwrap_err();
        for text in texts {
            assert!(e.contains(text), "{value}: {e}");
        }
    }
}

#[test]
fn reference_like_text_outside_id_fields_stays_text() {
    let mut d = doc();
    let commands = batch(vec![
        rectangle(),
        json!({"type": "make_component", "ids": ["$0:*"], "name": "$0:f0"}),
    ]);
    d.execute_batch(commands, true).unwrap();
    let definitions = ask(&d, json!({"type": "definitions"}));
    assert_eq!(
        definitions["definitions"][0]["name"], "$0:f0",
        "{definitions}"
    );

    let open = BatchCommand::parse(json!({"type": "open", "path": "$0:f0"}), 1).unwrap();
    assert!(!open.has_references());
    assert!(matches!(open.command(), Command::Open { path, .. } if path == "$0:f0"));
}

#[test]
fn open_path_takes_references() {
    let mut d = doc();
    let commands = batch(vec![
        rectangle(),
        json!({"type": "make_group", "ids": ["$0:*"]}),
        json!({"type": "open_path", "path": ["$1:i0"]}),
    ]);
    let outcomes = d.execute_batch(commands, false).unwrap();
    let instance = outcomes[1]
        .changes
        .created
        .iter()
        .find(|id| id.to_string().starts_with('i'))
        .unwrap()
        .to_string();
    assert_eq!(
        ask(&d, json!({"type": "context"}))["path"],
        json!([instance])
    );
}

#[test]
fn reference_keys_cover_every_id_field() {
    let schema = serde_json::to_value(schemars::schema_for!(Command)).unwrap();
    let variants = schema["oneOf"].as_array().expect("one schema per command");
    let mut checked = 0;
    let mut tag_fields = 0;
    let mut scene_fields = 0;
    for variant in variants {
        let name = variant["properties"]["type"]["const"]
            .as_str()
            .unwrap_or_else(|| panic!("{variant}"));
        let Some(properties) = variant["properties"].as_object() else {
            continue;
        };
        for (key, property) in properties {
            let text = property.to_string();
            // An annotation's anchor may be a vertex id.
            let has_ids = text.contains("#/$defs/EntityId")
                || text.contains("#/$defs/MaterialId")
                || text.contains("#/$defs/AnchorInput");
            let reference_key = matches!(key.as_str(), "ids" | "face" | "id" | "material")
                || matches!(
                    (key.as_str(), name),
                    ("path", "open_path" | "follow_me")
                        | ("a" | "b", "add_dimension")
                        | ("edge", "add_edge_dimension" | "add_radial_dimension")
                        | ("anchor", "add_note")
                        | ("note", "change_note")
                );
            // Tag ids are not references (a reference has no `t` kind), so a field holding
            // them must not be a reference key: "$0:t0" there is refused as a tag id.
            // Scene ids likewise.
            if !has_ids && text.contains("#/$defs/PageId") {
                scene_fields += 1;
                assert!(
                    !reference_key,
                    "{name}.{key} holds scene ids but is a reference key"
                );
                continue;
            }
            if !has_ids && text.contains("#/$defs/TagId") {
                tag_fields += 1;
                assert!(
                    !reference_key,
                    "{name}.{key} holds tag ids but is a reference key"
                );
                continue;
            }
            if !has_ids {
                continue;
            }
            checked += 1;
            assert!(
                reference_key,
                "{name}.{key} holds ids but is not a reference key"
            );
        }
    }
    assert!(checked > 10, "found only {checked} id fields");
    assert_eq!(
        tag_fields, 4,
        "change_tag.tag, delete_tag.tag and .move_to, set_tag.tag"
    );
    assert_eq!(
        scene_fields, 5,
        "update_scene, change_scene, move_scene, delete_scene and go_to_scene .scene"
    );
}

#[test]
fn a_reference_across_new_or_open_is_refused() {
    for replace in [
        json!({"type": "new", "discard": true}),
        json!({"type": "open", "path": "/no/such.skr", "discard": true}),
    ] {
        let commands = batch(vec![
            rectangle(),
            replace,
            json!({"type": "add_rectangle", "corner": [5, 5, 0], "opposite": [7, 7, 0]}),
            json!({"type": "push_pull", "face": "$0:f0", "distance": 1}),
        ]);
        let (index, message) = check_references(&commands).unwrap_err();
        assert_eq!(index, 3);
        assert!(message.contains("replaced the document"), "{message}");
    }
    let commands = batch(vec![
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
        json!({"type": "new", "discard": true}),
        rectangle(),
        json!({"type": "push_pull", "face": "$2:f0", "distance": 1}),
    ]);
    assert_eq!(check_references(&commands), Ok(()));
}

#[test]
fn a_command_without_references_comes_back_as_written() {
    let value = json!({"type": "erase", "ids": ["f4611686018427387904"]});
    let command = BatchCommand::parse(value.clone(), 0).unwrap();
    assert!(!command.has_references());
    let written: Command = serde_json::from_value(value).unwrap();
    assert_eq!(command.command(), &written);
    let e = doc().execute_batch(vec![command], false).unwrap_err();
    assert_eq!(e.error.kind, ErrorKind::NotFound);
}
