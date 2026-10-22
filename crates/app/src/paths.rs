//! Where the app keeps its files, per platform.
//!
//! macOS (and any other Unix) keeps everything under `~/.sketchup-alt`. Linux follows the XDG
//! Base Directory layout: preferences in the config folder, recovery files in the state folder,
//! and the default agent socket in the runtime folder. Windows keeps preferences in the roaming
//! application data folder and recovery files in the local one (its agent API uses a named
//! pipe, so no socket folder). `resolve` is pure apart from looking at the runtime folder, so
//! tests pass a fake environment for any OS.

use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Base folder under HOME for the files that live there on macOS, and for the socket when the
/// preferred folder is unusable.
pub const FALLBACK_DIR: &str = ".sketchup-alt";

/// The app's folder name under the XDG base folders and the Windows application data folders.
const XDG_DIR: &str = "sketchup-alt";

/// Unix socket paths are limited to about 104 bytes on macOS (108 on Linux).
pub const MAX_SOCKET_PATH: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)] // `Os::MacOs` is the platform's name
pub enum Os {
    MacOs,
    Linux,
    Windows,
    Other,
}

impl Os {
    pub const fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Other
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub prefs: PathBuf,
    pub recovery: PathBuf,
    /// The folder the default agent socket goes in (on Windows, which uses a named pipe, the
    /// temp folder, unused).
    pub socket_dir: PathBuf,
    /// The earlier prefs location, read once when `prefs` does not exist yet (Linux only).
    pub legacy_prefs: Option<PathBuf>,
}

pub type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// The paths for `os`, or None when `HOME` is missing or not absolute (an empty or relative
/// one would put the files under the working folder); on Windows, when neither the folder
/// variables nor `USERPROFILE` give an absolute path (see `windows_paths`).
pub fn resolve(os: Os, env: Env, temp: &Path, pid: u32) -> Option<Paths> {
    resolve_as(os, env, temp, pid, current_uid())
}

fn resolve_as(os: Os, env: Env, temp: &Path, pid: u32, uid: Option<u32>) -> Option<Paths> {
    if os == Os::Windows {
        return windows_paths(env, temp);
    }
    let home = home(env)?;
    let fallback = home.join(FALLBACK_DIR);
    let socket_dir = socket_dir_as(os, env, temp, Some(&home), pid, uid);
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
        // Windows was resolved above, without HOME.
        Os::MacOs | Os::Windows | Os::Other => Paths {
            prefs: fallback.join("prefs.json"),
            recovery: fallback.join("recovery"),
            socket_dir,
            legacy_prefs: None,
        },
    })
}

/// Windows: preferences in `%APPDATA%\sketchup-alt`, recovery files in
/// `%LOCALAPPDATA%\sketchup-alt\recovery`. A variable counts only when set to an absolute
/// path; a missing one falls back to `%USERPROFILE%\AppData\Roaming` or `\Local`. None when
/// either folder is still unknown.
fn windows_paths(env: Env, temp: &Path) -> Option<Paths> {
    let profile = env("USERPROFILE").and_then(windows_absolute);
    let folder = |var: &str, under: &str| {
        env(var)
            .and_then(windows_absolute)
            .or_else(|| Some(windows_join(profile.as_ref()?, &["AppData", under])))
    };
    let roaming = folder("APPDATA", "Roaming")?;
    let local = folder("LOCALAPPDATA", "Local")?;
    Some(Paths {
        prefs: windows_join(&roaming, &[XDG_DIR, "prefs.json"]),
        recovery: windows_join(&local, &[XDG_DIR, "recovery"]),
        socket_dir: temp.to_path_buf(),
        legacy_prefs: None,
    })
}

/// `value` as a path when Windows reads it as absolute (a drive and a separator, or a share),
/// whatever OS this runs on. A value that is not Unicode is not used.
fn windows_absolute(value: OsString) -> Option<PathBuf> {
    let text = value.to_str()?;
    session::fspath::windows_is_absolute(text).then(|| PathBuf::from(text))
}

/// `base` with `names` added, each after a backslash: `PathBuf::join` on Windows, on any OS.
fn windows_join(base: &Path, names: &[&str]) -> PathBuf {
    let mut path = base.as_os_str().to_owned();
    for name in names {
        if !matches!(path.as_encoded_bytes().last(), Some(b'\\' | b'/')) {
            path.push("\\");
        }
        path.push(name);
    }
    PathBuf::from(path)
}

