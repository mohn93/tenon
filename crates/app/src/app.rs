use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use api::{ApiRoots, EditorState, Hub};
#[cfg(not(target_arch = "wasm32"))]
use api::{Endpoint, listen};
use glam::DVec2;
use kernel::{EntityId, InstanceId, MaterialId, TagId, TextureImage};
use render::{Renderer, Snapshotter, Viewport};
use session::{Camera, Command, Lighting, OutlinerTable};
use tools::{Axis, NoteRequest, TextSpec};
use web_time::{Instant, SystemTime};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{
    DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, MouseScrollDelta, TouchPhase,
    WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowAttributes, WindowId};

mod library_panel;

#[cfg(not(target_arch = "wasm32"))]
use crate::awake::Awake;
use crate::editor::{
    DRAG_LOGICAL_PX, Editor, EditorKey, PrintJob, PrintOptions, PrintPurpose, PrintQueue,
};
use crate::feedback::Timing;
use crate::flows::{Effect, Env, Flows, PlatformReply, Then, file_name};
use crate::fly::FlyKey;
use crate::gui::Gui;
use crate::host::{AppHost, CLOSE_WAIT, Offscreen, editor_state};
use crate::info::entity_info_with;
#[cfg(target_os = "macos")]
use crate::menus;
#[cfg(target_os = "macos")]
use crate::native_menu::{self, NativeMenu};
use crate::navigation::{Button, Scroll};
use crate::paths;
use crate::platform::{Features, Platform, UnsavedChoice};
use crate::prefs::{self, Prefs};
use crate::recovery::{self, Session};
use crate::shortcuts::{self, Bound};
use crate::touch::{self, TouchEvent, Touches};
#[cfg(not(target_arch = "wasm32"))]
use crate::ui::PhoneView;
use crate::ui::theme::UiScale;
use crate::ui::{
    Agents, DockTab, JointRow, LibrarySave, LibraryView, MaterialRow, RecoverRow, RecoverView,
    SceneRow, SettingsView, Sheet, StartCard, StyleRow, TagRow, TexturePosition, TextureRow,
    UiAction, View,
};
use crate::ui::{ViewerPanel, ViewerView};
use crate::ui::{theme, viewport};

/// The window's least size, in points: the panels and a usable 3D view.
const MIN_WINDOW: LogicalSize<f64> = LogicalSize::new(960.0, 640.0);

/// The smallest window: 960 x 640 points at `scale`, in logical pixels, so the panels always
/// get at least `MIN_WINDOW` points.
fn min_window_size(scale: UiScale) -> LogicalSize<f64> {
    let factor = f64::from(scale.percent()) / 100.0;
    LogicalSize::new(MIN_WINDOW.width * factor, MIN_WINDOW.height * factor)
}

/// The size a window of `size` must grow to so it is at least `min` each way; None when it
/// already is.
fn grown_to_min(size: LogicalSize<f64>, min: LogicalSize<f64>) -> Option<LogicalSize<f64>> {
    (size.width < min.width || size.height < min.height)
        .then(|| LogicalSize::new(size.width.max(min.width), size.height.max(min.height)))
}

/// Room left on a monitor around the window, in points: the menu bar and the title bar above
/// it, and a little each side. winit gives no work area, so this stands in for one.
const MONITOR_MARGIN: LogicalSize<f64> = LogicalSize::new(16.0, 64.0);

/// `min` held to what fits on a monitor of `monitor` points less `MONITOR_MARGIN`, so a large
/// UI scale on a small display does not ask for a window bigger than the screen. With no
/// monitor known, `min` as is.
fn fit_to_monitor(min: LogicalSize<f64>, monitor: Option<LogicalSize<f64>>) -> LogicalSize<f64> {
    match monitor {
        Some(m) => LogicalSize::new(
            min.width.min((m.width - MONITOR_MARGIN.width).max(0.0)),
            min.height.min((m.height - MONITOR_MARGIN.height).max(0.0)),
        ),
        None => min,
    }
}

/// Holds `window` to the least size for `scale` (fitted to its monitor), growing it now if it
/// is smaller (a minimum alone may not resize a window that is already open).
fn set_min_window(window: &Window, scale: UiScale) {
    // A browser's window is its canvas: the page's layout sizes it (a minimum or a size set
    // here would pin the canvas's style).
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let monitor = window
        .current_monitor()
        .map(|m| m.size().to_logical(m.scale_factor()));
    let min = fit_to_monitor(min_window_size(scale), monitor);
    window.set_min_inner_size(Some(min));
    let size = window.inner_size().to_logical(window.scale_factor());
    if let Some(grown) = grown_to_min(size, min) {
        let _ = window.request_inner_size(grown);
    }
}

/// The largest surface at most `max` pixels on a side that keeps the aspect ratio of
/// `width` by `height` (a window larger than the GPU's textures must not fail to configure).
fn fit_surface(width: u32, height: u32, max: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= max || max == 0 {
        return (width, height);
    }
    let scale = f64::from(max) / f64::from(longest);
    let fit = |side: u32| ((f64::from(side) * scale).floor() as u32).clamp(1, max);
    (fit(width), fit(height))
}

/// The window as it opens: title, default size and least size (at 100%; `State::new` sets the
/// one for the UI scale). Hidden: the AccessKit adapter must be made before the window is first
/// shown, so `resumed` shows it once the adapter exists.
fn window_attributes() -> WindowAttributes {
    Window::default_attributes()
        .with_title("sketchup-alt")
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_min_inner_size(MIN_WINDOW)
        .with_visible(false)
}

/// The side of a texture's swatch image in the Materials list, in pixels.
const SWATCH_PIXELS: u32 = 32;

/// Sent to the event loop from other threads and from AppKit.
#[derive(Debug)]
pub enum AppEvent {
    /// An agent API request arrived.
    Api,
    /// A native menu item was chosen; `repeat` when a held key chose it.
    #[cfg(target_os = "macos")]
    Menu { id: String, repeat: bool },
    /// The system asked to quit with unsaved changes (Dock > Quit, log out, shut down) and
    /// waits for the window's answer (see `system_quit`).
    #[cfg(target_os = "macos")]
    SystemQuit,
    /// The AccessKit adapter: a screen reader asked for the tree, asked for an action, or went
    /// away.
    AccessKit(egui_winit::accesskit_winit::Event),
    /// The platform answered a request it had left pending (a file picked, a question
    /// answered).
    Platform(PlatformReply),
    /// The page's paste event: `text` goes to the focused text field, else the model pastes
    /// its clip (the platform's clipboard already holds `text`).
    Paste(String),
    /// Something a late platform request could not do, for the status line.
    Error(String),
    /// A check for updates finished.
    Update(crate::update::Outcome),
    /// The system asked to open these files (macOS: a `.skr` double-clicked in Finder or
    /// dropped on the Dock icon, see `open_files`).
    OpenFiles(Vec<PathBuf>),
    /// The viewer was asked to open another shared model (a kept one from the Recent list).
    Shared(SharedModel),
}

impl From<egui_winit::accesskit_winit::Event> for AppEvent {
    fn from(event: egui_winit::accesskit_winit::Event) -> Self {
        AppEvent::AccessKit(event)
    }
}

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
/// the open context, the camera, the lighting, coloring by tag, the section display, X-ray,
/// Hidden Geometry, the chosen style, the selected scene, the shadow settings and the location (the sun's inputs)
/// and the editor's state.
type Shown = (
    u64,
    BTreeSet<EntityId>,
    Vec<InstanceId>,
    Camera,
    Lighting,
    bool,
    session::SectionDisplay,
    (
        bool,
        bool,
        Option<kernel::StyleId>,
        Option<kernel::PageId>,
        kernel::Shadows,
        kernel::Location,
    ),
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
        doc.color_by_tag,
        doc.section_display,
        (
            doc.x_ray,
            doc.hidden_geometry,
            doc.style(),
            doc.selected_scene(),
            doc.shadows,
            doc.scene().location(),
        ),
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

/// The repaint deadline after a frame whose HUD was dressed at `shown_at`: `pending`, or
/// sooner when the HUD changes by itself before (the rest's end, when it reveals a `chip`; the
/// flash's end).
fn with_feedback(
    pending: Option<Instant>,
    timing: &Timing,
    shown_at: Instant,
    chip: bool,
) -> Option<Instant> {
    match timing.deadline(shown_at, chip) {
        Some(t) => Some(earliest(pending, t)),
        None => pending,
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

/// When the next autosave is due: `interval` after `since` (the later of the last autosave
/// and the first unsaved change), while autosave is on, the model is `modified` and its
/// `revision` is not the one last autosaved; while a tool operation holds it back (`held`,
/// see `held_by_operation`), when that hold ends. None: nothing to wait for.
fn autosave_deadline(
    since: Instant,
    interval: Duration,
    revision: u64,
    saved_revision: Option<u64>,
    modified: bool,
    held: bool,
) -> Option<Instant> {
    let waits = if held { 2 } else { 1 };
    (!interval.is_zero() && modified && saved_revision != Some(revision))
        .then(|| since + interval * waits)
}

/// Whether a tool operation in progress (`busy`) holds a due autosave back at `now`: at most
/// one interval past when it was due (`since + interval`).
fn held_by_operation(busy: bool, now: Instant, since: Instant, interval: Duration) -> bool {
    busy && now < since + interval * 2
}

/// What the window does with its autosave as it exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AtExit {
    /// Remove it: the changes were saved or discarded.
    Clear,
    /// Write it first: the newest changes are not in it yet.
    Write,
    /// Leave it as it is.
    Keep,
}

/// What happens to the autosave as the window exits: removed when the exit was settled (the
/// unsaved prompt answered, or an agent's `quit`) or there is nothing unsaved. An exit the
/// system asks for without the prompt (macOS: Dock > Quit, log out, shut down, once
/// `system_quit` found the changes autosaved) keeps it, to be offered at the next start,
/// written first when autosave is on (`minutes` not 0) and the model's `revision` is not the
/// one last autosaved.
fn at_exit(
    settled: bool,
    modified: bool,
    revision: u64,
    saved_revision: Option<u64>,
    minutes: u32,
) -> AtExit {
    if settled || !modified {
        AtExit::Clear
    } else if minutes > 0 && saved_revision != Some(revision) {
        AtExit::Write
    } else {
        AtExit::Keep
    }
}

/// What the window answers when the system asks it to quit (macOS: Dock > Quit, log out, shut
/// down).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum SystemQuit {
    /// Let the system end the app, without asking.
    Exit,
    /// Ask about the unsaved changes, as Cmd+Q does, while the system waits for the answer.
    Prompt,
}

/// Whether a quit the system asks for goes ahead without the unsaved prompt: only when the exit
/// is settled (see `at_exit`), nothing is unsaved, or the changes are in the autosave, which
/// with autosave on (`minutes` not 0) is the model's `revision` autosaved already or `write`
/// writing it now. With autosave off, or a write that fails (no recovery folder, a full disk),
/// the prompt shows.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn system_quit(
    settled: bool,
    modified: bool,
    revision: u64,
    saved_revision: Option<u64>,
    minutes: u32,
    write: impl FnOnce() -> Result<(), String>,
) -> SystemQuit {
    if settled || !modified {
        return SystemQuit::Exit;
    }
    if minutes == 0 {
        return SystemQuit::Prompt;
    }
    if saved_revision == Some(revision) {
        return SystemQuit::Exit;
    }
    match write() {
        Ok(()) => SystemQuit::Exit,
        Err(why) => {
            eprintln!("warning: autosave failed as the system asked to quit: {why}");
            SystemQuit::Prompt
        }
    }
}

/// Where a quit the system asked for stands.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SystemQuitState {
    /// None asked, or it was cancelled.
    None,
    /// The unsaved prompt it raised is open; the system waits for its answer.
    Prompting,
    /// The system was told to go ahead and ends the app itself.
    Handed,
}

/// The answer to the system's quit once the unsaved prompt it raised has moved on: go ahead
/// when the window quits (after Save, once saved, or Don't Save), cancel when it does not
/// (Cancel, a save that failed or was cancelled); none yet while one of the prompt's dialogs is
/// still open (`waiting`).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn prompt_answer(quit: bool, waiting: bool) -> Option<bool> {
    if quit {
        Some(true)
    } else if waiting {
        None
    } else {
        Some(false)
    }
}

/// The deadline the event loop wakes at: the earlier of the pending repaint and the next
/// autosave.
fn wake_deadline(repaint: Option<Instant>, autosave: Option<Instant>) -> Option<Instant> {
    match autosave {
        Some(t) => Some(earliest(repaint, t)),
        None => repaint,
    }
}

/// Whether the Recover Models window opens at start, given the autosaves of windows that
/// ended without cleaning up (`recovery::list` without discarded work).
fn opens_recover_at_start(crashed: &[recovery::Entry]) -> bool {
    crashed.iter().any(|e| e.kind == recovery::Kind::Autosave)
}

/// How long before `now` a recovery copy was `saved`: "just now", "5 min ago", "3 h ago",
/// "2 days ago".
fn saved_ago(now: SystemTime, saved: SystemTime) -> String {
    let secs = now.duration_since(saved).map_or(0, |d| d.as_secs());
    match secs {
        0..60 => "just now".into(),
        60..7200 => format!("{} min ago", secs / 60),
        7200..172_800 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

/// The Recover Models window's rows for `entries`, their ages as of `now`.
fn recover_rows(entries: &[recovery::Entry], now: SystemTime) -> Vec<RecoverRow> {
    entries
        .iter()
        .map(|e| RecoverRow {
            model: e.model.clone(),
            name: e.name.clone(),
            original: e.original.clone(),
            saved: saved_ago(now, e.saved),
            discarded: e.kind == recovery::Kind::Discarded,
            error: e.error.clone(),
        })
        .collect()
}

/// The Discard Recovered sheet for the entries among `entries` whose copies are `which`
/// (None: all of them); None when none is listed.
fn discard_sheet(entries: &[recovery::Entry], which: Option<&Path>) -> Option<Sheet> {
    let chosen: Vec<&recovery::Entry> = entries
        .iter()
        .filter(|e| which.is_none_or(|m| e.model == m))
        .collect();
    (!chosen.is_empty()).then(|| Sheet::DiscardRecovered {
        models: chosen.iter().map(|e| e.model.clone()).collect(),
        names: chosen.iter().map(|e| e.name.clone()).collect(),
    })
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
    /// Whether the adapter can draw light shadows (see `render::supports_shadows`).
    shadows: bool,
    /// Whether it can draw ambient occlusion (see `render::supports_ambient_occlusion`).
    occlusion: bool,
}

/// A GPU adapter, device and queue with the surface they draw to. The desktop makes them when
/// the window opens; a browser makes them before the app starts, because it can only ask for
/// them asynchronously, and hands them to `App::new`.
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub surface: wgpu::Surface<'static>,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

/// A surface for `window` and a device to draw into it. Blocks until the adapter and device
/// are ready, which only a desktop may do.
#[cfg(not(target_arch = "wasm32"))]
fn native_context(instance: &wgpu::Instance, window: &Arc<Window>) -> Result<GpuContext, String> {
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
    Ok(GpuContext {
        instance: instance.clone(),
        surface,
        adapter,
        device,
        queue,
    })
}

/// The window's GPU objects from `context`, configured at `size`. GPU errors are logged
/// rather than ending the app, and a lost device is logged and flagged.
fn gpu(context: GpuContext, size: (u32, u32)) -> Result<Gpu, String> {
    let GpuContext {
        surface,
        adapter,
        device,
        queue,
        ..
    } = context;
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
        .get_default_config(&adapter, size.0.max(1), size.1.max(1))
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
        // A desktop's renderer finds out for itself (it checks its shader); a browser's cannot
        // wait for that.
        shadows: cfg!(not(target_arch = "wasm32")) || render::supports_shadows(&adapter),
        occlusion: render::supports_ambient_occlusion(&adapter),
    })
}

/// The renderer for `format`, with light shadows when the adapter can draw them.
fn make_renderer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    shadows: bool,
    occlusion: bool,
) -> Renderer {
    let mut renderer = if shadows {
        Renderer::new(device, queue, format)
    } else {
        Renderer::without_shadows(device, queue, format)
    };
    if !occlusion {
        renderer.disable_ambient_occlusion();
    }
    renderer
}

/// The offscreen snapshotter for pictures of the model (swatches, thumbnails), sharing the
/// window's device.
fn make_snapshotter(device: &wgpu::Device, queue: &wgpu::Queue, occlusion: bool) -> Snapshotter {
    let mut snapshotter = Snapshotter::with_device(device, queue);
    if !occlusion {
        snapshotter.disable_ambient_occlusion();
    }
    snapshotter
}

/// True for the input that makes a held Ctrl part of a chord rather than a tap: presses,
/// scrolls, pinches, focus changes and keys other than modifiers (the window reports the
/// modifier keys themselves as key events too).
/// Whether `event` is input that ends a scene transition at once: a press (key other than a
/// modifier, mouse button, touch), a scroll or a gesture. Moving the cursor does not.
fn finishes_transition(event: &WindowEvent) -> bool {
    match event {
        WindowEvent::KeyboardInput { event, .. } => {
            event.state == ElementState::Pressed && !is_modifier_key(&event.logical_key)
        }
        WindowEvent::MouseInput { state, .. } => *state == ElementState::Pressed,
        WindowEvent::Touch(touch) => touch.phase == TouchPhase::Started,
        _ => matches!(
            event,
            WindowEvent::MouseWheel { .. }
                | WindowEvent::PinchGesture { .. }
                | WindowEvent::PanGesture { .. }
        ),
    }
}

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

/// Whether a command key is down: Cmd or Ctrl on macOS and in a browser (where the page cannot
/// tell Apple's computers apart from others, and Cmd must work on them), Ctrl alone elsewhere,
/// where Super is the desktop's own key and never runs a command.
fn command_held(os: paths::Os, super_key: bool, control_key: bool) -> bool {
    control_key || (matches!(os, paths::Os::MacOs | paths::Os::Web) && super_key)
}

/// Whether the key that has the menu bar's chords is down: Cmd on macOS, none elsewhere.
fn menu_bar_held(os: paths::Os, super_key: bool) -> bool {
    os == paths::Os::MacOs && super_key
}

/// Whether a fly-mode movement key press is a shortcut instead. On macOS: with Cmd held
/// (Cmd+S saves; Ctrl+S still moves). Elsewhere: with Ctrl held, when Ctrl plus the key's
/// `letter` (as command dispatch reads it, layout first, see `shortcut_letter`) is a bound
/// window command (Ctrl+S saves), so Ctrl+W still steers and the Ctrl press itself, which
/// flies straight down, always reaches fly mode.
fn fly_shortcut(
    os: paths::Os,
    letter: Option<char>,
    shift: bool,
    super_key: bool,
    control_key: bool,
) -> bool {
    if os == paths::Os::MacOs {
        return super_key;
    }
    (control_key || (os == paths::Os::Web && super_key))
        && letter
            .is_some_and(|l| shortcuts::window_command(l, shift, false, false, false).is_some())
}

