//! Clip text (copy) and the `paste` command.

use std::io::{Read, Write};

use base64::Engine;
use glam::{DVec2, DVec3};
use kernel::EntityId;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query, clip};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, ErrorKind> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| e.kind)
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn paste(d: &mut Document, text: &str, at: Option<[f64; 3]>) -> Result<Value, ErrorKind> {
    let mut command = json!({"type": "paste", "clip": text});
    if let Some(at) = at {
        command["at"] = json!(at);
    }
    run(d, command)
}

fn created(out: &Value) -> Vec<String> {
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

fn instance_of(out: &Value) -> String {
    created(out)
        .into_iter()
        .find(|id| id.starts_with('i'))
        .expect("an instance was created")
}

/// Every edge and face id in the open context.
fn everything(d: &Document) -> Vec<EntityId> {
    let m = d.model();
    m.edges()
        .map(|(id, _)| EntityId::Edge(id))
        .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
        .collect()
}

fn rectangle(d: &mut Document, corner: [f64; 3], opposite: [f64; 3]) -> Vec<EntityId> {
    let before: Vec<EntityId> = everything(d);
    run(
        d,
        json!({"type": "add_rectangle", "corner": corner, "opposite": opposite}),
    )
    .unwrap();
    everything(d)
        .into_iter()
        .filter(|id| !before.contains(id))
        .collect()
}

/// The open context's vertex positions mapped to world coordinates, rounded and sorted.
fn world_points(d: &Document) -> Vec<[i64; 3]> {
    let t = d.context_transform();
    let mut points: Vec<[i64; 3]> = d
        .model()
        .vertices()
        .map(|(_, v)| rounded(t.transform_point3(v.pos)))
        .collect();
    points.sort();
    points
}

fn rounded(p: DVec3) -> [i64; 3] {
    (p * 1e6).round().to_array().map(|x| x as i64)
}

fn stats(d: &Document) -> Value {
    ask(d, json!({"type": "stats"}))
}

fn definitions(d: &Document) -> usize {
    d.scene().definitions().count()
}

fn header_body(text: &str) -> Vec<u8> {
    let body = text.strip_prefix("sketchup-alt clip 1\n").expect("header");
    base64::engine::general_purpose::STANDARD
        .decode(body)
        .unwrap()
}

fn entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut e = zip.by_index(i).unwrap();
            let mut b = Vec::new();
            e.read_to_end(&mut b).unwrap();
            (e.name().unwrap().to_string(), b)
        })
        .collect()
}

