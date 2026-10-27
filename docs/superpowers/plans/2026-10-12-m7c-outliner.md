# M7c Outliner Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The tree of groups and components can be read by agents (`outliner` query) and browsed in the window (an Outliner section in the right panel), and any nested instance can be selected or opened from it (`open_path`).

**Architecture:**
- session: one query (`outliner`, whole-model tree built recursively from `Scene::mesh` and `Model::instances`) and one context command (`open_path`) in `document/groups.rs` next to `open_instance`/`close_instance`; `Document::outliner()` is public so the window reuses it.
- No kernel, render, tools, api-crate or file-format changes; the api picks the new command and query up through serde/schemars (`describe`).
- app: the panel only emits existing commands (`open_path`, `select`) through two `Editor` methods; the tree is passed to the UI as `&[OutlinerNode]`; expand state lives in egui memory.

**Tech Stack:** Rust 2024; no dependency changes.

**Spec:** `docs/superpowers/specs/2026-10-10-tenon-design.md`: the outliner (UI section), commands and queries (section 5). Task 1 adds the M7c milestone line.

## Global Constraints

- Every new capability is a `session::Command` reachable through the API, covered by a command-driven test. Tools only emit commands.
- Each modeling command is one undo step. Command inputs and the points they produce must be finite and within +/-`MAX_COORDINATE` (1e6 m).
- No dependency changes.
- Git:
  - work on `dev`;
  - stage only the files you changed;
  - short plain commit messages, no emoji;
  - run `cargo fmt --all` before each commit;
  - `cargo clippy --workspace --all-targets -- -D warnings` stays clean.
- Do not launch the app window.

## Spec notes (rulings made in this plan)

