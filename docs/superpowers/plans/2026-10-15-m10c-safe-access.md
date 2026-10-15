# M10c Safe Agent Access Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Giving an agent the socket stops being a blind, whole-filesystem capability: the window shows how many agents are connected and what they did (status-bar indicator, View > Agent Activity); an opt-in `--api-root DIR` (repeatable) keeps every path the API reads or writes inside those folders, resolved after symlinks; API saves and exports refuse to overwrite a file of another format unless `overwrite: true`; file errors never echo file contents; an API `new`/`open`/`quit` that discards unsaved window work first keeps it in a recovery file and the window says so; and the two minor holes the audit found (socket bound before it is made private, `.partial` temp file followed through a planted symlink) are closed.

**Architecture:**
- `session`: `file::replace_file` removes whatever is at `<path>.partial` and creates it with `create_new` (never through a link); read errors from OBJ, STL, `.skr` and texture files are worded without quoting the file; new module `sniff` recognises `.skr`, OBJ, MTL, STL, glTF and PNG/JPEG files, and `Save`/`Export` gain `overwrite` (default false) checked against it before anything is written; `Import` gains a hidden `library_roots` (never in JSON) that keeps `mtllib` reads inside the API roots; `Document::write_copy` writes the model to a path without making it the document's file.
- `api`: new module `roots` (`ApiRoots`: resolve a read or a write, `apply` to a `Command`); `Host::roots` (default None); `dispatch` applies roots to `execute` and every `batch` command before anything runs, and `query file` reports `api_roots`; `listen_unix` binds in a fresh 0700 folder, makes the socket 0600 and hard-links it into place; the `Hub` keeps an activity log (`Activity`, last 100, repeats folded, connects and disconnects).
- `app`: `--api-root DIR` (repeatable) in `cli`, resolved at start (`ApiRoots::new`, exit 2 on a bad folder); `AppHost` gains `roots` and `recovery` (the window's `~/.sketchup-alt/recovery`, None headless) and keeps discarded work before a discarding `new`/`open`/`quit` runs; the window's status bar shows "Agents: N" while any are connected, a View menu opens the Agent Activity window (connections, recovery files, the log newest first), which opens itself when a recovery file is written.

**Tech Stack:** Rust 2024; serde, serde_json, schemars, zip (already a session dependency), egui; no new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md` ("Agent control milestones", M10c). Security findings: the API audit's "Security notes" (path handling, overwrite of arbitrary files, contents echoed in import errors, discard dropping window work, bind-then-chmod window, `.partial` following a symlink). M10a's and M10b's plans (`docs/superpowers/plans/2026-10-15-m10a-agent-state.md`, `2026-10-15-m10b-tool-driving.md`) describe `Host`, `AppHost`, `Hub`, `EditorState` and `editor_state` that this builds on.

## Global Constraints

- No emoji anywhere (code, comments, docs, commit messages). Comments succinct, in the style of the surrounding code.
- `cargo fmt --all` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` passes after every task. Cargo is at `$HOME/.cargo/bin/cargo` (not on the subagents' PATH).
- No new external dependencies. Disk is limited: no extra worktrees or target directories.
- From the spec: the window shows when agents are connected (a status-bar indicator with the count, and a log of the last commands in View > Agent Activity). An opt-in `--api-root DIR` (repeatable) limits every path the API reads or writes (open, save, import, export, textures) to those folders, resolved after symlinks; relative paths resolve against the first root. Without roots, behaviour is unchanged. API saves and exports refuse to overwrite a file that is not of the target format unless `overwrite: true`. File errors never echo file contents. When an API `open`/`new` with `discard: true` drops unsaved window work, the dropped model is first written to a recovery file and the window says so.
- Git: work on `dev`; other agents may share the checkout: run `git status` and `git diff` first and stage only the files you changed (never `git add -A` or `git add .`); short plain commit messages, no attribution lines. Never push or merge.
- Do not launch the app window. Tests that start the binary use stdio or their own scratch sockets; never touch `~/.sketchup-alt/house.sock` or `h.sock` or their processes. Tests never write under `~/.sketchup-alt`: recovery tests pass their own scratch folder.
- Test files go under `env!("CARGO_TARGET_TMPDIR")` (integration tests) or `std::env::temp_dir()` with the process id in the name (unit tests), as the existing tests do.

## Review Focus

- A root given through a symlink or as a relative path: paths through the link's target are inside (Task 3 unit test in `roots.rs`: `ApiRoots::new` on a link to a folder, then `write("a.skr")` lands in the target folder).
- A write path that names no file (`dir/..`, `dir/.`, a trailing `/`, empty): refused with `invalid_input`, never written as the folder (Task 3 unit test: `write("..")`, `write("sub/.")`, `write("")` are errors).
- An OBJ export under roots whose `.mtl` or image name beside it is a link pointing outside: the link is replaced, its target untouched (Task 1 test `a_link_at_the_partial_name...` covers `.partial`; add to Task 2's export tests: a link `lib-link.mtl -> outside file`, export with `overwrite: true`, the outside file keeps its text).
- A recovery folder that is a symlink: refused, nothing discarded (Task 5 test: `ensure_private_dir` refuses it; make `recovery_dir("link")` a link to a 0700 folder and expect the `file` error).
- A request that fails before reaching the host (bad params, quitting) still shows in the activity log with its error (Task 4 test: send `{"type": "nope"}` and check the entry's `error` is the `-32602` message).

## Spec notes (rulings made in this plan)

- **Which paths `--api-root` covers.** Every path an API request names: `open.path`, `save.path` (and the open file when `save` has none), `export.path` (the `.mtl` and texture images an OBJ export writes go beside it, so in the same folder), `import.path` and the `mtllib` libraries the OBJ names, `add_material`/`change_material` `texture.path`. Not covered: the window's own dialogs, the FILE argument, the socket path, the recovery folder (the app's own writes). `tool_input` cannot reach files (its Cmd keys are only undo/redo).
- **Resolving.** Roots are resolved after symlinks once, at start; a root that is not an existing folder stops the app with exit code 2 and a message. A relative path is joined to the first root; an absolute one is taken as is. A read (open, import, texture) resolves the whole path after symlinks (the file must exist) and must lie within a root. A write (save, export) resolves its folder after symlinks (the folder must exist) and must lie within a root; the file itself may not be a symbolic link (refused: renaming over it would replace the link, but the user who made the link meant the target). The command then runs on the resolved path (so a later swap of a link in the path changes nothing already checked; the window title and `query file` show the resolved path). A path outside every root is `invalid_input` ("... is outside the folders the API may use (--api-root)"); a path that cannot be resolved (missing file or folder) is `file`, naming the path as given. An import names its group after the resolved file.
- **`mtllib` under roots.** A library is read only if it resolves, after symlinks, within a root; others are skipped as unreadable libraries are now (the faces keep the default material). Carried to `session` in `Command::Import`'s hidden `library_roots` field (`#[serde(skip)]`: never read from or written to JSON, absent from the schema), set only by the API.
- **`save` without `path` under roots** is checked as a write to the open model's file when the request arrives, and runs on that resolved path. Inside a `batch` it is refused (`invalid_input`, "give save a path: ..."), because an earlier command of the batch may open another file. A batch whose command `i` names a path outside the roots fails before anything runs: `-32001` with `BatchError {index: i, error, completed: [], rolled_back: false}`.
- **Without roots** behaviour is unchanged: relative paths resolve against the app's working directory, as documented. (The spec sentence "or are refused when no root and no absolute path" is read with "Without roots, behaviour is unchanged", which governs.)
- **Agents can see the roots:** `query file` adds `api_roots`: the resolved folders, or null without `--api-root`. `describe` gets a `paths` note.
- **Overwrite guard.** `save` and `export` gain `overwrite` (default false). Without it, an existing target must be of the target format, or empty; a missing target is fine. Formats are recognised from at most the first 64 KiB (and the size): `.skr` = a zip archive with a `document.json` entry; OBJ = UTF-8 text whose every complete line is blank, a `#` comment or starts with an OBJ keyword, with at least one keyword line; MTL = the same with MTL keywords and at least one `newmtl`; STL = text starting (after spaces) with `solid` in any case, or a binary STL whose size is 84 + 50 x its triangle count; `glb` = starts with `glTF`; `gltf` = starts (after spaces) with `{` and has `"asset"` in the first 64 KiB; texture image beside an OBJ = PNG or JPEG signature. Something that is not a regular file (a folder, a device) is refused. An OBJ export checks its `.mtl` and every image it would write the same way, all before writing anything. A refusal is `invalid_input`: "PATH exists and is not a FORMAT file: pass \"overwrite\": true to replace it". The window's Save, Save As and Export pass `overwrite: true` (its dialogs ask before replacing). A link at the target is judged by the file it points to (the write itself replaces the link, as before).
- **No file contents in errors.** No error message quotes text read from a file: OBJ and STL number and index errors say what was wrong and where (`line 3: a coordinate is not a finite number`), `.skr` JSON errors give only line and column and the kind of fault, `.skr` texture errors name the material id but not the entry name the file gives, and image decoder errors become fixed wording ("the image is broken", "the image's header is broken"). Paths given by the caller, ids, counts, limits and system error text (`No such file or directory`) stay. This applies to every caller, window included.
- **Recovery file.** Kept only in the window (headless, the API's own work is all there is; nothing is written). Before an API `new`, `open` or `quit` with `discard: true` runs while the model has unsaved changes, the model is written to `~/.sketchup-alt/recovery/<name>-<unix seconds>.skr` (`<name>` the open file's stem or `untitled`, characters other than letters, digits, `-`, `_` and space replaced by `_`; `-2`, `-3`, ... added when taken), with `~/.sketchup-alt` and `recovery` created 0700 and refused if open to others or a link. `quit` is included (the spec names `open`/`new`; `quit {discard: true}` drops the same work). Non-atomic batches holding a discarding `new`/`open` are run one command at a time so the work is kept at that command's turn. If the file cannot be written the command fails (`file`, "could not keep the unsaved changes in a recovery file (...): nothing was discarded"); if the command then fails (a missing file to open), the recovery file is removed (nothing was dropped). Afterwards the status bar says "An agent discarded unsaved changes; they were kept in PATH", the Agent Activity window opens and lists the file (all recovery files of the session), and stderr gets the same line. The API reply is unchanged.
- **Indicator.** "Agents: N" at the right of the status bar while N > 0 socket connections are open (each open connection counts, whether or not it sent anything); hidden at 0. Clicking it opens Agent Activity. Hover text: "Programs connected to the agent API".
- **Activity log.** The `Hub` keeps the last 100 entries (`ACTIVITY_LEN`), in order: each connection opening and closing (`connected`, `disconnected`), and every request it handled: `method`, then the command or query `type`, then for `open`/`save`/`export`/`import` the path as given (`execute save /a/house.skr`); `batch (N commands)`; `tool_input (N events)`; a line that is not JSON is `(not JSON)`. Each entry has the connection id, the time it was handled (`Instant`), and the error message when the reply was an error. A request equal to the last entry (same connection, request text and error) increments its `repeats` instead (an agent polling `query camera` makes one line). Request text is cut to 160 characters. The window lists entries newest first as `12 s ago  #3  execute save /a/house.skr  ok` (or `error: ...`, and ` (x12)` when repeated). Headless keeps the log but shows nothing. `describe` is logged like any request.
- **Socket bind.** `listen_unix` binds the socket inside a fresh folder `.sa<pid>-<n>` created 0700 beside the target, sets it 0600, then hard-links it to the target path (failing, not replacing, if something appeared there) and removes the folder; so the socket is never reachable at a mode the umask chose. The stale-socket and non-socket checks stay.
- **`.partial` file.** `replace_file` removes whatever is at `<path>.partial` (a link itself, not its target) and creates the partial file with `create_new`, so the bytes never go through a planted link; the rest (sync, rename, parent sync) is unchanged.
- **describe** keeps `schema_version` 1 (additions only): `overwrite` appears in the `Command` schema; a `paths` note; the `file` query result's `api_roots` is documented in agent-api.md.

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/session/src/file.rs` | safe `replace_file`; `.skr` read errors without contents | 1 |
| `crates/session/src/import.rs`, `texture.rs` | OBJ/STL/image errors without contents | 1 |
| `crates/session/src/sniff.rs` (new), `lib.rs` | format recognisers | 2 |
| `crates/session/src/command.rs` | `overwrite` on Save/Export; hidden `library_roots` on Import | 2, 3 |
| `crates/session/src/document.rs`, `document/files.rs`, `document/import.rs` | guard; `write_copy`; library roots | 2, 3, 5 |
| `crates/app/src/editor.rs` | window passes `overwrite: true`, `library_roots: None`; recovery note | 2, 3, 5 |
| `crates/api/src/roots.rs` (new), `lib.rs`, `dispatch.rs`, `headless.rs` | `ApiRoots`, `Host::roots`, applying, `api_roots` in `query file`, describe | 3 |
| `crates/app/src/cli.rs`, `main.rs`, `headless.rs`, `app.rs`, `host.rs` | `--api-root`, AppHost fields | 3, 5 |
| `crates/api/src/transport.rs` | private bind | 4 |
| `crates/api/src/hub.rs` | activity log | 4 |
| `crates/app/src/ui.rs`, `app.rs` | indicator, View menu, Agent Activity window | 5 |
| `docs/agent-api.md` | Starting (`--api-root`), Files, Export, a new "Access and safety" section, Window actions | 1-5 |

---

### Task 1: Safe partial files and file errors without contents

**Files:**
- Modify: `crates/session/src/file.rs` (`replace_file`, `read`, `texture_entry`)
- Modify: `crates/session/src/import.rs` (`read_obj` corner index error, `number`)
- Modify: `crates/session/src/texture.rs` (`dimensions`, `decode`)
- Modify: `docs/agent-api.md` (Files: "A file that cannot be read ... gives the error kind `file`" paragraph)
- Test: `crates/session/tests/files.rs`, `crates/session/tests/import.rs`, `crates/session/tests/textures.rs`, `crates/session/tests/export.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `file::replace_file(path, bytes)` (same signature) never writes through a link at `<path>.partial`. Error wording: OBJ `line N: a coordinate is not a finite number`, `line N: a corner index is not a whole number`; STL `facet N: a coordinate is not a finite number`; `.skr` `the document is not valid JSON (line L, column C)` / `the document has a value of the wrong kind or an unknown name (line L, column C)` / `the document ends early`; texture `the image's header is broken` / `the image is broken`; `.skr` texture `material mID's texture is missing` / `material mID's texture: REASON`.

- [ ] **Step 1: Write the failing tests**

In `crates/session/tests/files.rs` (uses its `doc`, `run`, `scratch` helpers):

```rust
#[test]
fn a_link_at_the_partial_name_is_not_written_through() {
    let mut d = doc();
    run(&mut d, json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]})).unwrap();
    let victim = scratch("partial-victim.txt");
    std::fs::write(&victim, "keep me").unwrap();
    let target = scratch("partial-target.skr");
    let partial = scratch("partial-target.skr.partial");
    std::os::unix::fs::symlink(&victim, &partial).unwrap();
    run(&mut d, json!({"type": "save", "path": target})).unwrap();
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
    assert!(std::fs::symlink_metadata(&partial).is_err(), "the link is gone");
    let mut opened = doc();
    run(&mut opened, json!({"type": "open", "path": target})).unwrap();
}

/// A `.skr` archive holding `document.json` with `json` as its text.
fn skr_with(path: &Path, json: &str) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(json.as_bytes()).unwrap();
    zip.finish().unwrap();
}

#[test]
fn open_errors_do_not_quote_the_file() {
    let path = scratch("secret.skr");
    let cases = [
        r#"{"format": "sketchup-alt", "schema_version": 9, "camera": "hunter2-SECRET"}"#,
        r#"{"format": "sketchup-alt", "schema_version": 9, "hunter2-SECRET": 1}"#,
        r#"{"format": "sketchup-alt", "schema_version": 9, "camera": {"eye": [hunter2-SECRET"#,
    ];
    for json in cases {
        skr_with(&path, json);
        let (kind, message) =
            run(&mut doc(), json!({"type": "open", "path": path})).unwrap_err();
        assert_eq!(kind, ErrorKind::File);
        assert!(!message.contains("hunter2"), "{message}");
        assert!(message.contains("line"), "{message}");
    }
}
```

In `crates/session/tests/import.rs` (uses its `doc`, `run`, `scratch`):

```rust
#[test]
fn import_errors_do_not_quote_the_file() {
    let cases: [(&str, &str); 4] = [
        ("secret1.obj", "v 0 0 0\nv hunter2-SECRET 0 0\n"),
        ("secret2.obj", "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 hunter2-SECRET\n"),
        ("secret3.stl", "solid x\nfacet normal 0 0 1\nouter loop\nvertex hunter2-SECRET 0 0\n"),
        ("secret4.obj", "hunter2-SECRET\nv nan 0 0\n"),
    ];
    for (name, text) in cases {
        let path = scratch(name);
        std::fs::write(&path, text).unwrap();
        let (_, message) = run(&mut doc(), json!({"type": "import", "path": path})).unwrap_err();
        assert!(!message.contains("hunter2"), "{name}: {message}");
        assert!(message.contains("line ") || message.contains("facet "), "{message}");
    }
}
```

In `crates/session/tests/textures.rs` (uses its `doc`, `run`, `scratch`):

```rust
#[test]
fn texture_errors_do_not_quote_the_file() {
    let path = scratch("secret.png");
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(b"hunter2-SECRET hunter2-SECRET hunter2-SECRET");
    std::fs::write(&path, &bytes).unwrap();
    let (_, message) = run(
        &mut doc(),
        json!({"type": "add_material", "texture": {"path": path}}),
    )
    .unwrap_err();
    assert!(!message.contains("hunter2"), "{message}");
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p session --test files --test import --test textures -- secret quote partial`
Expected: `a_link_at_the_partial_name_is_not_written_through` FAILS (victim now holds the zip), the quote tests FAIL on `hunter2` in the message (at least the OBJ, STL and unknown-field cases).

- [ ] **Step 3: Implement**

`crates/session/src/file.rs`, replace `replace_file`:

```rust
/// Writes `bytes` to `path`, replacing the file there only once they are all on disk. The
/// bytes go to `<path>.partial` first: whatever is at that name (a file left by a failed
/// write, or a link planted there) is removed, the link itself and not what it points to, and
/// the file is created anew, so the bytes never go through a link.
pub(crate) fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut name = path.as_os_str().to_owned();
    name.push(".partial");
    let partial = Path::new(&name);
    match std::fs::remove_file(partial) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(partial)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        // The data must be on disk before the rename makes it the file.
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(partial, path).map_err(|e| e.to_string())?;
        sync_parent(path);
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(partial);
    }
    result
}
```

In `read`, replace both `serde_json::from_slice(&json).map_err(|e| e.to_string())?` with `.map_err(json_error)?`, and add:

```rust
/// A document that does not parse, said without quoting it: serde's own messages quote the
/// file's text (a string of the wrong kind, an unknown field's name).
fn json_error(e: serde_json::Error) -> String {
    use serde_json::error::Category;
    let (line, column) = (e.line(), e.column());
    match e.classify() {
        Category::Eof | Category::Io => "the document ends early".into(),
        Category::Syntax => format!("the document is not valid JSON (line {line}, column {column})"),
        Category::Data => format!(
            "the document has a value of the wrong kind or an unknown name (line {line}, column {column})"
        ),
    }
}
```

`texture_entry` and the two `format!("material m{}'s texture {}: {e}", m.id, t.file)` in `read`: drop the entry name (it is text from the file):

```rust
fn texture_entry(
    zip: &mut zip::ZipArchive<std::fs::File>,
    id: u64,
    file: &str,
) -> Result<Vec<u8>, String> {
    let entry = zip
        .by_name(file)
        .map_err(|_| format!("material m{id}'s texture is missing"))?;
    read_limited(entry, MAX_TEXTURE_BYTES, "the texture")
        .map_err(|e| format!("material m{id}'s texture: {e}"))
}
```

and in `read`: `.map_err(|e| format!("material m{}'s texture: {e}", m.id))` in both places.

`crates/session/src/import.rs`: the corner index error becomes `at("a corner index is not a whole number".into())`, and

```rust
/// A finite number. The error does not quote the text (it is the file's).
fn number(t: &str) -> Result<f64, String> {
    t.parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
        .ok_or_else(|| "a coordinate is not a finite number".to_string())
}
```

Then read every other `Err(`/`format!` in `import.rs`, `document/import.rs` and `kernel::Scene::from_data` (`crates/kernel/src/data.rs`, and the material name and texture checks it calls) and confirm none quotes text read from the file (names, tokens); ids, counts and limits are fine. Fix any that does in the same way.

`crates/session/src/texture.rs`: in `dimensions`, `with_guessed_format().map_err(|_| "the image's header is broken".to_string())` and `into_dimensions().map_err(|_| "the image's header is broken".to_string())`; in `decode`, `with_guessed_format().map_err(|_| "the image is broken".to_string())` and `reader.decode().map_err(|_| "the image is broken".to_string())`. (Decoder messages can carry bytes of the file, such as chunk names.)

Update existing tests whose expected messages quoted file text (search the session tests for `is not a finite number`, `is not a corner index`, `texture textures/`, `unknown field`, `invalid type`), to the new wording.

`docs/agent-api.md`, Files, after "... or a face texture position out of range);": add "error messages never quote the file's contents (a JSON fault gives its line and column);". Import section: where it lists import errors, add the same sentence for OBJ and STL ("a number or index that cannot be read is reported by its line or facet, without quoting it").

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p session`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status --short && git diff --stat
git add crates/session/src/file.rs crates/session/src/import.rs crates/session/src/texture.rs crates/session/tests/files.rs crates/session/tests/import.rs crates/session/tests/textures.rs docs/agent-api.md
git commit -m "Partial files never written through a link; file errors do not quote file contents"
```
(Add any other test or source file you changed by name.)

---

### Task 2: Overwrite guard for save and export

**Files:**
- Create: `crates/session/src/sniff.rs`
- Modify: `crates/session/src/lib.rs` (`mod sniff;`)
- Modify: `crates/session/src/command.rs` (`Save`, `Export` gain `overwrite`)
- Modify: `crates/session/src/document.rs` (pass `overwrite` through), `crates/session/src/document/files.rs` (`save_file`, `export_file`)
- Modify: `crates/app/src/editor.rs` (`save_file`, `export_file` pass `overwrite: true`)
- Modify: `docs/agent-api.md` (Files table row for `save`, Export section)
- Test: `crates/session/tests/files.rs`, `crates/session/tests/export.rs`

**Interfaces:**
- Consumes: Task 1's `replace_file`.
- Produces: `Command::Save { path: Option<String>, overwrite: bool }`, `Command::Export { path: String, format: Option<ExportFormat>, overwrite: bool }` (`#[serde(default)]`). `sniff::Kind { Skr, Obj, Mtl, Stl, Glb, Gltf, Image }`, `sniff::replaceable(path: &Path, kind: Kind) -> bool`, `sniff::refusal(path: &Path, kind: Kind) -> CommandError`.

- [ ] **Step 1: Write the failing tests**

`crates/session/tests/files.rs`:

```rust
#[test]
fn save_does_not_overwrite_another_kind_of_file_unless_told() {
    let mut d = doc();
    run(&mut d, json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]})).unwrap();
    let victim = scratch("victim.txt");
    std::fs::write(&victim, "notes").unwrap();
    let (kind, message) = run(&mut d, json!({"type": "save", "path": victim})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("\"overwrite\": true"), "{message}");
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "notes");
    // A model file, an empty file and a missing one are fine.
    let model = scratch("model.skr");
    run(&mut d, json!({"type": "save", "path": model})).unwrap();
    run(&mut d, json!({"type": "save", "path": model})).unwrap();
    run(&mut d, json!({"type": "save"})).unwrap();
    let empty = scratch("empty.skr");
    std::fs::write(&empty, "").unwrap();
    run(&mut d, json!({"type": "save", "path": empty})).unwrap();
    // A zip that is not a model is another kind of file.
    let other = scratch("other.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&other).unwrap());
    zip.start_file("a.txt", zip::write::SimpleFileOptions::default()).unwrap();
    zip.finish().unwrap();
    assert!(run(&mut d, json!({"type": "save", "path": other})).is_err());
    run(&mut d, json!({"type": "save", "path": victim, "overwrite": true})).unwrap();
    assert!(std::fs::read(&victim).unwrap().starts_with(b"PK"));
}

#[test]
fn save_refuses_a_folder() {
    let mut d = doc();
    let dir = scratch("a-folder");
    std::fs::create_dir_all(&dir).unwrap();
    let (kind, _) = run(&mut d, json!({"type": "save", "path": dir})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
}
```

`crates/session/tests/export.rs` (uses its `doc`, `run`, `scratch`; build a box with a painted face as the existing OBJ material tests do; call that helper `painted_box(&mut d)` or reuse the existing one):

```rust
#[test]
fn export_does_not_overwrite_another_kind_of_file_unless_told() {
    let mut d = doc();
    painted_box(&mut d);
    for (name, format) in [("v.txt", "stl"), ("v2.txt", "obj"), ("v3.txt", "glb"), ("v4.txt", "gltf")] {
        let victim = scratch(name);
        std::fs::write(&victim, "notes").unwrap();
        let (kind, _) = run(&mut d, json!({"type": "export", "path": victim, "format": format}))
            .unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{format}");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "notes");
        run(&mut d, json!({"type": "export", "path": victim, "format": format, "overwrite": true}))
            .unwrap();
        // Its own format is replaced freely.
        run(&mut d, json!({"type": "export", "path": victim, "format": format})).unwrap();
    }
}

#[test]
fn an_obj_export_checks_its_library_before_writing_anything() {
    let mut d = doc();
    painted_box(&mut d);
    let obj = scratch("lib-guard.obj");
    let mtl = scratch("lib-guard.mtl");
    std::fs::write(&mtl, "notes").unwrap();
    let (kind, message) = run(&mut d, json!({"type": "export", "path": obj})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("lib-guard.mtl"), "{message}");
    assert!(!obj.exists());
    assert_eq!(std::fs::read_to_string(&mtl).unwrap(), "notes");
    run(&mut d, json!({"type": "export", "path": obj, "overwrite": true})).unwrap();
    run(&mut d, json!({"type": "export", "path": obj})).unwrap();
}

#[test]
fn stl_files_of_either_kind_are_recognised() {
    let mut d = doc();
    painted_box(&mut d);
    let ascii = scratch("ascii.stl");
    std::fs::write(&ascii, "  SOLID x\nendsolid x\n").unwrap();
    run(&mut d, json!({"type": "export", "path": ascii})).unwrap();
    let binary = scratch("binary.stl");
    let mut bytes = vec![0u8; 80];
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 50]);
    std::fs::write(&binary, &bytes).unwrap();
    run(&mut d, json!({"type": "export", "path": binary})).unwrap();
    let wrong = scratch("wrong.stl");
    bytes.push(0);
    std::fs::write(&wrong, &bytes).unwrap();
    assert!(run(&mut d, json!({"type": "export", "path": wrong})).is_err());
}
```

Also add unit tests at the bottom of `sniff.rs` (Step 3) for OBJ/MTL recognition: an exported-looking OBJ head (`# x\nmtllib a.mtl\nv 0 0 0\nvn 0 0 1\nusemtl m\nf 1//1 1//1 1//1\n`) is Obj; `newmtl a\nKd 1 1 1\nmap_Kd a.png\n` is Mtl; `hello world\n` is neither; `# only a comment\n` is not Obj; PNG and JPEG signatures are Image; a 64 KiB head cutting a UTF-8 character in two still reads as OBJ.

- [ ] **Step 2: Run them to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p session --test files --test export -- overwrite folder library recognised`
Expected: FAIL (`overwrite` is an unknown field; the victim is overwritten).

- [ ] **Step 3: Implement**

`crates/session/src/sniff.rs`:

```rust
//! Telling whether a file already at a path is of a given format, so `save` and `export`
//! do not replace other kinds of files by mistake.

use std::io::Read;
use std::path::Path;

use crate::command::{CommandError, ErrorKind};

/// How much of a file is read to recognise it.
const HEAD: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Skr,
    Obj,
    Mtl,
    Stl,
    Glb,
    Gltf,
    /// A PNG or JPEG image.
    Image,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Skr => "sketchup-alt model",
            Kind::Obj => "OBJ",
            Kind::Mtl => "MTL material library",
            Kind::Stl => "STL",
            Kind::Glb => "binary glTF",
            Kind::Gltf => "glTF",
            Kind::Image => "PNG or JPEG",
        }
    }
}

