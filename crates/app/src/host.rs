//! The agent API's view of the app, in the window or headless: commands go through the Editor
//! (so the active tool stays consistent), editor commands set it, and `render_view` uses an
//! offscreen renderer.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use api::{
    ApiRoots, Dialog, EditorCommand, EditorState, Host, Hover, Issued, PhaseName, ToolInput,
    ToolInputError, ToolInputResult, ToolInputStopped, ToolName, snap_name,
};
use render::Snapshotter;
use session::{
    BatchCommand, BatchError, Camera, Command, CommandError, Document, ErrorKind, Lighting, Origin,
    Outcome,
};
use tools::Phase;

use crate::cli;
use crate::editor::{DriveError, Driven, Editor, Stopped};

/// How long a quitting app waits for its last replies to be written.
pub const CLOSE_WAIT: Duration = Duration::from_secs(2);

/// The offscreen renderer `render_view` uses: one on the window's GPU device, or, without a
/// window, one made on first use (None when there is no GPU).
pub struct Offscreen {
    snapshotter: Option<Snapshotter>,
    tried: bool,
}

impl Offscreen {
    /// `snapshotter`, already made.
    pub fn with(snapshotter: Snapshotter) -> Self {
        Offscreen {
            snapshotter: Some(snapshotter),
            tried: true,
        }
    }

    /// One made on its own GPU device when first needed.
    pub fn headless() -> Self {
        Offscreen {
            snapshotter: None,
            tried: false,
        }
    }

    fn get(&mut self) -> Option<&mut Snapshotter> {
        if !self.tried {
            self.tried = true;
            self.snapshotter = Snapshotter::new_headless();
        }
        self.snapshotter.as_mut()
    }
}

