//! Running without a window: the window's editor and the agent API on stdio or an endpoint (a
//! Unix socket, or a named pipe on Windows).

use std::path::PathBuf;

use api::{ApiRoots, Hub, listen, serve_stdio};
use glam::DVec2;
use session::Command;

use crate::cli::Transport;
use crate::editor::Editor;
use crate::host::{AppHost, CLOSE_WAIT, Offscreen};

/// The size of the view headless, in pixels: what `pick` and `infer` use without one given.
const VIEWPORT: DVec2 = DVec2::new(1280.0, 800.0);

/// Opens `file` if given, then serves until stdin closes (stdio), forever (socket), or until
/// an agent quits. Returns the process exit code.
pub fn run(transport: Transport, file: Option<PathBuf>, roots: ApiRoots) -> i32 {
    let mut editor = match editor_with(file) {
        Ok(editor) => editor,
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };
    let mut offscreen = Offscreen::headless();
    let mut hub = Hub::new(|| {});
    match transport {
        Transport::Stdio => {
            let writer = serve_stdio(hub.inbox());
            serve(&mut hub, &mut editor, &mut offscreen, &roots, |hub| {
                hub.open_connections() > 0
            });
            // Dropping the hub ends the writer once every queued reply is on stdout.
            drop(hub);
            let _ = writer.join();
            0
        }
        Transport::Socket(endpoint) => {
            if let Err(e) = listen(&endpoint, hub.inbox()) {
                eprintln!("cannot listen on {endpoint}: {e}");
                return 1;
            }
            eprintln!("agent API listening on {endpoint}");
            serve(&mut hub, &mut editor, &mut offscreen, &roots, |_| true);
            hub.close(CLOSE_WAIT);
            endpoint.remove();
            0
        }
    }
}

/// A headless editor with `file` open if given, or the message saying why it cannot be opened.
pub fn editor_with(file: Option<PathBuf>) -> Result<Editor, String> {
    let mut editor = Editor::new(VIEWPORT, 1.0);
    if let Some(path) = file {
        let open = Command::Open {
            path: path.display().to_string(),
            // A fresh model: nothing to lose.
            discard: false,
        };
        editor
            .execute(open)
            .map_err(|e| format!("cannot open {}", e.message))?;
    }
    Ok(editor)
}

/// Handles requests as they come, round by round, until `go_on` says to stop or an agent
/// quits. After each round, subscribers hear about selection, camera and editor changes.
fn serve(
    hub: &mut Hub,
    editor: &mut Editor,
    offscreen: &mut Offscreen,
    roots: &ApiRoots,
    go_on: impl Fn(&Hub) -> bool,
) {
    let mut quit = false;
    loop {
        let mut host = AppHost {
            editor: &mut *editor,
            offscreen: &mut *offscreen,
            quit: &mut quit,
            roots,
            recovery: None,
            session: None,
        };
        if !hub.process_blocking(&mut host) {
            return;
        }
        hub.notify_state(&host);
        if quit || !go_on(hub) {
            return;
        }
    }
}