const OBJ_KEYWORDS: &[&str] = &[
    "v", "vt", "vn", "vp", "f", "l", "p", "o", "g", "s", "mtllib", "usemtl", "cstype", "deg",
    "curv", "curv2", "surf", "parm", "end", "bmat", "step", "trim", "hole", "scrv", "sp",
    "con", "mg", "lod", "shadow_obj", "trace_obj",
];
const MTL_KEYWORDS: &[&str] = &[
    "newmtl", "Ka", "Kd", "Ks", "Ke", "Ns", "Ni", "d", "Tr", "Tf", "illum", "bump", "disp",
    "decal", "refl", "sharpness", "Pr", "Pm", "Ps", "Pc", "Pcr", "aniso", "anisor", "norm",
];

/// True when `path` may be replaced by a file of `kind`: nothing is there, or an empty
/// regular file, or a regular file of that kind. A link is judged by what it points to.
pub(crate) fn replaceable(path: &Path, kind: Kind) -> bool {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => return e.kind() == std::io::ErrorKind::NotFound,
    };
    if !meta.is_file() {
        return false;
    }
    if meta.len() == 0 {
        return true;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    if kind == Kind::Skr {
        return zip::ZipArchive::new(file).is_ok_and(|mut z| z.by_name("document.json").is_ok());
    }
    let mut head = Vec::new();
    if file.take(HEAD).read_to_end(&mut head).is_err() {
        return false;
    }
    is_kind(&head, meta.len(), kind)
}

