//! Byte-stream transports: newline-delimited JSON over a Unix socket or stdin/stdout.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::thread::{self, JoinHandle};

use crate::hub::Inbox;

/// Serves one connection: lines read from `reader` go to the hub, replies and notifications are
/// written to `writer`. Returns the connection id and the writer thread, which finishes once the
/// hub is dropped and every queued reply has been written.
pub fn serve_stream<R, W>(inbox: Inbox, reader: R, mut writer: W) -> (u64, JoinHandle<()>)
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let (conn, replies) = inbox.open();
    let writer_thread = thread::spawn(move || {
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
        for line in BufReader::new(reader).lines() {
            match line {
                Ok(line) if line.trim().is_empty() => continue,
                Ok(line) => inbox.send_line(conn, line),
                Err(_) => break,
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

/// Listens on a Unix socket readable only by the current user. A stale socket file at `path` is
/// replaced; any other existing file is an error.
pub fn listen_unix(path: &Path, inbox: Inbox) -> io::Result<()> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_socket() {
            std::fs::remove_file(path)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} exists and is not a socket", path.display()),
            ));
        }
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if let Ok(reader) = stream.try_clone() {
                serve_stream(inbox.clone(), reader, stream);
            }
        }
    });
    Ok(())
}
