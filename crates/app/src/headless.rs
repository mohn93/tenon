//! Running without a window: the agent API on stdio or a Unix socket.

use api::{HeadlessHost, Hub, listen_unix, serve_stdio};

use crate::cli::Transport;

/// Serves until stdin closes (stdio) or forever (socket). Returns the process exit code.
pub fn run(transport: Transport) -> i32 {
    let mut host = HeadlessHost::new();
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
