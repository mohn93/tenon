//! The parts of the web build that do not touch the browser: file names and types, the
//! keys the page keeps from the browser, the computer it runs on, and the preference storage.
//! They are plain Rust so the native gate tests them.

use app::platform::Filter;
use app::prefs::{self, Prefs};

/// The `localStorage` key the preferences are kept under (the same JSON as `prefs.json`).
/// Kept from before the rename to Tenon, as stored data stays where the browser has it.
pub const PREFS_KEY: &str = "sketchup-alt-prefs";

/// The id of the page's canvas; one is made if the page has none.
pub const CANVAS_ID: &str = "tenon";

/// The value of an `<input type=file accept>`: every extension of every filter, once, with a
/// dot.
pub fn accept_attribute(filters: &[Filter]) -> String {
    let mut seen: Vec<String> = Vec::new();
    for ext in filters.iter().flat_map(|f| &f.extensions) {
        let dotted = format!(".{ext}");
        if !seen.contains(&dotted) {
            seen.push(dotted);
        }
    }
    seen.join(",")
}

/// The name a download is saved under: the name without any folder, or `fallback` when nothing
/// is left.
pub fn download_name(name: &str, fallback: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    if base.is_empty() || base == "." || base == ".." {
        fallback.to_string()
    } else {
        base.to_string()
    }
}

/// The media type a file of this name is offered as.
pub fn mime_for(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "obj" => "model/obj",
        "stl" => "model/stl",
        "3mf" => "model/3mf",
        "gltf" => "model/gltf+json",
        "glb" => "model/gltf-binary",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "mtl" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Whether a file picked by the user may be written back by Save: only a `.tnn` the model was
/// opened from (a `.skp` or an OBJ it was imported from must never be overwritten by a model).
pub fn writes_back(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(stem, e)| !stem.is_empty() && session::file::is_model_extension(e))
}

/// The file handle Save may write to, and how it comes and goes. A handle is picked with a
/// model's Open (`picked`), kept once the model opened (`opened`: a file that fails to parse
/// never replaces the kept one), and replaced by every save of a model (`saved`). Import,
/// textures and Export never touch it. `for_save` hands it out only when the window says the
/// open model is that file (`SaveRequest::write_back`: false after New).
pub struct Handles<H> {
    candidate: Option<(String, Option<H>)>,
    kept: Option<H>,
}

impl<H> Default for Handles<H> {
    fn default() -> Self {
        Self {
            candidate: None,
            kept: None,
        }
    }
}

impl<H: Clone> Handles<H> {
    /// A model file was picked (with its handle, where the browser gives one).
    pub fn picked(&mut self, name: &str, handle: Option<H>) {
        self.candidate = Some((name.to_string(), handle));
    }

    /// The file picked last opened as the model `name`: its handle is kept when it is a `.tnn`.
    pub fn opened(&mut self, name: &str) {
        let picked = self.candidate.take();
        self.kept = picked
            .filter(|(picked, _)| picked == name && writes_back(name))
            .and_then(|(_, handle)| handle);
    }

    /// The model was saved as `name` (to `handle`; None: downloaded).
    pub fn saved(&mut self, name: &str, handle: Option<H>) {
        self.kept = handle.filter(|_| writes_back(name));
    }

    /// The handle a Save may write to.
    pub fn for_save(&self, write_back: bool) -> Option<H> {
        self.kept.clone().filter(|_| write_back)
    }
}

/// Whether the page keeps the browser from acting on this key press on the canvas. The app
/// handles plain keys (tools, arrows, Space, Tab, Backspace, Delete) and the command chords of
/// `app::shortcuts`; the browser must not also scroll, search, go back or save the page.
/// Left to the browser: Quit, Cut, Copy and Paste (the page needs their events), the function keys
/// (reload, full screen, developer tools), Alt chords, and every other Ctrl or Cmd chord
/// (Ctrl+R, Ctrl+L, Ctrl+W, Ctrl+T, ...).
pub fn should_prevent_default(key: &str, ctrl: bool, meta: bool, alt: bool, shift: bool) -> bool {
    if alt {
        return false;
    }
    let mut chars = key.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    };
    if ctrl || meta {
        let Some(letter) = single.map(|c| c.to_ascii_lowercase()) else {
            return false;
        };
        // Quit is not on the web, so the browser keeps Ctrl+Q.
        if matches!(letter, 'c' | 'x' | 'v' | 'q') {
            return false;
        }
        return app::shortcuts::command(letter, shift, false).is_some();
    }
    // F1 to F12 and the like.
    if key.len() > 1 && key.starts_with('F') && key[1..].chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    true
}

