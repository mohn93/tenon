//! Running without a window: the agent API on stdio or a Unix socket.

use std::path::PathBuf;

use api::{HeadlessHost, Hub, listen_unix, serve_stdio};
use session::Command;

use crate::cli::Transport;

/// Opens `file` if given, then serves until stdin closes (stdio) or forever (socket). Returns
/// the process exit code.
pub fn run(transport: Transport, file: Option<PathBuf>) -> i32 {
    let mut host = HeadlessHost::new();
    if let Some(path) = file {
        let open = Command::Open {
            path: path.display().to_string(),
            // A fresh model: nothing to lose.
            discard: false,
        };
        if let Err(e) = host.doc.execute(open) {
            eprintln!("cannot open {}", e.message);
            return 1;
        }
    }
    let mut hub = Hub::new(|| {});
    match transport {
        Transport::Stdio => {
            let writer = serve_stdio(hub.inbox());
            while hub.process_blocking(&mut host) && hub.open_connections() > 0 {}
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
            while hub.process_blocking(&mut host) {}
            0
        }
    }
}
