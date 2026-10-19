//! Window-independent editing state: routes input to navigation or the active tool, runs the
//! resulting commands on the document, and produces what the view shows over the model (the
//! HUD, the hover, the tool's solid) and the status text.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use api::{MessageKind, Mods};
use glam::{DAffine3, DVec2, DVec3};
use kernel::{DefinitionId, DefinitionKind, EntityId, InstanceId, MaterialId};
use render::PreviewVertex;
use render::scene::DEFAULT_FRONT;
use serde::{Deserialize, Serialize};
use session::{
    BatchCommand, BatchError, Camera, Command, CommandError, Document, ErrorKind, ImportReport,
    Lighting, Origin, Outcome, Projection, SelectMode, StandardView, TextureSource,
};
pub use session::{ImportAs, ImportUnits, UpAxis};
use tools::overlay::{marker, preview_lines, snap_label};
use tools::{
    Axis, Guides, Inference, Layout, Lock, Owner, Remembered, Scenery, Slab, SnapKind, Solid, Tool,
    ToolContext, ToolEvent, ToolKind, ToolOutput, Within, edge_under_in, face_under_in,
    infer_detail,
};

use crate::feedback::{Chip, Hud};
use crate::fly::{self, Fly, FlyKey};
use crate::navigation::{Button, LOGICAL_PX_PER_ZOOM_STEP, Navigation, Scroll, TrackpadScroll};
use crate::shortcuts::{self, Bound};

mod agent;
mod input;
pub use input::{DriveError, Driven, Stopped};
mod soften;
mod texture_position;

use soften::SoftenWindow;

/// How the window imports OBJ and STL files: the `import` command's `scale`, `up`, `as` and
/// `merge_coplanar`. Kept in the window's preferences, so missing keys are their defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportOptions {
    /// The length one file unit stands for; anything but meters becomes `scale`.
    pub units: ImportUnits,
    /// The file axis that points up (None: the format's own, Y for OBJ and Z for STL).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up: Option<UpAxis>,
    /// What the import makes: a group, a component or loose geometry.
    #[serde(rename = "as")]
    pub place: ImportAs,
    /// Whether coplanar neighbouring polygons merge into one face.
    pub merge_coplanar: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            units: ImportUnits::Meters,
            up: None,
            place: ImportAs::Group,
            merge_coplanar: true,
        }
    }
}

/// Whether `command` replaces the document (a new model, a file opened).
fn replaces_document(command: &Command) -> bool {
    matches!(command, Command::New { .. } | Command::Open { .. })
}

/// The one selected group or component of the open context, which Rename Component and Entity
/// Info's Name field rename.
#[derive(Clone, Debug, PartialEq)]
pub struct RenameTarget {
    pub definition: DefinitionId,
    pub kind: DefinitionKind,
    pub name: String,
}

/// A definition's name as the window shows it: an unnamed group is "Group".
fn shown_name(kind: DefinitionKind, name: &str) -> &str {
    if kind == DefinitionKind::Group && name.is_empty() {
        "Group"
    } else {
        name
    }
}

/// A layout of the scene with the revision and open context it was made for.
type LayoutCache = RefCell<Option<(u64, Vec<InstanceId>, Rc<Layout>)>>;

/// Snap radius in logical pixels (scaled to physical pixels by the display scale factor).
const PICK_RADIUS_LOGICAL_PX: f64 = 8.0;
/// How far (logical px) the cursor must travel from a press for it to be a drag. It is also
/// the window's one "same place" radius: a release this far from its press is not a click, and
/// a click this far from the last one does not count toward a double-click.
pub(crate) const DRAG_LOGICAL_PX: f64 = 4.0;
/// The opacity of the solid a tool previews.
const SOLID_ALPHA: f32 = 0.7;

/// Keys the editor handles itself (navigation and undo shortcuts are handled elsewhere).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorKey {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Escape,
    /// Lock the direction to an axis during an operation, or unlock it if it is locked there.
    LockAxis(Axis),
    /// Remove any direction lock.
    Unlock,
}

/// What the status bar shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub tool: &'static str,
    pub prompt: String,
    /// The measurements box label, e.g. "Length"; empty when the tool measures nothing.
    pub label: &'static str,
    /// Typed text (with a trailing "_") while typing, otherwise the live measurement.
    pub value: Option<String>,
    /// Feedback from the last tool action or failed command.
    pub message: Option<String>,
    /// The direction lock in force, e.g. "Locked: red axis".
    pub lock: Option<&'static str>,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}  |  {}", self.tool, self.prompt)?;
        if let Some(lock) = self.lock {
            write!(f, "  |  {lock}")?;
        }
        if let Some(value) = &self.value {
            write!(f, "  |  {}: {value}", self.label)?;
        }
        if let Some(message) = &self.message {
            write!(f, "  |  {message}")?;
        }
        Ok(())
    }
}

/// Feedback in the status bar, and whether it reports an error.
#[derive(Clone, Debug, PartialEq)]
struct Message {
    text: String,
    kind: MessageKind,
}

impl Message {
    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: MessageKind::Error,
        }
    }

    fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: MessageKind::Info,
        }
    }
}

pub struct Editor {
    doc: Document,
    nav: Navigation,
    tool: Box<dyn Tool>,
    kind: ToolKind,
    /// Tool settings kept across tool switches (circle sides, arc segments).
    remembered: Remembered,
    /// Whether Shift is held, as last reported by the window.
    shift: bool,
    /// Whether the left-button press reached the tool, so its release should too.
    tool_pressed: bool,
    /// Whether the next left press comes within the double-click time of the last click's
    /// release (`quick_press`); the press clears it.
    quick_press: bool,
    /// The document revision right after the tool's own last commands ran. A tool's retype
    /// or array starts with an Undo of that step; it runs only while the revision is still
    /// this one, so it never undoes a step made since (a material change, an undo, an edit).
    tool_step: Option<u64>,
    /// Direction of the last edge the cursor rested on, for parallel/perpendicular snaps.
    reference: Option<DVec3>,
    /// The direction lock in force during the current operation.
    lock: Option<Lock>,
    /// Whether `lock` comes from holding Shift (and ends when it is released).
    shift_lock: bool,
    cursor: DVec2,
    hit: Option<Inference>,
    /// Where the geometry `hit` snapped to lies, for the snap label.
    within: Within,
    preview: Vec<[DVec3; 2]>,
    /// The solid the tool previews (Push/Pull's pulled shape), in the open context's coordinates.
    solid: Option<Solid>,
    /// The inference under the cursor, the tool preview and what the tool shows of them are
    /// out of date. Input that may change them only sets this; they are worked out again once,
    /// when a frame is drawn (`refresh_hover`) or before input that uses them.
    hover_dirty: bool,
    /// The cursor left the view since the hover was last asked for: the marker stays hidden.
    hover_hidden: bool,
    /// A window, a menu or the gizmo lies over the cursor in the view: the window shows no
    /// highlight, marker or chip there (`set_covered`).
    covered: bool,
    /// Navigation moved the camera since the last frame: the hover waits until it stops.
    navigated: bool,
    /// Where the scene's meshes lie seen from the open context (see `cached_layout`).
    layout: LayoutCache,
    /// Text typed into the measurements box, not yet submitted.
    typed: String,
    /// Feedback from the last tool action or failed command.
    message: Option<Message>,
    /// The error a typed value just left, until the window takes it to flash.
    flash: Option<String>,
    /// Recovery files an agent's discards were kept in this session.
    recovered: Vec<PathBuf>,
    /// Physical pixels per logical pixel.
    scale_factor: f64,
    /// The material the Paint tool paints with (None: the default material).
    material: Option<MaterialId>,
    /// Whether the sampling modifier (Cmd or Ctrl) is held.
    sample: bool,
    /// How files are imported: the unit one file unit is (OBJ and STL files carry no units),
    /// the up axis, what the import makes and whether coplanar faces merge.
    import: ImportOptions,
    /// The extension of the last file exported, which the export dialog suggests next.
    last_export: &'static str,
    /// Fly mode's keys, speed and motion; the speed lasts for the session.
    fly: Fly,
    /// Whether fly mode is on: the mouse looks, keys move the eye, the tools rest.
    flying: bool,
    /// The Soften Edges window.
    soften: SoftenWindow,
    /// While an agent's `tool_input` runs: every command run, with its result.
    issued: Option<Vec<(Command, Result<Outcome, CommandError>)>>,
    /// The modifier keys the window reports the user holding.
    user_mods: Mods,
    /// The modifier keys `tool_input` holds, until a `modifiers` event lets them go or
    /// `cancel`.
    agent_mods: Mods,
    /// The modifiers a `tool_input` key event holds while its key goes down.
    key_mods: Mods,
    /// The model point (open context coordinates) a `tool_input` event put the cursor on:
    /// the tools look for it there, whatever hides it (see `Slab`).
    cursor_point: Option<DVec3>,
    /// How many times the document was replaced (a new model or a file opened, by the window
    /// or an agent): ids from before name other things afterwards.
    documents: u64,
}

/// The formats File > Export offers, as (filter name, extension).
pub const EXPORT_FORMATS: [(&str, &str); 4] = [
    ("OBJ", "obj"),
    ("STL", "stl"),
    ("glTF binary", "glb"),
    ("glTF", "gltf"),
];

/// The colors the Materials panel offers; new materials get them in turn.
pub const MATERIAL_PALETTE: [[u8; 3]; 12] = [
    [196, 84, 64],
    [222, 184, 135],
    [112, 146, 80],
    [70, 120, 180],
    [150, 150, 150],
    [240, 200, 80],
    [140, 90, 60],
    [120, 90, 160],
    [250, 250, 250],
    [40, 40, 40],
    [90, 170, 170],
    [230, 130, 160],
];

impl Editor {
    pub fn new(viewport: DVec2, scale_factor: f64) -> Self {
        let mut nav = Navigation::default();
        nav.set_scale_factor(scale_factor);
        Self {
            doc: Document::new(viewport),
            nav,
            tool: ToolKind::Select.make(),
            kind: ToolKind::Select,
            remembered: Remembered::default(),
            shift: false,
            tool_pressed: false,
            tool_step: None,
            quick_press: false,
            reference: None,
            lock: None,
            shift_lock: false,
            cursor: DVec2::ZERO,
            hit: None,
            within: Within::Open,
            preview: Vec::new(),
            solid: None,
            hover_dirty: false,
            hover_hidden: false,
            covered: false,
            navigated: false,
            layout: RefCell::new(None),
            typed: String::new(),
            message: None,
            flash: None,
            recovered: Vec::new(),
            scale_factor,
            material: None,
            sample: false,
            import: ImportOptions::default(),
            last_export: "obj",
            fly: Fly::default(),
            flying: false,
            soften: SoftenWindow::default(),
            issued: None,
            user_mods: Mods::default(),
            agent_mods: Mods::default(),
            key_mods: Mods::default(),
            cursor_point: None,
            documents: 0,
        }
    }

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    /// Who the document's next undo steps are made by.
    pub fn set_origin(&mut self, origin: Origin) {
        self.doc.set_origin(origin);
    }

