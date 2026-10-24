//! `--api-root`: the folders the API may read and write files in.

use std::path::{Component, Path, PathBuf};

use session::fspath::{self, canonical, is_link, same};
use session::{Command, CommandError, ErrorKind, Query, TextureSource};

/// Folders every path the API reads or writes must lie within, resolved after symlinks.
/// Empty: no limit (the default).
///
/// On Windows the paths compared are canonical, so names are compared exactly (a folder may be
/// case-sensitive) and a request in another case matches once resolved; verbatim forms
/// (`\\?\C:\`, `\\?\UNC\`) are judged as their plain ones, junctions and other reparse points count as
/// links, and forms Windows reads specially are refused (see `fspath::windows_refusal`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApiRoots {
    roots: Vec<PathBuf>,
}

impl ApiRoots {
    /// `dirs` resolved after symlinks. Fails, naming it, for one that is not an existing folder.
    pub fn new(dirs: &[PathBuf]) -> Result<ApiRoots, String> {
        let mut roots: Vec<PathBuf> = Vec::new();
        for dir in dirs {
            if let Some(reason) = windows_refusal(dir.as_os_str().to_str()) {
                return Err(format!("--api-root {}: {reason}", dir.display()));
            }
            let real = canonical(dir).map_err(|e| format!("--api-root {}: {e}", dir.display()))?;
            if !real.is_dir() {
                return Err(format!("--api-root {}: not a folder", dir.display()));
            }
            if !roots.iter().any(|r| same(r, &real)) {
                roots.push(real);
            }
        }
        Ok(ApiRoots { roots })
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// The folders, resolved.
    pub fn dirs(&self) -> &[PathBuf] {
        &self.roots
    }

    fn inside(&self, real: &Path) -> bool {
        // A root itself is not a file within it (a write there would put files beside it).
        self.roots
            .iter()
            .any(|r| !same(real, r) && fspath::starts_with(real, r))
    }

    /// Refuses `real` (`path` as given) unless it lies within a root. A root itself is
    /// refused as a folder.
    fn check_inside(&self, path: &str, real: &Path) -> Result<(), CommandError> {
        if self.roots.iter().any(|r| same(r, real)) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("{path} is a folder, not a file"),
            ));
        }
        if !self.inside(real) {
            return Err(outside(path));
        }
        Ok(())
    }

    /// `path` as given, made absolute against the first root. A NUL byte, which no file name
    /// can hold, is refused, and on Windows the forms `fspath::windows_refusal` names.
    fn absolute(&self, path: &str) -> Result<PathBuf, CommandError> {
        if path.contains('\0') {
            return Err(nul(path));
        }
        if let Some(reason) = windows_refusal(Some(path)) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("{path} is {reason}, which the API does not take on Windows"),
            ));
        }
        // On Windows a verbatim path is taken as its plain form, as the roots are.
        let given = if cfg!(windows) {
            PathBuf::from(fspath::strip_verbatim(path).as_ref())
        } else {
            PathBuf::from(path)
        };
        Ok(match self.roots.first() {
            Some(first) if given.is_relative() => first.join(given),
            _ => given,
        })
    }

    /// `absolute` (`path` as given) resolved after symlinks, one part at a time. The walk
    /// may only pass through folders that hold a root and folders within a root; anything
    /// else (a part outside, missing or unreadable before a root is reached, or a dangling
    /// link) is the one refusal for outside, so an agent learns nothing of what lies outside
    /// the roots. A part missing within a root is a `file` error.
    fn resolve(&self, path: &str, absolute: &Path) -> Result<PathBuf, CommandError> {
        let within = |p: &Path| self.roots.iter().any(|r| fspath::starts_with(p, r));
        let holds_root = |p: &Path| self.roots.iter().any(|r| fspath::starts_with(r, p));
        let mut real = PathBuf::new();
        for c in absolute.components() {
            match c {
                // A drive or share alone (`C:`) is no folder yet: judged with its root.
                Component::Prefix(_) => {
                    real.push(c);
                    continue;
                }
                Component::RootDir => real.push(c),
                Component::CurDir => {}
                Component::ParentDir => {
                    real.pop();
                }
                Component::Normal(name) => {
                    let next = real.join(name);
                    match canonical(&next) {
                        Ok(r) => real = r,
                        Err(_) if !within(&real) => return Err(outside(path)),
                        Err(_) if std::fs::symlink_metadata(&next).is_ok_and(|m| is_link(&m)) => {
                            return Err(outside(path));
                        }
                        Err(e) => return Err(file_error(path, &e)),
                    }
                }
            }
            if !within(&real) && !holds_root(&real) {
                return Err(outside(path));
            }
        }
        Ok(real)
    }

    /// The file to read at `path`, resolved after symlinks, when it lies within a root.
    pub fn read(&self, path: &str) -> Result<PathBuf, CommandError> {
        let real = self.resolve(path, &self.absolute(path)?)?;
        self.check_inside(path, &real)?;
        Ok(real)
    }

    /// The folder at `path`, resolved after symlinks, when it is a root or lies within one.
    pub fn read_folder(&self, path: &str) -> Result<PathBuf, CommandError> {
        let real = self.resolve(path, &self.absolute(path)?)?;
        let is_root = self.roots.iter().any(|r| same(r, &real));
        if !is_root && !self.inside(&real) {
            return Err(outside(path));
        }
        if !real.is_dir() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("{path} is not a folder"),
            ));
        }
        Ok(real)
    }

    /// The file to write at `path`: its folder, resolved after symlinks, must lie within a
    /// root, and the file must not be a symbolic link.
    pub fn write(&self, path: &str) -> Result<PathBuf, CommandError> {
        // `sub/.` and `dir/..` normalize to a folder; a trailing separator or nothing names
        // none.
        let last = path.rsplit(std::path::is_separator).next().unwrap_or("");
        let absolute = self.absolute(path)?;
        let named = !matches!(last, "" | "." | "..");
        let Some(Component::Normal(name)) =
            named.then(|| absolute.components().next_back()).flatten()
        else {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("{path} names no file"),
            ));
        };
        // A path ending in a name has a folder (perhaps empty, which no root holds).
        let folder = absolute.parent().unwrap_or(Path::new(""));
        let real = self.resolve(path, folder)?.join(name);
        self.check_inside(path, &real)?;
        if std::fs::symlink_metadata(&real).is_ok_and(|m| is_link(&m)) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!(
                    "{path} is a symbolic link or, on Windows, another reparse point (a junction, a \
                     cloud file placeholder): under --api-root the API does not write to links"
                ),
            ));
        }
        Ok(real)
    }

    /// Points the folder of a `library` query at the folder it resolves to, or refuses it.
    pub fn apply_query(&self, query: &mut Query) -> Result<(), CommandError> {
        if let Query::Library { folder } = query
            && !self.is_empty()
        {
            *folder = text(self.read_folder(folder)?)?;
        }
        Ok(())
    }

    /// Points every path in `command` at the file it resolves to, or refuses the command.
    /// `open_file` is the model's file, which `save` without a path writes; inside a batch
    /// such a `save` is refused, since an earlier command may open another file.
    pub fn apply(
        &self,
        command: &mut Command,
        open_file: Option<&Path>,
        in_batch: bool,
    ) -> Result<(), CommandError> {
        if self.is_empty() {
            return Ok(());
        }
        match command {
            Command::Open { path, .. } => *path = text(self.read(path)?)?,
            Command::Import {
                path,
                library_roots,
                ..
            } => {
                *path = text(self.read(path)?)?;
                *library_roots = Some(self.roots.clone());
            }
            Command::Export {
                path, refuse_links, ..
            } => {
                *path = text(self.write(path)?)?;
                *refuse_links = true;
            }
            Command::Save { path, .. } => {
                let given = match (path.as_ref(), open_file) {
                    (Some(p), _) => p.clone(),
                    (None, _) if in_batch => {
                        return Err(CommandError::new(
                            ErrorKind::InvalidInput,
                            "give save a path: under --api-root a batch cannot save to the open file",
                        ));
                    }
                    (None, Some(open)) => {
                        // The file's own place: a relative open file is from the working
                        // directory, not the first root.
                        let open = std::path::absolute(open)
                            .map_err(|e| file_error(&open.display().to_string(), &e))?;
                        open.display().to_string()
                    }
                    // Nothing to check: the session asks for a path.
                    (None, None) => return Ok(()),
                };
                *path = Some(text(self.write(&given)?)?);
            }
            Command::SaveToLibrary { path, .. } => *path = text(self.write(path)?)?,
            Command::InsertLibraryItem { path, .. } => *path = text(self.read(path)?)?,
            Command::AddMaterial {
                texture: Some(TextureSource { path: Some(p), .. }),
                ..
            }
            | Command::ChangeMaterial {
                texture: Some(Some(TextureSource { path: Some(p), .. })),
                ..
            } => *p = text(self.read(p)?)?,
            _ => {}
        }
        Ok(())
    }
}

