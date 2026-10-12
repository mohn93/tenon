use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::{Hub, listen_unix};
use glam::DVec2;
use kernel::{EntityId, InstanceId};
use render::{Renderer, Snapshotter, Viewport};
use session::Command;
use tools::Axis;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::editor::{Editor, EditorKey};
use crate::gui::Gui;
use crate::host::WindowHost;
use crate::info::entity_info;
use crate::navigation::Button;
use crate::ui::{UiAction, View};

/// Sent to the event loop when an agent API request arrives.
#[derive(Debug)]
pub struct ApiWake;

/// Longest delayed repaint worth scheduling; egui uses huge delays to mean "never".
const MAX_REPAINT_DELAY: Duration = Duration::from_secs(60);

/// What to do with the repaint egui asked for after a frame.
#[derive(Debug, PartialEq)]
enum Repaint {
    Now,
    At(Instant),
    Idle,
}

fn schedule_repaint(now: Instant, delay: Option<Duration>) -> Repaint {
    match delay {
        Some(d) if d.is_zero() => Repaint::Now,
        Some(d) if d < MAX_REPAINT_DELAY => Repaint::At(now + d),
        _ => Repaint::Idle,
    }
}

/// What the event loop does before sleeping, given the pending repaint deadline.
#[derive(Debug, PartialEq)]
enum Wake {
    /// The deadline passed: redraw and sleep until input.
    Redraw,
    Until(Instant),
    Wait,
}

fn wake_decision(now: Instant, deadline: Option<Instant>) -> Wake {
    match deadline {
        Some(t) if t <= now => Wake::Redraw,
        Some(t) => Wake::Until(t),
        None => Wake::Wait,
    }
}

/// Trackpad pixel deltas per zoom step.
const PIXELS_PER_SCROLL_STEP: f64 = 50.0;

/// Longest time between the two releases of a double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(500);
/// Farthest the cursor may move between them, in logical pixels.
const DOUBLE_CLICK_LOGICAL_PX: f64 = 6.0;

/// True if a release at `at` (physical pixels) at time `now` completes a double-click begun
/// by the release `last`.
fn is_double_click(last: Option<(Instant, DVec2)>, now: Instant, at: DVec2, max_px: f64) -> bool {
    last.is_some_and(|(t, p)| now.duration_since(t) <= DOUBLE_CLICK && p.distance(at) <= max_px)
}

/// True if a release at `at` ends a click, that is the cursor is within `max_px` of where the
/// button went down, rather than a drag.
fn is_click(press: Option<DVec2>, at: DVec2, max_px: f64) -> bool {
    press.is_some_and(|p| p.distance(at) <= max_px)
}

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    snapshotter: Snapshotter,
    editor: Editor,
    gui: Gui,
    /// Document revision, selection and open context the renderer's model buffers were built
    /// from.
    drawn: Option<(u64, BTreeSet<EntityId>, Vec<InstanceId>)>,
    modifiers: ModifiersState,
    /// Where the 3D view sits in the window (physical pixels), from the last frame's layout.
    view: Viewport,
    /// Last cursor position in window coordinates (physical pixels).
    cursor: DVec2,
    /// When egui wants its next frame without any input (e.g. a tooltip delay).
    repaint_at: Option<Instant>,
    /// The window title last set.
    title: String,
    /// When and where the left button was last released in the view, for double-clicks.
    last_release: Option<(Instant, DVec2)>,
    /// Where the left button was last pressed in the view, so a release that ends a drag does
    /// not count as a click.
    left_press: Option<DVec2>,
}

impl State {
    fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no compatible GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("request device");

        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface not supported by adapter");
        // Colors are authored as sRGB values, so prefer a non-sRGB target that writes them as-is.
        // Keep the default format's channel layout when its non-sRGB twin is available.
        let caps = surface.get_capabilities(&adapter);
        let linear = config.format.remove_srgb_suffix();
        config.format = if caps.formats.contains(&linear) {
            linear
        } else {
            caps.formats
                .iter()
                .copied()
                .find(|f| !f.is_srgb())
                .unwrap_or(config.format)
        };
        surface.configure(&device, &config);

