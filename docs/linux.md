# Linux

## Build

A Rust toolchain (the version in `.github/workflows/ci.yml`) and a C linker (`build-essential` on
Debian and Ubuntu). There are no `-dev` packages: the dependency tree links no GTK, X11 or
Wayland library at build time.

```bash
cargo build --release -p app
```

## Run

The window needs a Vulkan or OpenGL driver and these libraries at run time. On Debian and Ubuntu:

```bash
sudo apt-get install mesa-vulkan-drivers libvulkan1 libxkbcommon0 libwayland-client0 \
    libx11-6 libxcursor1 libxrandr2 libxi6 libxkbcommon-x11-0 libwayland-cursor0 libwayland-egl1
```

Use your GPU vendor's driver instead of `mesa-vulkan-drivers` where it has one. The window
library supports Wayland and X11, but CI runs no desktop session, so neither is verified by tests.
Open, Save and Export need `xdg-desktop-portal` (with a backend) or `zenity` (see File dialogs
below).

Headless mode and the MCP server (`--mcp`) open no window and need no display. Rendering to an
image (`render_view`) still needs a Vulkan or GL adapter; Mesa's software adapter (lavapipe, in
`mesa-vulkan-drivers`) is enough.

## File locations

The window follows the XDG Base Directory layout. An `XDG_*` variable counts only when it is set
to an absolute path; otherwise its default under `HOME` is used. Without an absolute `HOME` the
window warns, keeps no preferences and no recovery files.

| What | Location |
|---|---|
| Preferences | `$XDG_CONFIG_HOME/sketchup-alt/prefs.json` (default `~/.config/sketchup-alt/prefs.json`) |
| Recovery files (work an agent discards) | `$XDG_STATE_HOME/sketchup-alt/recovery/` (default `~/.local/state/sketchup-alt/recovery/`) |
| Default agent socket | `sketchup-alt-<pid>.sock` in `$XDG_RUNTIME_DIR`, else in `$TMPDIR`, else in `~/.sketchup-alt/` (see below) |

Folders the app creates are mode 0700 and files 0600. The runtime folder is used only when it is
a real folder you own that group and others cannot write to, and the socket path fits the Unix
limit. Otherwise the socket goes in `$TMPDIR` when that is set to a folder other than `/tmp`
and short enough for a socket path, and failing that in `~/.sketchup-alt/` (created 0700).

Earlier versions kept everything in `~/.sketchup-alt`. If the new preferences file does not exist
yet, the old `~/.sketchup-alt/prefs.json` is read on every start until the first change is saved;
that save writes the new location, which is read from then on, and the old file is left alone
(delete it when you like). Recovery files already in
`~/.sketchup-alt/recovery` are not moved.

macOS keeps `~/.sketchup-alt/prefs.json`, `~/.sketchup-alt/recovery` and the socket in the temp
folder, as before.

## Keys

Commands use Ctrl (Ctrl+S, Ctrl+Z, ...). The Super key never runs a command.

In fly mode Ctrl flies straight down and W, A, S, D, E, Q and Space move. A movement key pressed
with Ctrl held is a shortcut only when Ctrl plus that key is a bound command (Ctrl+S saves,
Ctrl+Q quits); otherwise it still steers, so Ctrl+W flies forward while descending. Bound
commands win: while flying, Ctrl+A selects all and Ctrl+D duplicates instead of strafing. The key
is judged by the letter it types on your layout, as for all shortcuts.

## File dialogs

Open, Save and Export use the xdg-desktop-portal file chooser, falling back to `zenity`. If
neither is installed, the dialog cannot open. The dialog library (rfd 0.17) logs that to stderr
and returns the same answer as Cancel, so the window cannot tell the two apart and shows no
message; install `xdg-desktop-portal` with a backend, or `zenity`.

## Tests

Linux is verified in GitHub Actions (`.github/workflows/ci.yml`): fmt, clippy with `-D warnings`
and `cargo test --workspace` on `ubuntu-24.04` (x86_64) and `ubuntu-24.04-arm` (aarch64), with
Mesa lavapipe as the GPU. Golden-image tests compare against images made on a real GPU, so on a
software adapter they skip with a printed note instead of comparing, and so does the axes check.

`scripts/linux-test.sh` runs the same three commands in an Ubuntu 24.04 Docker container, using
the Rust version of your local `rustc`. It keeps build output in the Docker volume
`sketchup-alt-linux-target` (not the repo's `target/`) and can be run repeatedly. It is not
verified yet (CI is the reference and the script mirrors it).

```bash
scripts/linux-test.sh
```