fn rezip(entries: &[(String, Vec<u8>)]) -> String {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip.start_file(name.as_str(), zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    text_of(&bytes)
}

fn text_of(bytes: &[u8]) -> String {
    format!(
        "sketchup-alt clip 1\n{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

#[test]
fn clip_text_is_a_header_and_an_archive_with_the_model_and_the_base_point() {
    let mut d = doc();
    let ids = rectangle(&mut d, [2.0, 3.0, 0.0], [4.0, 5.0, 0.0]);
    let text = d.clip(&ids).unwrap();
    assert!(text.starts_with("sketchup-alt clip 1\n"));
    let entries = entries(&header_body(&text));
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["model.json", "clip.json"]);
    let model: Value = serde_json::from_slice(&entries[0].1).unwrap();
    assert_eq!(model["format"], "sketchup-alt");
    assert_eq!(model["model"]["root"]["edges"].as_array().unwrap().len(), 4);
    let info: Value = serde_json::from_slice(&entries[1].1).unwrap();
    assert_eq!(info["base"], json!([2.0, 3.0, 0.0]));
    assert_eq!(info["source"], d.token());
    // Copying changes nothing.
    assert_eq!(d.revision(), 1);
}

#[test]
fn clip_refuses_an_empty_list_vertices_and_unknown_ids() {
    let mut d = doc();
    rectangle(&mut d, [0.0; 3], [1.0, 1.0, 0.0]);
    assert_eq!(d.clip(&[]).unwrap_err().kind, ErrorKind::InvalidInput);
    let v = d.model().vertices().next().unwrap().0;
    assert_eq!(
        d.clip(&[EntityId::Vertex(v)]).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    let f99: EntityId = serde_json::from_value(json!("f99")).unwrap();
    assert_eq!(d.clip(&[f99]).unwrap_err().kind, ErrorKind::NotFound);
}

#[test]
fn paste_puts_the_base_point_at_the_point_selects_it_and_undoes_in_one_step() {
    let mut d = doc();
    let ids = rectangle(&mut d, [0.0; 3], [1.0, 1.0, 0.0]);
    let text = d.clip(&ids).unwrap();
    let out = paste(&mut d, &text, Some([5.0, 0.0, 2.0])).unwrap();
    let made = created(&out);
    assert_eq!(made.iter().filter(|i| i.starts_with('e')).count(), 4);
    assert_eq!(made.iter().filter(|i| i.starts_with('f')).count(), 1);
    let selected: Vec<String> = d.selection().iter().map(|i| i.to_string()).collect();
    let mut expected: Vec<String> = made
        .iter()
        .filter(|i| i.starts_with('e') || i.starts_with('f'))
        .cloned()
        .collect();
    expected.sort();
    let mut sorted = selected.clone();
    sorted.sort();
    assert_eq!(sorted, expected);
    assert!(world_points(&d).contains(&rounded(DVec3::new(5.0, 0.0, 2.0))));
    assert!(world_points(&d).contains(&rounded(DVec3::new(6.0, 1.0, 2.0))));
    assert_eq!(d.undo_label(), Some("paste"));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(stats(&d)["edges"], 4);
    assert_eq!(stats(&d)["faces"], 1);
}

#[test]
fn paste_in_place_onto_a_new_model_lands_where_it_was_copied() {
    let mut d = doc();
    let ids = rectangle(&mut d, [1.0, 2.0, 3.0], [2.0, 4.0, 3.0]);
    let before = world_points(&d);
    let text = d.clip(&ids).unwrap();
    let mut other = doc();
    paste(&mut other, &text, None).unwrap();
    assert_eq!(world_points(&other), before);
    // Pasting onto the original merges with it: nothing new.
    let edges = stats(&d)["edges"].clone();
    paste(&mut d, &text, None).unwrap();
    assert_eq!(stats(&d)["edges"], edges);
}

#[test]
fn paste_goes_into_an_open_group() {
    let mut d = doc();
    let first = rectangle(&mut d, [0.0; 3], [1.0, 1.0, 0.0]);
    let group = instance_of(&run(&mut d, json!({"type": "make_group", "ids": first})).unwrap());
    let other = rectangle(&mut d, [3.0, 0.0, 0.0], [4.0, 1.0, 0.0]);
    let text = d.clip(&other).unwrap();
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    paste(&mut d, &text, Some([0.0, 2.0, 0.0])).unwrap();
    assert_eq!(stats(&d)["edges"], 8);
    assert_eq!(stats(&d)["faces"], 2);
    assert_eq!(d.selection().len(), 5);
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    // The root keeps the group and the copied rectangle only.
    assert_eq!(d.model().edges().count(), 4);
}

/// A group turned 30 degrees, stretched along x and moved, holding a 1 x 1 square.
fn turned_group(d: &mut Document) -> String {
    let ids = rectangle(d, [0.0; 3], [1.0, 1.0, 0.0]);
    let group = instance_of(&run(d, json!({"type": "make_group", "ids": ids})).unwrap());
    run(
        d,
        json!({"type": "rotate", "ids": [group], "center": [0, 0, 0], "angle": 30}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "scale", "ids": [group], "origin": [0, 0, 0], "factors": [2, 1, 1]}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "move", "ids": [group], "offset": [3, -2, 1]}),
    )
    .unwrap();
    group
}

#[test]
fn copy_inside_a_turned_group_and_paste_in_place_at_the_root_keeps_world_positions() {
    let mut d = doc();
    let group = turned_group(&mut d);
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    let inside = world_points(&d);
    let text = d.clip(&everything(&d)).unwrap();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    assert_eq!(d.model().vertices().count(), 0);
    paste(&mut d, &text, None).unwrap();
    assert_eq!(world_points(&d), inside);
}

#[test]
fn copy_at_the_root_and_paste_in_place_inside_a_turned_group_keeps_world_positions() {
    let mut d = doc();
    let group = turned_group(&mut d);
    let ids = rectangle(&mut d, [5.0, 5.0, 0.0], [6.0, 7.0, 0.0]);
    let original = world_points(&d);
    let text = d.clip(&ids).unwrap();
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    let before = world_points(&d);
    paste(&mut d, &text, None).unwrap();
    let mut expected = [before, original].concat();
    expected.sort();
    assert_eq!(world_points(&d), expected);
}

/// A component "Box" (a red square on tag "Walls") inside a component "Outer", at the root.
fn nested_component(d: &mut Document) -> String {
    let ids = rectangle(d, [0.0; 3], [1.0, 1.0, 0.0]);
    run(
        d,
        json!({"type": "add_material", "name": "Red", "color": [200, 0, 0]}),
    )
    .unwrap();
    run(d, json!({"type": "add_tag", "name": "Walls"})).unwrap();
    let red = d.scene().materials().next().unwrap().0.to_string();
    let walls = d.scene().tags().next().unwrap().0.to_string();
    let face: Vec<EntityId> = ids
        .iter()
        .copied()
        .filter(|i| matches!(i, EntityId::Face(_)))
        .collect();
    run(d, json!({"type": "paint", "ids": face, "material": red})).unwrap();
    run(d, json!({"type": "set_tag", "ids": ids, "tag": walls})).unwrap();
    let inner = instance_of(
        &run(
            d,
            json!({"type": "make_component", "ids": ids, "name": "Box"}),
        )
        .unwrap(),
    );
    instance_of(
        &run(
            d,
            json!({"type": "make_component", "ids": [inner], "name": "Outer"}),
        )
        .unwrap(),
    )
}

#[test]
fn components_pasted_into_the_same_document_share_their_definitions() {
    let mut d = doc();
    let outer = nested_component(&mut d);
    let id: EntityId = serde_json::from_value(json!(outer)).unwrap();
    let text = d.clip(&[id]).unwrap();
    assert_eq!(definitions(&d), 2);
    let first = paste(&mut d, &text, Some([5.0, 0.0, 0.0])).unwrap();
    let second = paste(&mut d, &text, Some([10.0, 0.0, 0.0])).unwrap();
    assert_eq!(definitions(&d), 2);
    assert_eq!(d.scene().materials().count(), 1);
    assert_eq!(d.scene().tags().count(), 1);
    let definition = |i: &str| {
        let id: EntityId = serde_json::from_value(json!(i)).unwrap();
        let EntityId::Instance(i) = id else { panic!() };
        d.model().instance(i).unwrap().definition
    };
    let original = definition(&outer);
    assert_eq!(definition(&instance_of(&first)), original);
    assert_eq!(definition(&instance_of(&second)), original);
    let selected: Vec<EntityId> = d.selection().iter().copied().collect();
    assert_eq!(
        selected,
        [serde_json::from_value::<EntityId>(json!(instance_of(&second))).unwrap()]
    );
}

#[test]
fn components_pasted_into_a_new_model_get_new_definitions_materials_and_tags() {
    let mut d = doc();
    let outer = nested_component(&mut d);
    let id: EntityId = serde_json::from_value(json!(outer)).unwrap();
    let text = d.clip(&[id]).unwrap();
    let token = d.token().to_string();
    run(&mut d, json!({"type": "new", "discard": true})).unwrap();
    assert_ne!(d.token(), token);
    paste(&mut d, &text, None).unwrap();
    assert_eq!(definitions(&d), 2);
    let names: Vec<&str> = d
        .scene()
        .definitions()
        .map(|(_, x)| x.name.as_str())
        .collect();
    assert!(
        names.contains(&"Box") && names.contains(&"Outer"),
        "{names:?}"
    );
    let materials: Vec<&str> = d
        .scene()
        .materials()
        .map(|(_, m)| m.name.as_str())
        .collect();
    assert_eq!(materials, ["Red"]);
    let tags: Vec<&str> = d.scene().tags().map(|(_, t)| t.name.as_str()).collect();
    assert_eq!(tags, ["Walls"]);
    assert_eq!(d.selection().len(), 1);
    // Pasting again in the new model places the definitions the first paste made.
    let placed = |d: &Document| {
        let [EntityId::Instance(i)] = d.selection().iter().copied().collect::<Vec<_>>()[..] else {
            panic!("one instance is selected")
        };
        d.model().instance(i).unwrap().definition
    };
    let first = placed(&d);
    paste(&mut d, &text, Some([3.0, 0.0, 0.0])).unwrap();
    assert_eq!(definitions(&d), 2);
    assert_eq!(d.scene().materials().count(), 1);
    assert_eq!(placed(&d), first);
    // Undo takes back everything a paste added.
    run(&mut d, json!({"type": "undo", "count": 2})).unwrap();
    assert_eq!(definitions(&d), 0);
    assert_eq!(d.scene().materials().count(), 0);
    assert_eq!(d.scene().tags().count(), 0);
    assert_eq!(d.model().instances().count(), 0);
    // The definitions the undone paste made are gone, so pasting makes them anew.
    paste(&mut d, &text, None).unwrap();
    assert_eq!(definitions(&d), 2);
    // A new model forgets what was pasted into the last one.
    run(&mut d, json!({"type": "new", "discard": true})).unwrap();
    paste(&mut d, &text, None).unwrap();
    assert_eq!(definitions(&d), 2);
}

#[test]
fn pasting_into_the_source_after_its_definitions_were_purged_shares_the_new_ones() {
    let mut d = doc();
    let outer = nested_component(&mut d);
    let id: EntityId = serde_json::from_value(json!(outer)).unwrap();
    let text = d.clip(&[id]).unwrap();
    run(&mut d, json!({"type": "erase", "ids": [outer]})).unwrap();
    run(&mut d, json!({"type": "purge_definitions"})).unwrap();
    assert_eq!(definitions(&d), 0);
    paste(&mut d, &text, Some([5.0, 0.0, 0.0])).unwrap();
    assert_eq!(definitions(&d), 2);
    paste(&mut d, &text, Some([10.0, 0.0, 0.0])).unwrap();
    assert_eq!(definitions(&d), 2);
}

#[test]
fn opening_a_file_gives_the_document_a_new_token_and_saving_keeps_it() {
    let dir = TempDir::new("clip-token");
    let path = dir.0.join("m.skr");
    let mut d = doc();
    let outer = nested_component(&mut d);
    let id: EntityId = serde_json::from_value(json!(outer)).unwrap();
    let text = d.clip(&[id]).unwrap();
    let token = d.token().to_string();
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(d.token(), token);
    run(
        &mut d,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_ne!(d.token(), token);
    // The reopened model is another document: its components are copied anew.
    paste(&mut d, &text, Some([4.0, 0.0, 0.0])).unwrap();
    assert_eq!(definitions(&d), 4);
}

/// A new folder under the system's temporary folder, removed when dropped (a failing test
/// included).
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("{name}-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn every_new_document_has_its_own_token() {
    let (a, b) = (doc(), doc());
    assert_ne!(a.token(), b.token());
    assert!(!a.token().is_empty());
}

#[test]
fn the_outline_is_the_base_point_and_every_edge_in_world_coordinates() {
    let mut d = doc();
    let ids = rectangle(&mut d, [2.0, 3.0, 0.0], [4.0, 5.0, 0.0]);
    let text = d.clip(&ids).unwrap();
    let outline = clip::outline(&text).unwrap();
    assert_eq!(outline.base, DVec3::new(2.0, 3.0, 0.0));
    assert_eq!(outline.segments.len(), 4);

    let mut d = doc();
    let group = turned_group(&mut d);
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    let inside = world_points(&d);
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    let id: EntityId = serde_json::from_value(json!(group)).unwrap();
    let outline = clip::outline(&d.clip(&[id]).unwrap()).unwrap();
    assert_eq!(outline.segments.len(), 4);
    let mut points: Vec<[i64; 3]> = outline
        .segments
        .iter()
        .flatten()
        .map(|p| rounded(*p))
        .collect();
    points.sort();
    points.dedup();
    assert_eq!(points, inside);
    let lowest = inside.iter().fold([i64::MAX; 3], |m, p| {
        [m[0].min(p[0]), m[1].min(p[1]), m[2].min(p[2])]
    });
    assert_eq!(rounded(outline.base), lowest);
    assert!(clip::outline("hello").is_err());
}

#[test]
fn guides_are_copied_and_pasted() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_guide_point", "position": [1, 2, 3]}),
    )
    .unwrap();
    let g = d.model().guides().next().unwrap().0;
    let text = d.clip(&[EntityId::Guide(g)]).unwrap();
    let out = paste(&mut d, &text, Some([0.0, 0.0, 0.0])).unwrap();
    assert_eq!(d.model().guides().count(), 2);
    assert!(created(&out).iter().any(|i| i.starts_with('g')));
    assert_eq!(d.selection().len(), 1);
}