1. **Query, not a new entity kind.** `{"type": "outliner"}` returns `QueryResult::Outliner { instances: Vec<OutlinerNode>, revision }`: the root's instances, each with the instances its definition places, recursively. It covers the whole model whatever instance is open (unlike `entities`, which is per open context), because the outliner's job is to navigate across contexts.
2. **Node fields.** `OutlinerNode { id, definition, kind, name, material?, open?, selected?, children }`. No separate child count: `children.len()` is it. `material` (the instance's own) is omitted when none, `open`/`selected` omitted when false (house style of `soft`, `curve`, `front_material`).
3. **Names.** `name` is the definition's name as stored, so an unnamed group reports `""` (same as `entity` and `definitions`; the query stays raw data). The window shows components as `<Name>` (SketchUp's outliner convention for unnamed component instances) and groups by name or as `Group`.
4. **No instance names.** SketchUp has per-instance names; adding them needs a kernel field and undo op, a `rename_instance` command, `.skr` schema 6 and Entity Info editing. Deferred: the roadmap scope is "tree, select and open", which works without them.
5. **Component contents repeat.** A component's contents appear under each of its instances with the same ids (ids are per definition). A node is therefore named by its path (ids from the root to it), which is what `open_path` takes. `open` marks only the nodes on the actual open path; `selected` marks only selected instances of the open context, never the same id under another placement.
6. **Order.** Each level in id order (creation order, the `BTreeMap` order of `Model::instances`). SketchUp sorts by name; id order is stable while renaming and avoids reshuffling equal names. Sorting by name deferred.
7. **One new command, `open_path {path}`.** Selection is per open context, so a node elsewhere needs a context change. With only `open_instance`/`close_instance` the outliner would need a run of closes and opens that depends on where the context is; `open_path` sets it in one command from anywhere (SketchUp Ruby's `model.active_path=`). Semantics: each entry must be an instance placed in the definition of the one before (the first in the root); `[]` is the root. Not an undo step, does not change the revision (like `open_instance`). Clears the selection, except when `path` is already the open context, in which case nothing changes (idempotent, so clients can always send it). Errors: a non-instance id is `invalid_input` ("... is not an instance: only instances can be opened"), an instance not placed where the path puts it (or unknown) is `not_found` ("no instance i9 in the root" / "in i4"), both with `ids` naming it; context and selection stay. Handled in `execute` (not `Document::edit`), so in an atomic batch it is allowed and rolled back with the context like `open_instance`. `Command::changes_model()` is true for it (as for open/close), so an API `open_path` ends the tool's operation.
8. **Selecting from the outliner = existing commands.** A click on a node whose parent path is the open context sends `select` (replace); a node elsewhere sends `open_path` (its parent's path) then `select` (SketchUp opens the context the entity is in and selects it). Shift-click or Cmd/Ctrl-click (egui `modifiers.command`) sends `select` with mode `toggle` when the node is in the open context; elsewhere it behaves as a plain click (one context at a time). No new select command.
9. **Double-click opens** the node (`open_path` with its full path), clearing the selection. The first click of the double-click has already selected it; that is harmless.
10. **Model row.** A "Model" row tops the tree (bold when the root is open); clicking it sends `open_path []` (back to the root; at the root it changes nothing).
11. **Placement in the UI.** An "Outliner" section in the existing right panel, under the Materials list (after a separator), heading plus a vertical scroll area (`id_salt("outliner")`, fills the space left). Entity Info and Materials keep their positions, so the existing panel tests are unaffected. No collapsible sections, no new panel, toolbar untouched (still ~1240 of 1280 pt).
12. **Tree widget.** egui `CollapsingState::load_with_default_open(.., node.open)` with `show_header` for nodes with children (expand arrow), an indented row for leaves. Nodes on the open path start expanded; others start collapsed; the state is egui memory keyed by the node's path (so the two placements of a component expand independently). Rows are full-width selectable buttons (`Button::selectable(..).right_text("").truncate().min_size(width)`): text left, long names cut short, the whole row clickable; selected rows highlighted, open ones bold.
13. **No filter box.** Deferred (with instance names it becomes more useful; models with few groups do not need it).
14. **Tree rebuilt every frame** from `Document::outliner()` in `App::render`. Cheap for normal models; caching by (revision, context, selection) deferred.
15. **Editor entry points.** `Editor::outliner_select(path, toggle)` and `Editor::open_path(path)` drop the tool's operation and typed text first (as other panel actions do via `run_selection_command`); a refused path (stale tree) shows the error in the status bar and leaves the context and selection.

## File Structure

| File | Change | Task |
|---|---|---|
| `crates/session/src/command.rs` | `Command::OpenPath` | 1 |
| `crates/session/src/query.rs` | `Query::Outliner`, `OutlinerNode`, `QueryResult::Outliner` | 1 |
| `crates/session/src/document.rs` | execute/edit/query arms | 1 |
| `crates/session/src/document/groups.rs` | `open_path`, `outliner`, `outliner_nodes` | 1 |
| `crates/session/src/lib.rs` | re-export `OutlinerNode` | 1 |
| `crates/session/tests/outliner.rs` | new: 8 command-driven tests | 1 |
| `crates/api/tests/dispatch.rs` | describe lists `open_path`, `outliner` | 1 |
| `docs/agent-api.md` | query row, command, open_path and outliner docs; window paragraph | 1, 2 |
| `docs/superpowers/specs/2026-10-10-tenon-design.md` | status line, section 5 lists, M7c line | 1 |
| `crates/app/src/editor.rs` | `outliner_select`, `open_path`, 3 tests | 2 |
| `crates/app/src/ui.rs` | Outliner section, `UiAction` variants, `View.outliner`, 3 tests | 2 |
| `crates/app/src/app.rs` | builds the tree per frame, applies the actions | 2 |

The code below was written and verified in a scratch copy of `dev` at 7b9c49c. Each task was checked on its own with `cargo fmt --all --check`, clippy with `-D warnings`, and the full test suite. Workspace test totals after each task: 725, 731 (from 717).

Code for new files is given in full. Changes to existing files are given as unified diffs against the state the previous task leaves.

---

### Task 1: Outliner query and open_path command

**Files:**
- Modify: `crates/session/src/command.rs` (variant `Command::OpenPath { path: Vec<EntityId> }` after `CloseInstance`, with doc comment)
- Modify: `crates/session/src/query.rs` (`Query::Outliner`, `OutlinerNode`, `QueryResult::Outliner`)
- Modify: `crates/session/src/document.rs` (`execute` arm `Command::OpenPath { path } => { self.open_path(&path)?; Ok(Outcome::default()) }`; `edit` returns None for it (added to the `OpenInstance | CloseInstance` arm); `query` arm `Query::Outliner => QueryResult::Outliner { instances: self.outliner(), revision: self.revision }`)
- Modify: `crates/session/src/document/groups.rs` (`open_path`, `outliner`, `outliner_nodes`; imports `InstanceId`, `OutlinerNode`)
- Modify: `crates/session/src/lib.rs` (re-export `OutlinerNode`)
- Create (Test): `crates/session/tests/outliner.rs`
- Modify (Test): `crates/api/tests/dispatch.rs` (describe schema lists gain `"open_path"` and `"outliner"`)
- Modify: `docs/agent-api.md` (query table row, command list, `open_path` paragraph in Groups and components, outliner result example and bullets)
- Modify: `docs/superpowers/specs/2026-10-10-tenon-design.md` (status line "M7c outliner"; section 5 command list "open a path of instances" and query list "the outliner tree of groups and components"; milestone line **M7c** after M7b)

**Interfaces produced:**
```rust
// session::command
pub enum Command { /* ... */ OpenPath { path: Vec<EntityId> }, /* ... */ }

// session::query
pub enum Query { /* ... */ Outliner, /* ... */ }
pub struct OutlinerNode {
    pub id: EntityId,
    pub definition: DefinitionId,
    pub kind: DefinitionKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<MaterialId>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub open: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub selected: bool,
    pub children: Vec<OutlinerNode>,
}
pub enum QueryResult { /* ... */ Outliner { instances: Vec<OutlinerNode>, revision: u64 }, /* ... */ }
// derives on OutlinerNode: Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema

// session::Document (document/groups.rs)
pub fn outliner(&self) -> Vec<OutlinerNode>;
pub(super) fn open_path(&mut self, path: &[EntityId]) -> Result<(), CommandError>;
fn outliner_nodes(&self, mesh: MeshId, path: &mut Vec<InstanceId>) -> Vec<OutlinerNode>;
```
`outliner_nodes` pushes each instance onto `path`, sets `open = self.context.starts_with(path)`, recurses into `MeshId::Definition(i.definition)`, pops; `selected = (*path == self.context before the push) && self.selection.contains(&EntityId::Instance(id))`.

- [ ] **Step 1: Write the failing tests**

Tests: - New integration file `crates/session/tests/outliner.rs` (8 tests; fixture `boxes()`: a 2x2x1 box grouped, the group made a "Box" component, the component copied 5 m along X, the copy painted with a new material, selection cleared): `an_empty_model_has_an_empty_tree`, `the_tree_lists_every_placed_instance_with_what_it_places`, `the_tree_marks_the_open_path_and_the_selection`, `open_path_opens_a_nested_instance_from_anywhere`, `open_path_clears_the_selection_unless_nothing_changes`, `open_path_refuses_paths_that_do_not_lead_anywhere`, `open_path_in_a_failed_atomic_batch_is_rolled_back`, `the_tree_follows_edits_and_undo`.
- Hunk in `crates/api/tests/dispatch.rs` (`describe` schema variant lists).

Test-module hunks are part of the Step 3 diffs; apply them first, along with the test files below.

Apply to `crates/api/tests/dispatch.rs`:
```diff
--- a/crates/api/tests/dispatch.rs
+++ b/crates/api/tests/dispatch.rs
@@ -139,8 +139,9 @@ fn describe_lists_methods_and_schemas() {
         "make_component",
         "explode",
         "open_instance",
         "close_instance",
+        "open_path",
         "rename_definition",
         "purge_definitions",
         "add_material",
         "change_material",
@@ -163,8 +164,9 @@ fn describe_lists_methods_and_schemas() {
         "context",
         "file",
         "curve",
         "materials",
+        "outliner",
     ] {
         assert!(
             query_schema.contains(variant),
             "Query schema lacks {variant}"
```

Create `crates/session/tests/outliner.rs`:
```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

verified: `cargo test -p session --test outliner` with the test file only -> 8 failed (unknown variant `outliner` / `open_path` in JSON deserialization). The dispatch hunk fails by construction (schema lacks the names).

- [ ] **Step 3: Implement**

Apply to `crates/session/src/command.rs`:
```diff
--- a/crates/session/src/command.rs
+++ b/crates/session/src/command.rs
@@ -135,8 +135,14 @@ pub enum Command {
     },
     /// Go back to editing the context the open instance is in. Clears the selection. Not
     /// recorded in undo history.
     CloseInstance,
+    /// Make `path` the open context: instances from the root, each placed in the definition
+    /// of the one before (empty: the root), as the `outliner` query lists them. Clears the
+    /// selection unless `path` is already open. Not recorded in undo history.
+    OpenPath {
+        path: Vec<EntityId>,
+    },
     /// Rename a group or component definition. A component's name must not be empty.
     RenameDefinition {
         definition: DefinitionId,
         name: String,
```

Apply to `crates/session/src/document.rs`:
```diff
--- a/crates/session/src/document.rs
+++ b/crates/session/src/document.rs
@@ -189,8 +189,12 @@ impl Document {
             Command::CloseInstance => {
                 self.close_instance()?;
                 Ok(Outcome::default())
             }
+            Command::OpenPath { path } => {
+                self.open_path(&path)?;
+                Ok(Outcome::default())
+            }
             Command::New { discard } => {
                 self.check_unsaved(discard)?;
                 self.new_model();
                 Ok(Outcome::default())
@@ -334,9 +338,10 @@ impl Document {
             | Command::StandardView { .. }
             | Command::SetProjection { .. }
             | Command::Select { .. }
             | Command::OpenInstance { .. }
-            | Command::CloseInstance => {
+            | Command::CloseInstance
+            | Command::OpenPath { .. } => {
                 return None;
             }
         })
     }
@@ -1031,8 +1036,12 @@ impl Document {
                     MeshId::Root => None,
                     MeshId::Definition(d) => Some(d),
                 },
             },
+            Query::Outliner => QueryResult::Outliner {
+                instances: self.outliner(),
+                revision: self.revision,
+            },
             Query::File => QueryResult::File {
                 path: self.path.as_ref().map(|p| p.display().to_string()),
                 modified: self.is_modified(),
                 revision: self.revision,
```

Apply to `crates/session/src/document/groups.rs`:
```diff
--- a/crates/session/src/document/groups.rs
+++ b/crates/session/src/document/groups.rs
@@ -1,13 +1,13 @@
 //! Groups, components and the open context.
 
 use std::collections::BTreeSet;
 
-use kernel::{DefinitionKind, EntityId, KernelError, MeshId, SceneChange};
+use kernel::{DefinitionKind, EntityId, InstanceId, KernelError, MeshId, SceneChange};
 
 use super::{Document, MAX_COORDINATE, check_coordinates};
 use crate::command::{CommandError, ErrorKind};
-use crate::query::{Bounds, InstanceInfo};
+use crate::query::{Bounds, InstanceInfo, OutlinerNode};
 
 impl Document {
     /// Makes a group (`name` None) or a named component from `ids` and selects its instance.
     pub(super) fn group_changes(
@@ -160,8 +160,73 @@ impl Document {
         self.selection.clear();
         Ok(())
     }
 
+    /// Makes `path` the open context; the selection is cleared unless it already was.
+    pub(super) fn open_path(&mut self, path: &[EntityId]) -> Result<(), CommandError> {
+        let mut context = Vec::with_capacity(path.len());
+        let mut mesh = MeshId::Root;
+        for id in path {
+            let EntityId::Instance(i) = *id else {
+                return Err(CommandError::new(
+                    ErrorKind::InvalidInput,
+                    format!("{id} is not an instance: only instances can be opened"),
+                )
+                .with_ids(vec![*id]));
+            };
+            let Some(instance) = self.scene.mesh(mesh).and_then(|m| m.instance(i)) else {
+                let place = match context.last() {
+                    None => "the root".to_string(),
+                    Some(parent) => EntityId::Instance(*parent).to_string(),
+                };
+                return Err(CommandError::new(
+                    ErrorKind::NotFound,
+                    format!("no instance {id} in {place}"),
+                )
+                .with_ids(vec![*id]));
+            };
+            mesh = MeshId::Definition(instance.definition);
+            context.push(i);
+        }
+        if context != self.context {
+            self.context = context;
+            self.selection.clear();
+        }
+        Ok(())
+    }
+
+    /// The tree of placed groups and components, from the root's instances down.
+    pub fn outliner(&self) -> Vec<OutlinerNode> {
+        self.outliner_nodes(MeshId::Root, &mut Vec::new())
+    }
+
+    /// The nodes for the instances in `mesh`, which `path` (from the root) opens.
+    fn outliner_nodes(&self, mesh: MeshId, path: &mut Vec<InstanceId>) -> Vec<OutlinerNode> {
+        let Some(m) = self.scene.mesh(mesh) else {
+            return Vec::new();
+        };
+        let in_context = *path == self.context;
+        m.instances()
+            .map(|(id, i)| {
+                let d = self.scene.definition(i.definition).expect("placed");
+                path.push(id);
+                let open = self.context.starts_with(path);
+                let children = self.outliner_nodes(MeshId::Definition(i.definition), path);
+                path.pop();
+                OutlinerNode {
+                    id: EntityId::Instance(id),
+                    definition: i.definition,
+                    kind: d.kind,
+                    name: d.name.clone(),
+                    material: i.material,
+                    open,
+                    selected: in_context && self.selection.contains(&EntityId::Instance(id)),
+                    children,
+                }
+            })
+            .collect()
+    }
+
     pub(super) fn instance_info(&self, id: kernel::InstanceId) -> InstanceInfo {
         let i = self.model().instance(id).unwrap();
         let d = self.scene.definition(i.definition).unwrap();
         InstanceInfo {
```

Apply to `crates/session/src/lib.rs`:
```diff
--- a/crates/session/src/lib.rs
+++ b/crates/session/src/lib.rs
@@ -18,8 +18,8 @@ pub use command::{BatchError, Command, CommandError, ErrorKind, Outcome, SelectM
 pub use document::{Document, MAX_COORDINATE, MAX_COPIES};
 pub use kernel::{CurveKind, MaterialId, Side};
 pub use query::{
     Bounds, ChangeEvent, CurveInfo, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, GuideInfo,
-    GuideKind, InstanceInfo, MaterialInfo, Query, QueryResult, VertexInfo,
+    GuideKind, InstanceInfo, MaterialInfo, OutlinerNode, Query, QueryResult, VertexInfo,
 };
 pub use rectangle::{rectangle_axes, rectangle_corners};
 pub use rotate::rotation;
```

Apply to `crates/session/src/query.rs`:
```diff
--- a/crates/session/src/query.rs
+++ b/crates/session/src/query.rs
@@ -39,8 +39,11 @@ pub enum Query {
     /// Every material.
     Materials,
     /// The open context: the path of instances being edited, from the root.
     Context,
+    /// The tree of placed groups and components: the root's instances, each with the
+    /// instances its definition places, at any depth.
+    Outliner,
     /// The file the model was last opened from or saved to, and whether it changed since.
     File,
     /// Model changes after `since` (a revision number), oldest first.
     Events {
@@ -184,8 +187,32 @@ pub struct DefinitionInfo {
     pub edges: usize,
     pub faces: usize,
 }
 
+/// A placed group or component in the `outliner` tree. A component's contents appear under
+/// each of its instances, with the same ids; a node's path is the ids from the root down to it.
+#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
+pub struct OutlinerNode {
+    pub id: EntityId,
+    pub definition: DefinitionId,
+    pub kind: DefinitionKind,
+    /// The definition's name; empty for an unnamed group.
+    pub name: String,
+    /// The instance's material; absent for none.
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub material: Option<MaterialId>,
+    /// True when the node is on the open context's path (the open instance or one it is in).
+    /// Absent when false.
+    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
+    pub open: bool,
+    /// True when the node is an instance of the open context and is selected. Absent when
+    /// false.
+    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
+    pub selected: bool,
+    /// The instances its definition places, in id order.
+    pub children: Vec<OutlinerNode>,
+}
+
 /// A material and how much of the model uses it.
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
 pub struct MaterialInfo {
     pub id: MaterialId,
@@ -257,8 +284,13 @@ pub enum QueryResult {
     Context {
         path: Vec<EntityId>,
         definition: Option<DefinitionId>,
     },
+    /// The root's instances, in id order, each with what it places.
+    Outliner {
+        instances: Vec<OutlinerNode>,
+        revision: u64,
+    },
     /// `path` is null for a model never saved. `modified` is true when the model changed
     /// since it was opened, saved or started.
     File {
         path: Option<String>,
```

Apply to `docs/agent-api.md`:
~~~~diff
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -53,16 +53,17 @@ Queries:
 | `{"type": "selection"}` | the selected edge, face, instance and guide ids, plus `revision` |
 | `{"type": "definitions"}` | every group and component definition: `id`, `name`, `kind`, how many `instances` place it, `edges`, `faces` |
 | `{"type": "materials"}` | every material: `id`, `name`, `color`, and how many `faces` and `instances` in the whole model are painted with it; see Materials below |
 | `{"type": "context"}` | the open context: `path` (open instances from the root, empty at the root) and the `definition` being edited |
+| `{"type": "outliner"}` | the tree of placed groups and components in the whole model, plus `revision`; see Groups and components below |
 | `{"type": "file"}` | `path` of the file last opened or saved (`null` if never), `modified` since then, and `revision` |
 | `{"type": "events", "since": 3}` | model changes after revision 3, oldest first |
 
 Unknown fields in commands, queries and params are rejected with `-32602` (a typo fails instead
 of silently doing nothing).
 
 Commands: `add_edge`, `add_rectangle`, `add_circle`, `add_arc`, `push_pull`, `erase`, `move`, `copy`, `rotate`, `make_group`,
-`make_component`, `explode`, `open_instance`, `close_instance`, `rename_definition`,
+`make_component`, `explode`, `open_instance`, `close_instance`, `open_path`, `rename_definition`,
 `purge_definitions`, `add_guide_line`, `add_guide_point`, `delete_guides`, `add_material`,
 `change_material`, `delete_material`, `paint`, `select`, `undo`,
 `redo`, `set_camera`, `orbit`, `pan`, `zoom`, `zoom_extents`, `standard_view`, `set_projection`,
 `new`, `open`, `save`, `export`.
@@ -213,9 +214,15 @@ current material and adds, renames, recolors and deletes materials, all through
 Commands and queries work on the open context: the root at first. `{"type": "open_instance",
 "id": "i4"}` opens an instance of the open context for editing, and `{"type": "close_instance"}`
 goes back out one level. Neither is an undo step or changes the revision; both clear the
 selection. `open_instance` with an id that is not an instance, and `close_instance` at the root,
-are `invalid_input`. While an instance is open:
+are `invalid_input`. `{"type": "open_path", "path": ["i4", "i9"]}` opens a nested instance
+directly, from wherever the open context is: `path` lists instances from the root, each placed in
+the definition of the one before (`i9` inside `i4`'s definition), and becomes the open context;
+`[]` goes back to the root. It clears the selection, except when `path` is already the open
+context (then nothing changes). An id in `path` that is not an instance is `invalid_input`, and
+one that is not placed where the path puts it is `not_found`, both naming it. While an instance
+is open:
 
 - `entities`, `entity`, `stats` and `selection` see only the definition's entities: `entity` and
   commands give ids from elsewhere `not_found`, and `entities` leaves them out;
 - points and offsets in commands, and positions in results, are in the definition's own
@@ -231,8 +238,32 @@ were made in (the instance's transform starts as the identity).
 
 A command's `changes.created` can list ids inside a new definition (`make_group`, copying a
 group): they are `not_found` from the open context until that instance is opened.
 
+`{"type": "outliner"}` returns the tree of every placed group and component, whatever instance is
+open:
+
+```
+{"type": "outliner", "revision": 7, "instances": [
+  {"id": "i55", "definition": "d54", "kind": "component", "name": "Box", "open": true,
+   "children": [{"id": "i27", "definition": "d26", "kind": "group", "name": "", "selected": true,
+                 "children": []}]},
+  {"id": "i56", "definition": "d54", "kind": "component", "name": "Box", "material": "m57",
+   "children": [{"id": "i27", "definition": "d26", "kind": "group", "name": "", "children": []}]}]}
+```
+
+- `instances` are the root's instances in id order; each node's `children` are the instances its
+  definition places, in id order, at any depth. A component's contents appear under each of its
+  instances with the same ids, so a node is named by its path: the ids from the root down to it
+  (`["i55", "i27"]` above, with `i55` open and `i27` selected in it), which is what `open_path` takes.
+- `name` is the definition's name (empty for an unnamed group; the window shows it as `Group`);
+  `material` is the instance's own (absent for none).
+- `open` (absent when false) marks the nodes on the open context's path: the open instance and
+  the ones it is inside of. `selected` (absent when false) marks selected instances of the open
+  context; the same ids under another placement are not marked.
+- To select a node elsewhere in the tree, open its parent's path, then select it:
+  `open_path` with the path minus its last id, then `select`.
+
 `{"type": "rename_definition", "definition": "d5", "name": "Chair"}` renames a definition (one
 undo step; a component's name must not be empty; an unknown definition is `not_found`).
 `{"type": "purge_definitions"}` removes every definition no instance places, such as a component
 whose last instance was erased, and what only it placed (one undo step).
~~~~

Apply to `docs/superpowers/specs/2026-10-10-tenon-design.md`:
```diff
--- a/docs/superpowers/specs/2026-10-10-tenon-design.md
+++ b/docs/superpowers/specs/2026-10-10-tenon-design.md
@@ -1,8 +1,8 @@
 # Tenon — Design
 
 Date: 2026-10-10
-Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope; M5 split into M5a-c; M5b tools); 2026-10-12 (M5c files, M5d export, M6a circles, M6b rotate, M6c guides, M6d curves, M7a camera, M7b materials)
+Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope; M5 split into M5a-c; M5b tools); 2026-10-12 (M5c files, M5d export, M6a circles, M6b rotate, M6c guides, M6d curves, M7a camera, M7b materials, M7c outliner)
 
 ## Goal
 
 A desktop 3D modeler in Rust with SketchUp-style direct modeling: draw edges, faces form automatically, push/pull faces, inference snapping, groups and components. macOS first; Windows and Linux supported through wgpu without dedicated work.
@@ -150,16 +150,16 @@ Unit tests for buffer building and camera math; a small set of headless golden-i
 Design rule for every milestone: the interface is one client of the command layer, and the API is another. No feature may exist only in the interface.
 
 ### Commands
 
-- `session::Command` is a serde-serializable enum covering every state change: modeling (add edge, draw rectangle/circle, push/pull face by distance, move/copy/rotate entities, erase, make group/component, explode), guides (add a guide line or point, delete guides), editing context (open/close an instance), selection, camera (set, orbit, pan, zoom, zoom extents, standard views, projection), undo/redo, file (new, open, save, import, export).
+- `session::Command` is a serde-serializable enum covering every state change: modeling (add edge, draw rectangle/circle, push/pull face by distance, move/copy/rotate entities, erase, make group/component, explode), guides (add a guide line or point, delete guides), editing context (open/close an instance, open a path of instances), selection, camera (set, orbit, pan, zoom, zoom extents, standard views, projection), undo/redo, file (new, open, save, import, export).
 - Commands take explicit values (points, distances, entity IDs), never screen coordinates or cursor state.
 - `Document::execute(Command) -> Result<Outcome, CommandError>`, where `Outcome` carries the net `ChangeSummary` (created/deleted/modified ids as strings). Each modeling command is one undo step; camera commands are not recorded. Errors are structured (`kind` + `message` + offending `ids`), never panics. Raw `ChangeSet`s stay internal.
 - Entity IDs are the kernel's stable ids, serialized as strings (`"v12"`, `"e3"`, `"f7"`, `"i4"` for an instance, `"g9"` for a guide); definitions are `"d<n>"`.
 
 ### Queries
 
-Read-only, structured: list/get entities (vertices, edges, faces, groups, instances, guides) with geometry, bounding-box and type filters, selection, camera, model stats, undo history. Agents do not need screenshots to know the model state.
+Read-only, structured: list/get entities (vertices, edges, faces, groups, instances, guides) with geometry, bounding-box and type filters, selection, camera, model stats, undo history, the outliner tree of groups and components. Agents do not need screenshots to know the model state.
 
 ### API server (`api`)
 
 - JSON-RPC 2.0 over a Unix domain socket (default, socket file mode 0600, path printed at startup) or stdio (headless). No network listener.
@@ -199,8 +199,9 @@ Every milestone's acceptance includes: new capabilities are reachable as `sessio
 - **M6c**: guides and the Tape Measure: guide lines (infinite) and guide points are model entities (`g<n>` ids) of the context they are drawn in; `add_guide_line`, `add_guide_point` and `delete_guides` commands, and `erase`, `select`, `move`, `rotate`, `copy`, `make_group` and `explode` take guides; saved in `.skr` (schema version 2; version 1 still opens), never exported. Inference snaps to guide points, onto guide lines, and where guide lines cross edges or each other. Guides are drawn dashed (guide points as small crosses), picked, box-selected and erased like edges, and shown in Entity Info. Tape Measure tool (`t`): from a point the second click adds a guide point, from an edge or guide line a parallel guide line; typed distances, retyped right after to adjust; Ctrl switches to measuring only. Edit > Delete Guides.
 - **M6d**: curves: edges carry an optional curve id; the edges of a curve always form one unbranched, non-straight chain (sticky splits keep both pieces on the curve, erasing leaves the rest a curve, a piece that falls off gets a new id, a lone edge is plain), and copies, groups and explodes get new curve ids; the shape (circle or arc, center, normal, radius, angle, segments, length) is derived from the edges, so moves and rotations need no upkeep; `add_circle` tags its edges; an `add_arc` command (center, start, signed angle in degrees, plane normal, 2-360 segments, default 12); a `curve` query and `curve`/`soft` fields on edges; `.skr` schema version 3 (versions 1 and 2 still open). Push/pull of a face bounded by a curve gives the cap a curve of its own and makes the swept edges between curve segments soft; a soft edge bounding fewer than two faces hardens. In the window, soft edges are not drawn, picked or snapped to; clicking a curve edge with Select selects the whole curve; Entity Info shows circles and arcs with radius, angle, segments and length. Arc tool (`a`, SketchUp's 2-point arc: start, end, bulge; typed chord and bulge lengths, "12s" segments). Smooth shading across soft edges is deferred.
 - **M7a**: camera: commands `orbit` (yaw and pitch in degrees about the target), `pan` (meters along the screen), `zoom` (factor), `zoom_extents` (the whole model in root coordinates, guides excluded; an empty model leaves the camera), `standard_view` (top, bottom, front, back, left, right, iso; same target and distance) and `set_projection` (perspective or parallel; the view height at the target is kept, so a parallel view's zoom is the eye's distance); `set_camera` takes `up` (the screen's up in views straight down or up, which are now allowed) and `projection`; the `camera` query reports `up`, `projection` and `view_height`; camera commands are not undo steps and do not change the revision; `.skr` schema version 4 saves `up` and `projection` (versions 1-3 open in perspective). A parallel view sees 100 eye-to-target distances either side of the target, behind the eye too: rays start there, and picking, inference, overlay sizes (snap markers, guide crosses, pick radius, protractor), guide line spans, render depth offsets and shading work in both projections; `render_view` takes `up` and `projection`. Camera menu: Zoom Extents (Shift+Z), the standard views, Perspective and Parallel Projection; camera moves from the menu keep the tool's operation going. Erase Selection moves from the toolbar to the Edit menu so the toolbar keeps fitting a 1280-point window.
 - **M7b**: materials: named sRGB colors (`m<n>` ids from the entity id counter) belonging to the whole model; each face side and each instance carries an optional material (none is the default material); an instance's material shows on face sides inside it without one (the nearest painted instance wins), and exploding bakes it onto them. Commands `add_material` (optional name, default "Material", "Material 2", ...; names distinct, not blank, at most 256 characters), `change_material` (name and/or color), `delete_material` (unpaints everything painted with it, one undo step) and `paint` (faces and instances of the open context, `side` front, back or both, null for the default); a `materials` query (with face and instance use counts); `front_material`/`back_material` on faces and `material` on instances in queries; `changes.materials` lists materials a command added, removed or changed. Paint follows geometry: split pieces keep it, merges keep the containing face's, copies/groups/components keep it, push/pull gives the cap and sides the pushed face's materials (swapped for a sheet pushed against its normal, so the side facing the pull ends up outside), a face turned over keeps each material on its side. `.skr` schema version 5 (versions 1-4 open unpainted); files painting with missing materials or with bad material names are refused. Rendering shows each side's color (selection tint on top); export writes OBJ with a `.mtl` beside it, glTF with one material per color (front side only; STL has no colors). Paint tool (`b`, last toolbar button: click a face to paint the side facing the viewer with the current material, a click on something selected paints the whole selection's faces and instances, an instance paints the instance; Cmd-click or Ctrl-click (either key on every system; Alt is the orbit modifier) picks up the clicked side's material) and a Materials list in the right panel under Entity Info (Default plus every material with a swatch; clicking one makes it current and switches to Paint; Add, Delete, a name field renaming on Enter, and a 12-color palette to recolor); Entity Info shows a face's front and back materials and a painted instance's. Opacity, textures and a free color picker are deferred.
+- **M7c**: outliner: an `outliner` query (the tree of placed groups and components, the root's instances down, each level in id order: `id`, `definition`, `kind`, `name` (the definition's; empty for an unnamed group), `material`, `children`; `open` marks the nodes on the open context's path and `selected` the selected instances of the open context; a component's contents appear under each of its instances with the same ids, so a node is named by its path from the root) and an `open_path` command (makes a path of instances from the root the open context from wherever it is, `[]` being the root; clears the selection unless the path is already open; not an undo step; non-instances `invalid_input`, misplaced instances `not_found`). In the window, an Outliner section in the right panel under Materials: a Model row (click: back to the root) and the tree with expand and collapse (nodes on the open path start expanded), groups shown by name or as "Group", components as "<Name>", selected rows highlighted and open ones bold. A click selects the node, first opening the context it is in (as SketchUp does); Shift-click or Cmd/Ctrl-click adds it to or removes it from the selection when it is in the open context; a double-click opens it for editing. Instance names, a filter box, sorting by name and drag-to-reparent are deferred.
 
 ## Risks
 
 - **Sticky geometry robustness**: edge merging and face detection under floating-point error is the main risk. Mitigation: robust predicates, explicit tolerances, invariant checks in every test, fuzz tests on random edge sequences.
```

- [ ] **Step 4: Run the tests to verify they pass**

`cargo test -p session --test outliner` (8 passed); `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace` -> 725 passed, 0 failed.

Expected workspace totals after each task: 725, 731.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/api/tests/dispatch.rs crates/session/tests/outliner.rs crates/session/src/command.rs crates/session/src/document.rs crates/session/src/document/groups.rs crates/session/src/lib.rs crates/session/src/query.rs docs/agent-api.md docs/superpowers/specs/2026-10-10-tenon-design.md
git commit -m "Outliner query and open_path command: the tree of placed groups and components with open and selected marks; open a path of instances from the root; spec M7c"
```

### Task 2: Outliner panel

**Files:**
- Modify: `crates/app/src/editor.rs` (`outliner_select`, `open_path`; test-module hunk)
- Modify: `crates/app/src/ui.rs` (module doc; imports `egui::collapsing_header::CollapsingState`, `kernel::{DefinitionKind, EntityId}`, `session::OutlinerNode`; `UiAction` variants; `View.outliner`; `outliner` section called after `materials` with a separator; helpers; every test `View { .. }` gains `outliner: &[]` (7 places); test-module hunk)
- Modify: `crates/app/src/app.rs` (`let outliner = self.editor.doc().outliner();` before `View`, `outliner: &outliner`; `apply` arms for the two actions)
- Modify: `docs/agent-api.md` ("In the window, the Outliner ..." paragraph after the outliner bullets)

**Interfaces produced:**
```rust
// app::editor::Editor
pub fn outliner_select(&mut self, path: &[EntityId], toggle: bool) -> bool;
// split_last of path (false when empty); reset_tool, typed.clear, message = None;
// in_context = parent ids == doc.context(); if !in_context run OpenPath { path: parent }
// (stop if it fails); then run Select { ids: [id], mode: Toggle if toggle && in_context else Replace };
// update_hover; true.
pub fn open_path(&mut self, path: Vec<EntityId>) -> bool; // run_selection_command(Command::OpenPath { path })

// app::ui
pub enum UiAction {
    /* ... */
    OutlinerSelect { path: Vec<EntityId>, toggle: bool },
    OpenPath(Vec<EntityId>),
}
pub struct View<'a> { /* ... */ pub outliner: &'a [OutlinerNode] }
fn outliner(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>);
fn outliner_node(ui: &mut Ui, node: &OutlinerNode, path: &mut Vec<EntityId>, actions: &mut Vec<UiAction>);
fn outliner_row(ui: &mut Ui, node: &OutlinerNode, path: &[EntityId]) -> Option<UiAction>;
fn row_button(ui: &mut Ui, selected: bool, text: RichText) -> egui::Response;
fn outliner_label(node: &OutlinerNode) -> String;

// app::app::App::apply
UiAction::OutlinerSelect { path, toggle } => { self.editor.outliner_select(&path, toggle); }
UiAction::OpenPath(path) => { self.editor.open_path(path); }
```
`outliner_row`: `double_clicked()` -> `OpenPath(path)`, else `clicked()` -> `OutlinerSelect { path, toggle: ui.input(|i| i.modifiers.shift || i.modifiers.command) }`. Collapsing id: `ui.make_persistent_id(("outliner", path.as_slice()))`; leaves get `ui.add_space(ui.spacing().indent)` in a horizontal row.

Tests (all test-module hunks, no new files):
- `editor.rs`: helpers `two_boxes` (moved_group -> "Box" component -> copied 5 m), `context_ids`, `selected`; tests `outliner_clicks_open_the_nodes_context_and_select_it`, `outliner_double_clicks_open_the_node_and_end_the_tools_operation`, `a_stale_outliner_path_shows_why_and_keeps_the_context`.
- `ui.rs`: helpers `node`, `boxes`, `ids`, `outliner_view`, `outliner_clicks` (one click per y every 4 pt in a fresh egui context, with modifiers sent as `egui::Event::ModifiersChanged`; RawInput has no `modifiers` field in egui 0.36); tests `outliner_labels_name_groups_and_components`, `outliner_rows_select_their_paths_and_the_model_row_goes_back_to_the_root`, `double_clicking_an_outliner_row_opens_it_and_expanding_shows_children` (expand button found by clicking from the panel's left edge `central.max.x + dx`; 30 idle frames for the animation before clicking below).

- [ ] **Step 1: Write the failing tests**

Test-module hunks are part of the Step 3 diffs; apply them first, along with the test files below.

- [ ] **Step 2: Run the tests to verify they fail**

editor and ui tests fail to compile without the methods/variants/field (by construction); verified for the panel: with the `outliner(ui, view, &mut actions)` call removed, the two click tests fail (2 failed).

- [ ] **Step 3: Implement**

Apply to `crates/app/src/app.rs`:
```diff
--- a/crates/app/src/app.rs
+++ b/crates/app/src/app.rs
@@ -321,8 +321,14 @@ impl State {
             }
             UiAction::RecolorMaterial(material, color) => {
                 self.editor.recolor_material(material, color);
             }
+            UiAction::OutlinerSelect { path, toggle } => {
+                self.editor.outliner_select(&path, toggle);
+            }
+            UiAction::OpenPath(path) => {
+                self.editor.open_path(path);
+            }
         }
     }
 
     fn resize(&mut self, width: u32, height: u32) {
@@ -371,8 +377,9 @@ impl State {
                 name: m.name.clone(),
                 color: m.color,
             })
             .collect();
+        let outliner = self.editor.doc().outliner();
         let panels = View {
             tool: self.editor.tool_kind(),
             status: &status,
             info: &info,
@@ -387,8 +394,9 @@ impl State {
             has_guides: self.editor.doc().model().guides().next().is_some(),
             projection: self.editor.doc().camera.projection,
             materials: &materials,
             material: self.editor.current_material(),
+            outliner: &outliner,
         };
         let mut gui_frame = self.gui.frame(&self.window, &panels);
         // Applied once the frame is presented: some open modal dialogs, which must not run
         // while this frame's surface texture is held.
```

Apply to `crates/app/src/editor.rs`:
```diff
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -554,8 +554,47 @@ impl Editor {
         }
         self.run_selection_command(Command::CloseInstance)
     }
 
+    /// Selects the outliner node at `path` (instance ids from the root down to it), first
+    /// opening the context it is in when that is not the open one. With `toggle` a node of the
+    /// open context is added to or removed from the selection instead. False for an empty path.
+    pub fn outliner_select(&mut self, path: &[EntityId], toggle: bool) -> bool {
+        let Some((id, parent)) = path.split_last() else {
+            return false;
+        };
+        self.reset_tool();
+        self.typed.clear();
+        self.message = None;
+        let open = self.doc.context().iter().map(|i| EntityId::Instance(*i));
+        let in_context = parent.iter().copied().eq(open);
+        if in_context
+            || self
+                .run(Command::OpenPath {
+                    path: parent.to_vec(),
+                })
+                .is_some()
+        {
+            let mode = if toggle && in_context {
+                SelectMode::Toggle
+            } else {
+                SelectMode::Replace
+            };
+            self.run(Command::Select {
+                ids: vec![*id],
+                mode,
+            });
+        }
+        self.update_hover();
+        true
+    }
+
+    /// Opens the instances along `path` (from the root; empty: the root) for editing, as a
+    /// double-click in the outliner does.
+    pub fn open_path(&mut self, path: Vec<EntityId>) -> bool {
+        self.run_selection_command(Command::OpenPath { path })
+    }
+
     /// Runs a command that works on the selection or the open context, dropping the tool's
     /// operation and any typed text first.
     fn run_selection_command(&mut self, command: Command) -> bool {
         self.reset_tool();
@@ -2168,8 +2207,88 @@ mod tests {
             "the square's edges and face are no group"
         );
     }
 
+    /// `moved_group` made a "Box" component, copied 5 m along X. Returns (first component,
+    /// second component, the group inside both).
+    fn two_boxes(e: &mut Editor) -> (EntityId, EntityId, EntityId) {
+        let group = moved_group(e);
+        e.execute(Command::MakeComponent {
+            ids: vec![group],
+            name: "Box".into(),
+        })
+        .unwrap();
+        let first = *e.doc().selection().first().unwrap();
+        let out = e
+            .execute(Command::Copy {
+                ids: vec![first],
+                offset: [5.0, 0.0, 0.0],
+                count: None,
+            })
+            .unwrap();
+        let second = *out.changes.created.first().unwrap();
+        (first, second, group)
+    }
+
+    fn context_ids(e: &Editor) -> Vec<EntityId> {
+        e.doc()
+            .context()
+            .iter()
+            .map(|i| EntityId::Instance(*i))
+            .collect()
+    }
+
+    fn selected(e: &Editor) -> Vec<EntityId> {
+        e.doc().selection().iter().copied().collect()
+    }
+
+    #[test]
+    fn outliner_clicks_open_the_nodes_context_and_select_it() {
+        let mut e = editor();
+        let (first, second, group) = two_boxes(&mut e);
+        assert!(e.outliner_select(&[second, group], false));
+        assert_eq!(context_ids(&e), [second]);
+        assert_eq!(selected(&e), [group]);
+        assert!(e.outliner_select(&[first], false));
+        assert!(context_ids(&e).is_empty());
+        assert_eq!(selected(&e), [first]);
+        // Shift or Cmd adds and removes nodes of the open context.
+        assert!(e.outliner_select(&[second], true));
+        assert_eq!(selected(&e), [first, second]);
+        assert!(e.outliner_select(&[first], true));
+        assert_eq!(selected(&e), [second]);
+        // A node elsewhere is selected alone, in its own context.
+        assert!(e.outliner_select(&[first, group], true));
+        assert_eq!(context_ids(&e), [first]);
+        assert_eq!(selected(&e), [group]);
+        assert!(!e.outliner_select(&[], false));
+    }
+
+    #[test]
+    fn outliner_double_clicks_open_the_node_and_end_the_tools_operation() {
+        let mut e = editor();
+        let (_, second, group) = two_boxes(&mut e);
+        click_at(&mut e, DVec3::new(0.0, -3.0, 0.0));
+        assert!(e.tool.anchor().is_some(), "a line is started");
+        assert!(e.open_path(vec![second, group]));
+        assert!(e.tool.anchor().is_none());
+        assert_eq!(context_ids(&e), [second, group]);
+        assert!(selected(&e).is_empty());
+        assert!(e.open_path(Vec::new()));
+        assert!(context_ids(&e).is_empty());
+    }
+
+    #[test]
+    fn a_stale_outliner_path_shows_why_and_keeps_the_context() {
+        let mut e = editor();
+        let (first, _, group) = two_boxes(&mut e);
+        e.execute(Command::OpenInstance { id: first }).unwrap();
+        let gone: EntityId = "i999".parse().unwrap();
+        assert!(e.outliner_select(&[gone, group], false));
+        assert!(e.status().contains("no instance i999"), "{}", e.status());
+        assert_eq!(context_ids(&e), [first]);
+    }
+
     #[test]
     fn close_instance_goes_back_out() {
         let mut e = editor();
         assert!(!e.close_instance());
```

Apply to `crates/app/src/ui.rs`:
```diff
--- a/crates/app/src/ui.rs
+++ b/crates/app/src/ui.rs
@@ -1,9 +1,11 @@
 //! The window's panels: toolbar, status bar with the measurements box, and on the right Entity
-//! Info and the Materials list.
+//! Info, the Materials list and the Outliner.
 
+use egui::collapsing_header::CollapsingState;
 use egui::{Align, Color32, Layout, Panel, RichText, Sense, Ui, Vec2};
-use session::{MaterialId, Projection, StandardView};
+use kernel::{DefinitionKind, EntityId};
+use session::{MaterialId, OutlinerNode, Projection, StandardView};
 use tools::ToolKind;
 
 use crate::editor::{MATERIAL_PALETTE, Status};
 
@@ -32,8 +34,16 @@ pub enum UiAction {
     AddMaterial,
     DeleteMaterial(MaterialId),
     RenameMaterial(MaterialId, String),
     RecolorMaterial(MaterialId, [u8; 3]),
+    /// Select the outliner node at `path` (instances from the root down to it); `toggle`
+    /// (Shift or Cmd/Ctrl held) adds or removes it instead of replacing the selection.
+    OutlinerSelect {
+        path: Vec<EntityId>,
+        toggle: bool,
+    },
+    /// Open the instances along `path` for editing (empty: back to the root).
+    OpenPath(Vec<EntityId>),
 }
 
 /// A material as the Materials list shows it.
 #[derive(Clone, Debug, PartialEq)]
@@ -63,8 +73,10 @@ pub struct View<'a> {
     /// Every material, in id order.
     pub materials: &'a [MaterialRow],
     /// The material the Paint tool paints with (None: the default material).
     pub material: Option<MaterialId>,
+    /// The tree of placed groups and components (`Document::outliner`).
+    pub outliner: &'a [OutlinerNode],
 }
 
 /// Lays out the panels for one frame. Returns the actions clicked and the rectangle left for
 /// the 3D view, in points.
@@ -203,8 +215,10 @@ pub fn draw(ui: &mut Ui, view: &View) -> (Vec<UiAction>, egui::Rect) {
                 ui.label(line);
             }
             ui.separator();
             materials(ui, view, &mut actions);
+            ui.separator();
+            outliner(ui, view, &mut actions);
         });
     (actions, ui.available_rect_before_wrap())
 }
 
@@ -287,8 +301,103 @@ fn materials(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
         }
     });
 }
 
+/// The Outliner: a Model row (back to the root) and the tree of groups and components, the
+/// selected ones highlighted and the open ones bold. A click selects a node, a double-click
+/// opens it.
+fn outliner(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
+    ui.heading("Outliner");
+    egui::ScrollArea::vertical()
+        .id_salt("outliner")
+        .auto_shrink([false, true])
+        .show(ui, |ui| {
+            let mut model = RichText::new("Model");
+            if !view.inside_instance {
+                model = model.strong();
+            }
+            if row_button(ui, false, model).clicked() {
+                actions.push(UiAction::OpenPath(Vec::new()));
+            }
+            let mut path = Vec::new();
+            for node in view.outliner {
+                outliner_node(ui, node, &mut path, actions);
+            }
+        });
+}
+
+/// One node and, when expanded, its children. `path` leads to the node's parent.
+fn outliner_node(
+    ui: &mut Ui,
+    node: &OutlinerNode,
+    path: &mut Vec<EntityId>,
+    actions: &mut Vec<UiAction>,
+) {
+    path.push(node.id);
+    if node.children.is_empty() {
+        let clicked = ui
+            .horizontal(|ui| {
+                // Lined up with the names of nodes that have an expand button.
+                ui.add_space(ui.spacing().indent);
+                outliner_row(ui, node, path)
+            })
+            .inner;
+        actions.extend(clicked);
+    } else {
+        let id = ui.make_persistent_id(("outliner", path.as_slice()));
+        // Nodes on the open path start expanded.
+        let (_, header, _) = CollapsingState::load_with_default_open(ui.ctx(), id, node.open)
+            .show_header(ui, |ui| outliner_row(ui, node, path))
+            .body(|ui| {
+                for child in &node.children {
+                    outliner_node(ui, child, path, actions);
+                }
+            });
+        actions.extend(header.inner);
+    }
+    path.pop();
+}
+
+/// A node's name and what clicking it asks for. `path` leads to the node.
+fn outliner_row(ui: &mut Ui, node: &OutlinerNode, path: &[EntityId]) -> Option<UiAction> {
+    let mut text = RichText::new(outliner_label(node));
+    if node.open {
+        text = text.strong();
+    }
+    let row = row_button(ui, node.selected, text);
+    if row.double_clicked() {
+        Some(UiAction::OpenPath(path.to_vec()))
+    } else if row.clicked() {
+        let toggle = ui.input(|i| i.modifiers.shift || i.modifiers.command);
+        Some(UiAction::OutlinerSelect {
+            path: path.to_vec(),
+            toggle,
+        })
+    } else {
+        None
+    }
+}
+
+/// A selectable row as wide as the space left, its text on the left (cut short if too long).
+fn row_button(ui: &mut Ui, selected: bool, text: RichText) -> egui::Response {
+    let width = ui.available_width();
+    ui.add(
+        egui::Button::selectable(selected, text)
+            .right_text("")
+            .truncate()
+            .min_size(Vec2::new(width, 0.0)),
+    )
+}
+
+/// How the Outliner names a node: a component as `<Name>`, a group by its name or as `Group`.
+fn outliner_label(node: &OutlinerNode) -> String {
+    match node.kind {
+        DefinitionKind::Component => format!("<{}>", node.name),
+        DefinitionKind::Group if node.name.is_empty() => "Group".to_string(),
+        DefinitionKind::Group => node.name.clone(),
+    }
+}
+
 #[cfg(test)]
 mod tests {
     use super::*;
     use egui::{Pos2, Rect, Vec2};
@@ -351,8 +460,9 @@ mod tests {
             has_guides: false,
             projection: Projection::Perspective,
             materials: &[],
             material: None,
+            outliner: &[],
         };
         let (_, central) = run(&ctx, screen(), &view);
         let file = Pos2::new(14.0, central.min.y / 2.0);
         let mut clicked = Vec::new();
@@ -431,8 +541,9 @@ mod tests {
             has_guides: false,
             projection: Projection::Perspective,
             materials: &[],
             material: None,
+            outliner: &[],
         };
         // The Camera menu sits right after File and Edit.
         let clicked = menu_actions(&view, 80..140);
         assert!(clicked.contains(&UiAction::ZoomExtents), "{clicked:?}");
@@ -466,8 +577,9 @@ mod tests {
             has_guides,
             projection: Projection::Perspective,
             materials: &[],
             material: None,
+            outliner: &[],
         };
         // The Edit button sits right after File (which ends before x = 40).
         menu_actions(&view, 40..80)
     }
@@ -501,8 +613,9 @@ mod tests {
             has_guides: false,
             projection: Projection::Perspective,
             materials: &[],
             material: None,
+            outliner: &[],
         };
         let (actions, rect) = run(&ctx, screen(), &view);
         assert!(actions.is_empty());
         assert!(
@@ -529,8 +642,9 @@ mod tests {
             has_guides: true,
             projection: Projection::Perspective,
             materials: &[],
             material: None,
+            outliner: &[],
         };
         let (_, central) = run(&ctx, screen(), &view);
         let row = central.min.y / 2.0;
         // Click along the toolbar row until a press lands on the Rectangle button.
@@ -627,8 +741,9 @@ mod tests {
             has_guides: false,
             projection: Projection::Perspective,
             materials: &rows,
             material: Some(MaterialId(31)),
+            outliner: &[],
         };
         // Down the panel's left part: swatches and names, Add, the name field, the palette.
         let clicked = panel_actions(&view, 1280.0 - 160.0, None);
         for action in [
@@ -683,8 +798,9 @@ mod tests {
             has_guides: false,
             projection: Projection::Perspective,
             materials: rows,
             material: Some(MaterialId(31)),
+            outliner: &[],
         }
     }
 
     fn brick() -> [MaterialRow; 1] {
@@ -784,5 +900,190 @@ mod tests {
                 .any(|a| matches!(a, UiAction::RenameMaterial(..))),
             "{actions:?}"
         );
     }
+
+    fn node(
+        id: &str,
+        kind: DefinitionKind,
+        name: &str,
+        children: Vec<OutlinerNode>,
+    ) -> OutlinerNode {
+        OutlinerNode {
+            id: id.parse().unwrap(),
+            definition: "d1".parse().unwrap(),
+            kind,
+            name: name.into(),
+            material: None,
+            open: false,
+            selected: false,
+            children,
+        }
+    }
+
+    /// Two "Box" components each placing the group i27; the first open, the group in it
+    /// selected.
+    fn boxes() -> Vec<OutlinerNode> {
+        let group = node("i27", DefinitionKind::Group, "", Vec::new());
+        let mut first = node("i55", DefinitionKind::Component, "Box", vec![group.clone()]);
+        first.open = true;
+        first.children[0].selected = true;
+        let second = node("i56", DefinitionKind::Component, "Box", vec![group]);
+        vec![first, second]
+    }
+
+    fn ids(ids: &[&str]) -> Vec<EntityId> {
+        ids.iter().map(|id| id.parse().unwrap()).collect()
+    }
+
+    fn outliner_view<'a>(
+        status: &'a Status,
+        info: &'a [String],
+        tree: &'a [OutlinerNode],
+    ) -> View<'a> {
+        View {
+            tool: ToolKind::Line,
+            status,
+            info,
+            has_selection: true,
+            has_instance_selected: true,
+            inside_instance: true,
+            has_guides: false,
+            projection: Projection::Perspective,
+            materials: &[],
+            material: None,
+            outliner: tree,
+        }
+    }
+
+    /// For each y down the right panel at `x` (every 4 points), the actions of one click
+    /// there with `modifiers` held, each in a fresh context (so no two make a double-click).
+    fn outliner_clicks(view: &View, x: f32, modifiers: egui::Modifiers) -> Vec<(f32, UiAction)> {
+        let mut found = Vec::new();
+        for y in (0..800).step_by(4) {
+            let ctx = egui::Context::default();
+            run(&ctx, screen(), view);
+            let pos = Pos2::new(x, y as f32);
+            let pointer = |pressed: bool| egui::Event::PointerButton {
+                pos,
+                button: egui::PointerButton::Primary,
+                pressed,
+                modifiers,
+            };
+            let mut press = screen();
+            press.events = vec![
+                egui::Event::ModifiersChanged(modifiers),
+                egui::Event::PointerMoved(pos),
+                pointer(true),
+            ];
+            let mut actions = run(&ctx, press, view).0;
+            let mut release = screen();
+            release.events = vec![pointer(false)];
+            actions.extend(run(&ctx, release, view).0);
+            found.extend(actions.into_iter().map(|a| (y as f32, a)));
+        }
+        found
+    }
+
+    #[test]
+    fn outliner_labels_name_groups_and_components() {
+        let [first, _] = <[OutlinerNode; 2]>::try_from(boxes()).unwrap();
+        assert_eq!(outliner_label(&first), "<Box>");
+        assert_eq!(outliner_label(&first.children[0]), "Group");
+        let named = node("i3", DefinitionKind::Group, "Wall", Vec::new());
+        assert_eq!(outliner_label(&named), "Wall");
+    }
+
+    #[test]
+    fn outliner_rows_select_their_paths_and_the_model_row_goes_back_to_the_root() {
+        let status = status();
+        let info = vec!["1 group".to_string()];
+        let tree = boxes();
+        let view = outliner_view(&status, &info, &tree);
+        let clicked: Vec<UiAction> = outliner_clicks(&view, 1280.0 - 100.0, Default::default())
+            .into_iter()
+            .map(|(_, a)| a)
+            .collect();
+        for action in [
+            UiAction::OpenPath(Vec::new()),
+            UiAction::OutlinerSelect {
+                path: ids(&["i55"]),
+                toggle: false,
+            },
+            UiAction::OutlinerSelect {
+                path: ids(&["i55", "i27"]),
+                toggle: false,
+            },
+            UiAction::OutlinerSelect {
+                path: ids(&["i56"]),
+                toggle: false,
+            },
+        ] {
+            assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
+        }
+        // The second box is not on the open path, so it starts collapsed.
+        assert!(
+            !clicked.iter().any(|a| matches!(
+                a,
+                UiAction::OutlinerSelect { path, .. } if *path == ids(&["i56", "i27"])
+            )),
+            "{clicked:?}"
+        );
+        let shifted = outliner_clicks(&view, 1280.0 - 100.0, egui::Modifiers::SHIFT);
+        assert!(
+            shifted.iter().any(|(_, a)| *a
+                == UiAction::OutlinerSelect {
+                    path: ids(&["i56"]),
+                    toggle: true,
+                }),
+            "{shifted:?}"
+        );
+    }
+
+    #[test]
+    fn double_clicking_an_outliner_row_opens_it_and_expanding_shows_children() {
+        let status = status();
+        let info = vec!["1 group".to_string()];
+        let tree = boxes();
+        let view = outliner_view(&status, &info, &tree);
+        let x = 1280.0 - 100.0;
+        let rows = outliner_clicks(&view, x, Default::default());
+        let y_of = |path: &[&str]| {
+            rows.iter()
+                .find(|(_, a)| {
+                    matches!(a, UiAction::OutlinerSelect { path: p, .. } if *p == ids(path))
+                })
+                .map(|(y, _)| *y)
+                .expect("found the row")
+        };
+        let ctx = egui::Context::default();
+        run(&ctx, screen(), &view);
+        let inner = Pos2::new(x, y_of(&["i55", "i27"]));
+        let mut actions = click(&ctx, &view, inner);
+        actions.extend(click(&ctx, &view, inner));
+        assert!(
+            actions.contains(&UiAction::OpenPath(ids(&["i55", "i27"]))),
+            "{actions:?}"
+        );
+        // The expand button sits left of the name: clicking it shows the second box's group.
+        let second = y_of(&["i56"]);
+        let (_, central) = run(&egui::Context::default(), screen(), &view);
+        let wanted = UiAction::OutlinerSelect {
+            path: ids(&["i56", "i27"]),
+            toggle: false,
+        };
+        let expanded = (0..40).step_by(2).any(|dx| {
+            let ctx = egui::Context::default();
+            run(&ctx, screen(), &view);
+            let button = click(&ctx, &view, Pos2::new(central.max.x + dx as f32, second));
+            // Wait out the opening animation, then click down the rows below.
+            for _ in 0..30 {
+                run(&ctx, screen(), &view);
+            }
+            button.is_empty()
+                && (1..16).any(|k| {
+                    click(&ctx, &view, Pos2::new(x, second + 4.0 * k as f32)).contains(&wanted)
+                })
+        });
+        assert!(expanded, "an expand button left of the second box");
+    }
 }
```

Apply to `docs/agent-api.md`:
```diff
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -262,8 +262,16 @@ open:
   context; the same ids under another placement are not marked.
 - To select a node elsewhere in the tree, open its parent's path, then select it:
   `open_path` with the path minus its last id, then `select`.
 
+In the window, the Outliner under the Materials list shows this tree (components as `<Name>`,
+groups by name or as `Group`, selected rows highlighted, open ones bold, nodes on the open path
+expanded at first) and sends these same commands: a click selects the node, with `open_path`
+first when it is in another context; Shift-click or Cmd/Ctrl-click adds it to or removes it from
+the selection when it is in the open context (`select` with mode `toggle`); a double-click opens
+it (`open_path` with its whole path); the Model row at the top goes back to the root
+(`open_path` with `[]`).
+
 `{"type": "rename_definition", "definition": "d5", "name": "Chair"}` renames a definition (one
 undo step; a component's name must not be empty; an unknown definition is `not_found`).
 `{"type": "purge_definitions"}` removes every definition no instance places, such as a component
 whose last instance was erased, and what only it placed (one undo step).
```

- [ ] **Step 4: Run the tests to verify they pass**

`cargo test -p app --bin app outliner` (6 passed, ~15 s: fresh contexts per row); `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace` -> 731 passed, 0 failed.

Expected workspace totals after each task: 725, 731.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/app/src/app.rs crates/app/src/editor.rs crates/app/src/ui.rs docs/agent-api.md
git commit -m "Outliner panel: the tree of groups and components under Materials; click selects (opening its context), Shift or Cmd/Ctrl toggles, double-click opens, Model row goes back to the root"
```

## Self-review notes

- **Spec coverage:** outliner query and open_path (Task 1), Outliner panel (Task 2).
- **Deferred** (from the scratch build):

- Instance names (kernel field + op, `rename_instance`, `.skr` schema 6, Entity Info and outliner editing).
- Filter box; sorting by name; renaming a definition from the outliner (double-click opens instead).
- Drag-and-drop reparenting; context menu (explode, make unique, hide); hide/lock state.
- Expanding the open path when it changes later (expand state is only defaulted on first show; a node collapsed by the user stays collapsed when the viewport double-click opens something inside it); scrolling to the selected node.
- Outliner tree rebuilt every frame (cache by revision, context and selection if large models show it); a component placed n times repeats its subtree n times.
- UI tests scan rows with a fresh egui context per y (about 15 s for the outliner tests in debug).
- No "selected elsewhere" cue: selecting by clicking in the viewport inside a component highlights only the node under the open path.

Split recommendation: keep two tasks. Task 1 (session + docs + spec) is self-contained and command-tested; Task 2 (editor + UI + window doc) depends only on `Document::outliner` and `Command::OpenPath`. A third task is not needed: the UI change is ~300 lines of which about half is tests.
