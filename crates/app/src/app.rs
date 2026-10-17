use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use api::{ApiRoots, EditorState, Hub, listen_unix};
use glam::DVec2;
use kernel::{EntityId, InstanceId, MaterialId, TextureImage};
use render::{Renderer, Snapshotter, Viewport};
use session::{Camera, Command, Lighting, OutlinerTable};
use tools::Axis;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{
    DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowAttributes, WindowId};

use crate::awake::Awake;
use crate::editor::{Editor, EditorKey};
use crate::fly::FlyKey;
use crate::gui::Gui;
use crate::host::{AppHost, CLOSE_WAIT, Offscreen, editor_state};
use crate::info::entity_info;
use crate::navigation::{Button, Scroll};
use crate::shortcuts::{self, Bound};
use crate::ui::theme::{self, ThemeChoice};
use crate::ui::{Agents, MaterialRow, TexturePosition, TextureRow, UiAction, View};

/// The window's least size, in points: the panels and a usable 3D view.
const MIN_WINDOW: LogicalSize<f64> = LogicalSize::new(960.0, 600.0);

/// The window as it opens: title, default size and least size.
fn window_attributes() -> WindowAttributes {
    Window::default_attributes()
        .with_title("sketchup-alt")
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_min_inner_size(MIN_WINDOW)
}

/// The side of a texture's swatch image in the Materials list, in pixels.
const SWATCH_PIXELS: u32 = 32;

/// Sent to the event loop when an agent API request arrives.
#[derive(Debug)]
pub struct ApiWake;

/// Longest delayed repaint worth scheduling; egui uses huge delays to mean "never".
const MAX_REPAINT_DELAY: Duration = Duration::from_secs(60);

/// Least time between the frames drawn for agent API requests, about 30 a second. A frame waits
/// for the display, so drawing one per request would hold every request up by a frame.
const API_FRAME: Duration = Duration::from_millis(33);

/// How long navigation must pause before the frame that shows the hover it put off.
const HOVER_SETTLE: Duration = Duration::from_millis(50);

/// The document revision, selection and open context the renderer's model buffers show.
type Drawn = (u64, BTreeSet<EntityId>, Vec<InstanceId>);

/// What the window shows that a request can change: the model (its revision), the selection,
/// the open context, the camera, the lighting and the editor's state.
type Shown = (
    u64,
    BTreeSet<EntityId>,
    Vec<InstanceId>,
    Camera,
    Lighting,
    EditorState,
);

/// What `editor`'s window shows that a request can change.
fn shown(editor: &Editor) -> Shown {
    let doc = editor.doc();
    (
        doc.revision(),
        doc.selection().clone(),
        doc.context().to_vec(),
        doc.camera.clone(),
        doc.lighting,
        editor_state(editor),
    )
}

/// What the renderer must redo before drawing a frame.
#[derive(Debug, PartialEq)]
enum SceneUpdate {
    /// Nothing changed.
    Keep,
    /// Only the selection changed: rewrite the selection flags.
    Selection,
    /// The model or the open context changed: rebuild the buffers.
    Model,
}

/// What changed since the renderer's buffers were built from `drawn`.
fn scene_update(
    drawn: Option<&Drawn>,
    revision: u64,
    selection: &BTreeSet<EntityId>,
    context: &[InstanceId],
) -> SceneUpdate {
    match drawn {
        Some((r, s, c)) if *r == revision && c == context => {
            if s == selection {
                SceneUpdate::Keep
            } else {
                SceneUpdate::Selection
            }
        }
        _ => SceneUpdate::Model,
    }
}

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

/// When to draw what agent API requests changed, given when the last frame was presented: now
/// if that was `API_FRAME` ago or more (or never), else once it is.
fn api_repaint(now: Instant, presented: Option<Instant>) -> Repaint {
    match presented {
        Some(t) if now < t + API_FRAME => Repaint::At(t + API_FRAME),
        _ => Repaint::Now,
    }
}

/// The earlier of a pending repaint deadline and `t`.
fn earliest(deadline: Option<Instant>, t: Instant) -> Instant {
    deadline.map_or(t, |d| d.min(t))
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

/// Tapping Ctrl (pressing and releasing it with no other input in between) switches copying;
/// Ctrl held for a trackpad zoom, a click or a shortcut does not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CtrlTap {
    armed: bool,
}

impl CtrlTap {
    /// The modifiers changed from `was` to `now`. Returns true when that completes a tap.
    fn modifiers(&mut self, was: ModifiersState, now: ModifiersState) -> bool {
        let others = |m: ModifiersState| m - ModifiersState::CONTROL;
        let ctrl = (was.control_key(), now.control_key());
        if others(was) != others(now) {
            self.armed = false;
            return false;
        }
        match ctrl {
            (false, true) => {
                self.armed = true;
                false
            }
            (true, false) => std::mem::take(&mut self.armed),
            _ => false,
        }
    }

    /// Other input arrived while Ctrl was down: no tap.
    fn interrupt(&mut self) {
        self.armed = false;
    }
}

/// True if an event egui saw needs another frame. egui asks for a repaint on every
/// `RedrawRequested`, which would draw frames back to back forever; the frame egui itself
/// still needs after painting comes from `Gui::paint` instead.
fn redraws_for_egui(event: &WindowEvent, egui_repaint: bool) -> bool {
    egui_repaint && !matches!(event, WindowEvent::RedrawRequested)
}

/// How long a frame the surface could not give waits before trying again, when nothing else
/// asks for one: a hidden window, or one whose surface reported an error, is not left idle for
/// good if the window's own events and the surface disagree.
const SKIPPED_RETRY: Duration = Duration::from_millis(250);

/// What to do about a frame the surface had no texture for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum AfterSkip {
    /// The surface's configuration is out of date: configure it again and draw at once.
    Reconfigure,
    /// The surface is gone: make a new one, configure it and draw at once.
    Recreate,
    /// Draw again at once (a timeout).
    RetryNow,
    /// Draw again after `SKIPPED_RETRY` (a hidden window, or a surface error).
    RetrySoon,
}

fn after_skip(skipped: &wgpu::CurrentSurfaceTexture) -> AfterSkip {
    use wgpu::CurrentSurfaceTexture as S;
    match skipped {
        S::Outdated => AfterSkip::Reconfigure,
        S::Lost => AfterSkip::Recreate,
        S::Occluded | S::Validation => AfterSkip::RetrySoon,
        S::Timeout | S::Success(_) | S::Suboptimal(_) => AfterSkip::RetryNow,
    }
}

/// A short name for a skipped frame's surface status, for the log.
fn skip_name(skipped: &wgpu::CurrentSurfaceTexture) -> &'static str {
    use wgpu::CurrentSurfaceTexture as S;
    match skipped {
        S::Success(_) | S::Suboptimal(_) => "acquired",
        S::Timeout => "timed out",
        S::Occluded => "occluded",
        S::Outdated => "outdated",
        S::Lost => "lost",
        S::Validation => "a validation error",
    }
}

