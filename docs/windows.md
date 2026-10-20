# Windows

## Build

Install Rust with [rustup](https://rustup.rs) (the version in `.github/workflows/ci.yml`; the
default `x86_64-pc-windows-msvc` host) and the Visual Studio Build Tools with the "Desktop
development with C++" workload (the MSVC linker and the Windows SDK).

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

### Headless

```bash
app.exe --headless --stdio
app.exe --headless --socket sketchup-alt-main
```

`--stdio` talks over stdin and stdout. `--socket NAME` serves the agent API on the named pipe
`\\.\pipe\NAME` (a bare name gets the `\\.\pipe\` prefix; a full pipe path is used as given).
The window serves `\\.\pipe\sketchup-alt-<pid>` unless told otherwise. Only your user can open the
pipe, remote machines cannot, and a second program cannot take a pipe name that is in use.

### MCP with Claude Code

```bash
claude mcp add sketchup-alt -- C:\path\to\app.exe --mcp --api-root C:\Users\you\Models
claude mcp add sketchup-alt-live -- C:\path\to\app.exe --mcp --live sketchup-alt-main
```

The first runs its own headless model; the second drives a window started with
`app.exe --socket sketchup-alt-main`. Claude Desktop reads
`%APPDATA%\Claude\claude_desktop_config.json`; the entry has the same shape as in
[mcp.md](mcp.md), with `command` set to the path of `app.exe` (backslashes doubled in JSON).

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

## The zip from CI

Every push to `main` builds `sketchup-alt-windows-x86_64.zip` in the `package-windows` job and
keeps it for 30 days as a workflow artifact (Actions, the run, Artifacts). It holds `app.exe`,
`THIRD_PARTY_NOTICES.md` and this file. It is not signed, so SmartScreen warns on the first
start ("More info", then "Run anyway"); a downloaded zip may also need Properties, then Unblock,
before it is unpacked.

## Known gaps

- The Windows build has been run only by CI (a `windows-2022` runner with no GPU, so rendering
  tests use the software adapter or skip), not yet on a real machine with a graphics card.
- Under `--api-root`, a folder that is an OneDrive placeholder (a cloud file not yet downloaded,
  a reparse point) is refused, as other reparse points are.
- Long paths: the app does not opt in to paths past 260 characters; such paths fail with the
  system's error.
