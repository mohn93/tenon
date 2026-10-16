//! `--api-root`: the folders the API may read and write files in.

use std::path::{Component, Path, PathBuf};

use session::{Command, CommandError, ErrorKind, TextureSource};

/// Folders every path the API reads or writes must lie within, resolved after symlinks.
/// Empty: no limit (the default).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApiRoots {
    roots: Vec<PathBuf>,
}

impl ApiRoots {
    /// `dirs` resolved after symlinks. Fails, naming it, for one that is not an existing folder.
    pub fn new(dirs: &[PathBuf]) -> Result<ApiRoots, String> {
        let mut roots: Vec<PathBuf> = Vec::new();
        for dir in dirs {
            let real = dir
                .canonicalize()
                .map_err(|e| format!("--api-root {}: {e}", dir.display()))?;
            if !real.is_dir() {
                return Err(format!("--api-root {}: not a folder", dir.display()));
            }
            if !roots.contains(&real) {
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
        self.roots.iter().any(|r| real.starts_with(r))
    }

    /// `path` as given, made absolute against the first root.
    fn absolute(&self, path: &str) -> PathBuf {
        match self.roots.first() {
            Some(first) if Path::new(path).is_relative() => first.join(path),
            _ => PathBuf::from(path),
        }
    }

    /// The file to read at `path`, resolved after symlinks, when it lies within a root.
    pub fn read(&self, path: &str) -> Result<PathBuf, CommandError> {
        let real = self
            .absolute(path)
            .canonicalize()
            .map_err(|e| file_error(path, &e))?;
        if !self.inside(&real) {
            return Err(outside(path));
        }
        Ok(real)
    }

    /// The file to write at `path`: its folder, resolved after symlinks, must lie within a
    /// root, and the file must not be a symbolic link.
    pub fn write(&self, path: &str) -> Result<PathBuf, CommandError> {
        // `sub/.` and `dir/..` normalize to a folder; a trailing slash or nothing names none.
        let last = path.rsplit('/').next().unwrap_or("");
        let absolute = self.absolute(path);
        let named = !matches!(last, "" | "." | "..");
        let Some(Component::Normal(name)) =
            named.then(|| absolute.components().next_back()).flatten()
        else {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("{path} names no file"),
            ));
        };
        let folder = absolute.parent().unwrap_or(Path::new("/"));
        let real = folder
            .canonicalize()
            .map_err(|e| file_error(path, &e))?
            .join(name);
        if !self.inside(&real) {
            return Err(outside(path));
        }
        if std::fs::symlink_metadata(&real).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!(
                    "{path} is a symbolic link: under --api-root the API does not write to links"
                ),
            ));
        }
        Ok(real)
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
            Command::Export { path, .. } => *path = text(self.write(path)?)?,
            Command::Save { path, .. } => {
                let given = match (path.as_ref(), open_file) {
                    (Some(p), _) => p.clone(),
                    (None, _) if in_batch => {
                        return Err(CommandError::new(
                            ErrorKind::InvalidInput,
                            "give save a path: under --api-root a batch cannot save to the open file",
                        ));
                    }
                    (None, Some(open)) => open.display().to_string(),
                    // Nothing to check: the session asks for a path.
                    (None, None) => return Ok(()),
                };
                *path = Some(text(self.write(&given)?)?);
            }
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
        dir.canonicalize().unwrap()
    }

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
        let mut commands = vec![
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
            },
            Command::ChangeMaterial {
                material: MaterialId(1),
                name: None,
                color: None,
                opacity: None,
                texture: Some(Some(texture())),
                emission: None,
            },
        ];
        for command in &mut commands {
            let before = command.clone();
            roots.apply(command, None, false).unwrap();
            assert_ne!(*command, before, "{before:?}");
        }
        // Other commands pass untouched.
        let mut other = Command::Undo { count: None };
        roots.apply(&mut other, None, false).unwrap();
        assert_eq!(other, Command::Undo { count: None });
    }
}
