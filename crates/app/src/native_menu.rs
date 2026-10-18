//! The menu bar as macOS's own, through muda: built once from `menus::menu_bar`, then synced
//! each frame (labels, enabled flags, checks). Open Recent is the one submenu rebuilt, when its
//! list changes. Every Cmd chord is an item's accelerator, so the menu acts on it and the
//! window's key path leaves it alone (`shortcuts::window_command`).

use std::collections::HashMap;
use std::path::PathBuf;

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{AboutMetadata, CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSEventType};

use crate::menus::{self, APP_MENU, Entry, OPEN_RECENT, item_id, native_enabled};
use crate::shortcuts::{Chord, Key};
use crate::ui::UiAction;

/// The app's main menu and its items by id.
pub struct NativeMenu {
    /// Kept alive: it is the app's main menu.
    _menu: Menu,
    items: HashMap<String, Native>,
    /// The Open Recent submenu and the paths it lists now.
    recent: Option<(Submenu, Vec<PathBuf>)>,
}

/// One item and what was last applied to it.
struct Native {
    widget: Widget,
    /// What the item does now (Night Lighting's action flips with the lighting).
    action: UiAction,
    text: String,
    enabled: bool,
}

enum Widget {
    Plain(MenuItem),
    Check(CheckMenuItem),
}

impl NativeMenu {
    /// Builds the menu bar and makes it the app's main menu. Main thread only.
    pub fn new(bar: &[menus::Menu]) -> Self {
        let menu = Menu::new();
        let mut items = HashMap::new();
        let mut recent = None;
        for m in bar {
            let submenu = Submenu::new(m.title, true);
            if m.title == APP_MENU {
                for slot in app_menu_order(&m.entries) {
                    match slot {
                        AppSlot::About => {
                            let about = AboutMetadata {
                                name: Some("sketchup-alt".into()),
                                version: Some(env!("CARGO_PKG_VERSION").into()),
                                ..Default::default()
                            };
                            append(
                                &submenu,
                                &PredefinedMenuItem::about(Some("About sketchup-alt"), Some(about)),
                            );
                        }
                        AppSlot::Separator => append(&submenu, &PredefinedMenuItem::separator()),
                        AppSlot::Hide => append(&submenu, &PredefinedMenuItem::hide(None)),
                        AppSlot::HideOthers => {
                            append(&submenu, &PredefinedMenuItem::hide_others(None));
                        }
                        AppSlot::ShowAll => append(&submenu, &PredefinedMenuItem::show_all(None)),
                        AppSlot::Entries(entries) => {
                            add_entries(&submenu, entries, &mut items, &mut recent);
                        }
                    }
                }
            } else {
                add_entries(&submenu, &m.entries, &mut items, &mut recent);
            }
            menu.append(&submenu).expect("add a menu to the menu bar");
        }
        menu.init_for_nsapp();
        Self {
            _menu: menu,
            items,
            recent,
        }
    }

    /// Brings the items up to date with `bar`, touching only what changed. Checks are compared
    /// with the item's own state: a check item toggles itself when clicked. While a sheet is
    /// open (`sheet_open`) every item but Quit is disabled, the rebuilt recent rows too.
    pub fn sync(&mut self, bar: &[menus::Menu], text_focused: bool, sheet_open: bool) {
        self.sync_recent(bar);
        for item in menus::items(bar) {
            let Some(native) = self.items.get_mut(&item_id(&item.action)) else {
                continue;
            };
            native.action = item.action.clone();
            if native.text != item.label {
                native.text.clone_from(&item.label);
                match &native.widget {
                    Widget::Plain(w) => w.set_text(&item.label),
                    Widget::Check(w) => w.set_text(&item.label),
                }
            }
            let enabled = native_enabled(item, text_focused, sheet_open);
            if native.enabled != enabled {
                native.enabled = enabled;
                match &native.widget {
                    Widget::Plain(w) => w.set_enabled(enabled),
                    Widget::Check(w) => w.set_enabled(enabled),
                }
            }
            if let (Widget::Check(w), Some(checked)) = (&native.widget, item.checked)
                && w.is_checked() != checked
            {
                w.set_checked(checked);
            }
        }
    }

    /// Rebuilds the Open Recent submenu when `bar` lists other paths than it shows. Its new
    /// items are then synced with the rest.
    fn sync_recent(&mut self, bar: &[menus::Menu]) {
        let Some((submenu, shown)) = &mut self.recent else {
            return;
        };
        let paths = menus::recent_paths(bar);
        if *shown == paths {
            return;
        }
        while let Some(old) = submenu.remove_at(0) {
            self.items.remove(old.id().as_ref());
        }
        let entries = menus::recent_entries_in(bar).unwrap_or_default();
        add_entries(submenu, entries, &mut self.items, &mut None);
        *shown = paths;
    }

    /// What the item with `id` does now.
    pub fn action(&self, id: &str) -> Option<UiAction> {
        self.items.get(id).map(|n| n.action.clone())
    }
}

/// One place in the app menu: macOS's standard items, or some of the table's entries.
pub(crate) enum AppSlot<'a> {
    About,
    Separator,
    Hide,
    HideOthers,
    ShowAll,
    Entries(&'a [Entry]),
}

