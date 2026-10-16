//! Running without a window: the window's editor and the agent API on stdio or a Unix socket.

use std::path::PathBuf;

use api::{ApiRoots, Hub, listen_unix, serve_stdio};
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
    let mut editor = Editor::new(VIEWPORT, 1.0);
    if let Some(path) = file {
        let open = Command::Open {
            path: path.display().to_string(),
            // A fresh model: nothing to lose.
            discard: false,
        };
        if let Err(e) = editor.execute(open) {
            eprintln!("cannot open {}", e.message);
            return 1;
        }
    }
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
        Transport::Socket(path) => {
            if let Err(e) = listen_unix(&path, hub.inbox()) {
                eprintln!("cannot listen on {}: {e}", path.display());
                return 1;
            }
            eprintln!("agent API listening on {}", path.display());
            serve(&mut hub, &mut editor, &mut offscreen, &roots, |_| true);
            hub.close(CLOSE_WAIT);
            let _ = std::fs::remove_file(&path);
            0
        }
    }
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