/// Whether the browser runs on an Apple computer, phone or tablet, where keys are named and
/// pressed with Cmd: from `navigator.userAgentData.platform` (`ua_platform`, "" where the
/// browser has none), else `navigator.platform` (`platform`), else the user agent string
/// (`user_agent`). An iPad that says it is a Mac counts as Apple's either way.
pub fn is_apple(ua_platform: &str, platform: &str, user_agent: &str) -> bool {
    let apple = |text: &str| {
        let text = text.to_ascii_lowercase();
        ["mac", "iphone", "ipad", "ipod", "ios"]
            .iter()
            .any(|name| text.starts_with(name))
    };
    if !ua_platform.is_empty() {
        return apple(ua_platform);
    }
    if !platform.is_empty() {
        return apple(platform);
    }
    ["Macintosh", "Mac OS X", "iPhone", "iPad", "iPod"]
        .iter()
        .any(|name| user_agent.contains(name))
}

/// A place text is kept by name: `localStorage` in the browser, a map in tests.
pub trait Storage {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
}

/// The stored preferences as text; None when nothing is stored. The app checks the text.
pub fn read_prefs(storage: &dyn Storage) -> Option<String> {
    storage.get(PREFS_KEY)
}

/// Keeps `text` as the preferences.
pub fn write_prefs(storage: &dyn Storage, text: &str) -> Result<(), String> {
    storage.set(PREFS_KEY, text)
}

