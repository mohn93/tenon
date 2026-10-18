# M12b Batch References Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A command in a `batch` (and in the MCP `transact` tool) can name ids that an earlier command of the same batch made, such as `"$0:f0"` for the first face command 0 created, so an agent can draw and then edit in one atomic call without guessing ids.

**Architecture:** A new `session::BatchCommand` holds a parsed `Command` in which each reference is replaced by a stand-in id (number `STAND_IN_BASE + k`, k = the reference's index in the command), plus the list of parsed references. `Document::execute_batch` resolves each command just before its turn against the outcomes so far, replacing stand-ins in place on the typed command (so `--api-root` path rewrites made before the batch survive). `api::dispatch` parses and checks the references before anything runs; `describe` and the MCP `transact` schema accept a reference wherever a batch takes an entity or material id.

**Tech Stack:** Rust 2024, serde_json =1.0.151, schemars =1.2.2. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md`, the M12b bullet under "Agent integration milestones".

## Global Constraints

- No new crates or dependencies.
- Existing JSON-RPC behaviour is unchanged for batches without references. Every existing test keeps passing; `Document::execute_batch` callers that pass `Vec<Command>` compile unchanged.
- Gate per task: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. Use `export PATH="$HOME/.cargo/bin:$PATH"`.
- `docs/agent-api.md` and `docs/mcp.md` are updated in the task that changes the API. No emoji. Commit messages short, no attribution lines.
- Never `git add -A`; stage the files you changed.

## Spec notes (rulings)

1. Grammar, whole string only: `^\$(0|[1-9][0-9]*):(([vefigm])(0|[1-9][0-9]*|\*)|\*)$`. `"$N:KI"` is the I-th id of kind K in command N's outcome; `"$N:K*"` all of that kind; `"$N:*"` every edge, face, instance and guide in `changes.created` (no vertices, which most commands refuse, and no materials; `"$N:v*"` gives the vertices). Kinds `v e f i g` read `changes.created` filtered by `EntityId` kind, in the order listed (ids are sorted, so lowest id = made first); `m` reads `changes.materials`.
2. Where references are recognised (the "reference keys"): the top-level fields `ids`, `face`, `id` and `material` of any command, and `path` only when `type` is `open_path`. A string anywhere else (names, file paths) is never touched, even if it looks like a reference. A string in a reference key that starts with `$` but does not match the grammar is a parse error ("invalid reference").
3. `*` references are allowed only inside arrays (`ids`, `open_path`'s `path`) and expand in place, keeping order; `m*` and any `*` in a scalar field (`face`, `id`, `material`) is a parse error. An expansion to zero ids is allowed (an empty list is whatever the command makes of it).
4. Kind must fit the field: `material` takes only `m` references; `ids`, `face`, `id`, `path` take only entity references (`v e f i g` or `*`). A mismatch is a parse error. Whether the entity kind suits the command (a face for `push_pull`) is left to the command, as with literal ids.
5. Parse errors (grammar, kind/field mismatch, `*` in a scalar, N not lower than the command's own index) reject the whole batch before anything runs: JSON-RPC `-32602` with message `invalid params: commands[<index>]: <reason>`. A command that is not a valid command at all keeps failing with `-32602` as today, now also prefixed `commands[<index>]: `.
6. Resolution errors (index I past what command N made) fail that command at its turn with `ErrorKind::InvalidInput` and the message `<reference>: command N made <count> <kind word>` (kind words: vertex/vertices, edge/edges, face/faces, instance/instances, guide/guides, material/materials, e.g. `$0:f1: command 0 made 1 face`). It is a normal `BatchError` at that index: atomic batches roll back, non-atomic ones keep what ran.
7. `STAND_IN_BASE = 1 << 62`. Entity and material ids come from one counter that never gets near it. Stand-ins are an internal detail: never serialized to a client, and `BatchCommand::command()` (the stand-in command) is only used for checks before the batch runs (type, paths, `--api-root`).
8. `execute` (a single command) does not accept references: a reference there fails to parse as an id, as today. The MCP `transact` with one command goes through `execute`, so its description says references need two or more commands.
9. In schemas: the batch params schema and the MCP `transact` schema replace their `EntityId` and `MaterialId` `$defs` with `anyOf [<the id schema>, {"$ref": "#/$defs/BatchRef"}]` and add a `BatchRef` def (`type: string`, the grammar as `pattern`, a description). In `transact` the defs are shared with the editor and screen command schemas, which therefore also show references as allowed; they only run alone, through `execute`, where a reference is refused. Accepted looseness.
10. A reference to an id that a later command of the batch erased, or that does not exist any more, is passed through as a real id; the command fails as it would with that literal id (`not_found`).

## Review Focus

1. A name or file path that looks like a reference (`make_component` named `"$0:f0"`, `open` of a file `"$0:f0"`) must stay literal text. Test in Task 1.
2. `--api-root` rewrites `change_material`'s texture path before the batch runs; a `change_material` whose `material` is a reference must keep the rewritten path. Test in Task 2.
3. The window app's non-atomic batch with a `discard` command runs its own loop (`AppHost::execute_batch` with recovery); references must resolve there too. Test in Task 1.
4. A failed atomic batch with references rolls back everything, including camera, selection and open context, exactly as without references. Test in Task 1.
5. `$0:*` after `make_group` lists the group's instance and nothing inside it (only the open context's created ids); grouping "everything command 0 made" then painting the group is the common agent flow. Test in Task 1.

---

### Task 1: `BatchCommand` in session, executed with references resolved

**Files:**
- Create: `crates/session/src/batch.rs`
- Modify: `crates/session/src/lib.rs` (module, exports)
- Modify: `crates/session/src/document.rs:554-641` (`execute_batch`)
- Modify: `crates/app/src/editor.rs:355` (`Editor::execute_batch`), `crates/app/src/host.rs:178-203` (`AppHost::execute_batch`, recovery loop)
- Modify: `crates/api/src/dispatch.rs:76-80` (`Host::execute_batch` signature only), `crates/api/src/headless.rs:58`, `crates/api/tests/editor.rs:64`
- Test: `crates/session/tests/batch_refs.rs` (new), `crates/app/src/host.rs` tests module

**Interfaces:**
- Produces, in `session` (exported from `lib.rs`):
  - `pub const STAND_IN_BASE: u64 = 1 << 62;`
  - `pub struct BatchRef { pub command: usize, pub kind: Option<RefKind>, pub index: Option<usize> }` (`kind: None` = `*` over all entities; `index: None` = `*`), `pub enum RefKind { Vertex, Edge, Face, Instance, Guide, Material }`, `impl Display for BatchRef` printing the original text (`$0:f1`, `$0:e*`, `$0:*`).
  - `pub struct BatchCommand` with:
    - `pub fn parse(value: serde_json::Value, index: usize) -> Result<BatchCommand, String>` — rulings 1-5; the `String` is the reason without the `commands[i]` prefix.
    - `pub fn command(&self) -> &Command` and `pub fn command_mut(&mut self) -> &mut Command` — the stand-in command.
    - `pub fn has_references(&self) -> bool`
    - `pub fn resolve(&self, done: &[Outcome]) -> Result<Command, CommandError>` — `done[n]` is command n's outcome; returns the command with stand-ins replaced (ruling 6 errors).
    - `impl From<Command> for BatchCommand` (no references).
  - `Document::execute_batch<C: Into<BatchCommand>>(&mut self, commands: Vec<C>, atomic: bool) -> Result<Vec<Outcome>, BatchError>`
  - `Editor::execute_batch(&mut self, commands: Vec<BatchCommand>, atomic: bool)` and `Host::execute_batch(&mut self, commands: Vec<BatchCommand>, atomic: bool)`: same results as before.
- Consumed by Task 2: `BatchCommand::parse`, `command_mut`, `STAND_IN_BASE`.

Parsing approach for `parse`: on the JSON object, for each reference key (ruling 2) replace every reference string (scalar, or array element) with the stand-in id string of the right kind (`f<STAND_IN_BASE+k>`, `m<STAND_IN_BASE+k>`; a `*`-over-all reference uses `v<...>`), then deserialize `Command`. After deserializing, collect stand-ins from the typed command (below) and check there are exactly as many as references; a mismatch is an internal error string ("reference in a field that is not an id"), kept as a guard.

Resolving approach: one private function `fn id_slots(command: &mut Command, f: &mut dyn FnMut(Slot<'_>))` with an exhaustive `match` (no `_` arm, so a new command forces a decision) yielding `Slot::List(&mut Vec<EntityId>)`, `Slot::One(&mut EntityId)` and `Slot::Material(&mut MaterialId)` (including `Paint`'s `Option<MaterialId>` when `Some`). `resolve` clones the command and replaces each stand-in (an id whose number is `>= STAND_IN_BASE`) by its reference's ids; lists splice expansions in place.

In `Document::execute_batch`, call `resolve(&outcomes)` at each command's turn in both the atomic and the non-atomic loop; a resolve error is that command's error. `AppHost`'s recovery loop resolves the same way before `execute_keeping`. `Editor::execute_batch` and `AppHost` use `command()` for their checks before the batch (`changes_model`, `replaces_document`, `discards`).

- [ ] **Step 1: Write the failing tests** in `crates/session/tests/batch_refs.rs`, using `BatchCommand::parse(json, index)` to build commands and `Document::execute_batch(Vec<BatchCommand>, atomic)`:
  - `a_rectangle_then_push_pull_of_its_face_makes_a_box`: `[add_rectangle (0,0,0)-(2,1,0), push_pull face "$0:f0" distance 1]`, atomic: ok, model has 6 faces, `history` has one undo step. Same batch non-atomic: ok, 6 faces.
  - `star_expands_in_place_in_id_lists`: `[add_rectangle, erase ids ["$0:e*"]]`: model has no edges and no faces afterwards.
  - `group_everything_a_command_made_then_paint_it`: `[add_rectangle, push_pull "$0:f0" 1, make_group ids ["$0:*", "$1:*"], add_material color [200,0,0], paint ids ["$2:i0"] material "$3:m0"]` atomic: ok; the root has exactly one instance, painted with the new material (query `entity` on it). (Review Focus 5: `$2:i0` is the only instance `make_group` created.)
  - `a_reference_past_what_was_made_fails_that_command_and_rolls_back`: `[add_rectangle, push_pull "$0:f1" 1]` atomic: `BatchError { index: 1, rolled_back: true }`, `error.kind == InvalidInput`, `error.message == "$0:f1: command 0 made 1 face"`; model empty, camera, selection and open context as before (set a selection and open nothing first, compare after). Non-atomic: `index 1`, `completed.len() == 1`, the rectangle stays.
  - `parse_rejects_bad_references` (each `BatchCommand::parse(...).unwrap_err()` contains the given text): `{"type":"push_pull","face":"$1:f0","distance":1}` at index 1 → `"$1:f0"` and `"earlier"`; `"$0:x0"` → `"invalid reference"`; `"$00:f0"` → `"invalid reference"`; `face: "$0:f*"` → `"list"`; `{"type":"delete_material","material":"$0:f0"}` at index 1 → `"material"`; `{"type":"erase","ids":["$0:m0"]}` at index 1 → `"material"`.
  - `reference_like_text_outside_id_fields_stays_text` (Review Focus 1): `make_component` with `ids ["$0:*"]` and `name "$0:f0"` at index 1 parses and, after executing `[add_rectangle, that]`, the `definitions` query lists a component named `"$0:f0"`; `BatchCommand::parse({"type":"open","path":"$0:f0"}, 1)` gives a command whose `path` is `"$0:f0"`.
  - `open_path_takes_references`: `[add_rectangle, make_group ids ["$0:*"], open_path path ["$1:i0"]]` non-atomic: ok and the open context's path is that instance.
  - `reference_keys_cover_every_id_field`: walk `schema_for!(Command)`; for every variant, every property whose schema refers to `EntityId` or `MaterialId` (directly, as array items, or inside `anyOf`/`oneOf` with null) is a reference key under ruling 2 (`path` only for `open_path`). Fails if a new command adds an id field under another name.
- [ ] **Step 2: Add to the host tests in `crates/app/src/host.rs`** (Review Focus 3), next to `a_batch_that_discards_keeps_the_work_at_that_command`: `a_batch_that_discards_resolves_references` — with `call_kept`, a non-atomic batch `[add_rectangle (0,0,0)-(1,1,0), push_pull "$0:f0" 1, {"type":"new","discard":true}, add_edge]` has 4 outcomes, one recovery file, and the recovery file (opened as in the neighbouring test) has 6 faces. This test goes through `api::call`'s `batch`, so it passes only after Task 2; mark it `#[ignore = "needs M12b task 2"]` here and Task 2 removes the attribute.
- [ ] **Step 3: Run** `cargo test -p session --test batch_refs` — expected: compile errors (`BatchCommand` undefined).
- [ ] **Step 4: Implement** `batch.rs`, `Document::execute_batch`, and the signature changes in `Editor`, `Host`, `HeadlessHost`, `AppHost`, `crates/api/tests/editor.rs` (wrap commands with `.into()` / `BatchCommand::from` where `Vec<Command>` was passed). `dispatch.rs`'s `batch` arm converts its `Vec<Command>` with `.into_iter().map(BatchCommand::from).collect()` for now; Task 2 replaces it.
- [ ] **Step 5: Run** the gate (fmt, clippy, `cargo test --workspace`) — expected: all pass, the new host test ignored.
- [ ] **Step 6: Commit**

```bash
git add crates/session/src/batch.rs crates/session/src/lib.rs crates/session/src/document.rs crates/session/tests/batch_refs.rs crates/app/src/editor.rs crates/app/src/host.rs crates/api/src/dispatch.rs crates/api/src/headless.rs crates/api/tests/editor.rs
git commit -m "Session: batch commands can refer to ids earlier commands made"
```

### Task 2: References over JSON-RPC and MCP, schemas and docs

**Files:**
- Modify: `crates/api/src/dispatch.rs` (`BatchParams`, the `batch` arm, `describe`'s `batch` entry and `schema_names`)
- Modify: `crates/api/src/schema.rs` (new helper)
- Modify: `crates/api/src/mcp/tools.rs` (`transact` schema and description)
- Modify: `crates/app/src/host.rs` (remove the `#[ignore]` from Task 1's test)
- Modify: `docs/agent-api.md`, `docs/mcp.md`
- Modify: `docs/superpowers/roadmap.md` (M12b row status is set by the controller at merge, not here)
- Test: `crates/api/tests/dispatch.rs`, `crates/api/tests/roots.rs`, `crates/api/tests/mcp.rs`

**Interfaces:**
- Consumes: `session::BatchCommand::{parse, command_mut}`, `STAND_IN_BASE` (Task 1).
- Produces: `pub fn with_batch_refs(defs: &mut serde_json::Map<String, Value>)` in `crates/api/src/schema.rs` (ruling 9: wraps the `EntityId` and `MaterialId` entries present in `defs` and inserts `BatchRef`). `BatchParams.commands` becomes `Vec<Value>` with `#[schemars(with = "Vec<Command>")]`.

- [ ] **Step 1: Write the failing tests.**
  - `crates/api/tests/dispatch.rs`:
    - `batch_references_build_a_box_over_json_rpc`: `batch` with `[add_rectangle, push_pull face "$0:f0" distance 1]` → result `outcomes` has 2 entries, `changes.created` has 6 face ids (`f…`), `revision` present.
    - `a_bad_reference_is_invalid_params_and_runs_nothing`: `batch` `[add_edge, push_pull face "$1:f0" distance 1]` → `error.code == -32602`, message starts with `"invalid params: commands[1]: "`; the `history` query's `revision` is the same before and after, and the model has no edges.
    - `a_reference_past_what_was_made_is_a_batch_failure`: `[add_rectangle, push_pull "$0:f3" 1]` → `error.code == -32001`, `error.data.index == 1`, `error.data.error.kind == "invalid_input"`, `error.data.rolled_back == true`.
    - `an_invalid_command_names_its_index`: `batch` `[add_edge, {"type":"no_such"}]` → `-32602`, message contains `"commands[1]"`.
    - extend `describe_lists_methods_and_schemas` (or a new `describe_batch_schema_accepts_references`): in `methods.batch.params_schema.$defs`, `EntityId` has an `anyOf` whose second entry is `{"$ref": "#/$defs/BatchRef"}`, and `BatchRef.pattern` equals the grammar in ruling 1; `methods.execute.params_schema` has no `BatchRef` def.
  - `crates/api/tests/roots.rs` (Review Focus 2): `a_referenced_material_keeps_the_texture_path_rewritten_under_roots` — with a root folder holding a small PNG (the file's `png_file` helper), batch `[add_material color [1,2,3], change_material material "$0:m0" texture {path: <relative name of the PNG>}]` succeeds and the `materials` query shows a texture on the new material.
  - `crates/api/tests/mcp.rs`: `transact_accepts_references` — `tools/call transact` with two commands `[add_rectangle, push_pull "$0:f0"]` is not `isError` and its `changes.created` has 6 faces; and in `tools/list`, `transact`'s `inputSchema.$defs.BatchRef` exists. The existing `tool_schemas_have_plain_object_roots` keeps passing.
- [ ] **Step 2: Run** `cargo test -p api` — expected: the new tests fail.
- [ ] **Step 3: Implement.** In the `batch` arm: parse each `Value` with `BatchCommand::parse(value, index)`, mapping `Err(reason)` to `RpcError::new(codes::INVALID_PARAMS, format!("invalid params: commands[{index}]: {reason}"))`; then `within_roots` on `command_mut()` (unchanged error shape); then `host.execute_batch`. In `describe`, the `batch` entry's `params_schema` goes through `with_batch_refs` on its `$defs`; add `"BatchRef"` to the named schemas. In `mcp/tools.rs`, call `with_batch_refs(&mut defs)` for `transact` and append to its description: `Two or more commands may refer to ids an earlier one of them made: "$0:f0" is the first face command 0 created, "$0:e*" all its edges, "$0:*" everything it created, "$1:m0" the material command 1 added.` Remove the `#[ignore]` from Task 1's host test.
- [ ] **Step 4: Docs.** `docs/agent-api.md`: a "References inside a batch" subsection after the `batch` method's notes, with the grammar, the reference keys, the error behaviour (rulings 1-6, 8, 10 in plain words) and one example (the box batch); remove the line "A failed atomic batch is rolled back but still consumes entity ids, so later ids may skip numbers." only if it is no longer needed for anything else — it is still true, so keep it and add after it: "Use references (`$0:f0`) rather than predicting ids." `docs/mcp.md`: one paragraph under `transact` with the box example.
- [ ] **Step 5: Run** the gate — expected: all pass, nothing ignored from this milestone.
- [ ] **Step 6: Commit**

```bash
git add crates/api/src/dispatch.rs crates/api/src/schema.rs crates/api/src/mcp/tools.rs crates/api/tests/dispatch.rs crates/api/tests/roots.rs crates/api/tests/mcp.rs crates/app/src/host.rs docs/agent-api.md docs/mcp.md
git commit -m "API: batch references over JSON-RPC and MCP, with schemas and docs"
```

### After the tasks (controller)

- Live check, as in M12a: `claude -p` with a temporary `--mcp-config` and `--strict-mcp-config`, asking for a 4 x 3 x 2.5 box as a group painted red in one `transact` call; it should take one call and no guessed ids.
- Roadmap row M12b → `done <first>..<last>`, memory note, merge `dev` into `main` and push (standing permission).