/// Whether a file of `len` bytes starting with `head` is of `kind` (not `Skr`).
fn is_kind(head: &[u8], len: u64, kind: Kind) -> bool {
    match kind {
        Kind::Skr => false,
        Kind::Glb => head.starts_with(b"glTF"),
        Kind::Image => head.starts_with(b"\x89PNG\r\n\x1a\n") || head.starts_with(&[0xFF, 0xD8, 0xFF]),
        Kind::Stl => {
            let text = head.trim_ascii_start();
            text.get(..5).is_some_and(|w| w.eq_ignore_ascii_case(b"solid"))
                || (len >= 84
                    && head.len() >= 84
                    && u64::from(u32::from_le_bytes(head[80..84].try_into().expect("4 bytes")))
                        * 50
                        + 84
                        == len)
        }
        Kind::Gltf => {
            head.trim_ascii_start().starts_with(b"{")
                && head.windows(7).any(|w| w == b"\"asset\"")
        }
        Kind::Obj => keyword_text(head, len, OBJ_KEYWORDS, None),
        Kind::Mtl => keyword_text(head, len, MTL_KEYWORDS, Some("newmtl")),
    }
}

/// UTF-8 text whose complete lines are blank, `#` comments or start with one of `keywords`
/// (or `map_...`, for MTL), with at least one keyword line and, if `needed`, that keyword.
fn keyword_text(head: &[u8], len: u64, keywords: &[&str], needed: Option<&str>) -> bool {
    let text = match std::str::from_utf8(head) {
        Ok(t) => t,
        // A head cut inside a character.
        Err(e) if e.error_len().is_none() => {
            std::str::from_utf8(&head[..e.valid_up_to()]).expect("valid up to here")
        }
        Err(_) => return false,
    };
    let mut lines: Vec<&str> = text.lines().collect();
    // The last line may be cut short when the file goes on past the head.
    if (head.len() as u64) < len && !text.ends_with('\n') {
        lines.pop();
    }
    let (mut any, mut has_needed) = (false, needed.is_none());
    for line in lines {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let word = line.split_whitespace().next().expect("not blank");
        let known = keywords.contains(&word) || (needed.is_some() && word.starts_with("map_"));
        if !known {
            return false;
        }
        any = true;
        has_needed |= needed == Some(word);
    }
    any && has_needed
}

/// The error for a target that is not of `kind`.
pub(crate) fn refusal(path: &Path, kind: Kind) -> CommandError {
    CommandError::new(
        ErrorKind::InvalidInput,
        format!(
            "{} exists and is not a {} file: pass \"overwrite\": true to replace it",
            path.display(),
            kind.name()
        ),
    )
}
```

(Format the keyword arrays as `rustfmt` leaves them. Add the unit tests listed in Step 1 in a `#[cfg(test)] mod tests` calling `is_kind`.)

