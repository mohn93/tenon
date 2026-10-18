# M11e App Basics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The window has the basics a Mac app is expected to have, plus the UI that was missing:
- File > Open Recent lists the last 10 files.
- The unsaved-changes prompt offers Save, Don't Save and Cancel.
- On macOS the title is the file name, and the close button shows the edited dot.
- An options sheet comes up before an import is read.
- Components can be named when made, and renamed from the Group menu, with F2 or in Entity Info.
- Purge Unused and Field of View... are in the menus.
- A Settings window (Cmd+,) sets the theme, the UI scale and what trackpad scrolling does.
- VoiceOver reads the window, and every icon-only control has a name.

**Architecture:**
- The session's `purge_definitions` reports how many definitions it removed (`Outcome::purged`), and runs in one pass per nesting level.
- The app does the rest:
  - `prefs.rs` gains `ui_scale`, `trackpad` and `import`.
  - The one menu table gains Open Recent (the one dynamic submenu, which `native_menu` rebuilds when the list changes), Rename Component..., Purge Unused, Field of View... and Settings....
  - The shortcut table gains Cmd+, and F2.
  - A new `ui/sheets.rs` draws one modal sheet at a time (`egui::Modal`): import options, a component name, a rename, a field of view. `State` holds which sheet is open.
  - A new `ui/settings.rs` draws the non-modal Settings window.
  - The UI scale is egui's zoom factor. The renderer takes egui's pixels per point.
  - `navigation.rs` gains the trackpad mode.
  - AccessKit comes through egui-winit's `accesskit` feature. The tree is built only while a screen reader asks for it.
- The API gains nothing new except `purged` in the purge outcome; everything else is window chrome or an existing command.
- The renderer is not touched.

**Tech Stack:** Rust 2024; egui, egui-winit, egui-wgpu, egui_kittest `=0.36.2`; wgpu `=30.0.1`; egui-phosphor `=0.14.0`; muda `=0.21.2`; rfd `=0.17.2`; winit `=0.30.13`. One new feature flag, egui-winit's `accesskit`, and no new direct dependency.