    /// Runs a command from outside the UI (the agent API). A command that can change the model
    /// ends the tool's operation in progress, which may refer to geometry that changed.
    pub fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
        if command.changes_model() {
            self.reset_tool();
        }
        let replaces = replaces_document(&command);
        let result = self.doc.execute(command);
        if replaces && result.is_ok() {
            self.documents += 1;
        }
        self.invalidate_hover();
        result
    }

    /// Batch version of `execute`.
    pub fn execute_batch(
        &mut self,
        commands: Vec<BatchCommand>,
        atomic: bool,
    ) -> Result<Vec<Outcome>, BatchError> {
        if commands.iter().any(|c| c.command().changes_model()) {
            self.reset_tool();
        }
        // A batch that may have replaced the document counts as having done so.
        if commands.iter().any(|c| replaces_document(c.command())) {
            self.documents += 1;
        }
        let result = self.doc.execute_batch(commands, atomic);
        self.invalidate_hover();
        result
    }

    /// How many times the document was replaced (new or open, by the window or an agent).
    /// What was noted by id before a change names other things after it.
    pub fn documents(&self) -> u64 {
        self.documents
    }

    /// Shows `text` as an error in the status line.
    pub fn show_error(&mut self, text: &str) {
        self.message = Some(Message::error(text));
    }

    /// Switches to the tool `kind` (a fresh one, even if it is the current tool), dropping the
    /// current tool's operation and any typed text, and keeping the settings tools remember for
    /// the session, such as circle sides.
    pub fn select_tool(&mut self, kind: ToolKind) {
        self.tool.remember(&mut self.remembered);
        self.tool = kind.make_with(&self.remembered);
        self.kind = kind;
        self.preview.clear();
        self.solid = None;
        self.typed.clear();
        self.message = None;
        self.invalidate_hover();
    }

    pub fn tool_kind(&self) -> ToolKind {
        self.kind
    }

    pub fn resize(&mut self, viewport: DVec2) {
        if self.doc.camera.viewport != viewport {
            self.doc.camera.viewport = viewport;
            self.invalidate_hover();
        }
    }

    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
        self.nav.set_scale_factor(scale_factor);
        self.invalidate_hover();
    }

    /// The user's Shift and Alt, as the window reports them. Pressing Shift during an
    /// operation locks the direction the cursor follows (an axis, parallel or perpendicular);
    /// releasing it unlocks, unless `tool_input` holds it too. Returns true if the lock changed.
    pub fn set_modifiers(&mut self, shift: bool, alt: bool) -> bool {
        self.user_mods.shift = shift;
        self.user_mods.alt = alt;
        self.apply_modifiers()
    }

    /// The modifiers held: the user's and those `tool_input` holds.
    pub fn modifiers(&self) -> Mods {
        let (u, a, k) = (self.user_mods, self.agent_mods, self.key_mods);
        Mods {
            shift: u.shift || a.shift || k.shift,
            alt: u.alt || a.alt || k.alt,
            cmd: u.cmd || a.cmd || k.cmd,
        }
    }

    /// Puts the modifiers held into effect. Returns true if the lock changed.
    fn apply_modifiers(&mut self) -> bool {
        let held = self.modifiers();
        self.sample = held.cmd;
        self.nav.set_command(held.cmd);
        self.shift_and_alt(held.shift, held.alt)
    }

    fn shift_and_alt(&mut self, shift: bool, alt: bool) -> bool {
        self.fly.set_modifiers(shift, alt);
        if self.flying {
            // Shift and Alt change the flying speed; nothing locks.
            self.shift = shift;
            self.nav.set_modifiers(shift, alt);
            return false;
        }
        if shift != self.shift {
            // The lock follows what the cursor snaps to now.
            self.flush_hover();
        }
        let pressed = shift && !self.shift;
        let released = !shift && self.shift;
        self.shift = shift;
        self.nav.set_modifiers(shift, alt);
        if (pressed || released) && self.kind == ToolKind::Scale {
            // Shift switches uniform scaling: the preview follows at once.
            self.invalidate_hover();
            return true;
        }
        if pressed && self.lock.is_none() {
            let (Some(anchor), Some(hit)) = (self.tool.anchor(), self.hit) else {
                return false;
            };
            let dir = match hit.kind {
                SnapKind::Axis(a) => Some(a.dir()),
                SnapKind::Parallel | SnapKind::Perpendicular => {
                    (hit.point - anchor).try_normalize()
                }
                _ => None,
            };
            if let Some(dir) = dir {
                self.lock = Some(Lock {
                    dir,
                    kind: hit.kind,
                });
                self.shift_lock = true;
                self.invalidate_hover();
                return true;
            }
        } else if released && self.shift_lock {
            self.lock = None;
            self.shift_lock = false;
            self.invalidate_hover();
            return true;
        }
        false
    }

    /// Whether the sampling modifier (Cmd or Ctrl) is held: a Paint click then
    /// picks up a material instead of painting.
    /// It also makes trackpad scrolling zoom. The user's, as the window reports it.
    /// Returns true if that changes how the Paint tool's target is highlighted.
    pub fn set_sample_modifier(&mut self, held: bool) -> bool {
        let was = self.sampling();
        self.user_mods.cmd = held;
        self.apply_modifiers();
        self.sampling() != was
    }

    /// Whether the Paint tool is picking up a material (the sampling modifier is held), so its
    /// target is highlighted as Select's would be rather than in the paint's color.
    pub fn sampling(&self) -> bool {
        self.kind == ToolKind::Paint && self.sample
    }

    /// The material the Paint tool paints with; None (the default material) also when the
    /// one chosen no longer exists.
    pub fn current_material(&self) -> Option<MaterialId> {
        self.material
            .filter(|m| self.doc.scene().material(*m).is_some())
    }

    /// Makes `material` (None: the default material) the one to paint with, keeping the tool
    /// and what it is doing, as the API's `set_paint_material` does. False for a material the
    /// model does not have (the status says so).
    pub fn choose_material(&mut self, material: Option<MaterialId>) -> bool {
        match self.set_paint_material(material) {
            Ok(()) => true,
            Err(e) => {
                self.message = Some(Message::error(e.message));
                false
            }
        }
    }

    /// Chooses `material`, then the Paint tool unless it is active (the API's
    /// `set_paint_material`, then `set_tool` with `paint`).
    pub fn paint_with(&mut self, material: Option<MaterialId>) -> bool {
        if !self.choose_material(material) {
            return false;
        }
        if self.kind != ToolKind::Paint {
            self.select_tool(ToolKind::Paint);
        }
        true
    }

    /// Adds a material named "Material" (numbered when taken) in the next palette color and
    /// chooses it, keeping the tool. False if it could not be added (the status says why).
    pub fn add_material(&mut self) -> bool {
        let count = self.doc.scene().materials().count();
        let color = MATERIAL_PALETTE[count % MATERIAL_PALETTE.len()];
        self.message = None;
        let Some(outcome) = self.run(Command::AddMaterial {
            name: None,
            color: Some(color),
            opacity: None,
            texture: None,
            emission: None,
        }) else {
            return true;
        };
        let added = outcome.changes.materials.first().copied();
        self.choose_material(added)
    }

    /// Renames a material. A refused name leaves it and says why in the status.
    pub fn rename_material(&mut self, material: MaterialId, name: String) -> bool {
        self.message = None;
        self.run(Command::ChangeMaterial {
            material,
            name: Some(name),
            color: None,
            opacity: None,
            texture: None,
            emission: None,
        });
        true
    }

    /// Recolors a material.
    pub fn recolor_material(&mut self, material: MaterialId, color: [u8; 3]) -> bool {
        self.message = None;
        self.run(Command::ChangeMaterial {
            material,
            name: None,
            color: Some(color),
            opacity: None,
            texture: None,
            emission: None,
        });
        true
    }

    /// Sets a material's opacity, `percent` from 0 (invisible) to 100 (opaque).
    pub fn set_material_opacity(&mut self, material: MaterialId, percent: u8) -> bool {
        self.message = None;
        self.run(Command::ChangeMaterial {
            material,
            name: None,
            color: None,
            opacity: Some(f32::from(percent.min(100)) / 100.0),
            texture: None,
            emission: None,
        });
        true
    }

    /// Sets how strongly a material glows, in tenths (0 to 100: emission 0 to 10).
    pub fn set_material_emission(&mut self, material: MaterialId, tenths: u8) -> bool {
        self.message = None;
        self.run(Command::ChangeMaterial {
            material,
            name: None,
            color: None,
            opacity: None,
            texture: None,
            emission: Some(f32::from(tenths.min(100)) / 10.0),
        });
        true
    }

    /// Gives a material the image at `path` as its texture: as wide as the texture it replaces
    /// (or 1 m), its height following the new image's proportions. A file that cannot be used
    /// says why in the status.
    pub fn load_texture(&mut self, material: MaterialId, path: &Path) -> bool {
        self.message = None;
        self.run(Command::ChangeMaterial {
            material,
            name: None,
            color: None,
            opacity: None,
            texture: Some(Some(TextureSource {
                path: Some(path.display().to_string()),
                width: None,
                height: None,
            })),
            emission: None,
        });
        true
    }

    /// Takes a material's texture off; its color stays.
    pub fn remove_texture(&mut self, material: MaterialId) -> bool {
        self.message = None;
        self.run(Command::ChangeMaterial {
            material,
            name: None,
            color: None,
            opacity: None,
            texture: Some(None),
            emission: None,
        });
        true
    }

    /// Sets the width and height a material's texture covers, in millimeters.
    pub fn set_texture_size(&mut self, material: MaterialId, millimeters: [u32; 2]) -> bool {
        self.message = None;
        let [width, height] = millimeters.map(|mm| f64::from(mm) / 1000.0);
        self.run(Command::ChangeMaterial {
            material,
            name: None,
            color: None,
            opacity: None,
            texture: Some(Some(TextureSource {
                path: None,
                width: Some(width),
                height: Some(height),
            })),
            emission: None,
        });
        true
    }

    /// Deletes a material; what was painted with it goes back to the default material.
    pub fn delete_material(&mut self, material: MaterialId) -> bool {
        self.message = None;
        self.run(Command::DeleteMaterial { material });
        self.invalidate_hover();
        true
    }

    /// The window lost focus, so releases may never arrive: ends navigation, drops held
    /// modifiers and cancels a press the tool is waiting to see released. Returns true if
    /// anything visible may have changed (a Shift lock ended or a press was cancelled).
    pub fn focus_lost(&mut self) -> bool {
        self.user_mods = Mods::default();
        // Keys `tool_input` holds stay held, with a Shift lock they hold.
        let held = self.modifiers();
        self.shift = held.shift;
        self.sample = held.cmd;
        // Held fly keys are let go: their releases go to another window.
        self.fly.release_all();
        let dragged = self.nav.dragging();
        self.nav.reset();
        // A navigation drag that ended here shows the hover again.
        self.navigated = false;
        let cancelled = std::mem::take(&mut self.tool_pressed);
        if cancelled {
            self.send(ToolEvent::Cancel);
        }
        let unlocked = !held.shift && std::mem::take(&mut self.shift_lock);
        if unlocked {
            self.lock = None;
        }
        if unlocked || cancelled {
            self.invalidate_hover();
        }
        if held != Mods::default() {
            self.apply_modifiers();
        }
        unlocked || cancelled || (dragged && self.hover_dirty)
    }

    /// A key-repeat event. Backspace only deletes typed text and Delete does nothing, so holding
    /// them never erases the selection; a held letter shortcut (a tool, "g", Shift+G, Shift+Z)
    /// acts once; other keys behave as in [`Editor::key`].
    pub fn key_repeat(&mut self, key: EditorKey) -> bool {
        if self.flying {
            // Only the speed keys repeat; a held F or Escape leaves fly mode once.
            return match key {
                EditorKey::Char(c) if fly_speed_key(c).is_some() => self.key(key),
                _ => false,
            };
        }
        match key {
            EditorKey::Backspace => self.typed.pop().is_some(),
            EditorKey::Char(_) if self.typed.is_empty() => false,
            // Holding an arrow key must not toggle the lock at the repeat rate.
            EditorKey::Delete | EditorKey::LockAxis(_) | EditorKey::Unlock => false,
            _ => self.key(key),
        }
    }

    /// Returns true if anything visible may have changed: the camera moved, or the hover may
    /// have (it is worked out when the frame is drawn, see `refresh_hover`).
    pub fn cursor_moved(&mut self, pos: DVec2) -> bool {
        self.cursor_point = None;
        if self.flying {
            // No hover while flying; leaving fly mode works it out afresh.
            self.cursor = pos;
            return false;
        }
        let moved = pos != self.cursor || self.hover_hidden;
        self.cursor = pos;
        self.hover_hidden = false;
        let camera_moved = self.nav.cursor_moved(&mut self.doc.camera, pos);
        if camera_moved {
            self.navigated = true;
        }
        if moved || camera_moved {
            self.invalidate_hover();
        }
        moved || camera_moved
    }

    /// The cursor left the window: hide the inference marker.
    pub fn cursor_left(&mut self) -> bool {
        self.hover_hidden = true;
        self.hit.take().is_some() || self.hover_dirty
    }

    /// The copy modifier (Ctrl) was tapped. The Move and Rotate tools switch copying, the Tape
    /// Measure switches guide creation, the Polygon tool switches inscribed and circumscribed,
    /// the Scale tool switches scaling about the center; elsewhere it does nothing and returns
    /// false.
    pub fn toggle_copy(&mut self) -> bool {
        let takes_ctrl = matches!(
            self.kind,
            ToolKind::TapeMeasure | ToolKind::Polygon | ToolKind::Scale
        );
        if self.flying || !copies(self.kind) && !takes_ctrl {
            return false;
        }
        self.send(ToolEvent::ToggleCopy);
        true
    }

    /// Says whether the next left press comes within the double-click time of the last
    /// click's release. The window times it; `tool_input` says so for the later presses of a
    /// `double_click` or `triple_click`. Push/Pull swallows such a press near its first click (a
    /// double-click's second press) instead of finishing a tiny pull.
    pub fn quick_press(&mut self, quick: bool) {
        self.quick_press = quick;
    }

    /// A button went down or up. Returns true if anything visible may have changed.
    pub fn button(&mut self, button: Button, pressed: bool) -> bool {
        let changed = self.press_or_release(button, pressed);
        if pressed && button == Button::Left {
            self.quick_press = false;
        }
        changed
    }

    fn press_or_release(&mut self, button: Button, pressed: bool) -> bool {
        if self.flying {
            // Clicks do nothing to the model while flying.
            return false;
        }
        let dragged = self.nav.dragging();
        let (doc, cursor, cache) = (&self.doc, self.cursor, &self.layout);
        let to_root = doc.context_transform();
        let used = self.nav.button(&doc.camera, button, pressed, |c| {
            let layout = cached_layout(cache, doc);
            pick(&scenery_in(&layout, doc), to_root, c, cursor)
        });
        if used {
            if dragged && !self.nav.dragging() {
                // The drag ended: the hover, put off while it moved the camera, can show.
                self.navigated = false;
                return self.hover_dirty;
            }
            return false;
        }
        if button != Button::Left {
            return false;
        }
        // A release reaches the tool only if its press did (not one that began on a panel or
        // in another tool).
        if !pressed && !std::mem::take(&mut self.tool_pressed) {
            return false;
        }
        self.hover_now();
        let Some(hit) = self.hit else { return false };
        self.tool_pressed = pressed;
        self.send(if pressed {
            ToolEvent::Click(hit)
        } else {
            ToolEvent::Release(hit)
        });
        true
    }

    /// The primary button was double-clicked (call it after the second release). The Select
    /// tool opens the group under the cursor, or closes the open one when there is nothing of
    /// it there.
    pub fn double_click(&mut self) -> bool {
        if self.flying {
            return false;
        }
        self.hover_now();
        let Some(hit) = self.hit else { return false };
        self.send(ToolEvent::DoubleClick(hit));
        true
    }

    /// The primary button was triple-clicked (call it after the third release, the second having
    /// called `double_click`). The Select tool selects everything connected to what is under
    /// the cursor.
    pub fn triple_click(&mut self) -> bool {
        if self.flying {
            return false;
        }
        self.hover_now();
        let Some(hit) = self.hit else { return false };
        self.send(ToolEvent::TripleClick(hit));
        true
    }

    /// A scroll event: the wheel zooms, a trackpad orbits, pans or zooms (see `Navigation`).
    /// Returns true if the camera moved.
    pub fn scroll(&mut self, scroll: Scroll) -> bool {
        if self.flying {
            return self.fly_scroll(scroll);
        }
        self.navigate(|nav, cam, pick| nav.scroll(cam, scroll, pick))
    }

    /// Sets what a plain trackpad scroll does (the window's Settings; headless and the API
    /// keep Orbit). Fly mode's scrolling is the same either way.
    pub fn set_trackpad(&mut self, mode: TrackpadScroll) {
        self.nav.set_trackpad(mode);
    }

    /// A new trackpad scroll began: its orbit or pan picks a fresh pivot.
    pub fn scroll_started(&mut self) {
        self.nav.scroll_started();
    }

    /// A trackpad pinch by `delta` (magnification change, positive zooms in) zooms toward
    /// the cursor. Returns true if the camera moved.
    pub fn pinch(&mut self, delta: f64) -> bool {
        if self.flying {
            return false;
        }
        self.navigate(|nav, cam, pick| nav.pinch(cam, delta, pick))
    }

    /// Runs a navigation step on the document camera, giving it a picker for the model point
    /// under the cursor as any camera sees it. If the camera moved, the hover waits for
    /// navigation to stop (see `refresh_hover`).
    fn navigate(
        &mut self,
        step: impl FnOnce(
            &mut Navigation,
            &mut Camera,
            &mut dyn FnMut(&Camera) -> Option<DVec3>,
        ) -> bool,
    ) -> bool {
        let mut camera = self.doc.camera.clone();
        let layout = self.layout();
        let (doc, cursor) = (&self.doc, self.cursor);
        let to_root = doc.context_transform();
        let scenery = scenery_in(&layout, doc);
        let moved = step(&mut self.nav, &mut camera, &mut |c| {
            pick(&scenery, to_root, c, cursor)
        });
        if moved {
            self.doc.camera = camera;
            self.navigated = true;
            self.invalidate_hover();
        }
        moved
    }

    /// True while a button drag orbits or pans the camera.
    pub fn navigating(&self) -> bool {
        self.nav.dragging()
    }

    /// True while fly mode is on.
    pub fn flying(&self) -> bool {
        self.flying
    }

    /// Turns fly mode on or off (F, or Camera > Fly mode).
    pub fn toggle_fly(&mut self) -> bool {
        self.set_flying(!self.flying)
    }

    /// Enters or leaves fly mode. Entering switches a parallel view to perspective, ends a
    /// navigation drag and a press the tool waits on, and puts the camera's target a meter
    /// ahead of the eye with the view level enough to fly (see `fly::level`); the tool keeps
    /// its operation, out of sight until fly mode ends. Leaving stops at once and shows the
    /// hover again. Returns true if fly mode changed.
    pub fn set_flying(&mut self, on: bool) -> bool {
        if on == self.flying {
            return false;
        }
        self.fly.stop();
        if on {
            if self.doc.camera.is_parallel() {
                self.set_projection(Projection::Perspective);
            }
            self.nav.end_drag();
            self.navigated = false;
            if std::mem::take(&mut self.tool_pressed) {
                self.send(ToolEvent::Cancel);
            }
            fly::level(&mut self.doc.camera);
            self.message = None;
            self.flying = true;
        } else {
            self.flying = false;
        }
        self.invalidate_hover();
        true
    }

    /// A moving key (by its place on the keyboard) went down or up while flying. Returns true
    /// if frames must start: the eye starts or stops moving.
    pub fn fly_key(&mut self, key: FlyKey, pressed: bool) -> bool {
        self.flying && self.fly.key(key, pressed)
    }

    /// Turns the view by a mouse move of `delta` logical pixels while flying. Returns true if
    /// the view turned.
    pub fn fly_look(&mut self, delta: DVec2) -> bool {
        if !self.flying || delta == DVec2::ZERO || !delta.is_finite() {
            return false;
        }
        fly::look(&mut self.doc.camera, delta);
        true
    }

    /// Changes the flying speed by `steps` (positive faster). True if it changed.
    pub fn fly_speed(&mut self, steps: i32) -> bool {
        self.flying && self.fly.step_speed(steps)
    }

    /// A scroll while flying: a mouse wheel changes the speed a step a notch, and so does a
    /// trackpad with Cmd/Ctrl held (a step per `LOGICAL_PX_PER_ZOOM_STEP`); otherwise a
    /// trackpad scroll looks around as if dragging the scene.
    fn fly_scroll(&mut self, scroll: Scroll) -> bool {
        match scroll {
            Scroll::Lines(lines) => self.fly.scroll_speed(lines.y),
            Scroll::Pixels(px) if self.sample => self
                .fly
                .scroll_speed(px.y / self.scale_factor / LOGICAL_PX_PER_ZOOM_STEP),
            Scroll::Pixels(px) => self.fly_look(-px / self.scale_factor),
        }
    }

    /// Moves the eye for a frame drawn at `now` while flying. True while frames must keep
    /// coming: a key is held or the eye still glides. A view switched to parallel (by an agent)
    /// ends fly mode, which also needs a frame.
    pub fn fly_tick(&mut self, now: Instant) -> bool {
        if !self.flying {
            return false;
        }
        if self.doc.camera.is_parallel() {
            return self.set_flying(false);
        }
        self.fly.tick(&mut self.doc.camera, now)
    }

    /// Typing goes to the measurements box once it starts with a digit, ".", "-", "/" or "*"
    /// (or "x" in the Move and Rotate tools, for arrays like "x5"); other letters, and Space,
    /// do what `shortcuts::TABLE` binds them to ("g" makes a group, Shift+G a component,
    /// Shift+Z zooms to the whole model, "f" enters fly mode, the rest pick tools; a shifted
    /// letter with no binding of its own acts as the plain one). In fly mode only Escape and "f"
    /// (leave) and the speed keys ("[" "-" slower, "]" "=" faster) do anything. Enter
    /// submits the typed text to the tool. Escape clears the typed text, or else cancels the
    /// tool's operation (the Select tool then closes the open group). Delete, and Backspace
    /// with nothing typed, erase the selection. Returns true if anything changed.
    pub fn key(&mut self, key: EditorKey) -> bool {
        if self.flying {
            // Only leaving and the speed keys; tool shortcuts, typing and Escape's other
            // jobs wait until fly mode ends.
            return match key {
                EditorKey::Escape | EditorKey::Char('f' | 'F') => self.set_flying(false),
                EditorKey::Char(c) => fly_speed_key(c).is_some_and(|steps| self.fly_speed(steps)),
                _ => false,
            };
        }
        self.flush_hover();
        match key {
            EditorKey::Char(c) if self.typed.is_empty() => {
                if self.starts_typing(c) {
                    self.typed.push(c);
                    true
                } else if let Some(bound) = shortcuts::plain(c) {
                    self.run_shortcut(bound)
                } else {
                    false
                }
            }
            EditorKey::Char(c) => {
                if !c.is_control() {
                    self.typed.push(c);
                }
                true
            }
            EditorKey::Backspace if self.typed.is_empty() => self.erase_selection(),
            EditorKey::Backspace => self.typed.pop().is_some(),
            EditorKey::Delete => self.erase_selection(),
            EditorKey::Enter if self.typed.is_empty() => false,
            EditorKey::Enter => {
                let text = std::mem::take(&mut self.typed);
                self.send(ToolEvent::Value(text));
                true
            }
            EditorKey::Escape if !self.typed.is_empty() => {
                self.typed.clear();
                true
            }
            EditorKey::Escape => {
                self.send(ToolEvent::Cancel);
                true
            }
            EditorKey::LockAxis(axis) => {
                if self.tool.anchor().is_none() {
                    // Before the first click: the plane of a tool that works in one (Rotate);
                    // the same axis again unlocks it.
                    let axis = (self.tool.plane_lock() != Some(axis)).then_some(axis);
                    if !self.tool.lock_plane(axis) {
                        return false;
                    }
                    self.invalidate_hover();
                    return true;
                }
                let same = self.lock.is_some_and(|l| l.kind == SnapKind::Axis(axis));
                self.lock = (!same).then_some(Lock {
                    dir: axis.dir(),
                    kind: SnapKind::Axis(axis),
                });
                self.shift_lock = false;
                self.invalidate_hover();
                true
            }
            EditorKey::Unlock
                if self.tool.anchor().is_none() && self.tool.plane_lock().is_some() =>
            {
                self.tool.lock_plane(None);
                self.invalidate_hover();
                true
            }
            EditorKey::Unlock => {
                let had = self.lock.take().is_some();
                self.shift_lock = false;
                self.invalidate_hover();
                had
            }
        }
    }

    /// Whether `c`, typed with nothing typed yet, starts a value for the measurements box.
    fn starts_typing(&self, c: char) -> bool {
        let starts_value = c.is_ascii_digit() || matches!(c, '.' | '-' | '/' | '*');
        let starts_array = matches!(c, 'x' | 'X') && copies(self.kind);
        starts_value || starts_array
    }

    /// What `key` would run from the shortcut table now: not flying, nothing typed, a key with
    /// a plain binding. None otherwise.
    pub fn shortcut_for(&self, key: EditorKey) -> Option<Bound> {
        if self.flying || !self.typed.is_empty() {
            return None;
        }
        match key {
            EditorKey::Char(c) if !self.starts_typing(c) => shortcuts::plain(c),
            _ => None,
        }
    }

    /// Does what a key typed without Cmd is bound to. True if anything changed.
    fn run_shortcut(&mut self, bound: Bound) -> bool {
        match bound {
            Bound::Tool(kind) => {
                self.select_tool(kind);
                true
            }
            Bound::MakeGroup => self.make_group(),
            Bound::MakeComponent => self.make_component(),
            Bound::ZoomExtents => self.zoom_extents(),
            Bound::ToggleFly => self.set_flying(true),
            // Cmd chords, and Delete and Escape, which `key` handles before letters.
            _ => false,
        }
    }

    /// Erases the selected edges and faces as one undo step. False if nothing is selected.
    pub fn erase_selection(&mut self) -> bool {
        if self.doc.selection().is_empty() {
            return false;
        }
        let ids = self.doc.selection().iter().copied().collect();
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        self.run(Command::Erase { ids });
        self.invalidate_hover();
        true
    }

    /// Makes a group of the selection. With nothing selected it says so in the status line
    /// (and returns true, so the window redraws).
    pub fn make_group(&mut self) -> bool {
        let ids: Vec<EntityId> = self.doc.selection().iter().copied().collect();
        if ids.is_empty() {
            self.message = Some(Message::error("Select geometry to make a group"));
            return true;
        }
        self.run_selection_command(Command::MakeGroup { ids })
    }

    /// Makes a component of the selection, named "Component N" with the first free N. With
    /// nothing selected it says so in the status line (and returns true, so the window redraws).
    pub fn make_component(&mut self) -> bool {
        let name = self.next_component_name();
        self.make_component_named(&name)
    }

    /// "Component N" with the first free N.
    pub fn next_component_name(&self) -> String {
        let names: Vec<&str> = self
            .doc
            .scene()
            .definitions()
            .map(|(_, d)| d.name.as_str())
            .collect();
        (1..)
            .map(|n| format!("Component {n}"))
            .find(|n| !names.contains(&n.as_str()))
            .expect("a free name")
    }

    /// Makes a component of the selection named `name`. With nothing selected it says so in
    /// the status line (and returns true, so the window redraws); a refused name says why.
    pub fn make_component_named(&mut self, name: &str) -> bool {
        let ids: Vec<EntityId> = self.doc.selection().iter().copied().collect();
        if ids.is_empty() {
            self.message = Some(Message::error("Select geometry to make a component"));
            return true;
        }
        self.run_selection_command(Command::MakeComponent {
            ids,
            name: name.to_string(),
        })
    }

    /// Removes every definition no instance places (`purge_definitions`) as one undo step, and
    /// says how many went: "Purged N unused definitions", or "Nothing to purge". True, so the
    /// window redraws the message.
    pub fn purge_unused(&mut self) -> bool {
        self.message = None;
        if let Some(outcome) = self.run(Command::PurgeDefinitions {}) {
            self.message = Some(Message::info(match outcome.purged.unwrap_or(0) {
                0 => "Nothing to purge".to_string(),
                1 => "Purged 1 unused definition".to_string(),
                n => format!("Purged {n} unused definitions"),
            }));
        }
        true
    }

    /// The one selected group or component of the open context, if exactly one is selected.
    pub fn rename_target(&self) -> Option<RenameTarget> {
        let selection = self.doc.selection();
        let (Some(EntityId::Instance(id)), 1) = (selection.first(), selection.len()) else {
            return None;
        };
        let definition = self.doc.model().instance(*id)?.definition;
        let d = self.doc.scene().definition(definition)?;
        Some(RenameTarget {
            definition,
            kind: d.kind,
            name: d.name.clone(),
        })
    }

    /// Renames a definition as one undo step and says "Renamed <old> to <new>", or why the
    /// session refused. The same name again does nothing. True if the status may have changed.
    pub fn rename_definition(&mut self, definition: DefinitionId, name: &str) -> bool {
        let old = self
            .doc
            .scene()
            .definition(definition)
            .map(|d| (d.kind, d.name.clone()));
        if old.as_ref().is_some_and(|(_, n)| n == name) {
            return false;
        }
        self.message = None;
        if self
            .run(Command::RenameDefinition {
                definition,
                name: name.to_string(),
            })
            .is_some()
            && let Some((kind, old)) = old
        {
            self.message = Some(Message::info(format!(
                "Renamed {} to {}",
                shown_name(kind, &old),
                shown_name(kind, name)
            )));
        }
        true
    }

    /// Explodes the selected groups and components. With none selected it says so in the
    /// status line (and returns true, so the window redraws).
    pub fn explode_selection(&mut self) -> bool {
        let ids: Vec<EntityId> = self
            .doc
            .selection()
            .iter()
            .copied()
            .filter(|id| matches!(id, EntityId::Instance(_)))
            .collect();
        if ids.is_empty() {
            self.message = Some(Message::error("Select a group or component to explode"));
            return true;
        }
        self.run_selection_command(Command::Explode { ids })
    }

    /// Fits the whole model in the view (Shift+Z). The tool's operation goes on. True if
    /// anything visible changed (false, say, for an empty model or a view already fitted).
    pub fn zoom_extents(&mut self) -> bool {
        self.run_camera_command(Command::ZoomExtents {})
    }

    /// Looks at the target from a standard direction. The tool's operation goes on. True if
    /// anything visible changed.
    pub fn standard_view(&mut self, view: StandardView) -> bool {
        self.run_camera_command(Command::StandardView { view })
    }

    /// Switches the projection. False if the camera already has it.
    pub fn set_projection(&mut self, projection: Projection) -> bool {
        if self.doc.camera.projection == projection {
            return false;
        }
        self.run_camera_command(Command::SetProjection { projection })
    }

    /// The camera's vertical field of view in degrees.
    pub fn field_of_view(&self) -> f64 {
        self.doc.camera.fov_y.to_degrees()
    }

    /// Sets the camera's vertical field of view through `set_camera`, keeping eye, target and
    /// up (so a top or bottom view keeps its screen direction). The tool's operation goes on.
    /// A value out of range is refused with the session's message. True if anything visible
    /// changed.
    pub fn set_field_of_view(&mut self, degrees: f64) -> bool {
        let camera = &self.doc.camera;
        let command = Command::SetCamera {
            eye: camera.eye.to_array(),
            target: camera.target.to_array(),
            fov_y_deg: Some(degrees),
            up: Some(camera.up.to_array()),
            projection: None,
        };
        self.run_camera_command(command)
    }

    /// Lights the view by day or by night; the tool's operation goes on. False if it already
    /// is.
    pub fn set_lighting(&mut self, mode: Lighting) -> bool {
        if self.doc.lighting == mode {
            return false;
        }
        self.run_camera_command(Command::SetLighting { mode });
        true
    }

    /// Runs a camera command; the tool's operation and typed text stay. True if the camera or
    /// the status message changed (only a camera change can change the hover).
    fn run_camera_command(&mut self, command: Command) -> bool {
        let camera = self.doc.camera.clone();
        let cleared = self.message.take().is_some();
        self.run(command);
        let moved = self.doc.camera != camera;
        if moved {
            self.invalidate_hover();
        }
        moved || cleared || self.message.is_some()
    }

    /// Erases every guide of the open context as one undo step. False if there are none.
    pub fn delete_guides(&mut self) -> bool {
        if self.doc.model().guides().next().is_none() {
            return false;
        }
        self.run_selection_command(Command::DeleteGuides {})
    }

    /// Closes the open group or component. False if none is open.
    pub fn close_instance(&mut self) -> bool {
        if self.doc.context().is_empty() {
            return false;
        }
        self.run_selection_command(Command::CloseInstance {})
    }

    /// Selects the outliner node at `path` (instance ids from the root down to it), first
    /// opening the context it is in when that is not the open one. With `toggle` a node of the
    /// open context is added to or removed from the selection instead. False for an empty path.
    pub fn outliner_select(&mut self, path: &[EntityId], toggle: bool) -> bool {
        let Some((id, parent)) = path.split_last() else {
            return false;
        };
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        let open = self.doc.context().iter().map(|i| EntityId::Instance(*i));
        let in_context = parent.iter().copied().eq(open);
        if in_context
            || self
                .run(Command::OpenPath {
                    path: parent.to_vec(),
                })
                .is_some()
        {
            let mode = if toggle && in_context {
                SelectMode::Toggle
            } else {
                SelectMode::Replace
            };
            self.run(Command::Select {
                ids: vec![*id],
                mode,
                all: false,
            });
        }
        self.invalidate_hover();
        true
    }

    /// Opens the instances along `path` (from the root; empty: the root) for editing, as a
    /// double-click in the outliner does.
    pub fn open_path(&mut self, path: Vec<EntityId>) -> bool {
        self.run_selection_command(Command::OpenPath { path })
    }

    /// Runs a command that works on the selection or the open context, dropping the tool's
    /// operation and any typed text first.
    fn run_selection_command(&mut self, command: Command) -> bool {
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        self.run(command);
        self.invalidate_hover();
        true
    }

    /// Notes a recovery file an agent's discard was kept in: the status says where, and
    /// stderr too (the window may be quitting).
    pub fn note_recovery(&mut self, path: PathBuf) {
        let line = format!(
            "An agent discarded unsaved changes; they were kept in {}",
            path.display()
        );
        eprintln!("{line}");
        self.message = Some(Message::info(line));
        self.recovered.push(path);
    }

    /// The recovery files written this session, oldest first.
    pub fn recovered(&self) -> &[PathBuf] {
        &self.recovered
    }

    /// Starts an empty model, dropping unsaved changes (the window asks about them first).
    /// Always true (the view changes).
    pub fn new_model(&mut self) -> bool {
        // Material ids come from the model's own counter: the old choice would name another.
        self.material = None;
        self.run_selection_command(Command::New { discard: true })
    }

    /// Opens a `.skr` file, or a SketchUp file as a new model never saved, dropping unsaved
    /// changes (the window asks about them first). False if it cannot be opened; the status
    /// then says why. A SketchUp file's status lists what it made.
    pub fn open_file(&mut self, path: &Path) -> bool {
        let revision = self.doc.revision();
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        let outcome = self.run(Command::Open {
            path: path.display().to_string(),
            discard: true,
        });
        self.invalidate_hover();
        let opened = self.doc.revision() != revision;
        if opened {
            self.material = None;
        }
        if let Some(report) = outcome.and_then(|o| o.import) {
            self.message = Some(Message::info(format!(
                "Opened {}{}",
                display_name(path),
                sketchup_summary(&report)
            )));
        }
        opened
    }

    /// Saves to `path`, or to the file last opened or saved. False if it cannot be saved; the
    /// status then says why.
    pub fn save_file(&mut self, path: Option<&Path>) -> bool {
        self.message = None;
        let saved = self
            .run(Command::Save {
                path: path.map(|p| p.display().to_string()),
                // The file dialog asked before replacing a file.
                overwrite: true,
            })
            .is_some();
        if saved {
            self.message = Some(Message::info(format!(
                "Saved {}",
                self.doc
                    .path()
                    .map_or_else(String::new, |p| p.display().to_string())
            )));
        }
        saved
    }

    /// Exports every face of the model to `path`, in the format its extension names. False if
    /// it cannot be exported; the status then says why.
    pub fn export_file(&mut self, path: &Path) -> bool {
        self.message = None;
        let outcome = self.run(Command::Export {
            path: path.display().to_string(),
            format: None,
            // The file dialog asked before replacing a file.
            overwrite: true,
            refuse_links: false,
        });
        let Some(outcome) = outcome else {
            return false;
        };
        let skipped = outcome.export.map_or(0, |r| r.skipped_faces);
        self.message = Some(Message::info(export_message(path, skipped)));
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
        if let Some((_, ext)) = EXPORT_FORMATS
            .iter()
            .find(|(_, x)| Some(*x) == ext.as_deref())
        {
            self.last_export = ext;
        }
        true
    }

    /// The export formats, the one exported to last first (OBJ before any export): the
    /// export dialog lists them in this order and suggests the first one's extension.
    pub fn export_formats(&self) -> Vec<(&'static str, &'static str)> {
        let (last, rest): (Vec<_>, Vec<_>) = EXPORT_FORMATS
            .iter()
            .copied()
            .partition(|(_, x)| *x == self.last_export);
        [last, rest].concat()
    }

    /// How files are imported, for the rest of the session.
    pub fn import_options(&self) -> ImportOptions {
        self.import
    }

    /// Sets how files are imported, for the rest of the session.
    pub fn set_import_options(&mut self, options: ImportOptions) {
        self.import = options;
    }

    /// The unit imported files are read in.
    pub fn import_units(&self) -> ImportUnits {
        self.import.units
    }

    /// Sets the unit imported files are read in, for the rest of the session. The other
    /// import options stay.
    pub fn set_import_units(&mut self, units: ImportUnits) {
        self.import.units = units;
    }

    /// Imports an OBJ, STL or SketchUp file (the format its extension names) into the open
    /// context as `import_options` say (by default a group, which becomes the selection), as
    /// one undo step. A SketchUp file keeps its own units, axes and faces: only `place`
    /// applies. False if it cannot be imported; the status then says why.
    pub fn import_file(&mut self, path: &Path) -> bool {
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        let sketchup = is_sketchup_path(path);
        let options = self.import;
        let outcome = self.run(Command::Import {
            library_roots: None,
            path: path.display().to_string(),
            format: None,
            place: options.place,
            scale: (!sketchup && options.units != ImportUnits::Meters)
                .then(|| options.units.meters()),
            up: if sketchup { None } else { options.up },
            merge_coplanar: (!sketchup).then_some(options.merge_coplanar),
        });
        self.invalidate_hover();
        let Some(report) = outcome.and_then(|o| o.import) else {
            return false;
        };
        let name = display_name(path);
        if sketchup {
            self.message = Some(Message::info(format!(
                "Imported {name}{}",
                sketchup_summary(&report)
            )));
            return true;
        }
        let faces = match report.faces {
            1 => "1 face".to_string(),
            n => format!("{n} faces"),
        };
        let units = match options.units {
            ImportUnits::Meters => String::new(),
            u => format!(" in {}", u.label().to_lowercase()),
        };
        self.message = Some(Message::info(match report.skipped {
            0 => format!("Imported {name}{units}: {faces}"),
            1 => format!("Imported {name}{units}: {faces}; 1 polygon skipped"),
            n => format!("Imported {name}{units}: {faces}; {n} polygons skipped"),
        }));
        true
    }

    pub fn undo(&mut self) -> bool {
        self.reset_tool();
        self.run(Command::Undo {
            count: None,
            expect: None,
        });
        self.invalidate_hover();
        true
    }

    pub fn redo(&mut self) -> bool {
        self.reset_tool();
        self.run(Command::Redo { count: None });
        self.invalidate_hover();
        true
    }

    /// True for an empty, unedited model: the root open, the root mesh without vertices,
    /// instances or guides, and nothing to undo or redo. The window shows its start card then.
    pub fn untouched(&self) -> bool {
        let root = self.doc.scene().root();
        self.doc.context().is_empty()
            && root.vertices().next().is_none()
            && root.instances().next().is_none()
            && root.guides().next().is_none()
            && self.doc.undo_label().is_none()
            && self.doc.redo_label().is_none()
    }

    /// Selects every edge, face, group and guide of the open context (Cmd+A). The tool and
    /// its operation stay. Always true.
    pub fn select_all(&mut self) -> bool {
        self.run(Command::Select {
            ids: Vec::new(),
            mode: SelectMode::Replace,
            all: true,
        });
        // Move and Rotate prompt by whether something is selected.
        self.invalidate_hover();
        true
    }

    /// Selects nothing (Cmd+Shift+A). The tool and its operation stay. False if nothing was
    /// selected.
    pub fn select_none(&mut self) -> bool {
        if self.doc.selection().is_empty() {
            return false;
        }
        self.run(Command::Select {
            ids: Vec::new(),
            mode: SelectMode::Replace,
            all: false,
        });
        self.invalidate_hover();
        true
    }

    /// Duplicates the selection (Cmd+D): Move in copy mode, with the base point clicked under
    /// the cursor when it is in the view and under no window, so the copy follows the cursor
    /// and the next click or a typed distance places it as one undo step. With the cursor
    /// elsewhere Move waits for a base point. Refused, with a message, when nothing is
    /// selected.
    pub fn duplicate(&mut self) -> Result<(), CommandError> {
        if self.doc.selection().is_empty() {
            self.message = Some(Message::error("Select geometry or a group to duplicate"));
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "select something to duplicate first",
            ));
        }
        // Read before `hover_now`, which shows the hover again.
        let in_view = !self.flying && !self.hover_hidden && !self.covered;
        self.select_tool(ToolKind::Move);
        self.send(ToolEvent::ToggleCopy);
        if in_view {
            self.hover_now();
            if let Some(hit) = self.hit {
                self.send(ToolEvent::Click(hit));
            }
        }
        Ok(())
    }

    /// The status as one line: tool, prompt, measurement and feedback separated by bars, as
    /// the next frame shows it.
    #[cfg(test)]
    pub fn status(&mut self) -> String {
        self.flush_hover();
        self.status_parts().to_string()
    }

    /// The tool, what to do next, the measurement and any feedback, as of the last
    /// `refresh_hover`.
    pub fn status_parts(&self) -> Status {
        if self.flying {
            return Status {
                tool: "Fly",
                prompt: fly::status_text(self.fly.speed()),
                label: "",
                value: None,
                message: None,
                lock: None,
            };
        }
        let (label, value) = self.tool.measurement();
        let value = if self.typed.is_empty() {
            value
        } else {
            Some(format!("{}_", self.typed))
        };
        // The plane lock is shown until the first click; during the turn, the direction lock.
        let plane = self
            .tool
            .plane_lock()
            .filter(|_| self.tool.anchor().is_none())
            .map(SnapKind::Axis);
        let lock = self.lock.map(|l| l.kind).or(plane).map(|kind| match kind {
            SnapKind::Axis(Axis::X) => "Locked: red axis",
            SnapKind::Axis(Axis::Y) => "Locked: green axis",
            SnapKind::Axis(Axis::Z) => "Locked: blue axis",
            SnapKind::Parallel => "Locked: parallel",
            _ => "Locked: perpendicular",
        });
        Status {
            tool: self.tool.name(),
            prompt: self.tool.prompt().to_string(),
            label,
            value,
            message: self.message.as_ref().map(|m| m.text.clone()),
            lock,
        }
    }

    /// What a click would act on now (Select, Push/Pull, Paint, Erase), as of the last
    /// `refresh_hover`: an id of the open context. None while flying, with the cursor outside
    /// the view or under a window (`set_covered`), and for the other tools.
    pub fn hovered(&self) -> Option<EntityId> {
        if self.flying || self.hover_hidden || self.covered {
            return None;
        }
        self.tool.hovered()
    }

    /// The name of the snap under the cursor, e.g. "Endpoint" or "Midpoint in Group", as of
    /// the last `refresh_hover`. None in Select (which shows no marker), while flying, with
    /// the cursor outside the view or under a window (`set_covered`), and when there is no
    /// snap to name.
    pub fn snap_label(&self) -> Option<String> {
        if self.kind == ToolKind::Select || self.flying || self.covered {
            return None;
        }
        let hit = self.hit?;
        snap_label(hit.kind, self.within)
    }

    /// Whether a window, a menu or the gizmo lies over the cursor in the view, as the window
    /// works it out each frame. While it does, `hovered` and `snap_label` are None, as the
    /// window shows neither there. `tool_input` clears it: an agent's pointer is under no
    /// window.
    pub fn set_covered(&mut self, covered: bool) {
        self.covered = covered;
    }

    /// The triangles of the solid the tool previews (Push/Pull's pulled shape), as of the last
    /// `refresh_hover`, in root coordinates: each with its flat normal and the material's
    /// color (or the default material's front) at 70% opacity. Empty while flying.
    pub fn solid_preview(&self) -> Vec<PreviewVertex> {
        let Some(solid) = self.solid.as_ref().filter(|_| !self.flying) else {
            return Vec::new();
        };
        let [r, g, b] = solid
            .material
            .and_then(|m| self.doc.scene().material(m))
            .map_or(DEFAULT_FRONT, |m| m.color.map(|u| f32::from(u) / 255.0));
        let color = [r, g, b, SOLID_ALPHA];
        let t = self.doc.context_transform();
        solid
            .triangles
            .iter()
            .flat_map(|tri| {
                let [a, b, c] = tri.map(|p| t.transform_point3(p));
                let normal = (b - a)
                    .cross(c - a)
                    .normalize_or_zero()
                    .as_vec3()
                    .to_array();
                [a, b, c].map(|p| PreviewVertex {
                    pos: p.as_vec3().to_array(),
                    normal,
                    color,
                })
            })
            .collect()
    }

    /// The marker, preview lines and chip contents as of the last `refresh_hover`, in view
    /// pixels (the chip shown regardless of the rest; `Timing::dress` decides). Preview lines
    /// and halos take the colors of a dark canvas when `dark_canvas`. Empty while flying; no
    /// marker or chip in Select or with the cursor outside the view. The flash is the window's
    /// (`take_flash`).
    pub fn hud(&self, dark_canvas: bool) -> Hud {
        let mut hud = Hud {
            dark_canvas,
            ..Hud::default()
        };
        if self.flying {
            return hud;
        }
        // Tools work in the open context's coordinates; the view shows the root's.
        let t = self.doc.context_transform();
        let camera = &self.doc.camera;
        let preview: Vec<[DVec3; 2]> = self
            .preview
            .iter()
            .map(|[a, b]| [t.transform_point3(*a), t.transform_point3(*b)])
            .collect();
        let hit = self.hit.map(|h| Inference {
            point: t.transform_point3(h.point),
            ..h
        });
        hud.lines = preview_lines(
            &preview,
            hit.as_ref(),
            self.lock.is_some(),
            dark_canvas,
            camera,
        );
        if self.hover_hidden {
            return hud;
        }
        hud.cursor = Some(self.cursor);
        if self.kind != ToolKind::Select {
            hud.marker = hit.and_then(|h| {
                let (shape, color) = marker(h.kind)?;
                Some((camera.project(h.point)?, shape, color))
            });
        }
        let label = self.snap_label();
        let value = self.status_parts().value;
        if label.is_some() || value.is_some() {
            hud.chip = Some(Chip {
                label,
                value,
                typing: !self.typed.is_empty(),
            });
        }
        hud
    }

    /// The message of a typed value that just failed (the tool refused it or its command
    /// failed), once.
    pub fn take_flash(&mut self) -> Option<String> {
        self.flash.take()
    }

    /// Whether the status bar's message reports an error or informs; None with no message.
    pub fn message_kind(&self) -> Option<MessageKind> {
        if self.flying {
            return None;
        }
        self.message.as_ref().map(|m| m.kind)
    }

    /// Sends a click, release, typed value or cancel to the tool and runs what it asks for.
    fn send(&mut self, event: ToolEvent) {
        // The tool sees where the cursor is before anything else happens.
        self.flush_hover();
        let is_release = matches!(event, ToolEvent::Release(_));
        let is_value = matches!(event, ToolEvent::Value(_));
        let mut out = self.handle(event.clone());
        if matches!(out.commands.first(), Some(Command::Undo { .. }))
            && self.tool_step != Some(self.doc.revision())
        {
            // The step the tool would undo is not its own last one: the tool forgets it and
            // says, as for a value typed with nothing to change, what to do instead.
            self.tool.on_refused();
            out = self.handle(event);
            out.commands.clear();
        }
        // A release that did nothing keeps the message from the click that preceded it.
        let keep = is_release && out.commands.is_empty() && out.message.is_none();
        if !keep {
            self.message = out.message.clone().map(|text| Message {
                text,
                kind: if out.info {
                    MessageKind::Info
                } else {
                    MessageKind::Error
                },
            });
        }
        if let Some(sampled) = out.sampled {
            self.material = sampled;
            let name = sampled
                .and_then(|m| self.doc.scene().material(m))
                .map_or("the default material", |m| m.name.as_str());
            self.message = Some(Message::info(format!("Painting with {name}")));
        }
        self.apply(out);
        if is_value {
            self.flash = self
                .message
                .as_ref()
                .filter(|m| m.kind == MessageKind::Error)
                .map(|m| m.text.clone());
        }
        self.invalidate_hover();
    }

    /// The camera in the open context's coordinates. Placements turn and move, so inside a
    /// turned instance its up direction is no longer the context's Z.
    fn local_camera(&self) -> Camera {
        self.doc
            .camera
            .placed(self.doc.context_transform().inverse())
    }

    /// When the cursor stands for a model point an event gave (and still lies where `camera`,
    /// the local camera, shows it): the depths the tools look at, so the point is found even
    /// behind other geometry.
    fn point_slab(&self, camera: &Camera) -> Option<Slab> {
        let p = self.cursor_point?;
        let shown = camera.project(p)?;
        (shown.distance(self.cursor) < 1e-6)
            .then(|| Slab::around(camera, p, PICK_RADIUS_LOGICAL_PX * self.scale_factor))
    }

    /// Where the scene's meshes lie seen from the open context.
    fn layout(&self) -> Rc<Layout> {
        cached_layout(&self.layout, &self.doc)
    }

    fn handle(&mut self, event: ToolEvent) -> ToolOutput {
        let camera = self.local_camera();
        let layout = self.layout();
        let slab = self.point_slab(&camera);
        let scenery = seen_within(scenery_in(&layout, &self.doc), slab);
        let ctx = ToolContext {
            model: self.doc.model(),
            scenery: &scenery,
            camera: &camera,
            cursor: self.cursor,
            pick_radius: PICK_RADIUS_LOGICAL_PX * self.scale_factor,
            drag_threshold: DRAG_LOGICAL_PX * self.scale_factor,
            quick_press: self.quick_press && matches!(event, ToolEvent::Click(_)),
            shift: self.shift,
            selection: self.doc.selection(),
            material: self.current_material(),
            sample: self.sample,
        };
        self.tool.handle(event, &ctx)
    }

    /// Runs the tool's commands in order; a copy selects what it created, as in SketchUp.
    /// The first command that fails stops the rest, and so does a copy or rotation that changes
    /// nothing (a copy landing on existing geometry, a rotation of what lies on its axis): it
    /// leaves no undo step for an array or a retyped angle to replace. If it
    /// fails after an undo from the same list (an array replacing a copy), the undo is redone
    /// and the copy reselected; either way the tool drops its state.
    fn apply(&mut self, out: ToolOutput) {
        let start = self.doc.revision();
        let mut undone = false;
        let selected: Vec<EntityId> = self.doc.selection().iter().copied().collect();
        for command in out.commands {
            let is_undo = matches!(command, Command::Undo { .. });
            let is_copy = matches!(
                command,
                Command::Copy { .. }
                    | Command::Rotate {
                        copies: Some(_),
                        ..
                    }
            );
            let is_turn = matches!(command, Command::Rotate { copies: None, .. });
            let revision = self.doc.revision();
            let outcome = self.run(command);
            let unchanged =
                (is_copy || is_turn) && outcome.is_some() && self.doc.revision() == revision;
            if unchanged {
                self.message = Some(Message::error(if is_copy {
                    "Nothing new to copy"
                } else {
                    "Nothing turned: it lies on the axis"
                }));
            }
            let Some(outcome) = outcome.filter(|_| !unchanged) else {
                if undone && let Ok(redone) = self.execute_recorded(Command::Redo { count: None }) {
                    if is_copy && !redone.changes.created.is_empty() {
                        self.select_created(redone);
                    } else {
                        // Only a refused copy reselects its copies; anything else (a plain
                        // move or turn, a guide) keeps what was selected.
                        self.run(Command::Select {
                            ids: selected,
                            mode: SelectMode::Replace,
                            all: false,
                        });
                    }
                }
                self.reset_tool();
                return;
            };
            undone |= is_undo;
            if is_copy {
                self.select_created(outcome);
            }
        }
        // Only a step that changed the model is the tool's own (a selection is not).
        if self.doc.revision() != start {
            self.tool_step = Some(self.doc.revision());
        }
        self.preview = out.preview;
        self.solid = out.solid;
    }

    /// Selects the edges, faces, instances and guides a command created in the open context (a
    /// copied group's new contents live in its new definition and are left out).
    fn select_created(&mut self, outcome: Outcome) {
        let m = self.doc.model();
        let ids = outcome
            .changes
            .created
            .into_iter()
            .filter(|id| match *id {
                EntityId::Edge(e) => m.edge(e).is_some(),
                EntityId::Face(f) => m.face(f).is_some(),
                EntityId::Instance(i) => m.instance(i).is_some(),
                EntityId::Guide(g) => m.guide(g).is_some(),
                EntityId::Vertex(_) => false,
            })
            .collect();
        self.run(Command::Select {
            ids,
            mode: SelectMode::Replace,
            all: false,
        });
    }

    /// Drops the tool's operation in progress so it can't refer to geometry that changed.
    fn reset_tool(&mut self) {
        self.tool.on_external_change();
        self.preview.clear();
        self.solid = None;
    }

    /// Runs a command on the document, noting it and its result while `tool_input` runs.
    fn execute_recorded(&mut self, command: Command) -> Result<Outcome, CommandError> {
        let noted = self.issued.is_some().then(|| command.clone());
        let replaces = replaces_document(&command);
        let result = self.doc.execute(command);
        if replaces && result.is_ok() {
            self.documents += 1;
        }
        if let (Some(command), Some(issued)) = (noted, self.issued.as_mut()) {
            issued.push((command, result.clone()));
        }
        result
    }

    /// Runs a command; a failure becomes the status message.
    fn run(&mut self, command: Command) -> Option<Outcome> {
        match self.execute_recorded(command) {
            Ok(outcome) => Some(outcome),
            Err(e) if matches!(e.kind, ErrorKind::NothingToUndo | ErrorKind::NothingToRedo) => None,
            Err(e) => {
                self.message = Some(Message::error(e.message));
                None
            }
        }
    }

    /// Marks the hover out of date (see `hover_dirty`). With the cursor outside the view it
    /// stays hidden: only the cursor coming back (or input that acts where it is) shows it.
    fn invalidate_hover(&mut self) {
        self.hover_dirty = true;
    }

    /// Works the hover out if it is out of date. True if the marker or the preview changed.
    fn flush_hover(&mut self) -> bool {
        if !self.hover_dirty {
            return false;
        }
        self.update_hover()
    }

    /// Works the hover out now, for input that acts where the cursor is.
    fn hover_now(&mut self) -> bool {
        self.hover_hidden = false;
        self.update_hover()
    }

    /// Brings the inference marker, the tool preview and the status up to date for the frame
    /// about to be drawn. While navigation moves the camera (a drag, or scrolls and pinches
    /// arriving) the hover waits: the marker and the preview stay where they were in the
    /// model, and `hover_deferred` asks for another frame once navigation may have stopped.
    /// True if the marker or the preview changed.
    pub fn refresh_hover(&mut self) -> bool {
        if self.flying {
            return false;
        }
        if !self.hover_dirty || self.nav.dragging() || std::mem::take(&mut self.navigated) {
            return false;
        }
        self.flush_hover()
    }

    /// True when the hover is out of date (the next frame works it out).
    pub fn hover_pending(&self) -> bool {
        self.hover_dirty
    }

    /// True when `refresh_hover` put the hover off for navigation that has no drag holding it:
    /// a frame drawn a little later shows it if navigation stopped.
    pub fn hover_deferred(&self) -> bool {
        !self.flying && self.hover_dirty && !self.nav.dragging()
    }

    /// Works out the inference under the cursor and the tool preview. True if either changed.
    fn update_hover(&mut self) -> bool {
        self.hover_dirty = false;
        self.navigated = false;
        let anchor = self.tool.anchor();
        if anchor.is_none() {
            // Locks last for one operation.
            self.lock = None;
            self.shift_lock = false;
        }
        let radius = PICK_RADIUS_LOGICAL_PX * self.scale_factor;
        let camera = self.local_camera();
        let layout = self.layout();
        let slab = self.point_slab(&camera);
        let scenery = seen_within(scenery_in(&layout, &self.doc), slab);
        let guides = Guides {
            anchor,
            reference: self.reference,
            lock: self.lock,
        };
        let (hit, involved) = infer_detail(&scenery, &camera, self.cursor, &guides, radius);
        let within = involved
            .first()
            .map_or(Within::Open, |&(k, _)| match scenery.part(k).owner {
                Owner::Open => Within::Open,
                Owner::Outside => Within::Outside,
                Owner::Instance(i) => self.instance_within(i),
            });
        // The edge the cursor rests on (anywhere) becomes the reference for direction snaps.
        let reference = edge_under_in(&scenery, &camera, self.cursor, radius)
            .and_then(|(_, _, a, b)| (b - a).try_normalize());
        if reference.is_some() {
            self.reference = reference;
        }
        let ctx = ToolContext {
            model: self.doc.model(),
            scenery: &scenery,
            camera: &camera,
            cursor: self.cursor,
            pick_radius: radius,
            drag_threshold: DRAG_LOGICAL_PX * self.scale_factor,
            quick_press: false,
            shift: self.shift,
            selection: self.doc.selection(),
            material: self.current_material(),
            sample: self.sample,
        };
        let out = self.tool.handle(ToolEvent::Move(hit), &ctx);
        let hit = (!self.hover_hidden).then_some(hit);
        let changed = self.hit != hit
            || self.within != within
            || self.preview != out.preview
            || self.solid != out.solid;
        self.hit = hit;
        self.within = within;
        self.preview = out.preview;
        self.solid = out.solid;
        changed
    }

    /// Where geometry placed by instance `i` of the open context lies: in a group or in a
    /// component, by its definition's kind.
    fn instance_within(&self, i: InstanceId) -> Within {
        let kind = self
            .doc
            .model()
            .instance(i)
            .and_then(|inst| self.doc.scene().definition(inst.definition))
            .map(|d| d.kind);
        match kind {
            Some(DefinitionKind::Group) => Within::Group,
            Some(DefinitionKind::Component) => Within::Component,
            None => Within::Open,
        }
    }
}

