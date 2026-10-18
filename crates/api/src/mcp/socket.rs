//! A [`Backend`] that forwards the API's methods to a running app over its Unix socket.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::Backend;
use crate::protocol::{RpcError, codes};

/// The app listening on a Unix socket. Once the connection fails, every call fails with
/// [`codes::CONNECTION_LOST`].
pub struct SocketBackend {
    path: PathBuf,
    /// The connection's write half and buffered read half; None once it was lost.
    connection: Option<(UnixStream, BufReader<UnixStream>)>,
    next_id: u64,
}

impl SocketBackend {
    pub fn connect(path: &Path) -> io::Result<SocketBackend> {
        let stream = UnixStream::connect(path)?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(SocketBackend {
            path: path.to_path_buf(),
            connection: Some((stream, reader)),
            next_id: 1,
        })
    }

    fn lost(&self) -> RpcError {
        RpcError::new(
            codes::CONNECTION_LOST,
            format!("the connection to {} closed", self.path.display()),
        )
    }
}

impl Backend for SocketBackend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = self.next_id;
        self.next_id += 1;
        let Some((writer, reader)) = self.connection.as_mut() else {
            return Err(self.lost());
        };
        match exchange(writer, reader, id, method, params) {
            Ok(Some(reply)) => reply,
            // EOF or an I/O failure: the connection is gone for good.
            Ok(None) | Err(_) => {
                self.connection = None;
                Err(self.lost())
            }
        }
    }
}

/// Sends one request and reads lines until its reply, skipping notifications and anything
/// that is not a reply to it. None when the connection ends first.
fn exchange(
    writer: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    id: u64,
    method: &str,
    params: Value,
) -> io::Result<Option<Result<Value, RpcError>>> {
    let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    writeln!(writer, "{request}")?;
    writer.flush()?;
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(None);
        }
        let Ok(mut message) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if message.get("method").is_some() || message.get("id") != Some(&json!(id)) {
            continue;
        }
        if let Some(error) = message.get_mut("error") {
            let code = error.get("code").and_then(Value::as_i64);
            let message = error.get("message").and_then(Value::as_str);
            let mut e = RpcError::new(
                code.unwrap_or(codes::INVALID_REQUEST),
                message.unwrap_or("the app sent an error without a message"),
            );
            if let Some(data) = error.get_mut("data") {
                e = e.with_data(data.take());
            }
            return Ok(Some(Err(e)));
        }
        return Ok(Some(Ok(message
            .get_mut("result")
            .map(Value::take)
            .unwrap_or(Value::Null))));
    }
}