/// True if a key event goes to fly mode's movement: a moving key while flying, except a press
/// with Cmd held, which stays a shortcut (Cmd+S saves), and a press that does not reach the
/// editor (`reaches`: a text field has focus or a sheet is open, see `keys_reach_editor`).
/// Releases always go, so a key let go after Cmd went down, or after a text field took focus,
/// still stops: the eye would otherwise keep flying with no key down.
fn is_fly_move(
    flying: bool,
    key: PhysicalKey,
    pressed: bool,
    cmd: bool,
    reaches: bool,
) -> Option<FlyKey> {
    fly_move_key(key).filter(|_| flying && !(pressed && (cmd || !reaches)))
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

/// Longest time between two releases of a double- or triple-click. The farthest the cursor may
/// move between them, and from a press to its click's release, is the tools' drag threshold
/// (`DRAG_LOGICAL_PX`): a release the tools take as a drag is never a click.
const DOUBLE_CLICK: Duration = Duration::from_millis(500);

/// How many clicks a release at `at` (physical pixels) at `now` makes, counting `last` (the
/// previous click's time, place and count): one more when within `DOUBLE_CLICK` and `max_px`
/// of it, up to three; else, and after three, one.
fn click_count(last: Option<(Instant, DVec2, u8)>, now: Instant, at: DVec2, max_px: f64) -> u8 {
    match last {
        Some((t, p, n))
            if n < 3 && now.duration_since(t) <= DOUBLE_CLICK && p.distance(at) < max_px =>
        {
            n + 1
        }
        _ => 1,
    }
}

/// True if a press at `now` comes within `DOUBLE_CLICK` of the last click's release.
fn is_quick(last: Option<(Instant, DVec2, u8)>, now: Instant) -> bool {
    last.is_some_and(|(t, ..)| now.duration_since(t) <= DOUBLE_CLICK)
}

/// True if a release at `at` ends a click, that is the cursor is less than `max_px` from where
/// the button went down, rather than a drag (the tools take `max_px` or more as a drag).
fn is_click(press: Option<DVec2>, at: DVec2, max_px: f64) -> bool {
    press.is_some_and(|p| p.distance(at) < max_px)
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
    /// The system the window runs on: files, preferences, dialogs, clipboard, links.
    platform: Box<dyn Platform>,
    /// What it offers (`Platform::features`); what is off is hidden and never started.
    features: Features,
    /// The event loop's, for the answer of a check for updates made on another thread.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    proxy: EventLoopProxy<AppEvent>,
    /// Whether the renderer draws light shadows (off where the adapter cannot).
    shadows: bool,
    /// The file flows (open, save, import, export) and the request waiting for the platform.
    flows: Flows,
    /// The surface status of the last frame skipped and logged since one was drawn, so a run of
    /// skips is logged once.
    skip_logged: Option<&'static str>,
    renderer: Renderer,
    /// What `render_view` renders with, on the window's device.
    offscreen: Offscreen,
    /// A print waiting for its "Printing..." message to be shown.
    print_queue: PrintQueue,
    editor: Editor,
    /// The app should end: an agent's `quit`, or the Quit action or closing the window once
    /// any unsaved changes were saved or discarded. An exit without it was not settled.
    quit: bool,
    /// The folders the API's file paths must lie within (empty: no limit).
    roots: ApiRoots,
    /// Where discarded work is kept (None: nowhere).
    recovery: Option<PathBuf>,
    /// This window's hold on the recovery folder (None: it could not be used).
    session: Option<Session>,
    /// What the window autosaved and when.
    autosave: Autosave,
    /// The model opened from the Recover Models window, while it is the open document.
    recovered_doc: Option<RecoveredDoc>,
    /// The Recover Models window's entries; Some while it is open.
    recover: Option<RecoverList>,
    /// The window's preferences, as chosen in this session.
    prefs: Prefs,
    /// The recent files the start card lists: those that existed when the list last changed.
    recent_shown: Vec<PathBuf>,
    /// The start card was dismissed: it stays hidden for the rest of the session.
    start_dismissed: bool,
    /// Touch gestures (the viewer), the fingers that began in the view, when the window began
    /// (for the gestures' times) and the name the viewer's top bar shows.
    touches: Touches,
    touch_owned: Vec<u64>,
    epoch: Instant,
    model_name: String,
    /// The model was just opened in the viewer: fit it to the view once the view's size is known.
    fit_pending: bool,
    /// The viewer's list over the view (Parts or Recent), if one is open.
    viewer_panel: Option<ViewerPanel>,
    /// The Agent Activity window is open.
    agent_activity: bool,
    /// Whether the Keyboard Shortcuts window is open.
    shortcuts_window: bool,
    /// Whether the Settings window is open.
    settings_open: bool,
    /// The sheet open over the window: keys and native menu items (but Quit) wait while it is.
    sheet: Option<Sheet>,
    /// Where a quit the system asked for stands (see `system_quit`).
    #[cfg(target_os = "macos")]
    system_quit: SystemQuitState,
    /// The share the Open on Phone sheet serves; there is one only while that sheet is open.
    #[cfg(not(target_arch = "wasm32"))]
    share: Option<crate::share_server::Sharing>,
    /// What the open sheet was opened over, so its confirm acts only if it still applies.
    sheet_origin: SheetOrigin,
    /// What the 3D Text sheet last placed, which it opens with next (this session only).
    text_3d: TextSpec,
    /// How many of the editor's recovery files the window has already shown.
    recovered_seen: usize,
    gui: Gui,
    /// Document revision, selection and open context the renderer's model buffers were built
    /// from.
    drawn: Option<Drawn>,
    /// The Outliner's rows and the revision they were built at.
    outliner: Option<(u64, OutlinerTable)>,
    /// The Tags tab's rows, and the document (`Editor::documents`) and revision they were
    /// built at (counting entities on each tag walks the whole model).
    tag_rows: Option<((u64, u64), Vec<TagRow>)>,
    /// Each textured material's swatch in the Materials list, with the image it shows.
    swatches: HashMap<MaterialId, (Arc<TextureImage>, egui::TextureHandle)>,
    /// The component library's listing and thumbnails.
    library: library_panel::LibraryState,
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
    /// When the cursor last moved in the view and what flashes since when: the HUD's own
    /// repaint deadlines.
    timing: Timing,
    /// The window title last set.
    title: String,
    /// The edited flag last given to the window (the macOS close button's dot).
    #[cfg(target_os = "macos")]
    edited: bool,
    /// When and where the left button last ended a click in the view, and how many clicks in
    /// a row that made, for double- and triple-clicks.
    clicks: Option<(Instant, DVec2, u8)>,
    /// Where the left button was last pressed in the view, so a release that ends a drag does
    /// not count as a click.
    left_press: Option<DVec2>,
    /// Where the viewport's right-click menu was opened (points), while it is open.
    viewport_menu: Option<egui::Pos2>,
    /// The face under the cursor when the viewport menu opened.
    menu_face: Option<EntityId>,
    /// Whether the window has keyboard focus.
    focused: bool,
    /// How fly mode holds the pointer now.
    capture: Capture,
    /// The left button is down in the view while flying without a grabbed pointer: mouse
    /// motion looks around.
    look_held: bool,
    /// The native menu bar, built at the first frame.
    #[cfg(target_os = "macos")]
    native: Option<NativeMenu>,
}

/// What the window autosaved and when.
#[derive(Debug, Default)]
struct Autosave {
    /// The revision last autosaved, while its copy is this session's autosave.
    revision: Option<u64>,
    /// When an autosave was last written or tried.
    at: Option<Instant>,
    /// When the open model was first seen with unsaved changes since it was opened or saved.
    dirty_since: Option<Instant>,
    /// `Editor::documents` when last looked at: a new model starts over.
    documents: u64,
    /// The last failure shown, so each kind is shown once.
    failure: Option<String>,
}

impl Autosave {
    /// Copies the open model to `session`'s autosave when one is due (see
    /// `recovery::autosave_due`; every `minutes`, 0: never), naming the model's file, else
    /// `recovered` (the file a recovered model was a copy of). Returns when the next one is
    /// due. A failure is shown in the status once per kind, and tried again an interval
    /// later.
    fn tick(
        &mut self,
        now: Instant,
        session: &Session,
        editor: &mut Editor,
        minutes: u32,
        recovered: Option<&Path>,
    ) -> Option<Instant> {
        let doc = editor.doc();
        let modified = doc.is_modified();
        let revision = doc.revision();
        if self.documents != editor.documents() {
            self.documents = editor.documents();
            self.dirty_since = None;
        }
        if !modified {
            // Saved or replaced (by the window or an agent): there is no autosave of it.
            self.dirty_since = None;
            self.revision = None;
        } else if self.dirty_since.is_none() {
            self.dirty_since = Some(now);
        }
        let since = match (self.dirty_since, self.at) {
            (Some(d), Some(t)) => d.max(t),
            (d, t) => d.or(t).unwrap_or(now),
        };
        let interval = Duration::from_secs(u64::from(minutes) * 60);
        let busy = held_by_operation(editor.operation_in_progress(), now, since, interval);
        if !recovery::autosave_due(
            now,
            since,
            interval,
            revision,
            self.revision,
            modified,
            busy,
        ) {
            return autosave_deadline(since, interval, revision, self.revision, modified, busy);
        }
        let original = doc.path().or(recovered);
        let written = session.write_autosave(doc, original);
        self.at = Some(now);
        match written {
            Ok(()) => {
                self.revision = Some(revision);
                self.failure = None;
            }
            Err(why) => {
                if self.failure.as_deref() != Some(why.as_str()) {
                    editor.show_error(&format!("Autosave failed: {why}"));
                    self.failure = Some(why);
                }
            }
        }
        autosave_deadline(now, interval, revision, self.revision, true, false)
    }
}

/// A model opened from the Recover Models window.
#[derive(Clone, Debug)]
struct RecoveredDoc {
    /// `Editor::documents` once it was opened: a later document is not it.
    documents: u64,
    /// The name it went by, for the title.
    name: String,
    /// The file it was a copy of, which its autosaves keep naming.
    original: Option<PathBuf>,
}

/// The Recover Models window's list.
#[derive(Debug)]
struct RecoverList {
    /// Whether it lists the work agents discarded too (File > Recover Models...).
    discarded: bool,
    entries: Vec<recovery::Entry>,
    /// The window's rows for `entries`, made when they were listed.
    rows: Vec<RecoverRow>,
    /// How many older entries were left out.
    older: usize,
}

impl RecoverList {
    fn new(discarded: bool, listing: recovery::Listing) -> Self {
        Self {
            discarded,
            rows: recover_rows(&listing.entries, SystemTime::now()),
            entries: listing.entries,
            older: listing.older,
        }
    }
}

impl State {
    fn command_held(&self) -> bool {
        command_held(
            paths::Os::current(),
            self.modifiers.super_key(),
            self.modifiers.control_key(),
        )
    }

    fn menu_bar_key(&self) -> bool {
        menu_bar_held(paths::Os::current(), self.modifiers.super_key())
    }

    fn fly_shortcut(&self, event: &KeyEvent) -> bool {
        fly_shortcut(
            paths::Os::current(),
            shortcut_letter(&event.logical_key, event.physical_key),
            self.modifiers.shift_key(),
            self.modifiers.super_key(),
            self.modifiers.control_key(),
        )
    }

    fn new(
        window: Arc<Window>,
        roots: ApiRoots,
        recovery: Option<PathBuf>,
        session: Result<Session, String>,
        platform: Box<dyn Platform>,
        context: Option<GpuContext>,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Self {
        let flows_proxy = proxy.clone();
        let flows = Flows::new(Arc::new(move |reply| {
            let _ = flows_proxy.send_event(AppEvent::Platform(reply));
        }));
        let features = platform.features();
        let prefs = platform
            .read_prefs()
            .map(|text| prefs::from_text(&text))
            .unwrap_or_default();
        let recent_shown = if features.recent {
            prefs::existing(&prefs.recent, prefs::CARD_RECENT)
        } else {
            Vec::new()
        };
        let size = window.inner_size();
        let context = match context {
            Some(context) => context,
            #[cfg(not(target_arch = "wasm32"))]
            None => native_context(&wgpu::Instance::default(), &window)
                .unwrap_or_else(|e| panic!("{e}")),
            #[cfg(target_arch = "wasm32")]
            None => panic!("a browser must make the GPU context before the app starts"),
        };
        let instance = context.instance.clone();
        let Gpu {
            surface,
            device,
            queue,
            config,
            lost,
            shadows,
            occlusion,
        } = gpu(context, (size.width, size.height)).unwrap_or_else(|e| panic!("{e}"));

        let renderer = make_renderer(&device, &queue, config.format, shadows, occlusion);
        let gui = Gui::new(&window, &device, config.format, prefs.theme, prefs.ui_scale);
        set_min_window(&window, prefs.ui_scale);
        let view = Viewport::full((config.width, config.height));
        let offscreen = Offscreen::with(make_snapshotter(&device, &queue, occlusion));
        let mut editor = Editor::new(
            DVec2::new(config.width as f64, config.height as f64),
            window.scale_factor(),
        );
        editor.set_import_options(prefs.import);
        editor.set_trackpad(prefs.trackpad);
        editor.set_read_only(features.read_only);
        let mut touches = Touches::default();
        touches.set_scale_factor(window.scale_factor());
        editor.set_clipboard(platform.clipboard());
        if !shadows && !features.read_only {
            editor.show_info(crate::ui::SHADOWS_OFF);
        }
        let session = session
            .inspect_err(|why| {
                if features.autosave {
                    editor.show_error(&format!("Autosave is off: {why}"));
                }
            })
            .ok();
        Self {
            window,
            instance,
            surface,
            device,
            queue,
            config,
            device_lost: lost,
            platform,
            features,
            proxy,
            shadows,
            flows,
            skip_logged: None,
            renderer,
            offscreen,
            print_queue: PrintQueue::default(),
            editor,
            quit: false,
            roots,
            recovery,
            session,
            autosave: Autosave::default(),
            recovered_doc: None,
            recover: None,
            prefs,
            recent_shown,
            start_dismissed: features.read_only,
            touches,
            touch_owned: Vec::new(),
            epoch: Instant::now(),
            model_name: String::new(),
            fit_pending: false,
            viewer_panel: None,
            agent_activity: false,
            shortcuts_window: false,
            settings_open: false,
            sheet: None,
            #[cfg(target_os = "macos")]
            system_quit: SystemQuitState::None,
            #[cfg(not(target_arch = "wasm32"))]
            share: None,
            sheet_origin: SheetOrigin::default(),
            text_3d: default_text_3d(),
            recovered_seen: 0,
            gui,
            drawn: None,
            outliner: None,
            tag_rows: None,
            swatches: HashMap::new(),
            library: library_panel::LibraryState::new(),
            modifiers: ModifiersState::empty(),
            ctrl_tap: CtrlTap::default(),
            view,
            cursor: DVec2::ZERO,
            repaint_at: None,
            presented: None,
            timing: Timing::default(),
            clicks: None,
            title: String::new(),
            #[cfg(target_os = "macos")]
            edited: false,
            left_press: None,
            viewport_menu: None,
            menu_face: None,
            focused: true,
            capture: Capture::Free,
            look_held: false,
            #[cfg(target_os = "macos")]
            native: None,
        }
    }

    /// Holds or frees the pointer to match fly mode: held while flying in the focused window
    /// (hidden and locked, or confined, else visible with left-drag looking), free otherwise,
    /// so another app never gets a hidden or stuck pointer.
    fn sync_fly_cursor(&mut self) {
        let want = holds_pointer(self.editor.flying(), self.focused, self.sheet.is_some());
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

    /// Opens the model the viewer was given, read-only, and fits it to the view once the view's
    /// size is known; the window says so when it cannot.
    fn open_shared(&mut self, model: SharedModel) {
        self.viewer_panel = None;
        if open_shared_model(&mut self.editor, &mut self.model_name, model) {
            self.fit_pending = true;
        } else {
            self.editor.show_error(DAMAGED_LINK);
        }
    }

    /// A touch (or the viewer's mouse button, `MOUSE_TOUCH`) began, moved or ended at `p`, in
    /// window coordinates: only touches that began in the view, over no window, go to the
    /// gestures. True if anything visible may have changed.
    fn touch(&mut self, phase: touch::Phase, id: u64, p: DVec2) -> bool {
        if !self.features.read_only {
            return false;
        }
        let owned = self.touch_owned.contains(&id);
        match phase {
            touch::Phase::Start => {
                if owned {
                    // A start for a finger already down: its end was lost (a release outside
                    // the window). The old touch is dropped, without a tap, and this one begins.
                    self.touch(touch::Phase::Cancel, id, p);
                }
                if self.sheet.is_some() || !self.in_view(p) || self.gui.wants_pointer_at(p) {
                    return false;
                }
                self.touch_owned.push(id);
            }
            _ if !owned => return false,
            touch::Phase::End | touch::Phase::Cancel => self.touch_owned.retain(|i| *i != id),
            touch::Phase::Move => {}
        }
        // Where the pointer is, for what is covered by a window and what is under it.
        self.cursor = p;
        let origin = DVec2::new(self.view.x as f64, self.view.y as f64);
        let event = TouchEvent {
            phase,
            id,
            pos: p - origin,
            at_ms: self.epoch.elapsed().as_millis() as u64,
        };
        let mut changed = false;
        for gesture in self.touches.event(event) {
            changed |= self.editor.touch(gesture);
        }
        changed
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
            if moved {
                // The chip waits for the cursor to rest again.
                self.timing.moved(Instant::now());
            }
            moved
        } else {
            self.editor.cursor_left() || moved
        }
    }

    /// Runs a step of the file flows over the editor and the platform, then what it asks of the
    /// window (see `Flows`).
    fn flow(&mut self, step: impl FnOnce(&mut Flows, &mut Env) -> Vec<Effect>) {
        let recovered = self
            .recovered_doc
            .as_ref()
            .filter(|r| r.documents == self.editor.documents())
            .map(|r| r.name.as_str());
        let mut env = Env {
            editor: &mut self.editor,
            platform: &*self.platform,
            parent: Some(&*self.window),
            recovered,
        };
        let effects = step(&mut self.flows, &mut env);
        self.apply_effects(effects);
    }

    fn apply_effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::ClearAutosave => self.clear_autosave(),
                Effect::RememberFile(path) => self.remember_path(path.as_deref()),
                Effect::Quit => self.quit = true,
                Effect::OpenRecovered(model) => self.open_recovered_now(&model),
                Effect::ImportSheet(path) => {
                    let options = self.editor.import_options();
                    self.open_sheet(Sheet::Import { path, options });
                }
                Effect::LibraryFolder(folder) => {
                    self.set_library_folder(Some(std::path::absolute(&folder).unwrap_or(folder)));
                }
                Effect::Pdf(options, path) => self.start_print(&options, Some(path)),
                Effect::AskUnsaved(prompt) => self.open_sheet(Sheet::Unsaved { prompt }),
                Effect::ExportSheet(formats) => self.open_sheet(Sheet::Export { formats }),
            }
        }
    }

    /// The system's paste: into a focused text field, else the model's clip (not under a sheet).
    fn paste_event(&mut self, text: String) {
        if self.gui.wants_keyboard() {
            self.gui.paste_text(text);
        } else if self.sheet.is_none() {
            self.apply(UiAction::Paste);
        }
    }

    /// The platform answered a request it had left pending.
    fn platform_reply(&mut self, reply: PlatformReply) {
        self.flow(|flows, env| flows.answered(env, reply));
    }

    /// Before the model is replaced or the window closes: if it has unsaved changes, asks
    /// whether to save them, then does `then` (not if the answer cancels).
    fn settle_and(&mut self, then: Then) {
        self.flow(|flows, env| flows.settle_and(env, then));
    }

    fn new_model(&mut self) {
        self.settle_and(Then::NewModel);
    }

    fn open(&mut self) {
        self.flow(|flows, env| flows.open(env));
    }

    /// Removes this window's autosave: the model it copied was saved or replaced.
    fn clear_autosave(&mut self) {
        if let Some(session) = &self.session {
            session.clear_autosave();
        }
        self.autosave.revision = None;
    }

    /// Removes the autosave as the window exits, or writes it first when the exit was not
    /// settled (see `at_exit`).
    fn autosave_at_exit(&mut self) {
        let doc = self.editor.doc();
        let action = at_exit(
            self.quit,
            doc.is_modified(),
            doc.revision(),
            self.autosave.revision,
            self.prefs.autosave_minutes,
        );
        match action {
            AtExit::Clear => self.clear_autosave(),
            AtExit::Write => {
                if let Err(why) = self.write_autosave_now() {
                    eprintln!("warning: autosave failed as the app ended: {why}");
                }
            }
            AtExit::Keep => {}
        }
    }

    /// Copies the open model to this window's autosave now, naming the model's file, else the
    /// file a recovered model was a copy of.
    fn write_autosave_now(&mut self) -> Result<(), String> {
        let session = self.session.as_ref().ok_or("no recovery folder")?;
        let doc = self.editor.doc();
        let original = doc
            .path()
            .map(Path::to_path_buf)
            .or_else(|| self.recovered_doc().and_then(|r| r.original.clone()));
        session.write_autosave(doc, original.as_deref())?;
        self.autosave.revision = Some(doc.revision());
        Ok(())
    }

    /// The system asked to quit (see `system_quit`): lets it go ahead when the changes are
    /// safe in the autosave, else asks about them as Cmd+Q does while the system waits, and
    /// answers it after (see `answer_system_quit`). Cancels it when another request is open and
    /// the prompt cannot show.
    #[cfg(target_os = "macos")]
    fn system_quit(&mut self) {
        let doc = self.editor.doc();
        let (modified, revision) = (doc.is_modified(), doc.revision());
        let answer = system_quit(
            self.quit,
            modified,
            revision,
            self.autosave.revision,
            self.prefs.autosave_minutes,
            || self.write_autosave_now(),
        );
        match answer {
            SystemQuit::Exit => self.hand_system_quit(true),
            SystemQuit::Prompt if self.flows.is_waiting() || self.sheet.is_some() => {
                self.hand_system_quit(false);
            }
            SystemQuit::Prompt => {
                self.system_quit = SystemQuitState::Prompting;
                self.settle_and(Then::Quit);
                self.answer_system_quit();
            }
        }
    }

    /// Answers the system's quit once the prompt it raised has moved on (see `prompt_answer`).
    #[cfg(target_os = "macos")]
    fn answer_system_quit(&mut self) {
        if self.system_quit != SystemQuitState::Prompting {
            return;
        }
        let waiting = self.flows.is_waiting() || self.sheet.is_some();
        if let Some(go) = prompt_answer(self.quit, waiting) {
            self.hand_system_quit(go);
        }
    }

    /// Answers the system's quit: go ahead (`go`), and AppKit ends the app, or cancel it.
    #[cfg(target_os = "macos")]
    fn hand_system_quit(&mut self, go: bool) {
        crate::system_quit::reply(go);
        self.system_quit = if go {
            SystemQuitState::Handed
        } else {
            SystemQuitState::None
        };
    }

    /// Whether the window should end its event loop: it quits, and no quit the system asked
    /// for ends the app instead (once that quit is answered, AppKit ends the app itself).
    fn should_exit(&mut self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.answer_system_quit();
            if self.system_quit == SystemQuitState::Handed {
                return false;
            }
        }
        self.quit
    }

    /// The model opened from the Recover Models window, while it is the open document.
    fn recovered_doc(&self) -> Option<&RecoveredDoc> {
        self.recovered_doc
            .as_ref()
            .filter(|r| r.documents == self.editor.documents())
    }

    /// Writes the autosave when it is due; returns when the next one is.
    fn autosave_tick(&mut self, now: Instant) -> Option<Instant> {
        let session = self.session.as_ref()?;
        let recovered = self.recovered_doc().and_then(|r| r.original.clone());
        self.autosave.tick(
            now,
            session,
            &mut self.editor,
            self.prefs.autosave_minutes,
            recovered.as_deref(),
        )
    }

    /// Opens the Recover Models window listing the recovery folder's entries (with
    /// `discarded`, the work agents discarded too), or lists them again while it is open.
    fn list_recovered(&mut self, discarded: bool) {
        let (Some(dir), Some(session)) = (self.recovery.as_deref(), self.session.as_ref()) else {
            self.editor
                .show_error("Recovery is not available: the recovery folder cannot be used");
            return;
        };
        self.recover = Some(RecoverList::new(
            discarded,
            recovery::list_newest(dir, session, discarded, recovery::MAX_LISTED),
        ));
    }

    /// Lists the Recover Models window's entries again, closing it once none is left.
    fn refresh_recovered(&mut self) {
        if let Some(discarded) = self.recover.as_ref().map(|r| r.discarded) {
            self.list_recovered(discarded);
            if self
                .recover
                .as_ref()
                .is_some_and(|r| r.entries.is_empty() && r.older == 0)
            {
                self.recover = None;
            }
        }
    }

    /// Opens the recovered model whose copy is `model`, once unsaved changes are settled, as
    /// an untitled model with unsaved changes; its copy becomes this window's autosave.
    fn open_recovered(&mut self, model: &Path) {
        let Some(entry) = self
            .recover
            .as_ref()
            .and_then(|r| r.entries.iter().find(|e| e.model == model))
            .filter(|e| e.error.is_none())
            .cloned()
        else {
            return;
        };
        self.settle_and(Then::OpenRecovered(entry.model));
    }

    /// Opens the recovered model whose copy is `model`, its unsaved changes settled.
    fn open_recovered_now(&mut self, model: &Path) {
        let Some(entry) = self
            .recover
            .as_ref()
            .and_then(|r| r.entries.iter().find(|e| e.model == model))
            .filter(|e| e.error.is_none())
            .cloned()
        else {
            return;
        };
        if !self.editor.open_recovered(&entry.model) {
            return;
        }
        self.recovered_doc = Some(RecoveredDoc {
            documents: self.editor.documents(),
            name: entry.name.clone(),
            original: entry.original.clone(),
        });
        let now = Instant::now();
        let adopted = self.session.as_ref().map(|s| s.adopt(&entry));
        match adopted {
            Some(Ok(_)) => {
                // The copy holds this model as it is now.
                self.autosave.revision = Some(self.editor.doc().revision());
                self.autosave.at = Some(now);
            }
            Some(Err(e)) => {
                // The model replaced is gone: so is its autosave.
                self.clear_autosave();
                self.editor.show_error(&format!(
                    "The recovered model is open, but its copy could not be kept: {e}"
                ));
            }
            None => {}
        }
        self.refresh_recovered();
    }

    /// Removes the recovered copies the confirmed Discard Recovered sheet named.
    fn discard_recovered(&mut self, models: &[PathBuf]) {
        let entries = self.recover.as_ref().map_or(&[][..], |r| &r.entries[..]);
        let failed: Vec<String> = entries
            .iter()
            .filter(|e| models.contains(&e.model))
            .filter_map(|e| {
                recovery::discard(e)
                    .err()
                    .map(|why| format!("{}: {why}", e.name))
            })
            .collect();
        if let Some(first) = failed.first() {
            self.editor
                .show_error(&format!("Cannot discard the recovered model {first}"));
        }
        self.refresh_recovered();
    }

    /// Puts the model's file first in the recent files, and keeps that in the preferences.
    fn remember_file(&mut self) {
        let path = self.editor.doc().path().map(Path::to_path_buf);
        self.remember_path(path.as_deref());
    }

    /// Puts `path` first in the recent files, and keeps that in the preferences.
    fn remember_path(&mut self, path: Option<&Path>) {
        if !self.features.recent {
            return;
        }
        if let Some(path) = path {
            prefs::remember_recent(&mut self.prefs.recent, path);
            self.save_prefs();
            self.recent_shown = prefs::existing(&self.prefs.recent, prefs::CARD_RECENT);
        }
    }

    /// Saves to the current file, or to one chosen in a dialog when there is none or `choose`
    /// is set.
    fn save(&mut self, choose: bool) {
        self.flow(|flows, env| flows.save(env, choose, None));
    }

    /// Gives a material the PNG or JPEG image chosen in a dialog as its texture.
    fn load_texture(&mut self, material: MaterialId) {
        self.flow(|flows, env| {
            flows.load_texture(env, material);
            Vec::new()
        });
    }

    /// The sRGB color the Paint tool paints with: its material's, or the default material's
    /// front.
    fn paint_color(&self) -> [u8; 3] {
        let doc = self.editor.doc();
        self.editor
            .current_material()
            .and_then(|m| doc.scene().material(m))
            .map_or(
                render::scene::DEFAULT_FRONT.map(|c| (c * 255.0).round() as u8),
                |m| m.color,
            )
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
                metalness: m.metalness,
                roughness: m.roughness,
                density: m.density,
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

    /// Opens `sheet` over the window, replacing any other. Fly mode ends first: the held keys
    /// are let go (the sheet keeps their releases from the editor) and the pointer is freed.
    fn open_sheet(&mut self, sheet: Sheet) {
        end_fly_for_sheet(&mut self.editor);
        self.look_held = false;
        self.sheet_origin = SheetOrigin::of(&self.editor);
        self.sheet = Some(sheet);
        self.sync_fly_cursor();
    }

    /// What the model is now, for telling whether a share is stale: the open documents' count
    /// and the document's revision.
    #[cfg(not(target_arch = "wasm32"))]
    fn model_stamp(&self) -> (u64, u64) {
        (self.editor.documents(), self.editor.doc().revision())
    }

    /// Seals the model as it is (without marking it saved) and serves it, replacing any share
    /// (the old server stops first); what the sheet shows for the result.
    #[cfg(not(target_arch = "wasm32"))]
    fn start_share(&mut self) -> PhoneView {
        self.share = None;
        let stamp = self.model_stamp();
        let bytes = match self.editor.doc().encode_bytes() {
            Ok(b) => b,
            Err(e) => return PhoneView::Unavailable(e.message),
        };
        let doc = self.editor.doc();
        let name = share_name(doc.path(), doc.imported_from(), self.flows.kept_name());
        match crate::share_server::Sharing::begin_here(&name, &bytes, stamp) {
            Ok(share) => {
                let view = PhoneView::Sharing(share.view(stamp));
                self.share = Some(share);
                view
            }
            Err(why) => PhoneView::Unavailable(why),
        }
    }

    /// Once a frame: the share lives only while the Open on Phone sheet is open, and the sheet
    /// shows its current state (phones served, time left, a changed model). A share that ended
    /// by itself says so.
    #[cfg(not(target_arch = "wasm32"))]
    fn refresh_share(&mut self) {
        let open = matches!(self.sheet, Some(Sheet::Phone { .. }));
        if !open {
            self.share = None;
            return;
        }
        let stamp = self.model_stamp();
        let view = match &self.share {
            Some(s) if s.running() => PhoneView::Sharing(s.view(stamp)),
            Some(_) => {
                self.share = None;
                PhoneView::Unavailable(
                    "Sharing stopped after 30 minutes. Choose File > Open on Phone to share again."
                        .into(),
                )
            }
            None => return,
        };
        self.sheet = Some(Sheet::Phone { view });
    }

    /// Whether the open sheet shows a share's time left, which moves every second.
    #[cfg(not(target_arch = "wasm32"))]
    fn share_ticks(&self) -> bool {
        self.share.is_some() && matches!(self.sheet, Some(Sheet::Phone { .. }))
    }

    #[cfg(target_arch = "wasm32")]
    fn share_ticks(&self) -> bool {
        false
    }

    /// Takes the open sheet for its confirm action, when `kind` says it is the sheet the action
    /// belongs to (otherwise it stays open and this is None). A sheet that no longer applies
    /// (`sheet_applies`) is closed with a message instead.
    fn confirm_sheet(&mut self, kind: impl Fn(&Sheet) -> bool) -> Option<Sheet> {
        if !self.sheet.as_ref().is_some_and(kind) {
            return None;
        }
        let sheet = self.sheet.take()?;
        match sheet_applies(&sheet, &self.sheet_origin, &self.editor) {
            Ok(()) => Some(sheet),
            Err(why) => {
                self.editor.show_error(why);
                None
            }
        }
    }

    /// Closes the open sheet once the document it was opened over is replaced (new or open,
    /// by the window or an agent): what it names may no longer exist. The note sheet of a
    /// note being placed closes once the Text tool no longer places it (an agent's `cancel`
    /// or `set_tool`, or its text typed through `tool_input`).
    fn drop_outlived_sheet(&mut self) {
        if self.sheet.as_ref().is_some_and(|s| {
            !matches!(
                s,
                Sheet::DiscardRecovered { .. }
                    | Sheet::DeleteLibraryItem { .. }
                    | Sheet::Unsaved { .. }
                    | Sheet::About { .. }
                    | Sheet::Update { .. }
                    | Sheet::Phone { .. }
            )
        }) && self.editor.documents() != self.sheet_origin.document
        {
            self.sheet = None;
        }
        if let Some(Sheet::NoteText { request, .. }) = &self.sheet
            && !matches!(request, NoteRequest::Edit(_))
            && !self.editor.awaits_note_text()
        {
            self.sheet = None;
        }
    }

    /// Asks for an OBJ, STL or SketchUp file, then opens the Import Options sheet for it; the
    /// file is read when the sheet's Import is pressed (`UiAction::ConfirmImport`).
    fn import(&mut self) {
        self.flow(|flows, env| flows.import(env));
    }

    /// Exports to a file chosen in a dialog; its extension picks the format. The dialog
    /// suggests the format exported to last.
    fn export(&mut self) {
        self.flow(|flows, env| flows.export(env));
    }

    /// Starts printing the view as `spec` says: Export asks for the PDF's file first, Print
    /// uses a temp file. The status says "Printing..." and the render, which blocks the window,
    /// runs two frames later (see `PrintQueue`).
    fn print(&mut self, purpose: PrintPurpose, spec: &PrintOptions) {
        if !self.features.print {
            return;
        }
        match purpose {
            PrintPurpose::Export => {
                let options = *spec;
                self.flow(|flows, env| flows.export_pdf(env, options));
            }
            PrintPurpose::Print => self.start_print(spec, None),
        }
    }

    /// Queues the print of the view to the PDF at `path` (None: a temp file that is opened).
    fn start_print(&mut self, spec: &PrintOptions, path: Option<PathBuf>) {
        self.editor.show_info("Printing...");
        self.print_queue.queue(PrintJob {
            options: *spec,
            path,
        });
        self.window.request_redraw();
    }

    /// Runs the print `PrintQueue` released.
    fn run_print(&mut self, job: PrintJob) {
        let snapshotter = self.offscreen.get();
        match job.path {
            Some(path) => {
                self.editor
                    .export_pdf_file(snapshotter, &job.options, &path);
            }
            #[cfg(not(target_arch = "wasm32"))]
            None => {
                self.editor.print_and_open(snapshotter, &job.options);
            }
            // Printing is off where there is no program to open the file with.
            #[cfg(target_arch = "wasm32")]
            None => {
                let _ = snapshotter;
                self.editor.show_error("Printing is not available here");
            }
        }
        self.window.request_redraw();
    }

    /// Writes the preferences, when the window has a prefs file. A failed save leaves the
    /// change for this session, with a warning.
    fn save_prefs(&self) {
        if let Err(why) = self.platform.write_prefs(&prefs::to_text(&self.prefs)) {
            eprintln!("warning: cannot save the preferences to {why}");
        }
    }

    fn apply(&mut self, action: UiAction) {
        if self.features.read_only && !viewer_allows(&action) {
            return;
        }
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
            UiAction::ConfirmImport(options) => {
                // Only the first report counts: the sheet is gone after it.
                if let Some(Sheet::Import { path, .. }) =
                    self.confirm_sheet(|s| matches!(s, Sheet::Import { .. }))
                {
                    self.editor.set_import_options(options);
                    self.prefs.import = options;
                    self.save_prefs();
                    self.flow(|flows, env| {
                        flows.import_confirmed(env, &path);
                        Vec::new()
                    });
                }
            }
            UiAction::CloseSheet => {
                #[cfg(not(target_arch = "wasm32"))]
                if matches!(self.sheet, Some(Sheet::Phone { .. })) {
                    self.share = None;
                }
                // The unsaved-changes question closed any other way is a Cancel.
                if matches!(self.sheet, Some(Sheet::Unsaved { .. })) {
                    self.sheet = None;
                    self.platform_reply(PlatformReply::Unsaved(UnsavedChoice::Cancel));
                }
                // A note being placed is dropped with its sheet.
                if let Some(Sheet::NoteText { request, .. }) = self.sheet.take()
                    && !matches!(request, NoteRequest::Edit(_))
                {
                    self.editor.cancel_note();
                }
            }
            UiAction::ConfirmNote(text) => {
                if let Some(Sheet::NoteText { request, .. }) =
                    self.confirm_sheet(|s| matches!(s, Sheet::NoteText { .. }))
                {
                    self.editor.finish_note(request, text);
                }
            }
            UiAction::Show3dText => {
                let spec = self.text_3d.clone();
                self.open_sheet(Sheet::Text3d { spec });
            }
            // A refusal (no letters the font has) is in the status.
            UiAction::Confirm3dText(spec) => {
                if self
                    .confirm_sheet(|s| matches!(s, Sheet::Text3d { .. }))
                    .is_some()
                {
                    self.text_3d = spec.clone();
                    let _ = self.editor.place_3d_text(spec);
                }
            }
            UiAction::SetUnits(units) => {
                self.editor.set_units(units);
            }
            UiAction::SetSceneTransition(transition) => {
                self.editor.set_scene_transition(transition);
            }
            UiAction::ExportFile => self.export(),
            UiAction::ExportPdf => self.open_sheet(Sheet::Print {
                purpose: PrintPurpose::Export,
                spec: self.prefs.print,
            }),
            UiAction::Print => self.open_sheet(Sheet::Print {
                purpose: PrintPurpose::Print,
                spec: self.prefs.print,
            }),
            UiAction::ConfirmExport(ext) => {
                if self
                    .confirm_sheet(|s| matches!(s, Sheet::Export { .. }))
                    .is_some()
                {
                    self.flow(|flows, env| flows.export_as(env, &ext));
                }
            }
            UiAction::ConfirmPrint(spec) => {
                if let Some(Sheet::Print { purpose, .. }) =
                    self.confirm_sheet(|s| matches!(s, Sheet::Print { .. }))
                {
                    let spec = spec.cleaned();
                    self.prefs.print = spec;
                    self.save_prefs();
                    self.print(purpose, &spec);
                }
            }
            UiAction::Tool(kind) => self.editor.select_tool(kind),
            UiAction::Undo => {
                self.editor.undo();
            }
            UiAction::Redo => {
                self.editor.redo();
            }
            UiAction::SelectAll => {
                self.editor.select_all();
            }
            UiAction::SelectNone => {
                self.editor.select_none();
            }
            // A refusal is in the status: nothing was selected.
            UiAction::Duplicate => {
                let _ = self.editor.duplicate();
            }
            // Refusals are in the status (nothing selected, no clip on the clipboard).
            UiAction::Cut => {
                let _ = self.editor.cut();
            }
            UiAction::Copy => {
                let _ = self.editor.copy();
            }
            UiAction::Paste => {
                let _ = self.editor.paste(None);
            }
            UiAction::PasteInPlace => {
                let _ = self.editor.paste_in_place();
            }
            UiAction::EraseSelection => {
                self.editor.erase_selection();
            }
            UiAction::MakeGroup => {
                self.editor.make_group();
            }
            // With a selection the Make Component sheet asks for the name; without one the
            // editor says what is missing.
            UiAction::MakeComponent => {
                if self.editor.doc().selection().is_empty() {
                    self.editor.make_component();
                } else {
                    let suggested = self.editor.next_component_name();
                    self.open_sheet(Sheet::NameComponent { suggested });
                }
            }
            UiAction::RenameComponent => {
                if let Some(target) = self.editor.rename_target() {
                    self.open_sheet(Sheet::Rename { target });
                }
            }
            UiAction::RenameDefinition(definition, name) => {
                self.editor.rename_definition(definition, &name);
            }
            // A second report in one frame (egui laid it out twice) is harmless: input only
            // reaches the first pass, and a repeat would only say "Nothing to purge".
            UiAction::PurgeUnused => {
                self.editor.purge_unused();
            }
            UiAction::ShowFieldOfView => {
                let degrees = self.editor.field_of_view();
                self.open_sheet(Sheet::FieldOfView { degrees });
            }
            UiAction::ConfirmFieldOfView(degrees) => {
                if self
                    .confirm_sheet(|s| matches!(s, Sheet::FieldOfView { .. }))
                    .is_some()
                {
                    self.editor.set_field_of_view(degrees);
                }
            }
            // Only the first report counts: the sheet is gone after it.
            UiAction::ConfirmName(name) => {
                match self.confirm_sheet(|s| {
                    matches!(s, Sheet::NameComponent { .. } | Sheet::Rename { .. })
                }) {
                    Some(Sheet::NameComponent { .. }) => {
                        self.editor.make_component_named(&name);
                    }
                    Some(Sheet::Rename { target }) => {
                        self.editor.rename_definition(target.definition, &name);
                    }
                    _ => {}
                }
            }
            UiAction::Explode => {
                self.editor.explode_selection();
            }
            UiAction::MakeUnique => {
                self.editor.make_unique();
            }
            UiAction::Flip(axis, frame) => {
                self.editor.flip(axis, frame);
            }
            UiAction::ResetAxes => {
                self.editor.reset_axes();
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
            UiAction::SetShadowsOn(on) => {
                self.editor.set_shadows_on(on);
            }
            UiAction::SetShadows(change) => {
                self.editor.set_shadows(&change);
            }
            UiAction::SetLocation(location) => {
                self.editor.set_location(location);
            }
            UiAction::ChooseMaterial(material) => {
                self.editor.choose_material(material);
            }
            UiAction::PaintWithMaterial(material) => {
                self.editor.paint_with(material);
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
            UiAction::SetMaterialMetalness(material, percent) => {
                self.editor.set_material_metalness(material, percent);
            }
            UiAction::SetMaterialRoughness(material, percent) => {
                self.editor.set_material_roughness(material, percent);
            }
            UiAction::SetMaterialDensity(material, density) => {
                self.editor.set_material_density(material, density);
            }
            UiAction::LoadTexture(material) => self.load_texture(material),
            UiAction::RemoveTexture(material) => {
                self.editor.remove_texture(material);
            }
            UiAction::SetTextureSize(material, millimeters) => {
                self.editor.set_texture_size(material, millimeters);
            }
            UiAction::SetColorByTag(on) => {
                self.editor.set_color_by_tag(on);
            }
            UiAction::SetSectionPlanes(on) => {
                self.editor.set_section_display(Some(on), None, None);
            }
            UiAction::SetSectionCuts(on) => {
                self.editor.set_section_display(None, Some(on), None);
            }
            UiAction::SetSectionFill(on) => {
                self.editor.set_section_display(None, None, Some(on));
            }
            UiAction::Hide => {
                self.editor.hide_selection();
            }
            UiAction::UnhideSelected => {
                self.editor.unhide_selection();
            }
            UiAction::UnhideLast => {
                self.editor.unhide_last();
            }
            UiAction::UnhideAll => {
                self.editor.unhide_all();
            }
            UiAction::SetHidden(hidden) => {
                self.editor.set_selection_hidden(hidden);
            }
            UiAction::SetSectionCut(cut) => {
                self.editor.set_selection_section_cut(cut);
            }
            UiAction::OutlinerSetHidden { path, hidden } => {
                self.editor.outliner_set_hidden(&path, hidden);
            }
            UiAction::SetXRay(on) => {
                self.editor.set_x_ray(on);
            }
            UiAction::SetHiddenGeometry(on) => {
                self.editor.set_hidden_geometry(on);
            }
            UiAction::ReverseFaces => {
                self.editor.reverse_faces();
            }
            UiAction::OrientFaces(from) => {
                self.editor.orient_faces(from);
            }
            UiAction::FlipSection => {
                self.editor.flip_selected_section();
            }
            UiAction::SetSectionActive(on) => {
                self.editor.set_selected_section_active(on);
            }
            UiAction::CloseViewportMenu => self.viewport_menu = None,
            UiAction::AddTag => {
                self.editor.add_tag();
            }
            // A tag something is on asks where that goes first.
            UiAction::DeleteTag(tag) => match delete_tag_sheet(self.editor.doc(), tag) {
                Some(sheet) => self.open_sheet(sheet),
                None => {
                    self.editor.delete_tag(tag, None);
                }
            },
            UiAction::ConfirmDeleteTag(move_to) => {
                if let Some(Sheet::DeleteTag { tag, .. }) =
                    self.confirm_sheet(|s| matches!(s, Sheet::DeleteTag { .. }))
                {
                    self.editor.delete_tag(tag, move_to);
                }
            }
            UiAction::RenameTag(tag, name) => {
                self.editor.rename_tag(tag, name);
            }
            UiAction::RecolorTag(tag, color) => {
                self.editor.recolor_tag(tag, color);
            }
            UiAction::SetTagVisible(tag, visible) => {
                self.editor.set_tag_visible(tag, visible);
            }
            UiAction::SetSelectionTag(tag) => {
                self.editor.set_selection_tag(tag);
            }
            UiAction::AddStyle(from) => {
                self.editor.add_style(from);
            }
            UiAction::DeleteStyle(style) => {
                self.editor.delete_style(style);
            }
            UiAction::RenameStyle(style, name) => {
                self.editor.rename_style(style, name);
            }
            UiAction::ChooseStyle(style) => {
                self.editor.choose_style(style);
            }
            UiAction::ChangeStyle(style, patch) => {
                self.editor.change_style_settings(style, patch);
            }
            UiAction::ToggleEdgeEffect(effect) => {
                if let (Some(style), Some(settings)) =
                    (self.editor.doc().style(), self.editor.doc().chosen_style())
                {
                    let patch = crate::ui::edge_effect_patch(settings, effect);
                    self.editor.change_style_settings(style, patch);
                }
            }
            UiAction::AddScene => {
                self.editor.add_scene();
            }
            UiAction::UpdateScene(scene) => {
                self.editor.update_scene(scene, None, None);
            }
            UiAction::StoreScenePositions(scene, on) => {
                self.editor.set_scene_positions(scene, on);
            }
            UiAction::StoreSceneJoints(scene, on) => {
                self.editor.set_scene_joints(scene, on);
            }
            UiAction::StoreSceneCamera(scene, on) => {
                self.editor.set_scene_stores(scene, Some(on), None);
            }
            UiAction::StoreSceneTags(scene, on) => {
                self.editor.set_scene_stores(scene, None, Some(on));
            }
            UiAction::RenameScene(scene, name) => {
                self.editor.rename_scene(scene, name);
            }
            UiAction::MoveScene(scene, index) => {
                self.editor.move_scene(scene, index);
            }
            UiAction::DeleteScene(scene) => {
                self.editor.delete_scene(scene);
            }
            UiAction::SelectJoint(joint) => {
                self.editor.select_joint(joint);
            }
            UiAction::DeleteJoint(joint) => {
                self.editor.delete_joint(joint);
            }
            UiAction::PlayJoint(joint) => {
                self.editor.play_joint_range(joint);
            }
            UiAction::DriveJoint {
                joint,
                value,
                slide,
                done,
            } => {
                self.editor.drive_joint(joint, value, slide, done);
            }
            UiAction::GoToScene(scene) => {
                self.editor.go_to_scene(scene);
            }
            UiAction::PreviousScene => {
                self.editor.cycle_scene(false);
            }
            UiAction::NextScene => {
                self.editor.cycle_scene(true);
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
            UiAction::Quit => self.settle_and(Then::Quit),
            UiAction::ShowAgentActivity(open) => self.agent_activity = open,
            UiAction::ShowShortcuts(open) => self.shortcuts_window = open,
            UiAction::ShowAbout => self.open_sheet(Sheet::About {
                shadows_off: !self.shadows,
            }),
            UiAction::CheckForUpdates => {
                // The browser build hides the item and has no thread to spawn.
                #[cfg(not(target_arch = "wasm32"))]
                if self.features.update_check {
                    self.open_sheet(Sheet::Update { outcome: None });
                    let proxy = self.proxy.clone();
                    std::thread::spawn(move || {
                        let _ = proxy.send_event(AppEvent::Update(crate::update::check()));
                    });
                }
            }
            UiAction::OpenReleasePage(url) => {
                if let Err(e) = self.platform.open_url(&url) {
                    self.editor.show_error(&e);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            UiAction::OpenOnPhone => {
                if self.features.share {
                    let view = self.start_share();
                    self.open_sheet(Sheet::Phone { view });
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            UiAction::ShareNewVersion => {
                if matches!(self.sheet, Some(Sheet::Phone { .. })) {
                    let view = self.start_share();
                    self.sheet = Some(Sheet::Phone { view });
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            UiAction::StopSharing => {
                self.share = None;
                if matches!(self.sheet, Some(Sheet::Phone { .. })) {
                    self.sheet = None;
                }
            }
            #[cfg(target_arch = "wasm32")]
            UiAction::OpenOnPhone | UiAction::ShareNewVersion | UiAction::StopSharing => {}
            UiAction::ViewerPanel(panel) => self.viewer_panel = panel,
            UiAction::SelectParts(ids) if self.features.read_only => {
                self.viewer_panel = None;
                self.editor.select_parts(&ids);
            }
            UiAction::SelectParts(_) => {}
            UiAction::Isolate if self.features.read_only => {
                self.editor.isolate_selection();
            }
            UiAction::ShowAll if self.features.read_only => {
                self.editor.show_all();
            }
            UiAction::Isolate | UiAction::ShowAll => {}
            UiAction::KeepModel => {
                if self.features.read_only {
                    self.platform.keep_open_model();
                }
            }
            UiAction::MeasureMode(on) => {
                if on {
                    self.viewer_panel = None;
                    self.editor.start_measure();
                } else {
                    self.editor.stop_measure();
                }
            }
            UiAction::OpenKept(id) => {
                self.viewer_panel = None;
                self.platform.open_recent(&id);
            }
            UiAction::DeleteKept(id) => self.platform.delete_recent(&id),
            UiAction::UnsavedAnswer(choice) => {
                if matches!(self.sheet, Some(Sheet::Unsaved { .. })) {
                    self.sheet = None;
                    self.platform_reply(PlatformReply::Unsaved(choice));
                }
            }
            UiAction::SetTheme(choice) => {
                self.prefs.theme = choice;
                self.gui.set_theme(choice);
                self.save_prefs();
            }
            UiAction::ShowSettings(open) => self.settings_open = open,
            UiAction::SetUiScale(scale) => {
                if self.prefs.ui_scale != scale {
                    self.prefs.ui_scale = scale;
                    self.gui.set_ui_scale(scale);
                    set_min_window(&self.window, scale);
                    self.save_prefs();
                }
            }
            UiAction::SetTrackpad(mode) => {
                if self.prefs.trackpad != mode {
                    self.prefs.trackpad = mode;
                    self.editor.set_trackpad(mode);
                    self.save_prefs();
                }
            }
            UiAction::SetDock(dock) => {
                if self.prefs.dock != dock {
                    // Shown again, the Library tab lists the folder again.
                    if dock.tab == DockTab::Library && self.prefs.dock.tab != DockTab::Library {
                        self.library_changed();
                    }
                    self.prefs.dock = dock;
                    self.save_prefs();
                }
            }
            UiAction::ShowSoften(open) => {
                self.editor.show_soften(open);
            }
            UiAction::SetSoftenAngle(angle) => {
                self.editor.set_soften_angle(angle);
            }
            UiAction::Soften => {
                self.editor.soften_selection();
            }
            UiAction::IntersectFaces(scope) => {
                self.editor.intersect_faces(scope);
            }
            UiAction::SolidTool(op) => {
                self.editor.solid_tool(op);
            }
            UiAction::CheckInterference => {
                self.editor.check_interference();
            }
            UiAction::InterferencePair(index) => {
                self.editor.select_interference_pair(index);
            }
            UiAction::CloseInterference => {
                self.editor.close_interference();
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
            UiAction::OpenRecent(path) => {
                // A file that fails to open says why in the status and stays in the list.
                self.settle_and(Then::OpenFile(path));
            }
            UiAction::ClearRecent => {
                if !self.prefs.recent.is_empty() {
                    self.prefs.recent.clear();
                    self.recent_shown.clear();
                    self.save_prefs();
                }
            }
            UiAction::DismissStart => self.start_dismissed = true,
            UiAction::ShowRecover(true) => self.list_recovered(true),
            UiAction::ShowRecover(false) => self.recover = None,
            UiAction::OpenRecovered(model) => self.open_recovered(&model),
            UiAction::DiscardRecovered(which) => {
                let entries = self.recover.as_ref().map_or(&[][..], |r| &r.entries[..]);
                if let Some(sheet) = discard_sheet(entries, which.as_deref()) {
                    self.open_sheet(sheet);
                }
            }
            UiAction::ConfirmDiscardRecovered => {
                if let Some(Sheet::DiscardRecovered { models, .. }) =
                    self.confirm_sheet(|s| matches!(s, Sheet::DiscardRecovered { .. }))
                {
                    self.discard_recovered(&models);
                }
            }
            UiAction::ChooseLibraryFolder => self.choose_library_folder(),
            UiAction::ClearLibraryFolder => self.set_library_folder(None),
            UiAction::LibraryUse(path) => self.library_use(&path),
            UiAction::LibraryInsert(path) => self.library_insert(&path),
            UiAction::LibraryOpen(path) => self.library_open(&path),
            UiAction::LibraryDelete(path) => self.library_ask_delete(path),
            UiAction::ConfirmLibraryDelete => {
                if let Some(Sheet::DeleteLibraryItem { path, .. }) =
                    self.confirm_sheet(|s| matches!(s, Sheet::DeleteLibraryItem { .. }))
                {
                    self.library_delete(&path);
                }
            }
            UiAction::ThumbnailsWanted(paths) => self.want_thumbnails(&paths),
            UiAction::SaveToLibrary(save) => self.library_ask_name(save),
            UiAction::ConfirmSaveToLibrary(name) => {
                if let Some(Sheet::SaveToLibrary { save, .. }) =
                    self.confirm_sheet(|s| matches!(s, Sheet::SaveToLibrary { .. }))
                {
                    self.library_save_named(save, &name);
                }
            }
            UiAction::ConfirmOverwriteLibrary => {
                if let Some(Sheet::OverwriteLibrary { save, name, file }) =
                    self.confirm_sheet(|s| matches!(s, Sheet::OverwriteLibrary { .. }))
                {
                    self.library_write(save, &name, &file, true);
                }
            }
            UiAction::SetAutosave(minutes) => {
                let minutes = minutes.min(prefs::MAX_AUTOSAVE_MINUTES);
                if self.prefs.autosave_minutes != minutes {
                    self.prefs.autosave_minutes = minutes;
                    self.save_prefs();
                }
            }
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        let (width, height) =
            fit_surface(width, height, self.device.limits().max_texture_dimension_2d);
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Makes the device, the surface and everything drawn with them again after the device was
    /// lost. False, logged, when that fails (tried again later).
    fn recreate_gpu(&mut self) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            // A browser hands out its device asynchronously: only a reload gets a new one.
            eprintln!("the GPU device was lost; reload the page");
            false
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.recreate_native_gpu()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn recreate_native_gpu(&mut self) -> bool {
        let size = self.window.inner_size();
        let made = native_context(&self.instance, &self.window)
            .and_then(|context| gpu(context, (size.width, size.height)));
        let Gpu {
            surface,
            device,
            queue,
            config,
            lost,
            shadows,
            occlusion,
        } = match made {
            Ok(gpu) => gpu,
            Err(e) => {
                eprintln!("cannot make the GPU device again: {e}");
                return false;
            }
        };
        eprintln!("GPU device made again");
        self.renderer = make_renderer(&device, &queue, config.format, shadows, occlusion);
        self.offscreen = Offscreen::with(make_snapshotter(&device, &queue, occlusion));
        // The window and its AccessKit adapter stay; only the panels' renderer is new.
        self.gui = self.gui.rebuilt(
            &self.window,
            &device,
            config.format,
            self.prefs.theme,
            self.prefs.ui_scale,
        );
        // The swatches belonged to the old panels' context.
        self.swatches.clear();
        self.library.forget_thumbnails();
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
        // A scene transition draws interpolated state; the document is already at the end.
        let motion = self.editor.transition_frame(Instant::now());
        let animating = motion.is_some();
        // The hover is worked out once per frame, not per event.
        self.editor.refresh_hover();
        let doc = self.editor.doc();
        let modified = doc.is_modified();
        self.platform.set_modified(modified);
        // Where files have no path, the name the model was opened from or saved under.
        let kept = self.flows.kept_name().map(Path::new);
        let title = if self.features.read_only {
            format!("{} - sketchup-alt", self.model_name)
        } else {
            window_title(
                doc.path()
                    .or(kept.filter(|_| doc.imported_from().is_none())),
                doc.imported_from(),
                self.recovered_doc().map(|r| r.name.as_str()),
                modified,
                cfg!(target_os = "macos"),
            )
        };
        if title != self.title {
            self.window.set_title(&title);
            self.title = title;
        }
        #[cfg(target_os = "macos")]
        if modified != self.edited {
            winit::platform::macos::WindowExtMacOS::set_document_edited(&*self.window, modified);
            self.edited = modified;
        }
        let status = self.editor.status_parts();
        let info = entity_info_with(self.editor.doc(), &|i| self.editor.selected_solid(i));
        let materials = self.material_rows();
        // Thumbnails tiles in view asked for last frame; more wait for the next.
        self.make_thumbnails();
        self.update_library_rows();
        // Rebuilt only when the model changes; the open and selected marks are worked out
        // while drawing, from the context and selection.
        let revision = self.editor.doc().revision();
        if self.outliner.as_ref().map(|(r, _)| *r) != Some(revision) {
            self.outliner = Some((revision, self.editor.doc().outliner_rows()));
        }
        refresh_tag_rows(&mut self.tag_rows, &self.editor);
        // Built every frame: going to a scene selects it without a new revision, and there are
        // few of them.
        let scenes = scene_rows(self.editor.doc());
        let joints: Vec<JointRow> = self
            .editor
            .joint_infos()
            .into_iter()
            .map(|info| JointRow {
                selected: self.editor.joint_selected(&info),
                follows: self.editor.doc().scene().follower_of(info.id).is_some(),
                info,
            })
            .collect();
        let styles = style_rows(self.editor.doc());
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
        if let Some(message) = self.editor.take_flash() {
            self.timing.flash(message, now);
        }
        // Over a window, a menu or the gizmo a press never reaches the tools: no marker, chip
        // or highlight there. The gizmo (Task 11) is an egui Area, so this test covers it.
        let covered = self.in_view(self.cursor) && self.gui.wants_pointer_at(self.cursor);
        // The renderer and the API read the hover through the editor: one answer for both.
        self.editor.set_covered(covered);
        let dark_canvas =
            self.editor.doc().lighting == Lighting::Night || self.gui.theme() == egui::Theme::Dark;
        let navigating = self.editor.navigating();
        // While a transition runs the labels follow the drawn camera and instances.
        let hud = match &motion {
            Some(frame) => self.editor.transition_hud(dark_canvas, frame),
            None => self.editor.hud(dark_canvas),
        };
        // Whether the rest's end would show a chip: none to show, or one hidden while
        // navigating or under a window, is no reason to wake (Select over empty space, an
        // orbit drag).
        let chip_at_rest = hud.chip.is_some() && !navigating && !covered;
        let hud = self.timing.dress(hud, now, navigating);
        let hud = if covered { hud.covered() } else { hud };
        let dressed_at = now;
        let lines: Option<Vec<String>> = self.agent_activity.then(|| {
            hub.activity()
                .iter()
                .rev()
                .map(|a| activity_line(a, now))
                .collect()
        });
        // A tool asked for a note's text: the note sheet asks, once no other sheet is open.
        if let Some(sheet) = next_note_sheet(self.sheet.as_ref(), &mut self.editor) {
            self.open_sheet(sheet);
        }
        self.drop_outlived_sheet();
        #[cfg(not(target_arch = "wasm32"))]
        self.refresh_share();
        let rename = self.editor.rename_target();
        let component = rename
            .as_ref()
            .filter(|t| t.kind == kernel::DefinitionKind::Component)
            .and_then(|_| match self.editor.doc().selection().first() {
                Some(EntityId::Instance(id)) => Some(*id),
                _ => None,
            });
        let viewing = self.features.read_only;
        let part_rows = if viewing && self.viewer_panel == Some(ViewerPanel::Parts) {
            self.editor.part_rows()
        } else {
            Vec::new()
        };
        let part_card = if viewing {
            self.editor.part_card()
        } else {
            None
        };
        let measure_text = self
            .editor
            .measurement()
            .map(|m| m.text(&self.editor.doc().units()));
        let (recent_shares, recent_note) =
            if viewing && self.viewer_panel == Some(ViewerPanel::Recent) {
                (self.platform.recent_shares(), self.platform.recent_note())
            } else {
                (Vec::new(), None)
            };
        let viewport_menu = self
            .editor
            .open_viewport_menu(&mut self.viewport_menu, self.menu_face);
        let panels = View {
            viewer: ViewerView {
                panel: self.viewer_panel,
                rows: &part_rows,
                units: self.editor.doc().units(),
                card: part_card.as_ref(),
                isolated: self.editor.isolated(),
                measure: measure_text.as_deref(),
                recent: &recent_shares,
                recent_note: recent_note.as_deref(),
                can_keep: viewing && self.platform.can_keep_open_model(),
            },
            library: LibraryView {
                folder: self.prefs.library.as_deref(),
                rows: self.library.rows(),
                more: self.library.more(),
                error: self.library.error(),
            },
            component,
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
            has_clip: self.editor.clip_ready(),
            has_instance_selected: self
                .editor
                .doc()
                .selection()
                .iter()
                .any(|id| matches!(id, EntityId::Instance(_))),
            can_make_unique: self.editor.can_make_unique(),
            can_flip_own: self.editor.can_flip_own(),
            solid_tools: self.editor.solid_tools(),
            inside_instance: !self.editor.doc().context().is_empty(),
            has_guides: self.editor.doc().model().guides().next().is_some(),
            projection: self.editor.doc().camera.projection,
            lighting: self.editor.doc().lighting,
            flying: self.editor.flying(),
            materials: &materials,
            material: self.editor.current_material(),
            tags: self.tag_rows.as_ref().map(|(_, t)| t).expect("built above"),
            color_by_tag: self.editor.doc().color_by_tag,
            styles: &styles,
            ambient_occlusion: self.renderer.ambient_occlusion_available(),
            style: self.editor.doc().style(),
            section_display: self.editor.doc().section_display,
            x_ray: self.editor.doc().x_ray,
            hidden_geometry: self.editor.doc().hidden_geometry,
            shadows: self.editor.doc().shadows,
            location: self.editor.doc().scene().location(),
            viewport_menu,
            scenes: &scenes,
            joints: &joints,
            documents: self.editor.documents(),
            selection_tag: self.editor.selection_tag(),
            outliner: self.outliner.as_ref().map(|(_, t)| t).expect("built above"),
            context: &context,
            selection: self.editor.doc().selection(),
            soften: self.editor.soften_window(),
            interference: self.editor.interference_window(),
            edges_soft: self.editor.selection_soft(),
            selection_hidden: self.editor.selection_hidden(),
            selection_section_cut: self.editor.selection_section_cut(),
            has_hidden: self.editor.context_has_hidden(),
            has_last_hidden: self.editor.has_last_hidden(),
            has_hidden_selected: self.editor.selection_has_hidden(),
            has_face_selected: self.editor.selection_has_faces(),
            theme: self.prefs.theme,
            dock: self.prefs.dock,
            shortcuts: self.shortcuts_window,
            // macOS has the native menu bar instead.
            egui_menus: !cfg!(target_os = "macos"),
            texture_position: self.editor.selection_texture_position().map(|placement| {
                TexturePosition {
                    placement,
                    faces: self.editor.doc().selection().iter().copied().collect(),
                }
            }),
            hud: &hud,
            cursor: viewport::cursor_icon(
                self.editor.tool_kind(),
                self.editor.pasting(),
                self.editor.hovered().is_some(),
                navigating,
            ),
            camera: motion
                .as_ref()
                .map_or(&self.editor.doc().camera, |m| &m.camera),
            axes: self.editor.doc().drawing_axes_in_world().matrix3,
            start: (self.editor.untouched() && !self.start_dismissed && !self.editor.flying())
                .then_some(StartCard {
                    recent: &self.recent_shown,
                }),
            features: self.features,
            model_name: &self.model_name,
            recent: &self.prefs.recent,
            sheet: self.sheet.as_ref(),
            units: self.editor.doc().units(),
            rename: rename.as_ref(),
            settings: self.settings_open.then_some(SettingsView {
                ui_scale: self.prefs.ui_scale,
                trackpad: self.prefs.trackpad,
                autosave_minutes: self.prefs.autosave_minutes,
                units: self.editor.doc().units(),
                transition: self.editor.doc().scene().scene_transition(),
            }),
            recover: self.recover.as_ref().map(|r| RecoverView {
                rows: &r.rows,
                older: r.older,
            }),
        };
        let mut gui_frame = self.gui.frame(&self.window, &panels);
        #[cfg(target_os = "macos")]
        {
            let bar = menus::menu_bar(&panels, true);
            self.native
                .get_or_insert_with(|| NativeMenu::new(&bar))
                .sync(&bar, self.gui.wants_keyboard(), self.sheet.is_some());
        }
        gui_frame.hide_cursor = self.capture == Capture::Grabbed;
        // The tools pick dimensions and notes by the labels this frame drew.
        self.editor
            .set_labels(std::mem::take(&mut gui_frame.labels));
        // Applied once the frame is presented: some open modal dialogs, which must not run
        // while this frame's surface texture is held.
        let actions = std::mem::take(&mut gui_frame.actions);
        let mut acted = false;
        // The editor's view never reaches past the surface (a surface fitted to the GPU's
        // largest texture may be smaller than the window).
        gui_frame.viewport.width = gui_frame
            .viewport
            .width
            .min(self.config.width.saturating_sub(gui_frame.viewport.x));
        gui_frame.viewport.height = gui_frame
            .viewport
            .height
            .min(self.config.height.saturating_sub(gui_frame.viewport.y));
        if gui_frame.viewport != self.view {
            self.view = gui_frame.viewport;
            let v = self.view;
            self.editor
                .resize(DVec2::new(v.width.max(1) as f64, v.height.max(1) as f64));
        }
        if std::mem::take(&mut self.fit_pending) {
            self.editor.zoom_extents();
        }

        let doc = self.editor.doc();
        // The frames of a transition that moves instances rebuild the model, and the first one
        // after it does too; a camera-only transition keeps it.
        let moving = motion.as_ref().is_some_and(|m| m.scene.is_some());
        let update = if moving {
            SceneUpdate::Model
        } else {
            scene_update(
                self.drawn.as_ref(),
                doc.revision(),
                doc.selection(),
                doc.context(),
            )
        };
        // Coloring by tag changes how faces are colored, not the revision.
        let recolored = self.renderer.set_color_by_tag(doc.color_by_tag);
        // The chosen style's edge colors do too, and its background reaches the canvas.
        let recolored = self.renderer.set_model_style(doc.chosen_style()) || recolored;
        // So do the section display settings.
        let recolored = self.renderer.set_section_display(doc.section_display) || recolored;
        let recolored = self.renderer.set_glass_view(render::scene::GlassView {
            x_ray: doc.x_ray,
            hidden_geometry: doc.hidden_geometry && !self.features.read_only,
        }) || recolored;
        if update == SceneUpdate::Model || recolored {
            let scene = motion
                .as_ref()
                .and_then(|m| m.scene.as_ref())
                .unwrap_or(doc.scene());
            self.renderer.set_model(scene, doc.context());
        }
        if update != SceneUpdate::Keep || recolored {
            self.renderer.set_selection(doc.selection());
            self.drawn = Some((
                doc.revision(),
                doc.selection().clone(),
                doc.context().to_vec(),
            ));
            if moving {
                self.drawn = None;
            }
        }
        // What a click would act on, and the tool's solid: window feedback only (render_view's
        // renderer never shows them). Both are no-ops when unchanged.
        let paint = self.paint_color();
        let tool = self.editor.tool_kind();
        let ui_theme = self.gui.theme();
        let sampling = self.editor.sampling();
        // No hover while a transition draws: it is of the document's view.
        self.renderer.set_hover(
            self.editor
                .hovered()
                .filter(|_| !animating)
                .map(|id| (id, theme::hover_look(ui_theme, tool, id, paint, sampling))),
        );
        self.renderer.set_preview(&self.editor.solid_preview());
        // The view follows the panels' theme; render_view keeps its own renderer's classic look.
        self.renderer.set_style(theme::view_style(self.gui.theme()));
        // The view's widths in points follow the UI scale, as the panels' do.
        self.renderer
            .set_pixels_per_point(gui_frame.pixels_per_point);
        self.renderer.set_lighting(doc.lighting);
        self.renderer.set_sun(render::SunLight::of(doc));
        // The tools' marker and lines are egui's (the HUD); the renderer draws no overlay.
        self.renderer.render_in(
            &target,
            size,
            self.view,
            motion
                .as_ref()
                .map_or(&self.editor.doc().camera, |m| &m.camera),
            &[],
        );
        // Text copied in a text field reaches a clipboard the window does not otherwise reach.
        if self.features.paste_by_event {
            for text in gui_frame.copied_texts() {
                self.platform.copied(text);
            }
        }
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
        if flying_on || animating {
            self.window.request_redraw();
        }
        // The ages in the open log move.
        if lines.is_some() {
            self.repaint_at = Some(earliest(self.repaint_at, now + Duration::from_secs(1)));
        }
        // The share's time left moves.
        if self.share_ticks() {
            self.repaint_at = Some(earliest(self.repaint_at, now + Duration::from_secs(1)));
        }
        if self.editor.hover_deferred() {
            self.repaint_at = Some(earliest(self.repaint_at, now + HOVER_SETTLE));
        }
        // The chip's rest and the flash's end, as of the HUD this frame showed.
        self.repaint_at = with_feedback(self.repaint_at, &self.timing, dressed_at, chip_at_rest);
        // Actions from a pass with a sheet open (or after one opened) are only the sheet's own.
        let sheet_was_open = self.sheet.is_some();
        for action in actions {
            if !action_applies(sheet_was_open || self.sheet.is_some(), &action) {
                continue;
            }
            // Only what was applied asks for another frame: a tile's thumbnail request
            // dropped under a sheet must not keep the window redrawing.
            acted = true;
            self.apply(action);
        }
        // A print waits for a frame that shows its message.
        if let Some(job) = self.print_queue.frame_done() {
            self.run_print(job);
        }
        if acted || self.print_queue.waiting() {
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
            session: self.session.as_ref(),
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
            PhysicalKey::Code(KeyCode::KeyA) => Some('a'),
            PhysicalKey::Code(KeyCode::KeyD) => Some('d'),
            PhysicalKey::Code(KeyCode::KeyX) => Some('x'),
            PhysicalKey::Code(KeyCode::KeyC) => Some('c'),
            PhysicalKey::Code(KeyCode::KeyV) => Some('v'),
            PhysicalKey::Code(KeyCode::Comma) => Some(','),
            _ => None,
        },
    }
}

/// What a sheet was opened over: the document (`Editor::documents`) and the selection.
#[derive(Clone, Debug, Default, PartialEq)]
struct SheetOrigin {
    document: u64,
    selection: BTreeSet<EntityId>,
}

impl SheetOrigin {
    fn of(editor: &Editor) -> Self {
        Self {
            document: editor.documents(),
            selection: editor.doc().selection().clone(),
        }
    }
}

/// Whether `sheet`, opened over `origin`, still applies to what `editor` holds now (an agent
/// may have changed it meanwhile). Err: what the status says when it does not.
fn sheet_applies(sheet: &Sheet, origin: &SheetOrigin, editor: &Editor) -> Result<(), &'static str> {
    // Recovered copies and library files are not the model's.
    if matches!(
        sheet,
        Sheet::DiscardRecovered { .. }
            | Sheet::DeleteLibraryItem { .. }
            | Sheet::Unsaved { .. }
            | Sheet::About { .. }
            | Sheet::Update { .. }
            | Sheet::Phone { .. }
    ) {
        return Ok(());
    }
    if editor.documents() != origin.document {
        return Err("The model was replaced; nothing was done");
    }
    match sheet {
        Sheet::Import { .. } | Sheet::Print { .. } | Sheet::Export { .. } => Ok(()),
        Sheet::NameComponent { .. } if *editor.doc().selection() != origin.selection => {
            Err("The selection changed; no component was made")
        }
        Sheet::NameComponent { .. } => Ok(()),
        Sheet::Rename { target }
            if editor.rename_target().map(|t| t.definition) != Some(target.definition) =>
        {
            Err("The group or component is no longer selected; nothing was renamed")
        }
        Sheet::Rename { .. } => Ok(()),
        Sheet::FieldOfView { .. } if editor.doc().camera.is_parallel() => {
            Err("The field of view applies only to a perspective view")
        }
        Sheet::FieldOfView { .. } => Ok(()),
        Sheet::DeleteTag { tag, .. } if editor.doc().scene().tag(*tag).is_none() => {
            Err("The tag was already deleted")
        }
        Sheet::DeleteTag { .. }
        | Sheet::DiscardRecovered { .. }
        | Sheet::Unsaved { .. }
        | Sheet::About { .. }
        | Sheet::Update { .. }
        | Sheet::Phone { .. }
        | Sheet::DeleteLibraryItem { .. } => Ok(()),
        Sheet::SaveToLibrary { save, .. } | Sheet::OverwriteLibrary { save, .. } => {
            let doc = editor.doc();
            match save {
                LibrarySave::Component(id) if doc.model().instance(*id).is_none() => {
                    Err("The component is gone; nothing was saved")
                }
                LibrarySave::Material(id) if doc.scene().material(*id).is_none() => {
                    Err("The material is gone; nothing was saved")
                }
                _ => Ok(()),
            }
        }
        Sheet::NoteText {
            request: NoteRequest::Edit(EntityId::Note(n)),
            ..
        } if editor.doc().model().note(*n).is_none() => {
            Err("The note is gone; its text was not changed")
        }
        Sheet::NoteText {
            request: NoteRequest::Edit(_),
            ..
        } => Ok(()),
        Sheet::NoteText { .. } if !editor.awaits_note_text() => {
            Err("The note is no longer being placed; nothing was added")
        }
        Sheet::NoteText { .. } | Sheet::Text3d { .. } => Ok(()),
    }
}

/// What the 3D Text sheet first opens with: no text, capital letters 0.3 m tall, filled and
/// 0.05 m deep.
fn default_text_3d() -> TextSpec {
    TextSpec {
        text: String::new(),
        height: 0.3,
        depth: 0.05,
        filled: true,
    }
}

/// The note sheet to open now for the note a tool asked about, if any. While another sheet is
/// open the request waits in the editor until it closes (the user's sheet is never replaced);
/// the note sheet of a note being placed gives way to the next one the Text tool asks about.
fn next_note_sheet(open: Option<&Sheet>, editor: &mut Editor) -> Option<Sheet> {
    let replaceable = match open {
        None => true,
        Some(Sheet::NoteText { request, .. }) => !matches!(request, NoteRequest::Edit(_)),
        Some(_) => false,
    };
    if !replaceable {
        return None;
    }
    let request = editor.take_note_request()?;
    note_sheet(editor, request)
}

/// The note sheet for `request`, when it still applies (a note to change may be gone, a note
/// being placed dropped meanwhile).
fn note_sheet(editor: &Editor, request: NoteRequest) -> Option<Sheet> {
    if !matches!(request, NoteRequest::Edit(_)) && !editor.awaits_note_text() {
        return None;
    }
    let start = editor.note_text(request)?;
    Some(Sheet::NoteText {
        request,
        text: start.text,
        automatic: start.automatic,
    })
}

/// Rebuilds the Tags tab's rows when the document or its revision changed since `cache` was
/// built (an edit, an undo, or an agent's command).
fn refresh_tag_rows(cache: &mut Option<((u64, u64), Vec<TagRow>)>, editor: &Editor) {
    let built = (editor.documents(), editor.doc().revision());
    if cache.as_ref().map(|(b, _)| *b) != Some(built) {
        *cache = Some((built, tag_rows(editor.doc())));
    }
}

/// The Tags tab's rows: every tag, in id order, with how many entities are on it.
fn tag_rows(doc: &session::Document) -> Vec<TagRow> {
    let scene = doc.scene();
    scene
        .tags()
        .map(|(id, t)| TagRow {
            id,
            name: t.name.clone(),
            color: t.color,
            visible: t.visible,
            count: scene.tag_count(id),
        })
        .collect()
}

/// The Styles tab's rows: every style, in the order they were made.
fn style_rows(doc: &session::Document) -> Vec<StyleRow> {
    doc.scene()
        .styles()
        .map(|(id, s)| StyleRow {
            id,
            name: s.name.clone(),
            settings: s.settings,
        })
        .collect()
}

/// The scene tabs' and the Scenes tab's rows: every scene, in order.
fn scene_rows(doc: &session::Document) -> Vec<SceneRow> {
    let selected = doc.selected_scene();
    doc.scene()
        .pages()
        .map(|(id, p)| SceneRow {
            id,
            name: p.name.clone(),
            camera: p.view.is_some(),
            tags: p.tags.is_some(),
            positions: p.positions.is_some(),
            joints: p.joints.is_some(),
            selected: selected == Some(id),
        })
        .collect()
}

/// The Delete Tag sheet for `tag`, when something is on it (otherwise it is deleted at once).
fn delete_tag_sheet(doc: &session::Document, tag: TagId) -> Option<Sheet> {
    let scene = doc.scene();
    let name = scene.tag(tag)?.name.clone();
    let count = scene.tag_count(tag);
    (count > 0).then(|| Sheet::DeleteTag {
        tag,
        name,
        count,
        others: scene
            .tags()
            .filter(|(id, _)| *id != tag)
            .map(|(id, t)| (id, t.name.clone()))
            .collect(),
    })
}

/// Ends fly mode before a sheet opens. Leaving stops the eye and lets every held key go, so a
/// key held now does not move it once the sheet closes.
fn end_fly_for_sheet(editor: &mut Editor) {
    editor.set_flying(false);
}

/// Whether fly mode holds the pointer: flying in the focused window with no sheet open.
fn holds_pointer(flying: bool, focused: bool, sheet_open: bool) -> bool {
    flying && focused && !sheet_open
}

/// Whether a key press is the Quit chord on the window's key path (Ctrl+Q, and Cmd+Q without
/// native menus): it quits, through the unsaved prompt, even while a sheet is open.
fn is_quit_chord(letter: Option<char>, command_key: bool, shift: bool, cmd: bool) -> bool {
    command_key
        && letter
            .and_then(|l| shortcuts::window_command(l, shift, false, cmd, shortcuts::NATIVE_MENUS))
            == Some(Bound::Quit)
}

/// The table's keys that are not characters, which the window's key path acts on itself.
fn window_key(logical: &Key) -> Option<shortcuts::Key> {
    match logical {
        Key::Named(NamedKey::F2) => Some(shortcuts::Key::F2),
        Key::Named(NamedKey::PageUp) => Some(shortcuts::Key::PageUp),
        Key::Named(NamedKey::PageDown) => Some(shortcuts::Key::PageDown),
        _ => None,
    }
}

/// Whether a key press goes on to the editor and the shortcut table: not while a text field
/// has focus (the key is its), and not while a sheet is open (egui still gets every key, so the
/// sheet's own fields and buttons work).
fn keys_reach_editor(text_focused: bool, sheet_open: bool) -> bool {
    !text_focused && !sheet_open
}

/// Whether a panel's `action` is applied: always without a sheet, with one only if
/// `action_allowed_with_sheet`.
fn action_applies(sheet_open: bool, action: &UiAction) -> bool {
    !sheet_open || action_allowed_with_sheet(action)
}

/// Whether `action` is applied while a sheet is open: only closing or confirming the sheet,
/// and Quit. egui presses a widget behind the sheet's modal layer on a screen reader's Click,
/// so the panels' actions can still arrive and are dropped here.
fn action_allowed_with_sheet(action: &UiAction) -> bool {
    matches!(
        action,
        UiAction::CloseSheet
            | UiAction::ConfirmImport(_)
            | UiAction::ConfirmPrint(_)
            | UiAction::ConfirmExport(_)
            | UiAction::ConfirmName(_)
            | UiAction::ConfirmFieldOfView(_)
            | UiAction::ConfirmDeleteTag(_)
            | UiAction::ConfirmDiscardRecovered
            | UiAction::ConfirmLibraryDelete
            | UiAction::ThumbnailsWanted(_)
            | UiAction::ConfirmSaveToLibrary(_)
            | UiAction::ConfirmOverwriteLibrary
            | UiAction::ConfirmNote(_)
            | UiAction::Confirm3dText(_)
            | UiAction::UnsavedAnswer(_)
            | UiAction::ShareNewVersion
            | UiAction::StopSharing
            | UiAction::Quit
    )
}

/// Whether a scroll or pinch goes to the editor: not while a sheet is open, and otherwise
/// while flying (a held pointer may rest over a panel) or with the pointer in the view.
fn view_input_reaches_editor(flying: bool, pointer_in_view: bool, sheet_open: bool) -> bool {
    !sheet_open && (flying || pointer_in_view)
}

/// A file dialog for `.skr` models.
/// The window title: the file name alone on macOS, where the close button shows unsaved
/// changes; elsewhere "house.skr* - sketchup-alt". A model opened from a SketchUp file and
/// not saved yet is "desk.skp (imported)"; one opened from the Recover Models window
/// (`recovered`: its name) "house (recovered)".
fn window_title(
    path: Option<&Path>,
    imported_from: Option<&Path>,
    recovered: Option<&str>,
    modified: bool,
    mac: bool,
) -> String {
    let name = match (path, imported_from.and_then(|p| p.file_name()), recovered) {
        (None, Some(n), _) => format!("{} (imported)", n.to_string_lossy()),
        (None, None, Some(name)) => format!("{name} (recovered)"),
        _ => file_name(path),
    };
    if mac {
        return name;
    }
    let star = if modified { "*" } else { "" };
    format!("{name}{star} - sketchup-alt")
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

/// The id the mouse's left button has among the viewer's touches.
const MOUSE_TOUCH: u64 = u64::MAX;

/// The file name a shared model is opened under: the shared name is only for the bar (a model
/// called "Plan.skp" is still a `.skr`).
const SHARED_FILE: &str = "shared.skr";

/// Opens a shared model in `editor`, and only when that works makes `name` its name for the bar.
fn open_shared_model(editor: &mut Editor, name: &mut String, model: SharedModel) -> bool {
    if editor.open_bytes(SHARED_FILE, &model.bytes) {
        *name = model.name;
        true
    } else {
        false
    }
}

/// The name Open on Phone gives a model: its file's name without the extension, as the window
/// title names it (a model opened from a SketchUp file and not saved yet: that file), else
/// the name the system kept for it, else "Untitled".
#[cfg(not(target_arch = "wasm32"))]
fn share_name(path: Option<&Path>, imported_from: Option<&Path>, kept: Option<&str>) -> String {
    let stem = |p: &Path| p.file_stem().map(|n| n.to_string_lossy().into_owned());
    path.and_then(stem)
        .or_else(|| imported_from.and_then(stem))
        .or_else(|| kept.and_then(|k| stem(Path::new(k))))
        .unwrap_or_else(|| "Untitled".to_string())
}

/// The actions the viewer lets through (`Features::read_only`): the rest would change the
/// model or open what a viewer does not have. The document refuses edits as well
/// (`Document::set_read_only`); this keeps the window from starting them.
fn viewer_allows(action: &UiAction) -> bool {
    matches!(
        action,
        UiAction::SelectNone
            | UiAction::ViewerPanel(_)
            | UiAction::SelectParts(_)
            | UiAction::Isolate
            | UiAction::ShowAll
            | UiAction::KeepModel
            | UiAction::MeasureMode(_)
            | UiAction::OpenKept(_)
            | UiAction::DeleteKept(_)
            | UiAction::CloseSheet
            | UiAction::ZoomExtents
            | UiAction::StandardView(_)
            | UiAction::SetProjection(_)
            | UiAction::SetLighting(_)
            | UiAction::SetTheme(_)
    )
}

/// Whether a key event is Escape going down (the viewer's one key).
fn escape_pressed(key: &Key, state: ElementState) -> bool {
    state == ElementState::Pressed && matches!(key, Key::Named(NamedKey::Escape))
}

fn map_button(button: MouseButton) -> Button {
    match button {
        MouseButton::Left => Button::Left,
        MouseButton::Middle => Button::Middle,
        _ => Button::Other,
    }
}

/// What `App::new` needs besides the event loop's proxy.
/// A model given as bytes (`AppConfig::model`).
#[derive(Debug)]
pub struct SharedModel {
    /// The name the viewer's top bar shows.
    pub name: String,
    /// A `.skr` file's bytes.
    pub bytes: Vec<u8>,
}

/// What the viewer says of a model it cannot open.
pub const DAMAGED_LINK: &str = "This link is damaged or expired";

pub struct AppConfig {
    /// Files, preferences, dialogs, the clipboard and links.
    pub platform: Box<dyn Platform>,
    /// The GPU objects, made beforehand where they can only be requested asynchronously (a
    /// browser). None: the window makes them when it opens, waiting for them.
    pub gpu: Option<GpuContext>,
    /// How the window is made, where the platform shapes it (a browser's window is a canvas of
    /// the page). None: the desktop window.
    pub window: Option<WindowAttributes>,
    /// A model to open once the window exists.
    pub file: Option<PathBuf>,
    /// A model given as bytes, opened once the window exists and fitted to the view (the
    /// phone viewer's: with `Features::VIEWER` it is opened read-only). When it cannot be
    /// opened the window says "This link is damaged or expired".
    pub model: Option<SharedModel>,
    /// The folders the API's file paths must lie within.
    pub roots: ApiRoots,
    /// Where work an agent discards is kept.
    pub recovery: Option<PathBuf>,
    /// The recovery session, handed to the window when it is made (Err: why there is none).
    pub session: Result<Session, String>,
    /// Where the agent API listens; None runs without it.
    #[cfg(not(target_arch = "wasm32"))]
    pub socket: Option<Endpoint>,
}

impl AppConfig {
    /// A window with nothing beyond `platform`: no API folders, recovery, socket or model.
    pub fn new(platform: Box<dyn Platform>) -> Self {
        Self {
            platform,
            gpu: None,
            window: None,
            file: None,
            model: None,
            roots: ApiRoots::default(),
            recovery: None,
            session: Err("there is no folder to keep autosaves in".into()),
            #[cfg(not(target_arch = "wasm32"))]
            socket: None,
        }
    }
}

pub struct App {
    state: Option<State>,
    hub: Hub,
    /// Held while agents are connected, so macOS does not nap the window they drive.
    #[cfg(not(target_arch = "wasm32"))]
    awake: Awake,
    /// Where the agent API listens; a socket file is removed on exit (None if listening failed).
    #[cfg(not(target_arch = "wasm32"))]
    socket: Option<Endpoint>,
    /// A model to open once the window exists.
    file: Option<PathBuf>,
    /// A model given as bytes, opened once the window exists.
    model: Option<SharedModel>,
    /// The folders the API's file paths must lie within.
    roots: ApiRoots,
    /// Where work an agent discards is kept.
    recovery: Option<PathBuf>,
    /// The recovery session, handed to the window when it is made (Err: why there is none).
    session: Option<Result<Session, String>>,
    /// The platform and the GPU, handed to the window when it is made.
    platform: Option<Box<dyn Platform>>,
    gpu: Option<GpuContext>,
    /// How the window is made (None: `window_attributes`).
    window_attributes: Option<WindowAttributes>,
    /// Carries native menu and AccessKit events to the loop.
    proxy: EventLoopProxy<AppEvent>,
}

impl App {
    /// The window app. `proxy` is the event loop's (`EventLoop::create_proxy`).
    pub fn new(proxy: EventLoopProxy<AppEvent>, config: AppConfig) -> Self {
        let api_proxy = proxy.clone();
        let hub = Hub::new(move || {
            let _ = api_proxy.send_event(AppEvent::Api);
        });
        #[cfg(not(target_arch = "wasm32"))]
        let socket = config
            .socket
            .and_then(|socket| match listen(&socket, hub.inbox()) {
                Ok(()) => {
                    eprintln!("agent API listening on {socket}");
                    Some(socket)
                }
                Err(e) => {
                    eprintln!("agent API disabled: cannot listen on {socket}: {e}");
                    None
                }
            });
        Self {
            state: None,
            hub,
            #[cfg(not(target_arch = "wasm32"))]
            awake: Awake::default(),
            #[cfg(not(target_arch = "wasm32"))]
            socket,
            file: config.file,
            model: config.model,
            roots: config.roots,
            recovery: config.recovery,
            session: Some(config.session),
            platform: Some(config.platform),
            gpu: config.gpu,
            window_attributes: config.window,
            proxy,
        }
    }

    /// Serves the agent API requests waiting in the hub.
    fn serve_api(&mut self, event_loop: &ActiveEventLoop) {
        // Wake before the work, not after: a napped app runs a request (a save, say) many
        // times slower, and an agent's first request often arrives with its connection.
        let state = &mut self.state;
        let hub = &mut self.hub;
        #[cfg(not(target_arch = "wasm32"))]
        self.awake.serve(|_| {
            if let Some(s) = state.as_mut() {
                s.process_api(hub);
            }
            hub.open_connections() > 0
        });
        #[cfg(target_arch = "wasm32")]
        if let Some(s) = state.as_mut() {
            s.process_api(hub);
        }
        // An agent asked to quit; any unsaved changes were refused or discarded already.
        if self.state.as_mut().is_some_and(State::should_exit) {
            event_loop.exit();
        }
    }

    /// A native menu item was chosen. Acts unless a held key raised it and the binding does
    /// not repeat, or a text field has focus and takes the chord itself (Cmd+Z, Cmd+A).
    #[cfg(target_os = "macos")]
    fn menu_event(&mut self, event_loop: &ActiveEventLoop, id: &str, repeat: bool) {
        let Some(s) = self.state.as_mut() else { return };
        // The next frame syncs the items again: a check item toggles itself when clicked,
        // even when nothing changes.
        s.window.request_redraw();
        let Some(action) = s.native.as_ref().and_then(|n| n.action(id)) else {
            return;
        };
        if !menus::native_event_acts(&action, repeat, s.gui.wants_keyboard(), s.sheet.is_some()) {
            return;
        }
        s.apply(action);
        s.sync_fly_cursor();
        s.notify_api(&mut self.hub);
        if s.should_exit() {
            event_loop.exit();
        }
    }
}

/// Where the files the system asked to open go (`AppEvent::OpenFiles`). The window holds one
/// model, so only the first is opened. Before the window exists it becomes the model to open at
/// start, in place of one given as an argument; once it exists, it is returned to open now.
fn route_opened_files(
    paths: Vec<PathBuf>,
    window: bool,
    at_start: &mut Option<PathBuf>,
) -> Option<PathBuf> {
    let path = paths.into_iter().next()?;
    if window {
        return Some(path);
    }
    *at_start = Some(path);
    None
}

impl ApplicationHandler<AppEvent> for App {
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // The last replies (a `quit`'s) reach their agents before the app ends.
        self.hub.close(CLOSE_WAIT);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(endpoint) = self.socket.take() {
            endpoint.remove();
        }
        // The session's lock goes with the window either way.
        if let Some(s) = self.state.as_mut() {
            s.autosave_at_exit();
            // A quit AppKit ends (Dock > Quit, log out) exits the process without dropping the
            // window, so the session's lock file goes here.
            s.session = None;
            // The share's server stops with the app.
            #[cfg(not(target_arch = "wasm32"))]
            {
                s.share = None;
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        // What a quit the system asks for finds (see `system_quit`).
        #[cfg(target_os = "macos")]
        if let Some(s) = self.state.as_mut() {
            crate::system_quit::set_unsaved(!s.quit && s.editor.doc().is_modified());
            // A prompt the system's quit raised may have moved on by another path.
            s.answer_system_quit();
        }
        let deadline = self
            .state
            .as_mut()
            .and_then(|s| wake_deadline(s.repaint_at, s.autosave_tick(now)));
        match wake_decision(now, deadline) {
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

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Api => self.serve_api(event_loop),
            #[cfg(target_os = "macos")]
            AppEvent::Menu { id, repeat } => self.menu_event(event_loop, &id, repeat),
            #[cfg(target_os = "macos")]
            AppEvent::SystemQuit => {
                if let Some(s) = self.state.as_mut() {
                    s.system_quit();
                    s.notify_api(&mut self.hub);
                    s.window.request_redraw();
                    if s.should_exit() {
                        event_loop.exit();
                    }
                } else {
                    crate::system_quit::reply(true);
                }
            }
            AppEvent::Error(text) => {
                if let Some(s) = self.state.as_mut() {
                    s.editor.show_error(&text);
                    s.window.request_redraw();
                }
            }
            AppEvent::Update(outcome) => {
                if let Some(s) = self.state.as_mut() {
                    // The sheet may have been closed while the check ran.
                    if let Some(Sheet::Update { outcome: slot }) = s.sheet.as_mut() {
                        *slot = Some(outcome);
                    }
                    s.window.request_redraw();
                }
            }
            AppEvent::OpenFiles(paths) => {
                let window = self.state.is_some();
                if let Some(path) = route_opened_files(paths, window, &mut self.file)
                    && let Some(s) = self.state.as_mut().filter(|s| !s.features.read_only)
                {
                    // As File > Open does once the file is picked: unsaved changes are asked
                    // about first.
                    s.settle_and(Then::OpenFile(path));
                    s.notify_api(&mut self.hub);
                    s.window.request_redraw();
                    if s.should_exit() {
                        event_loop.exit();
                    }
                }
            }
            AppEvent::Shared(model) => {
                if let Some(s) = self.state.as_mut().filter(|s| s.features.read_only) {
                    s.open_shared(model);
                    s.window.request_redraw();
                }
            }
            AppEvent::Paste(text) => {
                if let Some(s) = self.state.as_mut().filter(|s| !s.features.read_only) {
                    s.paste_event(text);
                    s.notify_api(&mut self.hub);
                    s.window.request_redraw();
                }
            }
            AppEvent::Platform(reply) => {
                if let Some(s) = self.state.as_mut() {
                    s.platform_reply(reply);
                    s.notify_api(&mut self.hub);
                    s.window.request_redraw();
                    if s.should_exit() {
                        event_loop.exit();
                    }
                }
            }
            AppEvent::AccessKit(event) => {
                if let Some(s) = self.state.as_mut()
                    && s.gui.on_accesskit(event.window_event)
                {
                    s.window.request_redraw();
                }
            }
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        // Menu items' actions run on the main thread inside AppKit; they reach the loop as
        // events, with whether a held key raised them.
        #[cfg(target_os = "macos")]
        {
            let proxy = self.proxy.clone();
            muda::MenuEvent::set_event_handler(Some(move |e: muda::MenuEvent| {
                let _ = proxy.send_event(AppEvent::Menu {
                    id: e.id.0,
                    repeat: native_menu::current_event_repeats(),
                });
            }));
            // Dock > Quit, log out and shut down ask the window first when it has unsaved
            // changes.
            let proxy = self.proxy.clone();
            crate::system_quit::install(move || {
                let _ = proxy.send_event(AppEvent::SystemQuit);
            });
        }
        let Some(platform) = self.platform.take() else {
            return;
        };
        let attrs = self
            .window_attributes
            .take()
            .unwrap_or_else(window_attributes);
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let session = self
            .session
            .take()
            .unwrap_or_else(|| Err("autosave already started".into()));
        let mut state = State::new(
            window,
            self.roots.clone(),
            self.recovery.clone(),
            session,
            platform,
            self.gpu.take(),
            self.proxy.clone(),
        );
        // The adapter before the window is first shown (it panics otherwise), then the window,
        // whatever happens next.
        state
            .gui
            .init_accesskit(event_loop, &state.window, self.proxy.clone());
        state.window.set_visible(true);
        // Autosaves of windows that ended without saving are offered back.
        if let (Some(dir), Some(session)) = (state.recovery.as_deref(), state.session.as_ref()) {
            let crashed = recovery::list_newest(dir, session, false, recovery::MAX_LISTED);
            if opens_recover_at_start(&crashed.entries) {
                state.recover = Some(RecoverList::new(false, crashed));
            }
        }
        if let Some(model) = self.model.take() {
            state.open_shared(model);
        }
        if let Some(path) = self.file.take() {
            let open = Command::Open {
                path: path.display().to_string(),
                // A fresh model: nothing to lose.
                discard: false,
            };
            match state.editor.execute(open) {
                Ok(_) => state.remember_file(),
                Err(e) => eprintln!("cannot open {}", e.message),
            }
        }
        state.window.request_redraw();
        // Requests that arrived before the window existed.
        state.process_api(&mut self.hub);
        if state.should_exit() {
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
        if matches!(event, WindowEvent::KeyboardInput { .. })
            && let Some(held) = s.platform.event_modifiers()
        {
            s.modifiers = held;
        }
        let egui_repaint = s.gui.on_event(&s.window, &event);
        if redraws_for_egui(&event, egui_repaint) {
            s.window.request_redraw();
        }
        if interrupts_ctrl_tap(&event) {
            s.ctrl_tap.interrupt();
        }
        if finishes_transition(&event) && s.editor.finish_transition() {
            s.window.request_redraw();
        }
        let changed = match event {
            WindowEvent::CloseRequested => {
                s.settle_and(Then::Quit);
                if s.should_exit() {
                    event_loop.exit();
                }
                false
            }
            WindowEvent::Resized(size) => {
                s.resize(size.width, size.height);
                true
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                s.touches.set_scale_factor(scale_factor);
                s.editor.set_scale_factor(scale_factor);
                false
            }
            WindowEvent::RedrawRequested => {
                if s.render(&self.hub) {
                    s.notify_api(&mut self.hub);
                }
                s.notify_frame(&mut self.hub);
                if s.should_exit() {
                    event_loop.exit();
                }
                false
            }
            WindowEvent::ModifiersChanged(m) => {
                let tapped = s.ctrl_tap.modifiers(s.modifiers, m.state());
                s.modifiers = m.state();
                let locked = s
                    .editor
                    .set_modifiers(m.state().shift_key(), m.state().alt_key());
                // Cmd or Ctrl held makes a Paint click pick up a material (and changes how
                // its target is highlighted).
                let resampled = s.editor.set_sample_modifier(command_held(
                    paths::Os::current(),
                    m.state().super_key(),
                    m.state().control_key(),
                ));
                // Tapping Ctrl switches tools that can copy (Move, Rotate) between moving and
                // copying, the Tape Measure between adding guides and measuring only, the
                // Polygon tool between inscribed and circumscribed, and the Scale tool scaling
                // about the center.
                let toggled = tapped && s.editor.toggle_copy();
                locked || toggled || resampled
            }
            WindowEvent::Focused(false) => {
                // Releases may never arrive: no click or double-click spans the focus change.
                s.touches.reset();
                s.touch_owned.clear();
                s.left_press = None;
                s.clicks = None;
                s.focused = false;
                s.editor.focus_lost()
            }
            WindowEvent::Focused(true) => {
                s.focused = true;
                // Another app may have changed the clipboard: Paste is enabled by what it
                // holds now.
                let had = s.editor.clip_ready();
                s.editor.refresh_clipboard();
                // So may another app have changed the library folder.
                s.library_changed();
                had != s.editor.clip_ready() || s.prefs.dock.tab == DockTab::Library
            }
            // The frames skipped while the window was hidden (see `retries_at_once`).
            WindowEvent::Occluded(false) => true,
            WindowEvent::CursorMoved { position, .. } => {
                let p = DVec2::new(position.x, position.y);
                let moved = s.cursor_moved(p);
                // A mouse drag in the viewer is a one-finger touch.
                s.touch(touch::Phase::Move, MOUSE_TOUCH, p) || moved
            }
            WindowEvent::Touch(t) if s.features.read_only => {
                let phase = match t.phase {
                    TouchPhase::Started => touch::Phase::Start,
                    TouchPhase::Moved => touch::Phase::Move,
                    TouchPhase::Ended => touch::Phase::End,
                    TouchPhase::Cancelled => touch::Phase::Cancel,
                };
                s.touch(phase, t.id, DVec2::new(t.location.x, t.location.y))
            }
            // The viewer has no keyboard but Escape: it leaves Measure, else closes a list.
            WindowEvent::KeyboardInput { event, .. } if s.features.read_only => {
                if escape_pressed(&event.logical_key, event.state) {
                    let measuring = s.editor.measurement().is_some();
                    s.editor.stop_measure();
                    let listed = s.viewer_panel.take().is_some();
                    measuring || listed
                } else {
                    false
                }
            }
            // The mouse's left button in the viewer is a finger: a click selects, a drag
            // orbits, a double click zooms. The other buttons and the wheel navigate as usual.
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } if s.features.read_only => {
                let phase = if state == ElementState::Pressed {
                    touch::Phase::Start
                } else {
                    touch::Phase::End
                };
                let at = s.cursor;
                s.touch(phase, MOUSE_TOUCH, at)
            }
            WindowEvent::CursorLeft { .. } => {
                // The mouse button's release may happen outside the window and never arrive.
                let at = s.cursor;
                s.touch(touch::Phase::Cancel, MOUSE_TOUCH, at);
                s.editor.cursor_left()
            }
            // While flying, clicks do nothing to the model; without a grabbed pointer the left
            // button held in the view looks around.
            WindowEvent::MouseInput { state, button, .. } if s.editor.flying() => {
                if button == MouseButton::Left {
                    s.look_held = state == ElementState::Pressed && s.pointer_in_view();
                }
                false
            }
            // Right-clicking a selection (a section plane has its own entries), or anywhere
            // while the model axes are moved (Reset Axes), opens the viewport menu.
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Right,
                ..
            } if s.pointer_in_view() && s.editor.opens_viewport_menu(s.features.read_only) => {
                let ppp = f64::from(s.gui.pixels_per_point());
                s.menu_face = s.editor.hovered();
                s.viewport_menu = Some(egui::pos2(
                    (s.cursor.x / ppp) as f32,
                    (s.cursor.y / ppp) as f32,
                ));
                true
            }
            // Presses go to the view only when they land in it; releases always do, so a drag
            // that ends over a panel still finishes.
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                if pressed && !s.pointer_in_view() {
                    false
                } else {
                    if pressed && button == MouseButton::Left {
                        s.editor.quick_press(is_quick(s.clicks, Instant::now()));
                    }
                    let handled = s.editor.button(map_button(button), pressed);
                    if pressed && button == MouseButton::Left {
                        s.left_press = Some(s.cursor);
                    } else if handled && button == MouseButton::Left {
                        let now = Instant::now();
                        let max = DRAG_LOGICAL_PX * s.window.scale_factor();
                        let click = is_click(s.left_press.take(), s.cursor, max);
                        if click {
                            let n = click_count(s.clicks, now, s.cursor, max);
                            s.clicks = Some((now, s.cursor, n));
                            match n {
                                2 => {
                                    s.editor.double_click();
                                }
                                3 => {
                                    s.editor.triple_click();
                                }
                                _ => {}
                            }
                        } else {
                            s.clicks = None;
                        }
                    }
                    handled
                }
            }
            // A held pointer may rest over a panel; while flying, scrolls still fly. Not while
            // a sheet is open, even if an agent turned fly on.
            WindowEvent::MouseWheel { .. } | WindowEvent::PinchGesture { .. }
                if !view_input_reaches_editor(
                    s.editor.flying(),
                    s.pointer_in_view(),
                    s.sheet.is_some(),
                ) =>
            {
                false
            }
            WindowEvent::KeyboardInput { ref event, .. }
                if let Some(key) = is_fly_move(
                    s.editor.flying(),
                    event.physical_key,
                    event.state == ElementState::Pressed,
                    s.fly_shortcut(event),
                    keys_reach_editor(s.gui.wants_keyboard(), s.sheet.is_some()),
                ) =>
            {
                s.editor.fly_key(key, event.state == ElementState::Pressed)
            }
            WindowEvent::KeyboardInput { ref event, .. }
                if !keys_reach_editor(s.gui.wants_keyboard(), s.sheet.is_some())
                    && !(event.state == ElementState::Pressed
                        && !event.repeat
                        && is_quit_chord(
                            shortcut_letter(&event.logical_key, event.physical_key),
                            s.command_held(),
                            s.modifiers.shift_key(),
                            s.menu_bar_key(),
                        )) =>
            {
                false
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
                let command_key = s.command_held();
                let letter = shortcut_letter(&event.logical_key, event.physical_key);
                if command_key {
                    let shift = s.modifiers.shift_key();
                    // With native menus, Cmd chords are the menu bar's; Ctrl ones still act here.
                    let cmd = s.menu_bar_key();
                    let bound = letter.and_then(|l| {
                        shortcuts::window_command(
                            l,
                            shift,
                            event.repeat,
                            cmd,
                            shortcuts::NATIVE_MENUS,
                        )
                    });
                    match bound {
                        // A browser tab cannot be quit; Paste comes with the page's paste event.
                        Some(Bound::Quit) if !s.features.quit => false,
                        Some(Bound::Paste) if s.features.paste_by_event => false,
                        Some(Bound::Quit) => {
                            s.settle_and(Then::Quit);
                            if s.should_exit() {
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
                } else if let Some(key) = window_key(&event.logical_key) {
                    // The table's Rename Component (F2), Previous and Next Scene (Page Up and
                    // Page Down): not native accelerators, so they act only here. Not with
                    // Shift, not held, not while flying.
                    let chord = shortcuts::Chord {
                        command: false,
                        shift: false,
                        key,
                    };
                    let action = (!s.modifiers.shift_key() && !event.repeat && !s.editor.flying())
                        .then(|| shortcuts::lookup(chord).and_then(|b| b.bound.action()))
                        .flatten();
                    match action {
                        Some(action) => {
                            s.apply(action);
                            true
                        }
                        None => false,
                    }
                } else {
                    editor_key(
                        &event.logical_key,
                        event.physical_key,
                        s.modifiers.shift_key(),
                    )
                    .is_some_and(|k| {
                        if event.repeat {
                            s.editor.key_repeat(k)
                        } else if s.editor.shortcut_for(k) == Some(Bound::MakeComponent) {
                            // Shift+G asks for the name in the window; tool_input and headless
                            // keep the automatic "Component N" through `Editor::key`.
                            s.apply(UiAction::MakeComponent);
                            true
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
    fn the_viewer_features_turn_everything_that_edits_off() {
        let v = Features::VIEWER;
        assert!(v.read_only);
        assert!(escape_pressed(
            &Key::Named(NamedKey::Escape),
            ElementState::Pressed
        ));
        assert!(!escape_pressed(
            &Key::Named(NamedKey::Escape),
            ElementState::Released
        ));
        assert!(!escape_pressed(
            &Key::Named(NamedKey::Enter),
            ElementState::Pressed
        ));
        const { assert!(!Features::ALL.read_only && !Features::WEB.read_only) };
        assert!(!v.library && !v.autosave && !v.print && !v.recent && !v.quit && !v.share);
        assert!(!v.update_check && Features::ALL.update_check);
        assert!(!v.file_paths && v.in_app_dialogs && v.paste_by_event);
    }

    #[test]
    fn the_viewer_lets_only_view_actions_through() {
        use crate::ui::UiAction as A;
        for action in [
            A::SelectNone,
            A::CloseSheet,
            A::ZoomExtents,
            A::StandardView(session::StandardView::Top),
        ] {
            assert!(viewer_allows(&action), "{action:?}");
        }
        for action in [
            A::Undo,
            A::Redo,
            A::Cut,
            A::Paste,
            A::EraseSelection,
            A::MakeGroup,
            A::MakeComponent,
            A::Explode,
            A::MakeUnique,
            A::Flip(session::FlipAxis::Red, session::FlipFrame::Axes),
            A::ResetAxes,
            A::NewModel,
            A::OpenFile,
            A::SaveFile,
            A::ImportFile,
            A::Tool(tools::ToolKind::Line),
            A::SelectAll,
            A::ToggleFly,
            A::Quit,
            A::OpenOnPhone,
            A::PurgeUnused,
            A::DeleteGuides,
            A::AddTag,
            A::AddScene,
            A::AddStyle(None),
            A::DeleteStyle(kernel::StyleId(1)),
            A::RenameStyle(kernel::StyleId(1), String::new()),
            A::ChooseStyle(None),
            A::ChangeStyle(kernel::StyleId(1), session::StylePatch::default()),
            A::ToggleEdgeEffect(crate::ui::EdgeEffect::Profiles),
        ] {
            assert!(!viewer_allows(&action), "{action:?}");
        }
    }

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
    use crate::flows::save_name;
    use winit::keyboard::SmolStr;

    #[test]
    fn the_window_cannot_shrink_below_960_by_640() {
        use winit::dpi::Size;
        let attrs = window_attributes();
        assert_eq!(
            attrs.min_inner_size,
            Some(Size::Logical(LogicalSize::new(960.0, 640.0)))
        );
        assert_eq!(
            attrs.inner_size,
            Some(Size::Logical(LogicalSize::new(1280.0, 800.0)))
        );
    }

    #[test]
    fn the_minimum_window_grows_with_the_ui_scale() {
        assert_eq!(
            min_window_size(UiScale::P100),
            LogicalSize::new(960.0, 640.0)
        );
        assert_eq!(min_window_size(UiScale::P100), MIN_WINDOW);
        assert_eq!(
            min_window_size(UiScale::P125),
            LogicalSize::new(1200.0, 800.0)
        );
        assert_eq!(
            min_window_size(UiScale::P90),
            LogicalSize::new(864.0, 576.0)
        );
    }

    #[test]
    fn a_window_smaller_than_the_new_minimum_grows_to_it() {
        let min = min_window_size(UiScale::P125);
        assert_eq!(
            grown_to_min(LogicalSize::new(1000.0, 900.0), min),
            Some(LogicalSize::new(1200.0, 900.0))
        );
        assert_eq!(grown_to_min(LogicalSize::new(960.0, 600.0), min), Some(min));
        assert_eq!(grown_to_min(LogicalSize::new(1280.0, 800.0), min), None);
    }

    #[test]
    fn the_minimum_window_fits_a_small_monitor() {
        let min = min_window_size(UiScale::P125);
        let small = LogicalSize::new(1280.0, 800.0);
        let fitted = fit_to_monitor(min, Some(small));
        assert_eq!(
            fitted,
            LogicalSize::new(1200.0, 800.0 - MONITOR_MARGIN.height)
        );
        assert!(fitted.height < small.height && fitted.width <= small.width);
        // The grow asked for follows the fitted minimum, so it fits too.
        let grown = grown_to_min(LogicalSize::new(1000.0, 600.0), fitted).unwrap();
        assert!(grown.width <= small.width && grown.height < small.height);
        // A big monitor and no monitor leave the minimum alone.
        assert_eq!(
            fit_to_monitor(min, Some(LogicalSize::new(2560.0, 1440.0))),
            min
        );
        assert_eq!(fit_to_monitor(min, None), min);
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
    fn a_request_that_chooses_a_style_still_shows() {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        let run = |e: &mut Editor, json: serde_json::Value| {
            let out = e.execute(serde_json::from_value(json).unwrap()).unwrap();
            serde_json::to_value(out).unwrap()
        };
        let out = run(
            &mut e,
            serde_json::json!({"type": "add_style", "name": "Dark"}),
        );
        let y = out["changes"]["styles"][0].clone();
        let before = shown(&e);
        run(&mut e, serde_json::json!({"type": "set_style", "style": y}));
        assert!(e.doc().style().is_some());
        assert_ne!(shown(&e), before);
    }

    #[test]
    fn a_request_that_only_goes_to_a_scene_still_shows() {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        let mut run = |e: &mut Editor, command: session::Command| {
            e.execute(command).unwrap();
        };
        let add = |e: &mut Editor, run: &mut dyn FnMut(&mut Editor, session::Command)| {
            run(
                e,
                session::Command::AddScene {
                    name: None,
                    camera: false,
                    tags: false,
                    positions: None,
                    joints: None,
                },
            );
            e.doc().selected_scene().unwrap()
        };
        let first = add(&mut e, &mut run);
        let second = add(&mut e, &mut run);
        assert_ne!(first, second);
        let before = shown(&e);
        run(&mut e, session::Command::GoToScene { scene: first });
        assert_eq!(e.doc().selected_scene(), Some(first));
        assert_ne!(shown(&e), before);
    }

    #[test]
    fn a_request_that_changes_the_sun_still_shows() {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        let run = |e: &mut Editor, json: serde_json::Value| {
            e.execute(serde_json::from_value(json).unwrap()).unwrap();
        };
        let before = shown(&e);
        run(
            &mut e,
            serde_json::json!({"type": "set_shadows", "on": true}),
        );
        let shadows = shown(&e);
        assert_ne!(shadows, before, "the shadow settings");
        run(
            &mut e,
            serde_json::json!({"type": "set_shadows", "time": "09:00"}),
        );
        assert_ne!(shown(&e), shadows, "the time");
        let moved = shown(&e);
        run(
            &mut e,
            serde_json::json!({"type": "set_location", "latitude": 10.0}),
        );
        assert_ne!(shown(&e), moved, "the location");
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
    fn presses_scrolls_and_gestures_finish_a_scene_transition_but_not_a_drifting_cursor() {
        let id = winit::event::DeviceId::dummy();
        let mouse = |state| WindowEvent::MouseInput {
            device_id: id,
            state,
            button: MouseButton::Left,
        };
        assert!(finishes_transition(&mouse(ElementState::Pressed)));
        // The release of the click that started it must not end it.
        assert!(!finishes_transition(&mouse(ElementState::Released)));
        assert!(finishes_transition(&WindowEvent::MouseWheel {
            device_id: id,
            delta: MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 4.0)),
            phase: TouchPhase::Moved,
        }));
        assert!(finishes_transition(&WindowEvent::PinchGesture {
            device_id: id,
            delta: 0.01,
            phase: TouchPhase::Moved,
        }));
        assert!(!finishes_transition(&WindowEvent::CursorMoved {
            device_id: id,
            position: winit::dpi::PhysicalPosition::new(1.0, 2.0),
        }));
        assert!(!finishes_transition(&WindowEvent::Focused(true)));
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
    fn in_a_browser_ctrl_and_cmd_both_run_chords() {
        use crate::paths::Os;
        assert!(command_held(Os::Web, false, true), "Ctrl");
        assert!(command_held(Os::Web, true, false), "Cmd");
        assert!(!command_held(Os::Web, false, false));
        assert!(!menu_bar_held(Os::Web, true), "no native menu bar");
        // No native menus: the key path takes the chord whichever key made it.
        assert_eq!(
            shortcuts::window_command('z', false, false, false, false),
            Some(Bound::Undo)
        );
        assert!(
            fly_shortcut(Os::Web, Some('s'), false, true, false),
            "Cmd+S"
        );
        assert!(
            fly_shortcut(Os::Web, Some('s'), false, false, true),
            "Ctrl+S"
        );
        assert!(!fly_shortcut(Os::Web, Some('w'), false, false, true));
    }

    #[test]
    fn a_surface_larger_than_the_gpu_allows_is_fitted_keeping_its_shape() {
        assert_eq!(fit_surface(800, 600, 2048), (800, 600));
        assert_eq!(fit_surface(4096, 2048, 2048), (2048, 1024));
        assert_eq!(fit_surface(1000, 5000, 2500), (500, 2500));
        assert_eq!(fit_surface(10000, 1, 100), (100, 1));
        assert_eq!(fit_surface(10, 10, 0), (10, 10));
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
    fn a_still_window_asks_for_no_repaint() {
        let now = Instant::now();
        let mut timing = Timing::default();
        assert_eq!(
            with_feedback(None, &timing, now, true),
            None,
            "nothing pending"
        );
        assert_eq!(wake_decision(now, None), Wake::Wait);
        // A move asks for the frame at the rest, and that frame for nothing more.
        timing.moved(now);
        let rest = now + crate::feedback::LABEL_REST;
        assert_eq!(with_feedback(None, &timing, now, true), Some(rest));
        assert_eq!(with_feedback(None, &timing, rest, true), None);
        // With no chip to show at the rest, the move asks for nothing.
        assert_eq!(with_feedback(None, &timing, now, false), None);
        // An earlier deadline already pending stays.
        let sooner = now + Duration::from_millis(10);
        assert_eq!(
            with_feedback(Some(sooner), &timing, now, true),
            Some(sooner)
        );
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
    fn a_press_is_quick_within_the_double_click_time_of_the_last_click() {
        let now = Instant::now();
        let at = DVec2::new(100.0, 100.0);
        assert!(!is_quick(None, now));
        assert!(is_quick(
            Some((now, at, 1)),
            now + Duration::from_millis(300)
        ));
        assert!(is_quick(Some((now, at, 2)), now + DOUBLE_CLICK));
        assert!(!is_quick(
            Some((now, at, 1)),
            now + Duration::from_millis(800)
        ));
    }

    #[test]
    fn clicks_count_up_to_three_then_start_over() {
        let now = Instant::now();
        let at = DVec2::new(100.0, 100.0);
        let soon = now + Duration::from_millis(300);
        let near = at + DVec2::new(2.0, 2.0);
        assert_eq!(click_count(None, now, at, DRAG_LOGICAL_PX), 1);
        for (n, next) in [(1, 2), (2, 3), (3, 1)] {
            assert_eq!(
                click_count(Some((now, at, n)), soon, near, DRAG_LOGICAL_PX),
                next
            );
        }
        let far = at + DVec2::new(10.0, 0.0);
        assert_eq!(
            click_count(Some((now, at, 1)), soon, far, DRAG_LOGICAL_PX),
            1
        );
        let late = now + Duration::from_millis(800);
        assert_eq!(
            click_count(Some((now, at, 1)), late, at, DRAG_LOGICAL_PX),
            1
        );
    }

    #[test]
    fn a_release_far_from_its_press_ends_a_drag_not_a_click() {
        let at = DVec2::new(100.0, 100.0);
        assert!(!is_click(None, at, DRAG_LOGICAL_PX));
        assert!(is_click(
            Some(at),
            at + DVec2::new(2.0, 2.0),
            DRAG_LOGICAL_PX
        ));
        assert!(!is_click(
            Some(at),
            at + DVec2::new(40.0, 0.0),
            DRAG_LOGICAL_PX
        ));
        // The tools' drag threshold is the one radius: exactly that far is a drag.
        assert!(is_click(
            Some(at),
            at + DVec2::new(3.9, 0.0),
            DRAG_LOGICAL_PX
        ));
        assert!(!is_click(
            Some(at),
            at + DVec2::new(4.0, 0.0),
            DRAG_LOGICAL_PX
        ));
        assert_eq!(
            click_count(
                Some((Instant::now(), at, 1)),
                Instant::now(),
                at + DVec2::new(0.0, 4.0),
                DRAG_LOGICAL_PX
            ),
            1
        );
    }

    #[test]
    fn keys_wait_while_a_sheet_is_open() {
        assert!(keys_reach_editor(false, false));
        assert!(!keys_reach_editor(true, false));
        assert!(!keys_reach_editor(false, true));
        assert!(!keys_reach_editor(true, true));
    }

    #[test]
    fn only_the_sheets_own_actions_and_quit_pass_a_sheet() {
        for dropped in [
            UiAction::OpenFile,
            UiAction::OpenRecent(PathBuf::from("/tmp/a.skr")),
            UiAction::Tool(tools::ToolKind::Line),
            UiAction::MakeComponent,
            UiAction::NewModel,
            UiAction::RenameComponent,
            UiAction::ShowFieldOfView,
            UiAction::Show3dText,
            UiAction::ShowSettings(true),
            UiAction::ExportPdf,
            UiAction::Print,
            UiAction::LibraryUse("/lib/a.skr".into()),
            UiAction::LibraryDelete("/lib/a.skr".into()),
            UiAction::SaveToLibrary(LibrarySave::Model),
            UiAction::ChooseLibraryFolder,
        ] {
            assert!(!action_allowed_with_sheet(&dropped), "{dropped:?}");
        }
        for passed in [
            UiAction::CloseSheet,
            UiAction::ConfirmImport(crate::editor::ImportOptions::default()),
            UiAction::ConfirmName("Component 1".into()),
            UiAction::ConfirmFieldOfView(35.0),
            UiAction::ConfirmPrint(PrintOptions::default()),
            UiAction::Confirm3dText(default_text_3d()),
            UiAction::ConfirmSaveToLibrary("Chair".into()),
            UiAction::ConfirmOverwriteLibrary,
            UiAction::ConfirmLibraryDelete,
            UiAction::Quit,
        ] {
            assert!(action_allowed_with_sheet(&passed), "{passed:?}");
        }
    }

    #[test]
    fn view_navigation_waits_while_a_sheet_is_open() {
        assert!(view_input_reaches_editor(false, true, false));
        assert!(view_input_reaches_editor(true, false, false));
        assert!(!view_input_reaches_editor(false, false, false));
        // An agent may turn fly on under a sheet: scroll and pinch still wait.
        assert!(!view_input_reaches_editor(true, false, true));
        assert!(!view_input_reaches_editor(true, true, true));
        assert!(!view_input_reaches_editor(false, true, true));
    }

    /// An editor with a 2 x 2 rectangle, all of it selected.
    fn editor_with_selection() -> Editor {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        e.execute(session::Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        })
        .unwrap();
        e.select_all();
        e
    }

    #[test]
    fn opening_a_sheet_ends_fly_mode_and_lets_held_keys_go() {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        assert!(e.set_flying(true));
        assert!(e.fly_key(FlyKey::W, true));
        end_fly_for_sheet(&mut e);
        assert!(!e.flying());
        // The W release went to the sheet. Once it closes, flying again does not drift.
        assert!(e.set_flying(true));
        let eye = e.doc().camera.eye;
        let t = Instant::now();
        e.fly_tick(t);
        e.fly_tick(t + Duration::from_millis(100));
        assert_eq!(e.doc().camera.eye, eye, "no key is still held");
    }

    #[test]
    fn the_pointer_is_not_held_while_a_sheet_is_open() {
        assert!(holds_pointer(true, true, false));
        assert!(!holds_pointer(true, true, true));
        assert!(!holds_pointer(true, false, false));
        assert!(!holds_pointer(false, true, false));
    }

    #[test]
    fn a_sheet_does_not_outlive_its_model() {
        let mut e = editor_with_selection();
        e.make_group();
        let target = e.rename_target().expect("one group selected");
        let sheet = Sheet::Rename { target };
        let origin = SheetOrigin::of(&e);
        assert_eq!(sheet_applies(&sheet, &origin, &e), Ok(()));
        // An agent starts a new model, then draws and groups again: d1 names a new group.
        e.execute(session::Command::New { discard: true }).unwrap();
        assert_ne!(
            e.documents(),
            origin.document,
            "the window closes the sheet"
        );
        assert!(sheet_applies(&sheet, &origin, &e).is_err());
        // An agent opens a file.
        let path =
            std::env::temp_dir().join(format!("sketchup-alt-app-{}-sheet.skr", std::process::id()));
        e.execute(session::Command::Save {
            path: Some(path.display().to_string()),
            overwrite: true,
        })
        .unwrap();
        let origin = SheetOrigin::of(&e);
        e.execute(session::Command::Open {
            path: path.display().to_string(),
            discard: true,
        })
        .unwrap();
        assert!(sheet_applies(&sheet, &origin, &e).is_err());
        let _ = std::fs::remove_file(&path);
        // The rename target must still be the one selected.
        let mut e = editor_with_selection();
        e.make_group();
        let target = e.rename_target().unwrap();
        let origin = SheetOrigin::of(&e);
        e.select_none();
        assert!(sheet_applies(&Sheet::Rename { target }, &origin, &e).is_err());
    }

    #[test]
    fn deleting_a_used_tag_asks_and_the_sheet_needs_the_tag() {
        let mut e = editor_with_selection();
        e.add_tag();
        e.add_tag();
        let ids: Vec<TagId> = e.doc().scene().tags().map(|(id, _)| id).collect();
        let (walls, other) = (ids[0], ids[1]);
        assert_eq!(delete_tag_sheet(e.doc(), walls), None, "nothing on it");
        e.set_selection_tag(Some(walls));
        let sheet = delete_tag_sheet(e.doc(), walls).expect("five entities on it");
        assert_eq!(
            sheet,
            Sheet::DeleteTag {
                tag: walls,
                name: "Tag".into(),
                count: 5,
                others: vec![(other, "Tag 2".into())],
            }
        );
        let rows = tag_rows(e.doc());
        assert_eq!(
            rows.iter().map(|r| (r.id, r.count)).collect::<Vec<_>>(),
            [(walls, 5), (other, 0)]
        );
        let origin = SheetOrigin::of(&e);
        assert_eq!(sheet_applies(&sheet, &origin, &e), Ok(()));
        // An agent deletes the tag meanwhile.
        e.delete_tag(walls, None);
        assert!(sheet_applies(&sheet, &origin, &e).is_err());
    }

    #[test]
    fn dropped_actions_do_not_count_and_thumbnail_requests_pass_a_sheet() {
        let wanted = UiAction::ThumbnailsWanted(vec!["/lib/a.skr".into()]);
        assert!(
            action_applies(true, &wanted),
            "thumbnails render under a sheet"
        );
        assert!(action_applies(
            false,
            &UiAction::LibraryUse("/lib/a.skr".into())
        ));
        assert!(!action_applies(
            true,
            &UiAction::LibraryUse("/lib/a.skr".into())
        ));
        assert!(action_applies(true, &UiAction::CloseSheet));
    }

    #[test]
    fn library_sheets_apply_only_while_what_they_save_exists() {
        let mut e = editor_with_selection();
        let origin = SheetOrigin::of(&e);
        let gone = Sheet::SaveToLibrary {
            save: LibrarySave::Material(MaterialId(99)),
            suggested: "x".into(),
        };
        assert!(sheet_applies(&gone, &origin, &e).is_err());
        let gone = Sheet::OverwriteLibrary {
            save: LibrarySave::Component(InstanceId(999)),
            name: "x".into(),
            file: "x.skr".into(),
        };
        assert!(sheet_applies(&gone, &origin, &e).is_err());
        let model = Sheet::SaveToLibrary {
            save: LibrarySave::Model,
            suggested: "x".into(),
        };
        assert!(sheet_applies(&model, &origin, &e).is_ok());
        // Deleting a library file does not depend on the model: it survives a new one.
        let delete = Sheet::DeleteLibraryItem {
            path: "/lib/a.skr".into(),
            name: "a".into(),
        };
        e.new_model();
        assert!(sheet_applies(&delete, &origin, &e).is_ok());
        assert!(sheet_applies(&model, &origin, &e).is_err());
    }

    #[test]
    fn the_tag_rows_follow_an_agents_add_and_undo() {
        let mut e = editor_with_selection();
        let mut cache = None;
        let names = |c: &Option<((u64, u64), Vec<TagRow>)>| -> Vec<String> {
            c.as_ref()
                .unwrap()
                .1
                .iter()
                .map(|r| r.name.clone())
                .collect()
        };
        refresh_tag_rows(&mut cache, &e);
        assert!(names(&cache).is_empty());
        e.execute(session::Command::AddTag {
            name: Some("Walls".into()),
            color: None,
        })
        .unwrap();
        refresh_tag_rows(&mut cache, &e);
        assert_eq!(names(&cache), ["Walls"]);
        e.execute(session::Command::Undo {
            count: None,
            expect: None,
        })
        .unwrap();
        refresh_tag_rows(&mut cache, &e);
        assert!(names(&cache).is_empty(), "the undo took it away");
        e.execute(session::Command::Redo { count: None }).unwrap();
        refresh_tag_rows(&mut cache, &e);
        assert_eq!(names(&cache), ["Walls"]);
    }

    #[test]
    fn make_component_needs_the_selection_it_was_opened_with() {
        let mut e = editor_with_selection();
        let sheet = Sheet::NameComponent {
            suggested: "Component 1".into(),
        };
        let origin = SheetOrigin::of(&e);
        assert_eq!(sheet_applies(&sheet, &origin, &e), Ok(()));
        // An agent selects one edge only.
        let edge = *e.doc().selection().first().unwrap();
        e.execute(session::Command::Select {
            ids: vec![edge],
            mode: session::SelectMode::Replace,
            all: false,
        })
        .unwrap();
        assert!(sheet_applies(&sheet, &origin, &e).is_err());
    }

    #[test]
    fn the_field_of_view_sheet_needs_perspective_at_confirm() {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        let sheet = Sheet::FieldOfView { degrees: 35.0 };
        let origin = SheetOrigin::of(&e);
        assert_eq!(sheet_applies(&sheet, &origin, &e), Ok(()));
        e.set_projection(session::Projection::Parallel);
        assert!(sheet_applies(&sheet, &origin, &e).is_err());
    }

    /// Clicks `e`'s tool at the model point `p` (seen by the document camera).
    fn click_at(e: &mut Editor, p: glam::DVec3) {
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        e.button(Button::Left, true);
        e.button(Button::Left, false);
    }

    #[test]
    fn the_note_sheet_answers_the_text_tool_and_double_clicks_on_notes() {
        use glam::DVec3;
        let mut e = editor_with_selection();
        e.select_tool(tools::ToolKind::Text);
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        assert_eq!(
            e.take_note_request(),
            None,
            "where the text goes comes first"
        );
        click_at(&mut e, DVec3::new(3.0, 3.0, 0.0));
        let request = e.take_note_request().expect("the text is asked for");
        assert_eq!(e.take_note_request(), None, "asked once");
        let sheet = note_sheet(&e, request).unwrap();
        let Sheet::NoteText {
            ref text,
            ref automatic,
            ..
        } = sheet
        else {
            panic!("{sheet:?}");
        };
        assert_eq!(text, "");
        assert_eq!(automatic.as_deref(), Some("(2 m, 2 m, 0 m)"));
        let origin = SheetOrigin::of(&e);
        assert_eq!(sheet_applies(&sheet, &origin, &e), Ok(()));
        assert!(e.finish_note(request, "Corner".into()));
        assert_eq!(e.doc().undo_label(), Some("add_note"));
        assert!(
            sheet_applies(&sheet, &origin, &e).is_err(),
            "the note is placed"
        );
        let (note, _) = e.doc().model().notes().next().unwrap();
        // A double-click on the note's leader with Select asks for its new text.
        e.select_tool(tools::ToolKind::Select);
        let on_leader = DVec3::new(2.5, 2.5, 0.0);
        click_at(&mut e, on_leader);
        click_at(&mut e, on_leader);
        e.double_click();
        let request = e.take_note_request().expect("a double-click on a note");
        assert_eq!(request, NoteRequest::Edit(EntityId::Note(note)));
        let sheet = note_sheet(&e, request).unwrap();
        assert!(
            matches!(&sheet, Sheet::NoteText { text, automatic: Some(_), .. } if text == "Corner"),
            "{sheet:?}"
        );
        // Blank: the automatic text again.
        assert!(e.finish_note(request, " ".into()));
        assert_eq!(e.doc().model().note(note).unwrap().text(), None);
        assert_eq!(e.doc().undo_label(), Some("change_note"));
        // Closing the sheet drops a note being placed.
        e.select_tool(tools::ToolKind::Text);
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        click_at(&mut e, DVec3::new(-1.0, -1.0, 0.0));
        assert!(e.awaits_note_text());
        e.cancel_note();
        assert!(!e.awaits_note_text());
        // A note that is gone has no sheet, and its sheet no longer applies.
        e.execute(session::Command::Erase {
            ids: vec![EntityId::Note(note)],
        })
        .unwrap();
        assert_eq!(note_sheet(&e, request), None);
        assert!(sheet_applies(&sheet, &origin, &e).is_err());
    }

    #[test]
    fn a_note_request_waits_while_another_sheet_is_open() {
        use glam::DVec3;
        let mut e = editor_with_selection();
        e.select_tool(tools::ToolKind::Text);
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        click_at(&mut e, DVec3::new(3.0, 3.0, 0.0));
        // The user's Field of View sheet stays; the request waits.
        let theirs = Sheet::FieldOfView { degrees: 35.0 };
        assert_eq!(next_note_sheet(Some(&theirs), &mut e), None);
        let sheet = next_note_sheet(None, &mut e).expect("asked once the sheet closed");
        assert!(matches!(sheet, Sheet::NoteText { .. }), "{sheet:?}");
        assert_eq!(next_note_sheet(None, &mut e), None, "asked once");
        // The next note the Text tool asks about replaces a note sheet of its own.
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        click_at(&mut e, DVec3::new(-1.0, -1.0, 0.0));
        let next = next_note_sheet(Some(&sheet), &mut e).expect("the new note's sheet");
        assert_ne!(next, sheet);
        // A note dropped while it waited has no sheet.
        click_at(&mut e, DVec3::new(2.0, 0.0, 0.0));
        click_at(&mut e, DVec3::new(3.0, -1.0, 0.0));
        e.cancel_note();
        assert_eq!(next_note_sheet(None, &mut e), None);
    }

    #[test]
    fn the_quit_chord_passes_a_sheet() {
        // Ctrl+Q is the key path's on every system.
        assert!(is_quit_chord(Some('q'), true, false, false));
        assert!(!is_quit_chord(Some('q'), false, false, false), "plain q");
        assert!(!is_quit_chord(Some('s'), true, false, false));
        assert!(!is_quit_chord(Some('q'), true, true, false), "Shift+Ctrl+Q");
        // Cmd+Q is the native menu's on macOS (Quit stays enabled while a sheet is open).
        assert_eq!(
            is_quit_chord(Some('q'), true, false, true),
            !shortcuts::NATIVE_MENUS
        );
    }

    #[test]
    fn titles_name_the_file_and_mark_unsaved_changes() {
        let house = Some(Path::new("/m/house.skr"));
        assert_eq!(
            window_title(None, None, None, false, false),
            "Untitled.skr - sketchup-alt"
        );
        assert_eq!(
            window_title(house, None, None, true, false),
            "house.skr* - sketchup-alt"
        );
        // macOS: the close button's dot shows unsaved changes.
        assert_eq!(window_title(house, None, None, true, true), "house.skr");
        assert_eq!(window_title(None, None, None, true, true), "Untitled.skr");
    }

    #[test]
    fn a_model_opened_from_sketchup_is_titled_imported_until_saved() {
        let desk = Some(Path::new("/m/desk.skp"));
        assert_eq!(
            window_title(None, desk, None, false, true),
            "desk.skp (imported)"
        );
        assert_eq!(
            window_title(None, desk, None, true, false),
            "desk.skp (imported)* - sketchup-alt"
        );
        // Saving gives it a path of its own (and the document forgets the SketchUp file).
        let saved = Some(Path::new("/m/desk.skr"));
        assert_eq!(window_title(saved, None, None, false, true), "desk.skr");
        // The save dialog suggests a .skr beside it, never the .skp.
        assert_eq!(save_name(None, desk, None), "desk.skr");
        assert_eq!(save_name(saved, None, None), "desk.skr");
        assert_eq!(save_name(None, None, None), "Untitled.skr");
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
    fn f2_and_the_page_keys_are_the_window_keys() {
        use crate::shortcuts::{Bound, Chord, Key as ShortcutKey, lookup};
        for (named, bound) in [
            (NamedKey::F2, Bound::RenameComponent),
            (NamedKey::PageUp, Bound::PreviousScene),
            (NamedKey::PageDown, Bound::NextScene),
        ] {
            let key = window_key(&Key::Named(named)).expect("a window key");
            let chord = Chord {
                command: false,
                shift: false,
                key,
            };
            assert_eq!(lookup(chord).map(|s| s.bound), Some(bound));
        }
        assert_eq!(window_key(&Key::Named(NamedKey::Home)), None);
        assert_eq!(window_key(&Key::Character(SmolStr::new("l"))), None);
        // They are plain keys in the table: the key path acts on them without Cmd or Shift.
        for s in crate::shortcuts::TABLE {
            if matches!(
                s.chord.key,
                ShortcutKey::F2 | ShortcutKey::PageUp | ShortcutKey::PageDown
            ) {
                assert!(!s.chord.command && !s.chord.shift, "{:?}", s.bound);
            }
        }
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
            'a' => KeyCode::KeyA,
            'd' => KeyCode::KeyD,
            'x' => KeyCode::KeyX,
            'c' => KeyCode::KeyC,
            'v' => KeyCode::KeyV,
            ',' => KeyCode::Comma,
            other => panic!("add {other} to shortcut_letter's fallbacks and here"),
        };
        let arabic = Key::Character(SmolStr::new("ب"));
        for s in TABLE.iter().filter(|s| s.chord.command) {
            let c = match s.chord.key {
                ShortcutKey::Letter(c) => c,
                ShortcutKey::Comma => ',',
                _ => continue,
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
    fn super_is_not_command_off_macos() {
        use paths::Os;
        for os in [Os::Linux, Os::Other] {
            assert!(command_held(os, false, true), "Ctrl");
            assert!(!command_held(os, true, false), "Super alone");
            assert!(command_held(os, true, true));
            assert!(!command_held(os, false, false));
            assert!(!menu_bar_held(os, true));
        }
        assert!(command_held(Os::MacOs, true, false), "Cmd");
        assert!(command_held(Os::MacOs, false, true), "Ctrl still counts");
        assert!(!command_held(Os::MacOs, false, false));
        assert!(menu_bar_held(Os::MacOs, true));
        assert!(!menu_bar_held(Os::MacOs, false));
    }

    #[test]
    fn fly_shortcuts_per_os() {
        use paths::Os;
        let ctrl = |os, c| fly_shortcut(os, c, false, false, true);
        for os in [Os::Linux, Os::Other] {
            assert!(ctrl(os, Some('s')), "Ctrl+S saves");
            assert!(ctrl(os, Some('q')), "Ctrl+Q quits");
            assert!(!ctrl(os, Some('w')), "Ctrl+W steers");
            assert!(!ctrl(os, None), "Space, the Ctrl press itself");
            assert!(!fly_shortcut(os, Some('s'), false, false, false), "plain S");
            assert!(!fly_shortcut(os, Some('s'), false, true, false), "Super+S");
        }
        assert!(
            fly_shortcut(Os::MacOs, Some('s'), false, true, false),
            "Cmd+S"
        );
        assert!(!ctrl(Os::MacOs, Some('s')), "Ctrl+S flies on macOS");
        assert!(!ctrl(Os::MacOs, Some('w')));
    }

    #[test]
    fn fly_shortcuts_follow_the_layout() {
        use paths::Os;
        let ch = |s: &str| Key::Character(s.into());
        let letter = |logical: &Key, code| shortcut_letter(logical, PhysicalKey::Code(code));
        // AZERTY: the key in the US W place types "z": Ctrl+it is Undo, not steering.
        let z = letter(&ch("z"), KeyCode::KeyW);
        assert!(fly_shortcut(Os::Linux, z, false, false, true));
        // The key in the US A place types "q": Ctrl+it is Quit.
        let q = letter(&ch("q"), KeyCode::KeyA);
        assert!(fly_shortcut(Os::Linux, q, false, false, true));
        // The key in the US Z place types "w": Ctrl+it steers.
        let w = letter(&ch("w"), KeyCode::KeyZ);
        assert!(!fly_shortcut(Os::Linux, w, false, false, true));
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
        assert_eq!(is_fly_move(true, s, true, false, true), Some(FlyKey::S));
        assert_eq!(is_fly_move(false, s, true, false, true), None, "not flying");
        assert_eq!(is_fly_move(true, s, true, true, true), None, "Cmd+S saves");
        assert_eq!(
            is_fly_move(true, s, false, true, true),
            Some(FlyKey::S),
            "a release always stops"
        );
        // A text field has focus, or a sheet is open: the key is theirs, but a key held since
        // before is still let go.
        assert_eq!(is_fly_move(true, s, true, false, false), None, "typed");
        assert_eq!(
            is_fly_move(true, s, false, false, false),
            Some(FlyKey::S),
            "released while typing"
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

    /// A fresh recovery folder for one test.
    fn recovery_folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sketchup-alt-app-recovery-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// The model files in `dir` (autosaves and kept discards).
    fn models_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".skr"))
            .collect();
        names.sort();
        names
    }

    fn edited_editor() -> Editor {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        e.execute(session::Command::AddEdge {
            start: [0.0, 0.0, 0.0],
            end: [1.0, 0.0, 0.0],
        })
        .unwrap();
        e
    }

    #[test]
    fn the_autosave_deadline_waits_for_changes_the_interval_and_the_tools() {
        let t = Instant::now();
        let five = Duration::from_secs(300);
        assert_eq!(
            autosave_deadline(t, five, 3, None, true, false),
            Some(t + five)
        );
        assert_eq!(
            autosave_deadline(t, five, 3, Some(2), true, false),
            Some(t + five)
        );
        assert_eq!(
            autosave_deadline(t, five, 3, Some(3), true, false),
            None,
            "saved"
        );
        assert_eq!(
            autosave_deadline(t, five, 3, None, false, false),
            None,
            "unmodified"
        );
        assert_eq!(
            autosave_deadline(t, five, 3, None, true, true),
            Some(t + 2 * five),
            "held by a tool operation for one interval more"
        );
        // A busy tool holds a due autosave back for one interval at most.
        assert!(!held_by_operation(false, t + five, t, five));
        assert!(held_by_operation(true, t + five, t, five));
        assert!(held_by_operation(
            true,
            t + 2 * five - Duration::from_secs(1),
            t,
            five
        ));
        assert!(!held_by_operation(true, t + 2 * five, t, five));
        assert_eq!(
            autosave_deadline(t, Duration::ZERO, 3, None, true, false),
            None,
            "off"
        );
    }

    #[test]
    fn only_a_settled_exit_or_a_saved_model_removes_the_autosave() {
        let at = |settled, modified, saved| at_exit(settled, modified, 7, saved, 5);
        assert_eq!(
            at(true, true, None),
            AtExit::Clear,
            "prompt answered or agent quit"
        );
        assert_eq!(at(false, false, None), AtExit::Clear, "nothing unsaved");
        assert_eq!(at(true, false, Some(7)), AtExit::Clear);
        // The system ended the app: the newest changes are written first.
        assert_eq!(at(false, true, None), AtExit::Write);
        assert_eq!(at(false, true, Some(6)), AtExit::Write);
        assert_eq!(at(false, true, Some(7)), AtExit::Keep, "already autosaved");
        assert_eq!(
            at_exit(false, true, 7, Some(6), 0),
            AtExit::Keep,
            "autosave off"
        );
    }

    #[test]
    fn files_the_system_opens_before_the_window_wait_for_it() {
        let mut at_start = Some(PathBuf::from("/m/argument.skr"));
        let opened = route_opened_files(
            vec![PathBuf::from("/m/a b.skr"), PathBuf::from("/m/second.skr")],
            false,
            &mut at_start,
        );
        assert_eq!(opened, None);
        assert_eq!(
            at_start,
            Some(PathBuf::from("/m/a b.skr")),
            "the first, kept"
        );
        assert_eq!(route_opened_files(Vec::new(), false, &mut at_start), None);
        assert_eq!(
            at_start,
            Some(PathBuf::from("/m/a b.skr")),
            "nothing replaces it"
        );
    }

    #[test]
    fn files_the_system_opens_with_the_window_open_now() {
        let mut at_start = None;
        assert_eq!(
            route_opened_files(
                vec![PathBuf::from("/m/a.skr"), PathBuf::from("/m/b.skr")],
                true,
                &mut at_start
            ),
            Some(PathBuf::from("/m/a.skr"))
        );
        assert_eq!(at_start, None);
        assert_eq!(route_opened_files(Vec::new(), true, &mut at_start), None);
    }

    #[test]
    fn a_system_quit_goes_ahead_only_with_the_changes_autosaved() {
        let written = || Ok(());
        let failed = || Err("no recovery folder".to_string());
        let unused = || -> Result<(), String> { panic!("no autosave written") };
        // Nothing unsaved, or the prompt was answered: the app ends at once.
        assert_eq!(
            system_quit(false, false, 7, None, 5, unused),
            SystemQuit::Exit
        );
        assert_eq!(
            system_quit(true, true, 7, None, 5, unused),
            SystemQuit::Exit
        );
        assert_eq!(
            system_quit(false, false, 7, None, 0, unused),
            SystemQuit::Exit
        );
        // Unsaved and autosaved: the newest changes are written first, or are in it already.
        assert_eq!(
            system_quit(false, true, 7, None, 5, written),
            SystemQuit::Exit
        );
        assert_eq!(
            system_quit(false, true, 7, Some(6), 5, written),
            SystemQuit::Exit
        );
        assert_eq!(
            system_quit(false, true, 7, Some(7), 5, unused),
            SystemQuit::Exit
        );
        // The write failed (or there is no recovery folder): ask.
        assert_eq!(
            system_quit(false, true, 7, Some(6), 5, failed),
            SystemQuit::Prompt
        );
        // Autosave off: ask, even with an older autosave of the same revision.
        assert_eq!(
            system_quit(false, true, 7, None, 0, unused),
            SystemQuit::Prompt
        );
        assert_eq!(
            system_quit(false, true, 7, Some(7), 0, unused),
            SystemQuit::Prompt
        );
    }

    #[test]
    fn the_unsaved_prompt_answers_the_system_quit_once_it_moves_on() {
        assert_eq!(
            prompt_answer(true, false),
            Some(true),
            "Save then saved, or Don't Save"
        );
        assert_eq!(
            prompt_answer(false, false),
            Some(false),
            "Cancel, or the save failed"
        );
        assert_eq!(
            prompt_answer(false, true),
            None,
            "a dialog of the prompt still open"
        );
    }

    #[test]
    fn the_loop_wakes_for_the_earlier_of_a_repaint_and_an_autosave() {
        let now = Instant::now();
        let soon = now + Duration::from_millis(16);
        let later = now + Duration::from_secs(300);
        assert_eq!(wake_deadline(None, None), None);
        assert_eq!(wake_deadline(Some(soon), None), Some(soon));
        assert_eq!(wake_deadline(None, Some(later)), Some(later));
        assert_eq!(wake_deadline(Some(soon), Some(later)), Some(soon));
        assert_eq!(wake_deadline(Some(later), Some(soon)), Some(soon));
        assert_eq!(
            wake_decision(now, wake_deadline(None, Some(later))),
            Wake::Until(later)
        );
    }

    #[test]
    fn the_recover_window_opens_at_start_only_for_crashed_autosaves() {
        let entry = |kind| recovery::Entry {
            kind,
            name: "house".into(),
            original: None,
            saved: SystemTime::UNIX_EPOCH,
            model: PathBuf::from("/r/house-1-2.autosave.skr"),
            error: None,
        };
        assert!(!opens_recover_at_start(&[]));
        assert!(opens_recover_at_start(&[entry(recovery::Kind::Autosave)]));
        assert!(!opens_recover_at_start(&[entry(recovery::Kind::Discarded)]));
    }

    #[test]
    fn recover_rows_say_how_long_ago_each_copy_was_saved() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let ago = |secs| saved_ago(now, now - Duration::from_secs(secs));
        assert_eq!(ago(5), "just now");
        assert_eq!(ago(300), "5 min ago");
        assert_eq!(ago(3 * 3600), "3 h ago");
        assert_eq!(ago(3 * 86_400), "3 days ago");
        assert_eq!(
            saved_ago(now, now + Duration::from_secs(60)),
            "just now",
            "a clock set back"
        );
        let entries = [
            recovery::Entry {
                kind: recovery::Kind::Autosave,
                name: "house".into(),
                original: Some(PathBuf::from("/m/house.skr")),
                saved: now - Duration::from_secs(600),
                model: PathBuf::from("/r/house-1-2.autosave.skr"),
                error: None,
            },
            recovery::Entry {
                kind: recovery::Kind::Discarded,
                name: "Untitled".into(),
                original: None,
                saved: now,
                model: PathBuf::from("/r/untitled-3.skr"),
                error: Some("the model file cannot be used".into()),
            },
        ];
        let rows = recover_rows(&entries, now);
        assert_eq!(rows[0].saved, "10 min ago");
        assert!(!rows[0].discarded);
        assert_eq!(rows[0].original.as_deref(), Some(Path::new("/m/house.skr")));
        assert!(rows[1].discarded);
        assert!(rows[1].error.is_some());
        // Discard asks about one; Discard All about every one listed.
        assert_eq!(
            discard_sheet(&entries, Some(Path::new("/r/untitled-3.skr"))),
            Some(Sheet::DiscardRecovered {
                models: vec![PathBuf::from("/r/untitled-3.skr")],
                names: vec!["Untitled".into()],
            })
        );
        let Some(Sheet::DiscardRecovered { models, .. }) = discard_sheet(&entries, None) else {
            panic!("a sheet for all");
        };
        assert_eq!(models.len(), 2);
        assert_eq!(
            discard_sheet(&entries, Some(Path::new("/r/gone.skr"))),
            None
        );
        assert_eq!(discard_sheet(&[], None), None);
    }

    #[test]
    fn a_recovered_model_is_titled_recovered_until_saved() {
        assert_eq!(
            window_title(None, None, Some("house"), true, true),
            "house (recovered)"
        );
        assert_eq!(
            window_title(None, None, Some("Untitled"), true, false),
            "Untitled (recovered)* - sketchup-alt"
        );
        let saved = Some(Path::new("/m/house.skr"));
        assert_eq!(
            window_title(saved, None, Some("house"), false, true),
            "house.skr"
        );
        assert_eq!(save_name(None, None, Some("house")), "house.skr");
        assert_eq!(save_name(saved, None, Some("house")), "house.skr");
    }

    #[test]
    fn a_discard_sheet_outlives_a_new_model() {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        let origin = SheetOrigin::of(&e);
        let sheet = Sheet::DiscardRecovered {
            models: vec![PathBuf::from("/r/a.skr")],
            names: vec!["a".into()],
        };
        e.execute(session::Command::New { discard: true }).unwrap();
        assert_eq!(sheet_applies(&sheet, &origin, &e), Ok(()));
        assert!(action_allowed_with_sheet(
            &UiAction::ConfirmDiscardRecovered
        ));
    }

    #[test]
    fn autosave_writes_once_the_interval_passed_and_follows_the_model() {
        let dir = recovery_folder("tick");
        let session = Session::start(&dir).unwrap();
        let mut a = Autosave::default();
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        let t = Instant::now();
        let min = Duration::from_secs(60);
        assert_eq!(
            a.tick(t, &session, &mut e, 5, None),
            None,
            "nothing unsaved"
        );
        e.execute(session::Command::AddEdge {
            start: [0.0, 0.0, 0.0],
            end: [1.0, 0.0, 0.0],
        })
        .unwrap();
        // The first unsaved change starts the interval.
        assert_eq!(a.tick(t, &session, &mut e, 5, None), Some(t + 5 * min));
        assert_eq!(
            a.tick(t + min, &session, &mut e, 5, None),
            Some(t + 5 * min)
        );
        assert!(models_in(&dir).is_empty());
        assert_eq!(
            a.tick(t + 5 * min, &session, &mut e, 5, None),
            None,
            "written"
        );
        assert_eq!(models_in(&dir).len(), 1);
        assert!(models_in(&dir)[0].starts_with("untitled-"));
        // The next change waits an interval from the last autosave.
        e.execute(session::Command::AddEdge {
            start: [0.0, 0.0, 0.0],
            end: [0.0, 1.0, 0.0],
        })
        .unwrap();
        let next = a.tick(t + 6 * min, &session, &mut e, 5, None);
        assert_eq!(next, Some(t + 10 * min));
        // Not while a tool operation is in progress, for one interval more at most.
        e.select_tool(tools::ToolKind::Line);
        let px = e
            .doc()
            .camera
            .project(glam::DVec3::new(3.0, 3.0, 0.0))
            .unwrap();
        e.cursor_moved(px);
        e.button(Button::Left, true);
        e.button(Button::Left, false);
        assert!(e.operation_in_progress());
        assert_eq!(
            a.tick(t + 11 * min, &session, &mut e, 5, None),
            Some(t + 15 * min),
            "busy"
        );
        assert!(e.operation_in_progress());
        assert_eq!(
            a.tick(t + 15 * min, &session, &mut e, 5, None),
            None,
            "written although still busy"
        );
        e.key(EditorKey::Escape);
        // Off: nothing more.
        e.execute(session::Command::AddEdge {
            start: [0.0, 0.0, 0.0],
            end: [0.0, 0.0, 1.0],
        })
        .unwrap();
        assert_eq!(a.tick(t + 30 * min, &session, &mut e, 0, None), None);
        // A recovered model's autosave keeps naming its original file.
        a.tick(
            t + 30 * min,
            &session,
            &mut e,
            5,
            Some(Path::new("/m/house.skr")),
        );
        let models = models_in(&dir);
        assert_eq!(models.len(), 1);
        assert!(models[0].starts_with("house-"), "{models:?}");
        // Saving makes it clean: the window clears the autosave, and the next change is
        // autosaved again.
        let saved = dir.with_extension("saved.skr");
        assert!(e.save_file(Some(&saved)));
        session.clear_autosave();
        assert_eq!(a.tick(t + 31 * min, &session, &mut e, 5, None), None);
        assert_eq!(a.revision, None);
        assert!(models_in(&dir).is_empty());
        e.execute(session::Command::AddEdge {
            start: [0.0, 0.0, 0.0],
            end: [2.0, 0.0, 0.0],
        })
        .unwrap();
        assert_eq!(
            a.tick(t + 32 * min, &session, &mut e, 5, None),
            Some(t + 37 * min)
        );
        assert_eq!(a.tick(t + 37 * min, &session, &mut e, 5, None), None);
        assert_eq!(models_in(&dir).len(), 1);
        let _ = std::fs::remove_file(&saved);
        drop(session);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_autosave_is_shown_once_and_tried_again_an_interval_later() {
        let dir = recovery_folder("failing");
        let session = Session::start(&dir).unwrap();
        let mut a = Autosave::default();
        let mut e = edited_editor();
        let t = Instant::now();
        let min = Duration::from_secs(60);
        a.tick(t, &session, &mut e, 1, None);
        // The folder is gone, and a file stands in its place: the folder check fails.
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::write(&dir, b"not a folder").unwrap();
        assert_eq!(
            a.tick(t + min, &session, &mut e, 1, None),
            Some(t + 2 * min)
        );
        assert!(e.status().contains("Autosave failed"), "{}", e.status());
        e.show_error("something else");
        assert_eq!(
            a.tick(t + 2 * min, &session, &mut e, 1, None),
            Some(t + 3 * min)
        );
        assert!(
            e.status().contains("something else"),
            "shown once: {}",
            e.status()
        );
        std::fs::remove_file(&dir).unwrap();
        assert_eq!(
            a.tick(t + 3 * min, &session, &mut e, 1, None),
            None,
            "written"
        );
        assert!(a.failure.is_none());
        drop(session);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod shared_tests {
    use super::*;

    fn model_bytes() -> Vec<u8> {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        e.execute(session::Command::AddEdge {
            start: [0.0, 0.0, 0.0],
            end: [1.0, 0.0, 0.0],
        })
        .unwrap();
        e.doc().encode_bytes().unwrap()
    }

    fn viewer() -> Editor {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        e.set_read_only(true);
        e
    }

    #[test]
    fn a_shared_model_with_a_sketchup_name_opens_and_names_the_bar() {
        let mut e = viewer();
        let mut name = String::from("old");
        let model = SharedModel {
            name: "Plan.skp".into(),
            bytes: model_bytes(),
        };
        assert!(open_shared_model(&mut e, &mut name, model));
        assert_eq!(name, "Plan.skp", "the name is for the bar only");
        assert!(e.doc().model().edges().next().is_some());
        assert!(e.doc().is_read_only());
    }

    #[test]
    fn the_name_changes_only_when_the_model_opened() {
        let mut e = viewer();
        let mut name = String::from("Kitchen");
        let bad = SharedModel {
            name: "Broken".into(),
            bytes: b"not a model".to_vec(),
        };
        assert!(!open_shared_model(&mut e, &mut name, bad));
        assert_eq!(name, "Kitchen");
    }

    #[test]
    fn open_on_phone_names_a_model_as_the_window_does() {
        fn p(s: &str) -> Option<&Path> {
            Some(Path::new(s))
        }
        assert_eq!(share_name(p("/a/house.skr"), None, None), "house");
        assert_eq!(share_name(None, p("/a/desk.skp"), None), "desk");
        assert_eq!(share_name(None, None, Some("shed.skr")), "shed");
        assert_eq!(
            share_name(p("/a/house.skr"), p("/b/old.skp"), None),
            "house"
        );
        assert_eq!(share_name(None, None, None), "Untitled");
    }

    #[test]
    fn an_editor_cannot_isolate_even_when_asked() {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        e.select_all();
        assert!(!e.isolate_selection());
        assert!(!e.isolated());
        assert!(!viewer_allows(&UiAction::Tool(tools::ToolKind::Line)));
        assert!(viewer_allows(&UiAction::KeepModel));
    }
}