/// A model with a square, a clip of it, and what it looked like.
fn target() -> (Document, String) {
    let mut d = doc();
    let ids = rectangle(&mut d, [0.0; 3], [1.0, 1.0, 0.0]);
    let text = d.clip(&ids).unwrap();
    (d, text)
}

fn refuses(d: &mut Document, text: &str, why: &str) {
    let before = (d.revision(), stats(d), d.selection().clone());
    assert_eq!(
        paste(d, text, Some([3.0, 0.0, 0.0])),
        Err(ErrorKind::InvalidInput),
        "{why}"
    );
    assert_eq!(
        (d.revision(), stats(d), d.selection().clone()),
        before,
        "{why}"
    );
    assert!(clip::outline(text).is_err(), "{why}");
}

#[test]
fn text_that_is_not_a_good_clip_is_refused_and_changes_nothing() {
    let (mut d, good) = target();
    let bytes = header_body(&good);
    let parts = entries(&bytes);
    let model = parts[0].1.clone();
    let info = parts[1].1.clone();

    refuses(&mut d, "", "empty");
    refuses(&mut d, "hello", "not ours");
    refuses(&mut d, "sketchup-alt clip 2\nAAAA", "another version");
    refuses(&mut d, "sketchup-alt clip 1", "header only");
    refuses(&mut d, "sketchup-alt clip 1\n", "no body");
    refuses(&mut d, &good[..good.len() / 2], "truncated");
    refuses(&mut d, &good[..good.len() - 1], "one character short");
    refuses(&mut d, "sketchup-alt clip 1\n@@@@!!!!", "bad base64");
    refuses(&mut d, &text_of(b"hello, world"), "not a zip");
    let mut damaged = bytes.clone();
    let middle = damaged.len() / 2;
    damaged[middle..middle + 16].fill(0xff);
    refuses(&mut d, &text_of(&damaged), "damaged zip");
    refuses(
        &mut d,
        &rezip(&[("clip.json".into(), info.clone())]),
        "no model.json",
    );
    refuses(
        &mut d,
        &rezip(&[("model.json".into(), model.clone())]),
        "no clip.json",
    );
    refuses(
        &mut d,
        &rezip(&[
            ("document.json".into(), model.clone()),
            ("clip.json".into(), info.clone()),
        ]),
        "a .skr archive",
    );
    refuses(
        &mut d,
        &rezip(&[
            ("model.json".into(), b"{".to_vec()),
            ("clip.json".into(), info.clone()),
        ]),
        "model.json not JSON",
    );
    let mut far: Value = serde_json::from_slice(&model).unwrap();
    far["model"]["root"]["vertices"][0]["position"][0] = json!(2.0e6);
    refuses(
        &mut d,
        &rezip(&[
            ("model.json".into(), serde_json::to_vec(&far).unwrap()),
            ("clip.json".into(), info.clone()),
        ]),
        "a vertex beyond the coordinate limit",
    );
    let mut newer: Value = serde_json::from_slice(&model).unwrap();
    newer["schema_version"] = json!(99);
    refuses(
        &mut d,
        &rezip(&[
            ("model.json".into(), serde_json::to_vec(&newer).unwrap()),
            ("clip.json".into(), info.clone()),
        ]),
        "a newer schema",
    );
    for bad_info in [
        json!({"base": [2.0e6, 0, 0], "source": "x"}),
        json!({"base": [0, 0], "source": "x"}),
        json!({"base": [0, 0, 0]}),
        json!({"base": [0, 0, 0], "source": "x", "extra": 1}),
        json!({"base": [0, 0, 0], "source": "x".repeat(1000)}),
    ] {
        refuses(
            &mut d,
            &rezip(&[
                ("model.json".into(), model.clone()),
                ("clip.json".into(), serde_json::to_vec(&bad_info).unwrap()),
            ]),
            &format!("clip.json {bad_info}"),
        );
    }
    // Over the size cap: refused before decoding.
    let huge = format!(
        "sketchup-alt clip 1\n{}",
        "A".repeat((clip::MAX_CLIP_BYTES / 3 + 2) * 4)
    );
    refuses(&mut d, &huge, "too large");
    // A placement beyond the coordinate limit.
    assert_eq!(
        paste(&mut d, &good, Some([1.0e6, 0.0, 0.0])),
        Err(ErrorKind::InvalidInput)
    );
    let nan = Command::Paste {
        clip: good.clone(),
        at: Some([f64::NAN, 0.0, 0.0]),
    };
    assert_eq!(d.execute(nan).unwrap_err().kind, ErrorKind::InvalidInput);
    // The good clip still pastes, with a clipboard's line breaks.
    let wrapped = good.replace('\n', "\r\n") + "\r\n";
    paste(&mut d, &wrapped, Some([3.0, 0.0, 0.0])).unwrap();
    assert_eq!(stats(&d)["edges"], 8);
}