/// Where the scene's meshes lie seen from the open context: the layout kept in `cache` while
/// the model (its revision) and the open context stay the same, else a new one, kept there.
fn cached_layout(cache: &LayoutCache, doc: &Document) -> Rc<Layout> {
    let mut cache = cache.borrow_mut();
    if let Some((revision, context, layout)) = cache.as_ref()
        && *revision == doc.revision()
        && context == doc.context()
    {
        return layout.clone();
    }
    let layout = Rc::new(Layout::of(doc.scene(), doc.context()).expect("the open context exists"));
    *cache = Some((doc.revision(), doc.context().to_vec(), layout.clone()));
    layout
}

/// The scene seen through `layout` (made for `doc`'s current model and open context).
fn scenery_in<'a>(layout: &'a Layout, doc: &'a Document) -> Scenery<'a> {
    Scenery::view(layout, doc.scene())
        .or_else(|| Scenery::of(doc.scene(), doc.context()))
        .expect("the open context exists")
}

/// `scenery`, seen only within `slab` when there is one.
fn seen_within(scenery: Scenery<'_>, slab: Option<Slab>) -> Scenery<'_> {
    match slab {
        Some(slab) => scenery.within(slab),
        None => scenery,
    }
}

/// The model point under `cursor` (view pixels) as `camera` (in root coordinates) sees it, in
/// root coordinates: where the cursor's ray meets the nearest face the tools can see in
/// `scenery` (the open context's, which `to_root` maps to the root's). Navigation centers on
/// it.
fn pick(scenery: &Scenery, to_root: DAffine3, camera: &Camera, cursor: DVec2) -> Option<DVec3> {
    let camera = camera.placed(to_root.inverse());
    face_under_in(scenery, &camera, cursor).map(|(_, _, p)| to_root.transform_point3(p))
}

/// Speed steps a key gives in fly mode: "[" and "-" slower, "]", "=" and "+" faster.
fn fly_speed_key(c: char) -> Option<i32> {
    match c {
        '[' | '-' | '_' => Some(-1),
        ']' | '=' | '+' => Some(1),
        _ => None,
    }
}

/// True for the tools Ctrl switches to copying.
fn copies(kind: ToolKind) -> bool {
    matches!(kind, ToolKind::Move | ToolKind::Rotate)
}

/// The status after exporting to `path`, leaving out `skipped` faces that could not be
/// triangulated.
fn export_message(path: &Path, skipped: usize) -> String {
    match skipped {
        0 => format!("Exported {}", path.display()),
        1 => format!(
            "Exported {} without 1 face that could not be triangulated",
            path.display()
        ),
        n => format!(
            "Exported {} without {n} faces that could not be triangulated",
            path.display()
        ),
    }
}

/// True for a path the import and open treat as a SketchUp file: one ending in `.skp`.
pub fn is_sketchup_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("skp"))
}

