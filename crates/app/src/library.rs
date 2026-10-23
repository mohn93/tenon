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

/// The longest file name stem a saved item gets, in characters and in UTF-8 bytes.
const MAX_STEM: usize = 100;
const MAX_STEM_BYTES: usize = 200;

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
    let mut bytes = 0;
    let stem: String = cleaned
        .trim_matches(|c: char| c.is_whitespace() || c == '.')
        .chars()
        .take(MAX_STEM)
        .take_while(|c| {
            bytes += c.len_utf8();
            bytes <= MAX_STEM_BYTES
        })
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

/// What tells one version of a file from another: its modification time (seconds) and size,
/// so an overwrite within the same second still shows.
pub type Stamp = (u64, u64);

/// A thumbnail asked for: a file and the version of it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThumbKey {
    pub path: PathBuf,
    pub stamp: Stamp,
}

/// What is known of a thumbnail.
#[derive(Debug, PartialEq)]
pub enum Slot<T> {
    Ready(T),
    /// Could not be made. With a reason, the item itself is refused (see `session::library::
    /// read_item`); without, there is only no preview (empty, no GPU).
    Failed(Option<String>),
}

/// The thumbnails made so far, by file and modification time, and the ones asked for. Which
/// to make next is decided here (at most `THUMBS_PER_FRAME` a frame, in the order asked); how
/// to make one is the caller's.
pub struct Thumbnails<T> {
    slots: HashMap<PathBuf, (Stamp, Slot<T>)>,
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
    /// What is held for `path` as of `stamp`; None when not made yet (or made for another
    /// version of the file).
    pub fn get(&self, path: &Path, stamp: Stamp) -> Option<&Slot<T>> {
        self.slots
            .get(path)
            .filter(|(m, _)| *m == stamp)
            .map(|(_, slot)| slot)
    }

