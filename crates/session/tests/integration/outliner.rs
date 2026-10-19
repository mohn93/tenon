//! The outliner query (the tree of placed groups and components) and `open_path`, driven
//! through JSON commands.

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, Vec<String>)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.ids.iter().map(|i| i.to_string()).collect()))
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn outliner(d: &Document) -> Value {
    ask(d, json!({"type": "outliner"}))
}

fn path(d: &Document) -> Value {
    ask(d, json!({"type": "context"}))["path"].clone()
}

fn selection(d: &Document) -> Value {
    ask(d, json!({"type": "selection"}))["ids"].clone()
}

/// The instance id among a command's created ids.
fn created_instance(out: &Value) -> String {
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .find(|id| id.starts_with('i'))
        .expect("an instance was created")
}

/// Every edge and face id in the open context.
fn everything(d: &Document) -> Vec<Value> {
    let all = ask(d, json!({"type": "entities", "kinds": ["edge", "face"]}));
    all["edges"]
        .as_array()
        .unwrap()
        .iter()
        .chain(all["faces"].as_array().unwrap())
        .map(|e| e["id"].clone())
        .collect()
}

/// Two "Box" components side by side, each placing the same group (a 2 x 2 x 1 box); the
/// second one painted. Returns (first component, second component, inner group, material).
fn boxes(d: &mut Document) -> (String, String, String, String) {
    run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "push_pull", "face": "f8", "distance": 1.0}),
    )
    .unwrap();
    let ids = everything(d);
    let group = created_instance(&run(d, json!({"type": "make_group", "ids": ids})).unwrap());
    let first = created_instance(
        &run(
            d,
            json!({"type": "make_component", "ids": [group], "name": "Box"}),
        )
        .unwrap(),
    );
    let second = created_instance(
        &run(
            d,
            json!({"type": "copy", "ids": [first], "offset": [5, 0, 0]}),
        )
        .unwrap(),
    );
    let out = run(d, json!({"type": "add_material", "color": [200, 0, 0]})).unwrap();
    let red = out["changes"]["materials"][0].as_str().unwrap().to_string();
    run(
        d,
        json!({"type": "paint", "ids": [second], "material": red}),
    )
    .unwrap();
    run(d, json!({"type": "select", "ids": []})).unwrap();
    (first, second, group, red)
}

#[test]
fn an_empty_model_has_an_empty_tree() {
    let d = doc();
    assert_eq!(
        outliner(&d),
        json!({"type": "outliner", "instances": [], "revision": 0})
    );
}