pub struct AppHost<'a> {
    pub editor: &'a mut Editor,
    pub offscreen: &'a mut Offscreen,
    /// Set when an agent asks the app to quit.
    pub quit: &'a mut bool,
    /// The folders the API's file paths must lie within (empty: no limit).
    pub roots: &'a ApiRoots,
    /// The folder unsaved changes are kept in when an API `new`, `open` or `quit` discards
    /// them (the window's; None keeps nothing, as headless, where the agent's work is all
    /// there is).
    pub recovery: Option<&'a Path>,
}

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
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
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
    /// Runs `f` with the editor's steps tagged as the agent's, and back to the user's after,
    /// whatever `f` returns.
    fn as_agent<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.editor.set_origin(Origin::Agent);
        let result = f(self);
        self.editor.set_origin(Origin::User);
        result
    }

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
                format!(
                    "could not keep the unsaved changes in a recovery file ({e}): nothing was discarded"
                ),
            )
        };
        if let Some(parent) = cli::private_dir_for(dir) {
            cli::ensure_private_dir(parent).map_err(|e| failed(e.to_string()))?;
        }
        cli::ensure_private_xdg_dir(crate::paths::Os::current(), dir)
            .map_err(|e| failed(e.to_string()))?;
        let path = recovery_path(dir, self.editor.doc().path(), SystemTime::now());
        self.editor
            .doc()
            .write_copy(&path)
            .map_err(|e| failed(e.message))?;
        Ok(Some(path))
    }

    /// Runs `command`, first keeping the work it would discard.
    fn execute_keeping(&mut self, command: Command) -> Result<Outcome, CommandError> {
        let kept = if discards(&command) {
            self.keep_discarded()?
        } else {
            None
        };
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

impl Host for AppHost<'_> {
    fn roots(&self) -> Option<&ApiRoots> {
        (!self.roots.is_empty()).then_some(self.roots)
    }

    fn document(&self) -> &Document {
        self.editor.doc()
    }

    fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
        self.as_agent(|host| host.execute_keeping(command))
    }

    fn execute_batch(
        &mut self,
        commands: Vec<BatchCommand>,
        atomic: bool,
    ) -> Result<Vec<Outcome>, BatchError> {
        self.as_agent(|host| {
            // Run one by one when a command would discard work, so it is kept at its turn.
            if !atomic && host.recovery.is_some() && commands.iter().any(|c| discards(c.command()))
            {
                let mut completed = Vec::new();
                for (index, command) in commands.iter().enumerate() {
                    match command
                        .resolve(&completed)
                        .and_then(|command| host.execute_keeping(command))
                    {
                        Ok(outcome) => completed.push(outcome),
                        Err(error) => {
                            return Err(BatchError {
                                index,
                                error,
                                completed,
                                rolled_back: false,
                            });
                        }
                    }
                }
                return Ok(completed);
            }
            host.editor.execute_batch(commands, atomic)
        })
    }

    fn render_view(
        &mut self,
        camera: &Camera,
        lighting: Lighting,
        width: u32,
        height: u32,
    ) -> Option<Vec<u8>> {
        let doc = self.editor.doc();
        self.offscreen.get()?.capture(
            doc.scene(),
            doc.context(),
            doc.selection(),
            camera,
            lighting,
            width,
            height,
        )
    }

    fn editor(&self) -> Option<EditorState> {
        Some(editor_state(self.editor))
    }

    fn editor_command(&mut self, command: EditorCommand) -> Result<(), CommandError> {
        self.as_agent(|host| {
            let editor = &mut *host.editor;
            match command {
                EditorCommand::SetTool { tool } => editor.select_tool(tool.into()),
                EditorCommand::SetPaintMaterial { material } => {
                    editor.set_paint_material(material)?
                }
                EditorCommand::SetImportUnits { units } => editor.set_import_units(units),
                EditorCommand::Cancel {} => editor.cancel(),
                EditorCommand::SetFlyMode { on, speed } => editor.set_fly_mode(on, speed)?,
                EditorCommand::SetDialog { dialog } => {
                    editor.show_soften(dialog == Some(Dialog::SoftenEdges));
                }
                EditorCommand::SetSoftenAngle { angle } => {
                    editor.set_soften_angle_checked(angle)?
                }
                EditorCommand::Duplicate {} => editor.duplicate()?,
            }
            Ok(())
        })
    }

    fn tool_input(&mut self, input: ToolInput) -> Result<ToolInputResult, ToolInputError> {
        self.as_agent(|host| match host.editor.drive(input) {
            Ok(driven) => Ok(tool_input_result(host.editor, driven)),
            Err(DriveError::Refused(e)) => Err(ToolInputError::Refused(e)),
            Err(DriveError::Stopped(stopped)) => {
                let Stopped {
                    event,
                    error,
                    driven,
                } = *stopped;
                Err(ToolInputError::Stopped(Box::new(ToolInputStopped {
                    error,
                    event,
                    completed: tool_input_result(host.editor, driven),
                })))
            }
        })
    }

    fn quit(&mut self, discard: bool) -> Result<(), CommandError> {
        self.editor.doc().check_unsaved(discard)?;
        if let Some(path) = self.keep_discarded()? {
            self.editor.note_recovery(path);
        }
        *self.quit = true;
        Ok(())
    }

    fn quitting(&self) -> bool {
        *self.quit
    }
}

/// What `tool_input` reports after the events that ran.
fn tool_input_result(editor: &Editor, driven: Driven) -> ToolInputResult {
    ToolInputResult {
        editor: editor_state(editor),
        inference: driven.hover.map(|h| {
            let (kind, axis) = snap_name(h.kind);
            Hover {
                kind,
                axis,
                point: h.point.to_array(),
            }
        }),
        preview: driven
            .preview
            .iter()
            .map(|[a, b]| [a.to_array(), b.to_array()])
            .collect(),
        commands: driven
            .issued
            .into_iter()
            .map(|(command, result)| match result {
                Ok(outcome) => Issued {
                    command,
                    outcome: Some(outcome),
                    error: None,
                },
                Err(error) => Issued {
                    command,
                    outcome: None,
                    error: Some(error),
                },
            })
            .collect(),
        revision: editor.doc().revision(),
    }
}