        let renderer = Renderer::new(&device, &queue, config.format);
        let gui = Gui::new(&window, &device, config.format);
        let view = Viewport::full((config.width, config.height));
        let snapshotter = Snapshotter::with_device(&device, &queue);
        let editor = Editor::new(
            DVec2::new(config.width as f64, config.height as f64),
            window.scale_factor(),
        );
        Self {
            window,
            surface,
            device,
            queue,
            config,
            renderer,
            snapshotter,
            editor,
            gui,
            drawn: None,
            modifiers: ModifiersState::empty(),
            view,
            cursor: DVec2::ZERO,
            repaint_at: None,
            last_release: None,
            title: String::new(),
            left_press: None,
        }
    }

    /// True if `p` (window coordinates) lies in the 3D view.
    fn in_view(&self, p: DVec2) -> bool {
        let v = self.view;
        p.x >= v.x as f64
            && p.y >= v.y as f64
            && p.x < (v.x + v.width) as f64
            && p.y < (v.y + v.height) as f64
    }

    /// The cursor moved to `p` in window coordinates; the editor works in view coordinates.
    fn cursor_moved(&mut self, p: DVec2) -> bool {
        self.cursor = p;
        let origin = DVec2::new(self.view.x as f64, self.view.y as f64);
        let moved = self.editor.cursor_moved(p - origin);
        if self.in_view(p) {
            moved
        } else {
            self.editor.cursor_left() || moved
        }
    }

    /// Before the model is replaced or the window closes: if it has unsaved changes, asks
    /// whether to save them. True when it is fine to go on (saved, or changes discarded).
    fn settle_changes(&mut self) -> bool {
        let doc = self.editor.doc();
        if !doc.is_modified() {
            return true;
        }
        let name = file_name(doc.path());
        let answer = rfd::MessageDialog::new()
            .set_title("Unsaved changes")
            .set_description(format!("Save the changes to {name} first?"))
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .show();
        match answer {
            rfd::MessageDialogResult::Yes => self.save(false),
            rfd::MessageDialogResult::No => true,
            _ => false,
        }
    }

    fn new_model(&mut self) {
        if self.settle_changes() {
            self.editor.new_model();
        }
    }

    fn open(&mut self) {
        if !self.settle_changes() {
            return;
        }
        if let Some(path) = skr_dialog().pick_file() {
            self.editor.open_file(&path);
        }
    }

    /// Saves to the current file, or to one chosen in a dialog when there is none or `choose`
    /// is set. False if cancelled or the save failed.
    fn save(&mut self, choose: bool) -> bool {
        let path = if choose || self.editor.doc().path().is_none() {
            let name = file_name(self.editor.doc().path());
            match skr_dialog().set_file_name(name).save_file() {
                Some(path) => Some(path),
                None => return false,
            }
        } else {
            None
        };
        self.editor.save_file(path.as_deref())
    }

    /// Exports to a file chosen in a dialog; its extension picks the format.
    fn export(&mut self) {
        let stem = file_name(self.editor.doc().path());
        let stem = stem.strip_suffix(".skr").unwrap_or(&stem);
        let chosen = rfd::FileDialog::new()
            .add_filter("OBJ", &["obj"])
            .add_filter("STL", &["stl"])
            .add_filter("glTF binary", &["glb"])
            .add_filter("glTF", &["gltf"])
            .set_file_name(format!("{stem}.obj"))
            .save_file();
        if let Some(path) = chosen {
            self.editor.export_file(&path);
        }
    }

    fn apply(&mut self, action: UiAction) {
        match action {
            UiAction::NewModel => self.new_model(),
            UiAction::OpenFile => self.open(),
            UiAction::SaveFile => {
                self.save(false);
            }
            UiAction::SaveFileAs => {
                self.save(true);
            }
            UiAction::ExportFile => self.export(),
            UiAction::Tool(kind) => self.editor.select_tool(kind),
            UiAction::Undo => {
                self.editor.undo();
            }
            UiAction::Redo => {
                self.editor.redo();
            }
            UiAction::EraseSelection => {
                self.editor.erase_selection();
            }
            UiAction::MakeGroup => {
                self.editor.make_group();
            }
            UiAction::MakeComponent => {
                self.editor.make_component();
            }
            UiAction::Explode => {
                self.editor.explode_selection();
            }
            UiAction::CloseInstance => {
                self.editor.close_instance();
            }
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Lays out and draws the 3D view and the panels, then applies what was clicked in them
    /// and asks for another frame to show the result. Returns true if a panel action ran (so
    /// subscribers should hear about edits).
    fn render(&mut self) -> bool {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return false;
            }
            _ => return false, // Timeout or occluded: skip this frame.
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let size = (self.config.width, self.config.height);

        let doc = self.editor.doc();
        let title = window_title(doc.path(), doc.is_modified());
        if title != self.title {
            self.window.set_title(&title);
            self.title = title;
        }
        let status = self.editor.status_parts();
        let info = entity_info(self.editor.doc());
        let panels = View {
            tool: self.editor.tool_kind(),
            status: &status,
            info: &info,
            has_selection: !self.editor.doc().selection().is_empty(),
            has_instance_selected: self
                .editor
                .doc()
                .selection()
                .iter()
                .any(|id| matches!(id, EntityId::Instance(_))),
            inside_instance: !self.editor.doc().context().is_empty(),
        };
        let mut gui_frame = self.gui.frame(&self.window, &panels);
        // Applied once the frame is presented: some open modal dialogs, which must not run
        // while this frame's surface texture is held.
        let actions = std::mem::take(&mut gui_frame.actions);
        let acted = !actions.is_empty();
        if gui_frame.viewport != self.view {
            self.view = gui_frame.viewport;
            let v = self.view;
            self.editor
                .resize(DVec2::new(v.width.max(1) as f64, v.height.max(1) as f64));
        }

        let doc = self.editor.doc();
        let drawn = (
            doc.revision(),
            doc.selection().clone(),
            doc.context().to_vec(),
        );
        if self.drawn.as_ref() != Some(&drawn) {
            self.renderer
                .set_scene(doc.scene(), doc.context(), doc.selection());
            self.drawn = Some(drawn);
        }
        self.renderer.render_in(
            &target,
            size,
            self.view,
            &self.editor.doc().camera,
            &self.editor.overlay(),
        );
        let again = self.gui.paint(
            &self.window,
            &self.device,
            &self.queue,
            &target,
            size,
            gui_frame,
        );
        self.window.pre_present_notify();
        self.queue.present(frame);
        self.repaint_at = None;
        match schedule_repaint(Instant::now(), again) {
            Repaint::Now => self.window.request_redraw(),
            Repaint::At(t) => self.repaint_at = Some(t),
            Repaint::Idle => {}
        }
        for action in actions {
            self.apply(action);
        }
        if acted {
            self.window.request_redraw();
        }
        acted
    }

    /// Handles waiting API requests; redraws if they changed the model, selection, open
    /// context or camera.
    fn process_api(&mut self, hub: &mut Hub) {
        let before = (
            self.editor.doc().revision(),
            self.editor.doc().selection().clone(),
            self.editor.doc().context().to_vec(),
            self.editor.doc().camera.clone(),
            self.editor.overlay(),
        );
        let mut host = WindowHost {
            editor: &mut self.editor,
            snapshotter: &mut self.snapshotter,
        };
        hub.process(&mut host);
        if (
            self.editor.doc().revision(),
            self.editor.doc().selection().clone(),
            self.editor.doc().context().to_vec(),
            self.editor.doc().camera.clone(),
            self.editor.overlay(),
        ) != before
        {
            self.window.request_redraw();
        }
    }

    /// Tells API subscribers about edits made in the window.
    fn notify_api(&mut self, hub: &mut Hub) {
        let host = WindowHost {
            editor: &mut self.editor,
            snapshotter: &mut self.snapshotter,
        };
        hub.notify(&host);
    }
}