/// The app menu in macOS's order: About, then the table's first entry (Settings...), then
/// Hide, Hide Others and Show All, then the rest of the table's entries (Quit), each group
/// after a separator.
pub(crate) fn app_menu_order(entries: &[Entry]) -> Vec<AppSlot<'_>> {
    let (first, rest) = entries.split_at(entries.len().min(1));
    vec![
        AppSlot::About,
        AppSlot::Separator,
        AppSlot::Entries(first),
        AppSlot::Separator,
        AppSlot::Hide,
        AppSlot::HideOthers,
        AppSlot::ShowAll,
        AppSlot::Separator,
        AppSlot::Entries(rest),
    ]
}

fn append(submenu: &Submenu, item: &dyn IsMenuItem) {
    submenu.append(item).expect("add an item to a menu");
}

/// Appends `entries` to `submenu`, recording each item in `items`. The Open Recent submenu,
/// when met, is kept in `recent` with the paths it lists.
fn add_entries(
    submenu: &Submenu,
    entries: &[Entry],
    items: &mut HashMap<String, Native>,
    recent: &mut Option<(Submenu, Vec<PathBuf>)>,
) {
    for entry in entries {
        match entry {
            Entry::Item(item) => {
                let id = item_id(&item.action);
                let key = item
                    .bound
                    .and_then(crate::shortcuts::chord)
                    .and_then(accelerator);
                let widget = match item.checked {
                    None => {
                        let w = MenuItem::with_id(id.clone(), &item.label, item.enabled, key);
                        append(submenu, &w);
                        Widget::Plain(w)
                    }
                    Some(checked) => {
                        let w = CheckMenuItem::with_id(
                            id.clone(),
                            &item.label,
                            item.enabled,
                            checked,
                            key,
                        );
                        append(submenu, &w);
                        Widget::Check(w)
                    }
                };
                items.insert(
                    id,
                    Native {
                        widget,
                        action: item.action.clone(),
                        text: item.label.clone(),
                        enabled: item.enabled,
                    },
                );
            }
            Entry::Separator => append(submenu, &PredefinedMenuItem::separator()),
            Entry::Submenu { label, entries } => {
                let inner = Submenu::new(*label, true);
                add_entries(&inner, entries, items, recent);
                append(submenu, &inner);
                if *label == OPEN_RECENT {
                    *recent = Some((inner, menus::recent_paths_of(entries)));
                }
            }
        }
    }
}

const LETTERS: [Code; 26] = [
    Code::KeyA,
    Code::KeyB,
    Code::KeyC,
    Code::KeyD,
    Code::KeyE,
    Code::KeyF,
    Code::KeyG,
    Code::KeyH,
    Code::KeyI,
    Code::KeyJ,
    Code::KeyK,
    Code::KeyL,
    Code::KeyM,
    Code::KeyN,
    Code::KeyO,
    Code::KeyP,
    Code::KeyQ,
    Code::KeyR,
    Code::KeyS,
    Code::KeyT,
    Code::KeyU,
    Code::KeyV,
    Code::KeyW,
    Code::KeyX,
    Code::KeyY,
    Code::KeyZ,
];

/// The menu accelerator for `chord`: Some only for Cmd chords, which the menu bar owns. Plain
/// keys stay the editor's, so the native items show no key for them.
pub fn accelerator(chord: Chord) -> Option<Accelerator> {
    if !chord.command {
        return None;
    }
    let code = match chord.key {
        Key::Letter(c) => *LETTERS.get((c as usize).checked_sub('a' as usize)?)?,
        Key::Comma => Code::Comma,
        _ => return None,
    };
    let mods = if chord.shift {
        Modifiers::META | Modifiers::SHIFT
    } else {
        Modifiers::META
    };
    Some(Accelerator::new(mods, code))
}

/// Whether the event AppKit is handling now is a held key's repeat. A menu item's action runs
/// inside the handling of the key or click that chose it, so this tells a held Cmd+S from a
/// press.
pub fn current_event_repeats() -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else {
        return false;
    };
    // isARepeat raises for anything but a key event.
    event.r#type() == NSEventType::KeyDown && event.isARepeat()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcuts::{Bound, TABLE, chord};

    #[test]
    fn only_cmd_chords_become_accelerators() {
        for s in TABLE {
            assert_eq!(
                accelerator(s.chord).is_some(),
                s.chord.command,
                "{:?}",
                s.bound
            );
        }
        let save_as = chord(Bound::SaveFileAs).unwrap();
        assert_eq!(
            accelerator(save_as),
            Some(Accelerator::new(
                Modifiers::META | Modifiers::SHIFT,
                Code::KeyS
            ))
        );
        assert_eq!(
            accelerator(chord(Bound::Undo).unwrap()),
            Some(Accelerator::new(Modifiers::META, Code::KeyZ))
        );
        assert_eq!(
            accelerator(chord(Bound::NewModel).unwrap()),
            Some(Accelerator::new(Modifiers::META, Code::KeyN))
        );
        assert_eq!(
            accelerator(chord(Bound::Settings).unwrap()),
            Some(Accelerator::new(Modifiers::META, Code::Comma))
        );
    }

    #[test]
    fn letters_map_in_order() {
        for (i, c) in ('a'..='z').enumerate() {
            let chord = Chord {
                command: true,
                shift: false,
                key: Key::Letter(c),
            };
            assert_eq!(
                LETTERS[i].to_string(),
                format!("Key{}", c.to_ascii_uppercase())
            );
            assert_eq!(
                accelerator(chord),
                Some(Accelerator::new(Modifiers::META, LETTERS[i])),
                "{c}"
            );
        }
        let off = Chord {
            command: true,
            shift: false,
            key: Key::Letter('5'),
        };
        assert_eq!(accelerator(off), None);
    }
}
