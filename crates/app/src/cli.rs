//! Command-line modes.

use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const USAGE: &str =
    "usage: app [--socket PATH]                 window, agent API on a Unix socket
       app --headless --stdio               no window, agent API on stdin/stdout
       app --headless [--socket PATH]       no window, agent API on a Unix socket";

#[derive(Debug, PartialEq)]
pub enum Transport {
    Stdio,
    Socket(PathBuf),
}

#[derive(Debug, PartialEq)]
pub enum Mode {
    Window { socket: PathBuf },
    Headless { transport: Transport },
}

/// Unix socket paths are limited to about 104 bytes on macOS.
const MAX_SOCKET_PATH: usize = 100;

/// Directory under HOME used when the temp directory is too deep for a socket path.
pub const FALLBACK_DIR: &str = ".sketchup-alt";

/// Default socket path: per-process, in the user's temp directory, or in `~/.sketchup-alt/` when
/// the temp directory is too deep for a socket path. Never in a shared directory such as /tmp.
pub fn default_socket() -> PathBuf {
    let name = format!("sketchup-alt-{}.sock", std::process::id());
    let preferred = std::env::temp_dir().join(&name);
    if preferred.as_os_str().len() <= MAX_SOCKET_PATH {
        return preferred;
    }
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(FALLBACK_DIR).join(name),
        // Listening on the too-long path reports the error.
        None => preferred,
    }
}

/// True when `socket` lives in the `~/.sketchup-alt` fallback directory.
pub fn private_dir_for(socket: &Path) -> Option<&Path> {
    let parent = socket.parent()?;
    (parent.file_name()? == FALLBACK_DIR).then_some(parent)
}

/// Makes sure `dir` exists, is a real directory (not a symlink) and is closed to group and
/// others. A missing directory is created with mode 0700; its parent must exist.
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => std::fs::DirBuilder::new()
            .recursive(false)
            .mode(0o700)
            .create(dir),
        Err(e) => Err(e),
        Ok(meta) => {
            if !meta.file_type().is_dir() {
                return Err(io::Error::other(format!(
                    "{} is not a real directory",
                    dir.display()
                )));
            }
            if meta.permissions().mode() & 0o077 != 0 {
                return Err(io::Error::other(format!(
                    "{} is accessible to other users; chmod 700 it",
                    dir.display()
                )));
            }
            Ok(())
        }
    }
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Mode, String> {
    let (mut headless, mut stdio, mut socket) = (false, false, None);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--headless" => headless = true,
            "--stdio" => stdio = true,
            "--socket" => {
                let path = args.next().ok_or("--socket needs a path")?;
                socket = Some(PathBuf::from(path));
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    if stdio && socket.is_some() {
        return Err("choose either --stdio or --socket".into());
    }
    if stdio && !headless {
        return Err("--stdio needs --headless (the window keeps stdout for logs)".into());
    }
    let socket = socket.unwrap_or_else(default_socket);
    Ok(if !headless {
        Mode::Window { socket }
    } else if stdio {
        Mode::Headless {
            transport: Transport::Stdio,
        }
    } else {
        Mode::Headless {
            transport: Transport::Socket(socket),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("sketchup-alt-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn modes_parse() {
        assert_eq!(
            parse(args("")).unwrap(),
            Mode::Window {
                socket: default_socket()
            }
        );
        assert_eq!(
            parse(args("--headless --stdio")).unwrap(),
            Mode::Headless {
                transport: Transport::Stdio
            }
        );
        assert_eq!(
            parse(args("--headless --socket /tmp/a.sock")).unwrap(),
            Mode::Headless {
                transport: Transport::Socket("/tmp/a.sock".into())
            }
        );
        assert_eq!(
            parse(args("--socket /tmp/b.sock")).unwrap(),
            Mode::Window {
                socket: "/tmp/b.sock".into()
            }
        );
    }

    #[test]
    fn default_socket_path_fits_the_os_limit() {
        assert!(default_socket().as_os_str().len() <= MAX_SOCKET_PATH);
    }

    #[test]
    fn bad_arguments_are_explained() {
        assert!(parse(args("--stdio")).unwrap_err().contains("--headless"));
        assert!(parse(args("--headless --stdio --socket x")).is_err());
        assert!(parse(args("--socket")).is_err());
        assert!(parse(args("--fly")).unwrap_err().contains("--fly"));
    }

    #[test]
    fn ensure_private_dir_creates_a_0700_dir() {
        let dir = scratch("create");
        let _ = std::fs::remove_dir_all(&dir);
        ensure_private_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        // An existing private directory is accepted.
        ensure_private_dir(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ensure_private_dir_refuses_a_symlink() {
        let target = scratch("link-target");
        let link = scratch("link");
        let _ = std::fs::remove_dir_all(&target);
        let _ = std::fs::remove_file(&link);
        ensure_private_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(ensure_private_dir(&link).is_err());
        std::fs::remove_file(&link).unwrap();
        std::fs::remove_dir_all(&target).unwrap();
    }

    #[test]
    fn ensure_private_dir_refuses_a_group_readable_dir() {
        let dir = scratch("open");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o750)).unwrap();
        assert!(ensure_private_dir(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
