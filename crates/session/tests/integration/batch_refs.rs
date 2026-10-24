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
                    ("path", "open_path" | "follow_me" | "copy_along_path")
                        | ("a" | "b", "add_dimension")
                        | ("edge", "add_edge_dimension" | "add_radial_dimension")
                        | ("anchor", "add_note")
                        | ("note", "change_note")
                        | ("plane", "flip_section_plane" | "activate_section_plane")
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

/// A group holding a group, in the root: returns the document and the outer group's id.
fn nested_groups() -> (Document, String) {
    let mut d = doc();
    let out = d
        .execute_batch(
            batch(vec![
                rectangle(),
                json!({"type": "make_group", "ids": ["$0:*"]}),
                json!({"type": "add_rectangle", "corner": [5, 0, 0], "opposite": [7, 1, 0]}),
                json!({"type": "make_group", "ids": ["$1:i0", "$2:*"]}),
            ]),
            true,
        )
        .unwrap();
    let outer = out[3].placed.as_ref().unwrap()[0].to_string();
    (d, outer)
}

fn instance_ids(d: &Document) -> Vec<String> {
    ask(d, json!({"type": "entities", "kinds": ["instance"]}))["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn copy_of_a_group_with_groups_refers_to_the_copy_not_nested_instances() {
    for atomic in [true, false] {
        let (mut d, outer) = nested_groups();
        let out = d
            .execute_batch(
                batch(vec![
                    json!({"type": "copy", "ids": [outer], "offset": [0, 10, 0]}),
                    json!({"type": "move", "ids": ["$0:i0"], "offset": [0, 0, 1]}),
                ]),
                atomic,
            )
            .unwrap();
        let instances = out[0]
            .changes
            .created
            .iter()
            .filter(|id| matches!(id, kernel::EntityId::Instance(_)))
            .count();
        assert!(instances > 1, "the copy deep-copies the inner group");
        let placed = out[0].placed.as_ref().unwrap();
        assert_eq!(placed.len(), 1, "atomic {atomic}");
        assert!(instance_ids(&d).contains(&placed[0].to_string()));
        assert_eq!(out[1].changes.modified, vec![placed[0]]);
    }
}

#[test]
fn rotate_copies_lists_placed_in_creation_order() {
    let (mut d, outer) = nested_groups();
    let out = d
        .execute(
            serde_json::from_value(json!({
                "type": "rotate", "ids": [outer], "center": [0, 0, 0], "angle": 30, "copies": 3
            }))
            .unwrap(),
        )
        .unwrap();
    let placed = out.placed.unwrap();
    assert_eq!(placed.len(), 3);
    assert!(placed.windows(2).all(|w| w[0] < w[1]));
    let all = ask(&d, json!({"type": "entities", "kinds": ["instance"]}));
    let all = all["instances"].as_array().unwrap();
    for (k, p) in placed.iter().enumerate() {
        let entry = all
            .iter()
            .find(|i| i["id"].as_str() == Some(&p.to_string()))
            .expect("placed instance is in the open context");
        let m: Vec<f64> = entry["transform"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect();
        let angle = 30f64.to_radians() * (k + 1) as f64;
        assert!((m[0] - angle.cos()).abs() < 1e-9, "copy {k}: {m:?}");
        assert!((m[1] - angle.sin()).abs() < 1e-9, "copy {k}: {m:?}");
    }
}

#[test]
fn placed_is_absent_for_commands_that_place_nothing() {
    let mut d = doc();
    let out = d
        .execute(serde_json::from_value(rectangle()).unwrap())
        .unwrap();
    assert!(out.placed.is_none());
    let v = serde_json::to_value(&out).unwrap();
    assert!(v.get("placed").is_none());
}

#[test]
fn star_skips_ids_a_later_command_erased() {
    let mut d = doc();
    let out = d
        .execute_batch(
            batch(vec![
                rectangle(),
                json!({"type": "erase", "ids": ["$0:f0"]}),
                json!({"type": "erase", "ids": ["$0:*"]}),
            ]),
            true,
        )
        .unwrap();
    assert_eq!(out.len(), 3);
    assert_eq!(stats(&d, "edges"), 0);
}

#[test]
fn star_after_an_erase_in_non_atomic_batches_too() {
    let mut d = doc();
    d.execute_batch(
        batch(vec![
            rectangle(),
            json!({"type": "erase", "ids": ["$0:f0"]}),
            json!({"type": "erase", "ids": ["$0:*"]}),
        ]),
        false,
    )
    .unwrap();
    assert_eq!(stats(&d, "edges"), 0);
}

#[test]
fn an_indexed_reference_to_an_erased_id_is_not_found() {
    for atomic in [true, false] {
        let mut d = doc();
        let err = d
            .execute_batch(
                batch(vec![
                    rectangle(),
                    json!({"type": "erase", "ids": ["$0:f0"]}),
                    json!({"type": "erase", "ids": ["$0:f0"]}),
                ]),
                atomic,
            )
            .unwrap_err();
        assert_eq!(err.index, 2);
        assert_eq!(err.error.kind, ErrorKind::NotFound);
        assert!(err.error.message.contains("$0:f0"), "{}", err.error.message);
    }
}

#[test]
fn a_star_that_resolves_to_nothing_is_an_empty_list() {
    let mut d = doc();
    let err = d
        .execute_batch(
            batch(vec![
                rectangle(),
                json!({"type": "erase", "ids": ["$0:f*"]}),
                json!({"type": "make_group", "ids": ["$0:f*"]}),
            ]),
            true,
        )
        .unwrap_err();
    assert_eq!(err.error.kind, ErrorKind::InvalidInput);
}

#[test]
fn open_path_takes_references_to_nested_groups() {
    for atomic in [true, false] {
        let mut d = doc();
        let out = d
            .execute_batch(
                batch(vec![
                    rectangle(),
                    json!({"type": "make_group", "ids": ["$0:*"]}),
                    json!({"type": "make_group", "ids": ["$1:i0"]}),
                    json!({"type": "open_path", "path": ["$2:i0", "$1:i0"]}),
                ]),
                atomic,
            )
            .unwrap();
        assert_eq!(out.len(), 4, "atomic {atomic}");
        assert_eq!(stats(&d, "faces"), 1, "the inner group is open");
    }
}

#[test]
fn open_path_references_work_after_an_open_instance() {
    let mut d = doc();
    d.execute_batch(
        batch(vec![
            rectangle(),
            json!({"type": "make_group", "ids": ["$0:*"]}),
            json!({"type": "make_group", "ids": ["$1:i0"]}),
            json!({"type": "open_instance", "id": "$2:i0"}),
            json!({"type": "open_path", "path": ["$2:i0", "$1:i0"]}),
        ]),
        false,
    )
    .unwrap();
    assert_eq!(stats(&d, "faces"), 1);
}
