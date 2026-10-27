# M11d Interaction Conventions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The editor behaves as SketchUp users expect:
- Line, Rectangle, Circle, Arc and Push/Pull draw by dragging.
- Push/Pull can be retyped right after a pull and repeated by a double-click.
- Double- and triple-clicks select what borders the clicked entity, or everything connected to it.
- Cmd+A, Cmd+Shift+A and Cmd+D select all, select none and duplicate.
- Before Rotate's first click, the arrow keys lock its plane.
- The app starts in Select.
- Push/Pull, Paint and Erase say what they work on, and Move's idle prompt tells the truth.
- An empty model shows a start card with New, Open and the recent files.

**Architecture:**
- The tools crate does most of the work:
  - A small `DragStart` helper turns a release past the drag threshold into the second click.
  - Push/Pull remembers its last pull, and the last distance through `Remembered`.
  - `ToolEvent::TripleClick` is new.
  - `Tool::lock_plane` and `Tool::plane_lock` let Rotate take the arrow keys before its first click.
- The kernel gains `faces_of_edge` and `connected`.
- The session's `select` gains `all`.
- The app:
  - counts clicks up to three;
  - adds three bindings to the one shortcut table, so the menus, the native accelerators and the Keyboard Shortcuts window follow;
  - adds `Editor::{select_all, select_none, duplicate, triple_click, untouched}`;
  - keeps a recent-files list in prefs;
  - draws the start card as a bottom-left `egui::Area`.
- The API gains the `triple_click` event, the `duplicate` editor command and Cmd+A/Cmd+D keys in `tool_input`.
- The renderer is not touched.

**Tech Stack:** Rust 2024; egui, egui-winit, egui-wgpu, egui_kittest `=0.36.2`; wgpu `=30.0.1`; egui-phosphor `=0.14.0`; muda (native menus, as pinned); glam. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-10-tenon-design.md`, "Interface milestones", M11d (and M11b's Edit menu line, which names Select All and Select None).

The previous plans define what this one consumes:
- `docs/superpowers/plans/2026-10-17-m11c-viewport-feedback.md`: `Tool::hovered`, `ToolOutput::info`, the `covered` bit, the `Area` pattern of the gizmo and `ui::tests::drawn_texts`.
- `2026-10-17-m11b-layout.md`: the shortcut table, `menus::menu_bar`, `native_menu`, `shortcuts::window_command`, the rail and the dock.
- `2026-10-16-m11a-design-foundation.md`: theme tokens, prefs, the widget kit and the kittest harness.

The design audits (`interaction.md` F5-F8 and P5-P8, `ia.md` F7 and P5) only explain where values came from. They live in `/private/tmp/claude-501/-Users-mohn93-Desktop-side-projects-sketchup/7b020704-4344-4767-b2d2-50b34442d2ad/scratchpad/design/`. The executor reads the spec bullet and this plan.

## Global Constraints

- No emoji anywhere (code, comments, docs, commit messages). Keep comments succinct and in the style of the surrounding code.
- Before any cargo command, run `export PATH="$HOME/.cargo/bin:$PATH"`.
- After every task, these must pass:
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
- Test count:
  - The suite has 1311 tests at the start. Count them with `cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'`.
  - No task may leave fewer tests.
  - A test replaced by its migrated twin keeps the count.
  - A deleted test needs a replacement in the same task.
  - Before trusting a green run, grep the whole output for `FAILED` and `failed;`.
- Pinned versions: egui, egui-winit, egui-wgpu and egui_kittest `=0.36.2`; wgpu `=30.0.1`; egui-phosphor `=0.14.0`. Add no dependency.
- Values from the spec:
  - Drag to draw: Line, Rectangle, Circle, Arc and Push/Pull take a release more than the drag threshold from the press as the second click. The threshold is the existing one: `DRAG_LOGICAL_PX * scale_factor`, which reaches the tools as `ToolContext::drag_threshold`.
  - Push/Pull: a typed distance right after a pull redoes it at that distance; a double-click on a face repeats the last pull distance.
  - Select: a double-click on a face selects it with its edges; a double-click on an edge selects it with its faces; a triple-click selects everything connected; a double-click on an instance still opens it.
  - Cmd+A selects everything in the open context; Cmd+Shift+A selects nothing; `select` takes `all: true`.
  - Cmd+D duplicates the selection and starts Move on the copy (see Spec notes for what "in place" means here).
  - Rotate: before the first click, the arrow keys lock the protractor plane to an axis.
  - The app starts in Select.
  - Clicking empty space with Push/Pull, Paint or Erase says what the tool works on. Move's idle prompt describes what it does.
  - Empty-model card: 360 x 200 pt, bottom left of the view. It lists New, Open and the five most recent files, with a one-line hint for drawing a box. It can be dismissed and goes away at the first edit.
- Shortcuts:
  - Every new binding goes in `shortcuts::TABLE` and nowhere else.
  - On macOS, Cmd chords are native-menu-only (`shortcuts::window_command` returns None for them), so Select All, Select None and Duplicate are Edit menu items carrying the binding.
  - `with_native_menus_the_key_path_leaves_cmd_chords_to_the_menu` and `every_cmd_chord_is_a_native_menu_accelerator` must keep passing with the new rows: they are the no-double-fire proof.
- Idle stays at 0% CPU: no new `request_repaint`, `request_redraw` or `repaint_at` that is not caused by input. The card has no animation and no fade.
- Icons draw only in the "phosphor" family (`fonts::icons()`), through `widgets::icon_button`. Each new icon constant goes in `icons::all()`.
- egui 0.36.2 gotchas:
  - A selected button reports AccessKit `toggled` (use `ui::tests::is_on`).
  - A tooltip repeats its widget's name, so query before hovering.
  - `Window::anchor` makes a window immovable.
  - `ui::draw` appends actions on every pass, so anything applied from an action must be harmless twice. `DismissStart` and `OpenRecent` are harmless twice: `OpenRecent` runs `settle_changes` first and opening the same file twice gives the same model.
- Agent API parity: every new behaviour has a command or a `tool_input` path, and `docs/agent-api.md` is updated in Task 12.
- The renderer (`crates/render`) is not touched. CLASSIC renders stay byte-identical. Task 13 checks that `git diff --stat <base>..HEAD -- crates/render` is empty. Only if it is not does Task 13 run the two-worktree golden `cmp`.
- Prefs are window chrome. The API and headless mode never read or write them, and tests never write under `~/.tenon`.
- Git:
  - Work on `dev`. Other agents may share the checkout, so run `git status` and `git diff` first and stage only the files you changed. Never `git add -A` or `git add .`.
  - Short plain commit messages, no attribution lines. Never push or merge.
- Clippy denies dead code in the `app` binary, so an item lands in the task that first uses it. Public items of the library crates (`kernel`, `session`, `tools`, `api`) may land before their first user.
- Do not launch the app window in tests (no `cargo run` of the GUI).

## Review Focus

- **A double-click in Push/Pull whose two clicks are a few pixels apart.**
  - The second click must not commit a pull of a few centimeters before the repeat.
  - The double-click must make exactly one `push_pull`, at the last distance.
  - Task 5 test: `a_jittery_double_click_repeats_without_a_tiny_pull`.
- **A retyped distance after something else changed the model** (the user's undo, an agent's edit, another pull elsewhere).
  - The retype sends an Undo, so it must never undo a step other than the pull it redoes. It must explain itself instead.
  - Task 5 tests: `a_retype_after_an_undo_explains_instead_of_undoing`, `a_retype_after_an_agent_edit_undoes_nothing`.
- **Cmd+A while typing in a text field** (the Outliner filter, a material name).
  - It must select the text, not the model.
  - On macOS the native item is disabled while a field has focus, and a raced event does not act, as for Undo.
  - Task 9 tests: `select_all_and_none_yield_to_a_focused_text_field`, `native_menu_events_act_once_and_leave_typing_alone` (extended).
- **Navigation drags in a drawing tool**: a middle-button drag, Shift+middle, Alt+left.
  - They must orbit or pan and draw nothing.
  - A drag that starts on a panel and ends in the view must not draw either.
  - Task 4 tests: `navigation_drags_draw_nothing`, `a_release_whose_press_was_not_the_tools_draws_nothing`.
- **A triple-click whose double-click opened a group.**
  - The third click lands inside the group just opened. It must not select everything connected in there: the triple-click does nothing more than its click.
  - Task 6 test: `a_triple_click_after_opening_a_group_selects_only_the_click`.

## Spec notes (rulings made in this plan)

