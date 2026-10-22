//! What the window does for the component library besides the panel itself: file names for
//! saved items, deleting an item's file, and the thumbnails (which tiles asked for, at most a
//! few made per frame, kept by file and modification time).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use glam::DVec2;
use kernel::MeshId;
use render::Snapshotter;
use session::{Camera, LibraryKind, Lighting, StandardView, library};

/// A thumbnail's side, in pixels.
pub const THUMB_SIZE: u32 = 128;

/// The most thumbnails made in one frame.
pub const THUMBS_PER_FRAME: usize = 2;

/// The longest file name stem a saved item gets, in characters.
const MAX_STEM: usize = 100;

/// The file name (with `.skr`) an item named `name` is saved under: characters a file name may
/// not hold become `_`, surrounding spaces and dots go, a name Windows reserves gets a `_`
/// after it. None when nothing is left.
pub fn file_name_for(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let stem: String = cleaned
        .trim_matches(|c: char| c.is_whitespace() || c == '.')
        .chars()
        .take(MAX_STEM)
        .collect();
    let stem = stem.trim_end_matches(|c: char| c.is_whitespace() || c == '.');
    if stem.is_empty() {
        return None;
    }
    let upper = stem.to_uppercase();
    let base = upper.split('.').next().unwrap_or("");
    let reserved = matches!(base, "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|p| {
            base.strip_prefix(p)
                .is_some_and(|n| matches!(n.as_bytes(), [b'1'..=b'9']))
        });
    Some(if reserved {
        format!("{stem}_.skr")
    } else {
        format!("{stem}.skr")
    })
}

/// Deletes the library item at `path`, only when it is a regular `.skr` file directly in
/// `folder` and not a link (what a listing shows, and nothing it follows elsewhere).
pub fn delete_item(folder: &Path, path: &Path) -> Result<(), String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| n.to_lowercase().ends_with(".skr"))
        .ok_or("it is not a library file")?;
    let parent = path.parent().ok_or("it is not in the library folder")?;
    let inside = match (std::fs::canonicalize(parent), std::fs::canonicalize(folder)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !inside {
        return Err("it is not in the library folder".into());
    }
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{name}: {e}"))?;
    if !meta.file_type().is_file() {
        return Err(format!("{name} is not a regular file"));
    }
    std::fs::remove_file(path).map_err(|e| format!("{name}: {e}"))
}

/// A thumbnail asked for: a file and when it was last written.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThumbKey {
    pub path: PathBuf,
    pub modified: u64,
}

/// What is known of a thumbnail.
#[derive(Debug, PartialEq)]
pub enum Slot<T> {
    Ready(T),
    Failed,
}

/// The thumbnails made so far, by file and modification time, and the ones asked for. Which
/// to make next is decided here (at most `THUMBS_PER_FRAME` a frame, in the order asked); how
/// to make one is the caller's.
pub struct Thumbnails<T> {
    slots: HashMap<PathBuf, (u64, Slot<T>)>,
    wanted: Vec<ThumbKey>,
}

impl<T> Default for Thumbnails<T> {
    fn default() -> Self {
        Self {
            slots: HashMap::new(),
            wanted: Vec::new(),
        }
    }
}

impl<T> Thumbnails<T> {
    /// What is held for `path` as of `modified`; None when not made yet (or made for another
    /// version of the file).
    pub fn get(&self, path: &Path, modified: u64) -> Option<&Slot<T>> {
        self.slots
            .get(path)
            .filter(|(m, _)| *m == modified)
            .map(|(_, slot)| slot)
    }

    /// Asks for a thumbnail, unless it is held or already asked for.
    pub fn want(&mut self, key: ThumbKey) {
        if self.get(&key.path, key.modified).is_none() && !self.wanted.contains(&key) {
            self.wanted.push(key);
        }
    }