fn outside(path: &str) -> CommandError {
    CommandError::new(
        ErrorKind::InvalidInput,
        format!("{path} is outside the folders the API may use (--api-root)"),
    )
}

/// Why Windows would read `path` (as text; None when it is not Unicode) other than as a
/// plain file path, on Windows; never elsewhere.
fn windows_refusal(path: Option<&str>) -> Option<&'static str> {
    if cfg!(windows) {
        path.and_then(fspath::windows_refusal)
    } else {
        None
    }
}

fn nul(path: &str) -> CommandError {
    CommandError::new(
        ErrorKind::InvalidInput,
        format!(
            "{} contains a NUL byte, which no file name can hold",
            path.escape_debug()
        ),
    )
}

fn file_error(path: &str, e: &std::io::Error) -> CommandError {
    CommandError::new(ErrorKind::File, format!("{path}: {e}"))
}

/// A resolved path as command text; a path that is not UTF-8 cannot be passed on.
fn text(path: PathBuf) -> Result<String, CommandError> {
    path.into_os_string().into_string().map_err(|p| {
        CommandError::new(
            ErrorKind::InvalidInput,
            format!("{} is not a UTF-8 path", PathBuf::from(p).display()),
        )
    })
}

#[cfg(test)]
mod tests {
    use session::{ImportAs, MaterialId};

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roots-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        canonical(&dir).unwrap()
    }

    /// A junction at `link` to the folder `target` (needs no privilege, unlike a symlink).
    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[cfg(windows)]
    #[test]
    fn windows_forms_of_a_path_are_judged_alike() {
        let base = scratch("winforms");
        let root = base.join("Root");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.skr"), "").unwrap();
        let text = root.to_str().unwrap().to_string();
        // Given in other case or verbatim, the root is the folder as the system names it.
        for given in [
            text.to_uppercase(),
            text.to_lowercase(),
            format!(r"\\?\{text}"),
        ] {
            let roots = ApiRoots::new(&[PathBuf::from(&given)]).unwrap();
            assert_eq!(roots.dirs(), std::slice::from_ref(&root), "{given}");
        }
        let roots = ApiRoots::new(&[PathBuf::from(text.to_lowercase())]).unwrap();
        let want = root.join("a.skr");
        for given in [
            format!(r"{}\A.SKR", text.to_uppercase()),
            format!(r"\\?\{text}\a.skr"),
            format!(r"{text}\sub\..\a.skr"),
            format!("{}/a.skr", text.replace('\\', "/")),
            r"sub\..\a.skr".to_string(),
        ] {
            // Resolved part by part, so the case on disk is what is compared.
            assert_eq!(roots.read(&given).unwrap(), want, "{given}");
            // The file to write keeps its name as given; its folder is resolved.
            let written = roots.write(&given).unwrap();
            assert_eq!(written.parent(), Some(root.as_path()), "{given}");
            assert!(
                written.ends_with("a.skr") || written.ends_with("A.SKR"),
                "{given}"
            );
        }
        // The root's own name in another case, not yet resolved, names no file within it.
        let upper_name = base.join("ROOT");
        let e = roots.write(upper_name.to_str().unwrap()).unwrap_err();
        assert!(e.message.contains("outside"), "{}", e.message);
        // The root itself, in any form, is no file in it.
        for given in [text.to_uppercase(), format!(r"\\?\{text}")] {
            assert_eq!(
                roots.read(&given).unwrap_err().kind,
                ErrorKind::InvalidInput
            );
        }
        // Beside the root, whatever the form, stays outside.
        std::fs::write(base.join("secret"), "").unwrap();
        for given in [
            format!(r"{text}\..\secret"),
            format!(r"\\?\{text}\..\secret"),
            r"..\secret".to_string(),
            base.join("secret").to_str().unwrap().to_uppercase(),
            format!("{text}shop\\a.skr"),
        ] {
            let e = roots.read(&given).unwrap_err();
            assert!(e.message.contains("outside"), "{given}: {}", e.message);
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_forms_read_specially_are_refused() {
        let base = scratch("winrefused");
        let roots = ApiRoots::new(std::slice::from_ref(&base)).unwrap();
        for given in [
            r"C:foo",
            r"\foo",
            "a.skr:x",
            "nul.txt",
            r"sub\CON",
            "a. ",
            r"\\.\pipe\x",
        ] {
            let e = roots.write(given).unwrap_err();
            assert_eq!(e.kind, ErrorKind::InvalidInput, "{given}");
            assert!(e.message.contains("Windows"), "{given}: {}", e.message);
            assert!(roots.read(given).is_err(), "{given}");
        }
        assert!(ApiRoots::new(&[PathBuf::from(r"\foo")]).is_err());
        assert!(ApiRoots::new(&[PathBuf::from("C:")]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn junctions_count_as_links() {
        let base = scratch("junction");
        let (root, outside) = (base.join("root"), base.join("outside"));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("x"), "").unwrap();
        junction(&root.join("out"), &outside);
        junction(&root.join("in"), &root.join("sub"));
        let roots = ApiRoots::new(std::slice::from_ref(&root)).unwrap();
        // Through a junction to outside: refused; to inside: the folder it points at.
        assert!(
            roots
                .read(r"out\x")
                .unwrap_err()
                .message
                .contains("outside")
        );
        assert!(
            roots
                .write(r"out\a.skr")
                .unwrap_err()
                .message
                .contains("outside")
        );
        assert_eq!(
            roots.write(r"in\a.skr").unwrap(),
            root.join("sub").join("a.skr")
        );
        // A junction is never written to.
        let e = roots.write("in").unwrap_err();
        assert!(e.message.contains("link"), "{}", e.message);
        // A junction whose target is gone tells nothing of it.
        std::fs::remove_dir_all(&outside).unwrap();
        assert!(
            roots
                .read(r"out\x")
                .unwrap_err()
                .message
                .contains("outside")
        );
        // A root given through a junction is its target.
        let roots = ApiRoots::new(&[root.join("in")]).unwrap();
        assert_eq!(roots.dirs(), [root.join("sub")]);
    }

    #[cfg(unix)]
    #[test]
    fn a_root_given_through_a_link_resolves_to_its_target() {
        let base = scratch("rootlink");
        std::fs::create_dir(base.join("real")).unwrap();
        std::os::unix::fs::symlink(base.join("real"), base.join("link")).unwrap();
        let roots = ApiRoots::new(&[base.join("link")]).unwrap();
        assert_eq!(roots.dirs(), [base.join("real")]);
        assert_eq!(roots.write("a.skr").unwrap(), base.join("real/a.skr"));
    }

    #[test]
    fn a_write_that_names_no_file_is_refused() {
        let base = scratch("nofile");
        std::fs::create_dir(base.join("sub")).unwrap();
        let roots = ApiRoots::new(std::slice::from_ref(&base)).unwrap();
        for path in ["..", "sub/.", "", "sub/", ".", "sub/..", "/"] {
            let e = roots.write(path).unwrap_err();
            assert_eq!(e.kind, ErrorKind::InvalidInput, "{path:?}");
        }
        assert_eq!(roots.write("sub/a.skr").unwrap(), base.join("sub/a.skr"));
    }

    #[test]
    fn every_command_with_a_path_is_pointed_at_the_resolved_file() {
        let base = scratch("apply");
        std::fs::write(base.join("a.obj"), "").unwrap();
        std::fs::write(base.join("t.png"), "").unwrap();
        std::fs::write(base.join("m.skr"), "").unwrap();
        let roots = ApiRoots::new(std::slice::from_ref(&base)).unwrap();
        let texture = || TextureSource {
            path: Some("t.png".into()),
            width: None,
            height: None,
        };
        let mut commands = [
            Command::Open {
                path: "m.skr".into(),
                discard: false,
            },
            Command::Save {
                path: Some("s.skr".into()),
                overwrite: false,
            },
            Command::Export {
                path: "e.obj".into(),
                format: None,
                overwrite: false,
                refuse_links: false,
                scale: None,
            },
            Command::Import {
                path: "a.obj".into(),
                format: None,
                place: ImportAs::default(),
                scale: None,
                up: None,
                merge_coplanar: None,
                library_roots: None,
            },
            Command::AddMaterial {
                name: None,
                color: None,
                opacity: None,
                texture: Some(texture()),
                emission: None,
                metalness: None,
                roughness: None,
                density: None,
                existing: Default::default(),
            },
            Command::ChangeMaterial {
                material: MaterialId(1),
                name: None,
                color: None,
                opacity: None,
                texture: Some(Some(texture())),
                emission: None,
                metalness: None,
                roughness: None,
                density: None,
            },
        ];
        let expected = ["m.skr", "s.skr", "e.obj", "a.obj", "t.png", "t.png"];
        for (command, name) in commands.iter_mut().zip(expected) {
            roots.apply(command, None, false).unwrap();
            let want = base.join(name).display().to_string();
            let got = match &*command {
                Command::Open { path, .. } | Command::Export { path, .. } => path.clone(),
                Command::Save { path, .. } => path.clone().unwrap(),
                Command::Import {
                    path,
                    library_roots,
                    ..
                } => {
                    assert_eq!(library_roots.as_deref(), Some(roots.dirs()));
                    path.clone()
                }
                Command::AddMaterial {
                    texture: Some(t), ..
                }
                | Command::ChangeMaterial {
                    texture: Some(Some(t)),
                    ..
                } => t.path.clone().unwrap(),
                other => panic!("{other:?}"),
            };
            assert_eq!(got, want);
        }
        assert!(matches!(
            commands[2],
            Command::Export {
                refuse_links: true,
                ..
            }
        ));
        // Other commands pass untouched.
        let mut other = Command::Undo {
            count: None,
            expect: None,
        };
        roots.apply(&mut other, None, false).unwrap();
        assert_eq!(
            other,
            Command::Undo {
                count: None,
                expect: None
            }
        );
    }

    #[test]
    fn library_commands_and_queries_are_judged_by_the_roots() {
        let base = scratch("library");
        let (root, other) = (base.join("root"), base.join("other"));
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(root.join("lib/a.skr"), "").unwrap();
        std::fs::write(other.join("b.skr"), "").unwrap();
        let roots = ApiRoots::new(std::slice::from_ref(&root)).unwrap();
        let save = |path: &std::path::Path| Command::SaveToLibrary {
            path: path.display().to_string(),
            kind: session::LibraryKind::Model,
            item: None,
            name: None,
            overwrite: false,
        };
        let insert = |path: &std::path::Path| Command::InsertLibraryItem {
            path: path.display().to_string(),
            at: None,
        };
        for mut c in [save(&other.join("b.skr")), insert(&other.join("b.skr"))] {
            let e = roots.apply(&mut c, None, false).unwrap_err();
            assert_eq!(e.kind, ErrorKind::InvalidInput);
            assert!(e.message.contains("--api-root"), "{}", e.message);
        }
        let mut c = save(Path::new("lib/new.skr"));
        roots.apply(&mut c, None, false).unwrap();
        assert_eq!(c, save(&root.join("lib").join("new.skr")));
        let mut c = insert(Path::new("lib/a.skr"));
        roots.apply(&mut c, None, false).unwrap();
        assert_eq!(c, insert(&root.join("lib").join("a.skr")));

        let query = |folder: String| Query::Library { folder };
        // Folders within a root, the root itself and a relative name pass; others do not.
        for (given, want) in [
            ("lib", root.join("lib")),
            (root.to_str().unwrap(), root.clone()),
            (".", root.clone()),
        ] {
            let mut q = query(given.to_string());
            roots.apply_query(&mut q).unwrap();
            assert_eq!(q, query(want.display().to_string()), "{given}");
        }
        for given in [
            other.display().to_string(),
            "../other".to_string(),
            root.join("lib/a.skr").display().to_string(),
        ] {
            let mut q = query(given.clone());
            assert!(roots.apply_query(&mut q).is_err(), "{given}");
        }
        // Without roots nothing is checked.
        let mut q = query("anywhere".into());
        ApiRoots::default().apply_query(&mut q).unwrap();
        assert_eq!(q, query("anywhere".into()));
    }

    #[test]
    fn save_without_a_path_judges_the_open_file_where_it_lies() {
        let base = scratch("openfile");
        let (root, other) = (base.join("root"), base.join("other"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let roots = ApiRoots::new(std::slice::from_ref(&root)).unwrap();
        let save = || Command::Save {
            path: None,
            overwrite: false,
        };
        // A relative open file is from the working directory, not the first root.
        let cwd = std::env::current_dir().unwrap();
        let outside = std::path::absolute("house.skr").unwrap();
        assert!(!outside.starts_with(&root) && cwd != root);
        let mut c = save();
        let e = roots
            .apply(&mut c, Some(Path::new("house.skr")), false)
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
        assert_eq!(c, save());
        // An open file inside a root is saved where it is.
        let inside = root.join("m.skr");
        let mut c = save();
        roots.apply(&mut c, Some(&inside), false).unwrap();
        assert_eq!(
            c,
            Command::Save {
                path: Some(inside.display().to_string()),
                overwrite: false
            }
        );
        assert!(!root.join("house.skr").exists());
    }
}
