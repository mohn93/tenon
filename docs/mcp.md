# MCP server

`app --mcp` serves the [agent API](agent-api.md) as a Model Context Protocol server on stdin and
stdout, so MCP clients such as Claude Code and Claude Desktop can model with it. It runs either
headless, with its own model, or with `--live SOCKET`, forwarding every call to a running app (a
window or a headless instance) so you watch the agent work. Only MCP messages are written to
stdout; diagnostics go to stderr. The server exits when stdin closes.

## Tools

| Tool | What it does |
|---|---|
| `describe` | The API's description; with `schema`, the JSON Schema of one type or method |
| `transact` | Runs one or more commands; several are one atomic undo step unless `atomic` is false |
| `query` | Reads the model: entities, selection, camera, stats, history and more |
| `pick` | What lies under a view pixel or along a model ray |
| `infer` | What a drawing tool's click would snap to |
| `render_view` | A PNG of the view, without moving the document camera |
| `tool_input` | Drives the active drawing tool with mouse and keyboard events |

With two or more commands, `transact` runs them as a `batch`, so a command may refer to ids an
earlier one made (see References inside a batch in [agent-api.md](agent-api.md)): `"$0:f0"` is
the first face command 0 created, `"$0:e*"` all its edges, `"$0:*"` every edge, face, instance and
guide it created and `"$1:m0"` the material command 1 added. A box in one call:
`{"commands": [{"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [4, 3, 0]},
{"type": "push_pull", "face": "$0:f0", "distance": 2.5}]}`. A single command goes through
`execute`, which takes no references.

Units are meters, Z is up, angles are in degrees. A failed call is a tool result with `isError`
and `{code, message, data?}` as its content, using the API's error codes.

## Build

```bash
cargo build --release -p app
```

The binary is `target/release/app`. Use its absolute path below.

## Headless setup

The server keeps its own model, which starts empty or with a `.skr` file given as the last
argument (a SketchUp `.skp` file opens as a new model, not saved anywhere). Pass `--api-root DIR` (repeatable) to keep every file the agent opens, saves, imports or
exports inside those folders.

Claude Code:

```bash
claude mcp add sketchup-alt -- /path/to/app --mcp --api-root ~/Models
```

Claude Desktop (`~/Library/Application Support/Claude/claude_desktop_config.json`):

```json
{"mcpServers": {"sketchup-alt": {"command": "/path/to/app", "args": ["--mcp", "--api-root", "/Users/you/Models"]}}}
```

Claude Desktop does not expand `~`; use absolute paths there.

## Live setup

Start the window on a fixed socket, then point the MCP server at it:

```bash
/path/to/app --socket ~/.sketchup-alt/main.sock
claude mcp add sketchup-alt-live -- /path/to/app --mcp --live ~/.sketchup-alt/main.sock
```

`~/.sketchup-alt` is created with mode 0700 if missing. The window must be running when the client
starts the server: a socket that cannot be reached stops it with exit code 1. If the window quits
later, every call fails with code `-32003` (connection lost) until the client restarts the server.
`--live` takes no FILE or `--api-root`; open files and set `--api-root` on the window.

## Notes

- Undo steps an agent makes are tagged `agent` (`query` `{"type": "history"}` lists
  `undo_origins`). In a window someone else also edits, undo with `expect` so only the agent's own
  step is undone: `{"type": "undo", "expect": {"label": "add_edge", "origin": "agent"}}`.
- Headless, `--api-root` is the only limit on the files the agent can read and write.
