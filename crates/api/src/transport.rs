//! Byte-stream transports: newline-delimited JSON over a Unix socket or stdin/stdout.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::thread::{self, JoinHandle};

use serde_json::Value;

use crate::hub::Inbox;
use crate::protocol::{MAX_LINE_BYTES, RpcError, codes, failure};

/// Serves one connection: lines read from `reader` go to the hub, replies and notifications are
/// written to `writer`. Returns the connection id and the writer thread, which finishes once the
/// hub is dropped and every queued reply has been written.
pub fn serve_stream<R, W>(inbox: Inbox, reader: R, mut writer: W) -> (u64, JoinHandle<()>)
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let (conn, replies) = inbox.open();
    let writing = inbox.writing();
    let writer_thread = thread::spawn(move || {
        let _writing = writing;
        for line in replies {
            if writer
                .write_all(line.as_bytes())
                .and_then(|_| writer.write_all(b"\n"))
                .and_then(|_| writer.flush())
                .is_err()
            {
                break;
            }
        }
    });
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            let limit = MAX_LINE_BYTES as u64 + 1;
            match (&mut reader).take(limit).read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            if buf.len() > MAX_LINE_BYTES && buf.last() != Some(&b'\n') {
                inbox.reply(
                    conn,
                    failure(
                        &Value::Null,
                        RpcError::new(codes::INVALID_REQUEST, "request too large"),
                    ),
                );
                // Skip the rest of the oversized line.
                let mut sink = Vec::new();
                loop {
                    sink.clear();
                    match (&mut reader).take(64 * 1024).read_until(b'\n', &mut sink) {
                        Ok(0) | Err(_) => {
                            inbox.close(conn);
                            return;
                        }
                        Ok(_) if sink.last() == Some(&b'\n') => break,
                        Ok(_) => {}
                    }
                }
                continue;
            }
            // Invalid UTF-8 is replaced so the request fails to parse with a -32700 reply instead
            // of ending the connection.
            let line = String::from_utf8_lossy(&buf);
            let line = line.trim_end_matches(['\n', '\r']);
            if !line.trim().is_empty() {
                inbox.send_line(conn, line.to_string());
            }
        }
        inbox.close(conn);
    });
    (conn, writer_thread)
}

/// Serves the API on stdin/stdout as a single connection. Join the returned thread after
/// dropping the hub so the last replies reach stdout before the process exits.
pub fn serve_stdio(inbox: Inbox) -> JoinHandle<()> {
    serve_stream(inbox, io::stdin(), io::stdout()).1
}

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

/// Listens on a Unix socket readable only by the current user. A stale socket file (nobody
/// listening) at `path` is replaced; a live socket or any other existing file is an error.
pub fn listen_unix(path: &Path, inbox: Inbox) -> io::Result<()> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_socket() {
            if UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("another instance is listening on {}", path.display()),
                ));
            }
            std::fs::remove_file(path)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} exists and is not a socket", path.display()),
            ));
        }
    }
    let listener = bind_private(path)?;
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if let Ok(reader) = stream.try_clone() {
                serve_stream(inbox.clone(), reader, stream);
            }
        }
    });
    Ok(())
}
