//! Every keyboard shortcut in one table. Menus and tooltips take their key text from it and
//! the window's key handling looks keys up in it, so a label cannot name a key that does
//! something else.

use tools::ToolKind;

use crate::ui::UiAction;

/// What a shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bound {
    Tool(ToolKind),
    NewModel,
    OpenFile,
    SaveFile,
    SaveFileAs,
    ImportFile,
    ExportFile,
    Quit,
    Undo,
    Redo,
    SelectAll,
    SelectNone,
    Duplicate,
    EraseSelection,
    MakeGroup,
    MakeComponent,
    CloseInstance,
    ZoomExtents,
    ToggleFly,
    RenameComponent,
    Settings,
}

/// The key of a shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A lowercase ASCII letter.
    Letter(char),
    Space,
    Delete,
    Escape,
    /// A function key: not a character, so never a plain key the editor types.
    F2,
    /// ",", only with Cmd (Settings).
    Comma,
}

/// A key and the modifiers held with it. `command` is Cmd on macOS and Ctrl elsewhere (the
/// window takes either everywhere).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub command: bool,
    pub shift: bool,
    pub key: Key,
}

#[derive(Clone, Copy, Debug)]
pub struct Shortcut {
    pub bound: Bound,
    pub chord: Chord,
    /// Held down, the key acts again.
    pub repeats: bool,
}

const fn on(bound: Bound, command: bool, shift: bool, key: Key) -> Shortcut {
    Shortcut {
        bound,
        chord: Chord {
            command,
            shift,
            key,
        },
        repeats: false,
    }
}

/// A tool's key. The letters restate `ToolKind::shortcut`, which is not `const`; a test keeps
/// the two the same.
const fn tool(kind: ToolKind, c: char) -> Shortcut {
    on(Bound::Tool(kind), false, false, Key::Letter(c))
}

const fn repeating(s: Shortcut) -> Shortcut {
    Shortcut { repeats: true, ..s }
}

pub const TABLE: [Shortcut; 33] = [
    on(Bound::Tool(ToolKind::Select), false, false, Key::Space),
    tool(ToolKind::Line, 'l'),
    tool(ToolKind::Rectangle, 'r'),
    tool(ToolKind::Circle, 'c'),
    tool(ToolKind::Polygon, 'n'),
    tool(ToolKind::Arc, 'a'),
    tool(ToolKind::PushPull, 'p'),
    tool(ToolKind::Move, 'm'),
    tool(ToolKind::Rotate, 'q'),
    tool(ToolKind::Scale, 's'),
    tool(ToolKind::TapeMeasure, 't'),
    tool(ToolKind::Erase, 'e'),
    tool(ToolKind::Paint, 'b'),
    on(Bound::NewModel, true, false, Key::Letter('n')),
    on(Bound::OpenFile, true, false, Key::Letter('o')),
    on(Bound::SaveFile, true, false, Key::Letter('s')),
    on(Bound::SaveFileAs, true, true, Key::Letter('s')),
    on(Bound::ImportFile, true, false, Key::Letter('i')),
    on(Bound::ExportFile, true, true, Key::Letter('e')),
    on(Bound::Quit, true, false, Key::Letter('q')),
    repeating(on(Bound::Undo, true, false, Key::Letter('z'))),
    repeating(on(Bound::Redo, true, true, Key::Letter('z'))),
    on(Bound::SelectAll, true, false, Key::Letter('a')),
    on(Bound::SelectNone, true, true, Key::Letter('a')),
    on(Bound::Duplicate, true, false, Key::Letter('d')),
    on(Bound::Settings, true, false, Key::Comma),
    on(Bound::EraseSelection, false, false, Key::Delete),
    on(Bound::MakeGroup, false, false, Key::Letter('g')),
    on(Bound::MakeComponent, false, true, Key::Letter('g')),
    on(Bound::CloseInstance, false, false, Key::Escape),
    on(Bound::RenameComponent, false, false, Key::F2),
    on(Bound::ZoomExtents, false, true, Key::Letter('z')),
    on(Bound::ToggleFly, false, false, Key::Letter('f')),
];

