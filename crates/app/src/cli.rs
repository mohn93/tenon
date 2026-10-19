//! Command-line modes.

use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const USAGE: &str =
    "usage: app [--socket PATH] [--api-root DIR]... [FILE]            window, agent API on a Unix socket
       app --headless --stdio [--api-root DIR]... [FILE]          no window, agent API on stdin/stdout
       app --headless [--socket PATH] [--api-root DIR]... [FILE]  no window, agent API on a Unix socket
       app --mcp [--api-root DIR]... [FILE]                MCP server on stdin/stdout with its own model
       app --mcp --live SOCKET                             MCP server driving the app listening on SOCKET
FILE is a .skr model to open at start, or a SketchUp .skp file to open as a new model.
--api-root DIR (repeatable) keeps every file the API reads or writes inside DIR.";

#[derive(Debug, PartialEq)]
pub enum Transport {
    Stdio,
    Socket(PathBuf),
}

#[derive(Debug, PartialEq)]
pub enum Mode {
    Help,
    Window {
        socket: PathBuf,
        file: Option<PathBuf>,
        api_roots: Vec<PathBuf>,
    },
    Headless {
        transport: Transport,
        file: Option<PathBuf>,
        api_roots: Vec<PathBuf>,
    },
    /// An MCP server on stdin/stdout, over its own model or, with `live`, the app listening
    /// on that socket.
    Mcp {
        live: Option<PathBuf>,
        file: Option<PathBuf>,
        api_roots: Vec<PathBuf>,
    },
}

/// Unix socket paths are limited to about 104 bytes on macOS.
const MAX_SOCKET_PATH: usize = 100;

/// Directory under HOME used when the temp directory is too deep for a socket path.
pub const FALLBACK_DIR: &str = ".sketchup-alt";

/// Default socket path: per-process, in the user's temp directory, or in `~/.sketchup-alt/` when
/// the temp directory is too deep for a socket path. Never in a shared directory such as /tmp.
pub fn default_socket() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    choose_socket(&std::env::temp_dir(), home.as_deref(), std::process::id())
}

/// The temp directory is used unless it is the shared `/tmp` (Linux without TMPDIR) or too deep
/// for a socket path; then `<home>/.sketchup-alt` is used. Without a home, the temp path is
/// kept and listening reports the error.
fn choose_socket(temp_dir: &Path, home: Option<&Path>, pid: u32) -> PathBuf {
    let name = format!("sketchup-alt-{pid}.sock");
    let preferred = temp_dir.join(&name);
    let usable = temp_dir != Path::new("/tmp") && preferred.as_os_str().len() <= MAX_SOCKET_PATH;
    match home {
        Some(home) if !usable => home.join(FALLBACK_DIR).join(name),
        _ => preferred,
    }
}