`crates/session/src/command.rs`: `Save` and `Export` gain, after their existing fields:

```rust
        /// Replace an existing file even when it is not a `.skr` model (without it such a
        /// file is refused; an empty or missing file is always fine).
        #[serde(default)]
        overwrite: bool,
```

(for `Export`: "even when it is not of the format written; for an OBJ, also its `.mtl` and images"). Update the variants' doc comments to say so.

`crates/session/src/document.rs`: where `Command::Save { path }` and `Command::Export { path, format }` are matched, take `overwrite` and pass it to `save_file(path.as_deref(), overwrite)` / `export_file(path, *format, overwrite)`.

`crates/session/src/document/files.rs`: `save_file(&mut self, path: Option<&str>, overwrite: bool)`: after computing `target`, before writing:

```rust
        if !overwrite && !sniff::replaceable(&target, Kind::Skr) {
            return Err(sniff::refusal(&target, Kind::Skr));
        }
```

`export_file(&self, path, format, overwrite: bool)`: after the format, `.skr` and own-file checks, compute the kind (`ExportFormat::Obj => Kind::Obj`, `Stl => Kind::Stl`, `Glb => Kind::Glb`, `Gltf => Kind::Gltf`) and, once `encoded` exists and before the first `replace_file`:

```rust
        if !overwrite {
            let mut checks = vec![(target.to_path_buf(), kind)];
            if encoded.mtl.is_some() {
                checks.push((mtl_path.clone(), Kind::Mtl));
                for (name, _) in &encoded.images {
                    checks.push((mtl_path.with_file_name(name), Kind::Image));
                }
            }
            if let Some((p, k)) = checks.into_iter().find(|(p, k)| !sniff::replaceable(p, *k)) {
                return Err(sniff::refusal(&p, k));
            }
        }
```

(Place it after the existing `same(&mtl_path, target) || own(&mtl_path)` check so that error keeps precedence.)

`crates/app/src/editor.rs`: `Command::Save { path: ..., overwrite: true }` and `Command::Export { path: ..., format: None, overwrite: true }` with a comment: "The file dialog asked before replacing a file." Fix every other `Command::Save {`/`Command::Export {` literal the compiler reports (tests in `session/src/command.rs`, `document.rs`).

`docs/agent-api.md`: Files table `save` row: add "An existing file at `path` that is not a model (any other file but an empty one) is refused with `invalid_input` unless `\"overwrite\": true`." Export section: after "export never overwrites a model; use `save`.": "An existing file at `path` (and, for an OBJ with materials, at its `.mtl` and image paths) that is not of the format written is refused with `invalid_input` unless `\"overwrite\": true`; all are checked before anything is written. An empty file is always replaced. Formats are recognised from their first 64 KiB: ..." (copy the list from the Spec notes).

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p session && $HOME/.cargo/bin/cargo test -p app && $HOME/.cargo/bin/cargo test -p api`
Expected: PASS (fix any existing test that exported or saved over a file of another kind by adding `"overwrite": true` only where that was its intent).

- [ ] **Step 5: Commit**

```bash
git status --short && git diff --stat
git add crates/session/src/sniff.rs crates/session/src/lib.rs crates/session/src/command.rs crates/session/src/document.rs crates/session/src/document/files.rs crates/app/src/editor.rs crates/session/tests/files.rs crates/session/tests/export.rs docs/agent-api.md
git commit -m "Save and export refuse to replace a file of another format unless overwrite"
```

---

### Task 3: `--api-root` folders

**Files:**
- Create: `crates/api/src/roots.rs`
- Modify: `crates/api/src/lib.rs` (`mod roots; pub use roots::ApiRoots;`), `crates/api/src/dispatch.rs`, `crates/api/src/headless.rs`
- Modify: `crates/session/src/command.rs` (`Import.library_roots`), `crates/session/src/document.rs`, `crates/session/src/document/import.rs`
- Modify: `crates/app/src/cli.rs`, `crates/app/src/main.rs`, `crates/app/src/headless.rs`, `crates/app/src/app.rs`, `crates/app/src/host.rs` (and `host/tool_input_tests.rs` where it builds `AppHost`), `crates/app/src/editor.rs` (`library_roots: None`), `crates/render/tests/golden.rs` (`library_roots: None`)
- Modify: `docs/agent-api.md` (Starting table and a new "### Access and safety" section before "### Notifications")
- Test: `crates/api/tests/roots.rs` (new), `crates/app/src/cli.rs` tests, `crates/app/tests/headless_agent.rs`

**Interfaces:**
- Consumes: `Command` from Task 2 (with `overwrite`).
- Produces:
  - `api::ApiRoots`: `ApiRoots::new(dirs: &[PathBuf]) -> Result<ApiRoots, String>`, `Default` (no roots), `is_empty(&self) -> bool`, `dirs(&self) -> &[PathBuf]`, `read(&self, path: &str) -> Result<PathBuf, CommandError>`, `write(&self, path: &str) -> Result<PathBuf, CommandError>`, `apply(&self, command: &mut Command, open_file: Option<&Path>, in_batch: bool) -> Result<(), CommandError>`.
  - `Host::roots(&self) -> Option<&ApiRoots>` (default None).
  - `HeadlessHost::with_roots(roots: ApiRoots) -> HeadlessHost`.
  - `Command::Import { ..., library_roots: Option<Vec<PathBuf>> }` (`#[serde(skip)]`).
  - `cli::Mode::Window { socket, file, api_roots: Vec<PathBuf> }`, `cli::Mode::Headless { transport, file, api_roots: Vec<PathBuf> }`.
  - `AppHost { editor, offscreen, quit, roots: &'a ApiRoots }` (Task 5 adds `recovery`).
  - `headless::run(transport, file, roots: ApiRoots) -> i32`; `App::new(event_loop, socket, file, roots: ApiRoots)`.

- [ ] **Step 1: Write the failing tests**

`crates/api/tests/roots.rs`:

```rust
//! `--api-root`: every path the API reads or writes stays inside the roots.

use std::path::PathBuf;

use api::{ApiRoots, HeadlessHost, codes, handle};
use serde_json::{Value, json};

fn call(host: &mut HeadlessHost, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    serde_json::from_str(&handle(host, &line).expect("a reply")).unwrap()
}

/// A fresh folder for one test, with `root` and `outside` inside it.
fn folders(name: &str) -> (PathBuf, PathBuf) {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("api-roots-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (root, outside) = (base.join("root"), base.join("outside"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    (root, outside)
}

fn host(root: &PathBuf) -> HeadlessHost {
    HeadlessHost::with_roots(ApiRoots::new(std::slice::from_ref(root)).unwrap())
}

fn edge(h: &mut HeadlessHost) {
    call(h, "execute", json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}));
}

#[test]
fn relative_paths_land_in_the_first_root() {
    let (root, _) = folders("relative");
    let mut h = host(&root);
    edge(&mut h);
    let r = call(&mut h, "execute", json!({"type": "save", "path": "a.skr"}));
    assert!(r["result"].is_object(), "{r}");
    assert!(root.join("a.skr").exists());
    let r = call(&mut h, "execute", json!({"type": "open", "path": "a.skr"}));
    assert!(r["result"].is_object(), "{r}");
    let file = call(&mut h, "query", json!({"type": "file"}));
    let real = root.canonicalize().unwrap();
    assert_eq!(file["result"]["api_roots"], json!([real]));
    assert_eq!(file["result"]["path"], json!(real.join("a.skr")));
}

#[test]
fn paths_outside_the_roots_are_refused() {
    let (root, outside) = folders("outside");
    let mut h = host(&root);
    edge(&mut h);
    std::fs::write(outside.join("model.obj"), "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
    let cases = [
        json!({"type": "save", "path": outside.join("a.skr")}),
        json!({"type": "save", "path": "../outside/a.skr"}),
        json!({"type": "export", "path": outside.join("a.stl")}),
        json!({"type": "import", "path": outside.join("model.obj")}),
        json!({"type": "open", "path": "../outside/model.obj"}),
        json!({"type": "add_material", "texture": {"path": outside.join("t.png")}}),
    ];
    for c in cases {
        let r = call(&mut h, "execute", c.clone());
        assert_eq!(r["error"]["code"], codes::COMMAND_FAILED, "{c}: {r}");
        assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{c}: {r}");
        assert!(r["error"]["message"].as_str().unwrap().contains("--api-root"), "{r}");
    }
    assert!(!outside.join("a.skr").exists() && !outside.join("a.stl").exists());
}

#[test]
fn links_out_of_the_roots_are_followed_and_refused() {
    let (root, outside) = folders("links");
    let mut h = host(&root);
    edge(&mut h);
    std::os::unix::fs::symlink(&outside, root.join("door")).unwrap();
    std::fs::write(outside.join("secret.obj"), "v 0 0 0\n").unwrap();
    let r = call(&mut h, "execute", json!({"type": "save", "path": "door/a.skr"}));
    assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
    let r = call(&mut h, "execute", json!({"type": "import", "path": "door/secret.obj"}));
    assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
    // A link as the file to write is refused even when it points inside.
    std::fs::write(root.join("real.skr"), "").unwrap();
    std::os::unix::fs::symlink(root.join("real.skr"), root.join("link.skr")).unwrap();
    let r = call(&mut h, "execute", json!({"type": "save", "path": "link.skr"}));
    assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
    // A missing file is a file error naming the path given.
    let r = call(&mut h, "execute", json!({"type": "open", "path": "nope.skr"}));
    assert_eq!(r["error"]["data"]["kind"], "file", "{r}");
    assert!(r["error"]["message"].as_str().unwrap().contains("nope.skr"));
}

#[test]
fn a_batch_with_a_path_outside_runs_nothing() {
    let (root, outside) = folders("batch");
    let mut h = host(&root);
    let r = call(
        &mut h,
        "batch",
        json!({"commands": [
            {"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]},
            {"type": "save", "path": outside.join("a.skr")}
        ]}),
    );
    assert_eq!(r["error"]["code"], codes::BATCH_FAILED, "{r}");
    assert_eq!(r["error"]["data"]["index"], 1);
    assert_eq!(r["error"]["data"]["completed"], json!([]));
    assert_eq!(call(&mut h, "query", json!({"type": "stats"}))["result"]["edges"], 0);
    // Save without a path in a batch is refused under roots.
    let r = call(&mut h, "batch", json!({"commands": [{"type": "save"}]}));
    assert_eq!(r["error"]["data"]["error"]["kind"], "invalid_input", "{r}");
}

#[test]
fn material_libraries_outside_the_roots_are_not_read() {
    let (root, outside) = folders("mtllib");
    std::fs::write(outside.join("paint.mtl"), "newmtl red\nKd 1 0 0\n").unwrap();
    let obj = "mtllib ../outside/paint.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    std::fs::write(root.join("tri.obj"), obj).unwrap();
    let mut limited = host(&root);
    call(&mut limited, "execute", json!({"type": "import", "path": "tri.obj"}));
    let m = call(&mut limited, "query", json!({"type": "materials"}));
    assert_eq!(m["result"]["materials"], json!([]), "{m}");
    // Without roots the library is read, as before.
    let mut free = HeadlessHost::new();
    call(&mut free, "execute", json!({"type": "import", "path": root.join("tri.obj")}));
    let m = call(&mut free, "query", json!({"type": "materials"}));
    assert_eq!(m["result"]["materials"].as_array().unwrap().len(), 1, "{m}");
}

#[test]
fn without_roots_the_file_query_says_null() {
    let mut h = HeadlessHost::new();
    let file = call(&mut h, "query", json!({"type": "file"}));
    assert_eq!(file["result"]["api_roots"], Value::Null);
    assert!(ApiRoots::new(&[PathBuf::from("/no/such/folder")]).is_err());
}
```