    /// The next thumbnails to make, at most `THUMBS_PER_FRAME`, no longer asked for.
    pub fn next_batch(&mut self) -> Vec<ThumbKey> {
        let n = self.wanted.len().min(THUMBS_PER_FRAME);
        self.wanted.drain(..n).collect()
    }

    /// Whether thumbnails are still asked for.
    #[cfg(test)]
    pub fn pending(&self) -> bool {
        !self.wanted.is_empty()
    }

    /// Holds the thumbnail made for `key`, or that it could not be made.
    pub fn store(&mut self, key: ThumbKey, made: Option<T>) {
        let slot = made.map_or(Slot::Failed, Slot::Ready);
        self.slots.insert(key.path, (key.modified, slot));
    }

    /// Drops what is held and asked for, except for these files (the listing now).
    pub fn keep_only(&mut self, listed: &[(PathBuf, u64)]) {
        let live: BTreeSet<&PathBuf> = listed.iter().map(|(p, _)| p).collect();
        self.slots.retain(|p, _| live.contains(p));
        self.wanted.retain(|k| live.contains(&k.path));
    }

    /// Drops everything.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.wanted.clear();
    }
}

/// A thumbnail's pixels: `THUMB_SIZE` square RGBA, top row first.
pub struct Pixels(pub Vec<u8>);