/// The file name of `path`, for the status.
fn display_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// ": 616 faces, 7 definitions, 12 instances; 2 faces skipped; with 3 warnings (see the
/// import report)": what a SketchUp file made, for the status. The warnings go to stderr.
fn sketchup_summary(report: &ImportReport) -> String {
    let count = |n: usize, one: &str| match n {
        1 => format!("1 {one}"),
        n => format!("{n} {one}s"),
    };
    let mut out = format!(
        ": {}, {}, {}",
        count(report.faces, "face"),
        count(report.definitions, "definition"),
        count(report.instances, "instance")
    );
    if report.skipped > 0 {
        out += &format!("; {} skipped", count(report.skipped, "face"));
    }
    if !report.warnings.is_empty() {
        for w in &report.warnings {
            eprintln!("import: {w}");
        }
        out += &format!(
            "; with {} (see the import report)",
            count(report.warnings.len(), "warning")
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use session::{Query, QueryResult};
    use tools::infer_in;
    use tools::overlay::{ENDPOINT, PARALLEL, Shape};

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.25)
    }

    /// What the next frame draws over the view, on a light canvas.
    fn shown(e: &mut Editor) -> Hud {
        e.refresh_hover();
        e.hud(false)
    }

    /// Neither a marker nor a preview line.
    fn blank(hud: &Hud) -> bool {
        hud.marker.is_none() && hud.lines.is_empty()
    }

    fn click_at(e: &mut Editor, p: DVec3) {
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        e.button(Button::Left, true);
        e.button(Button::Left, false);
    }

    fn stats(e: &Editor) -> (usize, usize) {
        match e.doc().query(&Query::Stats {}).unwrap() {
            QueryResult::Stats { edges, faces, .. } => (edges, faces),
            other => panic!("unexpected {other:?}"),
        }
    }

    fn square(e: &mut Editor) {
        for (x, y) in [(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0), (1.0, 1.0)] {
            click_at(e, DVec3::new(x, y, 0.0));
        }
    }

    #[test]
    fn the_editor_starts_in_select() {
        let e = editor();
        assert_eq!(e.tool_kind(), ToolKind::Select);
        assert_eq!(e.status_parts().tool, "Select");
    }

    #[test]
    fn clicking_four_corners_draws_a_face() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        assert_eq!(stats(&e), (4, 1));
        let model = e.doc().model();
        let (id, _) = model.faces().next().unwrap();
        assert!((model.area(id).unwrap() - 4.0).abs() < 1e-6);
    }

    #[test]
    fn undo_and_redo_step_through_edges() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.undo();
        assert_eq!(stats(&e), (3, 0));
        e.redo();
        assert_eq!(stats(&e), (4, 1));
    }

    #[test]
    fn preview_follows_the_cursor_until_cancelled() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        let px = e.doc().camera.project(DVec3::new(-2.0, 4.0, 0.0)).unwrap();
        assert!(e.cursor_moved(px));
        assert!(!shown(&mut e).lines.is_empty(), "preview line expected");
        e.key(EditorKey::Escape);
        assert!(
            blank(&shown(&mut e)),
            "ground hover has no marker and no preview after cancel"
        );
        assert_eq!(stats(&e), (0, 0));
    }

    #[test]
    fn undo_mid_chain_cancels_the_chain() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        for (x, y) in [(1.0, 1.0), (3.0, 1.0), (3.0, 3.0)] {
            click_at(&mut e, DVec3::new(x, y, 0.0));
        }
        e.undo();
        click_at(&mut e, DVec3::new(5.0, 5.0, 0.0));
        assert_eq!(stats(&e).0, 1, "the click only starts a new chain");
    }

    #[test]
    fn alt_left_drag_orbits_without_drawing() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        let before = e.doc().camera.clone();
        e.cursor_moved(DVec2::new(400.0, 300.0));
        e.set_modifiers(false, true);
        e.button(Button::Left, true);
        e.cursor_moved(DVec2::new(460.0, 300.0));
        e.button(Button::Left, false);
        assert_ne!(e.doc().camera, before);
        assert_eq!(stats(&e), (0, 0));
    }

    /// An editor with a 2 m square face floating 3 m above the ground, and a point on it.
    fn raised_face() -> (Editor, DVec3) {
        let mut e = editor();
        e.execute(Command::AddRectangle {
            corner: [-1.0, -1.0, 3.0],
            opposite: [1.0, 1.0, 3.0],
            normal: None,
        })
        .unwrap();
        (e, DVec3::new(0.5, 0.25, 3.0))
    }

    #[test]
    fn a_middle_drag_orbits_about_the_face_under_the_cursor() {
        let (mut e, p) = raised_face();
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        let d = e.doc().camera.eye.distance(p);
        e.button(Button::Middle, true);
        e.cursor_moved(px + DVec2::new(40.0, 25.0));
        e.button(Button::Middle, false);
        let cam = &e.doc().camera;
        assert!(
            (cam.eye.distance(p) - d).abs() < 1e-9,
            "orbits about the face point"
        );
        assert!(cam.project(p).unwrap().abs_diff_eq(px, 1e-6));
    }

    #[test]
    fn a_middle_drag_is_navigating() {
        let (mut e, p) = raised_face();
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        assert!(!e.navigating());
        e.button(Button::Middle, true);
        assert!(e.navigating(), "from the press");
        e.cursor_moved(px + DVec2::new(40.0, 25.0));
        assert!(e.navigating(), "while it moves");
        e.button(Button::Middle, false);
        assert!(!e.navigating(), "until the release");
    }

    #[test]
    fn scrolling_zooms_toward_the_face_under_the_cursor() {
        let (mut e, p) = raised_face();
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        let d = e.doc().camera.eye.distance(p);
        assert!(e.scroll(Scroll::Lines(DVec2::new(0.0, 1.0))));
        let cam = &e.doc().camera;
        assert!((cam.eye.distance(p) - d * session::camera::ZOOM_STEP).abs() < 1e-9);
        assert!(cam.project(p).unwrap().abs_diff_eq(px, 1e-6));
    }

    #[test]
    fn a_trackpad_scroll_orbits_and_a_pinch_zooms_about_the_face() {
        let (mut e, p) = raised_face();
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        let d = e.doc().camera.eye.distance(p);
        assert!(e.scroll(Scroll::Pixels(DVec2::new(30.0, 0.0))));
        assert!((e.doc().camera.eye.distance(p) - d).abs() < 1e-9);
        assert!(e.pinch(0.25));
        assert!((e.doc().camera.eye.distance(p) - d / 1.25).abs() < 1e-9);
        assert!(!e.pinch(f64::NAN));
    }

    #[test]
    fn command_trackpad_scroll_zooms_in_the_editor() {
        let (mut e, p) = raised_face();
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        e.set_sample_modifier(true);
        let forward = e.doc().camera.forward();
        assert!(e.scroll(Scroll::Pixels(DVec2::new(0.0, 125.0))));
        assert!(
            e.doc().camera.forward().abs_diff_eq(forward, 1e-12),
            "zoom, not orbit"
        );
        e.focus_lost();
        assert!(e.scroll(Scroll::Pixels(DVec2::new(0.0, 125.0))));
        assert!(
            !e.doc().camera.forward().abs_diff_eq(forward, 1e-9),
            "orbits again"
        );
    }

    #[test]
    fn the_editor_passes_the_trackpad_setting_on() {
        let (mut e, p) = raised_face();
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        e.set_trackpad(TrackpadScroll::Zoom);
        let d = e.doc().camera.eye.distance(p);
        let forward = e.doc().camera.forward();
        assert!(e.scroll(Scroll::Pixels(DVec2::new(0.0, 125.0))));
        assert!(e.doc().camera.eye.distance(p) < d, "zooms toward the face");
        assert!(e.doc().camera.forward().abs_diff_eq(forward, 1e-12));
        // Flying, a two-finger scroll still looks around.
        e.toggle_fly();
        let eye = e.doc().camera.eye;
        let (yaw, _) = fly::heading(&e.doc().camera);
        assert!(e.scroll(Scroll::Pixels(DVec2::new(50.0, 0.0))));
        let (turned, _) = fly::heading(&e.doc().camera);
        assert!(turned > yaw, "fingers right turn the view left");
        assert_eq!(e.doc().camera.eye, eye);
    }

    #[test]
    fn scrolling_through_a_face_carries_on_to_the_ground() {
        let (mut e, p) = raised_face();
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        let mut passed = false;
        for _ in 0..40 {
            assert!(e.scroll(Scroll::Lines(DVec2::new(0.0, 3.0))));
            passed |= e.doc().camera.eye.z < 3.0;
            if passed {
                break;
            }
        }
        assert!(passed, "the eye passed the face: {}", e.doc().camera.eye);
        let cam = &e.doc().camera;
        // The target moved on to the ground below, not a centimeter past the face, so the
        // next zooms and orbits work at that depth.
        assert!(cam.distance() > 1.0, "{}", cam.distance());
        assert!(cam.depth(cam.anchor_at(px)) >= cam.distance() - 1e-6);
        for _ in 0..30 {
            e.scroll(Scroll::Lines(DVec2::new(0.0, 3.0)));
            if e.doc().camera.eye.z < 0.02 {
                break;
            }
        }
        assert!(
            e.doc().camera.eye.z < 0.02,
            "reached the ground: {}",
            e.doc().camera.eye
        );
    }

    #[test]
    fn a_camera_command_drops_the_trackpad_pivot() {
        let (mut e, p) = raised_face();
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        assert!(e.scroll(Scroll::Pixels(DVec2::new(30.0, 0.0))));
        e.execute(Command::StandardView {
            view: StandardView::Top,
        })
        .unwrap();
        let fresh = e.doc().camera.anchor_for(px, None);
        let doc = e.doc();
        let scenery = Scenery::of(doc.scene(), doc.context()).unwrap();
        let hit = pick(&scenery, doc.context_transform(), &doc.camera, px).unwrap_or(fresh);
        let d = e.doc().camera.eye.distance(hit);
        assert!(e.scroll(Scroll::Pixels(DVec2::new(30.0, 0.0))));
        assert!((e.doc().camera.eye.distance(hit) - d).abs() < 1e-9);
    }

    #[test]
    fn losing_focus_cancels_a_pending_tool_press() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        let a = e.doc().camera.project(DVec3::new(1.0, 1.0, 0.0)).unwrap();
        let b = e.doc().camera.project(DVec3::new(3.0, 1.0, 0.0)).unwrap();
        e.cursor_moved(a);
        assert!(e.button(Button::Left, true));
        e.cursor_moved(b);
        assert!(e.focus_lost(), "the cancelled press changes the view");
        assert!(
            !e.button(Button::Left, false),
            "the release no longer reaches the tool"
        );
        assert_eq!(stats(&e), (0, 0));
        assert!(
            blank(&shown(&mut e)),
            "no line in progress: {:?}",
            shown(&mut e).lines
        );
    }

    #[test]
    fn hovering_an_endpoint_shows_a_marker() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        let px = e.doc().camera.project(DVec3::new(3.0, 1.0, 0.0)).unwrap();
        e.cursor_moved(px + DVec2::new(3.0, 0.0));
        let hud = shown(&mut e);
        let (at, shape, color) = hud.marker.expect("the endpoint's marker");
        assert_eq!((shape, color), (Shape::Dot, ENDPOINT));
        assert!(at.distance(px) < 1e-6, "on the endpoint: {at} vs {px}");
        assert_eq!(hud.lines, [], "no preview");
    }

    #[test]
    fn select_shows_no_marker_and_a_typed_error_flashes_once() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        let corner = DVec3::new(1.0, 1.0, 0.0);
        move_to(&mut e, corner);
        e.refresh_hover();
        assert_eq!(
            e.hud(false).marker.map(|m| (m.1, m.2)),
            Some((Shape::Dot, ENDPOINT))
        );
        e.select_tool(ToolKind::Select);
        move_to(&mut e, corner);
        e.refresh_hover();
        assert_eq!(e.hud(false).marker, None);
        e.select_tool(ToolKind::Rectangle);
        click_at(&mut e, DVec3::ZERO);
        type_text(&mut e, "2,x");
        let flash = e.take_flash().expect("the refused value");
        assert!(e.status().contains(&flash), "it stays in the status bar");
        assert_eq!(e.message_kind(), Some(MessageKind::Error));
        assert_eq!(e.take_flash(), None);
    }

    #[test]
    fn only_errors_flash() {
        let mut e = editor();
        e.select_tool(ToolKind::Circle);
        type_text(&mut e, "8s");
        assert!(e.status().contains("Circles get 8 sides"), "{}", e.status());
        assert_eq!(e.message_kind(), Some(MessageKind::Info));
        assert_eq!(e.take_flash(), None, "a side count is not an error");
        // An error from a click (not a typed value) stays in the status bar only.
        e.select_tool(ToolKind::Select);
        e.make_group();
        assert_eq!(e.message_kind(), Some(MessageKind::Error));
        assert_eq!(e.take_flash(), None);
        e.select_tool(ToolKind::Line);
        assert_eq!(e.message_kind(), None, "no message");
    }

    #[test]
    fn the_chip_names_the_snap_and_shows_the_value() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.select_tool(ToolKind::Line);
        move_to(&mut e, DVec3::new(3.0, 1.0, 0.0));
        let hud = shown(&mut e);
        let chip = hud.chip.expect("a snap to name");
        assert_eq!(chip.label.as_deref(), Some("Endpoint"));
        assert_eq!(
            (chip.value, chip.typing),
            (None, false),
            "nothing measured yet"
        );
        assert_eq!(hud.cursor, Some(e.cursor));
        click(&mut e);
        move_to(&mut e, DVec3::new(3.0, 2.0, 0.0));
        e.key(EditorKey::Char('2'));
        let chip = shown(&mut e).chip.expect("the typed text");
        assert_eq!((chip.value.as_deref(), chip.typing), (Some("2_"), true));
        // No chip with the cursor outside the view.
        e.cursor_left();
        assert_eq!(shown(&mut e).chip, None);
    }

    #[test]
    fn the_hover_stays_hidden_until_the_cursor_comes_back() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.select_tool(ToolKind::Select);
        let px = e.doc().camera.project(DVec3::new(2.0, 1.0, 0.0)).unwrap();
        e.cursor_moved(px);
        e.refresh_hover();
        assert!(e.hovered().is_some(), "the edge under the cursor");
        e.cursor_left();
        e.refresh_hover();
        assert_eq!(e.hovered(), None);
        // Neither a tool switch nor an agent's edit brings it back under the stale cursor.
        e.select_tool(ToolKind::Erase);
        e.refresh_hover();
        assert_eq!(e.hovered(), None, "after a tool switch");
        e.select_tool(ToolKind::Line);
        e.execute(Command::AddEdge {
            start: [5.0, 5.0, 0.0],
            end: [6.0, 5.0, 0.0],
        })
        .unwrap();
        let hud = shown(&mut e);
        assert_eq!((hud.marker, hud.cursor, hud.chip), (None, None, None));
        e.select_tool(ToolKind::Erase);
        e.refresh_hover();
        assert_eq!(e.hovered(), None, "after an agent's edit");
        // The cursor coming back shows it.
        e.cursor_moved(px);
        e.refresh_hover();
        assert!(e.hovered().is_some());
    }

    #[test]
    fn paint_samples_while_the_modifier_is_held() {
        let mut e = editor();
        assert!(!e.set_sample_modifier(true), "the Line tool paints nothing");
        assert!(!e.sampling());
        e.set_sample_modifier(false);
        e.select_tool(ToolKind::Paint);
        assert!(e.set_sample_modifier(true), "the highlight changes");
        assert!(e.sampling());
        assert!(!e.set_sample_modifier(true), "held already");
        assert!(e.set_sample_modifier(false));
        assert!(!e.sampling());
    }

    #[test]
    fn the_marker_lands_on_the_cursor_pixel_at_any_scale() {
        // The same window on a 2x display: twice the pixels, and the marker at twice the
        // view pixel; its size is in points (`ui::viewport`).
        let at = |scale: f64| {
            let mut e = Editor::new(DVec2::new(800.0, 600.0) * scale, scale);
            e.select_tool(ToolKind::Line);
            square(&mut e);
            move_to(&mut e, DVec3::new(3.0, 1.0, 0.0));
            shown(&mut e).marker.expect("the endpoint").0
        };
        let (one, two) = (at(1.0), at(2.0));
        assert!((two - one * 2.0).length() < 1e-6, "{one} {two}");
    }

    #[test]
    fn api_edit_mid_chain_cancels_the_chain() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        let out = e.execute(Command::AddEdge {
            start: [5.0, 5.0, 0.0],
            end: [6.0, 5.0, 0.0],
        });
        assert!(out.is_ok());
        click_at(&mut e, DVec3::new(-2.0, -2.0, 0.0));
        assert_eq!(
            stats(&e),
            (1, 0),
            "the click starts a new chain instead of adding an edge"
        );
    }

    #[test]
    fn api_camera_move_keeps_the_chain() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        e.execute(Command::SetCamera {
            eye: [-6.0, -10.0, 6.0],
            target: [0.0, 0.0, 0.0],
            fov_y_deg: None,
            up: None,
            projection: None,
        })
        .unwrap();
        click_at(&mut e, DVec3::new(3.0, 1.0, 0.0));
        assert_eq!(stats(&e), (1, 0), "the chain continued");
    }

    fn move_to(e: &mut Editor, p: DVec3) {
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
    }

    fn click(e: &mut Editor) {
        e.button(Button::Left, true);
        e.button(Button::Left, false);
    }

    fn type_text(e: &mut Editor, text: &str) {
        for c in text.chars() {
            e.key(EditorKey::Char(c));
        }
        e.key(EditorKey::Enter);
    }

    #[test]
    fn typing_shows_in_the_status_and_escape_clears_it_first() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        e.key(EditorKey::Char('2'));
        e.key(EditorKey::Char('.'));
        e.key(EditorKey::Char('5'));
        e.key(EditorKey::Backspace);
        assert!(e.status().contains("Length: 2._"), "{}", e.status());
        e.key(EditorKey::Escape);
        assert!(!e.status().contains('_'));
        assert!(e.status().contains("Click the next point"), "chain kept");
        e.key(EditorKey::Escape);
        assert!(e.status().contains("Click to start a line"));
    }

    #[test]
    fn typed_length_draws_the_next_edge() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        move_to(&mut e, DVec3::new(4.0, 1.0, 0.0));
        type_text(&mut e, "2");
        assert_eq!(stats(&e), (1, 0));
        let end = DVec3::new(3.0, 1.0, 0.0);
        let model = e.doc().model();
        assert!(model.vertices().any(|(_, v)| v.pos.abs_diff_eq(end, 1e-9)));
    }

    #[test]
    fn letters_are_shortcuts_only_when_nothing_is_typed() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        assert!(
            e.key(EditorKey::Char('l')),
            "selecting Line again restarts it"
        );
        assert!(e.status().contains("Click to start a line"));
        assert!(!e.key(EditorKey::Char('z')));
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        e.key(EditorKey::Char('1'));
        e.key(EditorKey::Char('m'));
        assert!(e.status().contains("Length: 1m_"), "{}", e.status());
    }

    #[test]
    fn rectangle_tool_draws_one_undo_step_per_rectangle() {
        let mut e = editor();
        assert!(e.key(EditorKey::Char('r')));
        assert!(e.status().starts_with("Rectangle"), "{}", e.status());
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        assert_eq!(stats(&e), (4, 1));
        e.undo();
        assert_eq!(stats(&e), (0, 0));
    }

    #[test]
    fn typed_size_finishes_a_rectangle() {
        let mut e = editor();
        e.key(EditorKey::Char('r'));
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        move_to(&mut e, DVec3::new(1.0, 1.0, 0.0));
        type_text(&mut e, "3,2");
        let model = e.doc().model();
        let (face, _) = model.faces().next().unwrap();
        assert!((model.area(face).unwrap() - 6.0).abs() < 1e-9);
    }

    /// Draws a 2 m square on the ground with the Rectangle tool and selects Push/Pull.
    fn square_then_push_pull(e: &mut Editor) {
        e.key(EditorKey::Char('r'));
        click_at(e, DVec3::new(0.0, 0.0, 0.0));
        click_at(e, DVec3::new(2.0, 2.0, 0.0));
        e.key(EditorKey::Char('p'));
        assert!(e.status().starts_with("Push/Pull"), "{}", e.status());
    }

    #[test]
    fn push_pull_turns_a_rectangle_into_a_box() {
        let mut e = editor();
        square_then_push_pull(&mut e);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        move_to(&mut e, DVec3::new(1.0, 1.0, 1.5));
        assert!(e.status().contains("Distance: 1.5 m"), "{}", e.status());
        click(&mut e);
        assert_eq!(stats(&e), (12, 6));
        e.undo();
        assert_eq!(stats(&e), (4, 1));
    }

    #[test]
    fn typed_distance_finishes_a_push_pull() {
        let mut e = editor();
        square_then_push_pull(&mut e);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        move_to(&mut e, DVec3::new(1.0, 1.0, 0.5));
        type_text(&mut e, "750mm");
        assert_eq!(stats(&e), (12, 6));
        let top = e
            .doc()
            .model()
            .vertices()
            .map(|(_, v)| v.pos.z)
            .fold(0.0, f64::max);
        assert!((top - 0.75).abs() < 1e-9);
    }

    #[test]
    fn failed_tool_commands_show_why() {
        let mut e = editor();
        square_then_push_pull(&mut e);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        type_text(&mut e, "1");
        assert_eq!(stats(&e), (12, 6));
        // Pushing the top 3 m down would turn the 1 m sides inside out.
        click_at(&mut e, DVec3::new(1.0, 1.0, 1.0));
        type_text(&mut e, "-3");
        assert_eq!(stats(&e), (12, 6));
        assert!(e.status().contains("collapse"), "{}", e.status());
    }

    #[test]
    fn failed_clicks_keep_their_message_after_release() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.key(EditorKey::Char('p'));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        type_text(&mut e, "1");
        click_at(&mut e, DVec3::new(2.0, 2.0, 1.0));
        click_at(&mut e, DVec3::new(2.0, 2.0, -3.0));
        assert!(e.status().contains("collapse"), "{}", e.status());
    }

    #[test]
    fn a_drawing_drag_released_outside_the_view_keeps_its_first_point() {
        let mut e = editor();
        e.select_tool(ToolKind::Rectangle);
        move_to(&mut e, DVec3::ZERO);
        e.button(Button::Left, true);
        // Released over the dock, left of the view.
        e.cursor_moved(DVec2::new(-60.0, 300.0));
        e.cursor_left();
        e.button(Button::Left, false);
        assert_eq!(stats(&e), (0, 0));
        assert_eq!(
            e.phase(),
            tools::Phase::Active {
                points: vec![DVec3::ZERO]
            }
        );
        // In the view, the same drag draws.
        e.key(EditorKey::Escape);
        move_to(&mut e, DVec3::ZERO);
        e.button(Button::Left, true);
        move_to(&mut e, DVec3::new(2.0, 1.0, 0.0));
        e.button(Button::Left, false);
        assert_eq!(stats(&e), (4, 1));
    }

    /// The top of the model: the highest vertex.
    fn top(e: &Editor) -> f64 {
        e.doc()
            .model()
            .vertices()
            .map(|(_, v)| v.pos.z)
            .fold(0.0, f64::max)
    }

    #[test]
    fn a_retyped_pull_redoes_only_its_own_step() {
        let mut e = editor();
        assert!(e.add_material());
        let material = e.current_material().unwrap();
        square_then_push_pull(&mut e);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        type_text(&mut e, "1");
        type_text(&mut e, "2");
        assert_eq!(stats(&e), (12, 6));
        assert!((top(&e) - 2.0).abs() < 1e-9, "the pull was redone");
        // A material change after the pull: a typed value no longer undoes anything.
        e.recolor_material(material, [1, 2, 3]);
        type_text(&mut e, "3");
        assert_eq!(e.doc().scene().material(material).unwrap().color, [1, 2, 3]);
        assert!((top(&e) - 2.0).abs() < 1e-9);
        assert_eq!(stats(&e), (12, 6));
        assert_eq!(
            e.status_parts().message.as_deref(),
            Some("Click a face first")
        );
    }

    #[test]
    fn a_retyped_scale_redoes_only_its_own_step() {
        let mut e = editor();
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        })
        .unwrap();
        assert!(e.select_all());
        e.select_tool(ToolKind::Scale);
        // The square's corners at x = 4 (after a scale by 2).
        let at_four = |e: &Editor| {
            e.doc()
                .model()
                .vertices()
                .filter(|(_, v)| (v.pos.x - 4.0).abs() < 1e-9)
                .count()
        };
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        type_text(&mut e, "2");
        assert_eq!(at_four(&e), 2);
        // Another command after the scale: a typed value no longer undoes anything.
        e.execute(Command::AddEdge {
            start: [6.0, 0.0, 0.0],
            end: [6.0, 1.0, 0.0],
        })
        .unwrap();
        let before = stats(&e);
        type_text(&mut e, "3");
        assert_eq!(stats(&e), before, "the edge stays");
        assert_eq!(at_four(&e), 2, "the scale stays");
        assert_eq!(
            e.status_parts().message.as_deref(),
            Some("Click a grip first")
        );
    }

    #[test]
    fn a_refused_retype_keeps_the_double_click_distance() {
        let mut e = editor();
        assert!(e.add_material());
        let material = e.current_material().unwrap();
        square_then_push_pull(&mut e);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        type_text(&mut e, "2");
        e.recolor_material(material, [1, 2, 3]);
        // Refused: no pull ran at 3, so a double-click still repeats 2.
        type_text(&mut e, "3");
        assert!((top(&e) - 2.0).abs() < 1e-9);
        double_click_at(&mut e, DVec3::new(1.0, 1.0, 2.0));
        assert!((top(&e) - 4.0).abs() < 1e-9, "top {}", top(&e));
    }

    #[test]
    fn only_a_quick_press_near_the_first_click_leaves_a_pull_unfinished() {
        for quick in [true, false] {
            let mut e = editor();
            square_then_push_pull(&mut e);
            click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
            // 5 px higher, within twice the drag threshold of the first release.
            let px = e.doc().camera.project(DVec3::new(1.0, 1.0, 0.0)).unwrap();
            e.cursor_moved(px - DVec2::new(0.0, 5.0));
            e.quick_press(quick);
            e.button(Button::Left, true);
            e.button(Button::Left, false);
            if quick {
                assert_eq!(stats(&e), (4, 1), "a double-click's second press");
                assert!(matches!(e.phase(), tools::Phase::Active { .. }));
            } else {
                assert_eq!(stats(&e), (12, 6), "a late click finishes the pull");
                assert_eq!(e.phase(), tools::Phase::Idle);
            }
        }
    }

    #[test]
    fn an_agent_edit_after_a_retype_keeps_the_retyped_distance() {
        let mut e = editor();
        square_then_push_pull(&mut e);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        type_text(&mut e, "2");
        type_text(&mut e, "3");
        assert!((top(&e) - 3.0).abs() < 1e-9);
        // Straight after, with no tool event between: the retyped pull stands.
        e.execute(Command::AddEdge {
            start: [5.0, 5.0, 0.0],
            end: [6.0, 5.0, 0.0],
        })
        .unwrap();
        double_click_at(&mut e, DVec3::new(1.0, 1.0, 3.0));
        assert!((top(&e) - 6.0).abs() < 1e-9, "top {}", top(&e));
    }

    #[test]
    fn an_array_after_another_step_undoes_nothing() {
        let mut e = editor();
        assert!(e.add_material());
        let material = e.current_material().unwrap();
        e.select_tool(ToolKind::Line);
        start_copy(&mut e);
        type_text(&mut e, "3");
        assert_eq!(stats(&e), (8, 2));
        e.recolor_material(material, [1, 2, 3]);
        type_text(&mut e, "3x");
        assert_eq!(e.doc().scene().material(material).unwrap().color, [1, 2, 3]);
        assert_eq!(stats(&e), (8, 2), "nothing was undone or copied");
        assert_eq!(
            e.status_parts().message.as_deref(),
            Some("Make a copy first, then type 5x or /5")
        );
    }

    #[test]
    fn api_edit_ends_a_push_pull_in_progress() {
        let mut e = editor();
        square_then_push_pull(&mut e);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        assert!(e.status().contains("Distance"), "{}", e.status());
        e.execute(Command::AddEdge {
            start: [5.0, 5.0, 0.0],
            end: [6.0, 5.0, 0.0],
        })
        .unwrap();
        assert!(e.status().contains("Click a face"), "{}", e.status());
    }

    #[test]
    fn leaving_the_window_hides_the_marker() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        let px = e.doc().camera.project(DVec3::new(3.0, 1.0, 0.0)).unwrap();
        e.cursor_moved(px);
        assert!(!blank(&shown(&mut e)));
        assert!(e.cursor_left());
        assert!(blank(&shown(&mut e)));
        // Leaving before any frame showed the move hides it too.
        e.cursor_moved(px + DVec2::new(1.0, 0.0));
        assert!(e.cursor_left());
        assert!(blank(&shown(&mut e)));
        // Coming back shows it again.
        assert!(e.cursor_moved(px));
        assert!(!blank(&shown(&mut e)));
    }

    #[test]
    fn the_hover_is_worked_out_once_per_frame() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        let px = e.doc().camera.project(DVec3::new(3.0, 1.0, 0.0)).unwrap();
        for k in 0..5 {
            assert!(e.cursor_moved(px + DVec2::new(k as f64, 0.0)));
            assert!(e.hover_pending());
        }
        assert!(e.refresh_hover(), "the marker appears");
        assert!(!e.hover_pending());
        assert!(!e.refresh_hover(), "nothing left to work out");
        assert_eq!(
            e.hud(false).marker.map(|m| m.1),
            Some(Shape::Dot),
            "the endpoint marker"
        );
        assert!(!e.cursor_moved(px + DVec2::new(4.0, 0.0)), "the same spot");
        assert!(!e.hover_pending());
    }

    #[test]
    fn api_commands_leave_the_hover_to_the_next_frame() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        let end = DVec3::new(2.0, 1.0, 0.0);
        e.cursor_moved(e.doc().camera.project(end).unwrap());
        e.refresh_hover();
        assert_eq!(e.hit.map(|h| h.kind), Some(SnapKind::Ground));
        e.execute(Command::AddEdge {
            start: [0.0, 1.0, 0.0],
            end: end.to_array(),
        })
        .unwrap();
        assert!(e.hover_pending());
        assert_eq!(e.hit.map(|h| h.kind), Some(SnapKind::Ground));
        e.refresh_hover();
        assert_eq!(
            e.hit.map(|h| (h.kind, h.point)),
            Some((SnapKind::Endpoint, end))
        );
    }

    #[test]
    fn navigation_puts_the_hover_off_until_it_stops() {
        let (mut e, p) = raised_face();
        e.select_tool(ToolKind::Line);
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        e.refresh_hover();
        let before = e.hit;

        // A drag holds the hover off; its release lets the next frame show it.
        e.button(Button::Middle, true);
        assert!(e.cursor_moved(px + DVec2::new(30.0, 10.0)));
        assert!(!e.refresh_hover());
        assert!(e.hover_pending() && !e.hover_deferred());
        assert_eq!(e.hit, before, "the marker stays where it was in the model");
        assert!(e.button(Button::Middle, false), "the release redraws");
        assert!(e.refresh_hover());
        assert!(!e.hover_pending());

        // Scrolls put it off for one frame, which asks for another; with no more scrolling
        // that one shows it.
        assert!(e.scroll(Scroll::Lines(DVec2::new(0.0, 1.0))));
        assert!(!e.refresh_hover());
        assert!(e.hover_deferred());
        assert!(e.scroll(Scroll::Lines(DVec2::new(0.0, 1.0))));
        assert!(!e.refresh_hover());
        assert!(e.hover_deferred());
        e.refresh_hover();
        assert!(!e.hover_pending());

        // Input that acts where the cursor is does not wait: a click right after a zoom and a
        // move, with no frame between, starts the line at what is under the cursor now.
        e.select_tool(ToolKind::Line);
        e.refresh_hover();
        let stale = e.hit.unwrap().point;
        assert!(e.scroll(Scroll::Lines(DVec2::new(0.0, 1.0))));
        e.cursor_moved(e.cursor + DVec2::new(-25.0, 15.0));
        let scenery = Scenery::of(e.doc().scene(), &[]).unwrap();
        let radius = PICK_RADIUS_LOGICAL_PX * e.scale_factor;
        let now = infer_in(
            &scenery,
            &e.doc().camera,
            e.cursor,
            &Guides::default(),
            radius,
        )
        .point;
        assert!(now.distance(stale) > 1e-3);
        click(&mut e);
        assert_eq!(e.tool.anchor(), Some(now));
    }

    #[test]
    fn select_tool_clicks_pick_and_delete_erases_them() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        assert!(e.key(EditorKey::Char(' ')));
        assert!(e.status().starts_with("Select"), "{}", e.status());
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        assert_eq!(e.doc().selection().len(), 1);
        assert!(e.key(EditorKey::Delete));
        assert_eq!(stats(&e), (4, 0));
        assert!(e.doc().selection().is_empty());
        e.undo();
        assert_eq!(stats(&e), (4, 1));
    }

    #[test]
    fn lines_snap_to_guides_and_select_picks_and_erases_them() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        e.execute(Command::AddGuidePoint {
            position: [1.0, 1.0, 0.0],
        })
        .unwrap();
        e.execute(Command::AddGuideLine {
            point: [0.0, 3.0, 0.0],
            direction: [1.0, 0.0, 0.0],
        })
        .unwrap();
        let point = DVec3::new(1.0, 1.0, 0.0);
        e.cursor_moved(e.doc().camera.project(point).unwrap() + DVec2::new(2.0, 1.0));
        e.refresh_hover();
        assert_eq!(
            e.hit.map(|h| (h.point, h.kind)),
            Some((point, SnapKind::GuidePoint))
        );
        // A line drawn from the guide point to the guide line ends on it.
        click(&mut e);
        click_at(&mut e, DVec3::new(1.0, 3.0, 0.0));
        e.key(EditorKey::Escape);
        let ends = edge_ends(&e);
        assert_eq!(ends.len(), 1);
        assert_eq!(ends[0][0], point);
        assert!(
            ends[0][1].abs_diff_eq(DVec3::new(1.0, 3.0, 0.0), 1e-9),
            "the line ends on the guide line: {ends:?}"
        );

        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(-2.0, 3.0, 0.0));
        assert_eq!(
            e.doc().selection().iter().copied().collect::<Vec<_>>(),
            vec!["g1".parse().unwrap()]
        );
        assert!(e.key(EditorKey::Delete));
        assert_eq!(e.doc().model().guides().count(), 1);
    }

    #[test]
    fn t_measures_from_an_edge_to_a_parallel_guide_and_ctrl_measures_only() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        assert!(e.key(EditorKey::Char('t')));
        assert_eq!(e.tool_kind(), ToolKind::TapeMeasure);
        // From the bottom side of the square, 1.5 m out (the guide follows the cursor's side).
        click_at(&mut e, DVec3::new(2.5, 1.0, 0.0));
        move_to(&mut e, DVec3::new(2.5, -0.5, 0.0));
        assert!(e.status().contains("Length: 1.5 m"), "{}", e.status());
        type_text(&mut e, "2");
        let guides: Vec<kernel::Guide> = e.doc().model().guides().map(|(_, g)| *g).collect();
        let [kernel::Guide::Line { point, direction }] = guides[..] else {
            panic!("{guides:?}");
        };
        assert!((point.y + 1.0).abs() < 1e-9, "{point:?}");
        assert_eq!(direction, DVec3::X);
        // Retyping moves that guide: one undo step replaced by another.
        type_text(&mut e, "3");
        let lines: Vec<f64> = e
            .doc()
            .model()
            .guides()
            .map(|(_, g)| g.anchor().y)
            .collect();
        assert_eq!(lines.len(), 1);
        assert!((lines[0] + 2.0).abs() < 1e-9, "{lines:?}");

        assert!(e.toggle_copy());
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        click_at(&mut e, DVec3::new(3.0, 3.0, 0.0));
        assert!(e.status().contains("Distance: 2.828 m"), "{}", e.status());
        assert_eq!(
            e.doc().model().guides().count(),
            1,
            "no guide when measuring"
        );

        assert!(e.delete_guides());
        assert_eq!(e.doc().model().guides().count(), 0);
        assert!(!e.delete_guides());
        e.undo();
        assert_eq!(e.doc().model().guides().count(), 1);
    }

    #[test]
    fn a_refused_retyped_guide_distance_keeps_the_guide_and_the_selection() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        let face = e
            .doc()
            .model()
            .faces()
            .next()
            .map(|(id, _)| EntityId::Face(id))
            .unwrap();
        e.run(Command::Select {
            ids: vec![face],
            mode: SelectMode::Replace,
            all: false,
        });
        assert!(e.key(EditorKey::Char('t')));
        click_at(&mut e, DVec3::new(2.5, 1.0, 0.0));
        move_to(&mut e, DVec3::new(2.5, -0.5, 0.0));
        type_text(&mut e, "2");
        let guide = |e: &Editor| {
            e.doc()
                .model()
                .guides()
                .map(|(_, g)| *g)
                .collect::<Vec<_>>()
        };
        let before = guide(&e);
        assert_eq!(before.len(), 1);
        let selected: Vec<EntityId> = e.doc().selection().iter().copied().collect();
        type_text(&mut e, "2000000");
        assert!(e.status().contains("within +/-1000000 m"), "{}", e.status());
        assert_eq!(guide(&e), before);
        let after: Vec<EntityId> = e.doc().selection().iter().copied().collect();
        assert_eq!(after, selected);
    }

    #[test]
    fn backspace_erases_the_selection_only_when_nothing_is_typed() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        e.key(EditorKey::Char('1'));
        assert!(e.key(EditorKey::Backspace));
        assert_eq!(
            stats(&e),
            (4, 1),
            "the first backspace deletes the typed digit"
        );
        assert!(e.key(EditorKey::Backspace));
        assert_eq!(stats(&e), (4, 0));
        assert!(!e.key(EditorKey::Delete), "nothing left to erase");
    }

    #[test]
    fn repeated_backspace_and_delete_never_erase_the_selection() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        e.key(EditorKey::Char('1'));
        e.key(EditorKey::Char('2'));
        assert!(e.key_repeat(EditorKey::Backspace));
        assert!(e.key_repeat(EditorKey::Backspace));
        assert!(
            !e.key_repeat(EditorKey::Backspace),
            "the text is already empty"
        );
        assert_eq!(stats(&e), (4, 1));
        assert_eq!(e.doc().selection().len(), 1);
        assert!(!e.key_repeat(EditorKey::Delete));
        assert_eq!(stats(&e), (4, 1));
    }

    #[test]
    fn held_letter_shortcuts_act_once() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        assert!(e.key(EditorKey::Char('g')));
        assert_eq!(e.doc().model().instances().count(), 1);
        let camera = e.doc().camera.clone();
        for c in ['g', 'G', 'Z', 'l'] {
            assert!(!e.key_repeat(EditorKey::Char(c)), "{c}");
        }
        assert_eq!(
            e.doc().model().instances().count(),
            1,
            "no group of the group"
        );
        assert_eq!(e.doc().camera, camera, "no zoom at the repeat rate");
        assert_eq!(e.tool_kind(), ToolKind::Select);
        // Typed text still repeats.
        e.key(EditorKey::Char('1'));
        assert!(e.key_repeat(EditorKey::Char('1')));
        assert!(e.status().contains("11_"), "{}", e.status());
    }

    #[test]
    fn erasing_the_selection_clears_typed_text() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        e.key(EditorKey::Char('5'));
        assert!(e.key(EditorKey::Delete));
        assert!(!e.status().contains("5_"), "{}", e.status());
    }

    #[test]
    fn losing_focus_releases_shift() {
        let mut e = editor();
        e.set_modifiers(true, false);
        e.focus_lost();
        assert!(!e.shift);
    }

    #[test]
    fn dragging_a_box_selects_what_is_inside() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.key(EditorKey::Char(' '));
        let corners: Vec<DVec2> = [(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0)]
            .iter()
            .map(|&(x, y)| e.doc().camera.project(DVec3::new(x, y, 0.0)).unwrap())
            .collect();
        let lo = corners.iter().fold(DVec2::INFINITY, |a, p| a.min(*p)) - 10.0;
        let hi = corners.iter().fold(DVec2::NEG_INFINITY, |a, p| a.max(*p)) + 10.0;
        e.cursor_moved(lo);
        e.button(Button::Left, true);
        e.cursor_moved(hi);
        assert_eq!(
            shown(&mut e).lines.len(),
            4,
            "the box is drawn while dragging"
        );
        e.button(Button::Left, false);
        assert_eq!(e.doc().selection().len(), 5, "four edges and the face");
    }

    #[test]
    fn erase_tool_click_erases_an_edge_and_its_face() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        assert!(e.key(EditorKey::Char('e')));
        click_at(&mut e, DVec3::new(3.0, 2.0, 0.0));
        assert_eq!(stats(&e), (3, 0));
    }

    #[test]
    fn move_tool_moves_the_selection_by_a_typed_distance() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        assert!(e.key(EditorKey::Char('m')));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        move_to(&mut e, DVec3::new(2.0, 4.0, 0.0));
        type_text(&mut e, "2");
        assert_eq!(stats(&e), (4, 1));
        let moved = DVec3::new(1.0, 3.0, 0.0);
        let model = e.doc().model();
        assert!(
            model
                .vertices()
                .any(|(_, v)| v.pos.abs_diff_eq(moved, 1e-6))
        );
    }

    #[test]
    fn copies_are_selected_and_arrays_replace_them() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        e.key(EditorKey::Char('m'));
        assert!(e.toggle_copy());
        assert!(e.status().contains("Copy"), "{}", e.status());
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        move_to(&mut e, DVec3::new(5.0, 2.0, 0.0));
        type_text(&mut e, "3");
        assert_eq!(stats(&e), (8, 2));
        assert_eq!(
            e.doc().selection().len(),
            5,
            "the copy's four edges and face"
        );
        type_text(&mut e, "3x");
        assert_eq!(stats(&e), (16, 4));
        e.undo();
        assert_eq!(stats(&e), (4, 1), "the array is one undo step");
    }

    /// A square, selected, with the Move tool in copy mode and the base point picked.
    fn start_copy(e: &mut Editor) {
        square(e);
        e.key(EditorKey::Char(' '));
        click_at(e, DVec3::new(2.0, 2.0, 0.0));
        e.key(EditorKey::Char('m'));
        e.toggle_copy();
        click_at(e, DVec3::new(2.0, 2.0, 0.0));
        move_to(e, DVec3::new(5.0, 2.0, 0.0));
    }

    #[test]
    fn a_rejected_array_keeps_the_copy_and_disarms_the_array() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        start_copy(&mut e);
        type_text(&mut e, "50000");
        assert_eq!(stats(&e), (8, 2));
        // 30 copies 50 km apart pass the tool's count limit but leave the coordinate limit.
        type_text(&mut e, "30x");
        assert_eq!(stats(&e), (8, 2), "the copy is restored");
        assert!(!e.status_parts().message.unwrap_or_default().is_empty());
        assert_eq!(
            e.doc().selection().len(),
            5,
            "the restored copy is selected"
        );
        let model = e.doc().model();
        assert!(e.doc().selection().iter().all(|id| {
            match *id {
                EntityId::Edge(edge) => model
                    .edge(edge)
                    .unwrap()
                    .v
                    .iter()
                    .all(|v| model.position(*v).unwrap().x > 3.5),
                EntityId::Face(f) => model.face(f).unwrap().outer.len() == 4,
                EntityId::Vertex(_) | EntityId::Instance(_) | EntityId::Guide(_) => false,
            }
        }));
        // Typing another array must not undo anything.
        type_text(&mut e, "3x");
        assert_eq!(stats(&e), (8, 2));
        e.undo();
        assert_eq!(stats(&e), (4, 1), "the copy is still one undo step");
    }

    #[test]
    fn a_copy_that_changes_nothing_does_not_arm_an_array() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 1.0, 0.0));
        let selected = e.doc().selection().clone();
        assert_eq!(selected.len(), 1, "the bottom edge");
        e.key(EditorKey::Char('m'));
        e.toggle_copy();
        // Copy the bottom edge onto the top edge, which is already there.
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        click_at(&mut e, DVec3::new(1.0, 3.0, 0.0));
        assert_eq!(stats(&e), (4, 1));
        assert_eq!(
            e.status_parts().message.as_deref(),
            Some("Nothing new to copy")
        );
        type_text(&mut e, "3x");
        assert_eq!(stats(&e), (4, 1), "the last edge drawn was not undone");
        assert_eq!(e.doc().selection(), &selected);
        e.undo();
        assert_eq!(stats(&e), (3, 0), "the square's last edge is the last step");
    }

    #[test]
    fn typed_arrays_can_start_with_a_symbol_or_x() {
        for (distance, array) in [("6", "/3"), ("2", "*3"), ("2", "x3")] {
            let mut e = editor();
            e.select_tool(ToolKind::Line);
            start_copy(&mut e);
            type_text(&mut e, distance);
            assert_eq!(stats(&e).1, 2, "{array}");
            type_text(&mut e, array);
            assert_eq!(stats(&e), (13, 4), "{array}: four squares in a row");
        }
    }

    #[test]
    fn x_starts_typing_only_in_the_move_tool() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        assert!(!e.key(EditorKey::Char('x')));
        assert!(!e.status().contains("x_"), "{}", e.status());
        e.key(EditorKey::Char('m'));
        assert!(e.key(EditorKey::Char('x')));
        assert!(e.status().contains("x_"), "{}", e.status());
    }

    #[test]
    fn ctrl_switches_the_polygon_and_scale_tools() {
        let mut e = editor();
        for kind in [ToolKind::Polygon, ToolKind::Scale] {
            e.select_tool(kind);
            assert!(e.toggle_copy(), "{kind:?}");
        }
        assert!(
            e.status().contains("Scaling about the center"),
            "{}",
            e.status()
        );
    }

    #[test]
    fn ctrl_outside_the_move_tool_keeps_the_message() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        type_text(&mut e, "2");
        let message = e.status_parts().message;
        assert!(message.is_some(), "the line tool needs a start point");
        assert!(!e.toggle_copy());
        assert_eq!(e.status_parts().message, message);
    }

    #[test]
    fn a_rejected_copy_does_not_arm_an_array() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        start_copy(&mut e);
        type_text(&mut e, "2000000");
        assert_eq!(stats(&e), (4, 1), "the copy was rejected");
        assert!(!e.status_parts().message.unwrap_or_default().is_empty());
        type_text(&mut e, "5x");
        assert_eq!(stats(&e), (4, 1), "nothing was undone or copied");
        assert_eq!(e.doc().model().faces().count(), 1);
    }

    #[test]
    fn a_release_without_its_press_is_not_sent_to_the_tool() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.key(EditorKey::Char(' '));
        assert!(!e.button(Button::Left, false));
        assert!(e.doc().selection().is_empty());
    }

    /// The edge from the last click's chain: both ends of every edge, as positions.
    fn edge_ends(e: &Editor) -> Vec<[DVec3; 2]> {
        let m = e.doc().model();
        m.edges()
            .map(|(_, ed)| [m.position(ed.v[0]).unwrap(), m.position(ed.v[1]).unwrap()])
            .collect()
    }

    #[test]
    fn shift_locks_the_direction_under_the_cursor() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        let up = e.doc().camera.project(DVec3::new(1.0, 1.0, 2.5)).unwrap();
        e.cursor_moved(up + DVec2::new(4.0, 0.0));
        assert!(e.set_modifiers(true, false));
        assert!(e.status().contains("Locked: blue axis"), "{}", e.status());
        click_at(&mut e, DVec3::new(4.0, 4.0, 0.0));
        let [a, b] = edge_ends(&e)[0];
        let near = |v: f64, w: f64| (v - w).abs() < 1e-9;
        let vertical = near(a.x, b.x) && near(a.y, b.y) && near(a.x, 1.0) && near(a.y, 1.0);
        assert!(vertical && (a.z - b.z).abs() > 0.5, "{a} {b}");
        assert!(e.set_modifiers(false, false), "releasing Shift unlocks");
        assert!(!e.status().contains("Locked"));
    }

    #[test]
    fn arrow_keys_lock_an_axis_during_an_operation() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        assert!(!e.key(EditorKey::LockAxis(Axis::X)), "nothing to lock yet");
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        assert!(e.key(EditorKey::LockAxis(Axis::X)));
        assert!(e.status().contains("Locked: red axis"));
        click_at(&mut e, DVec3::new(3.0, 4.0, 0.0));
        let [a, b] = edge_ends(&e)[0];
        let near = |v: f64, w: f64| (v - w).abs() < 1e-9;
        let along_red = near(a.y, b.y) && near(a.z, 0.0) && near(b.z, 0.0);
        assert!(along_red && (a.x - b.x).abs() > 0.1, "{a} {b}");
        assert!(e.key(EditorKey::LockAxis(Axis::X)), "the same key unlocks");
        assert!(!e.status().contains("Locked"));
        e.key(EditorKey::LockAxis(Axis::Y));
        assert!(e.key(EditorKey::Unlock));
        e.key(EditorKey::Escape);
        assert!(
            !e.key(EditorKey::Unlock),
            "the chain ended, so nothing is locked"
        );
    }

    #[test]
    fn hovering_an_edge_teaches_parallel_snaps() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        e.execute(Command::AddEdge {
            start: [0.0, 0.0, 0.0],
            end: [2.0, 1.0, 0.0],
        })
        .unwrap();
        let on_edge = e.doc().camera.project(DVec3::new(1.5, 0.75, 0.0)).unwrap();
        e.cursor_moved(on_edge + DVec2::new(0.0, 2.0));
        // A frame is drawn while the cursor rests on the edge.
        shown(&mut e);
        click_at(&mut e, DVec3::new(0.0, 3.0, 0.0));
        let along = DVec3::new(0.0, 3.0, 0.0) + DVec3::new(2.0, 1.0, 0.0).normalize() * 2.0;
        e.cursor_moved(e.doc().camera.project(along).unwrap());
        assert_eq!(shown(&mut e).lines[0].color, PARALLEL);
    }

    #[test]
    fn held_arrow_key_does_not_toggle_the_lock() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        assert!(e.key(EditorKey::LockAxis(Axis::X)));
        for _ in 0..3 {
            assert!(!e.key_repeat(EditorKey::LockAxis(Axis::X)));
        }
        assert!(e.status().contains("Locked: red axis"), "{}", e.status());
        assert!(!e.key_repeat(EditorKey::Unlock));
        assert!(e.status().contains("Locked: red axis"));
    }

    /// A 2 x 2 x 1 box at the origin, grouped and moved by (10, 0, 0). Returns the instance.
    fn moved_group(e: &mut Editor) -> EntityId {
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        })
        .unwrap();
        e.execute(Command::PushPull {
            face: "f8".parse().unwrap(),
            distance: 1.0,
        })
        .unwrap();
        let m = e.doc().model();
        let ids = m
            .edges()
            .map(|(id, _)| EntityId::Edge(id))
            .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
            .collect();
        e.execute(Command::MakeGroup { ids }).unwrap();
        let group = *e.doc().selection().first().unwrap();
        e.execute(Command::Move {
            ids: vec![group],
            offset: [10.0, 0.0, 0.0],
        })
        .unwrap();
        group
    }

    /// The box of `moved_group`, turned a quarter turn about X (its top, z = 1, now faces -Y
    /// at y = -1) and then moved by (10, 0, 0). Returns the instance.
    fn turned_group(e: &mut Editor) -> EntityId {
        let group = moved_group(e);
        e.execute(Command::Rotate {
            ids: vec![group],
            center: [10.0, 0.0, 0.0],
            axis: Some([1.0, 0.0, 0.0]),
            angle: 90.0,
            copies: None,
        })
        .unwrap();
        group
    }

    /// True if the open context has an edge between these two points.
    fn has_edge(e: &Editor, a: DVec3, b: DVec3) -> bool {
        let m = e.doc().model();
        m.edges().any(|(_, edge)| {
            let (p, q) = (
                m.position(edge.v[0]).unwrap(),
                m.position(edge.v[1]).unwrap(),
            );
            (p.abs_diff_eq(a, 1e-9) && q.abs_diff_eq(b, 1e-9))
                || (p.abs_diff_eq(b, 1e-9) && q.abs_diff_eq(a, 1e-9))
        })
    }

    #[test]
    fn tools_inside_a_moved_group_work_in_its_coordinates() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.execute(Command::OpenInstance { id: group }).unwrap();
        e.select_tool(ToolKind::Line);
        click_at(&mut e, DVec3::new(10.0, 0.0, 1.0));
        click_at(&mut e, DVec3::new(12.0, 2.0, 1.0));
        e.key(EditorKey::Escape);
        assert!(
            has_edge(&e, DVec3::new(0.0, 0.0, 1.0), DVec3::new(2.0, 2.0, 1.0)),
            "the diagonal is drawn in the group's own coordinates"
        );
        assert_eq!(stats(&e), (13, 7), "it splits the group's top");
    }

    #[test]
    fn tools_inside_a_turned_group_work_in_its_coordinates() {
        let mut e = editor();
        let group = turned_group(&mut e);
        e.execute(Command::OpenInstance { id: group }).unwrap();
        e.select_tool(ToolKind::Line);
        // The group's top corners (0, 0, 1) and (2, 2, 1), where they are shown.
        click_at(&mut e, DVec3::new(10.0, -1.0, 0.0));
        click_at(&mut e, DVec3::new(12.0, -1.0, 2.0));
        e.key(EditorKey::Escape);
        assert!(
            has_edge(&e, DVec3::new(0.0, 0.0, 1.0), DVec3::new(2.0, 2.0, 1.0)),
            "the diagonal is drawn in the group's own coordinates"
        );
        assert_eq!(stats(&e), (13, 7), "it splits the group's top");
    }

    #[test]
    fn a_turned_group_is_picked_and_boxed_where_it_is_shown() {
        let mut e = editor();
        let group = turned_group(&mut e);
        clear_selection(&mut e);
        e.select_tool(ToolKind::Select);
        click_at(&mut e, DVec3::new(11.0, -1.0, 1.0));
        assert_eq!(
            e.doc().selection().iter().copied().collect::<Vec<_>>(),
            [group]
        );
        assert_eq!(box_min(&e, group), DVec3::new(10.0, -1.0, 0.0));
        assert_eq!(box_max(&e, group), DVec3::new(12.0, 0.0, 2.0));
    }

    #[test]
    fn the_marker_follows_the_cursor_inside_a_moved_group() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.execute(Command::OpenInstance { id: group }).unwrap();
        e.select_tool(ToolKind::Line);
        let corner = DVec3::new(10.0, 0.0, 1.0);
        let px = e.doc().camera.project(corner).unwrap();
        e.cursor_moved(px + DVec2::new(3.0, 2.0));
        let (at, shape, _) = shown(&mut e).marker.expect("the endpoint marker");
        assert_eq!(shape, Shape::Dot);
        assert!(
            at.distance(px) < 0.5,
            "the endpoint marker is drawn at the corner in root coordinates: {at} vs {px}"
        );
    }

    #[test]
    fn lines_from_the_root_snap_to_a_groups_corner() {
        let mut e = editor();
        moved_group(&mut e);
        e.select_tool(ToolKind::Line);
        let corner = DVec3::new(10.0, 0.0, 0.0);
        let px = e.doc().camera.project(corner).unwrap();
        e.cursor_moved(px + DVec2::new(3.0, -2.0));
        e.button(Button::Left, true);
        e.button(Button::Left, false);
        click_at(&mut e, DVec3::new(10.0, -3.0, 0.0));
        e.key(EditorKey::Escape);
        assert!(has_edge(&e, corner, DVec3::new(10.0, -3.0, 0.0)));
    }

    fn clear_selection(e: &mut Editor) {
        e.execute(Command::Select {
            ids: Vec::new(),
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
    }

    fn double_click_at(e: &mut Editor, p: DVec3) {
        click_at(e, p);
        click_at(e, p);
        e.double_click();
    }

    /// The minimum corner of an instance's bounding box in the open context.
    fn box_min(e: &Editor, id: EntityId) -> DVec3 {
        match e.doc().query(&Query::Entity { id }).unwrap() {
            QueryResult::Instance(info) => DVec3::from(info.bounds.unwrap().min),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// The maximum corner of an instance's bounding box in the open context.
    fn box_max(e: &Editor, id: EntityId) -> DVec3 {
        match e.doc().query(&Query::Entity { id }).unwrap() {
            QueryResult::Instance(info) => DVec3::from(info.bounds.unwrap().max),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn clicking_a_group_selects_it() {
        let mut e = editor();
        let group = moved_group(&mut e);
        clear_selection(&mut e);
        e.select_tool(ToolKind::Select);
        click_at(&mut e, DVec3::new(11.0, 1.0, 1.0));
        assert_eq!(
            e.doc().selection().iter().copied().collect::<Vec<_>>(),
            [group]
        );
    }

    #[test]
    fn a_triple_click_after_opening_a_group_selects_only_the_click() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.select_tool(ToolKind::Select);
        let top = DVec3::new(11.0, 1.0, 1.0);
        double_click_at(&mut e, top);
        click_at(&mut e, top);
        e.triple_click();
        let open: Vec<EntityId> = e
            .doc()
            .context()
            .iter()
            .map(|i| EntityId::Instance(*i))
            .collect();
        assert_eq!(open, [group]);
        let selected: Vec<EntityId> = e.doc().selection().iter().copied().collect();
        assert!(
            matches!(selected[..], [EntityId::Face(_)]),
            "the face clicked, not what is connected to it: {selected:?}"
        );
    }

    #[test]
    fn only_a_step_that_changed_the_model_is_the_tools_own() {
        let mut e = editor();
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        })
        .unwrap();
        e.select_tool(ToolKind::Select);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        assert_eq!(e.doc().selection().len(), 1);
        assert_eq!(e.tool_step, None, "a selection is not a step");
        e.select_tool(ToolKind::PushPull);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        e.key(EditorKey::Char('1'));
        e.key(EditorKey::Enter);
        assert_eq!(e.tool_step, Some(e.doc().revision()));
    }

    #[test]
    fn double_clicking_a_group_opens_it_and_escape_closes_it() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.select_tool(ToolKind::Select);
        double_click_at(&mut e, DVec3::new(11.0, 1.0, 1.0));
        let open: Vec<EntityId> = e
            .doc()
            .context()
            .iter()
            .map(|i| EntityId::Instance(*i))
            .collect();
        assert_eq!(open, [group]);
        e.key(EditorKey::Escape);
        assert!(e.doc().context().is_empty());
    }

    #[test]
    fn shifted_letters_keep_their_meanings() {
        let mut e = editor();
        assert!(e.key(EditorKey::Char('L')));
        assert_eq!(e.tool_kind(), ToolKind::Line);
        assert!(e.key(EditorKey::Char('E')));
        assert_eq!(e.tool_kind(), ToolKind::Erase);
        assert!(e.key(EditorKey::Char('F')));
        assert!(e.flying(), "Shift+F flies as f does");
    }

    /// The editor key the table's plain `chord` arrives as.
    fn table_key(bound: crate::shortcuts::Bound) -> EditorKey {
        use crate::shortcuts::{Key, chord};
        let chord = chord(bound).expect("bound to a key");
        assert!(!chord.command, "{bound:?} is a plain key");
        match chord.key {
            Key::Letter(c) if chord.shift => EditorKey::Char(c.to_ascii_uppercase()),
            Key::Letter(c) => EditorKey::Char(c),
            Key::Space => EditorKey::Char(' '),
            Key::Delete => EditorKey::Delete,
            Key::Escape => EditorKey::Escape,
            // The window's key path acts on F2 (Rename Component); the editor has no key for it.
            Key::F2 => panic!("{bound:?} is the window's, not the editor's"),
            Key::Comma => panic!("{bound:?} is a Cmd chord"),
        }
    }

    #[test]
    fn the_editor_does_what_the_table_says_of_delete_and_escape() {
        use crate::shortcuts::Bound;
        // Erase Selection's key erases a selected edge.
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        e.execute(Command::AddEdge {
            start: [0.0, 0.0, 0.0],
            end: [1.0, 0.0, 0.0],
        })
        .unwrap();
        let ids: Vec<EntityId> = e
            .doc()
            .model()
            .edges()
            .map(|(id, _)| EntityId::Edge(id))
            .collect();
        e.execute(Command::Select {
            ids,
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
        assert!(e.key(table_key(Bound::EraseSelection)));
        assert_eq!(e.doc().model().edges().count(), 0);
        // Close Group's key closes the open group in the Select tool.
        let mut e = editor();
        let group = moved_group(&mut e);
        e.execute(Command::OpenInstance { id: group }).unwrap();
        e.select_tool(ToolKind::Select);
        assert!(!e.doc().context().is_empty());
        assert!(e.key(table_key(Bound::CloseInstance)));
        assert!(e.doc().context().is_empty());
    }

    #[test]
    fn the_editor_does_what_the_table_says_of_plain_letters() {
        use crate::shortcuts::Bound;
        let mut e = editor();
        for kind in ToolKind::ALL {
            e.select_tool(if kind == ToolKind::Line {
                ToolKind::Move
            } else {
                ToolKind::Line
            });
            assert!(e.key(table_key(Bound::Tool(kind))));
            assert_eq!(e.tool_kind(), kind);
        }
        // Make Group and Make Component's keys say what they need with nothing selected.
        e.key(table_key(Bound::MakeGroup));
        assert_eq!(
            e.message.as_ref().map(|m| m.text.as_str()),
            Some("Select geometry to make a group")
        );
        e.key(table_key(Bound::MakeComponent));
        assert_eq!(
            e.message.as_ref().map(|m| m.text.as_str()),
            Some("Select geometry to make a component")
        );
        assert!(e.key(table_key(Bound::ToggleFly)));
        assert!(e.flying());
    }

    #[test]
    fn double_clicking_outside_the_open_group_closes_it() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.execute(Command::OpenInstance { id: group }).unwrap();
        e.select_tool(ToolKind::Select);
        double_click_at(&mut e, DVec3::new(-3.0, -3.0, 0.0));
        assert!(e.doc().context().is_empty());
    }

    #[test]
    fn a_window_around_a_group_selects_it() {
        let mut e = editor();
        let group = moved_group(&mut e);
        clear_selection(&mut e);
        e.select_tool(ToolKind::Select);
        let corners: Vec<DVec2> = [0.0, 2.0]
            .into_iter()
            .flat_map(|x| [0.0, 2.0].map(move |y| (x, y)))
            .flat_map(|(x, y)| [0.0, 1.0].map(move |z| DVec3::new(10.0 + x, y, z)))
            .map(|p| e.doc().camera.project(p).unwrap())
            .collect();
        let lo = corners.iter().fold(corners[0], |a, b| a.min(*b)) - DVec2::splat(20.0);
        let hi = corners.iter().fold(corners[0], |a, b| a.max(*b)) + DVec2::splat(20.0);
        e.cursor_moved(lo);
        e.button(Button::Left, true);
        e.cursor_moved(hi);
        e.button(Button::Left, false);
        assert_eq!(
            e.doc().selection().iter().copied().collect::<Vec<_>>(),
            [group]
        );
    }

    #[test]
    fn the_move_tool_moves_a_selected_group() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.select_tool(ToolKind::Move);
        click_at(&mut e, DVec3::new(10.0, 0.0, 0.0));
        let px = e.doc().camera.project(DVec3::new(10.0, 3.0, 0.0)).unwrap();
        e.cursor_moved(px);
        for c in "5".chars() {
            e.key(EditorKey::Char(c));
        }
        e.key(EditorKey::Enter);
        assert!(
            box_min(&e, group).abs_diff_eq(DVec3::new(10.0, 5.0, 0.0), 1e-6),
            "{:?}",
            box_min(&e, group)
        );
    }

    #[test]
    fn q_turns_a_selected_group_by_a_typed_angle() {
        let mut e = editor();
        let group = moved_group(&mut e);
        assert!(e.key(EditorKey::Char('q')));
        assert!(e.status().starts_with("Rotate"), "{}", e.status());
        click_at(&mut e, DVec3::new(10.0, 0.0, 0.0));
        type_text(&mut e, "90");
        assert_eq!(box_min(&e, group), DVec3::new(8.0, 0.0, 0.0));
        assert_eq!(box_max(&e, group), DVec3::new(10.0, 2.0, 1.0));
        type_text(&mut e, "-90");
        assert_eq!(box_min(&e, group), DVec3::new(10.0, -2.0, 0.0));
        e.execute(Command::Undo {
            count: None,
            expect: None,
        })
        .unwrap();
        assert_eq!(
            box_min(&e, group),
            DVec3::new(10.0, 0.0, 0.0),
            "one undo step"
        );
    }

    #[test]
    fn a_rotation_that_turns_nothing_is_not_replaced_by_a_retyped_angle() {
        let mut e = editor();
        e.execute(Command::AddEdge {
            start: [5.0, 5.0, 0.0],
            end: [5.0, 5.0, 1.0],
        })
        .unwrap();
        let edge = e.doc().model().edges().next().unwrap().0;
        e.execute(Command::Select {
            ids: vec![EntityId::Edge(edge)],
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
        // An unrelated later step, which a retyped angle's undo would take back.
        e.execute(Command::AddEdge {
            start: [-5.0, 0.0, 0.0],
            end: [-4.0, 0.0, 0.0],
        })
        .unwrap();
        e.select_tool(ToolKind::Rotate);
        click_at(&mut e, DVec3::new(5.0, 5.0, 0.0));
        type_text(&mut e, "90");
        assert!(e.status().contains("Nothing turned"), "{}", e.status());
        type_text(&mut e, "45");
        assert_eq!(stats(&e), (2, 0), "the later edge was not undone");
    }

    #[test]
    fn a_refused_retyped_angle_keeps_the_selection() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        let face = EntityId::Face(e.doc().model().faces().next().unwrap().0);
        e.execute(Command::Select {
            ids: vec![face],
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
        e.select_tool(ToolKind::Rotate);
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        type_text(&mut e, "30");
        type_text(&mut e, "360");
        assert!(
            e.status().contains("whole number of turns"),
            "{}",
            e.status()
        );
        assert_eq!(
            e.doc().selection().iter().copied().collect::<Vec<_>>(),
            vec![face],
            "the selection survives the refused angle"
        );
    }

    /// Every edge and face in the open context.
    fn context_contents(e: &Editor) -> Vec<EntityId> {
        let m = e.doc().model();
        m.edges()
            .map(|(id, _)| EntityId::Edge(id))
            .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
            .collect()
    }

    #[test]
    fn tools_inside_a_nested_turned_group_work_in_the_inner_groups_coordinates() {
        let mut e = editor();
        let outer = turned_group(&mut e);
        e.execute(Command::OpenInstance { id: outer }).unwrap();
        let ids = context_contents(&e);
        e.execute(Command::MakeGroup { ids }).unwrap();
        let inner = *e.doc().selection().first().unwrap();
        // The inner group's (x, y) go to (2 - y, x) in the outer one, and so to (12 - y, -z, x)
        // in the world.
        e.execute(Command::Rotate {
            ids: vec![inner],
            center: [1.0, 1.0, 0.0],
            axis: Some([0.0, 0.0, 1.0]),
            angle: 90.0,
            copies: None,
        })
        .unwrap();
        e.execute(Command::OpenInstance { id: inner }).unwrap();
        e.select_tool(ToolKind::Line);
        click_at(&mut e, DVec3::new(12.0, -1.0, 0.0));
        click_at(&mut e, DVec3::new(10.0, -1.0, 2.0));
        e.key(EditorKey::Escape);
        assert!(
            has_edge(&e, DVec3::new(0.0, 0.0, 1.0), DVec3::new(2.0, 2.0, 1.0)),
            "the diagonal is drawn in the inner group's own coordinates"
        );
        assert_eq!(stats(&e), (13, 7), "it splits the inner group's top");
    }

    #[test]
    fn rotating_inside_a_turned_group_uses_the_context_coordinates_and_the_faces_normal() {
        let mut e = editor();
        let group = turned_group(&mut e);
        e.execute(Command::OpenInstance { id: group }).unwrap();
        let ids = context_contents(&e);
        e.execute(Command::Select {
            ids,
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
        e.select_tool(ToolKind::Rotate);
        // The middle of the group's top (1, 1, 1), where it is shown; its normal is the
        // group's own Z, which points along -Y in the world.
        click_at(&mut e, DVec3::new(11.0, -1.0, 1.0));
        type_text(&mut e, "30");
        let (s, c) = 30f64.to_radians().sin_cos();
        let turn = |x: f64, y: f64| {
            DVec3::new(
                1.0 + (x - 1.0) * c - (y - 1.0) * s,
                1.0 + (x - 1.0) * s + (y - 1.0) * c,
                1.0,
            )
        };
        assert!(
            has_edge(&e, turn(0.0, 0.0), turn(2.0, 0.0)),
            "the box turned about the vertical line through (1, 1) in its own coordinates"
        );
        assert_eq!(stats(&e), (12, 6));
    }

    #[test]
    fn rotated_copies_select_the_copies_and_arrays_replace_them() {
        let mut e = editor();
        moved_group(&mut e);
        e.select_tool(ToolKind::Rotate);
        assert!(e.toggle_copy());
        click_at(&mut e, DVec3::new(10.0, 0.0, 0.0));
        type_text(&mut e, "90");
        let instances = |e: &Editor| e.doc().model().instances().count();
        assert_eq!(instances(&e), 2);
        let copy = *e.doc().selection().first().unwrap();
        assert_eq!(box_min(&e, copy), DVec3::new(8.0, 0.0, 0.0));
        type_text(&mut e, "x3");
        assert_eq!(instances(&e), 4, "a full turn of copies");
        assert_eq!(e.doc().selection().len(), 3);
    }

    #[test]
    fn copying_a_group_selects_the_copy() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.select_tool(ToolKind::Move);
        e.toggle_copy();
        click_at(&mut e, DVec3::new(10.0, 0.0, 0.0));
        let px = e.doc().camera.project(DVec3::new(10.0, 3.0, 0.0)).unwrap();
        e.cursor_moved(px);
        for c in "4".chars() {
            e.key(EditorKey::Char(c));
        }
        e.key(EditorKey::Enter);
        let selected: Vec<EntityId> = e.doc().selection().iter().copied().collect();
        assert_eq!(selected.len(), 1);
        assert_ne!(selected[0], group);
        assert!(matches!(selected[0], EntityId::Instance(_)));
        assert_eq!(e.status_parts().message, None);
        assert!(box_min(&e, selected[0]).abs_diff_eq(DVec3::new(10.0, 4.0, 0.0), 1e-6));
    }

    /// Names of the definitions in the model, in id order.
    fn definition_names(e: &Editor) -> Vec<String> {
        e.doc()
            .scene()
            .definitions()
            .map(|(_, d)| d.name.clone())
            .collect()
    }

    #[test]
    fn shift_z_zooms_to_the_model_and_keeps_the_line_going() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        click_at(&mut e, DVec3::new(5.0, 1.0, 0.0));
        let before = stats(&e);
        assert!(e.key(EditorKey::Char('Z')));
        let target = e.doc().camera.target;
        assert!(
            target.abs_diff_eq(DVec3::new(2.0, 2.0, 0.0), 1e-9),
            "{target}"
        );
        assert_eq!(stats(&e), before, "a camera move is not an edit");
        click_at(&mut e, DVec3::new(5.0, 3.0, 0.0));
        assert_eq!(stats(&e).0, before.0 + 1, "the chain went on from (5, 1)");
    }

    #[test]
    fn standard_views_and_projection_come_from_the_editor() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        assert!(e.standard_view(StandardView::Top));
        assert!(e.doc().camera.is_vertical());
        assert!(e.set_projection(Projection::Parallel));
        assert!(e.doc().camera.is_parallel());
        // Drawing still lands where the cursor points in a parallel top view.
        square(&mut e);
        assert_eq!(stats(&e), (4, 1));
        assert!(e.zoom_extents());
        assert!(
            e.doc()
                .camera
                .target
                .abs_diff_eq(DVec3::new(2.0, 2.0, 0.0), 1e-9)
        );
        assert!(!e.set_projection(Projection::Parallel), "already parallel");
        assert!(!e.standard_view(StandardView::Top), "already looking down");
        assert!(e.standard_view(StandardView::Front));
    }

    #[test]
    fn night_lighting_comes_from_the_editor_and_is_not_an_edit() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        let before = stats(&e);
        let revision = e.doc().revision();
        assert!(e.set_lighting(Lighting::Night));
        assert_eq!(e.doc().lighting, Lighting::Night);
        assert!(!e.set_lighting(Lighting::Night), "already night");
        assert_eq!(e.doc().revision(), revision);
        e.undo();
        assert_eq!(stats(&e).0, before.0 - 1, "undo skips the lighting");
        assert_eq!(e.doc().lighting, Lighting::Night);
        assert!(e.set_lighting(Lighting::Day));
    }

    #[test]
    fn zoom_extents_says_whether_the_view_changed() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        e.cursor_moved(DVec2::new(10.0, 10.0));
        assert!(!e.zoom_extents(), "nothing to fit");
        square(&mut e);
        assert!(e.zoom_extents());
        assert!(!e.zoom_extents(), "already fitted");
    }

    #[test]
    fn g_and_shift_g_group_the_selection_and_explode_takes_it_apart() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        assert!(e.key(EditorKey::Char('g')), "the message needs a redraw");
        assert!(e.status().contains("Select geometry to make a group"));
        square(&mut e);
        e.select_tool(ToolKind::Select);
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        assert!(e.key(EditorKey::Char('g')));
        let group = *e.doc().selection().first().unwrap();
        assert!(matches!(group, EntityId::Instance(_)));
        assert_eq!(definition_names(&e), [""]);
        assert!(e.key(EditorKey::Char('G')));
        assert!(e.key(EditorKey::Char('G')));
        assert_eq!(
            definition_names(&e),
            ["", "Component 1", "Component 2"],
            "each component gets the next free name"
        );
        assert!(e.explode_selection());
        assert!(e.explode_selection());
        assert!(e.explode_selection());
        assert_eq!(
            definition_names(&e).len(),
            3,
            "the components stay, and so does the group inside Component 1"
        );
        assert_eq!(stats(&e), (4, 1), "the square is back in the root");
        assert!(e.explode_selection(), "the message needs a redraw");
        assert!(
            e.status()
                .contains("Select a group or component to explode")
        );
        assert_eq!(
            stats(&e),
            (4, 1),
            "the square's edges and face are no group"
        );
    }

    /// `moved_group` made a "Box" component, copied 5 m along X. Returns (first component,
    /// second component, the group inside both).
    fn two_boxes(e: &mut Editor) -> (EntityId, EntityId, EntityId) {
        let group = moved_group(e);
        e.execute(Command::MakeComponent {
            ids: vec![group],
            name: "Box".into(),
        })
        .unwrap();
        let first = *e.doc().selection().first().unwrap();
        let out = e
            .execute(Command::Copy {
                ids: vec![first],
                offset: [5.0, 0.0, 0.0],
                count: None,
            })
            .unwrap();
        let second = *out.changes.created.first().unwrap();
        (first, second, group)
    }

    fn context_ids(e: &Editor) -> Vec<EntityId> {
        e.doc()
            .context()
            .iter()
            .map(|i| EntityId::Instance(*i))
            .collect()
    }

    fn selected(e: &Editor) -> Vec<EntityId> {
        e.doc().selection().iter().copied().collect()
    }

    #[test]
    fn outliner_clicks_open_the_nodes_context_and_select_it() {
        let mut e = editor();
        let (first, second, group) = two_boxes(&mut e);
        assert!(e.outliner_select(&[second, group], false));
        assert_eq!(context_ids(&e), [second]);
        assert_eq!(selected(&e), [group]);
        assert!(e.outliner_select(&[first], false));
        assert!(context_ids(&e).is_empty());
        assert_eq!(selected(&e), [first]);
        // Shift or Cmd adds and removes nodes of the open context.
        assert!(e.outliner_select(&[second], true));
        assert_eq!(selected(&e), [first, second]);
        assert!(e.outliner_select(&[first], true));
        assert_eq!(selected(&e), [second]);
        // A node elsewhere is selected alone, in its own context.
        assert!(e.outliner_select(&[first, group], true));
        assert_eq!(context_ids(&e), [first]);
        assert_eq!(selected(&e), [group]);
        assert!(!e.outliner_select(&[], false));
    }

    #[test]
    fn outliner_double_clicks_open_the_node_and_end_the_tools_operation() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        let (_, second, group) = two_boxes(&mut e);
        click_at(&mut e, DVec3::new(0.0, -3.0, 0.0));
        assert!(e.tool.anchor().is_some(), "a line is started");
        assert!(e.open_path(vec![second, group]));
        assert!(e.tool.anchor().is_none());
        assert_eq!(context_ids(&e), [second, group]);
        assert!(selected(&e).is_empty());
        assert!(e.open_path(Vec::new()));
        assert!(context_ids(&e).is_empty());
    }

    #[test]
    fn a_stale_outliner_path_shows_why_and_keeps_the_context() {
        let mut e = editor();
        let (first, _, group) = two_boxes(&mut e);
        e.execute(Command::OpenInstance { id: first }).unwrap();
        let gone: EntityId = "i999".parse().unwrap();
        assert!(e.outliner_select(&[gone, group], false));
        assert!(e.status().contains("no instance i999"), "{}", e.status());
        assert_eq!(context_ids(&e), [first]);
    }

    #[test]
    fn close_instance_goes_back_out() {
        let mut e = editor();
        assert!(!e.close_instance());
        let group = moved_group(&mut e);
        e.execute(Command::OpenInstance { id: group }).unwrap();
        assert!(e.close_instance());
        assert!(e.doc().context().is_empty());
    }

    /// Clicks along the 2-long edge at y = 0, z = 1 of the moved group's box, from x = 10 to 12.
    fn click_along_top_front_edge(e: &mut Editor) -> Vec<Vec<EntityId>> {
        (1..20)
            .map(|k| {
                clear_selection(e);
                click_at(e, DVec3::new(10.0 + 2.0 * k as f64 / 20.0, 0.0, 1.0));
                e.doc().selection().iter().copied().collect()
            })
            .collect()
    }

    #[test]
    fn an_outside_edge_on_the_open_edge_never_steals_the_pick() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.execute(Command::AddEdge {
            start: [9.0, 0.0, 1.0],
            end: [13.0, 0.0, 1.0],
        })
        .unwrap();
        e.execute(Command::OpenInstance { id: group }).unwrap();
        e.select_tool(ToolKind::Select);
        let picks = click_along_top_front_edge(&mut e);
        assert!(
            picks
                .iter()
                .all(|p| matches!(p.as_slice(), [EntityId::Edge(_)])),
            "{picks:?}"
        );
    }

    #[test]
    fn a_root_edge_on_a_groups_edge_wins_the_pick_at_the_root() {
        let mut e = editor();
        moved_group(&mut e);
        e.execute(Command::AddEdge {
            start: [9.0, 0.0, 1.0],
            end: [13.0, 0.0, 1.0],
        })
        .unwrap();
        e.select_tool(ToolKind::Select);
        let picks = click_along_top_front_edge(&mut e);
        assert!(
            picks
                .iter()
                .all(|p| matches!(p.as_slice(), [EntityId::Edge(_)])),
            "{picks:?}"
        );
    }

    #[test]
    fn push_pull_from_the_root_ignores_a_groups_face() {
        let mut e = editor();
        let group = moved_group(&mut e);
        let before = box_min(&e, group);
        let top = box_max(&e, group);
        e.select_tool(ToolKind::PushPull);
        click_at(&mut e, DVec3::new(11.0, 1.0, 1.0));
        let px = e.doc().camera.project(DVec3::new(11.0, 1.0, 2.0)).unwrap();
        e.cursor_moved(px);
        for c in "2".chars() {
            e.key(EditorKey::Char(c));
        }
        e.key(EditorKey::Enter);
        assert_eq!(box_min(&e, group), before);
        assert_eq!(box_max(&e, group), top);
        assert_eq!(stats(&e), (0, 0), "the root gained nothing");
    }

    #[test]
    fn crossing_a_group_with_a_box_dragged_leftwards_selects_it() {
        let mut e = editor();
        let group = moved_group(&mut e);
        clear_selection(&mut e);
        e.select_tool(ToolKind::Select);
        let centre = e.doc().camera.project(DVec3::new(11.0, 1.0, 0.5)).unwrap();
        e.cursor_moved(centre + DVec2::new(30.0, 30.0));
        e.button(Button::Left, true);
        e.cursor_moved(centre);
        e.button(Button::Left, false);
        assert_eq!(
            e.doc().selection().iter().copied().collect::<Vec<_>>(),
            [group]
        );
    }

    #[test]
    fn the_erase_tool_erases_a_group_and_undo_restores_it() {
        let mut e = editor();
        moved_group(&mut e);
        assert_eq!(definition_names(&e).len(), 1);
        e.select_tool(ToolKind::Erase);
        click_at(&mut e, DVec3::new(11.0, 1.0, 1.0));
        assert!(e.doc().model().instances().next().is_none());
        assert_eq!(definition_names(&e).len(), 0, "its definition went too");
        assert!(e.undo());
        assert!(e.doc().model().instances().next().is_some());
        assert_eq!(definition_names(&e).len(), 1);
    }

    /// A path for a test file, unique to this process.
    fn temp_file(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("sketchup-alt-editor-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn saving_opening_and_starting_over() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.key(EditorKey::Escape);
        assert!(!e.save_file(None), "a model never saved needs a path");
        assert!(e.status().contains("give a path"), "{}", e.status());
        let path = temp_file("square.skr");
        assert!(e.save_file(Some(&path)));
        assert!(e.status().contains("Saved"), "{}", e.status());
        assert!(!e.doc().is_modified());

        assert!(e.new_model());
        assert_eq!(stats(&e), (0, 0));
        assert!(e.open_file(&path));
        assert_eq!(stats(&e), (4, 1));
        assert_eq!(e.doc().path(), Some(path.as_path()));

        let missing = temp_file("missing.skr");
        assert!(!e.open_file(&missing));
        assert!(e.status().contains("missing.skr"), "{}", e.status());
        assert_eq!(stats(&e), (4, 1), "a failed open leaves the model");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_new_or_opened_model_forgets_the_chosen_material() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.key(EditorKey::Escape);
        assert!(e.add_material());
        assert!(e.current_material().is_some());
        let path = temp_file("material.skr");
        assert!(e.save_file(Some(&path)));
        // The file holds the material, so only the reset can make the choice go away.
        assert!(e.open_file(&path));
        assert_eq!(e.material, None);
        assert!(e.add_material());
        assert!(e.new_model());
        assert_eq!(e.material, None);
        assert_eq!(e.current_material(), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn exporting_writes_the_file_its_extension_names() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        let path = temp_file("square.stl");
        assert!(
            !e.export_file(&path),
            "an empty model has nothing to export"
        );
        assert!(e.status().contains("no faces"), "{}", e.status());
        square(&mut e);
        e.key(EditorKey::Escape);
        assert!(e.export_file(&path));
        assert!(e.status().contains("Exported"), "{}", e.status());
        assert_eq!(std::fs::read(&path).unwrap().len(), 84 + 50 * 2);
        let unknown = temp_file("square.dwg");
        assert!(!e.export_file(&unknown));
        assert!(e.status().contains(".obj"), "{}", e.status());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_export_status_counts_faces_left_out() {
        let path = Path::new("model.stl");
        assert_eq!(export_message(path, 0), "Exported model.stl");
        assert!(export_message(path, 1).ends_with("without 1 face that could not be triangulated"));
        assert!(export_message(path, 3).contains("without 3 faces"));
    }

    #[test]
    fn the_export_dialog_suggests_the_last_format_exported() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        assert_eq!(e.export_formats()[0], ("OBJ", "obj"));
        let path = temp_file("suggest.STL");
        assert!(e.export_file(&path));
        let formats = e.export_formats();
        assert_eq!(formats[0], ("STL", "stl"));
        assert_eq!(formats.len(), EXPORT_FORMATS.len());
        assert!(formats.contains(&("OBJ", "obj")));
        // A failed export changes nothing.
        assert!(!e.export_file(&temp_file("suggest.skr")));
        assert_eq!(e.export_formats()[0], ("STL", "stl"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn importing_adds_a_selected_group_and_reports_it() {
        let mut e = editor();
        let path = temp_file("tri.obj");
        std::fs::write(
            &path,
            "v 0 0 0\nv 1 0 0\nv 0 0 -1\nv 2 0 0\nf 1 2 3\nf 1 2 4\n",
        )
        .unwrap();
        assert!(e.import_file(&path));
        assert!(
            e.status().contains("1 face; 1 polygon skipped"),
            "{}",
            e.status()
        );
        let instance = e.doc().model().instances().next().unwrap().0;
        assert!(e.doc().selection().contains(&EntityId::Instance(instance)));
        assert!(e.undo());
        assert_eq!(e.doc().model().instances().count(), 0);

        let missing = temp_file("missing.stl");
        assert!(!e.import_file(&missing));
        assert!(e.status().contains("missing.stl"), "{}", e.status());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn imports_read_the_chosen_units() {
        let mut e = editor();
        let path = temp_file("mm.stl");
        let triangle = "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\n\
                        vertex 2000 0 0\nvertex 0 500 0\nendloop\nendfacet\nendsolid t\n";
        std::fs::write(&path, triangle).unwrap();
        let widest = |e: &Editor| {
            let (_, i) = e.doc().model().instances().next().unwrap();
            let mesh = e
                .doc()
                .scene()
                .mesh(kernel::MeshId::Definition(i.definition))
                .unwrap();
            mesh.vertices().map(|(_, v)| v.pos.x).fold(0.0, f64::max)
        };
        assert_eq!(e.import_units(), ImportUnits::Meters);
        assert!(e.import_file(&path));
        assert_eq!(widest(&e), 2000.0);
        e.undo();
        e.set_import_units(ImportUnits::Millimeters);
        assert!(e.import_file(&path));
        assert!((widest(&e) - 2.0).abs() < 1e-12, "{}", widest(&e));
        assert!(
            e.status().contains("mm.stl in millimeters: 1 face"),
            "{}",
            e.status()
        );
        e.undo();
        e.set_import_units(ImportUnits::Inches);
        assert!(e.import_file(&path));
        assert!((widest(&e) - 50.8).abs() < 1e-9, "{}", widest(&e));
        let _ = std::fs::remove_file(&path);
    }

    /// Adds a 2 x 2 rectangle with its corner at (x, 0, 0) and selects it alone.
    fn selected_rectangle(e: &mut Editor, x: f64) {
        let outcome = e
            .execute(Command::AddRectangle {
                corner: [x, 0.0, 0.0],
                opposite: [x + 2.0, 2.0, 0.0],
                normal: None,
            })
            .unwrap();
        let ids = outcome
            .changes
            .created
            .iter()
            .copied()
            .filter(|id| matches!(id, EntityId::Edge(_) | EntityId::Face(_)))
            .collect();
        e.execute(Command::Select {
            ids,
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
    }

    /// The one selected instance's definition.
    fn selected_definition(e: &Editor) -> kernel::DefinitionId {
        let selected: Vec<EntityId> = e.doc().selection().iter().copied().collect();
        let [EntityId::Instance(i)] = selected[..] else {
            panic!("one instance selected: {selected:?}")
        };
        e.doc().model().instance(i).unwrap().definition
    }

    #[test]
    fn make_component_named_uses_the_name() {
        let mut e = editor();
        selected_rectangle(&mut e, 0.0);
        assert_eq!(e.next_component_name(), "Component 1");
        assert!(e.make_component_named("Chair"));
        let d = e.doc().scene().definition(selected_definition(&e)).unwrap();
        assert_eq!(d.kind, DefinitionKind::Component);
        assert_eq!(d.name, "Chair");
        assert_eq!(e.next_component_name(), "Component 1");
    }

    #[test]
    fn naming_a_component_with_nothing_selected_says_so() {
        let mut e = editor();
        let revision = e.doc().revision();
        assert!(e.make_component_named("Chair"));
        assert_eq!(e.doc().revision(), revision);
        assert_eq!(definition_names(&e).len(), 0);
        assert!(
            e.status().contains("Select geometry to make a component"),
            "{}",
            e.status()
        );
        assert_eq!(e.message_kind(), Some(MessageKind::Error));
    }

    #[test]
    fn rename_target_is_the_one_selected_instance() {
        let mut e = editor();
        assert_eq!(e.rename_target(), None, "nothing selected");
        selected_rectangle(&mut e, 0.0);
        assert!(e.make_group());
        let group = selected_definition(&e);
        assert_eq!(
            e.rename_target(),
            Some(RenameTarget {
                definition: group,
                kind: DefinitionKind::Group,
                name: String::new(),
            })
        );
        let first = *e.doc().selection().first().unwrap();
        selected_rectangle(&mut e, 5.0);
        assert!(e.make_component_named("Chair"));
        let chair = selected_definition(&e);
        assert_eq!(
            e.rename_target(),
            Some(RenameTarget {
                definition: chair,
                kind: DefinitionKind::Component,
                name: "Chair".into(),
            })
        );
        let second = *e.doc().selection().first().unwrap();
        e.execute(Command::Select {
            ids: vec![first, second],
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
        assert_eq!(e.rename_target(), None, "two instances");
        selected_rectangle(&mut e, 10.0);
        let edge = *e.doc().selection().first().unwrap();
        assert!(matches!(edge, EntityId::Edge(_)));
        e.execute(Command::Select {
            ids: vec![second, edge],
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
        assert_eq!(e.rename_target(), None, "an instance and an edge");
    }

    #[test]
    fn rename_definition_renames_and_undoes() {
        let mut e = editor();
        selected_rectangle(&mut e, 0.0);
        assert!(e.make_component());
        let d = selected_definition(&e);
        assert!(e.rename_definition(d, "Table"));
        assert_eq!(
            e.status_parts().message.as_deref(),
            Some("Renamed Component 1 to Table")
        );
        assert_eq!(e.message_kind(), Some(MessageKind::Info));
        assert_eq!(definition_names(&e), ["Table"]);
        assert!(e.undo());
        assert_eq!(definition_names(&e), ["Component 1"]);
        e.rename_definition(d, "  ");
        assert!(
            e.status().contains("a component needs a name"),
            "{}",
            e.status()
        );
        assert_eq!(e.message_kind(), Some(MessageKind::Error));
        assert_eq!(definition_names(&e), ["Component 1"]);
        // The same name again is skipped: no step and no new message.
        let revision = e.doc().revision();
        assert!(!e.rename_definition(d, "Component 1"));
        assert_eq!(e.doc().revision(), revision);
        assert!(e.status().contains("a component needs a name"));
    }

    #[test]
    fn a_rename_whose_definition_is_gone_says_so() {
        let mut e = editor();
        selected_rectangle(&mut e, 0.0);
        assert!(e.make_component());
        let d = selected_definition(&e);
        assert!(e.erase_selection());
        e.execute(Command::PurgeDefinitions {}).unwrap();
        assert!(e.doc().scene().definition(d).is_none());
        e.rename_definition(d, "X");
        assert!(e.status().contains("no definition"), "{}", e.status());
    }

    #[test]
    fn shift_g_is_the_make_component_shortcut() {
        let mut e = editor();
        assert_eq!(
            e.shortcut_for(EditorKey::Char('G')),
            Some(Bound::MakeComponent)
        );
        assert_eq!(e.shortcut_for(EditorKey::Char('g')), Some(Bound::MakeGroup));
        assert!(e.key(EditorKey::Char('2')));
        assert_eq!(e.shortcut_for(EditorKey::Char('G')), None, "the G is text");
        e.key(EditorKey::Escape);
        assert!(e.set_flying(true));
        assert_eq!(e.shortcut_for(EditorKey::Char('G')), None, "flying");
    }

    /// Makes a component of a new rectangle at (x, 0, 0) and erases its instance, leaving the
    /// definition unused.
    fn unused_component(e: &mut Editor, x: f64) {
        selected_rectangle(e, x);
        assert!(e.make_component());
        assert!(e.erase_selection());
    }

    #[test]
    fn purge_unused_reports_the_count() {
        let mut e = editor();
        unused_component(&mut e, 0.0);
        unused_component(&mut e, 5.0);
        assert_eq!(definition_names(&e).len(), 2);
        assert!(e.purge_unused());
        assert_eq!(
            e.status_parts().message.as_deref(),
            Some("Purged 2 unused definitions")
        );
        assert_eq!(e.message_kind(), Some(MessageKind::Info));
        assert!(definition_names(&e).is_empty());
        assert!(e.purge_unused());
        assert_eq!(
            e.status_parts().message.as_deref(),
            Some("Nothing to purge")
        );
        assert!(
            e.undo(),
            "the empty purge was no step: this undoes the first"
        );
        assert_eq!(definition_names(&e).len(), 2, "one undo restores both");
        assert!(e.purge_unused());
        unused_component(&mut e, 10.0);
        assert!(e.purge_unused());
        assert_eq!(
            e.status_parts().message.as_deref(),
            Some("Purged 1 unused definition")
        );
    }

    #[test]
    fn field_of_view_keeps_the_view_and_its_up() {
        let mut e = editor();
        assert!(e.standard_view(StandardView::Top));
        let before = e.doc().camera.clone();
        let revision = e.doc().revision();
        assert!(e.set_field_of_view(60.0));
        let after = &e.doc().camera;
        assert_eq!(after.eye, before.eye);
        assert_eq!(after.target, before.target);
        assert!(after.up.abs_diff_eq(before.up, 1e-12), "{}", after.up);
        assert!((e.field_of_view() - 60.0).abs() < 1e-9);
        assert_eq!(e.doc().revision(), revision);
        assert!(!e.doc().is_modified());
        let camera = e.doc().camera.clone();
        e.set_field_of_view(200.0);
        assert!(e.status().contains("fov_y_deg"), "{}", e.status());
        assert_eq!(e.message_kind(), Some(MessageKind::Error));
        assert_eq!(e.doc().camera, camera);
    }

    /// A one-quad OBJ, 100 x 200 file units in the file's xy plane.
    fn quad_obj(name: &str) -> std::path::PathBuf {
        let path = temp_file(name);
        std::fs::write(
            &path,
            "v 0 0 0\nv 100 0 0\nv 100 200 0\nv 0 200 0\nf 1 2 3 4\n",
        )
        .unwrap();
        path
    }

    /// A SketchUp file among the `skp` crate's fixtures.
    fn skp_fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../skp/tests/fixtures")
            .join(name)
    }

    #[test]
    fn a_sketchup_import_says_what_it_made_and_warns() {
        let mut e = editor();
        // A SketchUp file has its own units and axes: the options' are not applied.
        e.set_import_options(ImportOptions {
            units: ImportUnits::Millimeters,
            up: Some(UpAxis::Y),
            ..Default::default()
        });
        assert!(e.import_file(&skp_fixture("desk.skp")), "{}", e.status());
        let message = e.status_parts().message.unwrap();
        assert!(message.starts_with("Imported desk"), "{message}");
        assert!(
            message.ends_with(
                ": 147 faces, 1 definition, 1 instance; with 1 warning (see the import report)"
            ),
            "{message}"
        );
        assert!(!message.contains("millimeters"), "{message}");
        let instance = e.doc().model().instances().next().unwrap().0;
        let [lo, hi] = e
            .doc()
            .scene()
            .bounds(kernel::MeshId::Definition(
                e.doc().model().instance(instance).unwrap().definition,
            ))
            .unwrap();
        assert!((hi - lo).max_element() > 0.5, "read in inches: {lo} {hi}");
    }

    #[test]
    fn opening_a_sketchup_file_makes_an_untitled_model() {
        let mut e = editor();
        let path = skp_fixture("cupboard.skp");
        assert!(e.open_file(&path), "{}", e.status());
        let message = e.status_parts().message.unwrap();
        assert!(message.starts_with("Opened cupboard.skp"), "{message}");
        assert_eq!(e.doc().path(), None);
        assert_eq!(e.doc().imported_from(), Some(path.as_path()));
        assert!(!e.doc().is_modified());
        assert!(e.doc().model().instances().count() > 1);
    }

    #[test]
    fn import_uses_the_options() {
        let mut e = editor();
        let path = quad_obj("options.obj");
        let options = ImportOptions {
            units: ImportUnits::Centimeters,
            up: Some(UpAxis::Z),
            place: ImportAs::Component,
            merge_coplanar: false,
        };
        e.set_import_options(options);
        assert_eq!(e.import_options(), options);
        assert!(e.import_file(&path), "{}", e.status());
        let selected: Vec<EntityId> = e.doc().selection().iter().copied().collect();
        let [EntityId::Instance(instance)] = selected[..] else {
            panic!("one instance selected: {selected:?}")
        };
        let definition = e.doc().model().instance(instance).unwrap().definition;
        let d = e.doc().scene().definition(definition).unwrap();
        assert_eq!(d.kind, DefinitionKind::Component);
        let (_, face) = d.mesh.faces().next().unwrap();
        assert!(
            face.normal.z.abs() > 1.0 - 1e-9,
            "the file's z stays up: {}",
            face.normal
        );
        let [lo, hi] = e
            .doc()
            .scene()
            .bounds(kernel::MeshId::Definition(definition))
            .unwrap();
        assert!(
            (hi - lo).abs_diff_eq(DVec3::new(1.0, 2.0, 0.0), 1e-9),
            "{lo} {hi}"
        );
        assert_eq!(e.import_units(), ImportUnits::Centimeters);
        e.set_import_units(ImportUnits::Feet);
        assert_eq!(
            e.import_options(),
            ImportOptions {
                units: ImportUnits::Feet,
                ..options
            },
            "only the units change"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_loose_import_selects_its_geometry() {
        let mut e = editor();
        let path = quad_obj("loose.obj");
        e.set_import_options(ImportOptions {
            place: ImportAs::Context,
            ..Default::default()
        });
        assert!(e.import_file(&path), "{}", e.status());
        assert_eq!(e.doc().model().instances().count(), 0);
        let faces: Vec<_> = e.doc().model().faces().map(|(id, _)| id).collect();
        assert_eq!(faces.len(), 1);
        assert!(e.doc().selection().contains(&EntityId::Face(faces[0])));
        let name = path.file_name().unwrap().to_string_lossy();
        assert_eq!(
            e.status_parts().message,
            Some(format!("Imported {name}: 1 face"))
        );
        assert_eq!(e.message_kind(), Some(MessageKind::Info));
        let _ = std::fs::remove_file(&path);
    }

    /// Vertices of the open context, as positions.
    fn positions(e: &Editor) -> Vec<DVec3> {
        e.doc().model().vertices().map(|(_, v)| v.pos).collect()
    }

    #[test]
    fn the_circle_tool_draws_a_circle_around_the_first_click() {
        let mut e = editor();
        assert!(e.key(EditorKey::Char('c')));
        assert_eq!(e.tool_kind(), ToolKind::Circle);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        click_at(&mut e, DVec3::new(3.0, 1.0, 0.0));
        assert_eq!(stats(&e), (24, 1));
        let center = DVec3::new(1.0, 1.0, 0.0);
        for p in positions(&e) {
            assert!((p.distance(center) - 2.0).abs() < 1e-6, "{p}");
        }
    }

    #[test]
    fn a_typed_side_count_and_radius_shape_the_circle() {
        let mut e = editor();
        e.select_tool(ToolKind::Circle);
        for c in "8s".chars() {
            e.key(EditorKey::Char(c));
        }
        e.key(EditorKey::Enter);
        assert!(e.status().contains("8 sides"), "{}", e.status());
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        let px = e.doc().camera.project(DVec3::new(1.0, 0.0, 0.0)).unwrap();
        e.cursor_moved(px);
        for c in "1.5".chars() {
            e.key(EditorKey::Char(c));
        }
        e.key(EditorKey::Enter);
        assert_eq!(stats(&e), (8, 1));
        for p in positions(&e) {
            assert!((p.length() - 1.5).abs() < 1e-9, "{p}");
        }
        // The count stays for the next circle.
        click_at(&mut e, DVec3::new(5.0, 0.0, 0.0));
        click_at(&mut e, DVec3::new(6.0, 0.0, 0.0));
        assert_eq!(stats(&e), (16, 2));
    }

    #[test]
    fn side_and_segment_counts_last_across_tool_switches() {
        let mut e = editor();
        e.select_tool(ToolKind::Circle);
        type_text(&mut e, "8s");
        e.select_tool(ToolKind::Arc);
        type_text(&mut e, "5s");
        e.select_tool(ToolKind::Line);
        e.select_tool(ToolKind::Circle);
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        click_at(&mut e, DVec3::new(1.0, 0.0, 0.0));
        assert_eq!(stats(&e), (8, 1));
        // Reselecting the tool that is in use keeps the count too.
        e.select_tool(ToolKind::Arc);
        e.select_tool(ToolKind::Arc);
        click_at(&mut e, DVec3::new(4.0, 0.0, 0.0));
        click_at(&mut e, DVec3::new(6.0, 0.0, 0.0));
        click_at(&mut e, DVec3::new(5.0, 1.0, 0.0));
        assert_eq!(e.doc().model().edges().count(), 8 + 5);
    }

    #[test]
    fn the_arc_tool_draws_an_arc_from_three_clicks() {
        let mut e = editor();
        assert!(e.key(EditorKey::Char('a')));
        assert_eq!(e.tool_kind(), ToolKind::Arc);
        for c in "4s".chars() {
            e.key(EditorKey::Char(c));
        }
        e.key(EditorKey::Enter);
        assert!(e.status().contains("4 segments"), "{}", e.status());
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        click_at(&mut e, DVec3::new(2.0, 0.0, 0.0));
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        assert_eq!(stats(&e), (4, 0));
        for p in positions(&e) {
            assert!(
                (p.distance(DVec3::new(1.0, 0.0, 0.0)) - 1.0).abs() < 1e-6,
                "{p}"
            );
        }
        let edges: Vec<_> = e.doc().model().edges().map(|(id, _)| id).collect();
        let curve = e.doc().model().curve_of(edges[0]).unwrap();
        assert_eq!(curve.edges.len(), 4);
    }

    #[test]
    fn a_circle_started_on_a_wall_lies_in_the_wall() {
        let mut e = editor();
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [4.0, 0.0, 3.0],
            normal: None,
        })
        .unwrap();
        e.select_tool(ToolKind::Circle);
        click_at(&mut e, DVec3::new(2.0, 0.0, 1.5));
        click_at(&mut e, DVec3::new(2.5, 0.0, 1.5));
        let circle: Vec<DVec3> = positions(&e)
            .into_iter()
            .filter(|p| p.distance(DVec3::new(2.0, 0.0, 1.5)) < 0.6)
            .collect();
        assert_eq!(circle.len(), 24);
        assert!(
            circle.iter().all(|p| p.y.abs() < 1e-9),
            "in the wall's plane"
        );
    }

    /// The front and back materials of the open context's only face.
    fn face_paint(e: &Editor) -> [Option<MaterialId>; 2] {
        e.doc().model().faces().next().unwrap().1.paint()
    }

    #[test]
    fn b_paints_faces_with_the_chosen_material_and_cmd_click_picks_one_up() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        assert!(e.key(EditorKey::Char('b')));
        assert_eq!(e.tool_kind(), ToolKind::Paint);
        assert!(e.add_material());
        let first = e.current_material().expect("the new material is chosen");
        assert_eq!(
            e.doc().scene().material(first).unwrap().color,
            MATERIAL_PALETTE[0]
        );
        let inside = DVec3::new(2.0, 2.0, 0.0);
        click_at(&mut e, inside);
        assert_eq!(face_paint(&e), [Some(first), None]);
        e.undo();
        assert_eq!(face_paint(&e), [None, None]);
        e.redo();

        // A second material, then pick the first back up with Cmd held: nothing is painted.
        e.add_material();
        let second = e.current_material().unwrap();
        assert_ne!(first, second);
        let revision = e.doc().revision();
        e.set_sample_modifier(true);
        click_at(&mut e, inside);
        e.set_sample_modifier(false);
        assert_eq!(e.doc().revision(), revision);
        assert_eq!(e.current_material(), Some(first));
        assert!(
            e.status().contains("Painting with Material"),
            "{}",
            e.status()
        );

        // Choosing the default material in the list paints faces back to default.
        e.choose_material(None);
        click_at(&mut e, inside);
        assert_eq!(face_paint(&e), [None, None]);

        // Renaming, recoloring and deleting go through commands; deleting the chosen
        // material leaves the default chosen.
        e.choose_material(Some(second));
        e.rename_material(second, "Brick".into());
        e.recolor_material(second, [1, 2, 3]);
        e.set_material_opacity(second, 40);
        e.set_material_emission(second, 25);
        let m = e.doc().scene().material(second).unwrap();
        assert_eq!((m.name.as_str(), m.color), ("Brick", [1, 2, 3]));
        assert_eq!(m.opacity, 0.4);
        assert_eq!(m.emission, 2.5);
        e.undo();
        assert_eq!(e.doc().scene().material(second).unwrap().emission, 0.0);
        e.undo();
        assert_eq!(e.doc().scene().material(second).unwrap().opacity, 1.0);
        e.redo();
        assert_eq!(e.doc().scene().material(second).unwrap().opacity, 0.4);
        e.rename_material(second, "  ".into());
        assert!(e.status().contains("needs a name"), "{}", e.status());
        e.delete_material(second);
        assert_eq!(e.current_material(), None);
    }

    #[test]
    fn choosing_a_material_keeps_the_tool_and_its_operation() {
        let mut e = editor();
        e.select_tool(ToolKind::Rectangle);
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0)); // the first corner
        let prompt = e.status_parts().prompt;
        e.add_material();
        let added = e.current_material();
        assert!(added.is_some());
        assert_eq!(e.tool_kind(), ToolKind::Rectangle, "adding keeps the tool");
        e.choose_material(None);
        assert_eq!(e.tool_kind(), ToolKind::Rectangle);
        assert_eq!(e.status_parts().prompt, prompt, "the operation goes on");
        e.paint_with(added);
        assert_eq!(
            (e.tool_kind(), e.current_material()),
            (ToolKind::Paint, added)
        );
    }

    /// A 2 x 1 pixel PNG: a red pixel, then a blue one.
    const RED_BLUE_PNG: [u8; 71] = [
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 244, 34, 127, 138, 0, 0, 0, 14, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 0,
        66, 255, 1, 15, 249, 3, 253, 133, 17, 153, 118, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
        130,
    ];

    #[test]
    fn textures_load_resize_and_come_off_through_commands() {
        let mut e = editor();
        e.add_material();
        let id = e.current_material().unwrap();
        let png = std::env::temp_dir().join(format!(
            "sketchup-alt-editor-{}-red-blue.png",
            std::process::id()
        ));
        std::fs::write(&png, RED_BLUE_PNG).unwrap();
        e.load_texture(id, &png);
        let m = e.doc().scene().material(id).unwrap();
        let t = m.texture.as_ref().expect("a texture");
        assert_eq!((t.width, t.height), (1.0, 0.5));
        assert_eq!((t.image.width, t.image.height), (2, 1));
        assert_eq!(m.color, [128, 0, 128], "the image's average");

        e.set_texture_size(id, [2000, 250]);
        let t = e
            .doc()
            .scene()
            .material(id)
            .unwrap()
            .texture
            .clone()
            .unwrap();
        assert_eq!((t.width, t.height), (2.0, 0.25));
        e.undo();
        let t = e
            .doc()
            .scene()
            .material(id)
            .unwrap()
            .texture
            .clone()
            .unwrap();
        assert_eq!((t.width, t.height), (1.0, 0.5));

        // Loading again over a 2 x 2 m texture keeps the width; the height follows the image.
        e.set_texture_size(id, [2000, 2000]);
        e.load_texture(id, &png);
        let t = e
            .doc()
            .scene()
            .material(id)
            .unwrap()
            .texture
            .clone()
            .unwrap();
        assert_eq!((t.width, t.height), (2.0, 1.0));
        e.undo();
        e.undo();

        e.remove_texture(id);
        assert!(e.doc().scene().material(id).unwrap().texture.is_none());
        e.undo();
        assert!(e.doc().scene().material(id).unwrap().texture.is_some());

        // A file that is not an image leaves the material and says why.
        let text = png.with_extension("txt");
        std::fs::write(&text, "hello").unwrap();
        let revision = e.doc().revision();
        e.load_texture(id, &text);
        assert_eq!(e.doc().revision(), revision);
        assert!(e.status().contains("not a PNG or JPEG"), "{}", e.status());
        let _ = std::fs::remove_file(png);
        let _ = std::fs::remove_file(text);
    }

    #[test]
    fn clicking_a_selected_face_paints_the_whole_selection() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        for (x, y) in [(5.0, 1.0), (7.0, 1.0), (7.0, 3.0), (5.0, 3.0), (5.0, 1.0)] {
            click_at(&mut e, DVec3::new(x, y, 0.0));
        }
        let faces: Vec<EntityId> = e
            .doc()
            .model()
            .faces()
            .map(|(f, _)| EntityId::Face(f))
            .collect();
        assert_eq!(faces.len(), 2);
        e.execute(Command::Select {
            ids: faces,
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
        e.select_tool(ToolKind::Paint);
        e.add_material();
        let m = e.current_material();
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        assert!(e.doc().model().faces().all(|(_, f)| f.front == m));
    }

    #[test]
    fn f_toggles_fly_mode_and_escape_leaves_it() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        assert!(!e.flying());
        assert!(e.key(EditorKey::Char('f')));
        assert!(e.flying());
        assert!(
            e.status().starts_with("Fly  |  Fly mode: 3.0 m/s"),
            "{}",
            e.status()
        );
        assert!(e.key(EditorKey::Char('f')));
        assert!(!e.flying());
        assert!(e.key(EditorKey::Char('F')), "Shift+F too");
        assert!(e.key(EditorKey::Escape));
        assert!(!e.flying());
        assert_eq!(e.tool_kind(), ToolKind::Line, "the tool is back");
        assert!(e.status().starts_with("Line"), "{}", e.status());
        // A held F leaves once and does not come back in.
        assert!(e.toggle_fly());
        assert!(!e.key_repeat(EditorKey::Char('f')));
        assert!(e.flying());
        assert!(e.toggle_fly());
        assert!(!e.flying());
        // While typing a length, f is just text.
        e.key(EditorKey::Char('2'));
        e.key(EditorKey::Char('f'));
        assert!(!e.flying());
    }

    #[test]
    fn entering_fly_mode_puts_the_target_a_meter_ahead_and_uses_perspective() {
        let mut e = editor();
        e.set_projection(Projection::Parallel);
        let eye = e.doc().camera.eye;
        assert!(e.toggle_fly());
        let cam = &e.doc().camera;
        assert!(!cam.is_parallel(), "switched to perspective");
        assert!(cam.eye.abs_diff_eq(eye, 1e-12), "the eye stays");
        assert!((cam.distance() - fly::TARGET_AHEAD).abs() < 1e-9);
        // The API's camera query reflects it.
        match e.doc().query(&session::Query::Camera {}).unwrap() {
            QueryResult::Camera { eye, target, .. } => {
                let d = DVec3::from(target) - DVec3::from(eye);
                assert!((d.length() - fly::TARGET_AHEAD).abs() < 1e-9);
            }
            other => panic!("unexpected {other:?}"),
        }
        // An agent switching to parallel ends fly mode at the next frame.
        e.execute(Command::SetProjection {
            projection: Projection::Parallel,
        })
        .unwrap();
        assert!(e.fly_tick(Instant::now()), "a frame shows it");
        assert!(!e.flying());
    }

    #[test]
    fn tool_keys_and_clicks_do_nothing_while_flying() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        // A line in progress.
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        move_to(&mut e, DVec3::new(2.0, 1.0, 0.0));
        assert!(!blank(&shown(&mut e)));
        let before = stats(&e);
        e.toggle_fly();
        assert_eq!(
            shown(&mut e),
            Hud::default(),
            "no marker, preview or chip while flying"
        );
        for c in ['r', 'e', 'q', ' ', 'g', 'G', 'Z', '5', 'm'] {
            assert!(!e.key(EditorKey::Char(c)), "{c:?}");
        }
        assert!(!e.key(EditorKey::Enter));
        assert!(!e.key(EditorKey::Delete));
        assert!(!e.key(EditorKey::Backspace));
        assert!(!e.key(EditorKey::LockAxis(Axis::X)));
        assert_eq!(e.tool_kind(), ToolKind::Line);
        let cam = e.doc().camera.clone();
        e.cursor_moved(DVec2::new(300.0, 200.0));
        assert!(!e.button(Button::Left, true));
        assert!(!e.button(Button::Left, false));
        assert!(!e.double_click());
        assert!(!e.button(Button::Middle, true), "no orbiting either");
        e.cursor_moved(DVec2::new(350.0, 260.0));
        assert!(!e.button(Button::Middle, false));
        assert!(!e.pinch(0.2));
        assert!(!e.toggle_copy());
        assert_eq!(e.doc().camera, cam);
        assert_eq!(stats(&e), before);
        // Escape leaves fly mode rather than ending the line, which goes on after.
        assert!(e.key(EditorKey::Escape));
        click_at(&mut e, DVec3::new(3.0, 1.0, 0.0));
        assert_eq!(stats(&e).0, before.0 + 1, "the line went on");
    }

    #[test]
    fn cmd_shortcuts_keep_working_while_flying() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.toggle_fly();
        assert!(e.undo());
        assert!(e.flying());
        assert_eq!(stats(&e).0, 3);
        assert!(e.redo());
        assert_eq!(stats(&e), (4, 1));
    }

    #[test]
    fn held_keys_fly_and_speed_keys_change_the_speed() {
        let mut e = editor();
        assert!(!e.fly_key(FlyKey::W, true), "not flying");
        e.toggle_fly();
        let start = e.doc().camera.clone();
        let t = Instant::now();
        assert!(!e.fly_tick(t), "idle stays idle");
        assert!(e.fly_key(FlyKey::W, true));
        assert!(!e.fly_key(FlyKey::W, true), "a repeat changes nothing");
        assert!(e.fly_tick(t));
        for i in 1..=30 {
            assert!(e.fly_tick(t + std::time::Duration::from_millis(16 * i)));
        }
        let cam = e.doc().camera.clone();
        let moved = cam.eye - start.eye;
        assert!(moved.length() > 0.5, "{moved}");
        let ahead = (start.target - start.eye).normalize();
        assert!(
            moved.normalize().abs_diff_eq(ahead, 1e-9),
            "forward follows the view"
        );
        assert!((cam.target - cam.eye).abs_diff_eq(start.target - start.eye, 1e-9));
        // Speed keys, repeated too.
        assert!(e.key(EditorKey::Char(']')));
        assert!(e.key_repeat(EditorKey::Char('=')));
        assert!(e.key(EditorKey::Char('[')));
        assert!(e.status().contains("3.8 m/s"), "{}", e.status());
        assert!(e.key(EditorKey::Char('-')));
        assert!(e.status().contains("3.0 m/s"), "{}", e.status());
        // The wheel changes it a step a notch.
        assert!(e.scroll(Scroll::Lines(DVec2::new(0.0, 1.0))));
        assert!(e.status().contains("3.8 m/s"), "{}", e.status());
        // The speed lasts for the session.
        e.toggle_fly();
        e.toggle_fly();
        assert!(e.status().contains("3.8 m/s"), "{}", e.status());
    }

    #[test]
    fn losing_focus_lets_go_of_the_fly_keys() {
        let mut e = editor();
        e.toggle_fly();
        let t = Instant::now();
        e.fly_key(FlyKey::W, true);
        e.fly_key(FlyKey::Space, true);
        e.fly_tick(t);
        e.fly_tick(t + std::time::Duration::from_millis(50));
        e.focus_lost();
        assert!(e.flying(), "still flying");
        let mut frames = 0;
        let mut now = t + std::time::Duration::from_millis(50);
        while e.fly_tick(now) {
            now += std::time::Duration::from_millis(16);
            frames += 1;
            assert!(frames < 100, "glides to a stop");
        }
        let rest = e.doc().camera.clone();
        assert!(!e.fly_tick(now + std::time::Duration::from_secs(1)));
        assert_eq!(e.doc().camera, rest);
    }

    #[test]
    fn the_mouse_and_trackpad_look_around_while_flying() {
        let mut e = editor();
        assert!(!e.fly_look(DVec2::new(10.0, 0.0)), "not flying");
        e.toggle_fly();
        let eye = e.doc().camera.eye;
        let (yaw, _) = fly::heading(&e.doc().camera);
        assert!(e.fly_look(DVec2::new(40.0, 0.0)));
        let (turned, _) = fly::heading(&e.doc().camera);
        assert!(turned < yaw, "mouse right turns right");
        assert_eq!(e.doc().camera.eye, eye);
        // Two-finger scroll drags the scene: fingers right turn the view left.
        assert!(e.scroll(Scroll::Pixels(DVec2::new(50.0, 0.0))));
        let (back, _) = fly::heading(&e.doc().camera);
        assert!(back > turned);
        // With Cmd it changes the speed instead.
        e.set_sample_modifier(true);
        let cam = e.doc().camera.clone();
        assert!(e.scroll(Scroll::Pixels(DVec2::new(
            0.0,
            1.25 * LOGICAL_PX_PER_ZOOM_STEP
        ))));
        assert_eq!(e.doc().camera, cam);
        assert!(e.status().contains("3.8 m/s"), "{}", e.status());
    }

    #[test]
    fn shift_flies_faster_without_locking() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        e.toggle_fly();
        assert!(!e.set_modifiers(true, false), "no lock while flying");
        assert!(!e.status().contains("Locked"));
        e.set_modifiers(false, false);
        e.toggle_fly();
        assert!(!e.status().contains("Locked"));
    }

    #[test]
    fn an_agent_edit_drops_the_highlight_until_the_next_frame() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.select_tool(ToolKind::Erase);
        // `square` draws 1..3 on both axes: the face's middle.
        move_to(&mut e, DVec3::new(2.0, 2.0, 0.0));
        e.refresh_hover();
        let face = e.hovered().expect("the face");
        e.execute(Command::Erase { ids: vec![face] }).unwrap();
        assert_eq!(
            e.hovered(),
            None,
            "nothing stale between the edit and the next frame"
        );
    }

    #[test]
    fn the_hover_and_the_snap_label_hide_while_flying_and_outside_the_view() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.select_tool(ToolKind::Erase);
        move_to(&mut e, DVec3::new(2.0, 2.0, 0.0));
        e.refresh_hover();
        assert!(e.hovered().is_some());
        e.cursor_left();
        assert_eq!(e.hovered(), None, "the cursor left the view");
        e.select_tool(ToolKind::Line);
        move_to(&mut e, DVec3::new(1.0, 1.0, 0.0));
        e.refresh_hover();
        assert_eq!(e.snap_label().as_deref(), Some("Endpoint"));
        e.toggle_fly();
        assert_eq!(e.snap_label(), None, "no label while flying");
    }

    #[test]
    fn a_window_over_the_view_hides_the_hover_and_the_snap_label() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.select_tool(ToolKind::Erase);
        move_to(&mut e, DVec3::new(2.0, 2.0, 0.0));
        e.refresh_hover();
        let face = e.hovered().expect("the face");
        e.set_covered(true);
        assert_eq!(e.hovered(), None, "a window lies over the cursor");
        e.set_covered(false);
        assert_eq!(e.hovered(), Some(face), "the window moved away");
        e.select_tool(ToolKind::Line);
        move_to(&mut e, DVec3::new(1.0, 1.0, 0.0));
        e.refresh_hover();
        e.set_covered(true);
        assert_eq!(e.snap_label(), None, "no label under a window");
        e.set_covered(false);
        assert_eq!(e.snap_label().as_deref(), Some("Endpoint"));
    }

    #[test]
    fn pulling_a_face_previews_a_translucent_solid() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        e.select_tool(ToolKind::PushPull);
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        move_to(&mut e, DVec3::new(2.0, 2.0, 1.5));
        e.refresh_hover();
        let solid = e.solid_preview();
        assert_eq!(solid.len(), 3 * (2 + 4 * 2));
        assert!(solid.iter().all(|v| v.color[3] == 0.7));
        assert!(
            solid
                .iter()
                .all(|v| v.color[..3] == render::scene::DEFAULT_FRONT[..])
        );
        e.key(EditorKey::Escape);
        e.refresh_hover();
        assert!(e.solid_preview().is_empty());
    }

    #[test]
    fn the_solid_goes_with_a_tool_switch_an_agent_edit_and_fly_mode() {
        let pulling = || {
            let mut e = editor();
            e.select_tool(ToolKind::Line);
            square(&mut e);
            e.select_tool(ToolKind::PushPull);
            click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
            move_to(&mut e, DVec3::new(2.0, 2.0, 1.5));
            e.refresh_hover();
            assert!(!e.solid_preview().is_empty());
            e
        };
        let mut e = pulling();
        e.select_tool(ToolKind::PushPull);
        assert!(e.solid_preview().is_empty(), "a tool switch drops it");
        let mut e = pulling();
        e.execute(Command::Select {
            ids: Vec::new(),
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
        e.execute(Command::AddEdge {
            start: [5.0, 5.0, 0.0],
            end: [6.0, 5.0, 0.0],
        })
        .unwrap();
        assert!(e.solid_preview().is_empty(), "an agent edit drops it");
        let mut e = pulling();
        e.toggle_fly();
        assert!(e.solid_preview().is_empty(), "none while flying");
    }

    #[test]
    fn the_solid_is_in_root_coordinates_inside_a_group() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        square(&mut e);
        let face = e.doc().model().faces().next().unwrap().0;
        let outcome = e
            .execute(Command::MakeGroup {
                ids: vec![EntityId::Face(face)],
            })
            .unwrap();
        let group = outcome
            .changes
            .created
            .iter()
            .find_map(|id| match id {
                EntityId::Instance(i) => Some(*i),
                _ => None,
            })
            .unwrap();
        e.execute(Command::Move {
            ids: vec![EntityId::Instance(group)],
            offset: [10.0, 0.0, 0.0],
        })
        .unwrap();
        e.open_path(vec![EntityId::Instance(group)]);
        e.select_tool(ToolKind::PushPull);
        click_at(&mut e, DVec3::new(12.0, 2.0, 0.0));
        move_to(&mut e, DVec3::new(12.0, 2.0, 1.5));
        e.refresh_hover();
        let solid = e.solid_preview();
        assert!(!solid.is_empty());
        assert!(solid.iter().all(|v| v.pos[0] >= 10.9), "{solid:?}");
    }

    /// A 2 m square on the ground, at (0, 0) to (2, 2).
    fn ground_square(e: &mut Editor) {
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        })
        .unwrap();
    }

    #[test]
    fn select_all_and_none_keep_the_tool() {
        let mut e = editor();
        ground_square(&mut e);
        let face = e.doc().model().faces().next().unwrap().0;
        e.execute(Command::PushPull {
            face: EntityId::Face(face),
            distance: 2.0,
        })
        .unwrap();
        e.select_tool(ToolKind::Rectangle);
        click_at(&mut e, DVec3::new(5.0, 5.0, 0.0));
        assert!(matches!(e.phase(), tools::Phase::Active { .. }));
        assert!(e.select_all());
        let (edges, faces) = stats(&e);
        assert_eq!((edges, faces), (12, 6), "a box");
        assert_eq!(e.doc().selection().len(), edges + faces);
        assert_eq!(e.tool_kind(), ToolKind::Rectangle);
        assert!(
            matches!(e.phase(), tools::Phase::Active { .. }),
            "it goes on"
        );
        assert!(e.select_none());
        assert!(e.doc().selection().is_empty());
        assert!(!e.select_none(), "nothing was selected");
        assert_eq!(e.tool_kind(), ToolKind::Rectangle);
        assert!(matches!(e.phase(), tools::Phase::Active { .. }));
    }

    #[test]
    fn duplicate_moves_a_copy_with_the_cursor() {
        let mut e = editor();
        ground_square(&mut e);
        e.select_all();
        let corner = DVec3::new(2.0, 2.0, 0.0);
        move_to(&mut e, corner);
        let before = stats(&e);
        e.duplicate().unwrap();
        assert_eq!(e.tool_kind(), ToolKind::Move);
        let tools::Phase::Active { points } = e.phase() else {
            panic!("the base point is clicked")
        };
        assert!(points[0].distance(corner) < 1e-9, "{points:?}");
        assert!(
            e.status_parts().prompt.starts_with("Copy:"),
            "{}",
            e.status()
        );
        click_at(&mut e, DVec3::new(5.0, 2.0, 0.0));
        assert_eq!(
            stats(&e),
            (before.0 * 2, before.1 * 2),
            "the original stays"
        );
        assert_eq!(e.doc().undo_label(), Some("copy"));
        e.undo();
        assert_eq!(stats(&e), before, "one undo step");
        // With nothing selected it says so and keeps the tool.
        e.select_none();
        e.select_tool(ToolKind::Line);
        let err = e.duplicate().unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidInput);
        assert_eq!(e.tool_kind(), ToolKind::Line);
        assert_eq!(
            e.status_parts().message.as_deref(),
            Some("Select geometry or a group to duplicate")
        );
    }

    #[test]
    fn duplicate_with_the_cursor_outside_waits_for_a_base_point() {
        let mut e = editor();
        ground_square(&mut e);
        e.select_all();
        move_to(&mut e, DVec3::new(2.0, 2.0, 0.0));
        e.cursor_left();
        e.duplicate().unwrap();
        assert_eq!(e.tool_kind(), ToolKind::Move);
        assert_eq!(e.phase(), tools::Phase::Idle);
        assert_eq!(
            e.status_parts().prompt,
            "Copy: click a base point to copy the selection"
        );
    }

    #[test]
    fn duplicate_inside_an_open_group_copies_within_it() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.select_tool(ToolKind::Select);
        double_click_at(&mut e, DVec3::new(11.0, 1.0, 1.0));
        assert_eq!(e.doc().context().len(), 1, "the group is open");
        e.select_all();
        let inside = (
            e.doc().model().edges().count(),
            e.doc().model().faces().count(),
        );
        assert_eq!(inside, (12, 6));
        move_to(&mut e, DVec3::new(12.0, 2.0, 1.0));
        e.duplicate().unwrap();
        click_at(&mut e, DVec3::new(12.0, 5.0, 1.0));
        assert_eq!(e.doc().undo_label(), Some("copy"));
        let EntityId::Instance(open) = group else {
            panic!("{group:?}")
        };
        assert_eq!(e.doc().context(), [open], "it stays open");
        let m = e.doc().model();
        assert_eq!(
            (m.edges().count(), m.faces().count()),
            (24, 12),
            "the copy is inside the group"
        );
        // The copies are selected, all in the group.
        assert_eq!(e.doc().selection().len(), 18);
        let root = e.doc().scene().root();
        assert_eq!(root.edges().count() + root.faces().count(), 0);
        assert_eq!(root.instances().count(), 1, "no new group outside");
    }

    #[test]
    fn duplicate_copies_selected_guides_with_the_geometry() {
        let mut e = editor();
        ground_square(&mut e);
        e.execute(Command::AddGuidePoint {
            position: [1.0, 1.0, 0.0],
        })
        .unwrap();
        e.execute(Command::AddGuideLine {
            point: [0.0, 3.0, 0.0],
            direction: [1.0, 0.0, 0.0],
        })
        .unwrap();
        e.select_all();
        assert_eq!(e.doc().selection().len(), 4 + 1 + 2, "guides included");
        move_to(&mut e, DVec3::new(2.0, 2.0, 0.0));
        e.duplicate().unwrap();
        click_at(&mut e, DVec3::new(7.0, 2.0, 0.0));
        let m = e.doc().model();
        assert_eq!((m.edges().count(), m.faces().count()), (8, 2));
        assert_eq!(m.guides().count(), 4, "both guides copied");
        assert_eq!(e.doc().selection().len(), 7, "the copies are selected");
        e.undo();
        assert_eq!(e.doc().model().guides().count(), 2, "one undo step");
        assert_eq!(stats(&e), (4, 1));
    }

    #[test]
    fn the_model_is_untouched_until_the_first_edit() {
        let mut e = editor();
        assert!(e.untouched());
        ground_square(&mut e);
        assert!(!e.untouched());
        e.undo();
        assert!(!e.untouched(), "there is a redo");
        e.new_model();
        assert!(e.untouched(), "File > New");
        e.execute(Command::AddGuidePoint {
            position: [1.0, 2.0, 0.0],
        })
        .unwrap();
        assert!(!e.untouched(), "a guide");
        // A group's model open: not the root.
        e.new_model();
        ground_square(&mut e);
        let face = e.doc().model().faces().next().unwrap().0;
        let made = e
            .execute(Command::MakeGroup {
                ids: vec![EntityId::Face(face)],
            })
            .unwrap();
        let group = made
            .changes
            .created
            .into_iter()
            .find(|id| matches!(id, EntityId::Instance(_)))
            .unwrap();
        e.open_path(vec![group]);
        assert!(!e.doc().context().is_empty());
        assert!(!e.untouched());
    }
}