#[test]
fn the_tree_lists_every_placed_instance_with_what_it_places() {
    let mut d = doc();
    let (first, second, group, red) = boxes(&mut d);
    let tree = outliner(&d);
    let top = tree["instances"].as_array().unwrap();
    assert_eq!(top.len(), 2);
    assert_eq!(top[0]["id"], json!(first));
    assert_eq!(top[1]["id"], json!(second), "in id order");
    for node in top {
        assert_eq!(node["kind"], "component");
        assert_eq!(node["name"], "Box");
        assert_eq!(node["definition"], top[0]["definition"], "one definition");
        let children = node["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        // A component's contents show under each of its instances, with the same ids.
        assert_eq!(children[0]["id"], json!(group));
        assert_eq!(children[0]["kind"], "group");
        assert_eq!(children[0]["name"], "", "groups are unnamed");
        assert_eq!(children[0]["children"], json!([]));
        assert!(node.get("open").is_none() && node.get("selected").is_none());
    }
    assert!(top[0].get("material").is_none());
    assert_eq!(top[1]["material"], json!(red));
    assert_eq!(
        tree["revision"],
        ask(&d, json!({"type": "stats"}))["revision"]
    );
}

#[test]
fn the_tree_marks_the_open_path_and_the_selection() {
    let mut d = doc();
    let (first, second, group, _) = boxes(&mut d);
    run(&mut d, json!({"type": "select", "ids": [second]})).unwrap();
    let tree = outliner(&d);
    assert!(tree["instances"][0].get("selected").is_none());
    assert_eq!(tree["instances"][1]["selected"], true);

    run(&mut d, json!({"type": "open_instance", "id": first})).unwrap();
    run(&mut d, json!({"type": "select", "ids": [group]})).unwrap();
    let tree = outliner(&d);
    let [a, b] = [&tree["instances"][0], &tree["instances"][1]];
    assert_eq!(a["open"], true);
    assert!(a.get("selected").is_none());
    assert!(b.get("open").is_none() && b.get("selected").is_none());
    // The same group under the other placement is neither open nor selected there.
    assert_eq!(a["children"][0]["selected"], true);
    assert!(a["children"][0].get("open").is_none());
    assert!(b["children"][0].get("selected").is_none());

    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    let tree = outliner(&d);
    assert_eq!(tree["instances"][0]["children"][0]["open"], true);
    assert!(tree["instances"][1]["children"][0].get("open").is_none());
}

#[test]
fn open_path_opens_a_nested_instance_from_anywhere() {
    let mut d = doc();
    let (first, second, group, _) = boxes(&mut d);
    let undo_depth = ask(&d, json!({"type": "history"}))["undo_depth"].clone();
    run(
        &mut d,
        json!({"type": "open_path", "path": [second, group]}),
    )
    .unwrap();
    assert_eq!(path(&d), json!([second, group]));
    // Inside the group: its box, in its own coordinates.
    let stats = ask(&d, json!({"type": "stats"}));
    assert_eq!(stats["faces"], 6);
    // Straight across to the other branch, then up to its component.
    run(&mut d, json!({"type": "open_path", "path": [first, group]})).unwrap();
    assert_eq!(path(&d), json!([first, group]));
    run(&mut d, json!({"type": "open_path", "path": [first]})).unwrap();
    assert_eq!(path(&d), json!([first]));
    // An empty path closes everything.
    run(&mut d, json!({"type": "open_path", "path": []})).unwrap();
    assert_eq!(path(&d), json!([]));
    assert_eq!(
        ask(&d, json!({"type": "history"}))["undo_depth"],
        undo_depth,
        "not undo steps"
    );
}

#[test]
fn open_path_clears_the_selection_unless_nothing_changes() {
    let mut d = doc();
    let (first, second, group, _) = boxes(&mut d);
    run(&mut d, json!({"type": "select", "ids": [first]})).unwrap();
    let revision = ask(&d, json!({"type": "stats"}))["revision"].clone();
    run(&mut d, json!({"type": "open_path", "path": []})).unwrap();
    assert_eq!(selection(&d), json!([first]), "already open: kept");
    run(&mut d, json!({"type": "open_path", "path": [second]})).unwrap();
    assert_eq!(selection(&d), json!([]));
    run(&mut d, json!({"type": "select", "ids": [group]})).unwrap();
    run(&mut d, json!({"type": "open_path", "path": [second]})).unwrap();
    assert_eq!(selection(&d), json!([group]));
    run(&mut d, json!({"type": "open_path", "path": [first]})).unwrap();
    assert_eq!(selection(&d), json!([]));
    assert_eq!(ask(&d, json!({"type": "stats"}))["revision"], revision);
}

#[test]
fn open_path_refuses_paths_that_do_not_lead_anywhere() {
    let mut d = doc();
    let (first, _, group, _) = boxes(&mut d);
    run(&mut d, json!({"type": "open_path", "path": [first]})).unwrap();
    run(&mut d, json!({"type": "select", "ids": [group]})).unwrap();
    assert_eq!(
        run(&mut d, json!({"type": "open_path", "path": [first, "f8"]})).unwrap_err(),
        (ErrorKind::InvalidInput, vec!["f8".to_string()])
    );
    // The group is inside the component, not in the root.
    assert_eq!(
        run(&mut d, json!({"type": "open_path", "path": [group]})).unwrap_err(),
        (ErrorKind::NotFound, vec![group.clone()])
    );
    assert_eq!(
        run(
            &mut d,
            json!({"type": "open_path", "path": [first, "i999"]})
        )
        .unwrap_err(),
        (ErrorKind::NotFound, vec!["i999".to_string()])
    );
    assert_eq!(path(&d), json!([first]), "unchanged");
    assert_eq!(selection(&d), json!([group]), "unchanged");
}

#[test]
fn open_path_in_a_failed_atomic_batch_is_rolled_back() {
    let mut d = doc();
    let (first, _, group, _) = boxes(&mut d);
    let commands: Vec<Command> = serde_json::from_value(json!([
        {"type": "open_path", "path": [first, group]},
        {"type": "push_pull", "face": "f999", "distance": 1.0}
    ]))
    .unwrap();
    let err = d.execute_batch(commands, true).unwrap_err();
    assert_eq!(err.index, 1);
    assert_eq!(path(&d), json!([]));
}

#[test]
fn the_tree_follows_edits_and_undo() {
    let mut d = doc();
    let (first, second, _, _) = boxes(&mut d);
    run(&mut d, json!({"type": "explode", "ids": [second]})).unwrap();
    let tree = outliner(&d);
    let top: Vec<&Value> = tree["instances"].as_array().unwrap().iter().collect();
    assert_eq!(top.len(), 2, "the component and the group that came out");
    assert_eq!(top[0]["id"], json!(first));
    assert_eq!(top[1]["kind"], "group");
    assert_eq!(top[1]["children"], json!([]));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(outliner(&d)["instances"][1]["id"], json!(second));
}

/// A group placing an "Outer" component placing an "Inner" component (a 2 x 2 x 1 box).
/// Returns (group, outer, inner).
fn nested(d: &mut Document) -> (String, String, String) {
    run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "push_pull", "face": "f8", "distance": 1.0}),
    )
    .unwrap();
    let ids = everything(d);
    let inner = created_instance(
        &run(
            d,
            json!({"type": "make_component", "ids": ids, "name": "Inner"}),
        )
        .unwrap(),
    );
    let outer = created_instance(
        &run(
            d,
            json!({"type": "make_component", "ids": [inner], "name": "Outer"}),
        )
        .unwrap(),
    );
    let group = created_instance(&run(d, json!({"type": "make_group", "ids": [outer]})).unwrap());
    run(d, json!({"type": "select", "ids": []})).unwrap();
    (group, outer, inner)
}

