//! `app --mcp`: an MCP server on stdin/stdout over its own headless model, or forwarding to a
//! running app's endpoint (socket or named pipe) with `--live`. Only MCP messages go to stdout.

use std::path::PathBuf;

use api::mcp::{Backend, SocketBackend, serve_mcp};
use api::{ApiRoots, Endpoint, RpcError};
use serde_json::Value;

use crate::editor::Editor;
use crate::headless;
use crate::host::{AppHost, Offscreen};

/// A model in this process, headless as `--headless`.
struct LocalBackend {
    editor: Editor,
    offscreen: Offscreen,
    roots: ApiRoots,
    quit: bool,
}

impl Backend for LocalBackend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        let mut host = AppHost {
            editor: &mut self.editor,
            offscreen: &mut self.offscreen,
            quit: &mut self.quit,
            roots: &self.roots,
            recovery: None,
        };
        api::call(&mut host, method, params)
    }
}

/// Serves MCP until stdin closes. Returns the process exit code: 1 when `file` cannot be
/// opened or `live` cannot be connected to.
pub fn run(live: Option<Endpoint>, file: Option<PathBuf>, roots: ApiRoots) -> i32 {
    let mut backend: Box<dyn Backend> = match live {
        Some(endpoint) => match SocketBackend::connect(&endpoint) {
            Ok(backend) => Box::new(backend),
            Err(e) => {
                eprintln!("cannot connect to {endpoint}: {e}");
                return 1;
            }
        },
        None => match headless::editor_with(file) {
            Ok(editor) => Box::new(LocalBackend {
                editor,
                offscreen: Offscreen::headless(),
                roots,
                quit: false,
            }),
            Err(message) => {
                eprintln!("{message}");
                return 1;
            }
        },
    };
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    if let Err(e) = serve_mcp(backend.as_mut(), stdin.lock(), stdout.lock()) {
        eprintln!("MCP server stopped: {e}");
        return 1;
    }
    0
}