#[test]
fn a_paste_in_an_atomic_batch_rolls_back_with_it() {
    let (mut d, text) = target();
    let before = stats(&d);
    let commands: Vec<Command> = vec![
        serde_json::from_value(json!({"type": "paste", "clip": text, "at": [3, 0, 0]})).unwrap(),
        serde_json::from_value(json!({"type": "erase", "ids": ["f999"]})).unwrap(),
    ];
    assert!(d.execute_batch(commands, true).is_err());
    assert_eq!(stats(&d)["edges"], before["edges"]);
    assert!(d.selection().is_empty());
}

/// A clip of a component holding `squares` squares, with `levels` definitions stacked on it,
/// each placing the one below `fan` times, and the root placing the top one.
fn nested_clip(squares: u64, levels: u64, fan: u64) -> String {
    let mut d = doc();
    let square = rectangle(&mut d, [0.0; 3], [1.0, 1.0, 0.0]);
    if squares > 1 {
        let count = u32::try_from(squares - 1).unwrap();
        run(
            &mut d,
            json!({"type": "copy", "ids": square, "offset": [2, 0, 0], "count": count}),
        )
        .unwrap();
    }
    let ids = everything(&d);
    let i = instance_of(
        &run(
            &mut d,
            json!({"type": "make_component", "ids": ids, "name": "Leaf"}),
        )
        .unwrap(),
    );
    let id: EntityId = serde_json::from_value(json!(i)).unwrap();
    let parts = entries(&header_body(&d.clip(&[id]).unwrap()));
    let mut model: Value = serde_json::from_slice(&parts[0].1).unwrap();
    let leaf = model["model"]["definitions"][0].clone();
    let kind = leaf["kind"].clone();
    let mut below = leaf["id"].as_u64().unwrap();
    let mut next = 10_000u64;
    let place = |definition: u64, k: u64, next: &mut u64| {
        *next += 1;
        json!({"id": *next, "definition": definition,
               "transform": [1, 0, 0, 0, 1, 0, 0, 0, 1, k as f64 * 2.0, 0, 0]})
    };
    for level in 0..levels {
        let instances: Vec<Value> = (0..fan).map(|k| place(below, k, &mut next)).collect();
        next += 1;
        let id = next;
        model["model"]["definitions"].as_array_mut().unwrap().push(
            json!({"id": id, "name": format!("Level {level}"), "kind": kind,
                "mesh": {"vertices": [], "edges": [], "faces": [], "instances": instances}}),
        );
        below = id;
    }
    model["model"]["root"]["instances"] = json!([place(below, 0, &mut next)]);
    model["model"]["next_id"] = json!(next + 1);
    rezip(&[
        ("model.json".into(), serde_json::to_vec(&model).unwrap()),
        parts[1].clone(),
    ])
}