    /// Asks for a thumbnail, unless it is held or already asked for.
    pub fn want(&mut self, key: ThumbKey) {
        if self.get(&key.path, key.stamp).is_none() && !self.wanted.contains(&key) {
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
    pub fn store(&mut self, key: ThumbKey, made: Result<T, Option<String>>) {
        let slot = made.map_or_else(Slot::Failed, Slot::Ready);
        self.slots.insert(key.path, (key.stamp, slot));
    }

    /// Drops what is held and asked for, except for these files (the listing now).
    pub fn keep_only(&mut self, listed: &[(PathBuf, Stamp)]) {
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

/// RGBA8 `pixels` of a `width` x `height` image fitted inside a `side` x `side` square on white,
/// centred, keeping the image's aspect (box filtered). Opaque.
pub fn letterbox(pixels: &[u8], width: u32, height: u32, side: u32) -> Vec<u8> {
    let (w, h, s) = (width.max(1) as usize, height.max(1) as usize, side as usize);
    let mut out = vec![255u8; s * s * 4];
    if pixels.len() < w * h * 4 || s == 0 {
        return out;
    }
    let longest = w.max(h);
    let nw = (s * w / longest).clamp(1, s);
    let nh = (s * h / longest).clamp(1, s);
    let (left, top) = ((s - nw) / 2, (s - nh) / 2);
    for y in 0..nh {
        let (y0, y1) = (
            y * h / nh,
            ((y + 1) * h).div_ceil(nh).clamp(y * h / nh + 1, h),
        );
        for x in 0..nw {
            let (x0, x1) = (
                x * w / nw,
                ((x + 1) * w).div_ceil(nw).clamp(x * w / nw + 1, w),
            );
            let mut sum = [0u32; 3];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let at = (sy * w + sx) * 4;
                    for c in 0..3 {
                        sum[c] += u32::from(pixels[at + c]);
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let at = ((top + y) * s + left + x) * 4;
            for c in 0..3 {
                out[at + c] = (sum[c] / n) as u8;
            }
        }
    }
    out
}

/// Makes the thumbnail of the item at `path`: a component or model in an iso view fitted to
/// its box on white paper, a material as its color or texture. Err says why not: the item
/// cannot be read (`Some` reason: also when it is over the clip caps, which `read_item`
/// checks before anything walks its placements), is empty or the GPU fails (`None`).
pub fn make(snapshotter: &mut Snapshotter, path: &Path) -> Result<Pixels, Option<String>> {
    let item = library::read_item(path).map_err(|e| Some(e.message))?;
    make_from(snapshotter, &item).ok_or(None)
}

fn make_from(snapshotter: &mut Snapshotter, item: &session::LibraryContent) -> Option<Pixels> {
    let side = THUMB_SIZE as usize;
    if item.kind == LibraryKind::Material {
        let (_, material) = item.scene.materials().next()?;
        let mut pixels = match &material.texture {
            Some(t) => letterbox(&t.image.pixels, t.image.width, t.image.height, THUMB_SIZE),
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
    snapshotter.set_glass_view(render::scene::GlassView::default());
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
            stamp: (modified, 10),
        }
    }

    #[test]
    fn a_file_name_stem_is_at_most_100_characters_and_200_bytes() {
        let long = file_name_for(&"a".repeat(300)).unwrap();
        assert_eq!(long.len(), 100 + ".skr".len());
        let wide = file_name_for(&"\u{1F600}".repeat(100)).unwrap();
        assert_eq!(wide.len(), 200 + ".skr".len());
        let kana = file_name_for(&"\u{3042}".repeat(100)).unwrap();
        assert_eq!(kana.len(), 66 * 3 + ".skr".len());
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
        t.store(key("a", 1), Ok(7u32));
        t.store(key("b", 1), Err(Some("too deep".into())));
        assert_eq!(
            t.get(Path::new("/lib/a.skr"), (1, 10)),
            Some(&Slot::Ready(7))
        );
        assert_eq!(
            t.get(Path::new("/lib/b.skr"), (1, 10)),
            Some(&Slot::Failed(Some("too deep".into())))
        );
        // Held ones are not asked for again; a file written since is.
        t.want(key("a", 1));
        t.want(key("b", 1));
        assert!(!t.pending());
        assert_eq!(t.get(Path::new("/lib/a.skr"), (2, 10)), None);
        t.want(key("a", 2));
        assert_eq!(t.next_batch(), [key("a", 2)]);
        // The newer one replaces the old.
        t.store(key("a", 2), Ok(8));
        assert_eq!(t.get(Path::new("/lib/a.skr"), (1, 10)), None);
        assert_eq!(
            t.get(Path::new("/lib/a.skr"), (2, 10)),
            Some(&Slot::Ready(8))
        );
    }

    #[test]
    fn a_same_second_overwrite_of_another_size_is_a_new_version() {
        let mut t = Thumbnails::default();
        t.store(key("a", 1), Ok(1u32));
        let rewritten = ThumbKey {
            stamp: (1, 11),
            ..key("a", 1)
        };
        assert!(t.get(&rewritten.path, rewritten.stamp).is_none());
        t.want(rewritten.clone());
        assert_eq!(t.next_batch(), [rewritten]);
    }

    #[test]
    fn items_gone_from_the_listing_are_forgotten() {
        let mut t = Thumbnails::default();
        t.store(key("a", 1), Ok(1u32));
        t.store(key("b", 1), Ok(2));
        t.want(key("c", 1));
        t.want(key("d", 1));
        t.keep_only(&[
            (PathBuf::from("/lib/a.skr"), (1, 10)),
            (PathBuf::from("/lib/d.skr"), (1, 10)),
        ]);
        assert!(t.get(Path::new("/lib/a.skr"), (1, 10)).is_some());
        assert!(t.get(Path::new("/lib/b.skr"), (1, 10)).is_none());
        assert_eq!(t.next_batch(), [key("d", 1)]);
        t.clear();
        assert!(t.get(Path::new("/lib/a.skr"), (1, 10)).is_none());
    }

    #[test]
    fn a_wide_texture_is_letterboxed_not_squashed() {
        // 4 x 2: left half red, right half blue.
        let mut image = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                image.extend_from_slice(if x < 2 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 0, 255, 255]
                });
            }
        }
        let out = letterbox(&image, 4, 2, 8);
        let px = |x: usize, y: usize| out[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4].to_vec();
        // The image fills the width and the middle four rows; white above and below.
        assert_eq!(px(0, 0), [255, 255, 255, 255]);
        assert_eq!(px(7, 7), [255, 255, 255, 255]);
        assert_eq!(px(0, 2), [255, 0, 0, 255]);
        assert_eq!(px(3, 5), [255, 0, 0, 255]);
        assert_eq!(px(4, 2), [0, 0, 255, 255]);
        assert_eq!(px(7, 5), [0, 0, 255, 255]);
        assert_eq!(px(3, 1), [255, 255, 255, 255]);
        assert_eq!(px(3, 6), [255, 255, 255, 255]);
        // A square image fills it; a tall one is centred in columns.
        let square = letterbox(&[9, 9, 9, 255].repeat(4), 2, 2, 4);
        assert!(square.chunks(4).all(|p| p == [9, 9, 9, 255]));
        let tall = letterbox(&[1, 2, 3, 255].repeat(2), 1, 2, 4);
        let at = |x: usize, y: usize| tall[(y * 4 + x) * 4..(y * 4 + x) * 4 + 3].to_vec();
        assert_eq!(at(0, 0), [255, 255, 255]);
        assert_eq!(at(1, 0), [1, 2, 3]);
        assert_eq!(at(2, 3), [1, 2, 3]);
        assert_eq!(at(3, 3), [255, 255, 255]);
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