/// The default socket path for this process. Works without `HOME` (the socket then stays in
/// the temp folder, and listening reports any error).
#[cfg(unix)]
pub fn default_socket(env: Env, temp: &Path, pid: u32) -> PathBuf {
    let home = home(env);
    socket_dir_as(
        Os::current(),
        env,
        temp,
        home.as_deref(),
        pid,
        current_uid(),
    )
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

fn socket_dir_as(
    os: Os,
    env: Env,
    temp: &Path,
    home: Option<&Path>,
    pid: u32,
    uid: Option<u32>,
) -> PathBuf {
    let name = socket_name(pid);
    if os == Os::Linux
        && let Some(dir) = xdg(env, "XDG_RUNTIME_DIR")
        && dir.join(&name).as_os_str().len() <= MAX_SOCKET_PATH
        && uid.is_some_and(|uid| is_private_dir_of(&dir, uid))
    {
        return dir;
    }
    choose_temp_or_fallback(temp, home, &name)
}

/// The temp folder (`std::env::temp_dir()`: `$TMPDIR`, else `/tmp`) unless it is the shared `/tmp` (Linux without a usable runtime folder) or
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
#[cfg(unix)]
fn is_private_dir_of(dir: &Path, uid: u32) -> bool {
    // `symlink_metadata`: a symlink is not a real folder, even when it points at one.
    std::fs::symlink_metadata(dir)
        .is_ok_and(|m| m.is_dir() && m.uid() == uid && m.mode() & 0o022 == 0)
}

/// The user this process runs as: the owner of `/proc/self` (Linux). None elsewhere, which
/// leaves the runtime folder unused. A non-dumpable process (setuid, or `prctl` off) sees
/// `/proc/self` owned by root, so the runtime folder is then not used: a safe fallback.
#[cfg(unix)]
fn current_uid() -> Option<u32> {
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

/// No runtime folder off Unix.
#[cfg(not(unix))]
fn is_private_dir_of(_: &Path, _: u32) -> bool {
    false
}

#[cfg(not(unix))]
fn current_uid() -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    /// A fake absolute path: on Windows a rooted path needs a drive to count as absolute.
    fn abs(path: &str) -> String {
        format!("{}{path}", if cfg!(windows) { "C:" } else { "" })
    }

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let map: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |k| map.get(k).cloned()
    }

    /// A fresh 0700 folder under a short base, so a socket path in it fits the limit.
    #[cfg(unix)]
    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(format!("/tmp/sa-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    #[cfg(unix)]
    fn my_uid(dir: &Path) -> u32 {
        std::fs::metadata(dir).unwrap().uid()
    }

    #[test]
    fn macos_paths_are_unchanged() {
        let tmp_dir = abs("/var/folders/ab/T");
        let tmp = Path::new(&tmp_dir);
        let e = env(&[
            ("HOME", abs("/Users/me").as_str()),
            ("XDG_CONFIG_HOME", abs("/ignored").as_str()),
            ("XDG_RUNTIME_DIR", abs("/ignored").as_str()),
        ]);
        let p = resolve_as(Os::MacOs, &e, tmp, 7, Some(0)).unwrap();
        assert_eq!(
            p.prefs,
            Path::new(abs("/Users/me/.sketchup-alt/prefs.json").as_str())
        );
        assert_eq!(
            p.recovery,
            Path::new(abs("/Users/me/.sketchup-alt/recovery").as_str())
        );
        assert_eq!(p.socket_dir, tmp, "the per-user temp folder");
        assert_eq!(p.legacy_prefs, None);
        let long = PathBuf::from(abs(&format!("/{}", "x".repeat(100))));
        let p = resolve_as(Os::MacOs, &e, &long, 7, Some(0)).unwrap();
        assert_eq!(
            p.socket_dir,
            Path::new(abs("/Users/me/.sketchup-alt").as_str())
        );
        // The shared-/tmp rule compares the literal path, so pass it as is on every host.
        let p = resolve_as(Os::Other, &e, Path::new("/tmp"), 7, Some(0)).unwrap();
        assert_eq!(
            p.socket_dir,
            Path::new(abs("/Users/me/.sketchup-alt").as_str()),
            "/tmp is shared"
        );
    }

    #[test]
    fn windows_folder_values_that_are_not_unicode_are_not_used() {
        #[cfg(unix)]
        let bad = <OsString as std::os::unix::ffi::OsStringExt>::from_vec(b"C:\\x\xff".to_vec());
        #[cfg(windows)]
        let bad = <OsString as std::os::windows::ffi::OsStringExt>::from_wide(&[
            0x43, 0x3a, 0x5c, 0xd800,
        ]);
        assert_eq!(windows_absolute(bad), None);
        assert_eq!(
            windows_absolute(OsString::from(r"C:\x")),
            Some(PathBuf::from(r"C:\x"))
        );
    }

    #[test]
    fn windows_paths_use_appdata() {
        let at =
            |vars: &[(&str, &str)]| resolve_as(Os::Windows, &env(vars), Path::new("/t"), 7, None);
        let p = at(&[
            ("APPDATA", r"C:\Users\me\AppData\Roaming"),
            ("LOCALAPPDATA", r"D:\local"),
            ("USERPROFILE", r"C:\Users\me"),
            ("HOME", "/ignored"),
            ("XDG_CONFIG_HOME", "/ignored"),
        ])
        .unwrap();
        assert_eq!(
            p.prefs.as_os_str(),
            r"C:\Users\me\AppData\Roaming\sketchup-alt\prefs.json"
        );
        assert_eq!(p.recovery.as_os_str(), r"D:\local\sketchup-alt\recovery");
        assert_eq!(p.legacy_prefs, None);
        // Relative, drive-relative, rooted without a drive or empty: ignored for USERPROFILE.
        for bad in ["", "AppData", r"C:AppData", r"\AppData", r"\\.\C:\AppData"] {
            let p = at(&[
                ("APPDATA", bad),
                ("LOCALAPPDATA", bad),
                ("USERPROFILE", r"C:\Users\me"),
            ])
            .unwrap();
            assert_eq!(
                p.prefs.as_os_str(),
                r"C:\Users\me\AppData\Roaming\sketchup-alt\prefs.json",
                "{bad:?}"
            );
            assert_eq!(
                p.recovery.as_os_str(),
                r"C:\Users\me\AppData\Local\sketchup-alt\recovery"
            );
        }
        // A share counts; a profile ending in a separator gets no second one.
        let p = at(&[("USERPROFILE", r"\\server\home\me\")]).unwrap();
        assert_eq!(
            p.prefs.as_os_str(),
            r"\\server\home\me\AppData\Roaming\sketchup-alt\prefs.json"
        );
        // No usable profile: no paths, unless both folders are given.
        assert_eq!(at(&[]), None);
        assert_eq!(at(&[("USERPROFILE", "me"), ("HOME", "/home/me")]), None);
        assert_eq!(at(&[("APPDATA", r"C:\r")]), None);
        assert!(at(&[("APPDATA", r"C:\r"), ("LOCALAPPDATA", r"C:\l")]).is_some());
    }

    #[test]
    fn linux_paths_follow_xdg() {
        let e = env(&[("HOME", abs("/home/me").as_str())]);
        let p = resolve_as(Os::Linux, &e, Path::new(abs("/tmp").as_str()), 7, None).unwrap();
        assert_eq!(
            p.prefs,
            Path::new(abs("/home/me/.config/sketchup-alt/prefs.json").as_str())
        );
        assert_eq!(
            p.recovery,
            Path::new(abs("/home/me/.local/state/sketchup-alt/recovery").as_str())
        );
        assert_eq!(
            p.legacy_prefs.as_deref(),
            Some(Path::new(abs("/home/me/.sketchup-alt/prefs.json").as_str()))
        );
        let e = env(&[
            ("HOME", abs("/home/me").as_str()),
            ("XDG_CONFIG_HOME", abs("/cfg").as_str()),
            ("XDG_STATE_HOME", abs("/st").as_str()),
        ]);
        let p = resolve_as(Os::Linux, &e, Path::new(abs("/tmp").as_str()), 7, None).unwrap();
        assert_eq!(
            p.prefs,
            Path::new(abs("/cfg/sketchup-alt/prefs.json").as_str())
        );
        assert_eq!(
            p.recovery,
            Path::new(abs("/st/sketchup-alt/recovery").as_str())
        );
    }

    #[cfg(unix)]
    #[test]
    fn relative_or_empty_xdg_values_are_ignored() {
        let run = scratch("relative-run");
        for bad in ["", "cfg", "./cfg"] {
            let e = env(&[
                ("HOME", "/home/me"),
                ("XDG_CONFIG_HOME", bad),
                ("XDG_STATE_HOME", bad),
                ("XDG_RUNTIME_DIR", bad),
            ]);
            let p = resolve_as(Os::Linux, &e, Path::new("/tmp"), 7, Some(my_uid(&run))).unwrap();
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
            assert_eq!(resolve_as(Os::Linux, &e, Path::new("/tmp"), 7, None), None);
        }
        assert_eq!(
            resolve_as(Os::Linux, &env(&[]), Path::new("/tmp"), 7, None),
            None
        );
        let _ = std::fs::remove_dir_all(run);
    }

    #[cfg(unix)]
    #[test]
    fn runtime_dir_must_be_private() {
        let run = scratch("runtime");
        let uid = my_uid(&run);
        let run_str = run.to_str().unwrap().to_string();
        let e = env(&[("HOME", "/home/me"), ("XDG_RUNTIME_DIR", &run_str)]);
        let dir = |uid| {
            resolve_as(Os::Linux, &e, Path::new("/tmp"), 7, uid)
                .unwrap()
                .socket_dir
        };
        let fallback = PathBuf::from("/home/me/.sketchup-alt");
        assert!(run.join("sketchup-alt-7.sock").as_os_str().len() <= MAX_SOCKET_PATH);
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
        // A symlink to a good folder is not a real folder.
        let link = run.join("link");
        let target = scratch("runtime-target");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let link_str = link.to_str().unwrap().to_string();
        let e = env(&[("HOME", "/home/me"), ("XDG_RUNTIME_DIR", &link_str)]);
        assert_eq!(
            resolve_as(Os::Linux, &e, Path::new("/tmp"), 7, Some(uid))
                .unwrap()
                .socket_dir,
            fallback
        );
        let _ = std::fs::remove_dir_all(target);
        // A file, a missing folder, a path too long for a socket.
        let file = run.join("file");
        std::fs::write(&file, b"").unwrap();
        let file_str = file.to_str().unwrap().to_string();
        let e = env(&[("HOME", "/home/me"), ("XDG_RUNTIME_DIR", &file_str)]);
        assert_eq!(
            resolve_as(Os::Linux, &e, Path::new("/tmp"), 7, Some(uid))
                .unwrap()
                .socket_dir,
            fallback
        );
        let e = env(&[
            ("HOME", "/home/me"),
            ("XDG_RUNTIME_DIR", "/nonexistent/run"),
        ]);
        assert_eq!(
            resolve_as(Os::Linux, &e, Path::new("/tmp"), 7, Some(uid))
                .unwrap()
                .socket_dir,
            fallback
        );
        let deep = run.join("d".repeat(100));
        std::fs::create_dir(&deep).unwrap();
        let deep_str = deep.to_str().unwrap().to_string();
        let e = env(&[("HOME", "/home/me"), ("XDG_RUNTIME_DIR", &deep_str)]);
        assert_eq!(
            resolve_as(Os::Linux, &e, Path::new("/tmp"), 7, Some(uid))
                .unwrap()
                .socket_dir,
            fallback
        );
        let _ = std::fs::remove_dir_all(run);
    }

    #[cfg(unix)]
    #[test]
    fn runtime_dir_is_ignored_off_linux() {
        let run = scratch("runtime-macos");
        let run_str = run.to_str().unwrap().to_string();
        let e = env(&[("HOME", "/Users/me"), ("XDG_RUNTIME_DIR", &run_str)]);
        let p = resolve_as(Os::MacOs, &e, Path::new("/var/t"), 7, Some(my_uid(&run))).unwrap();
        assert_ne!(p.socket_dir, run);
        let _ = std::fs::remove_dir_all(run);
    }

    #[test]
    fn default_socket_without_home_stays_in_temp() {
        let e = env(&[]);
        let at = |os, temp: &str| socket_dir_as(os, &e, Path::new(temp), None, 7, None);
        assert_eq!(at(Os::MacOs, "/var/t"), Path::new("/var/t"));
        assert_eq!(at(Os::Linux, "/tmp"), Path::new("/tmp"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn current_uid_is_the_owner_of_a_new_file() {
        let dir = scratch("uid");
        let file = dir.join("f");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(current_uid(), Some(my_uid(&file)));
        let _ = std::fs::remove_dir_all(dir);
    }
}