- **Drag to draw** (`tools::DragStart`):
  - Only a press that starts an operation is armed: the tool was idle before the click and active after it.
  - Its release is the operation's next click when the cursor is `drag_threshold` or more from the press (screen distance, physical pixels). The tool then handles the release exactly as a `Click` at that release's inference.
  - Any release disarms.
  - Effects per tool:
    - Line: a drag draws one edge, and the chain goes on from its end.
    - Rectangle: a drag gives the opposite corner.
    - Circle: a drag gives the rim.
    - Arc: a drag gives the end, and a later click the bulge.
    - Push/Pull: a drag sets the distance.
  - Later presses in an operation (a Line chain's second press, Arc's bulge click) are never armed, so dragging them does nothing more than clicking.
  - Move, Rotate and Tape Measure stay click-click: the spec lists five tools.
  - Navigation drags never reach the tools (the editor's `nav.button` takes them, and `tool_pressed` stays false), so they are untouched.
  - A drag release is not a click, so it never starts a double-click (`is_click` in `app.rs` already says so).
  - `tool_input`'s `drag` runs the same `Editor::button` calls, so it draws exactly as the window does.
- **Push/Pull retype.**
  - After a pull, the tool keeps `last: (face, distance)`.
  - A typed length while idle with a `last` runs `[Undo, PushPull { face, distance: d }]`. `d` keeps the last pull's direction: the typed value, negated when the last distance was negative. A typed negative value reverses it.
  - `last` keeps the face id as it was before the pull: undo restores ids exactly, so the second command finds the face.
  - `last` is cleared by `on_external_change` (the user's undo or redo, an agent's edit), by Escape, by a new pull's first click, and by a double-click repeat.
  - A typed value with no `last` keeps today's message, "Click a face first".
  - Idle prompt with a `last`: "Type a distance to change the pull, or click a face".
- **Push/Pull double-click repeat.**
  - The last pull distance lives in `Remembered::pull_distance` (session-wide, like circle sides), so it survives switching tools.
  - A `DoubleClick` acts on the face of the pull in progress, if the double-click's first press started one, else on the open context's face under the cursor. It drops that drag and runs `PushPull { face, distance: pull_distance }`.
  - With no `pull_distance` it says, as info: "Pull a face first; a double-click then repeats that distance". It says the same as info over no face: "Push/Pull works on faces: double-click a face to repeat the last pull".
  - A click less than `drag_threshold` from the click that started the drag never finishes it, so a double-click's second press cannot pull a few pixels. That press is a no-op.
- **Selection clicks** (Select tool):
  - Double-click on an open-context face: the face and `Model::face_edges(face)`.
  - Double-click on an open-context edge: the edge's whole curve (as a click selects) and `Model::faces_of_edge` of each of its edges.
  - Double-click on a guide selects the guide.
  - Triple-click on an edge or face: `Model::connected(seed)`, every edge and face joined to it through shared vertices.
  - Both replace the selection. With Shift they add to it.
  - The click's own Release (which selects the clicked entity) arrives first and stays in the event order. The double- or triple-click then widens it.
  - Double-click on an instance opens it, and double-click on nothing while inside one closes it, as today.
  - A `TripleClick` right after a `DoubleClick` that opened or closed an instance does nothing more.
  - Triple-click on anything else (instance, guide, nothing) does nothing more than its click.
  - The window counts clicks: a release within 500 ms and 6 logical px of the last click's release adds one, up to three. A fourth starts over at one. The second sends `double_click`, the third `triple_click`.
- **Select All / Select None.**
  - `select` gains `all: bool` (default false, not serialized when false): every edge (soft ones included), face, instance and guide of the open context, combined with `mode` as `ids` would be.
  - `ids` gets `#[serde(default)]`, so `{"type": "select", "all": true}` is complete.
  - `all: true` with non-empty `ids` is `invalid_input` ("give ids or all, not both").
  - `all` with a `box` is refused as an unknown field by the `box` form, as any other key is.
  - Select None is `select` with `ids: []`, which exists already.
  - Neither changes the tool or ends its operation.
- **Duplicate (Cmd+D)** maps to Move in copy mode on the selection, not to a `copy` with a zero offset:
  - The session refuses a zero-offset copy (`ZeroDistance`).
  - A loose copy in place would merge into the original (sticky geometry), so "Move on the copy" would move the original.
  - The behaviour of `Editor::duplicate`:
    - With nothing selected: the error message "Select geometry or a group to duplicate", and nothing else. Agents get `invalid_input`.
    - Otherwise it switches to Move, taps copy on (`ToggleCopy`), works the hover out, and, when the cursor is in the view under no window, clicks the base point there. The copy then follows the cursor.
    - The next click or a typed distance makes one `copy` command: one undo step. The original stays; Escape leaves no copy.
    - With the cursor outside the view (the menu clicked with the mouse), Move waits for a base-point click in copy mode.
  - The result after the second click is what "copy in place, then move the copy" gives, in one step instead of two. `5x` and `/5` arrays work after it as after any Move copy.
  - API: the editor command `{"type": "duplicate"}` and the `tool_input` key `d` with `cmd`.
- **Shortcut table rows** (group "Edit"):
  - `Bound::SelectAll`, Cmd+A, label "Select All".
  - `Bound::SelectNone`, Cmd+Shift+A, label "Select None".
  - `Bound::Duplicate`, Cmd+D, label "Duplicate".
  - None of them repeats.
  - The plain `a` (Arc) is unaffected.
- **Edit menu**, in order: Undo, Redo, separator, Select All, Select None, Duplicate, separator, Erase, Delete Guides, separator, Soften Edges...
  - Select All is always enabled.
  - Select None and Duplicate are enabled with a selection.
- **Text fields.** Select All and Select None yield to a focused text field, as Undo and Redo do:
  - `native_enabled` turns them off while `text_focused`.
  - `native_event_acts` refuses a raced event.
  - Off macOS the window's key path never sees keys while egui wants the keyboard.
  - Duplicate does not yield (a text field has no Cmd+D).
  - The chrome paragraph in the docs says so.
- **Rotate plane lock.**
  - New trait methods: `Tool::lock_plane(&mut self, axis: Option<Axis>) -> bool` (default false) and `Tool::plane_lock(&self) -> Option<Axis>` (default None).
  - The editor's `LockAxis(axis)` with no anchor calls `lock_plane`:
    - the same axis again unlocks (`lock_plane(None)`);
    - `Unlock` (Down) calls `lock_plane(None)`.
  - Rotate takes them only while idle (no `turn`).
  - A locked axis replaces `plane_normal` at the center click.
  - While idle and locked, the preview draws the protractor circle (24 segments, radius `PROTRACTOR * eye_distance`) at the cursor in the locked plane.
  - The lock lasts across rotations until it is unlocked, Escape (`Cancel`, which also ends a turn), or the tool is switched. `on_external_change` keeps it: it is not about geometry.
  - The status bar's lock shows it with the existing texts ("Locked: red axis" and so on), so `editor.lock` reports it.
  - After the first click the arrows keep today's meaning (a direction lock for the angle's points).
- **No silent dead ends** (info messages, set only when the click hits nothing the tool works on):
  - Push/Pull on no face: "Push/Pull works on faces: click a face to push or pull it".
  - Push/Pull on a face of another context: "Open the group to push or pull its faces".
  - Paint on nothing: "Paint works on faces and groups: click one to paint it".
  - Erase on nothing: "Erase works on edges, faces, guides and groups: click one to erase it".
- **Move's idle prompts** follow whether something is selected, as the tool last saw the selection on a Move event:
  - With a selection: "Click a base point to move the selection (Ctrl copies)".
  - With a selection, copying: "Copy: click a base point to copy the selection".
  - Without: "Click an edge, face or group to pick it up, or select first (Ctrl copies)".
  - Without, copying: "Copy: click an edge, face or group to pick it up, or select first".
  - Rotate's two idle prompts get the same treatment, plus the arrow keys:
    - With a selection: "Click the center to turn the selection; arrows lock the axis (Ctrl copies)".
    - Without: "Click the center on an edge, face or group to turn it; arrows lock the axis".
    - Copying, with a selection: "Copy: click the center to turn the selection; arrows lock the axis".
    - Copying, without: "Copy: click the center on an edge, face or group; arrows lock the axis".
- **Startup.**
  - `Editor::new` starts in `ToolKind::Select`, in the window and headless.
  - Tests that relied on Line at start get an explicit `select_tool(ToolKind::Line)` (or `tool(&mut e, "line")`) at their top. None is deleted or weakened.
- **Recent files** (prefs, window chrome):
  - M11d adds the minimal list that M11e's Open Recent menu reuses: `Prefs::recent: Vec<PathBuf>`, newest first, at most `RECENT_LEN = 10` entries.
  - The window records a file after a successful Open (dialog, card, or the FILE argument) and after a successful Save or Save As, as `std::path::absolute` of `doc.path()`. It moves to the front with duplicates removed.
  - Files agents open or save through the API are not recorded.
  - Loading drops relative paths and duplicates and keeps the first 10.
  - The card shows the first `CARD_RECENT = 5` entries that are regular files when the list is built: at startup and after each change, never per frame.
  - A recent file that fails to open shows the open error in the status bar and stays in the list. M11e may add clearing.
- **Empty-model card** (`ui::start_card`):
  - Shown when all of these hold:
    - `Editor::untouched()`: the root is open, the root mesh has no vertices, instances or guides, and there is nothing to undo or redo;
    - the card was not dismissed this session;
    - not flying.
  - So it hides at the first edit (the history is no longer empty), comes back after File > New, and stays hidden after Open of a non-empty file. Dismissing hides it for the rest of the session.
  - An `egui::Area` (`Order::Middle`, not movable), 360 x 200 pt. Its bottom-left corner is inset 12 pt from the view's. It is drawn with `Frame` fill `bg_raised`, a 1 pt `border` stroke, radius `WINDOW_RADIUS`, and 12 pt padding. Being an Area, it keeps its presses from the tools.
  - Contents, top to bottom:
    - a "Dismiss" icon button (`icons::X`) at the top right;
    - "New" and "Open..." buttons in a row, reporting `UiAction::NewModel` and `UiAction::OpenFile`;
    - the section header "Recent";
    - up to five rows, each a button named by the file name with its full path as the hover text, reporting `UiAction::OpenRecent(path)`. With none, "No recent files" in `text_secondary`;
    - at the bottom, the hint "R: drag a rectangle.  P: pull it up into a box." in 11 pt `text_secondary`, on one line.
  - No fade and no repaint of its own.
  - API: the card's buttons are `new` and `open`; the card itself and its dismissal are chrome.