/// The editor's state as agents see it.
pub fn editor_state(editor: &Editor) -> EditorState {
    let status = editor.status_parts();
    let (phase, points) = match editor.phase() {
        Phase::Idle => (PhaseName::Idle, Vec::new()),
        Phase::Active { points } => (
            PhaseName::Active,
            points.iter().map(|p| p.to_array()).collect(),
        ),
    };
    let typed = editor.typed();
    let (soften_angle, soften_open) = editor.soften_state();
    let viewport = editor.doc().camera.viewport;
    EditorState {
        tool: ToolName::from(editor.tool_kind()),
        phase,
        points,
        paint_material: editor.current_material(),
        import_units: editor.import_units(),
        flying: editor.flying(),
        fly_speed: editor.flying_speed(),
        viewport: viewport.to_array(),
        status: status.to_string(),
        prompt: status.prompt.clone(),
        message: status.message.clone(),
        message_kind: editor.message_kind(),
        lock: status.lock.map(str::to_string),
        measurement_label: status.label.to_string(),
        measurement: if typed.is_empty() {
            status.value.clone()
        } else {
            Some(typed.to_string())
        },
        typing: !typed.is_empty(),
        soften_angle: soften_angle.into(),
        dialog: soften_open.then_some(Dialog::SoftenEdges),
        modifiers: editor.modifiers(),
        hovered: editor.hovered(),
        snap_label: editor.snap_label(),
    }
}

