# Web build

sketchup-alt runs in a browser tab: the `web` crate is the same window app (`app::App`) drawing
to a canvas through WebGPU, or WebGL2 where WebGPU is missing.

## Build

Needs the wasm target and the wasm-bindgen CLI at the version the workspace pins:

```
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129
scripts/build-web.sh
```

The script builds `web` for `wasm32-unknown-unknown` with the size-optimised `web` profile
(`[profile.web]` in the root `Cargo.toml`: opt-level s, LTO, one codegen unit), runs `wasm-bindgen
--target web --no-typescript --out-dir web/dist` and copies `web/index.html`. `web/dist`
(git-ignored) then holds `index.html`, `web.js` and `web_bg.wasm`. `.cargo/config.toml` sets the
`getrandom` backend the wasm build needs.

## Serve

```
scripts/serve-web.sh        # http://localhost:8080, PORT=9000 to change it
```

A small Python `http.server` that serves `.wasm` as `application/wasm`. Any static server that
does so works; no special headers are needed.

CI runs `cargo check --target wasm32-unknown-unknown -p web` on Linux x86_64. The page itself is
tested by hand (open, draw, save, reopen).

## What works

- The whole editor: drawing and editing tools, groups and components, materials, tags, scenes,
  sections, text, dimensions, the dock panels, undo and redo, fly mode.
- Open: File > Open takes a `.skr`, or a `.skp` (opened as a new, unsaved model). Where the
  browser has the File System Access API (Chrome, Edge) the file is picked with its picker and
  Save writes back to the same `.skr`; elsewhere an `<input type=file>` is shown.
- Save downloads `<name>.skr` (or writes to the kept file). Save As asks with the browser's
  picker when it has one, else downloads. A `.skp` opened or imported is saved as `<name>.skr`.
- Import (OBJ, STL, SketchUp) and Export (the format last used; an OBJ with materials downloads
  its `.mtl` and textures beside it).
- Preferences live in `localStorage` under `sketchup-alt-prefs` (the JSON of `prefs.json`).
- The unsaved-changes question is an in-app dialog. Help > About says what is off.
- Cut, Copy and Paste of components between tabs go through text on the clipboard.
- The canvas fills the tab and follows its size.
- Ctrl (Cmd on macOS) chords the app uses are kept from the browser on the canvas; Ctrl+C, X and
  V, the function keys and other browser shortcuts are left alone. Browsers do not let a page
  block Ctrl/Cmd+N, W and T (new window, close tab, new tab), so those keep their browser
  meaning. Closing or reloading the tab with unsaved changes asks for confirmation.
- Paste (Ctrl/Cmd+V) acts on the browser's paste event, so it always pastes what is on the
  clipboard now, into a focused text field or as a clip into the model.

## What is off

- Quit (File > Quit, Ctrl+Q): the tab is closed by the browser.
- Library tab and Save to Library, autosave and Recover Models, Open Recent, Print and Export
  PDF, the agent socket and MCP, `render_view` (no readback on the web).
- Light shadows when the browser falls back to WebGL2 (no cube-array textures): the status line
  and Help > About say so.
- Textures from a file (Paint, Edit Texture > Open) say they cannot be loaded from files here.
- Export has no format chooser: it uses the last format.
- A lost GPU device needs a page reload.

## Clipboard note

A page cannot read the clipboard whenever it likes. Paste is driven by the paste event, which
carries the text; the Edit > Paste menu item (no event) uses the text last copied in the app or
pasted into the page, refreshed when the tab regains focus where the browser lets it read the
clipboard. Copy and Cut write the system clipboard where the browser allows it.