/// The GPU objects drawing into the window.
struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    /// Set when the device is lost; then it and everything made with it are made again.
    lost: Arc<AtomicBool>,
}

/// A surface for `window` and a device to draw into it, configured at the window's size. GPU
/// errors are logged rather than ending the app, and a lost device is logged and flagged.
fn gpu(instance: &wgpu::Instance, window: &Arc<Window>) -> Result<Gpu, String> {
    let size = window.inner_size();
    let surface = instance
        .create_surface(window.clone())
        .map_err(|e| format!("cannot create a surface: {e}"))?;
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: Some(&surface),
        ..Default::default()
    }))
    .map_err(|e| format!("no compatible GPU adapter: {e}"))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: render::required_limits(&adapter),
        ..Default::default()
    }))
    .map_err(|e| format!("cannot get a GPU device: {e}"))?;
    let lost = Arc::new(AtomicBool::new(false));
    device.set_device_lost_callback({
        let lost = lost.clone();
        move |reason, message| {
            // A device dropped on purpose (made again, or the app closing) is not news.
            if reason != wgpu::DeviceLostReason::Destroyed {
                eprintln!("GPU device lost ({reason:?}): {message}");
                lost.store(true, Ordering::SeqCst);
            }
        }
    });
    device.on_uncaptured_error(Arc::new(|e| eprintln!("GPU error: {e}")));

    let mut config = surface
        .get_default_config(&adapter, size.width.max(1), size.height.max(1))
        .ok_or("surface not supported by adapter")?;
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
    Ok(Gpu {
        surface,
        device,
        queue,
        config,
        lost,
    })
}

/// True for the input that makes a held Ctrl part of a chord rather than a tap: presses,
/// scrolls, pinches, focus changes and keys other than modifiers (the window reports the
/// modifier keys themselves as key events too).
fn interrupts_ctrl_tap(event: &WindowEvent) -> bool {
    match event {
        WindowEvent::KeyboardInput { event, .. } => !is_modifier_key(&event.logical_key),
        _ => matches!(
            event,
            WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::PinchGesture { .. }
                | WindowEvent::Focused(_)
        ),
    }
}

fn is_modifier_key(key: &Key) -> bool {
    matches!(
        key,
        Key::Named(
            NamedKey::Control
                | NamedKey::Shift
                | NamedKey::Alt
                | NamedKey::AltGraph
                | NamedKey::Super
                | NamedKey::Meta
                | NamedKey::Hyper
                | NamedKey::Fn
                | NamedKey::CapsLock
        )
    )
}

/// The editor's view of a wheel event, and whether it starts a new trackpad scroll (fingers
/// down or momentum beginning), which picks a fresh orbit or pan pivot.
fn scroll_input(delta: MouseScrollDelta, phase: TouchPhase) -> (Scroll, bool) {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => (Scroll::Lines(DVec2::new(x as f64, y as f64)), false),
        MouseScrollDelta::PixelDelta(p) => (
            Scroll::Pixels(DVec2::new(p.x, p.y)),
            phase == TouchPhase::Started,
        ),
    }
}

/// The fly mode key a physical key is, wherever the layout puts its letter.
fn fly_move_key(key: PhysicalKey) -> Option<FlyKey> {
    let PhysicalKey::Code(code) = key else {
        return None;
    };
    Some(match code {
        KeyCode::KeyW => FlyKey::W,
        KeyCode::KeyA => FlyKey::A,
        KeyCode::KeyS => FlyKey::S,
        KeyCode::KeyD => FlyKey::D,
        KeyCode::KeyE => FlyKey::E,
        KeyCode::KeyQ => FlyKey::Q,
        KeyCode::Space => FlyKey::Space,
        KeyCode::ControlLeft | KeyCode::ControlRight => FlyKey::Ctrl,
        _ => return None,
    })
}

/// True if a key event goes to fly mode's movement: a moving key while flying, except a press
/// with Cmd held, which stays a shortcut (Cmd+S saves). Releases always go, so a key let go
/// after Cmd went down still stops.
fn is_fly_move(flying: bool, key: PhysicalKey, pressed: bool, cmd: bool) -> Option<FlyKey> {
    fly_move_key(key).filter(|_| flying && !(pressed && cmd))
}

/// A raw mouse motion in logical pixels. macOS reports it in points (logical pixels) already;
/// elsewhere it comes in device pixels, taken as physical ones.
fn look_delta(raw: (f64, f64), scale: f64) -> DVec2 {
    let d = DVec2::new(raw.0, raw.1);
    if cfg!(target_os = "macos") || !(scale.is_finite() && scale > 0.0) {
        d
    } else {
        d / scale
    }
}

/// How fly mode holds the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Capture {
    /// Not flying, or the window is not focused: the pointer is free.
    Free,
    /// Hidden and held in the window: mouse motion looks around.
    Grabbed,
    /// The window could not hold the pointer: it stays visible and looks around while the
    /// left button is held.
    Held,
}

/// Holds the pointer for fly mode: locked in place where the platform can (macOS), else
/// confined to the window. True if either worked.
fn grab_cursor(window: &Window) -> bool {
    window.set_cursor_grab(CursorGrabMode::Locked).is_ok()
        || window.set_cursor_grab(CursorGrabMode::Confined).is_ok()
}

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
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    /// Set when the device is lost (see `gpu`).
    device_lost: Arc<AtomicBool>,
    /// The surface status of the last frame skipped and logged since one was drawn, so a run of
    /// skips is logged once.
    skip_logged: Option<&'static str>,
    renderer: Renderer,
    /// What `render_view` renders with, on the window's device.
    offscreen: Offscreen,
    editor: Editor,
    /// An agent asked the app to quit.
    quit: bool,
    /// The folders the API's file paths must lie within (empty: no limit).
    roots: ApiRoots,
    /// Where discarded work is kept (None: nowhere).
    recovery: Option<PathBuf>,
    /// The Agent Activity window is open.
    agent_activity: bool,
    /// How many of the editor's recovery files the window has already shown.
    recovered_seen: usize,
    gui: Gui,
    /// Document revision, selection and open context the renderer's model buffers were built
    /// from.
    drawn: Option<Drawn>,
    /// The Outliner's rows and the revision they were built at.
    outliner: Option<(u64, OutlinerTable)>,
    /// Each textured material's swatch in the Materials list, with the image it shows.
    swatches: HashMap<MaterialId, (Arc<TextureImage>, egui::TextureHandle)>,
    modifiers: ModifiersState,
    /// Watches for a Ctrl tap, which switches copying (see `CtrlTap`).
    ctrl_tap: CtrlTap,
    /// Where the 3D view sits in the window (physical pixels), from the last frame's layout.
    view: Viewport,
    /// Last cursor position in window coordinates (physical pixels).
    cursor: DVec2,
    /// When to draw the next frame without any input: egui's wish (e.g. a tooltip delay),
    /// API changes waiting for their frame, or a hover put off by navigation.
    repaint_at: Option<Instant>,
    /// When the last frame was presented.
    presented: Option<Instant>,
    /// The window title last set.
    title: String,
    /// When and where the left button was last released in the view, for double-clicks.
    last_release: Option<(Instant, DVec2)>,
    /// Where the left button was last pressed in the view, so a release that ends a drag does
    /// not count as a click.
    left_press: Option<DVec2>,
    /// Whether the window has keyboard focus.
    focused: bool,
    /// How fly mode holds the pointer now.
    capture: Capture,
    /// The left button is down in the view while flying without a grabbed pointer: mouse
    /// motion looks around.
    look_held: bool,
}