#[test]
fn a_clip_placing_too_many_nested_instances_is_refused_quickly() {
    let (mut d, _) = target();
    // 10 levels of 10: about 1.1e10 placements, refused before anything walks them.
    let start = std::time::Instant::now();
    refuses(&mut d, &nested_clip(1, 10, 10), "10 levels of 10");
    // 5 levels of 10 places 111,111 instances: just over the cap.
    refuses(&mut d, &nested_clip(1, 5, 10), "5 levels of 10");
    assert!(start.elapsed().as_secs() < 10, "{:?}", start.elapsed());
    // 3 levels of 10 (1,111) pastes, and its outline is the 1,000 squares.
    let small = nested_clip(1, 3, 10);
    assert_eq!(clip::outline(&small).unwrap().segments.len(), 4000);
    paste(&mut d, &small, Some([5.0, 0.0, 0.0])).unwrap();
    assert_eq!(definitions(&d), 4);
}

#[test]
fn a_clip_nested_too_deep_is_refused_quickly() {
    let (mut d, _) = target();
    // A chain of 20,000 definitions, each placing the next once.
    let deep = nested_clip(1, 19_999, 1);
    let start = std::time::Instant::now();
    refuses(&mut d, &deep, "20,000 levels");
    assert!(start.elapsed().as_secs() < 10, "{:?}", start.elapsed());
    // 33 levels (the leaf and 32 above it) are refused; 32 paste.
    let depth = kernel::MAX_IMPORT_DEPTH as u64;
    refuses(&mut d, &nested_clip(1, depth, 1), "33 levels");
    paste(&mut d, &nested_clip(1, depth - 1, 1), Some([5.0, 0.0, 0.0])).unwrap();
    assert_eq!(definitions(&d), kernel::MAX_IMPORT_DEPTH);
}

#[test]
fn a_clip_showing_too_many_faces_is_refused() {
    let (mut d, _) = target();
    // 4 levels of 17 place 88,741 instances (under the instance cap), showing the leaf's 250
    // squares 83,521 times: about 20.9 million faces.
    refuses(
        &mut d,
        &nested_clip(250, 4, 17),
        "over the placed faces cap",
    );
    // With 230 squares, about 19.2 million faces: under the cap (the outline is capped too).
    assert!(clip::outline(&nested_clip(230, 4, 17)).is_ok());
}