/// The chord bound to `bound`, if any.
pub fn chord(bound: Bound) -> Option<Chord> {
    TABLE.iter().find(|s| s.bound == bound).map(|s| s.chord)
}

/// The shortcut `chord` is bound to, if any.
pub fn lookup(chord: Chord) -> Option<&'static Shortcut> {
    TABLE.iter().find(|s| s.chord == chord)
}

/// What Cmd (or Ctrl) with `letter` (a lowercase letter, or ",") does; `repeat` when the key
/// is held down. The modifiers must match exactly: Shift+Cmd+N is not Cmd+N.
pub fn command(letter: char, shift: bool, repeat: bool) -> Option<Bound> {
    let key = match letter {
        ',' => Key::Comma,
        c => Key::Letter(c),
    };
    let chord = Chord {
        command: true,
        shift,
        key,
    };
    lookup(chord)
        .filter(|s| s.repeats || !repeat)
        .map(|s| s.bound)
}

/// Whether the window has native menus (macOS's menu bar), which take the Cmd chords.
pub const NATIVE_MENUS: bool = cfg!(target_os = "macos");

/// What a Cmd or Ctrl chord does in the window's key path. `cmd`: Cmd is held (Ctrl may be
/// too). With native menus the menu bar has every Cmd chord as an accelerator and acts on it
/// itself, so the key path leaves those alone; Ctrl chords still act here, as before.
pub fn window_command(
    letter: char,
    shift: bool,
    repeat: bool,
    cmd: bool,
    native_menus: bool,
) -> Option<Bound> {
    if native_menus && cmd {
        return None;
    }
    command(letter, shift, repeat)
}

/// What a key typed without Cmd does, as the editor gets it: `' '` for Space, a letter
/// uppercase exactly when Shift is held. An uppercase letter with no binding of its own acts
/// as its lowercase one, so Shift+L picks Line while Shift+G makes a component.
pub fn plain(c: char) -> Option<Bound> {
    let find = |shift, key| {
        lookup(Chord {
            command: false,
            shift,
            key,
        })
        .map(|s| s.bound)
    };
    if c == ' ' {
        return find(false, Key::Space);
    }
    if !c.is_ascii_alphabetic() {
        return None;
    }
    let key = Key::Letter(c.to_ascii_lowercase());
    let shift = c.is_ascii_uppercase();
    find(shift, key).or_else(|| if shift { find(false, key) } else { None })
}

impl Chord {
    /// How menus and tooltips show the chord on this system.
    pub fn text(self) -> String {
        self.text_for(cfg!(target_os = "macos"))
    }

    /// `⇧⌘S` on macOS, `Ctrl+Shift+S` elsewhere.
    pub fn text_for(self, mac: bool) -> String {
        let key = match self.key {
            Key::Letter(c) => c.to_ascii_uppercase().to_string(),
            Key::Space => "Space".into(),
            Key::Delete if mac => "⌫".into(),
            Key::Delete => "Delete".into(),
            Key::Escape => "Esc".into(),
            Key::F2 => "F2".into(),
            Key::Comma => ",".into(),
        };
        let (shift, command) = if mac {
            ("⇧", "⌘")
        } else {
            ("Shift+", "Ctrl+")
        };
        let shift = if self.shift { shift } else { "" };
        let command = if self.command { command } else { "" };
        if mac {
            format!("{shift}{command}{key}")
        } else {
            format!("{command}{shift}{key}")
        }
    }
}

/// The groups the Keyboard Shortcuts window lists, in order. The window lays them out by name
/// (`dialogs::SHORTCUT_COLUMNS`); tests check that covers all of them.
#[cfg_attr(not(test), allow(dead_code))]
pub const GROUPS: [&str; 5] = ["Tools", "File", "Edit", "Group", "Camera"];