impl State {
    fn new(window: Arc<Window>, roots: ApiRoots, recovery: Option<PathBuf>) -> Self {
        let instance = wgpu::Instance::default();
        let Gpu {
            surface,
            device,
            queue,
            config,
            lost,
        } = gpu(&instance, &window).unwrap_or_else(|e| panic!("{e}"));

        let renderer = Renderer::new(&device, &queue, config.format);
        let gui = Gui::new(&window, &device, config.format, ThemeChoice::System);
        let view = Viewport::full((config.width, config.height));
        let offscreen = Offscreen::with(Snapshotter::with_device(&device, &queue));
        let editor = Editor::new(
            DVec2::new(config.width as f64, config.height as f64),
            window.scale_factor(),
        );
        Self {
            window,
            instance,
            surface,
            device,
            queue,
            config,
            device_lost: lost,
            skip_logged: None,
            renderer,
            offscreen,
            editor,
            quit: false,
            roots,
            recovery,
            agent_activity: false,
            recovered_seen: 0,
            gui,
            drawn: None,
            outliner: None,
            swatches: HashMap::new(),
            modifiers: ModifiersState::empty(),
            ctrl_tap: CtrlTap::default(),
            view,
            cursor: DVec2::ZERO,
            repaint_at: None,
            presented: None,
            last_release: None,
            title: String::new(),
            left_press: None,
            focused: true,
            capture: Capture::Free,
            look_held: false,
        }
    }

    /// Holds or frees the pointer to match fly mode: held while flying in the focused window
    /// (hidden and locked, or confined, else visible with left-drag looking), free otherwise,
    /// so another app never gets a hidden or stuck pointer.
    fn sync_fly_cursor(&mut self) {
        let want = self.editor.flying() && self.focused;
        match (want, self.capture) {
            (true, Capture::Free) => {
                self.capture = if grab_cursor(&self.window) {
                    self.window.set_cursor_visible(false);
                    Capture::Grabbed
                } else {
                    Capture::Held
                };
            }
            (false, Capture::Grabbed | Capture::Held) => {
                let _ = self.window.set_cursor_grab(CursorGrabMode::None);
                self.window.set_cursor_visible(true);
                self.capture = Capture::Free;
                self.look_held = false;
            }
            _ => {}
        }
    }

    /// Mouse motion while flying turns the view when the pointer is held, or while the left
    /// button is down in the view when it could not be. True if the view turned.
    fn fly_motion(&mut self, raw: (f64, f64)) -> bool {
        let looks = match self.capture {
            Capture::Grabbed => true,
            Capture::Held => self.look_held,
            Capture::Free => false,
        };
        looks
            && self.focused
            && self
                .editor
                .fly_look(look_delta(raw, self.window.scale_factor()))
    }

