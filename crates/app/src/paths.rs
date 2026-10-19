//! Where the app keeps its files, per platform.
//!
//! macOS (and anything else, until Windows is done) keeps everything under `~/.sketchup-alt`.
//! Linux follows the XDG Base Directory layout: preferences in the config folder, recovery
//! files in the state folder, and the default agent socket in the runtime folder.
//! `resolve` is pure apart from looking at the runtime folder, so tests pass a fake
//! environment for either OS.

use std::ffi::OsString;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::cli::FALLBACK_DIR;

/// The app's folder name under the XDG base folders.
const XDG_DIR: &str = "sketchup-alt";

/// Unix socket paths are limited to about 104 bytes on macOS (108 on Linux).
pub const MAX_SOCKET_PATH: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)] // `Os::MacOs` is the platform's name
pub enum Os {
    MacOs,
    Linux,
    Other,
}

impl Os {
    pub const fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else {
            Os::Other
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub prefs: PathBuf,
    pub recovery: PathBuf,
    /// The folder the default agent socket goes in.
    pub socket_dir: PathBuf,
    /// The earlier prefs location, read once when `prefs` does not exist yet (Linux only).
    pub legacy_prefs: Option<PathBuf>,
}

pub type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// The paths for `os`, or None when `HOME` is missing or not absolute (an empty or relative
/// one would put the files under the working folder).
pub fn resolve(os: Os, env: Env, pid: u32) -> Option<Paths> {
    resolve_as(os, env, pid, current_uid())
}

fn resolve_as(os: Os, env: Env, pid: u32, uid: Option<u32>) -> Option<Paths> {
    let home = home(env)?;
    let fallback = home.join(FALLBACK_DIR);
    let socket_dir = socket_dir_as(os, env, Some(&home), pid, uid);
    Some(match os {
        Os::Linux => {
            let config = xdg(env, "XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
            let state = xdg(env, "XDG_STATE_HOME").unwrap_or_else(|| home.join(".local/state"));
            Paths {
                prefs: config.join(XDG_DIR).join("prefs.json"),
                recovery: state.join(XDG_DIR).join("recovery"),
                socket_dir,
                legacy_prefs: Some(fallback.join("prefs.json")),
            }
        }
        Os::MacOs | Os::Other => Paths {
            prefs: fallback.join("prefs.json"),
            recovery: fallback.join("recovery"),
            socket_dir,
            legacy_prefs: None,
        },
    })
}

/// The default socket path for this process. Works without `HOME` (the socket then stays in
/// the temp folder, and listening reports any error).
pub fn default_socket(env: Env, pid: u32) -> PathBuf {
    socket_dir_as(Os::current(), env, home(env).as_deref(), pid, current_uid())
        .join(socket_name(pid))
}

fn socket_name(pid: u32) -> String {
    format!("sketchup-alt-{pid}.sock")
}

fn home(env: Env) -> Option<PathBuf> {
    env("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
}

/// An XDG variable's value: only when set to an absolute path.
fn xdg(env: Env, name: &str) -> Option<PathBuf> {
    env(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn socket_dir_as(os: Os, env: Env, home: Option<&Path>, pid: u32, uid: Option<u32>) -> PathBuf {
    let name = socket_name(pid);
    if os == Os::Linux
        && let Some(dir) = xdg(env, "XDG_RUNTIME_DIR")
        && dir.join(&name).as_os_str().len() <= MAX_SOCKET_PATH
        && uid.is_some_and(|uid| is_private_dir_of(&dir, uid))
    {
        return dir;
    }
    let temp = env("TMPDIR")
        .filter(|t| !t.is_empty())
        .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    choose_temp_or_fallback(&temp, home, &name)
}

/// The temp folder unless it is the shared `/tmp` (Linux without a usable runtime folder) or
/// too deep for a socket path; then `<home>/.sketchup-alt`. Without a home the temp folder is
/// kept and listening reports the error.
fn choose_temp_or_fallback(temp: &Path, home: Option<&Path>, name: &str) -> PathBuf {
    let usable = temp != Path::new("/tmp") && temp.join(name).as_os_str().len() <= MAX_SOCKET_PATH;
    match home {
        Some(home) if !usable => home.join(FALLBACK_DIR),
        _ => temp.to_path_buf(),
    }
}

/// A real folder owned by `uid` that group and others cannot write to.
fn is_private_dir_of(dir: &Path, uid: u32) -> bool {
    std::fs::metadata(dir).is_ok_and(|m| m.is_dir() && m.uid() == uid && m.mode() & 0o022 == 0)
}

/// The user this process runs as: the owner of `/proc/self` (Linux). None elsewhere, which
/// leaves the runtime folder unused.
fn current_uid() -> Option<u32> {
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::os::unix::fs::PermissionsExt;

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let map: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |k| map.get(k).cloned()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sketchup-alt-paths-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn my_uid(dir: &Path) -> u32 {
        std::fs::metadata(dir).unwrap().uid()
    }

    #[test]
    fn macos_paths_are_unchanged() {
        let e = env(&[
            ("HOME", "/Users/me"),
            ("TMPDIR", "/var/folders/ab/T"),
            ("XDG_CONFIG_HOME", "/ignored"),
            ("XDG_RUNTIME_DIR", "/ignored"),
        ]);
        let p = resolve_as(Os::MacOs, &e, 7, Some(0)).unwrap();
        assert_eq!(p.prefs, Path::new("/Users/me/.sketchup-alt/prefs.json"));
        assert_eq!(p.recovery, Path::new("/Users/me/.sketchup-alt/recovery"));
        assert_eq!(p.socket_dir, Path::new("/var/folders/ab/T"));
        assert_eq!(p.legacy_prefs, None);
        let long = format!("/{}", "x".repeat(100));
        let e = env(&[("HOME", "/Users/me"), ("TMPDIR", &long)]);
        let p = resolve_as(Os::MacOs, &e, 7, Some(0)).unwrap();
        assert_eq!(p.socket_dir, Path::new("/Users/me/.sketchup-alt"));
        let e = env(&[("HOME", "/Users/me")]);
        let p = resolve_as(Os::Other, &e, 7, Some(0)).unwrap();
        assert_eq!(
            p.socket_dir,
            Path::new("/Users/me/.sketchup-alt"),
            "/tmp is shared"
        );
    }

    #[test]
    fn linux_paths_follow_xdg() {
        let e = env(&[("HOME", "/home/me")]);
        let p = resolve_as(Os::Linux, &e, 7, None).unwrap();
        assert_eq!(
            p.prefs,
            Path::new("/home/me/.config/sketchup-alt/prefs.json")
        );
        assert_eq!(
            p.recovery,
            Path::new("/home/me/.local/state/sketchup-alt/recovery")
        );
        assert_eq!(
            p.legacy_prefs.as_deref(),
            Some(Path::new("/home/me/.sketchup-alt/prefs.json"))
        );
        let e = env(&[
            ("HOME", "/home/me"),
            ("XDG_CONFIG_HOME", "/cfg"),
            ("XDG_STATE_HOME", "/st"),
        ]);
        let p = resolve_as(Os::Linux, &e, 7, None).unwrap();
        assert_eq!(p.prefs, Path::new("/cfg/sketchup-alt/prefs.json"));
        assert_eq!(p.recovery, Path::new("/st/sketchup-alt/recovery"));
    }

    #[test]
    fn relative_or_empty_xdg_values_are_ignored() {
        let run = scratch("relative-run");
        std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700)).unwrap();
        for bad in ["", "cfg", "./cfg"] {
            let e = env(&[
                ("HOME", "/home/me"),
                ("XDG_CONFIG_HOME", bad),
                ("XDG_STATE_HOME", bad),
                ("XDG_RUNTIME_DIR", bad),
            ]);
            let p = resolve_as(Os::Linux, &e, 7, Some(my_uid(&run))).unwrap();
            assert_eq!(
                p.prefs,
                Path::new("/home/me/.config/sketchup-alt/prefs.json")
            );
            assert_eq!(
                p.recovery,
                Path::new("/home/me/.local/state/sketchup-alt/recovery")
            );
            assert_eq!(p.socket_dir, Path::new("/home/me/.sketchup-alt"));
        }
        for bad_home in ["", "home/me"] {
            let e = env(&[("HOME", bad_home)]);
            assert_eq!(resolve_as(Os::Linux, &e, 7, None), None);
        }
        assert_eq!(resolve_as(Os::Linux, &env(&[]), 7, None), None);
        let _ = std::fs::remove_dir_all(run);
    }

    #[test]
    fn runtime_dir_must_be_private() {
        let run = scratch("runtime");
        let uid = my_uid(&run);
        let run_str = run.to_str().unwrap().to_string();
        let e = env(&[("HOME", "/home/me"), ("XDG_RUNTIME_DIR", &run_str)]);
        let dir = |uid| resolve_as(Os::Linux, &e, 7, uid).unwrap().socket_dir;
        let fallback = PathBuf::from("/home/me/.sketchup-alt");
        std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(dir(Some(uid)), run, "private and ours");
        assert_eq!(dir(Some(uid.wrapping_add(1))), fallback, "someone else's");
        assert_eq!(dir(None), fallback, "owner unknown");
        for mode in [0o777, 0o770, 0o707, 0o720] {
            std::fs::set_permissions(&run, std::fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(dir(Some(uid)), fallback, "mode {mode:o}");
        }
        std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            dir(Some(uid)),
            run,
            "readable by others is fine, writable is not"
        );
        // A file, a missing folder, a path too long for a socket.
        let file = run.join("file");
        std::fs::write(&file, b"").unwrap();
        let file_str = file.to_str().unwrap().to_string();
        let e = env(&[("HOME", "/home/me"), ("XDG_RUNTIME_DIR", &file_str)]);
        assert_eq!(
            resolve_as(Os::Linux, &e, 7, Some(uid)).unwrap().socket_dir,
            fallback
        );
        let e = env(&[
            ("HOME", "/home/me"),
            ("XDG_RUNTIME_DIR", "/nonexistent/run"),
        ]);
        assert_eq!(
            resolve_as(Os::Linux, &e, 7, Some(uid)).unwrap().socket_dir,
            fallback
        );
        let deep = run.join("d".repeat(100));
        std::fs::create_dir(&deep).unwrap();
        let deep_str = deep.to_str().unwrap().to_string();
        let e = env(&[("HOME", "/home/me"), ("XDG_RUNTIME_DIR", &deep_str)]);
        assert_eq!(
            resolve_as(Os::Linux, &e, 7, Some(uid)).unwrap().socket_dir,
            fallback
        );
        let _ = std::fs::remove_dir_all(run);
    }

    #[test]
    fn runtime_dir_is_ignored_off_linux() {
        let run = scratch("runtime-macos");
        std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700)).unwrap();
        let run_str = run.to_str().unwrap().to_string();
        let e = env(&[("HOME", "/Users/me"), ("XDG_RUNTIME_DIR", &run_str)]);
        let p = resolve_as(Os::MacOs, &e, 7, Some(my_uid(&run))).unwrap();
        assert_ne!(p.socket_dir, run);
        let _ = std::fs::remove_dir_all(run);
    }

    #[test]
    fn default_socket_without_home_stays_in_temp() {
        let e = env(&[("TMPDIR", "/var/t")]);
        assert_eq!(
            socket_dir_as(Os::MacOs, &e, None, 7, None),
            Path::new("/var/t")
        );
        assert_eq!(
            socket_dir_as(Os::Linux, &env(&[]), None, 7, None),
            Path::new("/tmp")
        );
    }
}