/// The preferences as the app reads them back from `storage`.
pub fn load_prefs(storage: &dyn Storage) -> Prefs {
    read_prefs(storage)
        .map(|text| prefs::from_text(&text))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    #[derive(Default)]
    struct Memory(RefCell<HashMap<String, String>>);

    impl Storage for Memory {
        fn get(&self, key: &str) -> Option<String> {
            self.0.borrow().get(key).cloned()
        }
        fn set(&self, key: &str, value: &str) -> Result<(), String> {
            self.0.borrow_mut().insert(key.into(), value.into());
            Ok(())
        }
    }

    #[test]
    fn the_accept_attribute_lists_each_extension_once() {
        let filters = vec![
            Filter::new("both", &["tnn", "skp"]),
            Filter::new("one", &["tnn"]),
            Filter::new("other", &["skp"]),
        ];
        assert_eq!(accept_attribute(&filters), ".tnn,.skp");
        assert_eq!(accept_attribute(&[]), "");
    }

    #[test]
    fn a_download_name_loses_its_folders() {
        assert_eq!(download_name("house.tnn", "x.tnn"), "house.tnn");
        assert_eq!(download_name("a/b\\c.obj", "x.tnn"), "c.obj");
        assert_eq!(download_name("  ", "x.tnn"), "x.tnn");
        assert_eq!(download_name("..", "x.tnn"), "x.tnn");
        assert_eq!(download_name("dir/", "x.tnn"), "x.tnn");
    }

    #[test]
    fn files_have_media_types() {
        assert_eq!(mime_for("a.tnn"), "application/octet-stream");
        assert_eq!(mime_for("a.OBJ"), "model/obj");
        assert_eq!(mime_for("a.glb"), "model/gltf-binary");
        assert_eq!(mime_for("a.mtl"), "text/plain");
        assert_eq!(mime_for("noext"), "application/octet-stream");
    }

    #[test]
    fn only_a_tnn_is_written_back() {
        assert!(writes_back("house.tnn"));
        assert!(writes_back("House.TNN"));
        assert!(!writes_back("house.skp"));
        assert!(!writes_back("house.obj"));
        assert!(!writes_back(".tnn"));
        assert!(!writes_back("tnn"));
    }

    #[test]
    fn a_kept_handle_belongs_to_the_model_opened_from_it() {
        let mut h: Handles<u32> = Handles::default();
        assert_eq!(h.for_save(true), None);
        // Open a.tnn: kept once it opened.
        h.picked("a.tnn", Some(1));
        assert_eq!(h.for_save(true), None, "not before it opened");
        h.opened("a.tnn");
        assert_eq!(h.for_save(true), Some(1));
        // The window says it is not that file's model (after New).
        assert_eq!(h.for_save(false), None);
        // A file that fails to open leaves the kept one: it is never `opened`.
        h.picked("bad.tnn", Some(2));
        assert_eq!(h.for_save(true), Some(1));
        // The next pick replaces the failed one; a SketchUp file keeps no handle.
        h.picked("b.skp", Some(3));
        h.opened("b.skp");
        assert_eq!(h.for_save(true), None);
        // A pick that is not the one opened keeps nothing.
        h.picked("c.tnn", Some(4));
        h.opened("other.tnn");
        assert_eq!(h.for_save(true), None);
    }

    #[test]
    fn saving_a_model_replaces_the_handle_and_downloads_clear_it() {
        let mut h: Handles<u32> = Handles::default();
        h.saved("x.tnn", Some(5));
        assert_eq!(h.for_save(true), Some(5));
        h.saved("y.tnn", None);
        assert_eq!(
            h.for_save(true),
            None,
            "a download is not the handle's file"
        );
        h.saved("z.tnn", Some(6));
        // An export's file is never the model's.
        h.saved("z.obj", Some(7));
        assert_eq!(h.for_save(true), None);
    }

    #[test]
    fn handled_chords_are_prevented_and_the_browsers_own_are_not() {
        // Command chords the app binds.
        assert!(should_prevent_default("s", true, false, false, false));
        assert!(should_prevent_default("s", false, true, false, false));
        assert!(
            should_prevent_default("S", true, false, false, true),
            "save as"
        );
        assert!(should_prevent_default("z", true, false, false, false));
        // Clipboard chords fire the page's copy, cut and paste events.
        for letter in ["c", "x", "v", "q"] {
            assert!(!should_prevent_default(letter, true, false, false, false));
            assert!(!should_prevent_default(letter, false, true, false, false));
        }
        // Chords the app does not bind stay the browser's.
        assert!(!should_prevent_default("r", true, false, false, false));
        assert!(!should_prevent_default("l", true, false, false, false));
        assert!(!should_prevent_default("t", true, false, false, false));
        assert!(!should_prevent_default(
            "ArrowLeft",
            true,
            false,
            false,
            false
        ));
        // Plain keys are the app's; function keys and Alt chords are the browser's.
        assert!(should_prevent_default("l", false, false, false, false));
        assert!(should_prevent_default(" ", false, false, false, false));
        assert!(should_prevent_default("Tab", false, false, false, false));
        assert!(should_prevent_default(
            "ArrowDown",
            false,
            false,
            false,
            false
        ));
        assert!(should_prevent_default(
            "Backspace",
            false,
            false,
            false,
            false
        ));
        assert!(!should_prevent_default("F5", false, false, false, false));
        assert!(!should_prevent_default("F12", false, false, false, false));
        assert!(!should_prevent_default(
            "ArrowLeft",
            false,
            false,
            true,
            false
        ));
    }

    #[test]
    fn apple_computers_are_told_by_the_platform() {
        // userAgentData (Chromium browsers) first.
        assert!(is_apple("macOS", "Win32", ""));
        assert!(!is_apple("Windows", "MacIntel", ""));
        assert!(!is_apple("Linux", "", ""));
        assert!(!is_apple("Chrome OS", "", ""));
        // navigator.platform (Safari, Firefox).
        assert!(is_apple("", "MacIntel", ""));
        assert!(is_apple("", "iPhone", ""));
        assert!(is_apple("", "iPad", ""));
        assert!(!is_apple("", "Win32", ""));
        assert!(!is_apple("", "Linux x86_64", ""));
        assert!(!is_apple("", "Linux armv8l", ""));
        // The user agent when neither says.
        let safari = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15";
        assert!(is_apple("", "", safari));
        let firefox = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:131.0) Gecko/20100101";
        assert!(!is_apple("", "", firefox));
        assert!(!is_apple("", "", ""));
    }

    #[test]
    fn prefs_round_trip_through_storage() {
        let storage = Memory::default();
        assert_eq!(read_prefs(&storage), None);
        assert_eq!(load_prefs(&storage), Prefs::default());
        let mut prefs = Prefs::default();
        prefs.autosave_minutes = prefs.autosave_minutes.wrapping_add(3);
        write_prefs(&storage, &prefs::to_text(&prefs)).unwrap();
        assert!(storage.get(PREFS_KEY).is_some());
        assert_eq!(load_prefs(&storage), prefs);
    }

    #[test]
    fn invalid_stored_prefs_fall_back_to_the_defaults() {
        let storage = Memory::default();
        storage.set(PREFS_KEY, "{ not json").unwrap();
        assert_eq!(load_prefs(&storage), Prefs::default());
    }
}