    /// True if a press or scroll at the cursor goes to the 3D view: the cursor is in it, egui
    /// is not using the pointer, and no egui window or menu lies over it. This tests the
    /// cursor as it is now, not where egui last saw it, so input right after crossing a panel
    /// edge goes to the side it lands on.
    fn pointer_in_view(&self) -> bool {
        self.in_view(self.cursor) && !self.gui.wants_pointer_at(self.cursor)
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

    /// Gives a material the PNG or JPEG image chosen in a dialog as its texture.
    fn load_texture(&mut self, material: MaterialId) {
        let chosen = rfd::FileDialog::new()
            .add_filter("PNG or JPEG image", &["png", "jpg", "jpeg"])
            .pick_file();
        if let Some(path) = chosen {
            self.editor.load_texture(material, &path);
        }
    }

    /// The Materials list rows, with a swatch image for each texture (made when a material's
    /// image changes; those of materials gone are dropped).
    fn material_rows(&mut self) -> Vec<MaterialRow> {
        let materials: Vec<(MaterialId, kernel::Material)> = self
            .editor
            .doc()
            .scene()
            .materials()
            .map(|(id, m)| (id, m.clone()))
            .collect();
        self.swatches
            .retain(|id, _| materials.iter().any(|(m, _)| m == id));
        materials
            .into_iter()
            .map(|(id, m)| MaterialRow {
                id,
                texture: m.texture.as_ref().map(|t| TextureRow {
                    swatch: self.swatch(id, &t.image),
                    width: t.width,
                    height: t.height,
                    pixels: [t.image.width, t.image.height],
                }),
                name: m.name,
                color: m.color,
                opacity: m.opacity,
                emission: m.emission,
            })
            .collect()
    }

    /// The swatch image of material `id`'s texture `image`.
    fn swatch(&mut self, id: MaterialId, image: &Arc<TextureImage>) -> egui::TextureId {
        if let Some((held, handle)) = self.swatches.get(&id)
            && Arc::ptr_eq(held, image)
        {
            return handle.id();
        }
        let mut pixels =
            render::texture::resample(&image.pixels, image.width, image.height, SWATCH_PIXELS);
        // Textures render opaque, so the swatch ignores the image's alpha.
        pixels.iter_mut().skip(3).step_by(4).for_each(|a| *a = 255);
        let side = SWATCH_PIXELS as usize;
        let handle = self.gui.ctx().load_texture(
            format!("texture-swatch-{}", id.0),
            egui::ColorImage::from_rgba_unmultiplied([side, side], &pixels),
            egui::TextureOptions::LINEAR,
        );
        let shown = handle.id();
        self.swatches.insert(id, (image.clone(), handle));
        shown
    }

    /// Imports an OBJ or STL file chosen in a dialog as a group in the open context.
    fn import(&mut self) {
        let chosen = rfd::FileDialog::new()
            .add_filter("OBJ or STL", &["obj", "stl"])
            .add_filter("OBJ", &["obj"])
            .add_filter("STL", &["stl"])
            .pick_file();
        if let Some(path) = chosen {
            self.editor.import_file(&path);
        }
    }

    /// Exports to a file chosen in a dialog; its extension picks the format. The dialog
    /// suggests the format exported to last.
    fn export(&mut self) {
        let stem = file_name(self.editor.doc().path());
        let stem = stem.strip_suffix(".skr").unwrap_or(&stem);
        let formats = self.editor.export_formats();
        let mut dialog = rfd::FileDialog::new();
        for (name, ext) in &formats {
            dialog = dialog.add_filter(*name, &[*ext]);
        }
        let chosen = dialog
            .set_file_name(format!("{stem}.{}", formats[0].1))
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
            UiAction::ImportFile => self.import(),
            UiAction::SetImportUnits(units) => self.editor.set_import_units(units),
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
            UiAction::DeleteGuides => {
                self.editor.delete_guides();
            }
            UiAction::ZoomExtents => {
                self.editor.zoom_extents();
            }
            UiAction::StandardView(view) => {
                self.editor.standard_view(view);
            }
            UiAction::SetProjection(projection) => {
                self.editor.set_projection(projection);
            }
            UiAction::SetLighting(mode) => {
                self.editor.set_lighting(mode);
            }
            UiAction::ChooseMaterial(material) => {
                self.editor.choose_material(material);
            }
            UiAction::AddMaterial => {
                self.editor.add_material();
            }
            UiAction::DeleteMaterial(material) => {
                self.editor.delete_material(material);
            }
            UiAction::RenameMaterial(material, name) => {
                self.editor.rename_material(material, name);
            }
            UiAction::RecolorMaterial(material, color) => {
                self.editor.recolor_material(material, color);
            }
            UiAction::SetMaterialOpacity(material, percent) => {
                self.editor.set_material_opacity(material, percent);
            }
            UiAction::SetMaterialEmission(material, tenths) => {
                self.editor.set_material_emission(material, tenths);
            }
            UiAction::LoadTexture(material) => self.load_texture(material),
            UiAction::RemoveTexture(material) => {
                self.editor.remove_texture(material);
            }
            UiAction::SetTextureSize(material, millimeters) => {
                self.editor.set_texture_size(material, millimeters);
            }
            UiAction::OutlinerSelect { path, toggle } => {
                self.editor.outliner_select(&path, toggle);
            }
            UiAction::OpenPath(path) => {
                self.editor.open_path(path);
            }
            UiAction::ToggleFly => {
                self.editor.toggle_fly();
            }
            UiAction::ShowAgentActivity(open) => self.agent_activity = open,
            UiAction::ShowSoften(open) => {
                self.editor.show_soften(open);
            }
            UiAction::SetSoftenAngle(angle) => {
                self.editor.set_soften_angle(angle);
            }
            UiAction::Soften => {
                self.editor.soften_selection();
            }
            UiAction::Harden => {
                self.editor.harden_selection();
            }
            UiAction::SetSoft(soft) => {
                self.editor.set_selection_soft(soft);
            }
            UiAction::SetTextureOffset(faces, millimeters) => {
                self.editor.set_texture_offset(faces, millimeters);
            }
            UiAction::SetTextureRotation(faces, tenths) => {
                self.editor.set_texture_rotation(faces, tenths);
            }
            UiAction::SetTextureScale(faces, thousandths) => {
                self.editor.set_texture_scale(faces, thousandths);
            }
            UiAction::RotateTexture90 => {
                self.editor.rotate_texture_90();
            }
            UiAction::ResetTexturePosition => {
                self.editor.reset_texture_position();
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

    /// Makes the device, the surface and everything drawn with them again after the device was
    /// lost. False, logged, when that fails (tried again later).
    fn recreate_gpu(&mut self) -> bool {
        let Gpu {
            surface,
            device,
            queue,
            config,
            lost,
        } = match gpu(&self.instance, &self.window) {
            Ok(gpu) => gpu,
            Err(e) => {
                eprintln!("cannot make the GPU device again: {e}");
                return false;
            }
        };
        eprintln!("GPU device made again");
        self.renderer = Renderer::new(&device, &queue, config.format);
        self.offscreen = Offscreen::with(Snapshotter::with_device(&device, &queue));
        self.gui = Gui::new(&self.window, &device, config.format, ThemeChoice::System);
        // The swatches belonged to the old panels' context.
        self.swatches.clear();
        self.drawn = None;
        self.surface = surface;
        self.device = device;
        self.queue = queue;
        self.config = config;
        self.device_lost = lost;
        true
    }

    /// Lays out and draws the 3D view and the panels, then applies what was clicked in them
    /// and asks for another frame to show the result. Returns true if a panel action ran (so
    /// subscribers should hear about edits).
    fn render(&mut self, hub: &Hub) -> bool {
        if self.device_lost.load(Ordering::SeqCst) && !self.recreate_gpu() {
            self.repaint_at = Some(Instant::now() + SKIPPED_RETRY);
            return false;
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.skip_logged = None;
                frame
            }
            skipped => {
                let name = skip_name(&skipped);
                if self.skip_logged != Some(name) {
                    eprintln!("frame skipped: the window's surface is {name}");
                    self.skip_logged = Some(name);
                }
                match after_skip(&skipped) {
                    AfterSkip::Reconfigure => {
                        self.surface.configure(&self.device, &self.config);
                        self.window.request_redraw();
                    }
                    AfterSkip::Recreate => {
                        match self.instance.create_surface(self.window.clone()) {
                            Ok(surface) => {
                                surface.configure(&self.device, &self.config);
                                self.surface = surface;
                                self.window.request_redraw();
                            }
                            Err(e) => {
                                eprintln!("cannot create a surface: {e}");
                                self.repaint_at = Some(Instant::now() + SKIPPED_RETRY);
                            }
                        }
                    }
                    // Fly mode would otherwise freeze until the next input.
                    AfterSkip::RetryNow => self.window.request_redraw(),
                    AfterSkip::RetrySoon => {
                        self.repaint_at = Some(Instant::now() + SKIPPED_RETRY);
                    }
                }
                return false;
            }
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let size = (self.config.width, self.config.height);

        // Fly mode moves the eye by the time since the last frame.
        let flying_on = self.editor.fly_tick(Instant::now());
        // The hover is worked out once per frame, not per event.
        self.editor.refresh_hover();
        let doc = self.editor.doc();
        let title = window_title(doc.path(), doc.is_modified());
        if title != self.title {
            self.window.set_title(&title);
            self.title = title;
        }
        let status = self.editor.status_parts();
        let info = entity_info(self.editor.doc());
        let materials = self.material_rows();
        // Rebuilt only when the model changes; the open and selected marks are worked out
        // while drawing, from the context and selection.
        let revision = self.editor.doc().revision();
        if self.outliner.as_ref().map(|(r, _)| *r) != Some(revision) {
            self.outliner = Some((revision, self.editor.doc().outliner_rows()));
        }
        let context: Vec<EntityId> = self
            .editor
            .doc()
            .context()
            .iter()
            .map(|i| EntityId::Instance(*i))
            .collect();
        // A discard an agent made opens the window that lists where its work went.
        if self.editor.recovered().len() > self.recovered_seen {
            self.recovered_seen = self.editor.recovered().len();
            self.agent_activity = true;
        }
        let now = Instant::now();
        let lines: Option<Vec<String>> = self.agent_activity.then(|| {
            hub.activity()
                .iter()
                .rev()
                .map(|a| activity_line(a, now))
                .collect()
        });
        let panels = View {
            undo: self.editor.doc().undo_label(),
            redo: self.editor.doc().redo_label(),
            agents: Agents {
                connected: hub.open_connections(),
                log: lines.as_deref(),
                recovered: self.editor.recovered(),
            },
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
            has_guides: self.editor.doc().model().guides().next().is_some(),
            projection: self.editor.doc().camera.projection,
            lighting: self.editor.doc().lighting,
            flying: self.editor.flying(),
            import_units: self.editor.import_units(),
            materials: &materials,
            material: self.editor.current_material(),
            outliner: self.outliner.as_ref().map(|(_, t)| t).expect("built above"),
            context: &context,
            selection: self.editor.doc().selection(),
            soften: self.editor.soften_window(),
            edges_soft: self.editor.selection_soft(),
            texture_position: self.editor.selection_texture_position().map(|placement| {
                TexturePosition {
                    placement,
                    faces: self.editor.doc().selection().iter().copied().collect(),
                }
            }),
        };
        let mut gui_frame = self.gui.frame(&self.window, &panels);
        gui_frame.hide_cursor = self.capture == Capture::Grabbed;
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
        let update = scene_update(
            self.drawn.as_ref(),
            doc.revision(),
            doc.selection(),
            doc.context(),
        );
        if update == SceneUpdate::Model {
            self.renderer.set_model(doc.scene(), doc.context());
        }
        if update != SceneUpdate::Keep {
            self.renderer.set_selection(doc.selection());
            self.drawn = Some((
                doc.revision(),
                doc.selection().clone(),
                doc.context().to_vec(),
            ));
        }
        // The view follows the panels' theme; render_view keeps its own renderer's classic look.
        self.renderer.set_style(theme::view_style(self.gui.theme()));
        self.renderer.set_lighting(doc.lighting);
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
        let now = Instant::now();
        self.presented = Some(now);
        self.repaint_at = None;
        match schedule_repaint(now, again) {
            Repaint::Now => self.window.request_redraw(),
            Repaint::At(t) => self.repaint_at = Some(t),
            Repaint::Idle => {}
        }
        if flying_on {
            self.window.request_redraw();
        }
        // The ages in the open log move.
        if lines.is_some() {
            self.repaint_at = Some(earliest(self.repaint_at, now + Duration::from_secs(1)));
        }
        if self.editor.hover_deferred() {
            self.repaint_at = Some(earliest(self.repaint_at, now + HOVER_SETTLE));
        }
        for action in actions {
            self.apply(action);
        }
        if acted {
            self.window.request_redraw();
        }
        acted
    }

    /// Handles waiting API requests, answering each at once. If they changed the model,
    /// selection, open context, camera or editor, or may have changed the hover, a frame shows
    /// it: at most about 30 a second while requests keep coming (see `api_repaint`).
    fn process_api(&mut self, hub: &mut Hub) {
        let before = shown(&self.editor);
        let agents_before = (hub.open_connections(), hub.activity_revision());
        let mut host = self.host();
        hub.process(&mut host);
        let agents_changed = hub.open_connections() != agents_before.0
            || (self.agent_activity && hub.activity_revision() != agents_before.1);
        // An agent may have turned fly mode on or off: the pointer follows.
        self.sync_fly_cursor();
        if shown(&self.editor) != before || self.editor.hover_pending() || agents_changed {
            match api_repaint(Instant::now(), self.presented) {
                Repaint::At(t) => self.repaint_at = Some(earliest(self.repaint_at, t)),
                _ => self.window.request_redraw(),
            }
        }
    }

    /// The API's view of the window: the editor, the offscreen renderer and the quit flag.
    fn host(&mut self) -> AppHost<'_> {
        AppHost {
            editor: &mut self.editor,
            offscreen: &mut self.offscreen,
            quit: &mut self.quit,
            roots: &self.roots,
            recovery: self.recovery.as_deref(),
        }
    }

    /// Tells API subscribers about edits made in the window.
    fn notify_api(&mut self, hub: &mut Hub) {
        let host = self.host();
        hub.notify(&host);
    }

    /// Tells API subscribers what changed of the selection, camera and editor since the last
    /// frame, once per frame drawn.
    fn notify_frame(&mut self, hub: &mut Hub) {
        let host = self.host();
        hub.notify_state(&host);
    }
}

/// A line of the Agent Activity log: how long ago, the connection, the request and how it
/// went. One line: control and format characters in a caller's text become spaces.
fn activity_line(a: &api::Activity, now: Instant) -> String {
    let secs = now.saturating_duration_since(a.at).as_secs();
    let ago = if secs < 120 {
        format!("{secs} s ago")
    } else if secs < 7200 {
        format!("{} min ago", secs / 60)
    } else {
        format!("{} h ago", secs / 3600)
    };
    let result = match &a.error {
        None => "ok".to_string(),
        Some(e) => format!("error: {e}"),
    };
    let mut line = format!("{ago}  #{}  {}  {result}", a.connection, a.request);
    if a.repeats > 1 {
        line += &format!("  (x{})", a.repeats);
    }
    line.chars()
        .map(|c| if hidden(c) { ' ' } else { c })
        .collect()
}

/// Characters that do not show as themselves in a log line: controls, line breaks and other
/// whitespace than the space, and format characters (Unicode Cf), among them the bidi
/// overrides and isolates that could make a path read as another.
fn hidden(c: char) -> bool {
    c.is_control()
        || (c.is_whitespace() && c != ' ')
        || matches!(
            c,
            '\u{ad}'
                | '\u{600}'..='\u{605}'
                | '\u{61c}'
                | '\u{6dd}'
                | '\u{70f}'
                | '\u{890}'..='\u{891}'
                | '\u{8e2}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206f}'
                | '\u{feff}'
                | '\u{fff9}'..='\u{fffb}'
                | '\u{110bd}'
                | '\u{110cd}'
                | '\u{13430}'..='\u{1343f}'
                | '\u{1bca0}'..='\u{1bca3}'
                | '\u{1d173}'..='\u{1d17a}'
                | '\u{e0001}'
                | '\u{e0020}'..='\u{e007f}'
        )
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
            PhysicalKey::Code(KeyCode::KeyI) => Some('i'),
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
/// the physical key so shortcuts and digits work on any layout. A letter is uppercase exactly
/// when Shift is held, so Caps Lock does not turn "g" (make group) into "G" (make component).
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
            let c = if c.is_ascii() {
                c
            } else {
                physical_char(physical)?
            };
            Some(EditorKey::Char(if !c.is_ascii_alphabetic() {
                c
            } else if shift {
                c.to_ascii_uppercase()
            } else {
                c.to_ascii_lowercase()
            }))
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
        KeyCode::KeyA => 'a',
        KeyCode::KeyQ => 'q',
        KeyCode::KeyT => 't',
        KeyCode::KeyZ => 'z',
        KeyCode::KeyF => 'f',
        KeyCode::BracketLeft => '[',
        KeyCode::BracketRight => ']',
        KeyCode::Equal => '=',
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
    /// Held while agents are connected, so macOS does not nap the window they drive.
    awake: Awake,
    /// Socket the agent API listens on, removed on exit (None if listening failed).
    socket: Option<PathBuf>,
    /// A model to open once the window exists.
    file: Option<PathBuf>,
    /// The folders the API's file paths must lie within.
    roots: ApiRoots,
    /// Where work an agent discards is kept.
    recovery: Option<PathBuf>,
}

impl App {
    /// `socket` is where the agent API listens; None runs without it. `file` is a model to
    /// open at start.
    pub fn new(
        event_loop: &EventLoop<ApiWake>,
        socket: Option<PathBuf>,
        file: Option<PathBuf>,
        roots: ApiRoots,
        recovery: Option<PathBuf>,
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
            awake: Awake::default(),
            socket,
            file,
            roots,
            recovery,
        }
    }
}

impl ApplicationHandler<ApiWake> for App {
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // The last replies (a `quit`'s) reach their agents before the app ends.
        self.hub.close(CLOSE_WAIT);
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