#[test]
fn a_three_level_tree_marks_the_open_path_and_the_selection_at_depth() {
    let mut d = doc();
    let (group, outer, inner) = nested(&mut d);
    run(&mut d, json!({"type": "open_path", "path": [group, outer]})).unwrap();
    run(&mut d, json!({"type": "select", "ids": [inner]})).unwrap();
    let tree = outliner(&d);
    let g = &tree["instances"][0];
    let o = &g["children"][0];
    let i = &o["children"][0];
    assert_eq!(
        (&g["id"], &o["id"], &i["id"]),
        (&json!(group), &json!(outer), &json!(inner))
    );
    assert_eq!(
        (&g["kind"], &o["name"], &i["name"]),
        (&json!("group"), &json!("Outer"), &json!("Inner"))
    );
    assert_eq!(g["open"], true);
    assert!(g.get("selected").is_none());
    assert_eq!(o["open"], true);
    assert!(o.get("selected").is_none());
    assert_eq!(i["selected"], true);
    assert!(i.get("open").is_none());
    assert_eq!(i["children"], json!([]));
}

#[test]
fn depth_one_truncates_the_middle_of_a_three_level_chain() {
    let mut d = doc();
    let (group, outer, inner) = nested(&mut d);
    let full = outliner(&d);
    let tree = ask(&d, json!({"type": "outliner", "depth": 1}));
    let g = &tree["instances"][0];
    assert_eq!(g["id"], json!(group));
    assert!(g.get("truncated").is_none());
    let o = &g["children"][0];
    assert_eq!(o["id"], json!(outer));
    assert_eq!(o["children"], json!([]));
    assert_eq!(o["truncated"], json!(true));
    // The full tree has the deeper level the cut left out.
    assert_eq!(
        full["instances"][0]["children"][0]["children"][0]["id"],
        json!(inner)
    );
}

#[test]
fn open_path_names_the_mid_path_parent_it_could_not_find_a_child_in() {
    let mut d = doc();
    let (group, outer, inner) = nested(&mut d);
    // The inner component is in the outer one, not directly in the group.
    let cmd: Command =
        serde_json::from_value(json!({"type": "open_path", "path": [group, inner, outer]}))
            .unwrap();
    let err = d.execute(cmd).unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    assert_eq!(
        err.ids.iter().map(|i| i.to_string()).collect::<Vec<_>>(),
        vec![inner.clone()]
    );
    assert!(err.message.contains(&group), "{}", err.message);
    assert_eq!(path(&d), json!([]), "unchanged");
}