#[cfg(test)]
mod tool_input_tests;

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use api::{ToolName, handle};
    use glam::{DVec2, DVec3};
    use serde_json::{Value, json};
    use session::ImportUnits;
    use tools::ToolKind;

    use super::*;
    use crate::editor::EditorKey;
    use crate::navigation::Button;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    /// Sends one request through an `AppHost` and returns the reply.
    fn call(e: &mut Editor, quit: &mut bool, method: &str, params: Value) -> Value {
        let mut offscreen = Offscreen::headless();
        let mut host = AppHost {
            editor: e,
            offscreen: &mut offscreen,
            quit,
            roots: &ApiRoots::default(),
            recovery: None,
        };
        let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        serde_json::from_str(&handle(&mut host, &line.to_string()).unwrap()).unwrap()
    }

    /// The origins of the undo steps, top first.
    fn undo_origins(e: &mut Editor) -> Value {
        let mut quit = false;
        call(e, &mut quit, "query", json!({"type": "history"}))["result"]["undo_origins"].clone()
    }

    #[test]
    fn api_edits_are_agent_steps_and_window_edits_are_user_steps() {
        let mut e = editor();
        let mut quit = false;
        call(&mut e, &mut quit, "execute", add_edge());
        assert_eq!(undo_origins(&mut e), json!(["agent"]));
        // The window runs its edits through the editor directly.
        e.execute(Command::AddEdge {
            start: [0.0, 1.0, 0.0],
            end: [1.0, 1.0, 0.0],
        })
        .unwrap();
        assert_eq!(undo_origins(&mut e), json!(["user", "agent"]));
        // An agent's guarded undo is refused while the user's step is on top.
        let r = call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "undo", "expect": {"label": "add_edge", "origin": "agent"}}),
        );
        assert_eq!(r["error"]["data"]["kind"], "unexpected_step", "{r}");
        assert_eq!(undo_origins(&mut e), json!(["user", "agent"]));
    }

    #[test]
    fn a_failed_api_command_leaves_the_origin_user() {
        let mut e = editor();
        let mut quit = false;
        let r = call(&mut e, &mut quit, "execute", json!({"type": "undo"}));
        assert!(!r["error"].is_null());
        e.execute(Command::AddEdge {
            start: [0.0, 0.0, 0.0],
            end: [1.0, 0.0, 0.0],
        })
        .unwrap();
        assert_eq!(undo_origins(&mut e), json!(["user"]));
    }

    #[test]
    fn tool_input_and_non_atomic_batches_make_agent_steps() {
        let mut e = editor();
        let mut quit = false;
        call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_tool", "tool": "line"}),
        );
        let r = call(
            &mut e,
            &mut quit,
            "tool_input",
            json!({"events": [
                {"type": "click", "point": [0, 0, 0]},
                {"type": "click", "point": [1, 0, 0]}
            ]}),
        );
        assert!(r["error"].is_null(), "{r}");
        assert_eq!(undo_origins(&mut e), json!(["agent"]));

        let mut e = editor();
        let r = call(
            &mut e,
            &mut quit,
            "batch",
            json!({"commands": [{"type": "new", "discard": true}, add_edge()]}),
        );
        assert!(r["error"].is_null(), "{r}");
        assert_eq!(undo_origins(&mut e), json!(["agent"]));
    }

    #[test]
    fn the_state_shows_the_tool_its_points_and_the_measurements_box() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        let a = DVec3::new(1.0, 1.0, 0.0);
        let px = e.doc().camera.project(a).unwrap();
        e.cursor_moved(px);
        e.button(Button::Left, true);
        e.button(Button::Left, false);
        e.key(EditorKey::Char('2'));
        let s = editor_state(&e);
        assert_eq!(s.tool, ToolName::Line);
        assert_eq!(s.phase, PhaseName::Active);
        assert_eq!(s.points.len(), 1);
        assert!(DVec3::from(s.points[0]).distance(a) < 1e-9);
        assert_eq!((s.typing, s.measurement.as_deref()), (true, Some("2")));
        assert_eq!(s.measurement_label, "Length");
        assert_eq!(s.prompt, "Click the next point, or type a length");
        assert!(s.status.starts_with("Line  |  "), "{}", s.status);
        assert_eq!(s.viewport, [800.0, 600.0]);
        assert_eq!((s.soften_angle, s.dialog), (20, None));
        e.show_soften(true);
        assert_eq!(editor_state(&e).dialog, Some(Dialog::SoftenEdges));
    }

    #[test]
    fn editor_commands_reach_the_editor() {
        let (mut e, mut quit) = (editor(), false);
        let r = call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_tool", "tool": "tape_measure"}),
        );
        assert_eq!(r["result"]["editor"]["tool"], "tape_measure", "{r}");
        assert_eq!(e.tool_kind(), ToolKind::TapeMeasure);
        call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_import_units", "units": "inches"}),
        );
        assert_eq!(e.import_units(), ImportUnits::Inches);
        let r = call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_paint_material", "material": "m42"}),
        );
        assert_eq!(r["error"]["data"]["kind"], "not_found", "{r}");
        let r = call(&mut e, &mut quit, "query", json!({"type": "editor"}));
        assert_eq!(r["result"]["import_units"], "inches");
        assert!(!quit);
    }

    #[test]
    fn agents_set_fly_mode_and_the_soften_window() {
        let (mut e, mut quit) = (editor(), false);
        let r = call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_fly_mode", "on": true, "speed": 10.0}),
        );
        assert_eq!(r["result"]["editor"]["flying"], true, "{r}");
        assert!(e.flying());
        assert_eq!(e.flying_speed(), 10.0);
        let r = call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_fly_mode", "on": false, "speed": 100.0}),
        );
        assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
        assert!(e.flying(), "a refused command changes nothing");
        assert_eq!(e.flying_speed(), 10.0);
        // `cancel` leaves fly mode, as Escape does.
        call(&mut e, &mut quit, "execute", json!({"type": "cancel"}));
        assert!(!e.flying());

        let r = call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_dialog", "dialog": "soften_edges"}),
        );
        assert_eq!(r["result"]["editor"]["dialog"], "soften_edges", "{r}");
        let r = call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_soften_angle", "angle": 35}),
        );
        assert_eq!(r["result"]["editor"]["soften_angle"], 35, "{r}");
        assert_eq!(e.soften_window(), Some(35));
        for angle in [181, 300] {
            let r = call(
                &mut e,
                &mut quit,
                "execute",
                json!({"type": "set_soften_angle", "angle": angle}),
            );
            assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
        }
        assert_eq!(e.soften_window(), Some(35));
        call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_dialog", "dialog": null}),
        );
        assert_eq!(e.soften_window(), None);
    }

    #[test]
    fn after_an_accepted_quit_nothing_more_is_done() {
        let (mut e, mut quit) = (editor(), false);
        e.select_tool(ToolKind::Line);
        let r = call(&mut e, &mut quit, "quit", json!({}));
        assert_eq!(r["result"], json!({}));
        let r = call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
        );
        assert_eq!(r["error"]["message"], "the program is quitting", "{r}");
        assert!(!e.doc().is_modified());
        let r = call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_tool", "tool": "erase"}),
        );
        assert_eq!(r["error"]["message"], "the program is quitting", "{r}");
        assert_eq!(e.tool_kind(), ToolKind::Line);
    }

    #[test]
    fn quit_is_refused_with_unsaved_changes_unless_discarded() {
        let (mut e, mut quit) = (editor(), false);
        e.execute(Command::AddEdge {
            start: [0.0; 3],
            end: [1.0, 0.0, 0.0],
        })
        .unwrap();
        let r = call(&mut e, &mut quit, "quit", Value::Null);
        assert_eq!(r["error"]["data"]["kind"], "unsaved_changes", "{r}");
        assert!(!quit);
        let r = call(&mut e, &mut quit, "quit", json!({"discard": true}));
        assert_eq!(r["result"], json!({}));
        assert!(quit);
        assert!(e.recovered().is_empty());
    }

    fn recovery_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sketchup-alt-recovery-{}-{name}",
            std::process::id()
        ));
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
        let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        serde_json::from_str(&handle(&mut host, &line.to_string()).unwrap()).unwrap()
    }

    fn files_in(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map_or_else(|_| Vec::new(), |r| r.map(|e| e.unwrap().path()).collect())
    }

    fn add_edge() -> Value {
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]})
    }

    #[test]
    fn work_an_agent_discards_is_kept_in_a_recovery_file() {
        let dir = recovery_dir("new");
        let mut e = editor();
        // Nothing unsaved: nothing kept.
        call_kept(
            &mut e,
            &dir,
            "execute",
            json!({"type": "new", "discard": true}),
        );
        assert!(files_in(&dir).is_empty());
        call_kept(&mut e, &dir, "execute", add_edge());
        let r = call_kept(
            &mut e,
            &dir,
            "execute",
            json!({"type": "new", "discard": true}),
        );
        assert!(r["result"].is_object(), "{r}");
        let kept = files_in(&dir);
        assert_eq!(kept.len(), 1);
        assert!(
            kept[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("untitled-")
        );
        assert_eq!(e.recovered(), &kept[..]);
        assert!(
            e.status_parts()
                .message
                .unwrap()
                .contains(&kept[0].display().to_string())
        );
        #[cfg(unix)]
        {
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        // The file holds the dropped edge.
        let mut back = editor();
        back.execute(Command::Open {
            path: kept[0].display().to_string(),
            discard: false,
        })
        .unwrap();
        assert_eq!(back.doc().model().edges().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_open_keeps_nothing_and_drops_nothing() {
        let dir = recovery_dir("open");
        let mut e = editor();
        call_kept(&mut e, &dir, "execute", add_edge());
        let r = call_kept(
            &mut e,
            &dir,
            "execute",
            json!({"type": "open", "path": "/no/such.skr", "discard": true}),
        );
        assert_eq!(r["error"]["data"]["kind"], "file", "{r}");
        assert!(files_in(&dir).is_empty());
        assert!(e.doc().is_modified());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_batch_that_discards_keeps_the_work_at_that_command() {
        let dir = recovery_dir("batch");
        let mut e = editor();
        let r = call_kept(
            &mut e,
            &dir,
            "batch",
            json!({"commands": [
                add_edge(),
                {"type": "new", "discard": true},
                {"type": "add_edge", "start": [0, 0, 0], "end": [0, 1, 0]}
            ]}),
        );
        assert_eq!(r["result"]["outcomes"].as_array().unwrap().len(), 3, "{r}");
        assert_eq!(files_in(&dir).len(), 1);
        let r = call_kept(
            &mut e,
            &dir,
            "batch",
            json!({"commands": [
                {"type": "new", "discard": true},
                {"type": "open", "path": "/no/such.skr", "discard": true}
            ]}),
        );
        assert_eq!(r["error"]["data"]["index"], 1, "{r}");
        assert_eq!(r["error"]["data"]["completed"].as_array().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_batch_that_discards_resolves_references() {
        let dir = recovery_dir("batch-refs");
        let mut e = editor();
        let r = call_kept(
            &mut e,
            &dir,
            "batch",
            json!({"commands": [
                {"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]},
                {"type": "push_pull", "face": "$0:f0", "distance": 1},
                {"type": "new", "discard": true},
                add_edge()
            ]}),
        );
        assert_eq!(r["result"]["outcomes"].as_array().unwrap().len(), 4, "{r}");
        let kept = files_in(&dir);
        assert_eq!(kept.len(), 1);
        let mut back = editor();
        back.execute(Command::Open {
            path: kept[0].display().to_string(),
            discard: false,
        })
        .unwrap();
        assert_eq!(back.doc().model().faces().count(), 6);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_batch_that_discards_resolves_references_made_after_the_new() {
        let dir = recovery_dir("batch-refs-after");
        let mut e = editor();
        let r = call_kept(
            &mut e,
            &dir,
            "batch",
            json!({"commands": [
                add_edge(),
                {"type": "new", "discard": true},
                {"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]},
                {"type": "push_pull", "face": "$2:f0", "distance": 1}
            ]}),
        );
        assert_eq!(r["result"]["outcomes"].as_array().unwrap().len(), 4, "{r}");
        assert_eq!(e.doc().model().faces().count(), 6);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn quitting_with_discard_keeps_the_work_and_a_folder_open_to_others_is_refused() {
        let dir = recovery_dir("quit");
        let mut e = editor();
        call_kept(&mut e, &dir, "execute", add_edge());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let r = call_kept(&mut e, &dir, "quit", json!({"discard": true}));
        assert_eq!(r["error"]["data"]["kind"], "file", "{r}");
        assert!(
            r["error"]["message"]
                .as_str()
                .unwrap()
                .contains("nothing was discarded")
        );
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let r = call_kept(&mut e, &dir, "quit", json!({"discard": true}));
        assert!(r["result"].is_object(), "{r}");
        assert_eq!(files_in(&dir).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_sketchup_alt_folder_is_made_private_with_the_recovery_folder_in_it() {
        let base = recovery_dir("home");
        let dir = base.join(cli::FALLBACK_DIR).join("recovery");
        std::fs::create_dir_all(&base).unwrap();
        let mut e = editor();
        call_kept(&mut e, &dir, "execute", add_edge());
        let r = call_kept(
            &mut e,
            &dir,
            "execute",
            json!({"type": "new", "discard": true}),
        );
        assert!(r["result"].is_object(), "{r}");
        for d in [dir.parent().unwrap(), &dir] {
            let mode = std::fs::metadata(d).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{}", d.display());
        }
        assert_eq!(files_in(&dir).len(), 1);
        // Open to others: refused, nothing discarded.
        std::fs::set_permissions(
            dir.parent().unwrap(),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        call_kept(&mut e, &dir, "execute", add_edge());
        let r = call_kept(
            &mut e,
            &dir,
            "execute",
            json!({"type": "new", "discard": true}),
        );
        assert_eq!(r["error"]["data"]["kind"], "file", "{r}");
        assert!(e.doc().is_modified());
        assert_eq!(files_in(&dir).len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn a_recovery_folder_that_is_a_link_is_refused_and_nothing_is_discarded() {
        let dir = recovery_dir("link");
        let target = recovery_dir("link-target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&target, &dir).unwrap();
        let mut e = editor();
        call_kept(&mut e, &dir, "execute", add_edge());
        let r = call_kept(
            &mut e,
            &dir,
            "execute",
            json!({"type": "new", "discard": true}),
        );
        assert_eq!(r["error"]["data"]["kind"], "file", "{r}");
        assert!(files_in(&target).is_empty());
        assert!(e.doc().is_modified());
        assert!(e.recovered().is_empty());
        let _ = std::fs::remove_file(&dir);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[cfg(windows)]
    #[test]
    fn prefs_and_recovery_work_under_temp_application_data_folders() {
        let base = recovery_dir("appdata");
        let (roaming, local) = (base.join("Roaming"), base.join("Local"));
        std::fs::create_dir_all(&roaming).unwrap();
        std::fs::create_dir_all(&local).unwrap();
        let vars = [("APPDATA", &roaming), ("LOCALAPPDATA", &local)];
        let env = |k: &str| {
            vars.iter()
                .find(|(name, _)| *name == k)
                .map(|(_, dir)| dir.as_os_str().to_owned())
        };
        let found = crate::paths::resolve(
            crate::paths::Os::Windows,
            &env,
            &std::env::temp_dir(),
            std::process::id(),
        )
        .unwrap();
        assert_eq!(found.prefs, roaming.join("sketchup-alt").join("prefs.json"));
        assert_eq!(found.recovery, local.join("sketchup-alt").join("recovery"));
        // Preferences: the folder and the file are the user's only.
        let prefs = crate::prefs::Prefs {
            recent: vec![base.join("house.skr")],
            ..Default::default()
        };
        crate::prefs::save(&found.prefs, &prefs).unwrap();
        assert_eq!(crate::prefs::load(&found.prefs), prefs);
        let owner_only = |p: &Path| {
            let sddl = session::windows::security_text(p).unwrap();
            session::windows::check_owner_only(&sddl, None)
                .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        };
        owner_only(found.prefs.parent().unwrap());
        owner_only(&found.prefs);
        // Recovery: both missing folders are made, and the kept file is the user's only.
        let mut e = editor();
        call_kept(&mut e, &found.recovery, "execute", add_edge());
        let r = call_kept(
            &mut e,
            &found.recovery,
            "execute",
            json!({"type": "new", "discard": true}),
        );
        assert!(r["result"].is_object(), "{r}");
        let kept = files_in(&found.recovery);
        assert_eq!(kept.len(), 1);
        owner_only(found.recovery.parent().unwrap());
        owner_only(&found.recovery);
        owner_only(&kept[0]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn recovery_names_follow_the_open_file_and_never_collide() {
        let dir = recovery_dir("names");
        std::fs::create_dir_all(&dir).unwrap();
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let a = recovery_path(&dir, Some(Path::new("/x/My house?.skr")), now);
        assert_eq!(a, dir.join("My house_-1700000000.skr"));
        std::fs::write(&a, "").unwrap();
        assert_eq!(
            recovery_path(&dir, Some(Path::new("/x/My house?.skr")), now),
            dir.join("My house_-1700000000-2.skr")
        );
        assert_eq!(
            recovery_path(&dir, None, now),
            dir.join("untitled-1700000000.skr")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
