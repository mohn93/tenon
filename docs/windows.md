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
app.exe --headless --socket tenon-main
```

`--stdio` talks over stdin and stdout. `--socket NAME` serves the agent API on the named pipe
`\\.\pipe\NAME` (a bare name gets the `\\.\pipe\` prefix; a full pipe path is used as given).
The window serves `\\.\pipe\tenon-<pid>` unless told otherwise. Only your user can open the
pipe, remote machines cannot, and a second program cannot take a pipe name that is in use.

### MCP with Claude Code

```bash
claude mcp add tenon -- C:\path\to\app.exe --mcp --api-root C:\Users\you\Models
claude mcp add tenon-live -- C:\path\to\app.exe --mcp --live tenon-main
```

The first runs its own headless model; the second drives a window started with
`app.exe --socket tenon-main`. Claude Desktop reads
`%APPDATA%\Claude\claude_desktop_config.json`; the entry has the same shape as in
[mcp.md](https://github.com/mohn93/sketchup/blob/main/docs/mcp.md), with `command` set to the path of `app.exe` (backslashes doubled in JSON).

## File locations

| What | Location |
|---|---|
| Preferences | `%APPDATA%\tenon\prefs.json` |
| Recovery files (autosaves, work an agent discards, their `.json` notes and the open windows' `session-*.lock` files) | `%LOCALAPPDATA%\tenon\recovery\` |
| Agent API | the named pipe `\\.\pipe\tenon-<pid>` (see [agent-api.md](https://github.com/mohn93/sketchup/blob/main/docs/agent-api.md)) |

`APPDATA` and `LOCALAPPDATA` count only when set to an absolute path (a drive and a backslash,
or a share). A missing or unusable one falls back to `%USERPROFILE%\AppData\Roaming` or
`%USERPROFILE%\AppData\Local`. When neither gives a folder, the window warns, keeps no
preferences and no recovery files.

Folders the app creates (`tenon` and `recovery`, and any missing folder above them) get a
protected access list: full control for you, nothing for anyone else, and nothing inherited from
the folder above; files made in them inherit that. Preferences and recovery files are also
created with that access list. A folder that already exists keeps its access list, but one that
is a symbolic link, a junction or another reparse point is refused (preferences are then not
saved, and an agent's command that would discard work fails without discarding it, and autosave
is off). File > Recover Models... relies on the recovery folder's access list: it skips links
but does not check each file's access list.

## Agent API files (`--api-root`)

Paths are resolved to the names on disk and then compared exactly (a request in another case
still matches once resolved; a case-sensitive folder's sibling differing only in case does not),
verbatim paths (`\\?\C:\...`) are judged as their
plain form, junctions count as links, and drive-relative paths (`C:foo`), paths without a drive
(`\foo`), alternate data streams (`a.tnn:x`) and reserved device names (`CON`, `NUL.txt`, ...)
are refused. A UNC root (`\\server\share\...`) must be given with the server and share names in the
case requests use, and a share and the same folder by drive letter are different roots. A root on
a mapped or `subst` drive is compared as the drive path it was given as, so requests must use
relative paths or the real (UNC) path: a request's drive is not resolved before the comparison,
and one that does not match is refused (it fails closed). Details in
Access and safety in [agent-api.md](https://github.com/mohn93/sketchup/blob/main/docs/agent-api.md).

## The zip from CI

Every push to `main` builds `tenon-windows-x86_64.zip` in the `package-windows` job and
keeps it for 30 days as a workflow artifact (Actions, the run, Artifacts). It holds `app.exe`,
`THIRD_PARTY_NOTICES.md` and this file. It is not signed, so SmartScreen warns on the first
start ("More info", then "Run anyway"); a downloaded zip may also need Properties, then Unblock,
before it is unpacked. The C runtime is linked into `app.exe`, so the zip needs no Visual C++
redistributable.

See [packaging.md](packaging.md) for the installer (MSI) built by CI.

## Known gaps

- The Windows build has been run only by CI (a `windows-2022` runner with no GPU, so rendering
  tests use the software adapter or skip), not yet on a real machine with a graphics card.
- Under `--api-root`, folders on the way to a file are followed (an OneDrive folder is fine),
  but writing over a file that is a reparse point, such as a cloud file placeholder, is refused
  as a link; the error names symbolic links and other reparse points.
- Long paths: the app does not opt in to paths past 260 characters; such paths fail with the
  system's error.