#[test]
fn an_undo_that_closes_the_open_context_updates_the_marks() {
    let mut d = doc();
    let (group, outer, _) = nested(&mut d);
    run(&mut d, json!({"type": "open_path", "path": [group, outer]})).unwrap();
    assert_eq!(outliner(&d)["instances"][0]["open"], true);
    // Undoing make_group removes the group the open path starts with.
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(path(&d), json!([]));
    let tree = outliner(&d);
    let top = &tree["instances"][0];
    assert_eq!(top["id"], json!(outer));
    assert!(top.get("open").is_none(), "{tree}");
    assert!(top["children"][0].get("open").is_none(), "{tree}");
}

#[test]
fn a_failed_atomic_batch_also_restores_the_selection() {
    let mut d = doc();
    let (first, _, group, _) = boxes(&mut d);
    run(&mut d, json!({"type": "select", "ids": [first]})).unwrap();
    let commands: Vec<Command> = serde_json::from_value(json!([
        {"type": "open_path", "path": [first]},
        {"type": "select", "ids": [group]},
        {"type": "push_pull", "face": "f999", "distance": 1.0}
    ]))
    .unwrap();
    let err = d.execute_batch(commands, true).unwrap_err();
    assert_eq!(err.index, 2);
    assert_eq!(path(&d), json!([]));
    assert_eq!(selection(&d), json!([first]));
    assert_eq!(outliner(&d)["instances"][0]["selected"], true);
}

#[test]
fn a_non_atomic_batch_opens_a_parent_and_selects_a_leaf() {
    let mut d = doc();
    let (_, second, group, _) = boxes(&mut d);
    let commands: Vec<Command> = serde_json::from_value(json!([
        {"type": "open_path", "path": [second]},
        {"type": "select", "ids": [group]}
    ]))
    .unwrap();
    d.execute_batch(commands, false).unwrap();
    assert_eq!(path(&d), json!([second]));
    assert_eq!(selection(&d), json!([group]));
    let tree = outliner(&d);
    assert_eq!(tree["instances"][1]["open"], true);
    assert_eq!(tree["instances"][1]["children"][0]["selected"], true);
    assert!(
        tree["instances"][0]["children"][0]
            .get("selected")
            .is_none()
    );
}

#[test]
fn the_row_table_lists_each_definitions_instances_once() {
    let mut d = doc();
    let (first, second, group, red) = boxes(&mut d);
    let table = d.outliner_rows();
    let roots: Vec<String> = table.roots.iter().map(|r| r.id.to_string()).collect();
    assert_eq!(roots, vec![first, second]);
    assert_eq!(table.roots[1].material.map(|m| m.to_string()), Some(red));
    let boxed = table.children(table.roots[0].definition);
    assert_eq!(boxed.len(), 1);
    assert_eq!(boxed[0].id.to_string(), group);
    assert_eq!(boxed[0].name, "");
    assert!(table.children(boxed[0].definition).is_empty());
    // One entry per definition, however many times it is placed.
    assert_eq!(table.by_definition.len(), 2);
}

#[test]
fn depth_limits_the_tree_and_marks_what_it_left_out() {
    let mut d = doc();
    let (first, second, group, _) = boxes(&mut d);
    let full = outliner(&d);
    assert_eq!(ask(&d, json!({"type": "outliner", "depth": 1})), full);
    assert_eq!(ask(&d, json!({"type": "outliner", "depth": 9})), full);
    assert!(!full.to_string().contains("truncated"));
    let top = ask(&d, json!({"type": "outliner", "depth": 0}));
    let roots = top["instances"].as_array().unwrap();
    assert_eq!(roots.len(), 2);
    for (node, id) in roots.iter().zip([&first, &second]) {
        assert_eq!(node["id"], json!(id));
        assert_eq!(node["children"], json!([]));
        assert_eq!(node["truncated"], json!(true));
    }
    assert_eq!(full["instances"][0]["children"][0]["id"], json!(group));
    assert!(
        full["instances"][0]["children"][0]
            .get("truncated")
            .is_none()
    );
}
