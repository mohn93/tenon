//! Command-line modes.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use api::Endpoint;

#[cfg(unix)]
use crate::paths;

pub const USAGE: &str =
    "usage: app [--socket NAME] [--api-root DIR]... [FILE]            window, agent API on a local socket
       app --headless --stdio [--api-root DIR]... [FILE]          no window, agent API on stdin/stdout
       app --headless [--socket NAME] [--api-root DIR]... [FILE]  no window, agent API on a local socket
       app --mcp [--api-root DIR]... [FILE]                MCP server on stdin/stdout with its own model
       app --mcp --live NAME                               MCP server driving the app listening on NAME
FILE is a .skr model to open at start, or a SketchUp .skp file to open as a new model.
NAME is a Unix socket path, or on Windows a named pipe (\\\\.\\pipe\\ is added to a bare name).
--api-root DIR (repeatable) keeps every file the API reads or writes inside DIR.";

#[derive(Debug, PartialEq)]
pub enum Transport {
    Stdio,
    Socket(Endpoint),
}

#[derive(Debug, PartialEq)]
pub enum Mode {
    Help,
    Window {
        socket: Endpoint,
        file: Option<PathBuf>,
        api_roots: Vec<PathBuf>,
    },
    Headless {
        transport: Transport,
        file: Option<PathBuf>,
        api_roots: Vec<PathBuf>,
    },
    /// An MCP server on stdin/stdout, over its own model or, with `live`, the app listening
    /// on that endpoint.
    Mcp {
        live: Option<Endpoint>,
        file: Option<PathBuf>,
        api_roots: Vec<PathBuf>,
    },
}

/// Base folder under HOME for the files that live there on macOS, and for the socket when the
/// preferred folder is unusable.
pub const FALLBACK_DIR: &str = ".sketchup-alt";

/// Default socket path: per-process, in the platform's private folder (see `paths`). Never in
/// a shared directory such as /tmp.
#[cfg(unix)]
pub fn default_endpoint() -> Endpoint {
    Endpoint::Socket(paths::default_socket(
        &|k| std::env::var_os(k),
        &std::env::temp_dir(),
        std::process::id(),
    ))
}

/// Default pipe: per-process.
#[cfg(windows)]
pub fn default_endpoint() -> Endpoint {
    Endpoint::from_arg(&format!("sketchup-alt-{}", std::process::id()))
}

/// Returns the private directory when `socket` lives in the `~/.sketchup-alt` fallback.
pub fn private_dir_for(socket: &Path) -> Option<&Path> {
    let parent = socket.parent()?;
    (parent.file_name()? == FALLBACK_DIR).then_some(parent)
}

/// `private_dir_for` an endpoint's socket.
#[cfg(unix)]
pub fn private_dir_for_endpoint(endpoint: &Endpoint) -> Option<&Path> {
    let Endpoint::Socket(socket) = endpoint;
    private_dir_for(socket)
}

/// A pipe needs no folder.
#[cfg(windows)]
pub fn private_dir_for_endpoint(_: &Endpoint) -> Option<&Path> {
    None
}

/// Makes sure `dir` exists, is a real directory (not a symlink, nor on Windows a junction or
/// other reparse point) and is closed to other users. A missing directory is created private
/// (Unix: mode 0700; Windows: a protected DACL for the current user only, inherited by what is
/// made in it); its parent must exist. An existing one must be mode 0700 on Unix; on Windows
/// its permissions are left as they are.
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => create_private_dir(dir),
        Err(e) => Err(e),
        Ok(meta) => {
            if !meta.file_type().is_dir() || session::fspath::is_link(&meta) {
                return Err(io::Error::other(format!(
                    "{} is not a real directory",
                    dir.display()
                )));
            }
            check_private(dir, &meta)
        }
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(false)
        .mode(0o700)
        .create(dir)
}

#[cfg(windows)]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    session::windows::create_private_dir(dir)
}

#[cfg(unix)]
fn check_private(dir: &Path, meta: &std::fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::other(format!(
            "{} is accessible to other users; chmod 700 it",
            dir.display()
        )));
    }
    Ok(())
}

/// An existing folder's ACL is the user's choice: not checked or rewritten.
#[cfg(windows)]
fn check_private(_: &Path, _: &std::fs::Metadata) -> io::Result<()> {
    Ok(())
}

/// `ensure_private_dir` for a folder under an XDG base or a Windows application data folder:
/// on Linux and Windows missing parent folders are created too, private (only `dir` itself is
/// checked, so an existing parent such as `~/.config` keeps its mode). Elsewhere it is
/// `ensure_private_dir`, so a missing parent (such as HOME) stays an error.
pub fn ensure_private_xdg_dir(os: crate::paths::Os, dir: &Path) -> io::Result<()> {
    let missing = |p: &&Path| !p.as_os_str().is_empty() && std::fs::symlink_metadata(p).is_err();
    #[cfg(unix)]
    if os == crate::paths::Os::Linux
        && let Some(parent) = dir.parent().filter(missing)
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    if os == crate::paths::Os::Windows {
        // Outermost first, each made private like `dir`.
        let parents: Vec<&Path> = dir.ancestors().skip(1).take_while(missing).collect();
        for parent in parents.into_iter().rev() {
            ensure_private_dir(parent)?;
        }
    }
    ensure_private_dir(dir)
}