/// Returns the private directory when `socket` lives in the `~/.sketchup-alt` fallback.
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
    let (mut headless, mut stdio, mut socket, mut file) = (false, false, None, None);
    let (mut mcp, mut live) = (false, None);
    let mut api_roots = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--headless" => headless = true,
            "--stdio" => stdio = true,
            "--mcp" => mcp = true,
            "--live" => {
                let path = args
                    .next()
                    .filter(|p| !p.starts_with("--"))
                    .ok_or("--live needs a socket path")?;
                live = Some(PathBuf::from(path));
            }
            "-h" | "--help" => return Ok(Mode::Help),
            "--socket" => {
                let path = args
                    .next()
                    .filter(|p| !p.starts_with("--"))
                    .ok_or("--socket needs a path")?;
                socket = Some(PathBuf::from(path));
            }
            "--api-root" => {
                let dir = args
                    .next()
                    .filter(|p| !p.starts_with("--"))
                    .ok_or("--api-root needs a folder")?;
                api_roots.push(PathBuf::from(dir));
            }
            other if !other.starts_with('-') && file.is_none() => {
                file = Some(PathBuf::from(other));
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    if live.is_some() && !mcp {
        return Err("--live needs --mcp".into());
    }
    if mcp {
        if headless || stdio || socket.is_some() {
            return Err("--mcp runs on stdin/stdout: drop --headless, --stdio and --socket".into());
        }
        if live.is_some() && (file.is_some() || !api_roots.is_empty()) {
            return Err(
                "--mcp --live drives a running app: open files and set --api-root there".into(),
            );
        }
        return Ok(Mode::Mcp {
            live,
            file,
            api_roots,
        });
    }
    if stdio && socket.is_some() {
        return Err("choose either --stdio or --socket".into());
    }
    if stdio && !headless {
        return Err("--stdio needs --headless (the window keeps stdout for logs)".into());
    }
    let socket = socket.unwrap_or_else(default_socket);
    Ok(if !headless {
        Mode::Window {
            socket,
            file,
            api_roots,
        }
    } else if stdio {
        Mode::Headless {
            transport: Transport::Stdio,
            file,
            api_roots,
        }
    } else {
        Mode::Headless {
            transport: Transport::Socket(socket),
            file,
            api_roots,
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
                socket: default_socket(),
                file: None,
                api_roots: vec![]
            }
        );
        assert_eq!(
            parse(args("--headless --stdio")).unwrap(),
            Mode::Headless {
                transport: Transport::Stdio,
                file: None,
                api_roots: vec![]
            }
        );
        assert_eq!(
            parse(args("--headless --socket /tmp/a.sock")).unwrap(),
            Mode::Headless {
                transport: Transport::Socket("/tmp/a.sock".into()),
                file: None,
                api_roots: vec![]
            }
        );
        assert_eq!(
            parse(args("--socket /tmp/b.sock")).unwrap(),
            Mode::Window {
                socket: "/tmp/b.sock".into(),
                file: None,
                api_roots: vec![]
            }
        );
    }

    #[test]
    fn a_file_to_open_can_follow_the_options() {
        assert_eq!(
            parse(args("--headless --stdio house.skr")).unwrap(),
            Mode::Headless {
                transport: Transport::Stdio,
                file: Some("house.skr".into()),
                api_roots: vec![]
            }
        );
        assert_eq!(
            parse(args("house.skr --socket /tmp/c.sock")).unwrap(),
            Mode::Window {
                socket: "/tmp/c.sock".into(),
                file: Some("house.skr".into()),
                api_roots: vec![]
            }
        );
        assert!(parse(args("a.skr b.skr")).unwrap_err().contains("b.skr"));
    }

    #[test]
    fn api_roots_repeat() {
        assert_eq!(
            parse(args("--headless --stdio --api-root /a --api-root b f.skr")).unwrap(),
            Mode::Headless {
                transport: Transport::Stdio,
                file: Some("f.skr".into()),
                api_roots: vec!["/a".into(), "b".into()],
            }
        );
        assert!(
            parse(args("--api-root"))
                .unwrap_err()
                .contains("needs a folder")
        );
    }

    #[test]
    fn mcp_modes_parse() {
        assert_eq!(
            parse(args("--mcp")).unwrap(),
            Mode::Mcp {
                live: None,
                file: None,
                api_roots: vec![]
            }
        );
        assert_eq!(
            parse(args("--mcp --api-root /a f.skr")).unwrap(),
            Mode::Mcp {
                live: None,
                file: Some("f.skr".into()),
                api_roots: vec!["/a".into()]
            }
        );
        assert_eq!(
            parse(args("--mcp --live /tmp/w.sock")).unwrap(),
            Mode::Mcp {
                live: Some("/tmp/w.sock".into()),
                file: None,
                api_roots: vec![]
            }
        );
        assert!(
            parse(args("--live /tmp/w.sock"))
                .unwrap_err()
                .contains("--mcp")
        );
        assert!(parse(args("--mcp --live /tmp/w.sock f.skr")).is_err());
        assert!(parse(args("--mcp --live /tmp/w.sock --api-root /a")).is_err());
        assert!(parse(args("--mcp --headless")).is_err());
        assert!(parse(args("--mcp --socket /tmp/x.sock")).is_err());
    }

    #[test]
    fn help_flags_return_help() {
        assert_eq!(parse(args("-h")).unwrap(), Mode::Help);
        assert_eq!(parse(args("--headless --help")).unwrap(), Mode::Help);
    }

    #[test]
    fn socket_choice() {
        let home = Path::new("/Users/me");
        let fallback = home.join(FALLBACK_DIR).join("sketchup-alt-7.sock");
        let short = Path::new("/var/folders/ab/T");
        assert_eq!(
            choose_socket(short, Some(home), 7),
            short.join("sketchup-alt-7.sock")
        );
        assert_eq!(choose_socket(Path::new("/tmp"), Some(home), 7), fallback);
        let long = PathBuf::from(format!("/{}", "x".repeat(100)));
        assert_eq!(choose_socket(&long, Some(home), 7), fallback);
        assert_eq!(
            choose_socket(&long, None, 7),
            long.join("sketchup-alt-7.sock")
        );
        assert_eq!(
            choose_socket(Path::new("/tmp"), None, 7),
            Path::new("/tmp/sketchup-alt-7.sock")
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
        assert!(
            parse(args("--socket --headless"))
                .unwrap_err()
                .contains("needs a path")
        );
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