/// Makes the thumbnail of the item at `path`: a component or model in an iso view fitted to
/// its box on white paper, a material as its color or texture. None when the item cannot be
/// read, is empty or the GPU fails.
pub fn make(snapshotter: &mut Snapshotter, path: &Path) -> Option<Pixels> {
    let item = library::read_item(path).ok()?;
    let side = THUMB_SIZE as usize;
    if item.kind == LibraryKind::Material {
        let (_, material) = item.scene.materials().next()?;
        let mut pixels = match &material.texture {
            Some(t) => render::texture::resample(
                &t.image.pixels,
                t.image.width,
                t.image.height,
                THUMB_SIZE,
            ),
            None => material
                .color
                .iter()
                .copied()
                .chain([255])
                .cycle()
                .take(side * side * 4)
                .collect(),
        };
        pixels.iter_mut().skip(3).step_by(4).for_each(|a| *a = 255);
        return Some(Pixels(pixels));
    }
    let [lo, hi] = item.scene.bounds(MeshId::Root)?;
    let mut camera = Camera::new(DVec2::splat(f64::from(THUMB_SIZE)));
    camera.look_from(StandardView::Iso);
    camera.fit(lo, hi);
    snapshotter.set_print_mode(true);
    snapshotter.set_color_by_tag(false);
    snapshotter.set_section_display(Default::default());
    let pixels = snapshotter.capture(
        &item.scene,
        &[],
        &Default::default(),
        &camera,
        Lighting::Day,
        THUMB_SIZE,
        THUMB_SIZE,
    );
    snapshotter.set_print_mode(false);
    pixels.map(Pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str, modified: u64) -> ThumbKey {
        ThumbKey {
            path: PathBuf::from(format!("/lib/{name}.skr")),
            modified,
        }
    }

    #[test]
    fn at_most_two_thumbnails_are_made_a_frame_in_the_order_asked() {
        let mut t: Thumbnails<u32> = Thumbnails::default();
        for n in ["a", "b", "c", "d", "e"] {
            t.want(key(n, 1));
        }
        t.want(key("a", 1));
        assert_eq!(t.next_batch(), [key("a", 1), key("b", 1)]);
        assert!(t.pending());
        assert_eq!(t.next_batch(), [key("c", 1), key("d", 1)]);
        assert_eq!(t.next_batch(), [key("e", 1)]);
        assert!(!t.pending());
        assert_eq!(t.next_batch(), []);
    }

    #[test]
    fn a_thumbnail_is_kept_by_file_and_modification_time() {
        let mut t = Thumbnails::default();
        t.store(key("a", 1), Some(7u32));
        t.store(key("b", 1), None);
        assert_eq!(t.get(Path::new("/lib/a.skr"), 1), Some(&Slot::Ready(7)));
        assert_eq!(t.get(Path::new("/lib/b.skr"), 1), Some(&Slot::Failed));
        // Held ones are not asked for again; a file written since is.
        t.want(key("a", 1));
        t.want(key("b", 1));
        assert!(!t.pending());
        assert_eq!(t.get(Path::new("/lib/a.skr"), 2), None);
        t.want(key("a", 2));
        assert_eq!(t.next_batch(), [key("a", 2)]);
        // The newer one replaces the old.
        t.store(key("a", 2), Some(8));
        assert_eq!(t.get(Path::new("/lib/a.skr"), 1), None);
        assert_eq!(t.get(Path::new("/lib/a.skr"), 2), Some(&Slot::Ready(8)));
    }

    #[test]
    fn items_gone_from_the_listing_are_forgotten() {
        let mut t = Thumbnails::default();
        t.store(key("a", 1), Some(1u32));
        t.store(key("b", 1), Some(2));
        t.want(key("c", 1));
        t.want(key("d", 1));
        t.keep_only(&[
            (PathBuf::from("/lib/a.skr"), 1),
            (PathBuf::from("/lib/d.skr"), 1),
        ]);
        assert!(t.get(Path::new("/lib/a.skr"), 1).is_some());
        assert!(t.get(Path::new("/lib/b.skr"), 1).is_none());
        assert_eq!(t.next_batch(), [key("d", 1)]);
        t.clear();
        assert!(t.get(Path::new("/lib/a.skr"), 1).is_none());
    }

    #[test]
    fn names_become_safe_file_names() {
        assert_eq!(file_name_for("Chair").as_deref(), Some("Chair.skr"));
        assert_eq!(file_name_for("  a/b:c*d  ").as_deref(), Some("a_b_c_d.skr"));
        assert_eq!(file_name_for("..sneaky..").as_deref(), Some("sneaky.skr"));
        assert_eq!(file_name_for("../../x").as_deref(), Some("_.._x.skr"));
        assert_eq!(file_name_for("con").as_deref(), Some("con_.skr"));
        assert_eq!(file_name_for("LPT3").as_deref(), Some("LPT3_.skr"));
        assert_eq!(file_name_for("COM10").as_deref(), Some("COM10.skr"));
        assert_eq!(file_name_for("  . "), None);
        assert_eq!(file_name_for(""), None);
        let long = "x".repeat(300);
        assert_eq!(file_name_for(&long).unwrap().len(), MAX_STEM + 4);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sketchup-alt-library-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn delete_removes_only_a_regular_skr_file_in_the_folder() {
        let dir = scratch("delete");
        let folder = dir.join("lib");
        let other = dir.join("other");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let item = folder.join("a.skr");
        std::fs::write(&item, b"x").unwrap();
        let outside = other.join("b.skr");
        std::fs::write(&outside, b"x").unwrap();
        let text = folder.join("note.txt");
        std::fs::write(&text, b"x").unwrap();
        // Not in the folder, not an item, not even there.
        assert!(delete_item(&folder, &outside).is_err());
        assert!(delete_item(&folder, &text).is_err());
        assert!(delete_item(&folder, &folder.join("gone.skr")).is_err());
        assert!(delete_item(&folder, &folder.join("..").join("other").join("b.skr")).is_err());
        assert!(outside.exists() && text.exists() && item.exists());
        // A folder named like an item.
        let dir_item = folder.join("d.skr");
        std::fs::create_dir_all(&dir_item).unwrap();
        assert!(delete_item(&folder, &dir_item).is_err());
        assert!(dir_item.exists());
        #[cfg(unix)]
        {
            let link = folder.join("link.skr");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(delete_item(&folder, &link).is_err());
            assert!(link.exists() && outside.exists());
        }
        delete_item(&folder, &item).unwrap();
        assert!(!item.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
