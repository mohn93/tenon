# Linux

## File locations

The window follows the XDG Base Directory layout. An `XDG_*` variable counts only when it is set
to an absolute path; otherwise its default under `HOME` is used. Without an absolute `HOME` the
window warns, keeps no preferences and no recovery files.

| What | Location |
|---|---|
| Preferences | `$XDG_CONFIG_HOME/sketchup-alt/prefs.json` (default `~/.config/sketchup-alt/prefs.json`) |
| Recovery files (work an agent discards) | `$XDG_STATE_HOME/sketchup-alt/recovery/` (default `~/.local/state/sketchup-alt/recovery/`) |
| Default agent socket | `$XDG_RUNTIME_DIR/sketchup-alt-<pid>.sock`, else `~/.sketchup-alt/sketchup-alt-<pid>.sock` |

Folders the app creates are mode 0700 and files 0600. The runtime folder is used only when it is
a real folder you own that group and others cannot write to, and the socket path fits the Unix
limit; otherwise the socket goes in `~/.sketchup-alt/` (created 0700).

Earlier versions kept everything in `~/.sketchup-alt`. If the new preferences file does not exist
yet, the old `~/.sketchup-alt/prefs.json` is read once; the next change is saved to the new
location and the old file is left alone (delete it when you like). Recovery files already in
`~/.sketchup-alt/recovery` are not moved.

macOS keeps `~/.sketchup-alt/prefs.json`, `~/.sketchup-alt/recovery` and the socket in the temp
folder, as before.

## Keys

Commands use Ctrl (Ctrl+S, Ctrl+Z, ...). The Super key never runs a command.

In fly mode Ctrl flies straight down and W, A, S, D, E, Q and Space move. A movement key pressed
with Ctrl held is a shortcut only when Ctrl plus that key is a bound command (Ctrl+S saves,
Ctrl+Q quits); otherwise it still steers, so Ctrl+W flies forward while descending.

## File dialogs

Open, Save and Export use the xdg-desktop-portal file chooser, falling back to `zenity`. If
neither is installed, the dialog cannot open. The dialog library (rfd 0.17) logs that to stderr
and returns the same answer as Cancel, so the window cannot tell the two apart and shows no
message; install `xdg-desktop-portal` with a backend, or `zenity`.