/// The letter of a Cmd/Ctrl shortcut: the layout's character when it is ASCII, else the
/// physical key's on a US layout (so Cmd+Z works on Arabic or Cyrillic layouts, and Cmd+W on
/// AZERTY is not Cmd+Z).
fn shortcut_letter(logical: &Key, physical: PhysicalKey) -> Option<char> {
    match logical {
        Key::Character(c) if c.is_ascii() => c.chars().next().map(|c| c.to_ascii_lowercase()),
        _ => match physical {
            PhysicalKey::Code(KeyCode::KeyZ) => Some('z'),
            PhysicalKey::Code(KeyCode::KeyN) => Some('n'),
            PhysicalKey::Code(KeyCode::KeyO) => Some('o'),
            PhysicalKey::Code(KeyCode::KeyS) => Some('s'),
            PhysicalKey::Code(KeyCode::KeyE) => Some('e'),
            PhysicalKey::Code(KeyCode::KeyQ) => Some('q'),
            _ => None,
        },
    }
}

/// A file dialog for `.skr` models.
fn skr_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("SketchUp Alt model", &["skr"])
}

/// The file's name, or "Untitled.skr" for a model never saved.
fn file_name(path: Option<&Path>) -> String {
    path.and_then(|p| p.file_name()).map_or_else(
        || "Untitled.skr".into(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// "house.skr - sketchup-alt", with "*" after the name while there are unsaved changes.
fn window_title(path: Option<&Path>, modified: bool) -> String {
    let star = if modified { "*" } else { "" };
    format!("{}{star} - sketchup-alt", file_name(path))
}

/// The editor key for a key press. Characters from non-ASCII layouts (e.g. Arabic) fall back to
/// the physical key so shortcuts and digits work on any layout; with Shift held the physical
/// letter is uppercased (ASCII characters already have Shift applied by the layout).
fn editor_key(logical: &Key, physical: PhysicalKey, shift: bool) -> Option<EditorKey> {
    match logical {
        Key::Named(NamedKey::Enter) => Some(EditorKey::Enter),
        Key::Named(NamedKey::Backspace) => Some(EditorKey::Backspace),
        Key::Named(NamedKey::Delete) => Some(EditorKey::Delete),
        // Arrow keys lock the drawing direction: right red, left green, up blue, down unlocks.
        Key::Named(NamedKey::ArrowRight) => Some(EditorKey::LockAxis(Axis::X)),
        Key::Named(NamedKey::ArrowLeft) => Some(EditorKey::LockAxis(Axis::Y)),
        Key::Named(NamedKey::ArrowUp) => Some(EditorKey::LockAxis(Axis::Z)),
        Key::Named(NamedKey::ArrowDown) => Some(EditorKey::Unlock),
        Key::Named(NamedKey::Escape) => Some(EditorKey::Escape),
        Key::Named(NamedKey::Space) => Some(EditorKey::Char(' ')),
        Key::Character(text) => {
            let c = text.chars().next()?;
            if c.is_ascii() {
                Some(EditorKey::Char(c))
            } else {
                let c = physical_char(physical)?;
                Some(EditorKey::Char(if shift {
                    c.to_ascii_uppercase()
                } else {
                    c
                }))
            }
        }
        _ => None,
    }
}

/// The US-layout character of a physical key, for the keys the editor uses.
fn physical_char(key: PhysicalKey) -> Option<char> {
    let PhysicalKey::Code(code) = key else {
        return None;
    };
    Some(match code {
        KeyCode::Digit0 => '0',
        KeyCode::Digit1 => '1',
        KeyCode::Digit2 => '2',
        KeyCode::Digit3 => '3',
        KeyCode::Digit4 => '4',
        KeyCode::Digit5 => '5',
        KeyCode::Digit6 => '6',
        KeyCode::Digit7 => '7',
        KeyCode::Digit8 => '8',
        KeyCode::Digit9 => '9',
        KeyCode::Period => '.',
        KeyCode::Minus => '-',
        KeyCode::Comma => ',',
        KeyCode::KeyL => 'l',
        KeyCode::KeyR => 'r',
        KeyCode::KeyP => 'p',
        KeyCode::KeyE => 'e',
        KeyCode::KeyM => 'm',
        KeyCode::KeyG => 'g',
        KeyCode::KeyS => 's',
        KeyCode::KeyC => 'c',
        _ => return None,
    })
}

fn map_button(button: MouseButton) -> Button {
    match button {
        MouseButton::Left => Button::Left,
        MouseButton::Middle => Button::Middle,
        _ => Button::Other,
    }
}

pub struct App {
    state: Option<State>,
    hub: Hub,
    /// Socket the agent API listens on, removed on exit (None if listening failed).
    socket: Option<PathBuf>,
    /// A model to open once the window exists.
    file: Option<PathBuf>,
}

impl App {
    /// `socket` is where the agent API listens; None runs without it. `file` is a model to
    /// open at start.
    pub fn new(
        event_loop: &EventLoop<ApiWake>,
        socket: Option<PathBuf>,
        file: Option<PathBuf>,
    ) -> Self {
        let proxy = event_loop.create_proxy();
        let hub = Hub::new(move || {
            let _ = proxy.send_event(ApiWake);
        });
        let socket = socket.and_then(|socket| match listen_unix(&socket, hub.inbox()) {
            Ok(()) => {
                eprintln!("agent API listening on {}", socket.display());
                Some(socket)
            }
            Err(e) => {
                eprintln!(
                    "agent API disabled: cannot listen on {}: {e}",
                    socket.display()
                );
                None
            }
        });
        Self {
            state: None,
            hub,
            socket,
            file,
        }
    }
}

impl ApplicationHandler<ApiWake> for App {
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(path) = self.socket.take() {
            let _ = std::fs::remove_file(path);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let deadline = self.state.as_ref().and_then(|s| s.repaint_at);
        match wake_decision(Instant::now(), deadline) {
            Wake::Redraw => {
                if let Some(s) = self.state.as_mut() {
                    s.repaint_at = None;
                    s.window.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Wake::Until(t) => event_loop.set_control_flow(ControlFlow::WaitUntil(t)),
            Wake::Wait => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ApiWake) {
        if let Some(s) = self.state.as_mut() {
            s.process_api(&mut self.hub);
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("sketchup-alt")
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let mut state = State::new(window);
        if let Some(path) = self.file.take() {
            let open = Command::Open {
                path: path.display().to_string(),
                // A fresh model: nothing to lose.
                discard: false,
            };
            if let Err(e) = state.editor.execute(open) {
                eprintln!("cannot open {}", e.message);
            }
        }
        state.window.request_redraw();
        // Requests that arrived before the window existed.
        state.process_api(&mut self.hub);
        self.state = Some(state);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(s) = self.state.as_mut() else { return };
        if s.gui.on_event(&s.window, &event) {
            s.window.request_redraw();
        }
        let changed = match event {
            WindowEvent::CloseRequested => {
                if s.settle_changes() {
                    event_loop.exit();
                }
                false
            }
            WindowEvent::Resized(size) => {
                s.resize(size.width, size.height);
                true
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                s.editor.set_scale_factor(scale_factor);
                false
            }
            WindowEvent::RedrawRequested => {
                if s.render() {
                    s.notify_api(&mut self.hub);
                }
                false
            }
            WindowEvent::ModifiersChanged(m) => {
                let ctrl_pressed = m.state().control_key() && !s.modifiers.control_key();
                s.modifiers = m.state();
                let locked = s
                    .editor
                    .set_modifiers(m.state().shift_key(), m.state().alt_key());
                // Tapping Ctrl switches tools that can copy (Move) between moving and copying.
                let toggled = ctrl_pressed && s.editor.toggle_copy();
                locked || toggled
            }
            WindowEvent::Focused(false) => s.editor.focus_lost(),
            WindowEvent::CursorMoved { position, .. } => {
                s.cursor_moved(DVec2::new(position.x, position.y))
            }
            WindowEvent::CursorLeft { .. } => s.editor.cursor_left(),
            // Presses go to the view only when they land in it; releases always do, so a drag
            // that ends over a panel still finishes.
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                if pressed && (s.gui.wants_pointer() || !s.in_view(s.cursor)) {
                    false
                } else {
                    let handled = s.editor.button(map_button(button), pressed);
                    if pressed && button == MouseButton::Left {
                        s.left_press = Some(s.cursor);
                    } else if handled && button == MouseButton::Left {
                        let now = Instant::now();
                        let max = DOUBLE_CLICK_LOGICAL_PX * s.window.scale_factor();
                        let click = is_click(s.left_press.take(), s.cursor, max);
                        if click && is_double_click(s.last_release, now, s.cursor, max) {
                            s.last_release = None;
                            s.editor.double_click();
                        } else if click {
                            s.last_release = Some((now, s.cursor));
                        } else {
                            s.last_release = None;
                        }
                    }
                    handled
                }
            }
            WindowEvent::MouseWheel { .. } if s.gui.wants_pointer() || !s.in_view(s.cursor) => {
                false
            }
            WindowEvent::KeyboardInput { .. } if s.gui.wants_keyboard() => false,
            WindowEvent::MouseWheel { delta, .. } => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / PIXELS_PER_SCROLL_STEP,
                };
                s.editor.scroll(steps)
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let command_key = s.modifiers.super_key() || s.modifiers.control_key();
                let letter = shortcut_letter(&event.logical_key, event.physical_key);
                if command_key && letter == Some('z') {
                    if s.modifiers.shift_key() {
                        s.editor.redo()
                    } else {
                        s.editor.undo()
                    }
                } else if command_key && !event.repeat && letter == Some('n') {
                    s.new_model();
                    true
                } else if command_key && !event.repeat && letter == Some('o') {
                    s.open();
                    true
                } else if command_key && !event.repeat && letter == Some('s') {
                    s.save(s.modifiers.shift_key());
                    true
                } else if command_key
                    && !event.repeat
                    && s.modifiers.shift_key()
                    && letter == Some('e')
                {
                    s.export();
                    true
                } else if command_key && !event.repeat && letter == Some('q') {
                    if s.settle_changes() {
                        event_loop.exit();
                    }
                    false
                } else if command_key || s.modifiers.alt_key() {
                    // Alt/Option is the orbit modifier; Option+letter must not type or switch
                    // tools through its physical-key fallback.
                    false
                } else {
                    editor_key(
                        &event.logical_key,
                        event.physical_key,
                        s.modifiers.shift_key(),
                    )
                    .is_some_and(|k| {
                        if event.repeat {
                            s.editor.key_repeat(k)
                        } else {
                            s.editor.key(k)
                        }
                    })
                }
            }
            _ => false,
        };
        if changed {
            s.window.request_redraw();
            s.notify_api(&mut self.hub);
        }
    }
}

#[cfg(test)]
mod tests {
    use winit::keyboard::SmolStr;

    #[test]
    fn non_ascii_layouts_fall_back_to_the_physical_key() {
        let arabic = Key::Character(SmolStr::new("ا"));
        let g = PhysicalKey::Code(KeyCode::KeyG);
        let ch = |k: Option<EditorKey>| match k {
            Some(EditorKey::Char(c)) => Some(c),
            _ => None,
        };
        assert_eq!(ch(editor_key(&arabic, g, true)), Some('G'));
        assert_eq!(ch(editor_key(&arabic, g, false)), Some('g'));
        let s = PhysicalKey::Code(KeyCode::KeyS);
        assert_eq!(ch(editor_key(&arabic, s, false)), Some('s'));
        let ascii = Key::Character(SmolStr::new("g"));
        assert_eq!(ch(editor_key(&ascii, g, true)), Some('g'));
    }

    use super::*;

    #[test]
    fn delayed_repaints_are_scheduled() {
        let now = Instant::now();
        let ms = Duration::from_millis;
        assert_eq!(schedule_repaint(now, None), Repaint::Idle);
        assert_eq!(schedule_repaint(now, Some(ms(0))), Repaint::Now);
        assert_eq!(
            schedule_repaint(now, Some(ms(500))),
            Repaint::At(now + ms(500))
        );
        assert_eq!(
            schedule_repaint(now, Some(Duration::from_secs(3600))),
            Repaint::Idle
        );
    }

    #[test]
    fn double_clicks_are_two_quick_releases_in_one_place() {
        let now = Instant::now();
        let at = DVec2::new(100.0, 100.0);
        let soon = now + Duration::from_millis(300);
        assert!(!is_double_click(None, now, at, 6.0));
        assert!(is_double_click(
            Some((now, at)),
            soon,
            at + DVec2::new(3.0, 3.0),
            6.0
        ));
        assert!(!is_double_click(
            Some((now, at)),
            soon,
            at + DVec2::new(10.0, 0.0),
            6.0
        ));
        let late = now + Duration::from_millis(800);
        assert!(!is_double_click(Some((now, at)), late, at, 6.0));
    }

    #[test]
    fn a_release_far_from_its_press_ends_a_drag_not_a_click() {
        let at = DVec2::new(100.0, 100.0);
        assert!(!is_click(None, at, 6.0));
        assert!(is_click(Some(at), at + DVec2::new(3.0, 3.0), 6.0));
        assert!(!is_click(Some(at), at + DVec2::new(40.0, 0.0), 6.0));
    }

    #[test]
    fn titles_name_the_file_and_mark_unsaved_changes() {
        assert_eq!(window_title(None, false), "Untitled.skr - sketchup-alt");
        assert_eq!(
            window_title(Some(Path::new("/m/house.skr")), true),
            "house.skr* - sketchup-alt"
        );
    }

    #[test]
    fn shortcut_letters_follow_the_layout_unless_it_is_not_ascii() {
        let code = |c| PhysicalKey::Code(c);
        let ch = |s: &str| Key::Character(s.into());
        assert_eq!(shortcut_letter(&ch("S"), code(KeyCode::KeyS)), Some('s'));
        // AZERTY: the key in the US Z place types "w"; it must not undo.
        assert_eq!(shortcut_letter(&ch("w"), code(KeyCode::KeyZ)), Some('w'));
        // Arabic: the character is not ASCII, so the physical key decides.
        assert_eq!(shortcut_letter(&ch("س"), code(KeyCode::KeyS)), Some('s'));
        assert_eq!(shortcut_letter(&ch("ئ"), code(KeyCode::KeyZ)), Some('z'));
        assert_eq!(shortcut_letter(&ch("ب"), code(KeyCode::KeyF)), None);
        assert_eq!(shortcut_letter(&ch("ض"), code(KeyCode::KeyQ)), Some('q'));
        assert_eq!(shortcut_letter(&ch("Q"), code(KeyCode::KeyQ)), Some('q'));
        assert_eq!(shortcut_letter(&ch("ث"), code(KeyCode::KeyE)), Some('e'));
    }

    #[test]
    fn the_loop_wakes_at_the_deadline() {
        let now = Instant::now();
        let later = now + Duration::from_millis(300);
        assert_eq!(wake_decision(now, None), Wake::Wait);
        assert_eq!(wake_decision(now, Some(later)), Wake::Until(later));
        assert_eq!(wake_decision(later, Some(later)), Wake::Redraw);
        assert_eq!(
            wake_decision(later + Duration::from_millis(1), Some(later)),
            Wake::Redraw
        );
    }
}