- **Tests.**
  - Tool behaviour is tested in `crates/tools`.
  - Window flows are tested through the `Editor` and `tool_input` (`crates/app/src/host/tool_input_tests.rs`).
  - Panels are tested with the kittest harness (`crates/app/src/ui/tests/`).
  - The final task looks at the real window through a `.app` bundle wrapper.

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/kernel/src/connected.rs` (new), `lib.rs`; `crates/kernel/tests/connected.rs` (new) | `faces_of_edge`, `connected` | 1 |
| `crates/session/src/command.rs`, `document.rs`; `crates/session/tests/selection.rs`; every `Command::Select {` site | `select` `all` | 2 |
| `crates/app/src/editor.rs` and the test helpers that build an `Editor` | start in Select; test migration | 3 |
| `crates/tools/src/tool.rs`, `line.rs`, `rectangle.rs`, `circle.rs`, `arc.rs`, `push_pull.rs` | `DragStart`, drag to draw | 4 |
| `crates/tools/src/push_pull.rs`, `tool.rs` (`Remembered::pull_distance`) | retype, double-click repeat, jitter guard | 5 |
| `crates/tools/src/tool.rs`, `select.rs` and every tool's `handle`; `crates/app/src/editor.rs`, `editor/input.rs`, `app.rs`; `crates/api/src/input.rs` | `TripleClick`, double/triple selection, click counting, `triple_click` event | 6 |
| `crates/tools/src/tool.rs`, `rotate.rs`; `crates/app/src/editor.rs` | Rotate plane lock | 7 |
| `crates/tools/src/push_pull.rs`, `paint.rs`, `erase.rs`, `move_tool.rs`, `rotate.rs` | dead-end messages, honest prompts | 8 |
| `crates/app/src/shortcuts.rs`, `menus.rs`, `ui/mod.rs`, `app.rs`, `editor.rs`, `editor/input.rs`, `host.rs`; `crates/api/src/editor.rs`, `input.rs`; `crates/app/src/ui/tests/menus.rs` | Select All/None, Duplicate | 9 |
| `crates/app/src/prefs.rs`, `app.rs` | recent files | 10 |
| `crates/app/src/ui/start.rs` (new), `ui/mod.rs`, `ui/icons.rs`, `ui/theme.rs`, `ui/tests/start.rs` (new), `ui/tests/mod.rs`, `editor.rs`, `app.rs` | empty-model card | 11 |
| `docs/agent-api.md` | API docs | 12 |
| `docs/superpowers/roadmap.md` | M11d row | 13 |

---

### Task 1: The kernel finds an edge's faces and what is connected

**Files:**
- Create: `crates/kernel/src/connected.rs`, `crates/kernel/tests/connected.rs`
- Modify: `crates/kernel/src/lib.rs` (`mod connected;`)

**Interfaces:**
- Produces:

```rust
impl Model {
    /// The faces with a loop side along edge `id` (either way round), in id order; empty for
    /// an unknown edge.
    pub fn faces_of_edge(&self, id: EdgeId) -> Vec<FaceId>;
    /// Every edge and face joined to `seed` (an edge or a face) through shared vertices:
    /// edges in id order, then faces in id order, `seed` among them. Empty for an unknown id,
    /// a vertex, an instance or a guide.
    pub fn connected(&self, seed: EntityId) -> Vec<EntityId>;
}
```

- [ ] **Step 1: Record the baseline**

```bash
cd ~/Desktop/side_projects/sketchup && export PATH="$HOME/.cargo/bin:$PATH"
git status && cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'
```
Expected: a clean tree on `dev` and 1311.

- [ ] **Step 2: Write the failing tests** (`crates/kernel/tests/connected.rs`)

- `an_edge_knows_the_faces_along_it`:
  - Build a 1 m box: a rectangle, then `push_pull` it 1 m.
  - A vertical side edge has exactly two faces, and both contain it in `face_edges`.
  - An edge drawn alone with `add_edge` has none.
  - An unknown `EdgeId(999_999)` gives an empty list.
- `connected_spreads_through_shared_vertices_only`:
  - Build the box, plus a separate square 5 m away, plus a lone edge touching the box at one corner.
  - From a box face: the 12 box edges, the lone edge, and the 6 box faces. That is 13 edges and 6 faces, none of the square's.
  - From the square's face: its 4 edges and its 1 face.
  - The result is sorted (edges then faces, each in id order) and contains the seed.
  - A guide id, an instance id and an unknown id give an empty list.

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test -p kernel --test connected`
Expected: compile errors (no `faces_of_edge`, no `connected`).

- [ ] **Step 4: Implement**

- `faces_of_edge` is `faces_on_side(a, b)` for the edge's two vertices.
- `connected` is a breadth-first walk over vertices:
  - Seed vertices: an edge's two, or every vertex of a face's loops.
  - At each vertex, take `edges_at`; each edge's other vertex joins the walk.
  - The faces are `faces_at` of every vertex reached.
  - Use `BTreeSet`s for both outputs.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p kernel`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git status && git diff --stat
git add crates/kernel/src/connected.rs crates/kernel/src/lib.rs crates/kernel/tests/connected.rs
git commit -m "Kernel: faces of an edge and connected geometry"
```

---

### Task 2: `select` takes `all: true`

**Files:**
- Modify: `crates/session/src/command.rs` (`Select`), `crates/session/src/document.rs` (the `Select` arm)
- Modify: every other `Command::Select {` construction site. Today they are in `crates/tools/src/select.rs`, `crates/app/src/editor.rs`, `info.rs`, `editor/texture_position.rs`, `editor/soften.rs`, `crates/render/tests/golden.rs`, `crates/render/tests/perf.rs`, `crates/api/tests/hub.rs` and `crates/api/src/dispatch.rs`. Each gains `all: false`. This is test-only or data-only for render: no renderer source changes.
- Test: `crates/session/tests/selection.rs`, `crates/api/tests/dispatch.rs`

**Interfaces:**
- Produces:

```rust
/// Change the selection of edges, faces, instances and guides. Not recorded in undo history.
Select {
    #[serde(default)]
    ids: Vec<EntityId>,
    #[serde(default)]
    mode: SelectMode,
    /// Everything in the open context (edges, faces, instances and guides) in place of
    /// `ids`, combined by `mode` the same way.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    all: bool,
},
```

- [ ] **Step 1: Write the failing tests**

In `crates/session/tests/selection.rs`:
- `select_all_takes_everything_in_the_open_context`:
  - Make a rectangle, a guide point and a group made of a second rectangle.
  - `Select { ids: vec![], mode: Replace, all: true }` selects every edge, face, guide and the instance of the root, and nothing inside the group. Compare with the set built from `model().edges()`, `faces()`, `instances()` and `guides()`.
  - After `OpenInstance` on the group, `all` selects only the group's own entities.
  - `mode: Remove, all: true` empties the selection.
- `select_all_with_ids_is_refused`: `Select { ids: vec![some_edge], all: true, .. }` is `InvalidInput`, with a message containing "ids or all", and the selection is unchanged.
- `select_all_reads_from_json`:
  - `serde_json::from_value::<Command>(json!({"type": "select", "all": true}))` gives `all: true` with empty ids.
  - Serializing a `Select` with `all: false` has no `"all"` key.

In `crates/api/tests/dispatch.rs`, `select_all_runs_and_box_refuses_all`:
- `execute {"type": "select", "all": true}` returns ok, and the `selection` query lists every edge and face.
- `{"type": "select", "box": {...}, "all": true}` is `-32602`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p session --test selection select_all && cargo test -p api --test dispatch select_all`
Expected: compile errors.

- [ ] **Step 3: Implement**

- The `Select` arm: when `all`, refuse non-empty `ids`, then take the ids from `self.model()`'s edges, faces, instances and guides. Skip `self.parts` for them: they exist by construction.
- Add `all: false` at every construction site. `cargo build --workspace --all-targets` lists them.

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: PASS. JSON comparisons of recorded `select` commands are unchanged, since `all: false` is not serialized.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/session/src/command.rs crates/session/src/document.rs crates/session/tests/selection.rs crates/api/tests/dispatch.rs <each construction-site file changed>
git commit -m "Session: select all in the open context"
```

---

### Task 3: The app starts in Select

**Files:**
- Modify: `crates/app/src/editor.rs` (`Editor::new`)
- Modify (tests only): whichever tests relied on Line at start, in `crates/app/src/editor.rs`, `editor/agent.rs`, `editor/soften.rs`, `editor/texture_position.rs`, `host.rs`, `host/tool_input_tests.rs`, `app.rs`, `crates/app/tests/headless_agent.rs`, `headless_stdio.rs`, `crates/api/tests/*`

**Interfaces:**
- Produces: `Editor::new(..).tool_kind() == ToolKind::Select`.

- [ ] **Step 1: Write the failing test** (`editor.rs` tests)

```rust
#[test]
fn the_editor_starts_in_select() {
    let e = editor();
    assert_eq!(e.tool_kind(), ToolKind::Select);
    assert_eq!(e.status_parts().tool, "Select");
}
```
In `tool_input_tests.rs`, `a_fresh_editor_reports_the_select_tool`: `ask(&mut e, json!({"type": "editor"}))["tool"] == "select"`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app starts_in_select fresh_editor_reports`
Expected: FAIL (`Line`).

- [ ] **Step 3: Implement and migrate**

- `Editor::new`: `tool: ToolKind::Select.make()`, `kind: ToolKind::Select`.
- Run `cargo test --workspace 2>&1 | grep -E "FAILED|panicked"`. For each failing test, add `e.select_tool(ToolKind::Line);` (or `tool(&mut e, "line")`, or a `set_tool` request in the headless tests) right after the editor is made, before anything else.
  - Do not change any assertion. A test asserting that the tool is back to Line after an operation (for example `"the tool is back"`) keeps asserting Line, because it now selects Line first.
- A helper used by many tests may select Line itself, but only if every caller relied on Line. Otherwise migrate per test.

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'`
Expected: every test passes, and the count is 1313 (grep for `FAILED` too).

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/editor.rs <each migrated test file>
git commit -m "Editor: start in Select"
```

---

### Task 4: Drag to draw

**Files:**
- Modify: `crates/tools/src/tool.rs` (`DragStart`), `line.rs`, `rectangle.rs`, `circle.rs`, `arc.rs`, `push_pull.rs`
- Test: `crates/tools/src/tool.rs`, `crates/app/src/host/tool_input_tests.rs`

**Interfaces:**
- Produces (crate-private in `tools::tool`):

```rust
/// Drag to draw: where the press that started an operation went down, until its release.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DragStart(Option<DVec2>);
impl DragStart {
    /// Remembers the press at `cursor` (call when a click started the operation).
    pub(crate) fn arm(&mut self, cursor: DVec2);
    /// Whether this release is the operation's next click: a press was armed and the cursor
    /// is now `ctx.drag_threshold` or more from it. Always disarms.
    pub(crate) fn released(&mut self, ctx: &ToolContext) -> bool;
}
```
- Each of the five tools has a `drag_start: DragStart` field.
  - In `Click`: note `let idle = self.phase() == Phase::Idle;` first, then arm after handling if the tool is now active.
  - `Release(hit)`: `if self.drag_start.released(ctx) { return self.handle(ToolEvent::Click(hit), ctx); }`. The preview code after the match still runs inside the recursive call.
  - `Cancel` and `on_external_change` disarm.

- [ ] **Step 1: Write the failing tests**

In `tool.rs` tests, beside `phases`, add a helper with this doc comment:

```rust
/// A fresh `kind` tool over the selected 2 m square of `phases`: pressed over `from`, the
/// cursor moved to `to`, released there. Returns every command and the phase after.
fn dragged(kind: ToolKind, from: DVec3, to: DVec3) -> (Vec<Command>, Phase);
```

```rust
#[test]
fn a_release_past_the_drag_threshold_is_the_second_click() {
    let (a, b) = (DVec3::new(0.5, 0.5, 0.0), DVec3::new(1.5, 1.2, 0.0));
    let (cmds, phase) = dragged(ToolKind::Rectangle, a, b);
    assert!(matches!(cmds[..], [Command::AddRectangle { .. }]), "{cmds:?}");
    assert_eq!(phase, Phase::Idle);
    let (cmds, _) = dragged(ToolKind::Circle, a, b);
    assert!(matches!(cmds[..], [Command::AddCircle { .. }]), "{cmds:?}");
    // Line: one edge, and the chain goes on from its end.
    let (cmds, phase) = dragged(ToolKind::Line, a, b);
    assert!(matches!(cmds[..], [Command::AddEdge { .. }]), "{cmds:?}");
    assert!(matches!(phase, Phase::Active { .. }));
    // Arc: the drag gives the end; the bulge is still to come.
    let (cmds, phase) = dragged(ToolKind::Arc, a, b);
    assert!(cmds.is_empty());
    assert!(matches!(phase, Phase::Active { ref points } if points.len() == 2));
    // Push/Pull: dragged up off the face, it pulls.
    let (cmds, _) = dragged(ToolKind::PushPull, DVec3::new(1.0, 1.0, 0.0), DVec3::new(1.0, 1.0, 0.8));
    assert!(matches!(cmds[..], [Command::PushPull { distance, .. }] if distance > 0.1), "{cmds:?}");
}

#[test]
fn a_release_near_the_press_waits_for_the_next_click() {
    // 1 mm on the ground is well under 4 px from the default camera.
    let a = DVec3::new(0.5, 0.5, 0.0);
    for kind in [ToolKind::Line, ToolKind::Rectangle, ToolKind::Circle, ToolKind::Arc] {
        let (cmds, phase) = dragged(kind, a, a + DVec3::new(0.001, 0.0, 0.0));
        assert!(cmds.is_empty(), "{kind:?}");
        assert!(matches!(phase, Phase::Active { .. }), "{kind:?}");
    }
}

#[test]
fn only_the_press_that_starts_an_operation_drags() {
    // Line: click a, click b (edge a-b), press at c (edge b-c: a press in a chain is a
    // click), drag and release at d far away. The release adds nothing: two AddEdge in all,
    // and the chain goes on from c (`phase` points end at c).
}
```
`dragged` builds the context as `phases` does, with `cursor` at `camera.project(point)`. It sends `Click(at from)`, then `Move(at to)`, then `Release(at to)`, each with an `OnFace` inference on the square for points on it, and `Free` otherwise.

In `tool_input_tests.rs`:
- `a_drag_draws_like_two_clicks`:
  - Rectangle by `drag` points `[[0,0,0],[2,3,0]]` makes an `add_rectangle` from (0,0,0) to (2,3,0).
  - Then Push/Pull by `drag` from `[1,1,0]` to `[1,1,1]` makes one `push_pull` whose distance is within 1e-6 of 1.0 (the snapped height).
  - `editor.phase` is `idle` after each.
- `navigation_drags_draw_nothing`:
  - With Rectangle, a middle `drag`, a middle `drag` under `{"type": "modifiers", "shift": true}`, and a left `drag` under `{"type": "modifiers", "alt": true}` run no commands and leave `phase` `idle`.
  - Then let go of the modifiers.
- `a_release_whose_press_was_not_the_tools_draws_nothing`:
  - Hold a Rectangle press with a `down`, cancel it, then send an `up` far away. No command runs.
  - Then a fresh `down`/`up` pair 1 px apart starts a rectangle and does not finish it.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p tools drag && cargo test -p app a_drag_draws navigation_drags release_whose_press`
Expected: the tools tests fail (releases ignored); the app navigation tests may already pass, as they guard today's behaviour.

- [ ] **Step 3: Implement** `DragStart` and the five tools' `Release` arms as in Interfaces. Update each module's doc comment to say a drag works as two clicks.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p tools && cargo test -p app`
Expected: PASS. `phases` (click then release in one place) is unchanged.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/tools/src/tool.rs crates/tools/src/line.rs crates/tools/src/rectangle.rs crates/tools/src/circle.rs crates/tools/src/arc.rs crates/tools/src/push_pull.rs crates/app/src/host/tool_input_tests.rs
git commit -m "Tools: drag to draw for Line, Rectangle, Circle, Arc and Push/Pull"
```

---

### Task 5: Push/Pull retype and double-click repeat

**Files:**
- Modify: `crates/tools/src/push_pull.rs`, `crates/tools/src/tool.rs` (`Remembered`)
- Test: `crates/tools/src/push_pull.rs`, `crates/app/src/host/tool_input_tests.rs`

**Interfaces:**
- Consumes: `DragStart` (Task 4).
- Produces:
  - `Remembered` gains `pub pull_distance: Option<f64>` (default None) and drops `Eq` (keeps `PartialEq`).
  - `PushPullTool::with_distance(d: Option<f64>) -> Self`, used by `make_with`.
  - `Tool::remember` for Push/Pull writes `pull_distance`.
  - `PushPullTool` fields:
    - `last: Option<(FaceId, f64)>`: the pull a typed value redoes.
    - `repeat: Option<f64>`: the distance a double-click repeats.
    - `Drag.press: DVec2`: where the drag's first click was, on screen.

- [ ] **Step 1: Write the failing tests** (`push_pull.rs`, with the module's `Rig`)

```rust
#[test]
fn a_typed_distance_after_a_pull_redoes_it() {
    let mut r = Rig::new();
    r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
    r.at(DVec3::new(1.0, 1.0, 1.0), ToolEvent::Move);
    let first = r.at(DVec3::new(1.0, 1.0, 1.0), ToolEvent::Click);
    assert!(matches!(first.commands[..], [Command::PushPull { .. }]));
    assert_eq!(r.tool.prompt(), "Type a distance to change the pull, or click a face");
    let out = r.tool.handle(ToolEvent::Value("2.5".into()), &r.ctx());
    assert_eq!(out.commands, vec![
        Command::Undo { count: None },
        Command::PushPull { face: EntityId::Face(r.face), distance: 2.5 },
    ]);
    // A pull down keeps its direction; a negative value reverses it.
    // (Pull to z = -1, retype "2" gives -2; retype "-2" gives 2.)
}

#[test]
fn a_double_click_repeats_the_last_pull_distance() {
    let mut r = Rig::with_distance(Some(1.5));
    r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
    r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Release);
    r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
    r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Release);
    let out = r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::DoubleClick);
    assert_eq!(out.commands, vec![Command::PushPull { face: EntityId::Face(r.face), distance: 1.5 }]);
    assert_eq!(r.tool.phase(), Phase::Idle);
    // Without a distance to repeat, it explains (info) and pulls nothing.
    let mut r = Rig::new();
    let out = r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::DoubleClick);
    assert!(out.commands.is_empty() && out.info);
    assert_eq!(out.message.as_deref(), Some("Pull a face first; a double-click then repeats that distance"));
}

#[test]
fn a_jittery_double_click_repeats_without_a_tiny_pull() {
    let mut r = Rig::with_distance(Some(1.0));
    let p = DVec3::new(1.0, 1.0, 0.0);
    r.at(p, ToolEvent::Click);
    r.at(p, ToolEvent::Release);
    // The second press lands 3 px higher on screen (under the 4 px threshold).
    r.cursor_offset(DVec2::new(0.0, -3.0));
    let second = r.send(ToolEvent::Click(r.hit(p)));
    assert!(second.commands.is_empty(), "no pull of a few pixels: {:?}", second.commands);
    r.send(ToolEvent::Release(r.hit(p)));
    let out = r.send(ToolEvent::DoubleClick(r.hit(p)));
    assert_eq!(out.commands, vec![Command::PushPull { face: EntityId::Face(r.face), distance: 1.0 }]);
}

#[test]
fn a_retype_after_an_undo_explains_instead_of_undoing() {
    // Pull, then on_external_change (what the editor's undo calls), then type "2":
    // no commands, message "Click a face first".
}

#[test]
fn the_pull_distance_is_remembered_across_tools() {
    // Pull 1.25 with a fresh tool; `remember` writes Some(1.25) into a Remembered; a tool
    // made with it repeats 1.25 on a double-click.
}
```
`Rig::with_distance`, `Rig::ctx`, `Rig::send`, `Rig::hit` and `Rig::cursor_offset` are small additions to the module's rig.

In `tool_input_tests.rs`:
- `a_typed_distance_after_a_pull_redoes_it_in_one_step`:
  - `square`, then `push_pull` with `click` `[1,1,0]`, `click` `[1,1,1]`, then `type` "2".
  - The commands are `push_pull`, `undo`, `push_pull`. The history has exactly one `push_pull` step (`history` query). `entities` reports a vertex at z = 2.
- `a_double_click_repeats_the_last_pull`:
  - Pull one square by typing "1".
  - `add_rectangle` a second square at `[5,0,0]-[7,2,0]`.
  - `double_click` at `[6,1,0]` runs exactly one `push_pull` with distance 1 on the second face.
- `a_retype_after_an_agent_edit_undoes_nothing`:
  - Pull, then `execute` an `add_edge` elsewhere, then `type` "2".
  - No `undo` runs, the edge is still there, and `editor.message` is "Click a face first".

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p tools push_pull && cargo test -p app redoes_it double_click_repeats retype_after`
Expected: FAIL.

- [ ] **Step 3: Implement** the Spec notes' Push/Pull retype and repeat rules.

- `finish` sets `last` and `repeat`. It records `last` with the drag's face, before the command runs.
- `Click` with a drag ignores a click within `drag_threshold` of `drag.press`.
- `Value` while idle with `last` redoes. `Cancel` and `on_external_change` clear `last` and keep `repeat`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p tools && cargo test -p app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/tools/src/push_pull.rs crates/tools/src/tool.rs crates/app/src/host/tool_input_tests.rs
git commit -m "Push/Pull: retype after a pull, double-click repeats it"
```

---

### Task 6: Double- and triple-click selection

**Files:**
- Modify: `crates/tools/src/tool.rs` (`ToolEvent::TripleClick`), `select.rs`, and the `handle` match of every other tool (`TripleClick` joins the arm that ignores `DoubleClick`; Push/Pull's ignores it too)
- Modify: `crates/app/src/editor.rs` (`triple_click`), `crates/app/src/editor/input.rs`, `crates/app/src/app.rs` (click counting)
- Modify: `crates/api/src/input.rs` (`InputEvent::TripleClick`, `Input::TripleClick`)
- Test: `crates/tools/src/select.rs`, `crates/app/src/editor.rs`, `crates/app/src/app.rs`, `crates/app/src/host/tool_input_tests.rs`, `crates/api/src/input.rs`

**Interfaces:**
- Consumes: `Model::face_edges`, `Model::faces_of_edge`, `Model::connected` (Task 1).
- Produces:
  - `ToolEvent::TripleClick(Inference)`: "The primary button was clicked a third time in quick succession (sent after the third release; the second sent `DoubleClick`)."
  - `Editor::triple_click(&mut self) -> bool`, the twin of `double_click`.
  - In `app.rs`, `click_count` replaces `is_double_click`, and `State::last_release` becomes `clicks: Option<(Instant, DVec2, u8)>`:

    ```rust
    /// How many clicks a release at `at` (physical pixels) at `now` makes, counting `last` (the
    /// previous click's time, place and count): one more when within `DOUBLE_CLICK` and `max_px`
    /// of it, up to three; else, and after three, one.
    fn click_count(last: Option<(Instant, DVec2, u8)>, now: Instant, at: DVec2, max_px: f64) -> u8;
    ```
  - In the API, `InputEvent::TripleClick { pixel, point }` ("Three left clicks in one place, then the double- and triple-click") and `Input::TripleClick(Spot)`.

- [ ] **Step 1: Write the failing tests**

In `select.rs` (the module's `Rig`, extended with a 1 m box beside the square for connection tests; give the box its own `Rig::boxed()`):
- `a_double_click_on_a_face_selects_it_with_its_edges`: the commands are `[Select { ids: face + its 4 edges (sorted), mode: Replace, all: false }]`.
- `a_double_click_on_an_edge_selects_it_with_its_faces`:
  - On a box's vertical edge: the edge and its 2 faces.
  - On a curve's edge (an `add_circle` in the rig), the whole curve and the faces of each of its edges.
- `a_triple_click_selects_everything_connected`:
  - On the box's top face: 12 edges and 6 faces, not the separate square.
  - With Shift held, the mode is `Add`.
- `a_double_click_on_a_group_still_opens_it`: an instance gives `OpenInstance`, as today (move the existing assertion here if one exists; else new).

In `editor.rs`, `a_triple_click_after_opening_a_group_selects_only_the_click`:
- Use the group fixture of `double_clicking_a_group_opens_it_and_escape_closes_it`.
- `double_click_at` the group, then press and release at the same spot, then `e.triple_click()`.
- The group is open and the selection is exactly the one face clicked (not its connected set).

In `app.rs`, migrate `double_clicks_are_two_quick_releases_in_one_place` to its twin `clicks_count_up_to_three_then_start_over`:
- `click_count(None, ..) == 1`.
- Within time and place: 1 -> 2 -> 3 -> 1.
- Too late or too far: 1.

In `tool_input_tests.rs`, `double_and_triple_clicks_select_bounds_and_connected`:
- Build a box with `add_rectangle` and `push_pull`, then select the Select tool.
- `double_click` on the top face selects 5 ids.
- `triple_click` there selects 18.
- `double_click` on a group still opens it (`editor`'s context changes; check with the `context` query).

In `crates/api/src/input.rs` tests, `a_triple_click_takes_a_pixel_or_a_point`, beside the double-click one.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p tools select && cargo test -p app click && cargo test -p api triple`
Expected: compile errors.

- [ ] **Step 3: Implement**

- `SelectTool` gains `opened: bool`. It is set when its `DoubleClick` pushed `OpenInstance` or `CloseInstance`. It is cleared by any `Click`, `Move` or `TripleClick`. A `TripleClick` with `opened` set does nothing.
- `Editor::triple_click` is `double_click` with `ToolEvent::TripleClick`.
- In `app.rs`'s `MouseInput` arm, a click release computes the count. 2 calls `double_click`, 3 calls `triple_click`. `clicks` keeps `(now, cursor, count)`. A drag release and focus loss clear it.
- `Input::TripleClick` in `drive`: three presses and releases. `double_click` runs after the second when it reached the tool, `triple_click` after the third when it did, as `DoubleClick` does now.

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/tools/src/tool.rs crates/tools/src/select.rs crates/tools/src/line.rs crates/tools/src/rectangle.rs crates/tools/src/circle.rs crates/tools/src/arc.rs crates/tools/src/push_pull.rs crates/tools/src/move_tool.rs crates/tools/src/rotate.rs crates/tools/src/tape.rs crates/tools/src/erase.rs crates/tools/src/paint.rs crates/app/src/editor.rs crates/app/src/editor/input.rs crates/app/src/app.rs crates/app/src/host/tool_input_tests.rs crates/api/src/input.rs
git commit -m "Select: double-click bounds, triple-click connected"
```

---

### Task 7: Arrow keys lock Rotate's plane before the first click

**Files:**
- Modify: `crates/tools/src/tool.rs` (trait methods), `crates/tools/src/rotate.rs`, `crates/app/src/editor.rs` (`key`'s `LockAxis`/`Unlock` arms, `status_parts`)
- Test: `crates/tools/src/rotate.rs`, `crates/app/src/host/tool_input_tests.rs`

**Interfaces:**
- Produces:

```rust
/// Before its first click, a tool whose operation lies in a plane (Rotate) can lock that
/// plane's normal to an axis (`Some`) or unlock it (`None`). True if the tool took it.
fn lock_plane(&mut self, _axis: Option<Axis>) -> bool { false }
/// The axis `lock_plane` locked, if any.
fn plane_lock(&self) -> Option<Axis> { None }
```

- [ ] **Step 1: Write the failing tests**

In `rotate.rs`:
- `an_axis_locked_before_the_first_click_sets_the_plane`:
  - With the rig over the square on the ground, `lock_plane(Some(Axis::X))` returns true.
  - A center click on the face (normal Z) then turns about `axis: Some([1,0,0])`. Check the finished `Rotate` command's `axis` after clicking a start and an end on the X plane (points `(0,0,1)` then `(0,1,0)` about center `(0,0,0)`).
  - The lock survives the rotation (`plane_lock() == Some(X)`).
  - `Cancel` clears it.
  - While a turn is in progress, `lock_plane` returns false.
- `a_locked_plane_shows_the_protractor_under_the_cursor`: idle and locked to Z, a `Move` gives 24 preview segments around the hover point. Unlocked idle gives none.

In `tool_input_tests.rs`, `arrow_keys_lock_rotates_plane_before_the_first_click`:
- Select Rotate, `key` `right`: `editor.lock == "Locked: red axis"`.
- Send `right` again: `null`. Send `up`, then `down`: `null`.
- With `up` locked (Z), select the box's side face (normal X). Click the center on that face, then `type` "90": the `rotate` command's `axis` is `[0,0,1]`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p tools rotate && cargo test -p app arrow_keys_lock_rotates`
Expected: FAIL.

- [ ] **Step 3: Implement** the Spec notes' plane-lock rules.

- `RotateTool` gains `plane: Option<Axis>`.
- In `Editor::key`, `LockAxis(axis)` with `self.tool.anchor().is_none()`:
  - It asks `lock_plane` for `axis`, or for `None` when `plane_lock()` is already `axis`.
  - When the tool took it, it invalidates the hover and returns true. Otherwise it returns false, as today.
- `Unlock` with no anchor and a plane lock calls `lock_plane(None)`.
- `status_parts`' `lock` falls back to `tool.plane_lock()` with the same three texts.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p tools && cargo test -p app`
Expected: PASS. `editor`'s existing `nothing to lock yet` assertion (Line, no anchor) still holds: Line does not take `lock_plane`.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/tools/src/tool.rs crates/tools/src/rotate.rs crates/app/src/editor.rs crates/app/src/host/tool_input_tests.rs
git commit -m "Rotate: arrow keys lock the protractor plane before the first click"
```

---

### Task 8: No silent dead ends; honest Move and Rotate prompts

**Files:**
- Modify: `crates/tools/src/push_pull.rs`, `paint.rs`, `erase.rs`, `move_tool.rs`, `rotate.rs`
- Test: the same files' test modules, `crates/app/src/host/tool_input_tests.rs`

**Interfaces:**
- Produces: the messages and prompts in the Spec notes, exactly as written there.
  - Every dead-end message is set with `out.info = true`.
  - `MoveTool` and `RotateTool` gain `selected: bool`, set from `!ctx.selection.is_empty()` on every event.

- [ ] **Step 1: Write the failing tests**

- `push_pull.rs`, `clicking_off_a_face_says_what_push_pull_works_on`:
  - A click off the square: no drag, `info`, and the message "Push/Pull works on faces: click a face to push or pull it".
  - A click on a group's face (a rig with an instance, or a `Scenery` whose face lies under `Owner::Instance`): "Open the group to push or pull its faces".
- `paint.rs`, `clicking_nothing_says_what_paint_works_on`: "Paint works on faces and groups: click one to paint it", `info`.
- `erase.rs`, `clicking_nothing_says_what_erase_works_on`: "Erase works on edges, faces, guides and groups: click one to erase it", `info`.
- `move_tool.rs`, `the_idle_prompt_says_what_a_click_moves`: the four prompts, after a `Move` event with and without a selection and with `ToggleCopy`.
- `rotate.rs`, `the_idle_prompt_says_what_a_click_turns`: the four prompts.
- `tool_input_tests.rs`, `dead_end_clicks_explain_without_flashing`:
  - Push/Pull `click` at empty ground leaves `editor.message_kind == "info"` and the Push/Pull message.
  - Run the existing typed-value check: an `abc` still gives `"error"`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p tools says_what idle_prompt && cargo test -p app dead_end`
Expected: FAIL.

- [ ] **Step 3: Implement.** Paint's and Erase's `Click` arms set the message where they return with no target. Push/Pull checks `face_under_in` for an `Owner::Instance` face before the generic message.

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: PASS. Update any test that compared the old Move or Rotate prompt text to the new text: those are its migrated twins.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/tools/src/push_pull.rs crates/tools/src/paint.rs crates/tools/src/erase.rs crates/tools/src/move_tool.rs crates/tools/src/rotate.rs crates/app/src/host/tool_input_tests.rs
git commit -m "Tools: say what Push/Pull, Paint and Erase work on; honest Move and Rotate prompts"
```

---

### Task 9: Select All, Select None and Duplicate

**Files:**
- Modify: `crates/app/src/shortcuts.rs` (`Bound`, `TABLE`, labels, group, actions), `crates/app/src/ui/mod.rs` (`UiAction::{SelectAll, SelectNone, Duplicate}`), `crates/app/src/menus.rs` (Edit menu, `yields_to_text`), `crates/app/src/app.rs` (`apply`)
- Modify: `crates/app/src/editor.rs` (`select_all`, `select_none`, `duplicate`), `crates/app/src/editor/input.rs` (`press`), `crates/app/src/host.rs` (`editor_command`)
- Modify: `crates/api/src/editor.rs` (`EditorCommand::Duplicate {}`, `EDITOR_COMMANDS`), `crates/api/src/input.rs` (the cmd key check and its doc)
- Test: `crates/app/src/shortcuts.rs`, `crates/app/src/ui/tests/menus.rs`, `crates/app/src/editor.rs`, `crates/app/src/host/tool_input_tests.rs`, `crates/api/src/input.rs`

**Interfaces:**
- Consumes: `Command::Select { all }` (Task 2).
- Produces:
  - `Bound::{SelectAll, SelectNone, Duplicate}` with the chords, labels and group from the Spec notes. `TABLE` becomes `[Shortcut; 29]`.
  - `UiAction::{SelectAll, SelectNone, Duplicate}`.
  - `Editor::select_all(&mut self) -> bool` (always true).
  - `Editor::select_none(&mut self) -> bool` (false when nothing was selected).
  - `Editor::duplicate(&mut self) -> Result<(), CommandError>`. With an empty selection it is `Err(InvalidInput, "select something to duplicate first")`, and the status message is "Select geometry or a group to duplicate". The window ignores the `Err`, since the status says why.
  - `menus.rs`: `fn yields_to_text(action: &UiAction) -> bool` (Undo, Redo, SelectAll, SelectNone) replaces `is_history`.
  - `EditorCommand::Duplicate {}` and `"duplicate"` in `EDITOR_COMMANDS` (`[&str; 8]`).

- [ ] **Step 1: Write the failing tests**

In `shortcuts.rs`, extend `command_chords_match_their_modifiers_and_repeat_only_undo_and_redo`:

```rust
assert_eq!(command('a', false, false), Some(Bound::SelectAll));
assert_eq!(command('a', true, false), Some(Bound::SelectNone));
assert_eq!(command('d', false, false), Some(Bound::Duplicate));
assert_eq!(command('a', false, true), None, "held Cmd+A selects once");
assert_eq!(plain('a'), Some(Bound::Tool(ToolKind::Arc)), "plain a is still Arc");
```
and `chords_read_as_mac_glyphs_or_words` with `text(Bound::SelectNone, true) == "⇧⌘A"` and `text(Bound::Duplicate, false) == "Ctrl+D"`. `with_native_menus_the_key_path_leaves_cmd_chords_to_the_menu` covers the new rows unchanged.

In `ui/tests/menus.rs`:
- `the_edit_menu_selects_all_none_and_duplicates`:
  - Open Edit. The items "Select All", "Select None" and "Duplicate" report their actions.
  - Select None and Duplicate are disabled without a selection (fixture `edit_fixture(false, false)`); Select All is enabled.
  - Their key texts are the table's.
- `select_all_and_none_yield_to_a_focused_text_field`: `native_enabled` is false for Select All and Select None while `text_focused`, and true for Duplicate.
- Extend `native_menu_events_act_once_and_leave_typing_alone`:
  - `!native_event_acts(&UiAction::SelectAll, false, true)`.
  - `native_event_acts(&UiAction::Duplicate, false, true)`.
  - `!native_event_acts(&UiAction::Duplicate, true, false)` (held: once).
- Migrate `undo_and_redo_menu_items_are_off_while_typing` to its twin `text_actions_are_off_while_typing`, with `matches!(Undo | Redo | SelectAll | SelectNone)`.

In `editor.rs`:
- `select_all_and_none_keep_the_tool`:
  - With a box and Rectangle active and its first corner clicked, `select_all()` selects every edge and face, and the tool is still Rectangle and active.
  - `select_none()` empties the selection and returns true; called again it returns false.
- `duplicate_moves_a_copy_with_the_cursor`:
  - Select a square. Put the cursor over its corner (`move_to`), then `duplicate()`.
  - The tool is Move, `phase` is active with that corner as base point, and the prompt starts with "Copy:".
  - Moving 3 m along X and clicking makes one `Copy` (one undo step), and the original stays.
  - With nothing selected, `duplicate()` is `Err` and the tool does not change.
- `duplicate_with_the_cursor_outside_waits_for_a_base_point`: after `cursor_left()`, `duplicate()` gives Move, idle, copying.

In `tool_input_tests.rs`, `cmd_keys_select_all_none_and_duplicate`:
- `key` `a` with `cmd` selects everything, and with `shift` too it selects nothing.
- `key` `d` with `cmd` after selecting starts Move copying.
- `key` `d` with `cmd` and `shift` is `invalid_input`.
- `execute {"type": "duplicate"}` with an empty selection is `invalid_input`.

In `api/src/input.rs`, migrate the test that expects "with cmd or ctrl only z ..." to its twin. Cmd+S is still refused, with the new message: "with cmd or ctrl only z (undo, with shift redo), a (select all, with shift none) and d (duplicate) are taken; file actions are commands".

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app shortcuts menus select_all duplicate cmd_keys && cargo test -p api input`
Expected: compile errors.

- [ ] **Step 3: Implement**

- Table rows, labels ("Select All", "Select None", "Duplicate"), group "Edit", and actions.
- The Edit menu as in the Spec notes.
- `App::apply` arms call the editor. They ignore `duplicate`'s `Err`, since the status says why.
- The key path in `app.rs` needs no change: `Some(bound) => s.apply(action)` covers the new bindings off macOS and for Ctrl on macOS.
- `Editor::duplicate`, as in the Spec notes:
  - `select_tool(Move)`, then `send(ToggleCopy)`, then `hover_now()`.
  - If `hit` is Some and the cursor is in the view (`!hover_hidden && !covered`), `send(Click(hit))`.
- `press` in `editor/input.rs` takes `a`/`A` and `d` under cmd. The static check in `api/src/input.rs` allows `z`, `a` and `d` (either case).

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: PASS, including `every_cmd_chord_is_a_native_menu_accelerator` and `the_menu_bar_has_the_same_items_in_every_state`.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/shortcuts.rs crates/app/src/ui/mod.rs crates/app/src/menus.rs crates/app/src/app.rs crates/app/src/editor.rs crates/app/src/editor/input.rs crates/app/src/host.rs crates/api/src/editor.rs crates/api/src/input.rs crates/app/src/ui/tests/menus.rs crates/app/src/host/tool_input_tests.rs
git commit -m "Edit: Select All, Select None and Duplicate"
```

---

### Task 10: Recent files in the preferences

**Files:**
- Modify: `crates/app/src/prefs.rs`, `crates/app/src/app.rs`
- Test: `crates/app/src/prefs.rs`

**Interfaces:**
- Produces (`prefs.rs`):

```rust
/// How many recent files the prefs keep.
pub const RECENT_LEN: usize = 10;
// Prefs gains (and loses `Copy`):
/// Files opened or saved in the window, newest first, absolute, at most `RECENT_LEN`.
pub recent: Vec<PathBuf>,
/// Puts `path` (made absolute) first in `recent`, dropping an equal entry and any past
/// `RECENT_LEN`.
pub fn remember_recent(recent: &mut Vec<PathBuf>, path: &Path);
/// The first `n` entries of `recent` that are regular files now.
pub fn existing(recent: &[PathBuf], n: usize) -> Vec<PathBuf>;
```
- `load` cleans `recent`: absolute only, no duplicates (first kept), at most `RECENT_LEN`.
- In `App`/`State`:
  - `fn remember_file(&mut self)`: records `editor.doc().path()`, saves prefs, and rebuilds `recent_shown: Vec<PathBuf>` (= `existing(&prefs.recent, CARD_RECENT)`; `CARD_RECENT = 5` lives in `prefs.rs`).
  - It is called after a successful `open` (dialog), a successful `save` (both forms) and the FILE argument at startup. Task 11 adds the card's opens.

- [ ] **Step 1: Write the failing tests** (`prefs.rs`, temp homes as the existing tests make them)

- `recent_files_round_trip_newest_first`:
  - Remember `/a/1.skr`, `/a/2.skr`, then `/a/1.skr` again: the list is `[/a/1.skr, /a/2.skr]`.
  - Twelve distinct files keep the last ten, newest first.
  - Save and load give the same list.
- `recent_files_load_clean`: a prefs file with `"recent": ["rel.skr", "/x/a.skr", "/x/a.skr", ...12 more absolute]` loads to absolute, unique, at most 10.
- `old_prefs_files_have_no_recent_files`: the existing old-file fixture loads with `recent` empty. Extend `dock_prefs_load_old_files_and_clamp_bad_values` or add this one.
- `the_card_lists_only_files_that_exist`: in a temp folder with two real files and one missing, `existing(&list, 5)` gives the two real ones in list order; `existing(&list, 1)` gives one.
- `a_relative_path_is_remembered_absolute`: `remember_recent(&mut v, Path::new("x.skr"))` stores `std::path::absolute("x.skr")`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app prefs`
Expected: compile errors.

- [ ] **Step 3: Implement.** Replace `Prefs`'s `Copy` uses with `clone()` where the compiler asks. In `app.rs`, call `remember_file` where the Interfaces list says, only on success: `open_file` returns true, `save` returns true, and the startup `execute(open)` is `Ok`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p app`
Expected: PASS. No test writes under the real `~/.tenon`.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/prefs.rs crates/app/src/app.rs
git commit -m "Prefs: recent files, recorded on open and save"
```

---

### Task 11: The empty-model card

**Files:**
- Create: `crates/app/src/ui/start.rs`, `crates/app/src/ui/tests/start.rs`
- Modify: `crates/app/src/ui/mod.rs` (`View::start`, `UiAction::{OpenRecent, DismissStart}`, `draw` calls `start::start_card`), `ui/icons.rs` (`X`), `ui/theme.rs` (card metrics), `ui/tests/mod.rs` (`Fixture::start`), `crates/app/src/editor.rs` (`untouched`), `crates/app/src/app.rs`
- Test: `crates/app/src/ui/tests/start.rs`, `crates/app/src/editor.rs`

**Interfaces:**
- Consumes: `prefs::{existing, CARD_RECENT}`, `State::recent_shown`, `remember_file` (Task 10).
- Produces:

```rust
/// What the empty-model card lists; `View::start` is None when the card is hidden.
pub struct StartCard<'a> { pub recent: &'a [PathBuf] }
// View gains: pub start: Option<StartCard<'a>>,
// UiAction gains:
/// Open this recent file (after asking about unsaved changes).
OpenRecent(PathBuf),
/// Hide the empty-model card for the rest of the session.
DismissStart,
// ui::start:
pub fn start_card(ui: &Ui, view_rect: Rect, card: &StartCard, actions: &mut Vec<UiAction>);
// theme:
pub const START_CARD: Vec2 = Vec2::new(360.0, 200.0);
pub const START_CARD_INSET: f32 = 12.0;
pub const START_CARD_PADDING: f32 = 12.0;
// icons:
pub const X: &str = ph::X;
// Editor:
/// True for an empty, unedited model: the root open, the root mesh without vertices,
/// instances or guides, and nothing to undo or redo.
pub fn untouched(&self) -> bool;
```
- `App` keeps `start_dismissed: bool`. It sets `View::start` to `Some(StartCard { recent: &self.recent_shown })` when `editor.untouched() && !start_dismissed && !editor.flying()`.
- `apply`:
  - `OpenRecent(path)`: if `settle_changes()` and `editor.open_file(&path)`, then `remember_file()`.
  - `DismissStart`: `start_dismissed = true`.

- [ ] **Step 1: Write the failing tests**

In `editor.rs`, `the_model_is_untouched_until_the_first_edit`:
- A fresh editor is untouched.
- After an `AddRectangle` it is not. After `undo()` it is still not: there is a redo.
- After `new_model()` it is untouched again.
- An `AddGuidePoint` on a fresh model makes it touched.
- Opening a group's model leaves it touched (not root).

In `ui/tests/start.rs`:

```rust
fn with_card(recent: &[&str]) -> Fixture {
    Fixture { start: Some(recent.iter().map(PathBuf::from).collect()), ..Default::default() }
}

#[test]
fn the_card_sits_bottom_left_at_its_size() {
    let h = harness(with_card(&["/m/house.skr"]));
    let (view, card) = (h.state().central, rect_of(&h, "Start card"));
    assert!((card.width() - 360.0).abs() < 0.5 && (card.height() - 200.0).abs() < 0.5, "{card:?}");
    assert!((card.left() - (view.left() + 12.0)).abs() < 0.5);
    assert!((card.bottom() - (view.bottom() - 12.0)).abs() < 0.5);
}

#[test]
fn the_cards_buttons_report_their_actions() {
    let mut h = harness(with_card(&["/m/house.skr", "/m/barn.skr"]));
    assert_eq!(click(&mut h, "New"), [UiAction::NewModel]);
    assert_eq!(click(&mut h, "Open..."), [UiAction::OpenFile]);
    assert_eq!(click(&mut h, "barn.skr"), [UiAction::OpenRecent("/m/barn.skr".into())]);
    assert_eq!(click(&mut h, "Dismiss"), [UiAction::DismissStart]);
}

#[test]
fn the_card_shows_five_recent_files_and_a_one_line_hint() {
    let names: Vec<String> = (1..=7).map(|i| format!("/m/{i}.skr")).collect();
    // The fixture passes what the app would: at most five.
    let five: Vec<&str> = names.iter().take(5).map(String::as_str).collect();
    let h = harness(with_card(&five));
    for i in 1..=5 { assert!(h.query_by_label(&format!("{i}.skr")).is_some()); }
    let texts = drawn_texts(&h);
    let hint = texts.iter().find(|t| t.text == "R: drag a rectangle.  P: pull it up into a box.").expect("hint");
    assert!(hint.rect.width() <= 360.0 - 2.0 * 12.0, "one line inside the card");
    let empty = harness(with_card(&[]));
    assert!(drawn_texts(&empty).iter().any(|t| t.text == "No recent files"));
}

#[test]
fn no_card_when_hidden_and_presses_on_it_stay_off_the_view() {
    let h = harness(Fixture::default());
    assert!(h.query_by_label("Start card").is_none());
    let h = harness(with_card(&[]));
    let at = rect_of(&h, "Start card").center();
    let layer = h.ctx.layer_id_at(at).expect("the card's area");
    assert_ne!(layer.order, egui::Order::Background, "Gui::wants_pointer_at is true there");
}

#[test]
fn the_card_settles_in_one_frame() {
    // As `the_panels_settle_in_one_frame`: after the first frame, no repaint is requested.
}
```
- The card's frame is a `ui.interact` region named "Start card" through `WidgetInfo::labeled(WidgetType::Other, true, "Start card")`, so tests can find its rect.
- `drawn_texts` returns the painted text and its rect. Extend it to carry the rect if it does not already, in `ui/tests/mod.rs`.
- In `ui/tests/mod.rs`, `Fixture` gains `start: Option<Vec<PathBuf>>` (default None) and `View::start` is built from it.
- Add `X` to `icons::all()`; the existing glyph test covers it.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app untouched start`
Expected: compile errors.

- [ ] **Step 3: Implement** `start_card` and the app wiring as in the Spec notes and Interfaces.

- The card is an `egui::Area::new("start-card")` with `.order(Order::Middle).movable(false).fixed_pos(...)`, holding an `egui::Frame` with the exact size (`ui.set_min_size` and `set_max_size` to the inner size).
- Recent rows are `ui.button(file_name)` with `.on_hover_text(full_path)`.
- In `ui::draw`, call `start_card` after the HUD and before the gizmo, only when `view.start` is Some.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p app`
Expected: PASS, including `the_panels_settle_in_one_frame`.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/ui/start.rs crates/app/src/ui/tests/start.rs crates/app/src/ui/mod.rs crates/app/src/ui/icons.rs crates/app/src/ui/theme.rs crates/app/src/ui/tests/mod.rs crates/app/src/editor.rs crates/app/src/app.rs
git commit -m "Viewport: a start card on an empty model with recent files"
```

---

### Task 12: Docs: the agent API

**Files:**
- Modify: `docs/agent-api.md`

- [ ] **Step 1: Edit**

- **Commands, `select`:** add `{"type": "select", "all": true}`. It selects every edge, face, group and guide of the open context, combined by `mode`. `ids` must then be absent or empty. Select None is `{"type": "select", "ids": []}`.
- **Editor:**
  - The example's `tool` becomes `"select"` with its prompt. Rewrite the example's status and prompt to the Select tool's.
  - Add one sentence: the editor starts in the Select tool.
  - In the editor-command table, add `{"type": "duplicate"}`. It starts Move copying the selection, with the base point under the cursor when it is in the view, as Cmd+D does. With nothing selected it is `invalid_input`. The copy is made by the next click or typed distance (one `copy` step).
  - `lock` also shows Rotate's plane lock before its first click.
- **Driving the tools:**
  - Add a `triple_click` row: three left clicks there; the Select tool selects everything connected.
  - Extend the `double_click` row: the Select tool selects a face with its edges, or an edge with its faces, and still opens a group. Push/Pull repeats the last pull distance on the face.
  - Extend the `drag` row: with Line, Rectangle, Circle, Arc or Push/Pull, a drag draws as two clicks (the release more than the drag threshold from the press).
  - In the `key` row, the arrow keys also lock Rotate's plane before its first click. With `cmd` or `ctrl`, `z` (undo, with `shift` redo), `a` (select all, with `shift` none) and `d` (duplicate) are taken.
  - Add a sentence after typed values: right after a pull, a typed distance redoes the pull at that distance, as Move and Rotate allow.
- **Window actions table:**
  - Edit > Select All, Select None (Cmd+A, Shift+Cmd+A): `select` with `all: true`, `select` with `ids: []`.
  - Edit > Duplicate (Cmd+D): `duplicate`, or `copy` with an explicit offset.
  - The Select row adds double- and triple-click (`double_click`, `triple_click`).
  - The tools row adds drag to draw and Push/Pull retype and repeat.
  - New row, the empty-model card's New, Open... and recent files: `new`, `open` (`path`).
- **Chrome paragraph:**
  - Add the empty-model card (shown on an untouched empty model; dismissing it is chrome) and the recent-files list kept in prefs (never read or written by the API; files agents open are not added).
  - The native-menu sentence now reads: except Undo, Redo, Select All and Select None, which go to the field.
  - Add: tools' dead-end messages are `info` (`editor.message_kind`).

- [ ] **Step 2: Check** that `cargo test -p api` passes and that the file has no emoji (`grep -nP '[\x{1F300}-\x{1FAFF}]' docs/agent-api.md` prints nothing).

- [ ] **Step 3: Commit**

```bash
git status && git diff --stat
git add docs/agent-api.md
git commit -m "Docs: drag to draw, selection clicks, select all, duplicate and the start card"
```

---

### Task 13: Roadmap, whole-branch review, renderer check, live look

- [ ] **Step 1: Full gate**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace 2>&1 | tee target/m11d-test.log | grep "test result" | awk '{s+=$4} END {print s}'
grep -E "FAILED|failed;" target/m11d-test.log
cargo build --release -p app
git diff --stat <base>..HEAD -- crates/render
```
Expected:
- clean;
- at least 1311 plus this milestone's new tests;
- no failure lines;
- a release build;
- `crates/render` changes are only the Task 2 `all: false` lines in `crates/render/tests/golden.rs` and `perf.rs`. No source or shader file changed, so CLASSIC renders cannot differ and the two-worktree `cmp` is skipped. If anything under `crates/render/src` changed, run the M11c two-worktree golden `cmp` (M11c plan, Task 13 Step 4) and fix any `DIFF`.

`<base>` is the commit before Task 1.

- [ ] **Step 2: Roadmap row**

In `docs/superpowers/roadmap.md`, change the M11d row's last cell from `planned` to `done <first>..<last>`. Use the short hashes of Task 1's commit and Task 12's commit.

```bash
git status && git diff --stat
git add docs/superpowers/roadmap.md
git commit -m "Roadmap: M11d done"
```

- [ ] **Step 3: Whole-branch review**

Use superpowers:requesting-code-review over `<base>..HEAD`, against the spec's M11d bullet and this plan's Spec notes and Review Focus. Ask the reviewer to check in particular:
- Drag to draw arms only the operation's first press, and navigation drags never reach a tool.
- A Push/Pull retype never undoes a step that is not its own pull; `on_external_change` clears `last`.
- No double fire:
  - every new Cmd chord is a native menu accelerator and is refused by `window_command` with Cmd on macOS;
  - Ctrl chords still act through the key path;
  - Select All and Select None yield to a focused text field.
- `select` `all` with `ids` is refused, and recorded `select` commands serialize as before.
- Duplicate makes one undo step and never a zero-offset copy.
- The card has no repaint of its own, sits at 360 x 200 pt inset 12 pt bottom left, and hides at the first edit.
- Prefs: recent paths are absolute, capped at 10, never written by the API or headless mode, and tests never touch the real home.
- Every Spec note value matches the code (messages, prompts, chords, labels, sizes).
- No test was lost (compare with Task 1's baseline), and every Task 3 migration only added a tool selection.

Fix findings in follow-up commits and rerun Step 1.

- [ ] **Step 4: Live look in the real window**

1. Build a `.app` bundle wrapper in the scratchpad around the release binary, with its own bundle id and socket, as the M11b memory note describes (a bare binary is invisible to computer use). Use `app_*` computer-use tools on that app only. Do not take over the user's screen (no full-control request).
2. Open it and check, by screenshot:
   - It starts in Select. The start card is at the bottom left with New, Open... and recent files.
   - R, then a drag, draws a rectangle, and the card is gone.
   - P, then a drag up, pulls it. Typing 2 and Enter redoes the pull at 2 m.
   - Double-clicking another face with Push/Pull repeats the distance.
   - With Select, a double-click on the top face selects it with its edges, and a triple-click selects the box.
   - Edit shows Select All (⌘A), Select None (⇧⌘A) and Duplicate (⌘D). Cmd+A selects everything and Cmd+Shift+A clears the selection.
   - Cmd+D makes a copy follow the cursor, and a click drops it.
   - Q, then the Right arrow, shows the "Locked: red axis" pill and a protractor in the X plane under the cursor.
   - Push/Pull clicking empty space shows its message in the status bar without a red flash.
   - Activity Monitor shows about 0% CPU once the window is still, with the card shown.
3. Report what was seen. Fix anything wrong in follow-up commits.

Report the milestone's summary with the rulings in "Spec notes".