**Spec:** `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md`, "Interface milestones", M11e (and M11b's note that Settings..., Open Recent, Rename Component..., Purge Unused and Field of View... wait for M11e).

The previous plans define what this one consumes:
- `docs/superpowers/plans/2026-10-17-m11d-interaction.md`: `Prefs::recent`, `prefs::remember_recent`, `UiAction::OpenRecent`, the start card.
- `2026-10-17-m11c-viewport-feedback.md`: `Renderer::set_pixels_per_point`, the HUD, the `covered` bit.
- `2026-10-17-m11b-layout.md`: the shortcut table, `menus::menu_bar`, `native_menu`, `shortcuts::window_command`, `native_enabled`, `native_event_acts`.
- `2026-10-16-m11a-design-foundation.md`: theme tokens, prefs, the widget kit and the kittest harness.

The design audits (`ia.md` F4, F6, F9 and P4, P7; `engineering.md` items 5 and E7, E9) only explain where values came from. They live in `/private/tmp/claude-501/-Users-mohn93-Desktop-side-projects-sketchup/7b020704-4344-4767-b2d2-50b34442d2ad/scratchpad/design/`. The executor reads the spec bullet and this plan.

## Global Constraints

- No emoji anywhere (code, comments, docs, commit messages). Keep comments succinct and in the style of the surrounding code.
- Before any cargo command, run `export PATH="$HOME/.cargo/bin:$PATH"`.
- After every task, these must pass:
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
- Test count:
  - Task 1 records the count at the start as N0, with `cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'`.
  - No task may leave fewer tests. A test replaced by its migrated twin keeps the count. A deleted test needs a replacement in the same task.
  - Before trusting a green run, grep the whole output for `FAILED` and `failed;`.
  - Concurrent cargo builds in this checkout make the suite take 15 to 25 minutes (lock contention). That is not a slow test.
- Cargo test filters: cargo takes one filter before `--`. Give several after it, for example `cargo test -p app -- recent_ unsaved_ title`.
- Pinned versions: egui, egui-winit, egui-wgpu and egui_kittest `=0.36.2`; wgpu `=30.0.1`; egui-phosphor `=0.14.0`; muda `=0.21.2`; rfd `=0.17.2`; winit `=0.30.13`. The only dependency change is egui-winit's `accesskit` feature (Task 9). No new direct dependency.
- Values from the spec:
  - Open Recent lists the last 10 files from prefs (`prefs::RECENT_LEN`).
  - The unsaved prompt reads `Do you want to save changes to "<name>"?` with Save, Don't Save and Cancel.
  - On macOS the title is the file name, and the window shows the edited dot instead of `*`.
  - Import options, set before the file is read: units, up axis, import as (group, component or loose) and merge coplanar faces. The last values are remembered.
  - Rename Component... renames a definition (`rename_definition`), as do F2 on a selected Outliner row and the Entity Info name field. Make Component asks for a name.
  - Purge Unused runs `purge_definitions` and reports the count.
  - Field of View... sets the camera's field of view (`set_camera` `fov_y_deg`).
  - Settings (Cmd+,) has two tabs. Appearance: theme and UI scale (90, 100, 110 or 125%). Input: trackpad scroll orbits or zooms. Both are saved in prefs.
  - AccessKit is on, so VoiceOver reads the window's controls. Every icon-only control has a label.
- Shortcuts:
  - Every new binding goes in `shortcuts::TABLE` and nowhere else.
  - On macOS, Cmd chords are native-menu-only (`shortcuts::window_command` returns None for them). Plain keys (F2 among them) are never native accelerators.
  - `with_native_menus_the_key_path_leaves_cmd_chords_to_the_menu` and `every_cmd_chord_is_a_native_menu_accelerator` must keep passing: they are the no-double-fire proof.
- Idle stays at 0% CPU: no new `request_repaint`, `request_redraw` or `repaint_at` that is not caused by input. Sheets and the Settings window have no animation of their own. AccessKit adds no per-frame work while no screen reader is attached (Task 9).
- Icons draw only in the "phosphor" family (`fonts::icons()`), through `widgets::icon_button`. Each new icon constant goes in `icons::all()`.
- egui 0.36.2 gotchas:
  - A selected button reports AccessKit `toggled` (use `ui::tests::is_on`).
  - A tooltip repeats its widget's name, so query before hovering.
  - `Window::anchor` makes a window immovable.
  - `ui::draw` appends actions on every pass, so anything applied from an action must be harmless twice. Each confirm action takes the open sheet (`State::sheet.take()`), so a second copy finds none and does nothing.
- Agent API parity: rename and purge are `rename_definition` and `purge_definitions`. Import options are `import`'s `scale`, `up`, `as` and `merge_coplanar`. Field of view is `set_camera` `fov_y_deg`. Settings, recent files, the sheets and the title are window chrome. `docs/agent-api.md` is updated in Task 10.
- The renderer (`crates/render`) is not touched. CLASSIC renders stay byte-identical. Task 11 checks that `git diff --stat <base>..HEAD -- crates/render` is empty. Only if it is not does Task 11 run the two-worktree golden `cmp`.
- Prefs are window chrome. The API and headless mode never read or write them, and tests never write under `~/.sketchup-alt` (they use `std::env::temp_dir()` folders, as `prefs.rs`'s tests do).
- Do not launch the app window in tests (no `cargo run` of the GUI). rfd dialogs and the native menu cannot be driven in tests: their logic is pure functions, tested on their own.
- Git:
  - Work on `dev`. Other agents may share the checkout, so run `git status` and `git diff` first and stage only the files you changed. Never `git add -A` or `git add .`.
  - Short plain commit messages, no attribution lines. Never push or merge.
- Clippy denies dead code in the `app` binary, so an item lands in the task that first uses it. Public items of the library crates (`kernel`, `session`, `tools`, `api`) may land before their first user.

## Review Focus

- **A prefs file from M11d, or one with odd values.**
  - A file without `ui_scale`, `trackpad` or `import` keeps its theme, dock and recent files and gets the defaults for the rest.
  - `"ui_scale": 300` loads as 100. A trackpad value that is not `"orbit"` or `"zoom"` makes the file invalid, which gives the defaults and a warning, as a bad theme does.
  - Task 4 test: `m11d_prefs_load_with_defaults_for_the_new_keys`. Task 7 test: `an_unknown_ui_scale_loads_as_100`. Task 8 test: `a_bad_trackpad_value_is_an_invalid_file`.
- **125% UI scale in the smallest window.**
  - The panels must still fit, so the window's minimum size grows with the scale: at 125% egui still sees at least 960 x 600 points.
  - Task 7 tests: `the_minimum_window_grows_with_the_ui_scale`, `physical_viewport_uses_the_zoomed_pixels_per_point`.
- **Keys and menu items while a sheet is open.**
  - Typing "r" in the import sheet must not switch to Rectangle. Cmd+O must not open a file dialog over the sheet. Quit still works.
  - Task 4 tests: `keys_wait_while_a_sheet_is_open`, `only_quit_works_while_a_sheet_is_open`.
- **The model changes under an open sheet** (an agent purges the definition being renamed, or clears the selection a Make Component sheet was opened for).
  - Confirming must not panic or act on something else. It says why in the status bar.
  - Task 5 tests: `a_rename_whose_definition_is_gone_says_so`, `naming_a_component_with_nothing_selected_says_so`.
- **The unsaved prompt closed without a button** (Escape, the alert dismissed), or a backend that answers Yes or No for custom buttons.
  - Escape and a dismissed alert cancel. Yes saves and No discards.
  - Task 2 test: `unsaved_answers_map_to_save_dont_save_and_cancel`.

## Spec notes (rulings made in this plan)

- **AccessKit dependencies.**
  - The only change is `egui-winit = { workspace = true, features = ["accesskit"] }` in `crates/app/Cargo.toml`.
  - egui-winit 0.36.2 re-exports the adapter crate (`egui_winit::accesskit_winit`), so the app names it through that path. There is no direct dependency, so nothing new is pinned with `=` in `Cargo.toml`. `Cargo.lock` pins what the feature brings.
  - The versions it brings (checked against crates.io on 2026-10-18):
    - egui-winit asks for `accesskit_winit` `0.32.0`, which resolves to 0.32.2 (Apache-2.0, about 35 KB of source).
    - On macOS that brings `accesskit_macos` `^0.26.0`, which resolves to 0.26.3 (MIT OR Apache-2.0). That depends on `accesskit_consumer` `^0.38.0` (MIT OR Apache-2.0), `objc2` 0.5 and `objc2-app-kit`/`objc2-foundation` 0.2, all already in the lock (winit uses them).
    - `accesskit` 0.24.1 is already in the lock (egui uses it).
    - `accesskit_unix` and `accesskit_windows` are target-only. They enter the lock but are not built on macOS.
  - Test builds carry two `accesskit_consumer` versions: 0.35.0 for egui_kittest and 0.38.0 for the adapter. The release binary has one. Ruled acceptable; do not downgrade the adapter to merge them.
  - No other dependency is added.
- **AccessKit runtime.**
  - `accesskit_winit` wants its adapter made before the window is first shown. Confirm this in `~/.cargo/registry/src/*/accesskit_winit-0.32.2/src/lib.rs` once the build fetches it. So:
    - `window_attributes()` adds `.with_visible(false)`;
    - `App::resumed` calls `Gui::init_accesskit` right after `State::new`, then `window.set_visible(true)`.
  - The adapter sends `AppEvent::AccessKit(accesskit_winit::Event)` through the event loop proxy. `App` keeps one `proxy: EventLoopProxy<AppEvent>` on every system; the macOS-only `menu_proxy` is renamed to it.
  - egui builds its AccessKit tree only while `Context::enable_accesskit` is set, and the app sets it only when the adapter reports `InitialTreeRequested`. `AccessibilityDeactivated` disables it again. `ActionRequested` goes to `egui_winit::State::on_accesskit_action_request`, followed by a redraw.
  - So without a screen reader, no node is built and no tree update is made per frame. The adapter's `process_event` runs per window event, not per frame. Task 9 pins this with a test on a plain `egui::Context`.
  - GPU recreation (`recreate_gpu`) builds a new `Gui`. The new one takes the old one's adapter (`egui_winit::State::accesskit` is a public field) and its screen-reader flag, since the adapter cannot be made again for a visible window.
- **Unsaved prompt** (rfd 0.17.2 has `MessageButtons::YesNoCancelCustom`; on macOS it adds the three buttons in order to an NSAlert, which gives Escape to "Cancel" and Cmd+D to "Don't Save"):
  - Message text (`set_title`): `Do you want to save changes to "<name>"?`, where `<name>` is `file_name(doc.path())` ("Untitled.skr" when never saved).
  - Informative text (`set_description`): `Your changes will be lost if you don't save them.`
  - Buttons: `YesNoCancelCustom("Save", "Don't Save", "Cancel")`, with level Warning. The parent is the window (`set_parent(&*self.window)`), so macOS can show it as a sheet. If the live look shows that `set_parent` makes the synchronous `show()` misbehave, drop it and note it in the commit.
  - Answers: `Custom("Save")` and `Yes` save; `Custom("Don't Save")` and `No` discard; anything else (`Custom("Cancel")`, `Cancel`, a dismissed alert) cancels.
- **Window title.**
  - macOS: the title is `file_name(path)` alone ("house.skr", "Untitled.skr"). `WindowExtMacOS::set_document_edited(modified)` shows the edited dot. It exists in winit 0.30.13 (`src/platform/macos.rs`).
  - Other systems keep `"house.skr* - sketchup-alt"`.
  - Both are set only when they change: `State::title` holds the last title, and a new `State::edited` the last edited flag.
- **Open Recent.**
  - File menu: New, Open..., Open Recent ▸, Save, Save As..., then as before.
  - The submenu lists every entry of `Prefs::recent` (up to 10, newest first), then a separator, then "Clear Menu".
    - Each row reports `UiAction::OpenRecent(path)`. Its label is the file name. When another entry has the same file name, the label adds ` — ` and the parent folder's name ("house.skr — Old"). The egui row's hover text is the full path.
    - "Clear Menu" reports `UiAction::ClearRecent` and is disabled when the list is empty. It empties `Prefs::recent`, refreshes `recent_shown` and saves the prefs.
  - The list is not checked for existence per frame. A missing file fails to open with the open error in the status bar and stays in the list, as in M11d.
  - Native menu: Open Recent is the one submenu whose items change. `NativeMenu` keeps that `Submenu` and the paths it shows. When `sync` finds a different list (`menus::recent_paths`), it removes the submenu's items (`remove_at(0)` until None), drops their ids from `items`, and appends the new entries with `add_entries`. Nothing else in the bar is rebuilt.
  - `the_menu_bar_has_the_same_items_in_every_state` compares ids without the Open Recent rows (`UiAction::OpenRecent(_)`). That submenu is the stated exception.
- **Import options.**
  - File > Import... opens the file dialog first. When a file is chosen, the "Import Options" sheet opens, and the file is read only when the sheet's Import button is pressed. Cancel or Escape reads nothing.
  - `editor::ImportOptions`:
    - `units: ImportUnits` (default Meters) becomes `scale` when not Meters, as today;
    - `up: Option<UpAxis>` (None: "Automatic", OBJ Y and STL Z, as `import`'s default);
    - `place: ImportAs` (default Group; the sheet labels `Context` as "Loose geometry");
    - `merge_coplanar: bool` (default true).
  - The editor keeps one `ImportOptions` for the session; it replaces the `import_units` field. `import_units()` and `set_import_units` stay: the `set_import_units` editor command and `editor.import_units` keep working and change only `units`.
  - The window loads `Prefs::import` into the editor at start, and saves the sheet's values to `Prefs::import` when Import is pressed. An agent's `set_import_units` changes the editor's options only. The next sheet shows them, and saving the sheet then writes them.
  - File > Import Units leaves the File menu: units are chosen in the sheet. `UiAction::SetImportUnits` and `View::import_units` go.
  - The status message after an import is today's. A loose import (no instance) reads the same.
- **Sheets** (`ui::Sheet`, drawn by `ui::sheets::sheet`):
  - Each is an `egui::Modal` with id `"sheet"`, 340 pt wide, with a `section_header` title, its fields, then the confirm and Cancel buttons at the right. The confirm button is the default: Enter in a text field presses it.
  - The modal's area does not fade in (`Modal::default_area(id).fade_in(false)`), so a sheet settles in one frame.
  - Escape, Cancel, or a click on the backdrop (`ModalResponse::should_close`) reports `UiAction::CloseSheet`.
  - At most one sheet is open (`State::sheet: Option<Sheet>`). Opening one replaces any other.
  - While a sheet is open:
    - the window's key path gives no key to the editor or the shortcut table (egui still gets every key, so the sheet's fields work);
    - presses and scrolls do not reach the view, because the modal's backdrop is a layer above it (`Gui::wants_pointer_at`);
    - native menu items are disabled except Quit, and a raced native event other than Quit does nothing.
  - The agent API keeps running. Sheets are window chrome: no editor state, no `editor.dialog` value. Unlike the rfd dialogs, they do not block requests.
- **Component names.**
  - Make Component (Group menu, now labelled "Make Component...", the context bar button and Shift+G in the window) opens the "Make Component" sheet when something is selected. Its Name field holds `Editor::next_component_name()` ("Component N", the first free N, as today) with all of it selected, and its button is Create. With nothing selected, it shows today's error and no sheet.
  - Shift+G reaches the sheet in the window because the key path asks `Editor::shortcut_for(key)` first. When that is `Bound::MakeComponent`, it applies `UiAction::MakeComponent` instead of `Editor::key`. `tool_input` keys and headless mode keep today's automatic "Component N".
  - Rename Component... (Group menu, F2) works on `Editor::rename_target()`: exactly one selected group or component in the open context. Otherwise the item is disabled and F2 does nothing.
    - The sheet is titled "Rename Component" or "Rename Group", with the current name filled in and selected, and the button Rename.
    - F2 is a shortcut-table row (`Bound::RenameComponent`, `Key::F2`, group "Group"). It acts on the window's plain-key path and is not a native accelerator.
  - A component name must not be blank (`trim().is_empty()`): Create and Rename are then disabled and Enter does nothing. A group may be renamed to blank, which shows as `Group`, as `rename_definition` allows.
  - Entity Info: with one group or component selected, the first line ("1 component" or "1 group") is followed by a "Name" text field holding the definition's name, then the other lines. The separate "Name: X" line goes.
    - The field commits on Enter or focus lost when the text changed, reporting `UiAction::RenameDefinition(definition, text)`. Escape discards, as the material name field does.
    - A blank name for a component is not sent; the field goes back to the name.
  - Messages (info): "Renamed <old> to <new>"; a session refusal shows its error message.
- **Purge Unused** (Group menu, after a separator below Close Group, with Rename Component...):
  - The menu item is always enabled: knowing whether anything is unused would mean counting instances every frame.
  - `Outcome::purged: Option<usize>` (new, absent for other commands) is how many definitions `purge_definitions` removed, nested ones included.
  - Messages (info): "Purged 1 unused definition", "Purged N unused definitions", "Nothing to purge".
  - `Scene::purge` counts instances once per pass (one walk over every mesh into a map from definition to count), removes every definition with none, and repeats until a pass removes nothing. That is one pass per nesting level instead of a scan per removal. The order of the removal steps, and so undo, is unchanged in effect.
- **Field of View...** (Camera menu, after Parallel Projection):
  - Enabled only in a perspective view. In a parallel view the field of view only sizes the view, so changing it would zoom.
  - The sheet "Field of View" has a whole-degree drag field labelled "Field of view", 1 to 170 (`set_camera`'s range), with suffix "°", holding the camera's value rounded. The button is Apply.
  - `Editor::set_field_of_view(degrees)` runs `SetCamera { eye, target, fov_y_deg: Some(degrees), up: Some(camera.up), projection: None }` from the document camera. Passing `up` keeps a top or bottom view's screen direction.
- **Settings.**
  - Opened by Settings... (`Bound::Settings`, Cmd+, on macOS, Ctrl+, elsewhere, `Key::Comma`).
    - On macOS it is in the app menu: About sketchup-alt, a separator, Settings..., a separator, Hide, Hide Others, Show All, a separator, Quit sketchup-alt.
    - Elsewhere it is the last Edit item, after a separator.
  - A non-modal `egui::Window` titled "Settings": not collapsible, not resizable, closable, centred over the window on first open, 380 pt wide. `UiAction::ShowSettings(bool)` opens and closes it.
  - Two selectable tab buttons at the top, "Appearance" and "Input". The tab lives in egui memory, default Appearance.
  - Appearance:
    - "Theme" with System, Dark and Light as selectable buttons, reporting `UiAction::SetTheme`. View > Theme stays too.
    - "UI scale" with 90%, 100%, 110% and 125% as selectable buttons, reporting `UiAction::SetUiScale`.
  - Input: "Trackpad scroll" with two radio buttons, "Orbits (Shift pans, Cmd zooms)" and "Zooms (Shift pans, Cmd orbits)", reporting `UiAction::SetTrackpad`. Off macOS the labels say Ctrl for Cmd.
  - Each change applies at once and saves the prefs.
- **UI scale.**
  - `theme::UiScale` has the variants `P90`, `P100` (default), `P110` and `P125`. It is stored as the integer percent (`"ui_scale": 110`). A number that is not one of the four loads as 100.
  - It is applied with `ctx.set_zoom_factor(scale.factor())`: at start in `Gui::new`, and on change through `Gui::set_ui_scale`.
  - egui's own keyboard zoom is turned off (`Options::zoom_with_keyboard = false`, in `theme::install`). Cmd+=, Cmd+- and Cmd+0 do nothing: one persisted value with one control, instead of an unsaved second zoom that would disagree with Settings. They are not bound in this milestone. The editor's `-` and `=` fly-speed keys are unaffected, since they are plain keys.
  - The renderer follows egui: `Renderer::set_pixels_per_point` gets the frame's `pixels_per_point`, which is the window scale factor times the zoom factor, carried in `GuiFrame::pixels_per_point`. The renderer's wide hover edges and axes are in points, as are the HUD's egui strokes, so all of them grow together at 125%. render_view keeps 1.
  - The view rectangle is converted to physical pixels with that same value, as today.
  - The editor's pick radius and drag threshold stay on the window scale factor: they are pointer tolerances, not UI size.
  - The minimum window size is 960 x 600 points times the factor in logical pixels (1200 x 750 at 125%), so the layout always gets at least 960 x 600 points. It is set at start and on every change.
- **Trackpad** (`navigation::TrackpadScroll`, `Orbit` by default, stored as `"orbit"` or `"zoom"`):
  - Orbit: today's behaviour. Two-finger scroll orbits, Shift pans, Cmd or Ctrl zooms.
  - Zoom: two-finger scroll zooms toward the cursor, at the speed Cmd+scroll zooms today. Shift pans. Cmd or Ctrl orbits.
  - The mouse wheel (`Scroll::Lines`) and pinch always zoom. Fly mode's scrolling is unchanged.
  - `Editor::set_trackpad` passes it to `Navigation`. The window sets it from prefs at start and on change. Headless and the API keep Orbit. `tool_input` has no scroll event, so agents are unaffected.
- **Prefs** gain three keys, each defaulted when missing:
  - `ui_scale` (integer percent, default 100);
  - `trackpad` (`"orbit"`, default);
  - `import` (`{"units": "meters", "as": "group", "merge_coplanar": true}`, plus `"up": "y"` or `"z"` when not automatic), using the session enums' JSON names.
  - Defaults are not left out of the file, except `import.up` when automatic.
- **Deferred** (from the spec): the Offset, Scale and Follow Me tools; cut, copy and paste between models; display units and precision; custom cursor images; number keys for standard views. Also deferred by this plan: Close (Cmd+W), Revert to Saved and a Recovered Files menu; UI zoom on Cmd+= and Cmd+-; the macOS proxy icon in the title.
- **Tests.**
  - Session behaviour is tested in `crates/session/tests`.
  - Editor behaviour is tested in `editor.rs` and `host/tool_input_tests.rs`.
  - Panels, sheets and the Settings window are tested with the kittest harness (`crates/app/src/ui/tests/`).
  - The title, the prompt, the key gate and the minimum size are pure functions in `app.rs`, tested there. The native menu's dynamic submenu is tested through `menus::recent_paths`.
  - The final task looks at the real window through a `.app` bundle wrapper.

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/kernel/src/scene.rs`; `crates/session/src/command.rs`, `document.rs`; `crates/session/tests/groups.rs` (or wherever purge is tested now) | `Outcome::purged`, quick purge | 1 |
| `crates/app/src/app.rs` | title and edited dot, unsaved prompt | 2 |
| `crates/app/src/menus.rs`, `native_menu.rs`, `ui/mod.rs`, `app.rs`, `ui/tests/menus.rs` | Open Recent, Clear Menu | 3 |
| `crates/app/src/editor.rs`, `prefs.rs`, `menus.rs`, `ui/mod.rs`, `ui/sheets.rs` (new), `app.rs`, `host.rs`, `ui/tests/mod.rs`, `ui/tests/sheets.rs` (new), `ui/tests/menus.rs` | import options, the sheet machinery | 4 |
| `crates/app/src/editor.rs`, `shortcuts.rs`, `menus.rs`, `ui/mod.rs`, `ui/sheets.rs`, `ui/right.rs`, `info.rs`, `app.rs`, `ui/tests/sheets.rs`, `ui/tests/entity.rs`, `ui/tests/menus.rs` | component names, rename | 5 |
| `crates/app/src/editor.rs`, `menus.rs`, `ui/mod.rs`, `ui/sheets.rs`, `app.rs`, tests | Purge Unused, Field of View... | 6 |
| `crates/app/src/ui/theme.rs`, `ui/settings.rs` (new), `ui/mod.rs`, `prefs.rs`, `gui.rs`, `shortcuts.rs`, `native_menu.rs`, `menus.rs`, `app.rs`, `ui/tests/settings.rs` (new) | Settings window, UI scale | 7 |
| `crates/app/src/navigation.rs`, `editor.rs`, `prefs.rs`, `ui/settings.rs`, `app.rs` | trackpad setting | 8 |
| `crates/app/Cargo.toml`, `Cargo.lock`, `crates/app/src/gui.rs`, `app.rs`, `ui/*` (labels), `ui/tests/access.rs` (new) | AccessKit | 9 |
| `docs/agent-api.md` | API docs | 10 |
| `docs/superpowers/roadmap.md` | M11e row | 11 |

---

### Task 1: The purge says how many definitions it removed

**Files:**
- Modify: `crates/kernel/src/scene.rs` (`Scene::purge`)
- Modify: `crates/session/src/command.rs` (`Outcome`), `crates/session/src/document.rs` (a `purged` field beside `positioned`, set in the `PurgeDefinitions` arm, taken where `positioned` is taken: in `execute` and in the batch path)
- Test: the session test file that already tests `purge_definitions` (find it with `grep -rln purge_definitions crates/session/tests`)

**Interfaces:**
- Produces:

```rust
pub struct Outcome {
    // ...
    /// How many definitions a `purge_definitions` removed (nested ones included); absent for
    /// other commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purged: Option<usize>,
}
```

- [ ] **Step 1: Record the baseline**

```bash
cd ~/Desktop/side_projects/sketchup && export PATH="$HOME/.cargo/bin:$PATH"
git status && git log --oneline -1
cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'
```
Expected: a clean tree on `dev` at or after `21f6602`. Write the count down as N0, and the HEAD hash as `<base>`.

- [ ] **Step 2: Write the failing tests**

- `purge_reports_what_it_removed`:
  - Make two components A and B, with B nested inside A's definition, plus a component C whose only instance is erased.
  - Erase A's only instance. `purge_definitions` then gives `purged == Some(3)` (A, the nested B, C), and the `definitions` query lists none of them.
  - One `undo` brings all three back.
  - A second purge gives `Some(0)`.
  - Any other command's outcome serializes without a `purged` key.
- `purging_many_unused_definitions_is_quick`:
  - Make 2,000 components from small rectangles, then erase every instance (one `erase` with all ids).
  - Time `purge_definitions`. It returns `Some(2000)` in under 2 s in a debug build.
  - Leave a comment that the old scan was one instance walk per definition per removal.

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test -p session -- purge_reports purging_many`
Expected: compile error (no `purged`).

- [ ] **Step 4: Implement**

- `Scene::purge` loops:
  - count instances per definition in one walk over every mesh's instances into a `HashMap<DefinitionId, usize>`;
  - remove (`remove_definition`, in id order) every definition with no count;
  - stop when a pass removes nothing.
- The `PurgeDefinitions` arm sets `self.purged = Some(change.steps.len())` from the `SceneChange` it returns. Each step is one `RemoveDefinition`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p session && cargo test -p api`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git status && git diff --stat
git add crates/kernel/src/scene.rs crates/session/src/command.rs crates/session/src/document.rs <the test file>
git commit -m "Session: purge reports the count and runs in one pass per level"
```

---

### Task 2: The title, the edited dot and the Save / Don't Save prompt

**Files:**
- Modify: `crates/app/src/app.rs` (`window_title`, `settle_changes`, the title update in `render`, `State::edited`, tests)

**Interfaces:**
- Produces (private to `app.rs`):

```rust
/// The window title: the file name alone on macOS, where the close button shows unsaved
/// changes; elsewhere "house.skr* - sketchup-alt".
fn window_title(path: Option<&Path>, modified: bool, mac: bool) -> String;
/// `Do you want to save changes to "house.skr"?`
fn unsaved_question(path: Option<&Path>) -> String;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnsavedChoice { Save, DontSave, Cancel }
/// What the unsaved-changes alert's answer asks for.
fn unsaved_choice(answer: &rfd::MessageDialogResult) -> UnsavedChoice;
```

- [ ] **Step 1: Write the failing tests** (`app.rs` tests)

- `titles_name_the_file_and_mark_unsaved_changes` (rewritten, so the count stays):
  - `window_title(None, false, false) == "Untitled.skr - sketchup-alt"`.
  - `window_title(Some("/m/house.skr"), true, false) == "house.skr* - sketchup-alt"`.
  - `window_title(Some("/m/house.skr"), true, true) == "house.skr"`.
  - `window_title(None, true, true) == "Untitled.skr"`.
- `the_unsaved_question_names_the_file`:
  - `unsaved_question(Some("/m/house.skr")) == "Do you want to save changes to \"house.skr\"?"`.
  - With None, the name is `Untitled.skr`.
- `unsaved_answers_map_to_save_dont_save_and_cancel`:
  - `Custom("Save")` and `Yes` give Save.
  - `Custom("Don't Save")` and `No` give DontSave.
  - `Custom("Cancel")`, `Cancel`, `Ok` and `Custom("other")` give Cancel.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app -- titles_name unsaved_`
Expected: compile errors.

- [ ] **Step 3: Implement**

- `settle_changes` builds the alert:
  - `set_level(rfd::MessageLevel::Warning)`;
  - `set_title(unsaved_question(..))`;
  - `set_description("Your changes will be lost if you don't save them.")`;
  - `set_buttons(YesNoCancelCustom("Save".into(), "Don't Save".into(), "Cancel".into()))`;
  - `set_parent(&*self.window)`.
  
  It then matches `unsaved_choice`: Save runs `self.save(false)`, DontSave is true, Cancel is false.
- In `render`:
  - the title is `window_title(path, modified, cfg!(target_os = "macos"))`, set when it differs from `self.title`;
  - on macOS, when `modified != self.edited`, call `winit::platform::macos::WindowExtMacOS::set_document_edited(&*self.window, modified)` and store it.
  - `State::edited: bool` starts false.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/app.rs
git commit -m "Window: Save / Don't Save prompt, file-name title and the macOS edited dot"
```

---

### Task 3: File > Open Recent

**Files:**
- Modify: `crates/app/src/menus.rs` (the File menu, `OPEN_RECENT`, `recent_label`, `recent_paths`), `crates/app/src/native_menu.rs` (the dynamic submenu), `crates/app/src/ui/mod.rs` (`UiAction::ClearRecent`, `View::recent`), `crates/app/src/app.rs` (`ClearRecent`, `View::recent`)
- Test: `crates/app/src/ui/tests/menus.rs`, `crates/app/src/ui/tests/mod.rs` (`Fixture::recent`)

**Interfaces:**
- Produces:

```rust
// menus.rs
pub const OPEN_RECENT: &str = "Open Recent";
/// A recent file's menu label: its file name, with " — <parent folder>" when another entry in
/// `recent` has the same file name.
pub fn recent_label(path: &Path, recent: &[PathBuf]) -> String;
/// The paths the Open Recent submenu lists in `bar`, in order.
#[cfg(any(target_os = "macos", test))]
pub fn recent_paths(bar: &[Menu]) -> Vec<PathBuf>;
// ui/mod.rs
UiAction::ClearRecent, // empty the recent files and keep that in the preferences
View { /* ... */ pub recent: &'a [PathBuf] } // Prefs::recent, newest first
```

- [ ] **Step 1: Write the failing tests** (`ui/tests/menus.rs`)

- `open_recent_lists_the_recent_files_and_clears`:
  - Use a fixture with `recent` = `/a/house.skr`, `/b/shed.skr`.
  - File > "Open Recent ⏵" shows "house.skr" and "shed.skr" in that order. Clicking "shed.skr" reports `OpenRecent("/b/shed.skr")`.
  - "Clear Menu" reports `ClearRecent`.
  - With an empty list, "Clear Menu" is disabled and no file rows exist.
- `recent_labels_tell_same_names_apart`:
  - With `/a/Old/house.skr` and `/a/New/house.skr`, the labels are "house.skr — Old" and "house.skr — New".
  - With `/a/shed.skr` alone, the label is "shed.skr".
- `recent_paths_reads_the_submenu`: `recent_paths(&menu_bar(&view, true))` equals the fixture's list, and is empty for an empty list.
- `the_menu_bar_has_the_same_items_in_every_state`: extend it. The busy fixture also has two recent files, and both id lists leave out `UiAction::OpenRecent(_)` items before comparing.
- `the_file_menu_reports_its_items`: extend it with Open Recent between Open... and Save.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app -- open_recent recent_labels recent_paths same_items file_menu`
Expected: compile errors.

- [ ] **Step 3: Implement**

- `menu_bar` puts `Entry::Submenu { label: OPEN_RECENT, entries }` after Open.... The entries are one `Item::new(recent_label(p, view.recent), UiAction::OpenRecent(p.clone()))` per path, with the full path as tooltip, then `Entry::Separator`, then `Item::new("Clear Menu", UiAction::ClearRecent).enabled(!view.recent.is_empty())`.
- `Item::tooltip` takes `&'static str` today. Change it to `Option<String>` so a path fits, and update `ui::menu_bar::show` and the Fly Mode item.
- `NativeMenu` gains `recent: Option<(Submenu, Vec<PathBuf>)>`, filled by `add_entries` when it meets the `OPEN_RECENT` submenu.
  - `sync` compares `menus::recent_paths(bar)` with the shown list. When they differ, it empties and refills only that submenu, as the Spec notes say, then syncs the rest as today.
- In `app.rs`:
  - `View::recent` is `&self.prefs.recent`.
  - `ClearRecent` clears `self.prefs.recent`, sets `self.recent_shown` to empty, and calls `save_prefs()`. It is harmless twice.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/menus.rs crates/app/src/native_menu.rs crates/app/src/ui/mod.rs crates/app/src/ui/menu_bar.rs crates/app/src/app.rs crates/app/src/ui/tests/menus.rs crates/app/src/ui/tests/mod.rs
git commit -m "File: Open Recent with Clear Menu, native submenu rebuilt on change"
```

---

### Task 4: The import options sheet

**Files:**
- Create: `crates/app/src/ui/sheets.rs`, `crates/app/src/ui/tests/sheets.rs`
- Modify: `crates/app/src/editor.rs` (`ImportOptions`, `import_options`, `set_import_options`, `import_file`), `crates/app/src/prefs.rs` (`Prefs::import`), `crates/app/src/menus.rs` (no Import Units; `native_enabled` and `native_event_acts` take `sheet_open`), `crates/app/src/native_menu.rs` (`sync` passes it), `crates/app/src/ui/mod.rs` (`Sheet`, `View::sheet`, actions; drop `SetImportUnits` and `View::import_units`), `crates/app/src/app.rs` (`State::sheet`, the import flow, the key gate), `crates/app/src/ui/tests/mod.rs`, `ui/tests/menus.rs`

**Interfaces:**
- Produces:

```rust
// editor.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportOptions {
    pub units: ImportUnits,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up: Option<UpAxis>,
    #[serde(rename = "as")]
    pub place: ImportAs,
    pub merge_coplanar: bool,
}
impl Default for ImportOptions { /* Meters, None, Group, true */ }
impl Editor {
    pub fn import_options(&self) -> ImportOptions;
    pub fn set_import_options(&mut self, options: ImportOptions);
    // import_units / set_import_units read and write `units` only.
}
// ui/mod.rs (Sheet lives in ui/sheets.rs, re-exported)
pub enum Sheet {
    Import { path: PathBuf, options: ImportOptions },
}
UiAction::ConfirmImport(ImportOptions),
UiAction::CloseSheet,
View { /* ... */ pub sheet: Option<&'a Sheet> }
// ui/sheets.rs
pub(super) fn sheet(ui: &mut Ui, sheet: &Sheet, actions: &mut Vec<UiAction>);
// menus.rs
pub fn native_enabled(item: &Item, text_focused: bool, sheet_open: bool) -> bool;
pub fn native_event_acts(action: &UiAction, repeat: bool, text_focused: bool, sheet_open: bool) -> bool;
// app.rs (private)
/// Whether a key press goes on to the editor and the shortcut table.
fn keys_reach_editor(text_focused: bool, sheet_open: bool) -> bool;
```

Later tasks add `Sheet` variants and confirm actions to this machinery.

- [ ] **Step 1: Write the failing tests**

`editor.rs` tests:
- `import_uses_the_options`:
  - Write a one-quad OBJ to a temp file, in the y-up style the existing `import_*` editor tests use.
  - Set options `{ units: Centimeters, up: Some(UpAxis::Z), place: ImportAs::Component, merge_coplanar: false }` and import.
  - The selection is one component instance. The face's extent is 0.01 times the file's numbers, and its normal is the file's z (no y-to-z swap).
  - `import_units()` reads Centimeters. `set_import_units(Feet)` changes only `units`.
- `a_loose_import_selects_its_geometry`: `place: Context` gives no instance, its faces are selected, and the status reads "Imported <name>: 1 face".

`prefs.rs` tests:
- `import_options_round_trip`: save then load gives the same options. The JSON has `"as": "component"` and no `"up"` key when it is None.
- `m11d_prefs_load_with_defaults_for_the_new_keys`: a file with only `theme`, `dock` and `recent` loads them, with `import == ImportOptions::default()`.

`ui/tests/sheets.rs`:
- `the_import_sheet_reports_its_options`:
  - Use a fixture with `sheet: Some(Sheet::Import { path: "/m/chair.obj", options: default })`.
  - "chair.obj" is drawn. Choose "Millimeters" in the "Units" combo box, "Z up", "Loose geometry", and untick "Merge coplanar faces".
  - "Import" then reports `ConfirmImport(ImportOptions { units: Millimeters, up: Some(Z), place: Context, merge_coplanar: false })`.
- `the_import_sheet_cancels_on_escape_and_cancel`: Escape reports `CloseSheet`, and so does the "Cancel" button.
- `a_sheet_settles_in_one_frame`: after the first frame, `h.try_run()` is ok and no repaint is requested.
- `a_sheet_keeps_presses_off_the_view`: the layer at the view's centre is not `Order::Background`.

`ui/tests/menus.rs`:
- `checked_menu_items_show_the_state`: replace its Import Units part with a check that File has no "Import Units ⏵" (the count stays).
- `only_quit_works_while_a_sheet_is_open`: for every item of `menu_bar(.., true)`, `native_enabled(i, false, true)` is true only for `UiAction::Quit`. `native_event_acts(&OpenFile, false, false, true)` is false, and for `Quit` it is true.
- Update the existing `native_enabled` and `native_event_acts` call sites with `false` for the new argument.

`app.rs` tests:
- `keys_wait_while_a_sheet_is_open`: `keys_reach_editor(false, false)` is true; `(true, false)`, `(false, true)` and `(true, true)` are false.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app -- import_ m11d_prefs sheet keys_wait only_quit checked_menu`
Expected: compile errors.

- [ ] **Step 3: Implement**

- `Editor`:
  - `import: ImportOptions` replaces `import_units`.
  - `import_file` passes `place`, `up`, `merge_coplanar: Some(options.merge_coplanar)` and today's `scale` rule.
  - `host.rs` keeps `SetImportUnits` working through `set_import_units`.
- `Prefs::import: ImportOptions` (`#[serde(default)]` through the struct).
- `ui::sheets::sheet` draws an `egui::Modal::new(Id::new("sheet"))` with `.area(Modal::default_area(Id::new("sheet")).fade_in(false))`, 340 pt wide. For `Sheet::Import`:
  - The `section_header` "Import Options", then the file name in secondary text.
  - "Units": a `ComboBox` labelled "Units" listing `ImportUnits::ALL` by `label()`.
  - "Up axis": radio buttons "Automatic", "Y up", "Z up", with the small hint "Automatic: Y for OBJ, Z for STL".
  - "Import as": radio buttons "Group", "Component", "Loose geometry".
  - The checkbox "Merge coplanar faces".
  - "Cancel" and "Import" buttons at the right.
  - The values being edited live in egui memory under the sheet's id, starting from the `Sheet`'s options. Clear them when the sheet closes, so it opens fresh next time.
  - `should_close()` reports `CloseSheet`.
- In `ui::draw`, call `sheets::sheet` last, after the gizmo, when `view.sheet` is Some.
- `menu_bar` drops the Import Units submenu. `native_enabled` and `native_event_acts` return false for everything but `UiAction::Quit` while `sheet_open`.
- In `app.rs`:
  - `State::sheet: Option<Sheet>`.
  - `import()` sets `self.sheet = Some(Sheet::Import { path, options: self.editor.import_options() })` after the file dialog.
  - `ConfirmImport(o)`: when `self.sheet.take()` is an Import sheet, `set_import_options(o)`, `self.prefs.import = o`, `save_prefs()`, then `import_file(&path)`.
  - `CloseSheet` sets `self.sheet = None`.
  - `State::new` calls `editor.set_import_options(prefs.import)`.
  - In `window_event`, the `KeyboardInput` guard becomes `if !keys_reach_editor(s.gui.wants_keyboard(), s.sheet.is_some())`.
  - `menu_event` and the native `sync` pass `s.sheet.is_some()`.
- `Fixture` gains `sheet: Option<Sheet>` and loses `import_units`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p app && cargo test -p api`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/ui/sheets.rs crates/app/src/ui/tests/sheets.rs crates/app/src/editor.rs crates/app/src/prefs.rs crates/app/src/menus.rs crates/app/src/native_menu.rs crates/app/src/ui/mod.rs crates/app/src/app.rs crates/app/src/host.rs crates/app/src/ui/tests/mod.rs crates/app/src/ui/tests/menus.rs
git commit -m "Import: an options sheet before the file is read, remembered in prefs"
```

---

### Task 5: Naming and renaming components

**Files:**
- Modify: `crates/app/src/editor.rs` (`next_component_name`, `make_component_named`, `RenameTarget`, `rename_target`, `rename_definition`, `shortcut_for`), `crates/app/src/shortcuts.rs` (`Bound::RenameComponent`, `Key::F2`), `crates/app/src/menus.rs` (the Group menu), `crates/app/src/ui/mod.rs` (`Sheet::{NameComponent, Rename}`, actions, `View::rename`), `crates/app/src/ui/sheets.rs`, `crates/app/src/ui/right.rs` (the Name field), `crates/app/src/info.rs` (no "Name:" line), `crates/app/src/app.rs` (Shift+G, F2, the actions)
- Test: `editor.rs` tests, `ui/tests/sheets.rs`, `ui/tests/entity.rs`, `ui/tests/menus.rs`, `shortcuts.rs` tests, `info.rs` tests

**Interfaces:**
- Produces:

```rust
// editor.rs
#[derive(Clone, Debug, PartialEq)]
pub struct RenameTarget { pub definition: DefinitionId, pub kind: DefinitionKind, pub name: String }
impl Editor {
    /// "Component N" with the first free N.
    pub fn next_component_name(&self) -> String;
    /// Makes a component of the selection named `name`. With nothing selected it says so.
    pub fn make_component_named(&mut self, name: &str) -> bool;
    /// The one selected group or component of the open context, if exactly one is selected.
    pub fn rename_target(&self) -> Option<RenameTarget>;
    /// Runs `rename_definition`; says "Renamed <old> to <new>", or the refusal.
    pub fn rename_definition(&mut self, definition: DefinitionId, name: &str) -> bool;
    /// What `key` would run from the shortcut table now: not flying, nothing typed, a key with
    /// a plain binding. None otherwise.
    pub fn shortcut_for(&self, key: EditorKey) -> Option<Bound>;
}
// shortcuts.rs
Key::F2, Bound::RenameComponent // label "Rename Component", group "Group", chord F2
// ui
Sheet::NameComponent { suggested: String },
Sheet::Rename { target: RenameTarget },
UiAction::RenameComponent,                      // open the rename sheet for the target
UiAction::RenameDefinition(DefinitionId, String), // Entity Info's Name field
UiAction::ConfirmName(String),
View { /* ... */ pub rename: Option<&'a RenameTarget> }
```

- [ ] **Step 1: Write the failing tests**

`editor.rs` tests:
- `make_component_named_uses_the_name`:
  - Draw a rectangle, select it, `make_component_named("Chair")`.
  - The selected instance's definition is a component named "Chair".
  - `next_component_name()` is "Component 1" before and still "Component 1" after.
- `naming_a_component_with_nothing_selected_says_so`: `make_component_named("Chair")` with nothing selected leaves the model unchanged and shows the error "Select geometry to make a component".
- `rename_target_is_the_one_selected_instance`:
  - None with nothing selected, with two instances, or with an instance and an edge.
  - Some with exactly one group (kind Group, name "") or one component.
- `rename_definition_renames_and_undoes`: renaming "Component 1" to "Table" shows the info "Renamed Component 1 to Table". `undo` restores the name. A blank name for a component shows the session's refusal "a component needs a name".
- `a_rename_whose_definition_is_gone_says_so`: purge the definition, then `rename_definition(old_id, "X")` returns without panic and shows an error containing "no definition".
- `shift_g_is_the_make_component_shortcut`: `shortcut_for(EditorKey::Char('G')) == Some(Bound::MakeComponent)` on a fresh editor. It is None once "2" has been typed (the G would be text) and None while flying.

`shortcuts.rs` tests:
- `f2_renames`:
  - `chord(Bound::RenameComponent)` is F2 without modifiers; its text is "F2" on both systems.
  - `plain` never returns it (F2 is not a character).
  - The existing tests (unique chords and labels, listed group) cover the new row.

`ui/tests/sheets.rs`:
- `make_component_asks_for_a_name`:
  - Use a fixture with `sheet: Some(Sheet::NameComponent { suggested: "Component 3" })`.
  - The "Name" field holds "Component 3". Typing "Chair" (replacing it) then Enter reports `ConfirmName("Chair")`.
  - Clearing the field to spaces disables "Create", and Enter then reports nothing.
- `the_rename_sheet_names_the_kind`: a group target titles the sheet "Rename Group" and a component "Rename Component". A blank group name is accepted, and "Rename" reports `ConfirmName("")`.

`ui/tests/entity.rs`:
- `entity_info_renames_a_selected_component`:
  - Use a fixture with `rename: Some(target "Chair", component)` and info lines `["1 component"]`.
  - The "Name" field holds "Chair". Typing "Stool" and Enter reports `RenameDefinition(d, "Stool")`.
  - Escape after typing reports nothing.
  - A blank component name reports nothing.

`info.rs` tests: the single-component case no longer has a "Name: X" line. Update its existing assertion rather than adding a test.

`ui/tests/menus.rs`:
- `the_group_menu_renames_and_purges`: the Group menu reads Make Group, "Make Component...", Explode, Close Group, separator, "Rename Component..." (shows "F2"; enabled only with `rename` Some, reporting `RenameComponent`).
  - Task 6 adds Purge Unused to this test.
  - Existing menu tests that click "Make Component" in the Group menu now click "Make Component...". The context bar button keeps its name "Make Component".

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app -- component rename f2_renames shift_g group_menu`
Expected: compile errors.

- [ ] **Step 3: Implement**

- `make_component` keeps its behaviour, through `next_component_name()`.
- `UiAction::MakeComponent` in `app.rs`:
  - with a selection, `self.sheet = Some(Sheet::NameComponent { suggested: self.editor.next_component_name() })`;
  - otherwise `self.editor.make_component()`, which shows the error.
- `ConfirmName(name)` takes the sheet:
  - `NameComponent` runs `make_component_named(&name)`;
  - `Rename { target }` runs `rename_definition(target.definition, &name)`.
- `RenameComponent` opens `Sheet::Rename` when `rename_target()` is Some.
- `RenameDefinition(d, name)` calls `rename_definition` directly.
- Key path (non-command branch), before `editor.key`:
  - `Key::Named(NamedKey::F2)` with no modifiers looks up `shortcuts::lookup(Chord { command: false, shift: false, key: Key::F2 })` and applies its action.
  - A key whose `editor.shortcut_for(k) == Some(Bound::MakeComponent)` applies `UiAction::MakeComponent` instead of `editor.key(k)`.
- `Bound::RenameComponent.action()` is `UiAction::RenameComponent`. `editor.run_shortcut` treats it as not a plain key (`_ => false`). `native_menu::accelerator` already returns None for it.
- Sheets:
  - `NameComponent` and `Rename` share `name_sheet(ui, title, button, initial, allow_blank, actions)`.
  - The text field is labelled "Name" (`WidgetInfo` TextEdit). It takes focus with all text selected on the first frame (`TextEdit::singleline(..).id(..)`, then `request_focus`, with the cursor range set over the whole text).
  - Enter (field lost focus with Enter pressed) or the button reports `ConfirmName`.
- Entity Info (`right.rs`): when `view.rename` is Some, draw the first info line, then the Name field (the material name field's pattern, memory key `("definition-name", definition.0)`), then the rest.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/editor.rs crates/app/src/shortcuts.rs crates/app/src/menus.rs crates/app/src/ui/mod.rs crates/app/src/ui/sheets.rs crates/app/src/ui/right.rs crates/app/src/info.rs crates/app/src/app.rs crates/app/src/ui/tests/sheets.rs crates/app/src/ui/tests/entity.rs crates/app/src/ui/tests/menus.rs crates/app/src/ui/tests/mod.rs
git commit -m "Components: name on Make Component, Rename Component (F2), Entity Info name field"
```

---

### Task 6: Purge Unused and Field of View...

**Files:**
- Modify: `crates/app/src/editor.rs` (`purge_unused`, `field_of_view`, `set_field_of_view`), `crates/app/src/menus.rs` (Group and Camera menus), `crates/app/src/ui/mod.rs` (`Sheet::FieldOfView`, actions), `crates/app/src/ui/sheets.rs`, `crates/app/src/app.rs`
- Test: `editor.rs` tests, `ui/tests/sheets.rs`, `ui/tests/menus.rs`

**Interfaces:**
- Consumes: `Outcome::purged` (Task 1), the sheet machinery (Task 4).
- Produces:

```rust
impl Editor {
    /// Runs `purge_definitions` and reports the count in the status.
    pub fn purge_unused(&mut self) -> bool;
    /// The camera's vertical field of view in degrees.
    pub fn field_of_view(&self) -> f64;
    /// Sets it through `set_camera`, keeping eye, target and up.
    pub fn set_field_of_view(&mut self, degrees: f64) -> bool;
}
Sheet::FieldOfView { degrees: f64 },
UiAction::PurgeUnused,
UiAction::ShowFieldOfView,          // open the sheet
UiAction::ConfirmFieldOfView(f64),
```

- [ ] **Step 1: Write the failing tests**

`editor.rs` tests:
- `purge_unused_reports_the_count`:
  - Make two components, erase both instances, `purge_unused()`. The info message is "Purged 2 unused definitions".
  - Again: "Nothing to purge".
  - With one: "Purged 1 unused definition".
  - One `undo` restores both.
- `field_of_view_keeps_the_view_and_its_up`:
  - `standard_view(Top)`, then `set_field_of_view(60.0)`.
  - Eye, target and `up` are unchanged, and `field_of_view()` is 60 (within 1e-9).
  - The revision is unchanged and the document is not modified.
  - `set_field_of_view(200.0)` is refused with the session's message and changes nothing.

`ui/tests/sheets.rs`:
- `the_field_of_view_sheet_applies_whole_degrees`:
  - Use `Sheet::FieldOfView { degrees: 35.4 }`. The drag field labelled "Field of view" shows "35°".
  - Typing 50 into it, then "Apply", reports `ConfirmFieldOfView(50.0)`.

`ui/tests/menus.rs`:
- `field_of_view_needs_perspective`: Camera > "Field of View..." is enabled in perspective and reports `ShowFieldOfView`. It is disabled with `projection: Parallel`.
- Extend `the_group_menu_renames_and_purges`: "Purge Unused" follows Rename Component..., is always enabled, and reports `PurgeUnused`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app -- purge_unused field_of_view group_menu`
Expected: compile errors.

- [ ] **Step 3: Implement**

- `purge_unused` runs `PurgeDefinitions {}` through `run` and reads `outcome.purged`.
- `set_field_of_view` builds `SetCamera` from `self.doc.camera` as in the Spec notes.
- `ShowFieldOfView` opens `Sheet::FieldOfView { degrees: self.editor.field_of_view() }`.
- `ConfirmFieldOfView(d)` takes that sheet and calls `set_field_of_view(d)`.
- The sheet's field is a `DragValue` with `.range(1.0..=170.0).speed(0.5).fixed_decimals(0).suffix("°")`, labelled with `labelled_by`. Its value lives in egui memory like the import sheet's, and is rounded to whole degrees when confirmed.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/editor.rs crates/app/src/menus.rs crates/app/src/ui/mod.rs crates/app/src/ui/sheets.rs crates/app/src/app.rs crates/app/src/ui/tests/sheets.rs crates/app/src/ui/tests/menus.rs
git commit -m "Group: Purge Unused reports the count; Camera: Field of View sheet"
```

---

### Task 7: The Settings window and the UI scale

**Files:**
- Create: `crates/app/src/ui/settings.rs`, `crates/app/src/ui/tests/settings.rs`
- Modify: `crates/app/src/ui/theme.rs` (`UiScale`; keyboard zoom off in `install`), `crates/app/src/prefs.rs` (`Prefs::ui_scale`), `crates/app/src/gui.rs` (zoom factor, `GuiFrame::pixels_per_point`, `physical_viewport`), `crates/app/src/shortcuts.rs` (`Key::Comma`, `Bound::Settings`, `command` for ','), `crates/app/src/native_menu.rs` (Comma accelerator, the app menu order), `crates/app/src/menus.rs`, `crates/app/src/ui/mod.rs` (`View::settings`, actions), `crates/app/src/app.rs` (actions, minimum size, the renderer's pixels per point), `crates/app/src/ui/tests/mod.rs`

**Interfaces:**
- Produces:

```rust
// ui/theme.rs
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "u16", into = "u16")]
pub enum UiScale { P90, #[default] P100, P110, P125 }
impl UiScale {
    pub const ALL: [UiScale; 4];
    pub fn percent(self) -> u16;   // 90, 100, 110, 125
    pub fn factor(self) -> f32;    // percent / 100
    pub fn label(self) -> String;  // "90%"
}
impl From<u16> for UiScale { /* unknown -> P100 */ }
impl From<UiScale> for u16 { /* percent */ }
// ui/mod.rs
pub struct SettingsView { pub ui_scale: UiScale }   // Task 8 adds trackpad
View { /* ... */ pub settings: Option<SettingsView> } // Some while the window is open
UiAction::ShowSettings(bool), UiAction::SetUiScale(UiScale),
// ui/settings.rs
pub(super) fn settings_window(ui: &mut Ui, view: &View, settings: &SettingsView, actions: &mut Vec<UiAction>);
// gui.rs
impl Gui { pub fn set_ui_scale(&self, scale: UiScale); }
pub struct GuiFrame { /* ... */ pub pixels_per_point: f32 }
/// The 3D view's rectangle in physical pixels, from its rectangle in points.
fn physical_viewport(central: egui::Rect, pixels_per_point: f32) -> Viewport;
// shortcuts.rs
Key::Comma, Bound::Settings // label "Settings", group "Edit", Cmd+,
// app.rs (private)
/// The smallest window: 960 x 600 points at `scale`, in logical pixels.
fn min_window_size(scale: UiScale) -> LogicalSize<f64>;
```

`Gui::new` gains a `ui_scale: UiScale` parameter.

- [ ] **Step 1: Write the failing tests**

`theme.rs` tests:
- `install_turns_egui_keyboard_zoom_off`: after `install` on a fresh context, `ctx.options(|o| o.zoom_with_keyboard)` is false.
- `ui_scales_read_as_percents`: `UiScale::ALL` percents are 90, 100, 110 and 125; `P125.factor() == 1.25`; `P110.label() == "110%"`; `UiScale::from(300) == P100`; `u16::from(P90) == 90`.

`prefs.rs` tests:
- `an_unknown_ui_scale_loads_as_100`: `{"ui_scale": 300}` loads `P100`, `{"ui_scale": 125}` loads `P125`, and `{"ui_scale": "big"}` is an invalid file (defaults).

`gui.rs` tests:
- `physical_viewport_uses_the_zoomed_pixels_per_point`: a 100 x 50 pt rectangle at (44, 36) with 2.5 pixels per point (2x display at 125%) is `Viewport { x: 110, y: 90, width: 250, height: 125 }`.
- `the_zoom_factor_reaches_the_frame`: on a plain `egui::Context` with native pixels per point 2.0 (in the `RawInput`'s viewport info) and `set_zoom_factor(1.25)`, a frame's `pixels_per_point` is 2.5.

`app.rs` tests:
- `the_minimum_window_grows_with_the_ui_scale`: `min_window_size(P100)` is 960 x 600, `P125` 1200 x 750, `P90` 864 x 540.

`shortcuts.rs` and `native_menu.rs` tests:
- `settings_is_cmd_comma`: `chord(Bound::Settings)` is Cmd+`Key::Comma`, its macOS text is "⌘,", and `command(',', false, false) == Some(Bound::Settings)`.
- `only_cmd_chords_become_accelerators` covers the row. Add `accelerator(chord(Settings)) == Accelerator::new(META, Code::Comma)`.

`ui/tests/menus.rs`:
- `settings_is_in_the_app_menu_or_ends_edit`:
  - `menu_bar(.., true)`'s first menu has "Settings..." before "Quit sketchup-alt".
  - `menu_bar(.., false)`'s Edit ends with a separator, then "Settings...". It reports `ShowSettings(true)` and shows "Ctrl+," off macOS (check with `text_for(false)`).
- `every_cmd_chord_is_a_native_menu_accelerator` covers Settings.

`ui/tests/settings.rs`:
- `settings_sets_the_theme_and_the_ui_scale`:
  - Use a fixture with `settings: Some(SettingsView { ui_scale: P100 })`.
  - The window "Settings" shows "Appearance" on. `is_on("100%") == Some(true)`.
  - Clicking "125%" reports `SetUiScale(P125)`, and "Dark" reports `SetTheme(Dark)`.
  - Its close button reports `ShowSettings(false)`.
- `the_settings_window_settles_in_one_frame`: after the first frame, `h.try_run()` is ok.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app -- keyboard_zoom ui_scale physical_viewport zoom_factor minimum_window settings`
Expected: compile errors.

- [ ] **Step 3: Implement**

- `theme::install` sets `ctx.options_mut(|o| o.zoom_with_keyboard = false)`.
- `Gui::new(.., ui_scale)` calls `ctx.set_zoom_factor(ui_scale.factor())`. `set_ui_scale` does the same.
- `Gui::frame` uses `physical_viewport(central, output.pixels_per_point)` and stores `pixels_per_point` in the frame.
- In `render`, `self.renderer.set_pixels_per_point(gui_frame.pixels_per_point)` replaces `window.scale_factor()`. The frame is laid out before the renderer draws, so take the value before `gui_frame` moves into `paint`.
- `SetUiScale(s)`:
  - when `s != self.prefs.ui_scale`, store it, `gui.set_ui_scale(s)`, `window.set_min_inner_size(Some(min_window_size(s)))`, `save_prefs()`;
  - harmless twice.
- `State::new` sets the minimum from prefs. `window_attributes` keeps `MIN_WINDOW` for the first frame.
- `ShowSettings(open)` sets `State::settings_open`. `View::settings` is `settings_open.then(..)`.
- `shortcuts::command(letter, ..)` maps ',' to `Key::Comma` and letters to `Key::Letter`. `Chord::text_for` shows "," for Comma.
- `native_menu::accelerator` maps `Key::Comma` with Cmd to `Code::Comma`.
- `NativeMenu::new` builds the app menu in the Spec notes' order: About, a separator, the first table entry (Settings...), a separator, Hide, Hide Others, Show All, a separator, the remaining entries (Quit).
- The app menu's table entries become `[Settings..., Quit sketchup-alt]`. Off macOS, Edit gets a separator and Settings... at its end.
- `ui::settings::settings_window`: as in the Spec notes. The tab buttons are `Button::selectable` named "Appearance" and "Input". The scale and theme buttons are selectable, named by their labels.
- In `ui::draw`, call it before the sheet when `view.settings` is Some.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/ui/settings.rs crates/app/src/ui/tests/settings.rs crates/app/src/ui/theme.rs crates/app/src/prefs.rs crates/app/src/gui.rs crates/app/src/shortcuts.rs crates/app/src/native_menu.rs crates/app/src/menus.rs crates/app/src/ui/mod.rs crates/app/src/app.rs crates/app/src/ui/tests/mod.rs crates/app/src/ui/tests/menus.rs
git commit -m "Settings: window with theme and UI scale (Cmd+,), renderer follows the zoom"
```

---

### Task 8: Settings > Input: what trackpad scrolling does

**Files:**
- Modify: `crates/app/src/navigation.rs` (`TrackpadScroll`, `Navigation::set_trackpad`, `scroll`), `crates/app/src/editor.rs` (`set_trackpad`), `crates/app/src/prefs.rs` (`Prefs::trackpad`), `crates/app/src/ui/settings.rs` (the Input tab), `crates/app/src/ui/mod.rs` (`SettingsView::trackpad`, `UiAction::SetTrackpad`), `crates/app/src/app.rs`
- Test: `navigation.rs` tests, `editor.rs` tests, `prefs.rs` tests, `ui/tests/settings.rs`

**Interfaces:**
- Produces:

```rust
// navigation.rs
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackpadScroll { #[default] Orbit, Zoom }
impl Navigation { pub fn set_trackpad(&mut self, mode: TrackpadScroll); }
// editor.rs
impl Editor { pub fn set_trackpad(&mut self, mode: TrackpadScroll); }
// ui
SettingsView { pub ui_scale: UiScale, pub trackpad: TrackpadScroll }
UiAction::SetTrackpad(TrackpadScroll),
```

- [ ] **Step 1: Write the failing tests**

`navigation.rs` tests:
- `trackpad_zoom_mode_swaps_plain_and_cmd_scroll`:
  - In Zoom mode, a plain `Scroll::Pixels((0, 100))` moves the eye toward the target, by the distance a Cmd scroll of the same pixels gives in Orbit mode (compare the two cameras).
  - The same scroll with Cmd held orbits: the eye turns about the pivot at its distance, as a plain scroll does in Orbit mode.
  - With Shift it pans in both modes.
- `wheel_and_pinch_ignore_the_trackpad_setting`: `Scroll::Lines` and `pinch` give the same camera in both modes.

`editor.rs` tests:
- `the_editor_passes_the_trackpad_setting_on`: after `set_trackpad(Zoom)`, `scroll(Scroll::Pixels(..))` changes the eye's distance to the target. Flying with `set_trackpad(Zoom)`, a two-finger scroll still looks around (as `fly_scroll`).

`prefs.rs` tests:
- `a_bad_trackpad_value_is_an_invalid_file`: `{"trackpad": "zoom"}` loads Zoom; `{"trackpad": "sideways"}` gives the defaults. A file without the key loads Orbit.

`ui/tests/settings.rs`:
- `the_input_tab_sets_the_trackpad`: click "Input", then "Zooms (Shift pans, Cmd orbits)" (on macOS; build the label with the same `cfg!` the window uses). It reports `SetTrackpad(Zoom)`, and the "Orbits (Shift pans, Cmd zooms)" radio is checked for `trackpad: Orbit`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app -- trackpad wheel_and_pinch`
Expected: compile errors.

- [ ] **Step 3: Implement**

- `Navigation::scroll`: for `Scroll::Pixels`, it zooms when `self.command != (self.trackpad == TrackpadScroll::Zoom)`, at today's Cmd-scroll speed, and otherwise orbits or pans as today.
- `Editor::set_trackpad` forwards it.
- `Prefs::trackpad`. `State::new` sets it on the editor.
- `SetTrackpad(m)` stores it, sets it on the editor, and saves the prefs when it changed.
- The radio labels say "Ctrl" for "Cmd" off macOS (`cfg!(target_os = "macos")`).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/navigation.rs crates/app/src/editor.rs crates/app/src/prefs.rs crates/app/src/ui/settings.rs crates/app/src/ui/mod.rs crates/app/src/app.rs crates/app/src/ui/tests/settings.rs crates/app/src/ui/tests/mod.rs
git commit -m "Settings: trackpad scroll orbits or zooms"
```

---

### Task 9: AccessKit and a name for every control

**Files:**
- Create: `crates/app/src/ui/tests/access.rs`
- Modify: `crates/app/Cargo.toml` (the feature), `Cargo.lock`, `crates/app/src/gui.rs` (`init_accesskit`, `on_accesskit`, `rebuilt`, `screen_reader`), `crates/app/src/app.rs` (`AppEvent::AccessKit`, `proxy`, invisible window, `recreate_gpu`), any `crates/app/src/ui/*.rs` whose control the new test finds unnamed, `crates/app/src/ui/tests/mod.rs`

**Interfaces:**
- Produces:

```rust
// app.rs
pub enum AppEvent { Api, #[cfg(target_os = "macos")] Menu { .. }, AccessKit(egui_winit::accesskit_winit::Event) }
impl From<egui_winit::accesskit_winit::Event> for AppEvent;
// gui.rs
impl Gui {
    pub fn init_accesskit(&mut self, event_loop: &ActiveEventLoop, window: &Window, proxy: EventLoopProxy<AppEvent>);
    /// Handles the adapter's event. True if a frame should be drawn.
    pub fn on_accesskit(&mut self, event: accesskit_winit::WindowEvent) -> bool;
    /// A new Gui for a new device, keeping this one's adapter and screen-reader state.
    pub fn rebuilt(self, window: &Window, device: &wgpu::Device, format: wgpu::TextureFormat, theme: ThemeChoice, ui_scale: UiScale) -> Gui;
}
/// Whether egui should build its tree after `event`: Some(true) when a screen reader asks for
/// it, Some(false) when it goes away, None otherwise.
fn screen_reader_after(event: &accesskit_winit::WindowEvent) -> Option<bool>;
```

- [ ] **Step 1: Turn the feature on and check what it brings**

```bash
# crates/app/Cargo.toml: egui-winit = { workspace = true, features = ["accesskit"] }
cargo build -p app
cargo tree -p app -e normal -i accesskit_winit
cargo tree -p app -e normal | grep -E "accesskit"
cargo tree -p app -d | grep -E "accesskit" || true
```
Expected:
- `accesskit_winit v0.32.2`, `accesskit_macos v0.26.3` and `accesskit v0.24.1` under `egui-winit`;
- the duplicate list shows only `accesskit_consumer` (0.35.0 dev, 0.38.0).

If the versions differ, stop and report them: the Spec notes' licence and weight ruling was made for these. Read `Adapter::with_event_loop_proxy` in `~/.cargo/registry/src/*/accesskit_winit-0.32.2/src/lib.rs` and confirm the window-visibility rule and the `WindowEvent` variant names used below.

- [ ] **Step 2: Write the failing tests**

`ui/tests/access.rs` (the fixture lives under `ui::tests`):
- `no_accesskit_tree_until_a_screen_reader_asks`:
  - On a plain `egui::Context` (not a kittest harness, which turns AccessKit on), install the theme and run two frames of `ui::draw` over `Fixture::default().view()`.
  - `platform_output.accesskit_update` is None.
  - After `ctx.enable_accesskit()` the next frame's is Some. After `disable_accesskit()` it is None again.

`gui.rs` tests:
- `screen_reader_events_switch_the_tree`: `screen_reader_after(&InitialTreeRequested) == Some(true)`, `AccessibilityDeactivated` gives `Some(false)`, and an `ActionRequested` gives None.

`ui/tests/access.rs`:
- `every_control_has_a_name`:
  - For each of these fixtures, walk `h.root().children_recursive()`:
    - the default;
    - `house_fixture` with a selected component and `rename` set;
    - materials with a textured row and a chosen material;
    - texture position fields;
    - the soften window, the agent activity window (two log lines), the shortcuts window;
    - the start card with two recent files;
    - the Settings window on each tab;
    - each `Sheet` variant;
    - inside a group, with the egui menus shown.
  - Every node whose role is Button, CheckBox, RadioButton, ComboBox, TextInput, SpinButton, Slider or Tab, or that supports `Action::Click`, has a non-empty `label()` (or a `labelled_by` node with one).
  - The failure message names the node's role, its rect and the fixture.
- `icon_buttons_are_named_by_their_label`: every `icons::all()` glyph drawn by the default and house fixtures belongs to a node with a non-empty name. Find them through `drawn_text_rects`: the glyph's rect lies in a named node's rect.

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test -p app -- accesskit_tree screen_reader every_control icon_buttons_are_named`
Expected: compile errors, then (once it compiles) `every_control_has_a_name` lists the unnamed controls, if any.

- [ ] **Step 4: Implement**

- `app.rs`:
  - `App::proxy` replaces `menu_proxy` on every system.
  - `window_attributes()` adds `.with_visible(false)`.
  - `resumed` calls `state.gui.init_accesskit(event_loop, &state.window, self.proxy.clone())`, then `state.window.set_visible(true)`, before `request_redraw`.
  - `user_event` handles `AppEvent::AccessKit(e)` by calling `s.gui.on_accesskit(e.window_event)` and redrawing when it returns true.
  - `recreate_gpu` uses `Gui::rebuilt`.
- `Gui::on_accesskit`:
  - `InitialTreeRequested`: `screen_reader = true`, `ctx.enable_accesskit()`, true.
  - `ActionRequested(r)`: `state.on_accesskit_action_request(r)`, true.
  - `AccessibilityDeactivated`: `screen_reader = false`, `ctx.disable_accesskit()`, false.
- `Gui::rebuilt` moves `self.state.accesskit.take()` into the new state, and enables AccessKit on the new context when `screen_reader` is set.
- Name each control the walk finds unnamed:
  - Use `widget_info` with `WidgetInfo::labeled` (or `selected` for toggles), with the label the control's tooltip or visible text gives.
  - If the Outliner's expand arrows are unnamed, draw them with `CollapsingState::show_toggle_button` inside the row's header and name them "Expand <name>" or "Collapse <name>".
  - Do not add an allowlist. If an egui-internal widget cannot be named, stop and report it.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 6: Reason about the per-frame cost in the commit message body**

One line: "No tree is built until a screen reader asks (tested); the adapter only sees window events."

- [ ] **Step 7: Commit**

```bash
git status && git diff --stat
git add crates/app/Cargo.toml Cargo.lock crates/app/src/gui.rs crates/app/src/app.rs crates/app/src/ui/tests/access.rs crates/app/src/ui/tests/mod.rs <each ui file given a name>
git commit -m "Accessibility: AccessKit adapter on screen reader request, every control named"
```

---

### Task 10: Docs: the agent API

**Files:**
- Modify: `docs/agent-api.md`

- [ ] **Step 1: Edit**

- **Groups and components:** after the `purge_definitions` sentence, add that its outcome's `purged` counts the definitions removed (nested ones included; 0 when nothing was unused).
- **Editor:**
  - The `set_import_units` row: it sets the units of the window's import options sheet for the rest of the session. The other sheet values (up axis, import as, merge coplanar faces) are the `import` command's own parameters.
  - The `dialog` sentence: the window's rfd file and unsaved-changes dialogs are modal and hold requests. Its sheets (import options, component name, rename, field of view) and the Settings window are chrome and do not.
- **Camera:** the trackpad line reads: by default two-finger scroll orbits, Shift pans, Cmd/Ctrl+scroll and pinch zoom. Settings > Input can swap scroll and Cmd+scroll, so scroll zooms and Cmd+scroll orbits. Window chrome, kept in prefs; `orbit`, `zoom` and `pan` are unaffected.
- **Window actions table:**
  - File row: the prompt reads `Do you want to save changes to "<name>"?` (Save, Don't Save, Cancel); it is still `unsaved_changes` / `discard`.
  - New row: File > Open Recent (the last 10 files) and Clear Menu: `open` (`path`); the list is chrome.
  - Import row: "File > Import... (Cmd+I) and its options sheet (units, up axis, import as, merge coplanar faces)": `import` (`scale`, `up`, `as`, `merge_coplanar`); `set_import_units`. Drop "File > Import Units".
  - Group row: Make Component... (Shift+G, asks for a name): `make_component` (`name`). New row: Group > Rename Component... (F2), Entity Info's Name field: `rename_definition` (the instance's `definition` from `entity`). New row: Group > Purge Unused: `purge_definitions` (`purged`).
  - Camera row: add Field of View...: `set_camera` (`fov_y_deg`, with the current `eye` and `target`).
  - Theme row becomes: Settings (Cmd+,): Appearance (theme, UI scale) and Input (trackpad scroll); View > Theme. None: window chrome, kept in `~/.sketchup-alt/prefs.json`.
  - Title row: "The window's title (the file name; the edited dot on macOS, `*` elsewhere)": `file`.
- **Chrome paragraph:** add the UI scale and trackpad setting, the sheets and the Settings window, and VoiceOver (AccessKit) reading the window.

- [ ] **Step 2: Check** that `cargo test -p api` passes and that the file has no emoji (`grep -nP '[\x{1F300}-\x{1FAFF}]' docs/agent-api.md` prints nothing).

- [ ] **Step 3: Commit**

```bash
git status && git diff --stat
git add docs/agent-api.md
git commit -m "Docs: purge count, import options, Open Recent, rename, field of view, Settings"
```

---

### Task 11: Roadmap, whole-branch review, renderer check, live look

- [ ] **Step 1: Full gate**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace 2>&1 | tee target/m11e-test.log | grep "test result" | awk '{s+=$4} END {print s}'
grep -E "FAILED|failed;" target/m11e-test.log
cargo build --release -p app
git diff --stat <base>..HEAD -- crates/render
```
Expected:
- fmt and clippy clean;
- at least N0 plus this milestone's new tests;
- no failure lines;
- a release build;
- no `crates/render` changes, so CLASSIC renders cannot differ and the two-worktree `cmp` is skipped.

If anything under `crates/render/src` changed, run the M11c two-worktree golden `cmp` (M11c plan, Task 13 Step 4) and fix any `DIFF`.

- [ ] **Step 2: Roadmap row**

In `docs/superpowers/roadmap.md`, change the M11e row's last cell from `planned` to `done <first>..<last>`, using the short hashes of Task 1's commit and Task 10's commit.

```bash
git status && git diff --stat
git add docs/superpowers/roadmap.md
git commit -m "Roadmap: M11e done"
```

- [ ] **Step 3: Whole-branch review**

Use superpowers:requesting-code-review over `<base>..HEAD`, against the spec's M11e bullet and this plan's Spec notes and Review Focus. Ask the reviewer to check in particular:
- No double fire: Settings' Cmd+, is a native accelerator and refused by `window_command` with Cmd on macOS; F2 and Shift+G act on the key path only; nothing in the key path runs while a sheet is open.
- The native Open Recent submenu is rebuilt only when the list changes, and its stale ids can never open a different file.
- Every confirm action takes the sheet, so a second copy in the same frame does nothing.
- Prefs: old files load with defaults; `ui_scale` out of range loads as 100; the API and headless mode never read or write prefs; tests never touch the real home.
- The renderer's pixels per point is egui's (scale factor times zoom), the pick tolerances are not, and the minimum window grows with the scale.
- AccessKit builds no tree until `InitialTreeRequested`, the window is created invisible and shown after the adapter, and `recreate_gpu` keeps the adapter.
- The unsaved prompt's text and buttons match the spec exactly, and every non-button answer cancels.
- `Outcome::purged` matches the definitions actually removed, and undo restores them.
- Every Spec note value matches the code (labels, messages, ranges, sizes, menu order).
- No test was lost (compare with N0).

Fix findings in follow-up commits and rerun Step 1.

- [ ] **Step 4: Live look in the real window**

1. Build a `.app` bundle wrapper in the scratchpad around the release binary, with its own bundle id and socket, as the M11b memory note describes (a bare binary is invisible to computer use). Use `app_*` computer-use tools on that app only. Do not take over the user's screen (no full-control request).
2. Open it and check, by screenshot:
   - The title reads "Untitled.skr". After drawing a rectangle (R, drag), the close button shows the edited dot.
   - Cmd+Q shows `Do you want to save changes to "Untitled.skr"?` with Save, Don't Save and Cancel. Cancel keeps the window.
   - Save to a scratchpad file, then open another. File > Open Recent lists both, and choosing one opens it. Clear Menu empties the list.
   - File > Import... with a scratchpad OBJ shows the options sheet. Typing "r" there does not switch tools. Import as Component with Z up places a component.
   - Shift+G on a selection asks for a name. F2 on the selected Outliner row opens Rename Component. Entity Info's Name field renames.
   - Group > Purge Unused after erasing a component shows "Purged 1 unused definition".
   - Camera > Field of View... at 60 widens the view.
   - Cmd+, opens Settings. 125% enlarges the panels, and the window cannot be made smaller than 1200 x 750. The hover edge and the snap marker grow with them, and edges and HUD lines still look crisp. Back to 100%.
   - Input > Zooms: two-finger scroll zooms (if no trackpad can be driven, note it as not checked).
   - With VoiceOver off, Activity Monitor shows about 0% CPU when the window is still. Turn VoiceOver on only if the user agrees, and check that it reads a rail button's name.
3. Report what was seen. Fix anything wrong in follow-up commits.

Report the milestone's summary with the rulings in "Spec notes".
