# Windows

## Build

A Rust toolchain (the version in `.github/workflows/ci.yml`) with the MSVC target and the
Visual Studio Build Tools (the C++ workload, for the linker).

```bash
cargo build --release -p app
```

The result is one program, `app.exe`, for every mode.

## Run

Double-click `app.exe`, or run it from a terminal. It is a console program, since headless mode
and the MCP server talk over stdin and stdout. Started from Explorer, the window lets go of the
console Windows opened for it, so no console window stays open beside it (and its log lines go
nowhere). Started from a terminal, the window keeps the terminal and writes its log lines there.
`--headless --stdio` and `--mcp` always keep stdin and stdout.

An argument that is not valid Unicode stops the app with a message (exit code 2).

## File locations

| What | Location |
|---|---|
| Preferences | `%APPDATA%\sketchup-alt\prefs.json` |
| Recovery files (work an agent discards) | `%LOCALAPPDATA%\sketchup-alt\recovery\` |
| Agent API | the named pipe `\\.\pipe\sketchup-alt-<pid>` (see [agent-api.md](agent-api.md)) |

`APPDATA` and `LOCALAPPDATA` count only when set to an absolute path (a drive and a backslash,
or a share). A missing or unusable one falls back to `%USERPROFILE%\AppData\Roaming` or
`%USERPROFILE%\AppData\Local`. When neither gives a folder, the window warns, keeps no
preferences and no recovery files.

Folders the app creates (`sketchup-alt` and `recovery`, and any missing folder above them) get a
protected access list: full control for you, nothing for anyone else, and nothing inherited from
the folder above; files made in them inherit that. Preferences and recovery files are also
created with that access list. A folder that already exists keeps its access list, but one that
is a symbolic link, a junction or another reparse point is refused (preferences are then not
saved, and an agent's command that would discard work fails without discarding it).

## Agent API files (`--api-root`)

Paths are compared without regard to case, verbatim paths (`\\?\C:\...`) are judged as their
plain form, junctions count as links, and drive-relative paths (`C:foo`), paths without a drive
(`\foo`), alternate data streams (`a.skr:x`) and reserved device names (`CON`, `NUL.txt`, ...)
are refused. Details in Access and safety in [agent-api.md](agent-api.md).