(Check the `materials` query's result field name in `session/src/query.rs` and adjust `["materials"]` if it differs.)

`crates/app/src/cli.rs` tests: update the `Mode` literals with `api_roots: vec![]`, and add:

```rust
    #[test]
    fn api_roots_repeat() {
        assert_eq!(
            parse(args("--headless --stdio --api-root /a --api-root b f.skr")).unwrap(),
            Mode::Headless {
                transport: Transport::Stdio,
                file: Some("f.skr".into()),
                api_roots: vec!["/a".into(), "b".into()],
            }
        );
        assert!(parse(args("--api-root")).unwrap_err().contains("needs a folder"));
    }
```

`crates/app/tests/headless_agent.rs`:

```rust
#[test]
fn an_agent_is_kept_inside_its_api_root() {
    let base = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("agent-root-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("root");
    std::fs::create_dir_all(&root).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_app"))
        .args(["--headless", "--stdio", "--api-root"])
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start app");
    let mut stdin = child.stdin.take().unwrap();
    let requests = [
        request(1, "execute", json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]})),
        request(2, "execute", json!({"type": "save", "path": "inside.skr"})),
        request(3, "execute", json!({"type": "save", "path": base.join("outside.skr")})),
        request(4, "query", json!({"type": "file"})),
    ];
    for r in &requests {
        writeln!(stdin, "{r}").unwrap();
    }
    drop(stdin);
    let status = exits(&mut child);
    assert!(status.success(), "exit status {status}");
    let replies: HashMap<u64, Value> = BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map(|l| serde_json::from_str::<Value>(&l.unwrap()).unwrap())
        .filter_map(|m| m["id"].as_u64().map(|id| (id, m)))
        .collect();
    assert!(replies[&2]["result"].is_object(), "{:?}", replies[&2]);
    assert_eq!(replies[&3]["error"]["data"]["kind"], "invalid_input");
    assert!(root.join("inside.skr").exists() && !base.join("outside.skr").exists());
    assert_eq!(replies[&4]["result"]["api_roots"], json!([root.canonicalize().unwrap()]));
}

#[test]
fn a_missing_api_root_stops_the_app() {
    let status = Command::new(env!("CARGO_BIN_EXE_app"))
        .args(["--headless", "--stdio", "--api-root", "/no/such/folder/at/all"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(2));
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p api --test roots`
Expected: compile error (`ApiRoots`, `with_roots` do not exist).

- [ ] **Step 3: Implement**

`crates/session/src/command.rs`, `Import` gains (last field):

```rust
        /// Set by the API under `--api-root`: the OBJ's material libraries are read only when
        /// they lie, after symlinks, within these folders. Never read from or written to JSON.
        #[serde(skip)]
        library_roots: Option<Vec<PathBuf>>,
```

(`use std::path::PathBuf;`). `document.rs`'s `Command::Import { .. }` arm passes it into `ImportOptions { ..., library_roots: library_roots.clone() }`; `document/import.rs`: `ImportOptions` gains `pub library_roots: Option<Vec<PathBuf>>`, `import_materials(file, &read, o.library_roots.as_deref())`, and in its library loop:

```rust
                let library = dir.join(library);
                let allowed = roots.is_none_or(|roots| {
                    library
                        .canonicalize()
                        .is_ok_and(|real| roots.iter().any(|r| real.starts_with(r)))
                });
                if !allowed {
                    continue;
                }
                if let Ok(bytes) = read_limited(&library, MAX_MTL_BYTES) {
```

`app/src/editor.rs` and `render/tests/golden.rs`: `library_roots: None` in their `Command::Import` literals.

`crates/api/src/roots.rs`:

```rust
//! `--api-root`: the folders the API may read and write files in.

use std::path::{Component, Path, PathBuf};

use session::{Command, CommandError, ErrorKind, TextureSource};

/// Folders every path the API reads or writes must lie within, resolved after symlinks.
/// Empty: no limit (the default).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApiRoots {
    roots: Vec<PathBuf>,
}

impl ApiRoots {
    /// `dirs` resolved after symlinks. Fails, naming it, for one that is not an existing folder.
    pub fn new(dirs: &[PathBuf]) -> Result<ApiRoots, String> {
        let mut roots: Vec<PathBuf> = Vec::new();
        for dir in dirs {
            let real = dir
                .canonicalize()
                .map_err(|e| format!("--api-root {}: {e}", dir.display()))?;
            if !real.is_dir() {
                return Err(format!("--api-root {}: not a folder", dir.display()));
            }
            if !roots.contains(&real) {
                roots.push(real);
            }
        }
        Ok(ApiRoots { roots })
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// The folders, resolved.
    pub fn dirs(&self) -> &[PathBuf] {
        &self.roots
    }

    fn inside(&self, real: &Path) -> bool {
        self.roots.iter().any(|r| real.starts_with(r))
    }

    /// `path` as given, made absolute against the first root.
    fn absolute(&self, path: &str) -> PathBuf {
        match self.roots.first() {
            Some(first) if Path::new(path).is_relative() => first.join(path),
            _ => PathBuf::from(path),
        }
    }

    /// The file to read at `path`, resolved after symlinks, when it lies within a root.
    pub fn read(&self, path: &str) -> Result<PathBuf, CommandError> {
        let real = self
            .absolute(path)
            .canonicalize()
            .map_err(|e| file_error(path, &e))?;
        if !self.inside(&real) {
            return Err(outside(path));
        }
        Ok(real)
    }

    /// The file to write at `path`: its folder, resolved after symlinks, must lie within a
    /// root, and the file must not be a symbolic link.
    pub fn write(&self, path: &str) -> Result<PathBuf, CommandError> {
        let absolute = self.absolute(path);
        let Some(Component::Normal(name)) = absolute.components().next_back() else {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("{path} names no file"),
            ));
        };
        let folder = absolute.parent().unwrap_or(Path::new("/"));
        let real = folder
            .canonicalize()
            .map_err(|e| file_error(path, &e))?
            .join(name);
        if !self.inside(&real) {
            return Err(outside(path));
        }
        if std::fs::symlink_metadata(&real).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("{path} is a symbolic link: under --api-root the API does not write to links"),
            ));
        }
        Ok(real)
    }

    /// Points every path in `command` at the file it resolves to, or refuses the command.
    /// `open_file` is the model's file, which `save` without a path writes; inside a batch
    /// such a `save` is refused, since an earlier command may open another file.
    pub fn apply(
        &self,
        command: &mut Command,
        open_file: Option<&Path>,
        in_batch: bool,
    ) -> Result<(), CommandError> {
        if self.is_empty() {
            return Ok(());
        }
        match command {
            Command::Open { path, .. } => *path = text(self.read(path)?)?,
            Command::Import {
                path,
                library_roots,
                ..
            } => {
                *path = text(self.read(path)?)?;
                *library_roots = Some(self.roots.clone());
            }
            Command::Export { path, .. } => *path = text(self.write(path)?)?,
            Command::Save { path, .. } => {
                let given = match (path.as_ref(), open_file) {
                    (Some(p), _) => p.clone(),
                    (None, _) if in_batch => {
                        return Err(CommandError::new(
                            ErrorKind::InvalidInput,
                            "give save a path: under --api-root a batch cannot save to the open file",
                        ));
                    }
                    (None, Some(open)) => open.display().to_string(),
                    // Nothing to check: the session asks for a path.
                    (None, None) => return Ok(()),
                };
                *path = Some(text(self.write(&given)?)?);
            }
            Command::AddMaterial {
                texture: Some(TextureSource { path: Some(p), .. }),
                ..
            }
            | Command::ChangeMaterial {
                texture: Some(Some(TextureSource { path: Some(p), .. })),
                ..
            } => *p = text(self.read(p)?)?,
            _ => {}
        }
        Ok(())
    }
}

fn outside(path: &str) -> CommandError {
    CommandError::new(
        ErrorKind::InvalidInput,
        format!("{path} is outside the folders the API may use (--api-root)"),
    )
}

fn file_error(path: &str, e: &std::io::Error) -> CommandError {
    CommandError::new(ErrorKind::File, format!("{path}: {e}"))
}

/// A resolved path as command text; a path that is not UTF-8 cannot be passed on.
fn text(path: PathBuf) -> Result<String, CommandError> {
    path.into_os_string().into_string().map_err(|p| {
        CommandError::new(
            ErrorKind::InvalidInput,
            format!("{} is not a UTF-8 path", PathBuf::from(p).display()),
        )
    })
}
```

(If the `Command` enum's struct-variant fields do not destructure as written, for example because `ChangeMaterial.texture` is named differently, adjust to the real field names. Add a unit test in `roots.rs` that builds every `Command` variant that holds a path (`Open`, `Save`, `Export`, `Import`, `AddMaterial`, `ChangeMaterial`) and checks `apply` changes each: this guards against a variant being missed.)

`crates/api/src/dispatch.rs`:
- `Host` gains:

```rust
    /// The folders file paths must lie within (`--api-root`), or None for no limit.
    fn roots(&self) -> Option<&ApiRoots> {
        None
    }
```

- a helper:

```rust
/// Applies the host's `--api-root` folders to `command`'s paths.
fn within_roots(host: &dyn Host, command: &mut Command, in_batch: bool) -> Result<(), CommandError> {
    match host.roots() {
        Some(roots) => roots.apply(command, host.document().path(), in_batch),
        None => Ok(()),
    }
}
```

- `"execute"`: `let mut command: Command = params_as(params)?; within_roots(host, &mut command, false).map_err(command_failed)?;` then as before.
- `"batch"`: after parsing, before running:

```rust
            let mut commands = p.commands;
            for (index, command) in commands.iter_mut().enumerate() {
                if let Err(error) = within_roots(host, command, true) {
                    let e = BatchError { index, error, completed: Vec::new(), rolled_back: false };
                    return Err(RpcError::new(codes::BATCH_FAILED, format!("batch stopped at command {index}"))
                        .with_data(json!(e)));
                }
            }
```

(Check `BatchError`'s fields are public; if not, add a `BatchError::refused(index, error)` constructor in session.)
- `"query"`: for a `file` query add the roots:

```rust
        "query" => {
            let file = type_of(&params) == Some("file");
            let query: Query = params_as(params)?;
            let mut result = json!(host.document().query(&query).map_err(command_failed)?);
            if file {
                result["api_roots"] = json!(host.roots().map(ApiRoots::dirs));
            }
            Ok(result)
        }
```

- `describe()`: add after `"units"`: `"paths": "file paths are absolute or relative to the app's working directory; with --api-root, relative to the first root and kept within the roots after symlinks (query file lists them as api_roots); save and export refuse to replace a file of another format unless overwrite: true"`.

`crates/api/src/headless.rs`: `HeadlessHost` gains `roots: ApiRoots` (default empty), `pub fn with_roots(roots: ApiRoots) -> Self { Self { roots, ..Self::new() } }`, and `fn roots(&self) -> Option<&ApiRoots> { (!self.roots.is_empty()).then_some(&self.roots) }`.

`crates/app/src/cli.rs`: `parse` accepts `--api-root DIR` (repeatable; a missing value or one starting with `--` is `"--api-root needs a folder"`), collects `api_roots: Vec<PathBuf>` into both `Mode::Window` and `Mode::Headless`. `USAGE` gains `[--api-root DIR]...` on each line and: `--api-root DIR (repeatable) keeps every file the API reads or writes inside DIR.`

`crates/app/src/main.rs`: for both modes, before starting:

```rust
fn roots(dirs: &[PathBuf]) -> api::ApiRoots {
    api::ApiRoots::new(dirs).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    })
}
```

and pass `roots(&api_roots)` to `headless::run(transport, file, roots)` and `app::App::new(&event_loop, socket, file, roots)`. When non-empty, print on stderr `agent API files limited to: DIR, DIR`.

`crates/app/src/host.rs`: `AppHost` gains

```rust
    /// The folders the API's file paths must lie within (empty: no limit).
    pub roots: &'a ApiRoots,
```

and `fn roots(&self) -> Option<&ApiRoots> { (!self.roots.is_empty()).then_some(self.roots) }`. `headless.rs` `serve` and `app.rs` (`State` or `App` holds an `ApiRoots`; `State::host` builds `AppHost` with it; since `host()` is a `State` method, keep the `ApiRoots` in `State`, moved there from `App` in `resumed`, or keep it in `App` and pass it to `process_api`; choose what compiles simply) build `AppHost` with `roots`. Fix the test helpers in `host.rs` and `host/tool_input_tests.rs` with `roots: &ApiRoots::default()`.

`docs/agent-api.md`: Starting table: add row `cargo run -p app -- --api-root DIR [--api-root DIR2]` "Any mode: the API reads and writes files only inside these folders (see Access and safety)". Change "Paths are relative to the app's working directory." in Files to "Paths are relative to the app's working directory, or with `--api-root` to the first root (see Access and safety)." Add the section:

```markdown
### Access and safety

Anyone who can connect to the socket (programs running as the same user) can drive the app. To
keep an agent's file access to some folders, start the app with `--api-root DIR` (repeatable):
then every path a request names (`open`, `save`, `export` and the `.mtl` and images an OBJ
writes beside it, `import` and the material libraries an OBJ names, `texture.path`) must lie
within one of them, after symlinks are resolved. A relative path is taken from the first root.
A file to read must exist; a file to write must be in an existing folder and must not be a
symbolic link. A path outside the roots is refused with `invalid_input`; one that cannot be
resolved gives `file`. `save` without a path checks the open model's file; inside a `batch` it
needs a path. A batch with a path outside the roots runs nothing (`-32001`, its `index` names
the command). Commands run on the resolved path, so `query file` shows it; `query file` also
lists the roots as `api_roots` (null without `--api-root`). The window's own dialogs and the
FILE argument are not limited. A root that is not an existing folder stops the app (exit code
2).
```

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p api && $HOME/.cargo/bin/cargo test -p app && $HOME/.cargo/bin/cargo test -p session`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status --short && git diff --stat
git add crates/api/src/roots.rs crates/api/src/lib.rs crates/api/src/dispatch.rs crates/api/src/headless.rs crates/api/tests/roots.rs crates/session/src/command.rs crates/session/src/document.rs crates/session/src/document/import.rs crates/app/src/cli.rs crates/app/src/main.rs crates/app/src/headless.rs crates/app/src/app.rs crates/app/src/host.rs crates/app/src/host/tool_input_tests.rs crates/app/src/editor.rs crates/app/tests/headless_agent.rs crates/render/tests/golden.rs docs/agent-api.md
git commit -m "--api-root keeps every path the API reads or writes inside the given folders"
```

---

### Task 4: Private socket bind and the activity log

**Files:**
- Modify: `crates/api/src/transport.rs` (`listen_unix`)
- Modify: `crates/api/src/hub.rs` (activity log), `crates/api/src/lib.rs` (`pub use hub::{Activity, ACTIVITY_LEN, Hub, Inbox};`)
- Test: `crates/api/tests/socket.rs`, `crates/api/tests/hub.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces:

```rust
/// How many entries the activity log keeps.
pub const ACTIVITY_LEN: usize = 100;

/// One line of the activity log: a connection opening or closing, or a request handled.
#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    /// When it happened (the last time, for a repeated request).
    pub at: std::time::Instant,
    pub connection: u64,
    /// `connected`, `disconnected`, or the request: method, command or query type, and the
    /// path of a file command (`execute save /a/house.skr`), at most 160 characters.
    pub request: String,
    /// The error message when the reply was an error.
    pub error: Option<String>,
    /// How many times in a row it came with the same result (1 at first).
    pub repeats: u32,
}

impl Hub {
    /// The log, oldest first.
    pub fn activity(&self) -> &std::collections::VecDeque<Activity>;
    /// Goes up whenever the log changes, so a window knows to draw it again.
    pub fn activity_revision(&self) -> u64;
}
```

- [ ] **Step 1: Write the failing tests**

`crates/api/tests/socket.rs` (use its `test_dir` helper):

```rust
#[test]
fn the_socket_is_private_from_the_start_and_leaves_no_folder() {
    let dir = test_dir("private-bind");
    let path = dir.join("p.sock");
    let hub = api::Hub::new(|| {});
    api::listen_unix(&path, hub.inbox()).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names, vec![std::ffi::OsString::from("p.sock")], "no bind folder left");
    std::os::unix::net::UnixStream::connect(&path).expect("connects at the final path");
    std::fs::remove_file(&path).unwrap();
}
```

(Bind under a umask of 0 cannot be set from a test without `libc`; the test checks the outcome and the folder clean-up. Import `PermissionsExt` if the file does not already.)

`crates/api/tests/hub.rs` (use its `next`, `request` helpers and `HeadlessHost`):

```rust
#[test]
fn the_activity_log_records_requests_folds_repeats_and_keeps_the_last_hundred() {
    let mut hub = api::Hub::new(|| {});
    let inbox = hub.inbox();
    let mut host = api::HeadlessHost::new();
    let (conn, rx) = inbox.open();
    inbox.send_line(conn, request(1, "execute", json!({"type": "save", "path": "/no/such/dir/a.skr"})));
    inbox.send_line(conn, request(2, "query", json!({"type": "camera"})));
    inbox.send_line(conn, request(3, "query", json!({"type": "camera"})));
    inbox.send_line(conn, "not json".into());
    hub.process(&mut host);
    for _ in 0..4 {
        next(&rx);
    }
    let log: Vec<_> = hub.activity().iter().collect();
    assert_eq!(log[0].request, "connected");
    assert_eq!(log[1].request, "execute save /no/such/dir/a.skr");
    assert!(log[1].error.as_deref().unwrap().contains("a.skr"));
    assert_eq!(log[2].request, "query camera");
    assert_eq!((log[2].repeats, log[2].error.clone()), (2, None));
    assert_eq!(log[3].request, "(not JSON)");
    assert_eq!(log.len(), 4);
    let revision = hub.activity_revision();
    for i in 0..150 {
        inbox.send_line(conn, request(10 + i, "execute", json!({"type": "zoom", "factor": 1.0 + i as f64 / 1000.0})));
    }
    inbox.close(conn);
    hub.process(&mut host);
    assert_eq!(hub.activity().len(), api::ACTIVITY_LEN);
    assert_eq!(hub.activity().back().unwrap().request, "disconnected");
    assert!(hub.activity_revision() > revision);
}
```

(The 150 zoom requests differ in nothing the summary shows, so they fold into one entry; to fill the log make each distinct: use `{"type": "select", "ids": []}` and `{"type": "zoom_extents"}` alternately, so no two in a row are equal, and assert the length is `ACTIVITY_LEN`. Write the loop that way.)

- [ ] **Step 2: Run them to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p api --test socket --test hub`
Expected: the hub test fails to compile (`activity` missing); the socket test may pass already on the mode but fails nothing else: that is fine, it guards the new bind.

- [ ] **Step 3: Implement**

`crates/api/src/transport.rs`, in `listen_unix` replace the bind and chmod:

```rust
    let listener = bind_private(path)?;
```

with

```rust
/// Binds a socket at `path` that is never reachable at a mode the umask chose: it is bound in
/// a fresh folder only this user can enter, made 0600 there, then linked to `path` (failing,
/// not replacing, when something appeared there meanwhile).
fn bind_private(path: &Path) -> io::Result<UnixListener> {
    use std::os::unix::fs::DirBuilderExt;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let mut n = 0u32;
    let dir = loop {
        let dir = parent.join(format!(".sa{}-{n}", std::process::id()));
        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => break dir,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && n < 100 => n += 1,
            Err(e) => return Err(e),
        }
    };
    let temp = dir.join("s");
    let result = (|| {
        let listener = UnixListener::bind(&temp)?;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
        std::fs::hard_link(&temp, path)?;
        Ok(listener)
    })();
    let _ = std::fs::remove_file(&temp);
    let _ = std::fs::remove_dir(&dir);
    result
}
```

(The bind folder's path is shorter than or close to the target's, so it fits the socket path limit whenever the target does; if `hard_link` turns out not to work for sockets on this system, fall back to `std::fs::rename` after checking again that `path` is free, and say so in a comment.)

`crates/api/src/hub.rs`: add `activity: VecDeque<Activity>` and `activity_revision: u64` to `Hub` (initialised empty and 0), the `Activity` type and `ACTIVITY_LEN` from Interfaces, and:

```rust
    /// Adds a line to the activity log, or counts it again when it repeats the last one.
    fn log(&mut self, connection: u64, request: String, error: Option<String>) {
        self.activity_revision += 1;
        let now = Instant::now();
        if let Some(last) = self.activity.back_mut()
            && last.connection == connection
            && last.request == request
            && last.error == error
        {
            last.repeats += 1;
            last.at = now;
            return;
        }
        if self.activity.len() == ACTIVITY_LEN {
            self.activity.pop_front();
        }
        self.activity.push_back(Activity { at: now, connection, request, error, repeats: 1 });
    }
```

and a summary function:

```rust
/// How the activity log names a request line: method, command or query type, and the path
/// of a file command; at most 160 characters.
fn summary(line: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return "(not JSON)".into();
    };
    let method = v["method"].as_str().unwrap_or("?");
    let params = &v["params"];
    let mut s = method.to_string();
    match method {
        "batch" => {
            let n = params["commands"].as_array().map_or(0, Vec::len);
            s += &format!(" ({n} commands)");
        }
        "tool_input" => {
            let n = params["events"].as_array().map_or(0, Vec::len);
            s += &format!(" ({n} events)");
        }
        _ => {
            if let Some(t) = params["type"].as_str() {
                s += &format!(" {t}");
            }
            if let Some(p) = params["path"].as_str() {
                s += &format!(" {p}");
            }
        }
    }
    if s.chars().count() > 160 {
        s = s.chars().take(159).collect::<String>() + "\u{2026}";
    }
    s
}
```

Call `self.log(conn, "connected".into(), None)` in `Event::Opened`, `"disconnected"` in `Event::Closed` (only for a connection that was open), and in `Event::Line` after the reply is made: `let error = reply.as_deref().and_then(|r| serde_json::from_str::<Value>(r).ok()).and_then(|r| r["error"]["message"].as_str().map(str::to_string)); self.log(conn, summary(&line), error);`. Use `use std::collections::VecDeque; use std::time::Instant;`.

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p api && $HOME/.cargo/bin/cargo test -p app --test headless_agent`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status --short && git diff --stat
git add crates/api/src/transport.rs crates/api/src/hub.rs crates/api/src/lib.rs crates/api/tests/socket.rs crates/api/tests/hub.rs
git commit -m "Socket bound private from the start; hub keeps an activity log"
```

---

### Task 5: The window shows agents, and keeps work an agent discards

**Files:**
- Modify: `crates/session/src/document/files.rs` (`write_copy`)
- Modify: `crates/app/src/host.rs` (`recovery` field, keeping discarded work), `crates/app/src/host/tool_input_tests.rs` (field), `crates/app/src/headless.rs` (`recovery: None`)
- Modify: `crates/app/src/editor.rs` (`recovered`, `note_recovery`)
- Modify: `crates/app/src/main.rs` / `crates/app/src/app.rs` (recovery folder, indicator and log wiring, Agent Activity open state)
- Modify: `crates/app/src/ui.rs` (`Agents`, status indicator, View menu, Agent Activity window, `UiAction::ShowAgentActivity`)
- Modify: `docs/agent-api.md` (Files: `discard`; Access and safety; Window actions table)
- Test: `crates/app/src/host.rs` tests, `crates/app/src/ui.rs` tests, `crates/app/src/app.rs` tests

**Interfaces:**
- Consumes: Task 3's `AppHost { roots }`, Task 4's `Hub::activity`, `activity_revision`, `open_connections`, `Activity`.
- Produces:
  - `Document::write_copy(&self, path: &Path) -> Result<(), CommandError>`: writes the model, view and lighting as a `.skr` at `path` without making it the document's file or marking it saved.
  - `AppHost { ..., recovery: Option<&'a Path> }`: the folder discarded work is kept in (window: `~/.sketchup-alt/recovery`; headless and tests that do not test it: None).
  - `host::recovery_path(dir: &Path, open_file: Option<&Path>, now: SystemTime) -> PathBuf` (a free name).
  - `Editor::note_recovery(&mut self, path: PathBuf)`, `Editor::recovered(&self) -> &[PathBuf]`.
  - `ui::Agents<'a> { connected: usize, log: Option<&'a [String]>, recovered: &'a [PathBuf] }`; `View.agents: Agents<'a>`; `UiAction::ShowAgentActivity(bool)`.
  - `app::activity_line(a: &Activity, now: Instant) -> String`.

- [ ] **Step 1: Write the failing tests**

`crates/app/src/host.rs` tests (adapt its `call` helper to take the recovery folder; a scratch folder per test):

```rust
    fn recovery_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("sketchup-alt-recovery-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn call_kept(e: &mut Editor, dir: &Path, method: &str, params: Value) -> Value {
        let (mut offscreen, mut quit) = (Offscreen::headless(), false);
        let roots = ApiRoots::default();
        let mut host = AppHost {
            editor: e,
            offscreen: &mut offscreen,
            quit: &mut quit,
            roots: &roots,
            recovery: Some(dir),
        };
        let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
        serde_json::from_str(&api::handle(&mut host, &line).unwrap()).unwrap()
    }

    fn files_in(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir).map_or_else(|_| Vec::new(), |r| r.map(|e| e.unwrap().path()).collect())
    }

    #[test]
    fn work_an_agent_discards_is_kept_in_a_recovery_file() {
        let dir = recovery_dir("new");
        let mut e = editor();
        // Nothing unsaved: nothing kept.
        call_kept(&mut e, &dir, "execute", json!({"type": "new", "discard": true}));
        assert!(files_in(&dir).is_empty());
        call_kept(&mut e, &dir, "execute", json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}));
        let r = call_kept(&mut e, &dir, "execute", json!({"type": "new", "discard": true}));
        assert!(r["result"].is_object(), "{r}");
        let kept = files_in(&dir);
        assert_eq!(kept.len(), 1);
        assert!(kept[0].file_name().unwrap().to_string_lossy().starts_with("untitled-"));
        assert_eq!(e.recovered(), &kept[..]);
        assert!(e.status_parts().message.unwrap().contains(&kept[0].display().to_string()));
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        // The file holds the dropped edge.
        let mut back = editor();
        back.execute(Command::Open { path: kept[0].display().to_string(), discard: false }).unwrap();
        assert_eq!(back.doc().model().edges().count(), 1);
    }

    #[test]
    fn a_failed_open_keeps_nothing_and_drops_nothing() {
        let dir = recovery_dir("open");
        let mut e = editor();
        call_kept(&mut e, &dir, "execute", json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}));
        let r = call_kept(&mut e, &dir, "execute", json!({"type": "open", "path": "/no/such.skr", "discard": true}));
        assert_eq!(r["error"]["data"]["kind"], "file");
        assert!(files_in(&dir).is_empty());
        assert!(e.doc().is_modified());
    }

    #[test]
    fn a_batch_that_discards_keeps_the_work_at_that_command() {
        let dir = recovery_dir("batch");
        let mut e = editor();
        let r = call_kept(&mut e, &dir, "batch", json!({"commands": [
            {"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]},
            {"type": "new", "discard": true},
            {"type": "add_edge", "start": [0, 0, 0], "end": [0, 1, 0]}
        ]}));
        assert_eq!(r["result"]["outcomes"].as_array().unwrap().len(), 3, "{r}");
        assert_eq!(files_in(&dir).len(), 1);
        let r = call_kept(&mut e, &dir, "batch", json!({"commands": [
            {"type": "new", "discard": true},
            {"type": "open", "path": "/no/such.skr", "discard": true}
        ]}));
        assert_eq!(r["error"]["data"]["index"], 1, "{r}");
        assert_eq!(r["error"]["data"]["completed"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn quitting_with_discard_keeps_the_work_and_a_folder_open_to_others_is_refused() {
        let dir = recovery_dir("quit");
        let mut e = editor();
        call_kept(&mut e, &dir, "execute", json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let r = call_kept(&mut e, &dir, "quit", json!({"discard": true}));
        assert_eq!(r["error"]["data"]["kind"], "file", "{r}");
        assert!(r["error"]["message"].as_str().unwrap().contains("nothing was discarded"));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let r = call_kept(&mut e, &dir, "quit", json!({"discard": true}));
        assert!(r["result"].is_object(), "{r}");
        assert_eq!(files_in(&dir).len(), 1);
    }

    #[test]
    fn recovery_names_follow_the_open_file_and_never_collide() {
        let dir = recovery_dir("names");
        std::fs::create_dir_all(&dir).unwrap();
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let a = recovery_path(&dir, Some(Path::new("/x/My house?.skr")), now);
        assert_eq!(a, dir.join("My house_-1700000000.skr"));
        std::fs::write(&a, "").unwrap();
        assert_eq!(recovery_path(&dir, Some(Path::new("/x/My house?.skr")), now), dir.join("My house_-1700000000-2.skr"));
        assert_eq!(recovery_path(&dir, None, now), dir.join("untitled-1700000000.skr"));
    }
```

(Headless keeps nothing: the existing `call` helper passes `recovery: None`; add one assertion in `quit_is_refused_with_unsaved_changes_unless_discarded` that no recovery is noted: `assert!(e.recovered().is_empty())`.)

`crates/app/src/ui.rs` tests: add `agents: Agents { connected: 0, log: None, recovered: &[] }` to every `View` literal (a helper `fn no_agents() -> Agents<'static>` keeps that short), and:

```rust
    #[test]
    fn the_view_menu_and_the_indicator_open_agent_activity() {
        // View menu: find it right of Camera as menu_actions finds the others (xs over the
        // menu bar), and check its item reports ShowAgentActivity(true).
        // Status bar: with connected 2, the text "Agents: 2" is drawn and clicking it reports
        // ShowAgentActivity(true); with connected 0 nothing is drawn there.
        // Window: with log Some(&["5 s ago  #1  execute save /a.skr  ok".into()]) and one
        // recovered path, both texts appear, and its close button reports
        // ShowAgentActivity(false).
    }
```

Write this test with the same egui harness the file already uses (`run`, `click`, `menu_actions`, `screen`): to find a label's position, iterate the frame's shapes or use `ctx.memory`/`Response` the way the soften window test locates its buttons; the three comment lines are the assertions to make.

`crates/app/src/app.rs` tests:

```rust
    #[test]
    fn activity_lines_read_newest_first_with_age_and_result() {
        let now = Instant::now();
        let a = api::Activity {
            at: now - Duration::from_secs(12),
            connection: 3,
            request: "execute save /a/house.skr".into(),
            error: None,
            repeats: 1,
        };
        assert_eq!(activity_line(&a, now), "12 s ago  #3  execute save /a/house.skr  ok");
        let b = api::Activity { error: Some("no".into()), repeats: 4, at: now, ..a };
        assert_eq!(activity_line(&b, now), "0 s ago  #3  execute save /a/house.skr  error: no  (x4)");
        let c = api::Activity { at: now - Duration::from_secs(3 * 60 + 5), ..b };
        assert!(activity_line(&c, now).starts_with("3 min ago"));
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p app -- recovery activity agent_activity discards`
Expected: compile errors (`recovery`, `recovered`, `activity_line`, `Agents` do not exist).

- [ ] **Step 3: Implement**

`crates/session/src/document/files.rs`:

```rust
    /// Writes the model, view and lighting as a `.skr` file at `path`, leaving the document's
    /// own file and its unsaved state as they are (a recovery copy).
    pub fn write_copy(&self, path: &Path) -> Result<(), CommandError> {
        file::write(path, &self.scene, &self.camera, self.lighting)
            .map_err(|e| file_error(&path.display().to_string(), e))
    }
```

`crates/app/src/host.rs`:

```rust
    /// The folder unsaved changes are kept in when an API `new`, `open` or `quit` discards
    /// them (the window's; None keeps nothing, as headless, where the agent's work is all
    /// there is).
    pub recovery: Option<&'a Path>,
```

```rust
/// Whether `command` drops unsaved changes when it runs.
fn discards(command: &Command) -> bool {
    matches!(
        command,
        Command::New { discard: true } | Command::Open { discard: true, .. }
    )
}

/// A free file name in `dir` for a recovery copy of the model at `open_file` (or an unsaved
/// one), stamped with `now` in seconds.
pub fn recovery_path(dir: &Path, open_file: Option<&Path>, now: SystemTime) -> PathBuf {
    let stem: String = open_file
        .and_then(Path::file_stem)
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "untitled".into())
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ') { c } else { '_' })
        .collect();
    let secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut path = dir.join(format!("{stem}-{secs}.skr"));
    let mut n = 2;
    while std::fs::symlink_metadata(&path).is_ok() {
        path = dir.join(format!("{stem}-{secs}-{n}.skr"));
        n += 1;
    }
    path
}

impl AppHost<'_> {
    /// Before a command that discards unsaved changes, writes the model to a recovery file
    /// (in the window). Fails, so the command does not run, when it cannot be written.
    fn keep_discarded(&mut self) -> Result<Option<PathBuf>, CommandError> {
        let Some(dir) = self.recovery else {
            return Ok(None);
        };
        if !self.editor.doc().is_modified() {
            return Ok(None);
        }
        let failed = |e: String| {
            CommandError::new(
                ErrorKind::File,
                format!("could not keep the unsaved changes in a recovery file ({e}): nothing was discarded"),
            )
        };
        if let Some(parent) = cli::private_dir_for(dir) {
            cli::ensure_private_dir(parent).map_err(|e| failed(e.to_string()))?;
        }
        cli::ensure_private_dir(dir).map_err(|e| failed(e.to_string()))?;
        let path = recovery_path(dir, self.editor.doc().path(), SystemTime::now());
        self.editor.doc().write_copy(&path).map_err(|e| failed(e.message))?;
        Ok(Some(path))
    }

    /// Runs `command`, first keeping the work it would discard.
    fn execute_keeping(&mut self, command: Command) -> Result<Outcome, CommandError> {
        let kept = if discards(&command) { self.keep_discarded()? } else { None };
        let result = self.editor.execute(command);
        if let Some(path) = kept {
            match &result {
                Ok(_) => self.editor.note_recovery(path),
                // Nothing was dropped.
                Err(_) => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        result
    }
}
```

(`cli::private_dir_for(dir)` returns `dir`'s parent only when it is the `.sketchup-alt` folder: for `~/.sketchup-alt/recovery` that is `~/.sketchup-alt`, which must then be private; a test folder's parent is left alone. Import `std::time::{SystemTime, UNIX_EPOCH}`, `std::path::{Path, PathBuf}`, `session::ErrorKind`, `crate::cli`.)

In `impl Host for AppHost`: `execute` calls `self.execute_keeping(command)`; `execute_batch`:

```rust
        // Run one by one when a command would discard work, so it is kept at its turn.
        if !atomic && self.recovery.is_some() && commands.iter().any(discards) {
            let mut completed = Vec::new();
            for (index, command) in commands.into_iter().enumerate() {
                match self.execute_keeping(command) {
                    Ok(outcome) => completed.push(outcome),
                    Err(error) => {
                        return Err(BatchError { index, error, completed, rolled_back: false });
                    }
                }
            }
            return Ok(completed);
        }
        self.editor.execute_batch(commands, atomic)
```

`quit`:

```rust
    fn quit(&mut self, discard: bool) -> Result<(), CommandError> {
        self.editor.doc().check_unsaved(discard)?;
        if let Some(path) = self.keep_discarded()? {
            self.editor.note_recovery(path);
        }
        *self.quit = true;
        Ok(())
    }
```

`crates/app/src/editor.rs`: field `recovered: Vec<PathBuf>` (empty in `new`), and

```rust
    /// Notes a recovery file an agent's discard was kept in: the status says where, and
    /// stderr too (the window may be quitting).
    pub fn note_recovery(&mut self, path: PathBuf) {
        let line = format!(
            "An agent discarded unsaved changes; they were kept in {}",
            path.display()
        );
        eprintln!("{line}");
        self.message = Some(line);
        self.recovered.push(path);
    }

    /// The recovery files written this session, oldest first.
    pub fn recovered(&self) -> &[PathBuf] {
        &self.recovered
    }
```

`crates/app/src/main.rs`: the window's recovery folder: `std::env::var_os("HOME").map(|h| PathBuf::from(h).join(cli::FALLBACK_DIR).join("recovery"))`, passed to `App::new` (stored, given to `AppHost` as `recovery: self.recovery.as_deref()`). Headless (`headless.rs` `serve`): `recovery: None`.

`crates/app/src/ui.rs`:

```rust
/// What the window shows of the agents using the API.
pub struct Agents<'a> {
    /// Connections open now.
    pub connected: usize,
    /// The activity log's lines, newest first, while the Agent Activity window is open.
    pub log: Option<&'a [String]>,
    /// Recovery files written this session.
    pub recovered: &'a [PathBuf],
}
```

`View` gains `pub agents: Agents<'a>`; `UiAction::ShowAgentActivity(bool)` ("Open or close the Agent Activity window."). In `draw`:
- after the Camera menu: `ui.menu_button("View", |ui| { if ui.button("Agent Activity").clicked() { actions.push(UiAction::ShowAgentActivity(true)); ui.close(); } });` (use the close call the other menus use).
- in the status bar's right-to-left layout, before the measurements box (so it is rightmost): 

```rust
                if view.agents.connected > 0 {
                    let text = format!("Agents: {}", view.agents.connected);
                    if ui
                        .add(egui::Button::new(RichText::new(text).strong()).small())
                        .on_hover_text("Programs connected to the agent API")
                        .clicked()
                    {
                        actions.push(UiAction::ShowAgentActivity(true));
                    }
                }
```

- after the soften window: `if let Some(log) = view.agents.log { agent_activity(ui, &view.agents, log, &mut actions); }` with

```rust
/// The Agent Activity window: how many agents are connected, the recovery files their
/// discards left, and what they asked for, newest first.
fn agent_activity(ui: &mut Ui, agents: &Agents, log: &[String], actions: &mut Vec<UiAction>) {
    let mut open = true;
    egui::Window::new("Agent Activity")
        .open(&mut open)
        .collapsible(false)
        .default_width(420.0)
        .show(ui.ctx(), |ui| {
            ui.label(match agents.connected {
                0 => "No agents connected".to_string(),
                1 => "1 agent connected".to_string(),
                n => format!("{n} agents connected"),
            });
            if !agents.recovered.is_empty() {
                ui.label(
                    RichText::new("Unsaved changes agents discarded were kept in:")
                        .color(ui.visuals().warn_fg_color),
                );
                for path in agents.recovered {
                    ui.monospace(path.display().to_string());
                }
            }
            ui.separator();
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                if log.is_empty() {
                    ui.label(RichText::new("Nothing yet").weak());
                }
                for line in log {
                    ui.monospace(line);
                }
            });
        });
    if !open {
        actions.push(UiAction::ShowAgentActivity(false));
    }
}
```

`crates/app/src/app.rs`:

```rust
/// A line of the Agent Activity log: how long ago, the connection, the request and how it
/// went.
fn activity_line(a: &api::Activity, now: Instant) -> String {
    let secs = now.saturating_duration_since(a.at).as_secs();
    let ago = if secs < 120 {
        format!("{secs} s ago")
    } else if secs < 7200 {
        format!("{} min ago", secs / 60)
    } else {
        format!("{} h ago", secs / 3600)
    };
    let result = match &a.error {
        None => "ok".to_string(),
        Some(e) => format!("error: {e}"),
    };
    let mut line = format!("{ago}  #{}  {}  {result}", a.connection, a.request);
    if a.repeats > 1 {
        line += &format!("  (x{})", a.repeats);
    }
    line
}
```

(`connected` and `disconnected` entries end in `ok` too; keep it simple.) `State` gains `agent_activity: bool` and `recovered_seen: usize`. `render` takes what it needs from the hub: change its call site (`s.render()` in `window_event`) to `s.render(&self.hub)` and inside build `let lines: Vec<String> = self.agent_activity.then(|| hub.activity().iter().rev().map(|a| activity_line(a, Instant::now())).collect()).unwrap_or_default();` and `agents: Agents { connected: hub.open_connections(), log: self.agent_activity.then_some(&lines[..]), recovered: self.editor.recovered() }`. Before building the view: `if self.editor.recovered().len() > self.recovered_seen { self.recovered_seen = self.editor.recovered().len(); self.agent_activity = true; }`. `apply`: `UiAction::ShowAgentActivity(open) => self.agent_activity = open`. `process_api`: also request a frame when `hub.open_connections()` or (`self.agent_activity` and `hub.activity_revision()`) changed across `hub.process` (compare before/after), so the indicator and the open log stay current. While the log is open and an entry is shown, schedule a repaint each second (`self.repaint_at = Some(earliest(self.repaint_at, now + Duration::from_secs(1)))` after presenting) so the ages move.

`docs/agent-api.md`:
- Files paragraph: after "a client that means it, or has asked the user, passes `\"discard\": true`.": "In the window, `new`, `open` and `quit` with `discard: true` first write the unsaved model to a recovery file, `~/.sketchup-alt/recovery/<name>-<seconds>.skr`; the window's status bar and its Agent Activity window name it. If the file cannot be written the command fails with `file` and nothing is discarded. Headless, nothing is written."
- Access and safety: append "The window shows `Agents: N` in its status bar while N connections are open; View > Agent Activity lists the connections' requests (the last 100, newest first, with their results and how long ago they came) and any recovery files."
- Window actions table: add rows for View > Agent Activity and the status-bar indicator ("none: they only show what agents do").

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p app && $HOME/.cargo/bin/cargo test -p session`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status --short && git diff --stat
git add crates/session/src/document/files.rs crates/app/src/host.rs crates/app/src/host/tool_input_tests.rs crates/app/src/headless.rs crates/app/src/editor.rs crates/app/src/main.rs crates/app/src/app.rs crates/app/src/ui.rs docs/agent-api.md
git commit -m "Window shows connected agents and their activity; work an agent discards is kept"
```

---

## After the last task

- `$HOME/.cargo/bin/cargo fmt --all --check`, `$HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings`, `$HOME/.cargo/bin/cargo test --workspace`, `$HOME/.cargo/bin/cargo build --release -p app`.
- Manual check without the window: start `target/release/app --headless --socket <scratch>/m.sock --api-root <scratch>/root`, check `ls -l` shows `srw-------`, send `save` inside and outside the root with `nc -U`, kill only that process.
- Roadmap row `done <first>..<last>`; memory file section.