impl Bound {
    /// What the binding is called in the Keyboard Shortcuts window. The Erase tool and the
    /// Erase command have distinct names there.
    pub fn label(self) -> &'static str {
        match self {
            Bound::Tool(ToolKind::Erase) => "Erase tool",
            Bound::Tool(kind) => kind.label(),
            Bound::NewModel => "New",
            Bound::OpenFile => "Open",
            Bound::SaveFile => "Save",
            Bound::SaveFileAs => "Save As",
            Bound::ImportFile => "Import",
            Bound::ExportFile => "Export",
            Bound::Quit => "Quit",
            Bound::Undo => "Undo",
            Bound::Redo => "Redo",
            Bound::SelectAll => "Select All",
            Bound::SelectNone => "Select None",
            Bound::Duplicate => "Duplicate",
            Bound::EraseSelection => "Erase selection",
            Bound::MakeGroup => "Make Group",
            Bound::MakeComponent => "Make Component",
            Bound::CloseInstance => "Close Group",
            Bound::ZoomExtents => "Zoom Extents",
            Bound::ToggleFly => "Fly Mode",
            Bound::RenameComponent => "Rename Component",
            Bound::Settings => "Settings",
        }
    }

    /// Which of `GROUPS` the binding is listed under.
    pub fn group(self) -> &'static str {
        match self {
            Bound::Tool(_) => "Tools",
            Bound::NewModel
            | Bound::OpenFile
            | Bound::SaveFile
            | Bound::SaveFileAs
            | Bound::ImportFile
            | Bound::ExportFile
            | Bound::Quit => "File",
            Bound::Undo
            | Bound::Redo
            | Bound::SelectAll
            | Bound::SelectNone
            | Bound::Duplicate
            | Bound::EraseSelection
            | Bound::Settings => "Edit",
            Bound::MakeGroup
            | Bound::MakeComponent
            | Bound::CloseInstance
            | Bound::RenameComponent => "Group",
            Bound::ZoomExtents | Bound::ToggleFly => "Camera",
        }
    }

    /// The panels' action for the binding.
    pub fn action(self) -> Option<UiAction> {
        Some(match self {
            Bound::Tool(kind) => UiAction::Tool(kind),
            Bound::NewModel => UiAction::NewModel,
            Bound::OpenFile => UiAction::OpenFile,
            Bound::SaveFile => UiAction::SaveFile,
            Bound::SaveFileAs => UiAction::SaveFileAs,
            Bound::ImportFile => UiAction::ImportFile,
            Bound::ExportFile => UiAction::ExportFile,
            Bound::Quit => UiAction::Quit,
            Bound::Undo => UiAction::Undo,
            Bound::Redo => UiAction::Redo,
            Bound::SelectAll => UiAction::SelectAll,
            Bound::SelectNone => UiAction::SelectNone,
            Bound::Duplicate => UiAction::Duplicate,
            Bound::EraseSelection => UiAction::EraseSelection,
            Bound::MakeGroup => UiAction::MakeGroup,
            Bound::MakeComponent => UiAction::MakeComponent,
            Bound::CloseInstance => UiAction::CloseInstance,
            Bound::ZoomExtents => UiAction::ZoomExtents,
            Bound::ToggleFly => UiAction::ToggleFly,
            Bound::RenameComponent => UiAction::RenameComponent,
            Bound::Settings => UiAction::ShowSettings(true),
        })
    }

    /// The key text menus and tooltips show for the binding, if it has a key.
    pub fn key_text(self) -> Option<String> {
        chord(self).map(Chord::text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_binding_and_every_chord_is_in_the_table_once() {
        for (i, a) in TABLE.iter().enumerate() {
            for b in &TABLE[i + 1..] {
                assert_ne!(a.bound, b.bound, "{:?} twice", a.bound);
                assert_ne!(
                    a.chord, b.chord,
                    "{:?} and {:?} share a key",
                    a.bound, b.bound
                );
            }
        }
    }

    #[test]
    fn every_binding_has_a_label_and_a_listed_group() {
        for s in TABLE {
            assert!(!s.bound.label().is_empty());
            assert!(GROUPS.contains(&s.bound.group()), "{:?}", s.bound);
        }
        assert_eq!(Bound::SaveFileAs.label(), "Save As");
        assert_eq!(Bound::Tool(ToolKind::TapeMeasure).group(), "Tools");
        let labels: Vec<_> = TABLE.iter().map(|s| s.bound.label()).collect();
        for (i, l) in labels.iter().enumerate() {
            assert!(!labels[i + 1..].contains(l), "{l} twice");
        }
    }

    #[test]
    fn tool_keys_are_the_tools_own() {
        for kind in ToolKind::ALL {
            let key = match kind.shortcut() {
                ' ' => Key::Space,
                c => Key::Letter(c),
            };
            assert_eq!(
                chord(Bound::Tool(kind)),
                Some(Chord {
                    command: false,
                    shift: false,
                    key
                }),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn plain_keys_find_their_bindings() {
        assert_eq!(plain(' '), Some(Bound::Tool(ToolKind::Select)));
        assert_eq!(plain('l'), Some(Bound::Tool(ToolKind::Line)));
        // An uppercase letter with no binding of its own acts as its lowercase one.
        assert_eq!(plain('L'), Some(Bound::Tool(ToolKind::Line)));
        assert_eq!(plain('F'), Some(Bound::ToggleFly));
        assert_eq!(plain('g'), Some(Bound::MakeGroup));
        assert_eq!(plain('G'), Some(Bound::MakeComponent));
        assert_eq!(plain('Z'), Some(Bound::ZoomExtents));
        assert_eq!(plain('z'), None);
        assert_eq!(plain('5'), None);
        // Cmd chords are not plain keys.
        assert_eq!(plain('o'), None);
        assert_eq!(plain('s'), Some(Bound::Tool(ToolKind::Scale)));
    }

    #[test]
    fn command_chords_match_their_modifiers_and_repeat_only_undo_and_redo() {
        assert_eq!(command('z', false, false), Some(Bound::Undo));
        assert_eq!(command('z', false, true), Some(Bound::Undo), "held");
        assert_eq!(command('z', true, true), Some(Bound::Redo), "held");
        assert_eq!(command('s', false, false), Some(Bound::SaveFile));
        assert_eq!(command('s', false, true), None, "held Cmd+S saves once");
        assert_eq!(command('s', true, false), Some(Bound::SaveFileAs));
        assert_eq!(command('i', false, false), Some(Bound::ImportFile));
        assert_eq!(command('i', true, false), None);
        assert_eq!(command('e', true, false), Some(Bound::ExportFile));
        assert_eq!(command('e', false, false), None);
        assert_eq!(command('q', false, false), Some(Bound::Quit));
        assert_eq!(command('l', false, false), None);
        assert_eq!(command('a', false, false), Some(Bound::SelectAll));
        assert_eq!(command('a', true, false), Some(Bound::SelectNone));
        assert_eq!(command('d', false, false), Some(Bound::Duplicate));
        assert_eq!(command('a', false, true), None, "held Cmd+A selects once");
        assert_eq!(
            plain('a'),
            Some(Bound::Tool(ToolKind::Arc)),
            "plain a is still Arc"
        );
    }

    #[test]
    fn with_native_menus_the_key_path_leaves_cmd_chords_to_the_menu() {
        for s in TABLE.iter().filter(|s| s.chord.command) {
            // The character the key types: a letter, or "," for Settings.
            let c = match s.chord.key {
                Key::Letter(c) => c,
                Key::Comma => ',',
                _ => panic!("{:?}", s.bound),
            };
            let shift = s.chord.shift;
            let both = command(c, shift, false);
            assert_eq!(
                window_command(c, shift, false, true, true),
                None,
                "{:?}",
                s.bound
            );
            // Ctrl chords are not the native menu's: they keep working through the key path.
            assert_eq!(
                window_command(c, shift, false, false, true),
                both,
                "{:?}",
                s.bound
            );
            // Without native menus the key path takes Cmd and Ctrl alike.
            assert_eq!(
                window_command(c, shift, false, true, false),
                both,
                "{:?}",
                s.bound
            );
            assert_eq!(
                window_command(c, shift, false, false, false),
                both,
                "{:?}",
                s.bound
            );
        }
        // Held keys still repeat only Undo and Redo on the key path.
        assert_eq!(window_command('s', false, true, false, true), None);
        assert_eq!(
            window_command('z', false, true, false, true),
            Some(Bound::Undo)
        );
        // Plain keys never go through here; they stay the editor's on every system.
        assert_eq!(plain('l'), Some(Bound::Tool(ToolKind::Line)));
        assert_eq!(NATIVE_MENUS, cfg!(target_os = "macos"));
    }

    #[test]
    fn chords_read_as_mac_glyphs_or_words() {
        let text = |b, mac| chord(b).unwrap().text_for(mac);
        assert_eq!(text(Bound::SaveFileAs, true), "⇧⌘S");
        assert_eq!(text(Bound::SaveFileAs, false), "Ctrl+Shift+S");
        assert_eq!(text(Bound::Undo, true), "⌘Z");
        assert_eq!(text(Bound::MakeComponent, true), "⇧G");
        assert_eq!(text(Bound::MakeComponent, false), "Shift+G");
        assert_eq!(text(Bound::Tool(ToolKind::Rectangle), false), "R");
        assert_eq!(text(Bound::Tool(ToolKind::Select), true), "Space");
        assert_eq!(text(Bound::EraseSelection, true), "⌫");
        assert_eq!(text(Bound::EraseSelection, false), "Delete");
        assert_eq!(text(Bound::CloseInstance, true), "Esc");
        assert_eq!(text(Bound::SelectNone, true), "⇧⌘A");
        assert_eq!(text(Bound::Duplicate, false), "Ctrl+D");
    }

    #[test]
    fn settings_is_cmd_comma() {
        let settings = chord(Bound::Settings).unwrap();
        assert_eq!(
            settings,
            Chord {
                command: true,
                shift: false,
                key: Key::Comma
            }
        );
        assert_eq!(settings.text_for(true), "⌘,");
        assert_eq!(settings.text_for(false), "Ctrl+,");
        assert_eq!(command(',', false, false), Some(Bound::Settings));
        assert_eq!(command(',', true, false), None);
        assert_eq!(command(',', false, true), None, "held, it opens once");
        assert_eq!(Bound::Settings.label(), "Settings");
        assert_eq!(Bound::Settings.group(), "Edit");
        assert_eq!(Bound::Settings.action(), Some(UiAction::ShowSettings(true)));
        // A comma typed without Cmd is no shortcut.
        assert_eq!(plain(','), None);
    }

    #[test]
    fn f2_renames() {
        let f2 = chord(Bound::RenameComponent).unwrap();
        assert_eq!(
            f2,
            Chord {
                command: false,
                shift: false,
                key: Key::F2
            }
        );
        assert_eq!(f2.text_for(true), "F2");
        assert_eq!(f2.text_for(false), "F2");
        assert_eq!(lookup(f2).map(|s| s.bound), Some(Bound::RenameComponent));
        assert_eq!(Bound::RenameComponent.group(), "Group");
        assert_eq!(
            Bound::RenameComponent.action(),
            Some(UiAction::RenameComponent)
        );
        // F2 is not a character: no typed character finds it.
        for c in (0u8..128).map(char::from) {
            assert_ne!(plain(c), Some(Bound::RenameComponent), "{c:?}");
        }
    }

    #[test]
    fn bindings_ask_for_the_panels_actions() {
        assert_eq!(
            Bound::Tool(ToolKind::Move).action(),
            Some(UiAction::Tool(ToolKind::Move))
        );
        assert_eq!(Bound::SaveFileAs.action(), Some(UiAction::SaveFileAs));
        assert_eq!(Bound::Quit.action(), Some(UiAction::Quit));
    }
}