    fn user_event(&mut self, event_loop: &ActiveEventLoop, _event: ApiWake) {
        // Wake before the work, not after: a napped app runs a request (a save, say) many
        // times slower, and an agent's first request often arrives with its connection.
        let state = &mut self.state;
        let hub = &mut self.hub;
        self.awake.serve(|_| {
            if let Some(s) = state.as_mut() {
                s.process_api(hub);
            }
            hub.open_connections() > 0
        });
        // An agent asked to quit; any unsaved changes were refused or discarded already.
        if self.state.as_ref().is_some_and(|s| s.quit) {
            event_loop.exit();
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attrs = window_attributes();
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let mut state = State::new(window, self.roots.clone(), self.recovery.clone());
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
        if state.quit {
            event_loop.exit();
        }
        self.state = Some(state);
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        let Some(s) = self.state.as_mut() else { return };
        if let DeviceEvent::MouseMotion { delta } = event
            && s.fly_motion(delta)
        {
            s.window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(s) = self.state.as_mut() else { return };
        let egui_repaint = s.gui.on_event(&s.window, &event);
        if redraws_for_egui(&event, egui_repaint) {
            s.window.request_redraw();
        }
        if interrupts_ctrl_tap(&event) {
            s.ctrl_tap.interrupt();
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
                if s.render(&self.hub) {
                    s.notify_api(&mut self.hub);
                }
                s.notify_frame(&mut self.hub);
                false
            }
            WindowEvent::ModifiersChanged(m) => {
                let tapped = s.ctrl_tap.modifiers(s.modifiers, m.state());
                s.modifiers = m.state();
                let locked = s
                    .editor
                    .set_modifiers(m.state().shift_key(), m.state().alt_key());
                // Cmd or Ctrl held makes a Paint click pick up a material.
                s.editor
                    .set_sample_modifier(m.state().super_key() || m.state().control_key());
                // Tapping Ctrl switches tools that can copy (Move, Rotate) between moving and
                // copying, and the Tape Measure between adding guides and measuring only.
                let toggled = tapped && s.editor.toggle_copy();
                locked || toggled
            }
            WindowEvent::Focused(false) => {
                // Releases may never arrive: no click or double-click spans the focus change.
                s.left_press = None;
                s.last_release = None;
                s.focused = false;
                s.editor.focus_lost()
            }
            WindowEvent::Focused(true) => {
                s.focused = true;
                false
            }
            // The frames skipped while the window was hidden (see `retries_at_once`).
            WindowEvent::Occluded(false) => true,
            WindowEvent::CursorMoved { position, .. } => {
                s.cursor_moved(DVec2::new(position.x, position.y))
            }
            WindowEvent::CursorLeft { .. } => s.editor.cursor_left(),
            // While flying, clicks do nothing to the model; without a grabbed pointer the left
            // button held in the view looks around.
            WindowEvent::MouseInput { state, button, .. } if s.editor.flying() => {
                if button == MouseButton::Left {
                    s.look_held = state == ElementState::Pressed && s.pointer_in_view();
                }
                false
            }
            // Presses go to the view only when they land in it; releases always do, so a drag
            // that ends over a panel still finishes.
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                if pressed && !s.pointer_in_view() {
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
            // A held pointer may rest over a panel; while flying, scrolls still fly.
            WindowEvent::MouseWheel { .. } | WindowEvent::PinchGesture { .. }
                if !s.editor.flying() && !s.pointer_in_view() =>
            {
                false
            }
            WindowEvent::KeyboardInput { .. } if s.gui.wants_keyboard() => false,
            WindowEvent::KeyboardInput { event, .. }
                if let Some(key) = is_fly_move(
                    s.editor.flying(),
                    event.physical_key,
                    event.state == ElementState::Pressed,
                    s.modifiers.super_key(),
                ) =>
            {
                s.editor.fly_key(key, event.state == ElementState::Pressed)
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                let (scroll, started) = scroll_input(delta, phase);
                if started {
                    s.editor.scroll_started();
                }
                s.editor.scroll(scroll)
            }
            WindowEvent::PinchGesture { delta, .. } => s.editor.pinch(delta),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let command_key = s.modifiers.super_key() || s.modifiers.control_key();
                let letter = shortcut_letter(&event.logical_key, event.physical_key);
                if command_key {
                    let shift = s.modifiers.shift_key();
                    match letter.and_then(|l| shortcuts::command(l, shift, event.repeat)) {
                        Some(Bound::Quit) => {
                            if s.settle_changes() {
                                event_loop.exit();
                            }
                            false
                        }
                        // Redraw only if there was a step to take back or redo.
                        Some(Bound::Undo) => s.editor.undo(),
                        Some(Bound::Redo) => s.editor.redo(),
                        Some(bound) => {
                            if let Some(action) = bound.action() {
                                s.apply(action);
                            }
                            true
                        }
                        None => false,
                    }
                } else if s.modifiers.alt_key() {
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
        // Whatever turned fly mode on or off (a key, the menu, focus), the pointer follows.
        s.sync_fly_cursor();
        if changed {
            s.window.request_redraw();
            s.notify_api(&mut self.hub);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn activity_lines_read_newest_first_with_age_and_result() {
        let now = Instant::now();
        let a = api::Activity {
            at: now - Duration::from_secs(12),
            connection: 3,
            request: "execute save /a/house.skr".into(),
            error: None,
            repeats: 1,
        };
        assert_eq!(
            activity_line(&a, now),
            "12 s ago  #3  execute save /a/house.skr  ok"
        );
        let b = api::Activity {
            error: Some("no".into()),
            repeats: 4,
            at: now,
            ..a
        };
        assert_eq!(
            activity_line(&b, now),
            "0 s ago  #3  execute save /a/house.skr  error: no  (x4)"
        );
        let c = api::Activity {
            at: now - Duration::from_secs(3 * 60 + 5),
            ..b
        };
        assert!(activity_line(&c, now).starts_with("3 min ago"));
        let d = api::Activity {
            request: "execute open /a\nb\u{1b}[0m".into(),
            ..c
        };
        assert!(!activity_line(&d, now).contains(char::is_control));
        // Bidi overrides, isolates and other format characters cannot reorder the line.
        let e = api::Activity {
            request: "execute save /a/\u{202e}piz.skr\u{2066}x\u{2028}y\u{200b}".into(),
            ..d
        };
        let line = activity_line(&e, now);
        assert!(line.contains("/a/ piz.skr x y "), "{line}");
    }

    use super::*;
    use winit::keyboard::SmolStr;

    #[test]
    fn the_window_cannot_shrink_below_960_by_600() {
        use winit::dpi::Size;
        let attrs = window_attributes();
        assert_eq!(
            attrs.min_inner_size,
            Some(Size::Logical(LogicalSize::new(960.0, 600.0)))
        );
        assert_eq!(
            attrs.inner_size,
            Some(Size::Logical(LogicalSize::new(1280.0, 800.0)))
        );
    }

    #[test]
    fn a_request_that_changes_only_the_editor_still_shows() {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        let before = shown(&e);
        assert_eq!(shown(&e), before);
        e.set_import_units(crate::editor::ImportUnits::Feet);
        assert_ne!(shown(&e), before);
    }

    #[test]
    fn only_a_ctrl_tap_switches_copying() {
        let none = ModifiersState::empty();
        let ctrl = ModifiersState::CONTROL;
        let mut tap = CtrlTap::default();
        assert!(!tap.modifiers(none, ctrl));
        assert!(tap.modifiers(ctrl, none), "press and release");
        // Ctrl held for a trackpad zoom (or a click, a key, a pinch).
        assert!(!tap.modifiers(none, ctrl));
        tap.interrupt();
        assert!(!tap.modifiers(ctrl, none));
        // Ctrl with another modifier.
        assert!(!tap.modifiers(none, ctrl));
        assert!(!tap.modifiers(ctrl, ctrl | ModifiersState::SHIFT));
        assert!(!tap.modifiers(ctrl | ModifiersState::SHIFT, ModifiersState::SHIFT));
        // A release alone is no tap.
        assert!(!tap.modifiers(ctrl, none));
    }

    #[test]
    fn pointer_and_key_input_interrupt_a_ctrl_tap() {
        let id = winit::event::DeviceId::dummy();
        let wheel = WindowEvent::MouseWheel {
            device_id: id,
            delta: MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 4.0)),
            phase: TouchPhase::Moved,
        };
        let pinch = WindowEvent::PinchGesture {
            device_id: id,
            delta: 0.01,
            phase: TouchPhase::Moved,
        };
        let press = WindowEvent::MouseInput {
            device_id: id,
            state: ElementState::Pressed,
            button: MouseButton::Left,
        };
        assert!(interrupts_ctrl_tap(&wheel));
        assert!(interrupts_ctrl_tap(&pinch));
        assert!(interrupts_ctrl_tap(&press));
        assert!(interrupts_ctrl_tap(&WindowEvent::Focused(false)));
        let moved = WindowEvent::CursorMoved {
            device_id: id,
            position: winit::dpi::PhysicalPosition::new(1.0, 2.0),
        };
        assert!(
            !interrupts_ctrl_tap(&moved),
            "the cursor drifts during a tap"
        );
        assert!(is_modifier_key(&Key::Named(NamedKey::Control)));
        assert!(!is_modifier_key(&Key::Character("z".into())));
    }

    #[test]
    fn wheel_events_map_to_lines_or_pixels_and_trackpad_starts() {
        let lines = scroll_input(MouseScrollDelta::LineDelta(0.0, 2.0), TouchPhase::Started);
        assert_eq!(lines, (Scroll::Lines(DVec2::new(0.0, 2.0)), false));
        let px = |phase| {
            scroll_input(
                MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(3.0, -4.0)),
                phase,
            )
        };
        assert_eq!(
            px(TouchPhase::Started),
            (Scroll::Pixels(DVec2::new(3.0, -4.0)), true)
        );
        assert!(!px(TouchPhase::Moved).1);
        assert!(!px(TouchPhase::Ended).1);
    }

    #[test]
    fn a_drawn_frame_does_not_ask_for_another() {
        assert!(!redraws_for_egui(&WindowEvent::RedrawRequested, true));
        assert!(redraws_for_egui(&WindowEvent::Occluded(false), true));
        assert!(!redraws_for_egui(&WindowEvent::Occluded(false), false));
    }

    #[test]
    fn a_skipped_frame_is_always_tried_again() {
        use wgpu::CurrentSurfaceTexture as S;
        assert_eq!(after_skip(&S::Timeout), AfterSkip::RetryNow);
        assert_eq!(after_skip(&S::Outdated), AfterSkip::Reconfigure);
        assert_eq!(after_skip(&S::Lost), AfterSkip::Recreate);
        // A hidden window, or a surface error (as after a lost device), tries again shortly
        // rather than waiting for an event that may not come.
        assert_eq!(after_skip(&S::Occluded), AfterSkip::RetrySoon);
        assert_eq!(after_skip(&S::Validation), AfterSkip::RetrySoon);
        assert_eq!(skip_name(&S::Validation), "a validation error");
    }

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
        assert_eq!(shortcut_letter(&ch("ه"), code(KeyCode::KeyI)), Some('i'));
        assert_eq!(shortcut_letter(&ch("Q"), code(KeyCode::KeyQ)), Some('q'));
        assert_eq!(shortcut_letter(&ch("ث"), code(KeyCode::KeyE)), Some('e'));
    }

    #[test]
    fn every_command_letter_has_a_physical_key_fallback() {
        use crate::shortcuts::{Key as ShortcutKey, TABLE};
        let code = |c: char| match c {
            'n' => KeyCode::KeyN,
            'o' => KeyCode::KeyO,
            's' => KeyCode::KeyS,
            'i' => KeyCode::KeyI,
            'e' => KeyCode::KeyE,
            'q' => KeyCode::KeyQ,
            'z' => KeyCode::KeyZ,
            other => panic!("add {other} to shortcut_letter's fallbacks and here"),
        };
        let arabic = Key::Character(SmolStr::new("ب"));
        for s in TABLE.iter().filter(|s| s.chord.command) {
            let ShortcutKey::Letter(c) = s.chord.key else {
                continue;
            };
            assert_eq!(
                shortcut_letter(&arabic, PhysicalKey::Code(code(c))),
                Some(c),
                "{:?}",
                s.bound
            );
        }
    }

    #[test]
    fn api_frames_come_at_most_every_api_frame() {
        let now = Instant::now();
        let ms = Duration::from_millis;
        assert_eq!(api_repaint(now, None), Repaint::Now);
        assert_eq!(api_repaint(now, Some(now - ms(40))), Repaint::Now);
        assert_eq!(api_repaint(now, Some(now - API_FRAME)), Repaint::Now);
        assert_eq!(
            api_repaint(now, Some(now - ms(10))),
            Repaint::At(now - ms(10) + API_FRAME)
        );
        assert_eq!(earliest(None, now), now);
        assert_eq!(earliest(Some(now + ms(5)), now + ms(9)), now + ms(5));
        assert_eq!(earliest(Some(now + ms(9)), now + ms(5)), now + ms(5));
    }

    #[test]
    fn only_a_model_or_context_change_rebuilds_the_scene() {
        let (a, b) = (InstanceId(1), InstanceId(2));
        let picked = BTreeSet::from([EntityId::Instance(b)]);
        let drawn: Drawn = (7, BTreeSet::new(), vec![a]);
        let none = BTreeSet::new();
        assert_eq!(scene_update(None, 7, &none, &[a]), SceneUpdate::Model);
        assert_eq!(
            scene_update(Some(&drawn), 7, &none, &[a]),
            SceneUpdate::Keep
        );
        assert_eq!(
            scene_update(Some(&drawn), 7, &picked, &[a]),
            SceneUpdate::Selection
        );
        assert_eq!(
            scene_update(Some(&drawn), 8, &none, &[a]),
            SceneUpdate::Model
        );
        assert_eq!(
            scene_update(Some(&drawn), 8, &picked, &[a]),
            SceneUpdate::Model
        );
        assert_eq!(
            scene_update(Some(&drawn), 7, &none, &[]),
            SceneUpdate::Model
        );
        assert_eq!(
            scene_update(Some(&drawn), 7, &picked, &[a, b]),
            SceneUpdate::Model
        );
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
        let q = PhysicalKey::Code(KeyCode::KeyQ);
        assert_eq!(ch(editor_key(&arabic, q, false)), Some('q'));
        let t = PhysicalKey::Code(KeyCode::KeyT);
        assert_eq!(ch(editor_key(&arabic, t, false)), Some('t'));
        let a = PhysicalKey::Code(KeyCode::KeyA);
        assert_eq!(ch(editor_key(&arabic, a, false)), Some('a'));
        let z = PhysicalKey::Code(KeyCode::KeyZ);
        assert_eq!(
            ch(editor_key(&arabic, z, true)),
            Some('Z'),
            "Shift+Z zooms extents"
        );
        // Caps Lock changes the character the layout gives, not the shortcut.
        let caps = Key::Character(SmolStr::new("G"));
        assert_eq!(ch(editor_key(&caps, g, false)), Some('g'));
        let shifted = Key::Character(SmolStr::new("g"));
        assert_eq!(ch(editor_key(&shifted, g, true)), Some('G'));
        let digit = Key::Character(SmolStr::new("5"));
        let five = PhysicalKey::Code(KeyCode::Digit5);
        assert_eq!(ch(editor_key(&digit, five, true)), Some('5'));
    }

    #[test]
    fn fly_keys_are_physical_and_cmd_presses_stay_shortcuts() {
        let code = |c| PhysicalKey::Code(c);
        assert_eq!(fly_move_key(code(KeyCode::KeyW)), Some(FlyKey::W));
        assert_eq!(fly_move_key(code(KeyCode::KeyA)), Some(FlyKey::A));
        assert_eq!(fly_move_key(code(KeyCode::KeyS)), Some(FlyKey::S));
        assert_eq!(fly_move_key(code(KeyCode::KeyD)), Some(FlyKey::D));
        assert_eq!(fly_move_key(code(KeyCode::KeyE)), Some(FlyKey::E));
        assert_eq!(fly_move_key(code(KeyCode::KeyQ)), Some(FlyKey::Q));
        assert_eq!(fly_move_key(code(KeyCode::Space)), Some(FlyKey::Space));
        assert_eq!(fly_move_key(code(KeyCode::ControlLeft)), Some(FlyKey::Ctrl));
        assert_eq!(
            fly_move_key(code(KeyCode::ControlRight)),
            Some(FlyKey::Ctrl)
        );
        assert_eq!(fly_move_key(code(KeyCode::KeyF)), None);
        let s = code(KeyCode::KeyS);
        assert_eq!(is_fly_move(true, s, true, false), Some(FlyKey::S));
        assert_eq!(is_fly_move(false, s, true, false), None, "not flying");
        assert_eq!(is_fly_move(true, s, true, true), None, "Cmd+S saves");
        assert_eq!(
            is_fly_move(true, s, false, true),
            Some(FlyKey::S),
            "a release always stops"
        );
    }

    #[test]
    fn fly_speed_and_exit_keys_work_on_any_layout() {
        let ch = |k: Option<EditorKey>| match k {
            Some(EditorKey::Char(c)) => Some(c),
            _ => None,
        };
        let arabic = Key::Character(SmolStr::new("ب"));
        let code = |c| PhysicalKey::Code(c);
        assert_eq!(
            ch(editor_key(&arabic, code(KeyCode::KeyF), false)),
            Some('f')
        );
        assert_eq!(
            ch(editor_key(&arabic, code(KeyCode::BracketLeft), false)),
            Some('[')
        );
        assert_eq!(
            ch(editor_key(&arabic, code(KeyCode::BracketRight), false)),
            Some(']')
        );
        assert_eq!(
            ch(editor_key(&arabic, code(KeyCode::Equal), false)),
            Some('=')
        );
    }

    #[test]
    fn mouse_motion_looks_in_logical_pixels() {
        let d = look_delta((10.0, -4.0), 2.0);
        if cfg!(target_os = "macos") {
            assert_eq!(d, DVec2::new(10.0, -4.0), "macOS reports points");
        } else {
            assert_eq!(d, DVec2::new(5.0, -2.0));
        }
        assert_eq!(look_delta((3.0, 1.0), f64::NAN), DVec2::new(3.0, 1.0));
    }
}