/// The command-line arguments as text; one that is not valid Unicode is an error naming it.
pub fn unicode_args(args: impl IntoIterator<Item = OsString>) -> Result<Vec<String>, String> {
    args.into_iter()
        .map(|arg| {
            arg.into_string()
                .map_err(|arg| format!("the argument {arg:?} is not valid Unicode"))
        })
        .collect()
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
                let name = args
                    .next()
                    .filter(|p| !p.starts_with("--"))
                    .ok_or("--live needs a socket path (a pipe name on Windows)")?;
                live = Some(Endpoint::from_arg(&name));
            }
            "-h" | "--help" => return Ok(Mode::Help),
            "--socket" => {
                let name = args
                    .next()
                    .filter(|p| !p.starts_with("--"))
                    .ok_or("--socket needs a path (a pipe name on Windows)")?;
                socket = Some(Endpoint::from_arg(&name));
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
    let socket = socket.unwrap_or_else(default_endpoint);
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
    #[cfg(unix)]
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
                socket: default_endpoint(),
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
                transport: Transport::Socket(Endpoint::from_arg("/tmp/a.sock")),
                file: None,
                api_roots: vec![]
            }
        );
        assert_eq!(
            parse(args("--socket /tmp/b.sock")).unwrap(),
            Mode::Window {
                socket: Endpoint::from_arg("/tmp/b.sock"),
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
                socket: Endpoint::from_arg("/tmp/c.sock"),
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
                live: Some(Endpoint::from_arg("/tmp/w.sock")),
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
    fn arguments_that_are_not_unicode_are_refused() {
        let ok = unicode_args(["--headless".into(), "ü.skr".into()]);
        assert_eq!(ok.unwrap(), ["--headless", "ü.skr"]);
        #[cfg(unix)]
        let bad = <OsString as std::os::unix::ffi::OsStringExt>::from_vec(vec![b'a', 0xff]);
        #[cfg(windows)]
        let bad = <OsString as std::os::windows::ffi::OsStringExt>::from_wide(&[0x61, 0xd800]);
        let e = unicode_args(["--headless".into(), bad]).unwrap_err();
        assert!(e.contains("not valid Unicode"), "{e}");
    }

    #[test]
    fn help_flags_return_help() {
        assert_eq!(parse(args("-h")).unwrap(), Mode::Help);
        assert_eq!(parse(args("--headless --help")).unwrap(), Mode::Help);
    }

    #[cfg(unix)]
    #[test]
    fn default_socket_path_fits_the_os_limit() {
        let Endpoint::Socket(path) = default_endpoint();
        assert!(path.as_os_str().len() <= paths::MAX_SOCKET_PATH);
    }

    #[cfg(windows)]
    #[test]
    fn the_default_endpoint_is_a_pipe_per_process() {
        let name = format!(r"\\.\pipe\sketchup-alt-{}", std::process::id());
        assert_eq!(default_endpoint(), Endpoint::Pipe(name));
        assert_eq!(
            parse(args(r"--headless --socket main")).unwrap(),
            Mode::Headless {
                transport: Transport::Socket(Endpoint::Pipe(r"\\.\pipe\main".into())),
                file: None,
                api_roots: vec![]
            }
        );
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

    #[cfg(windows)]
    #[test]
    fn ensure_private_dir_creates_an_owner_only_folder() {
        let dir = scratch("create-win");
        let _ = std::fs::remove_dir_all(&dir);
        ensure_private_dir(&dir).unwrap();
        // Owned by the user, protected, one entry: full control for the user, inherited.
        let sddl = session::windows::security_text(&dir).unwrap();
        session::windows::check_owner_only(&sddl, Some("OICI")).unwrap();
        // An existing folder is accepted as it is.
        ensure_private_dir(&dir).unwrap();
        // Missing parents under an application data folder are made private too.
        let nested = dir.join("a").join("b");
        ensure_private_xdg_dir(crate::paths::Os::Windows, &nested).unwrap();
        for d in [dir.join("a"), nested] {
            let sddl = session::windows::security_text(&d).unwrap();
            session::windows::check_owner_only(&sddl, Some("OICI")).unwrap();
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn ensure_private_dir_refuses_a_junction() {
        let target = scratch("junction-target");
        let link = scratch("junction");
        let _ = std::fs::remove_dir_all(&target);
        let _ = std::fs::remove_dir(&link);
        ensure_private_dir(&target).unwrap();
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(made.success());
        let e = ensure_private_dir(&link).unwrap_err();
        assert!(e.to_string().contains("not a real directory"), "{e}");
        std::fs::remove_dir(&link).unwrap();
        std::fs::remove_dir_all(&target).unwrap();
    }

    #[cfg(unix)]
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

    #[cfg(unix)]
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

    #[cfg(unix)]
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
