//! The panels' tests, driven through egui_kittest: widgets are found by their names in the
//! AccessKit tree, so the tests do not depend on where the layout puts them.

mod entity;
mod layout;
mod materials;
mod menus;
mod outliner;
mod widgets;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use egui::{Pos2, Rect, Ui, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use kernel::{EntityId, TexturePlacement};
use session::{Lighting, MaterialId, OutlinerTable, Projection};
use tools::ToolKind;

use super::theme::{self, ThemeChoice};
use super::{Agents, MaterialRow, TexturePosition, TextureRow, UiAction, View, draw};
use crate::editor::{ImportUnits, Status};

/// Everything a test's `View` borrows, owned, and what the panels reported.
pub(super) struct Fixture {
    pub undo: Option<String>,
    pub redo: Option<String>,
    pub tool: ToolKind,
    pub status: Status,
    pub info: Vec<String>,
    pub has_selection: bool,
    pub has_instance_selected: bool,
    pub inside_instance: bool,
    pub has_guides: bool,
    pub projection: Projection,
    pub lighting: Lighting,
    pub flying: bool,
    pub import_units: ImportUnits,
    pub materials: Vec<MaterialRow>,
    pub material: Option<MaterialId>,
    pub outliner: OutlinerTable,
    pub context: Vec<EntityId>,
    pub selection: BTreeSet<EntityId>,
    pub connected: usize,
    pub log: Option<Vec<String>>,
    pub recovered: Vec<PathBuf>,
    pub soften: Option<u8>,
    pub edges_soft: Option<bool>,
    pub texture_position: Option<TexturePosition>,
    pub theme: ThemeChoice,
    /// Whether the menus are egui's (the window has no native ones).
    pub egui_menus: bool,
    /// Every action the panels reported since the last `take`.
    pub actions: Vec<UiAction>,
    /// The 3D view's rectangle in the last frame.
    pub central: Rect,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            undo: None,
            redo: None,
            tool: ToolKind::Line,
            status: Status {
                tool: "Line",
                prompt: "Click to start a line".into(),
                label: "Length",
                value: Some("2.5_".into()),
                message: Some("Not a length: x".into()),
                lock: None,
            },
            info: vec!["Nothing selected".into()],
            has_selection: false,
            has_instance_selected: false,
            inside_instance: false,
            has_guides: false,
            projection: Projection::Perspective,
            lighting: Lighting::Day,
            flying: false,
            import_units: ImportUnits::Meters,
            materials: Vec::new(),
            material: None,
            outliner: OutlinerTable {
                roots: Vec::new(),
                by_definition: BTreeMap::new(),
            },
            context: Vec::new(),
            selection: BTreeSet::new(),
            connected: 0,
            log: None,
            recovered: Vec::new(),
            soften: None,
            edges_soft: None,
            texture_position: None,
            theme: ThemeChoice::System,
            egui_menus: true,
            actions: Vec::new(),
            central: Rect::NOTHING,
        }
    }
}

impl Fixture {
    pub fn view(&self) -> View<'_> {
        View {
            undo: self.undo.as_deref(),
            redo: self.redo.as_deref(),
            agents: Agents {
                connected: self.connected,
                log: self.log.as_deref(),
                recovered: &self.recovered,
            },
            tool: self.tool,
            status: &self.status,
            info: &self.info,
            has_selection: self.has_selection,
            has_instance_selected: self.has_instance_selected,
            inside_instance: self.inside_instance,
            has_guides: self.has_guides,
            projection: self.projection,
            lighting: self.lighting,
            flying: self.flying,
            import_units: self.import_units,
            materials: &self.materials,
            material: self.material,
            outliner: &self.outliner,
            context: &self.context,
            selection: &self.selection,
            soften: self.soften,
            edges_soft: self.edges_soft,
            texture_position: self.texture_position.clone(),
            theme: self.theme,
            egui_menus: self.egui_menus,
        }
    }

    /// The actions reported since the last call.
    pub fn take(&mut self) -> Vec<UiAction> {
        std::mem::take(&mut self.actions)
    }
}

pub(super) type H = Harness<'static, Fixture>;

/// The window's default size (see `app.rs`).
pub(super) const SCREEN: Vec2 = Vec2::new(1280.0, 800.0);

pub(super) fn harness(f: Fixture) -> H {
    harness_sized(f, SCREEN)
}

/// A harness drawing the panels for `f` in a window of `size` points, settled, with nothing
/// reported yet. Frames are 50 ms apart, so two clicks in a row are a double-click.
pub(super) fn harness_sized(f: Fixture, size: Vec2) -> H {
    let mut h = Harness::builder()
        .with_size(size)
        .with_step_dt(0.05)
        .build_ui_state(
            |ui: &mut Ui, f: &mut Fixture| {
                // A root over the whole window, as `Context::run_ui` gives the app: the
                // harness's own Ui has a margin.
                let ctx = ui.ctx().clone();
                if themed_first_pass(&ctx) {
                    return;
                }
                let mut root = Ui::new(
                    ctx.clone(),
                    egui::Id::new("panels"),
                    egui::UiBuilder::new()
                        .layer_id(egui::LayerId::background())
                        .max_rect(ctx.viewport_rect()),
                );
                // As `Gui::frame` does: every pass appends, the last pass's rectangle stays.
                let mut actions = std::mem::take(&mut f.actions);
                f.central = draw(&mut root, &f.view(), &mut actions);
                f.actions = actions;
            },
            f,
        );
    h.run_ok();
    // Laying the panels out reports nothing by itself.
    assert_eq!(h.state().actions, [], "actions from the first frames");
    h.state_mut().actions.clear();
    h
}

/// On the first pass of a harness, which runs before a test can install the theme: installs it
/// (the fonts take effect on the next pass) and asks for that pass. True when the caller should
/// draw nothing in this one.
pub(super) fn themed_first_pass(ctx: &egui::Context) -> bool {
    let key = egui::Id::new("themed-first-pass");
    if ctx.data(|d| d.get_temp::<bool>(key)).is_some() {
        return false;
    }
    ctx.data_mut(|d| d.insert_temp(key, true));
    install_theme(ctx);
    ctx.request_discard("the theme's fonts");
    true
}

/// The dark theme, without animations as the harness set it up, so one run settles everything.
pub(super) fn install_theme(ctx: &egui::Context) {
    theme::install(ctx, ThemeChoice::Dark);
    ctx.all_styles_mut(|s| {
        s.animation_time = 0.0;
        s.scroll_animation = egui::style::ScrollAnimation::none();
    });
}

/// Clicks the widget named `label` and runs the frames that follow. Returns the actions.
pub(super) fn click(h: &mut H, label: &str) -> Vec<UiAction> {
    h.get_by_label(label).click();
    h.run_ok();
    h.state_mut().take()
}

/// Opens the menus along `path` (a title, then submenus) and clicks the last entry. An item
/// may share its name with a rail or bar button, so the item clicked is the node with that
/// name that was not there before the last menu opened.
pub(super) fn click_menu(h: &mut H, path: &[&str]) -> Vec<UiAction> {
    let (item, menus) = path.split_last().expect("a menu path");
    let (last, before) = menus.split_last().expect("a menu title");
    for m in before {
        click(h, m);
    }
    let there: Vec<Rect> = h.query_all_by_label(item).map(|n| n.rect()).collect();
    click(h, last);
    let node = h
        .get_all_by_label(item)
        .find(|n| !there.contains(&n.rect()))
        .unwrap_or_else(|| panic!("{item} in {path:?}"));
    node.click();
    h.run_ok();
    h.state_mut().take()
}

/// Presses and releases the primary button at `pos` with `modifiers` held, then runs.
pub(super) fn click_at(h: &mut H, pos: Pos2, modifiers: egui::Modifiers) -> Vec<UiAction> {
    h.event(egui::Event::PointerMoved(pos));
    // egui reads held modifiers from the input state, not from the button event. A plain
    // click adds no frames, so two in a row stay a double-click.
    let held = modifiers != egui::Modifiers::NONE;
    if held {
        h.event(egui::Event::ModifiersChanged(modifiers));
    }
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        });
    }
    if held {
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    }
    h.run_ok();
    h.state_mut().take()
}

/// Clicks the `k`th field named `label`, types `text`, presses `end` and runs. Returns the
/// actions.
pub(super) fn type_into(
    h: &mut H,
    label: &str,
    k: usize,
    text: &str,
    end: egui::Key,
) -> Vec<UiAction> {
    h.get_all_by_label(label)
        .nth(k)
        .unwrap_or_else(|| panic!("field {k} named {label}"))
        .click();
    h.run_ok();
    h.event(egui::Event::Text(text.into()));
    h.run_ok();
    h.key_press(end);
    h.run_ok();
    h.state_mut().take()
}

/// Where the widget named `label` is, in points.
pub(super) fn rect_of(h: &H, label: &str) -> Rect {
    h.get_by_label(label).rect()
}

/// Whether the selectable widget named `label` is on. egui reports a button's selected state
/// to AccessKit as toggled, not as selected.
pub(super) fn is_on<S>(h: &Harness<'_, S>, label: &str) -> Option<bool> {
    use egui::accesskit::Toggled;
    h.get_by_label(label)
        .accesskit_node()
        .toggled()
        .map(|t| t == Toggled::True)
}

/// Every text the last frame painted (tooltips and shortcut texts are not in the AccessKit
/// tree under their own names).
pub(super) fn drawn_texts(h: &H) -> Vec<String> {
    drawn_texts_of(h)
}

/// `drawn_texts` for a harness with any state.
pub(super) fn drawn_texts_of<S>(h: &Harness<'_, S>) -> Vec<String> {
    fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &h.output().shapes {
        collect(&clipped.shape, &mut out);
    }
    out
}

/// A row the Outliner laid out in the last frame: a node's name (or the Model row, with an
/// empty path) or a node's expand button, with the marks it was drawn with.
#[derive(Clone, Debug)]
pub(super) struct Recorded {
    pub path: Vec<EntityId>,
    pub rect: Rect,
    pub toggle: bool,
    pub selected: bool,
    pub open: bool,
}

/// The egui memory key the Outliner records its rows under.
pub(super) fn recorded_id() -> egui::Id {
    egui::Id::new("outliner-test-rows")
}

/// Records a row the Outliner laid out (called from `outliner.rs` in tests).
pub(super) fn record(
    ui: &Ui,
    path: &[EntityId],
    rect: Rect,
    toggle: bool,
    selected: bool,
    open: bool,
) {
    let row = Recorded {
        path: path.to_vec(),
        rect,
        toggle,
        selected,
        open,
    };
    ui.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<Recorded>>(recorded_id())
            .push(row)
    });
}

/// The Outliner's rows in the last frame.
pub(super) fn rows(h: &H) -> Vec<Recorded> {
    h.ctx
        .data(|d| d.get_temp::<Vec<Recorded>>(recorded_id()))
        .unwrap_or_default()
}

/// Faces 3, 4, ... as entity ids.
pub(super) fn faces(ids: &[u64]) -> Vec<EntityId> {
    ids.iter()
        .map(|&k| EntityId::Face(kernel::FaceId(k)))
        .collect()
}

/// The Brick material, id 31, untextured.
pub(super) fn brick() -> MaterialRow {
    MaterialRow {
        id: MaterialId(31),
        name: "Brick".into(),
        color: [180, 60, 40],
        opacity: 1.0,
        emission: 0.0,
        texture: None,
    }
}

/// A texture shown with a material: 1 x 0.5 m, 2 x 1 px.
pub(super) fn small_texture() -> TextureRow {
    TextureRow {
        swatch: egui::TextureId::default(),
        width: 1.0,
        height: 0.5,
        pixels: [2, 1],
    }
}

/// Faces selected with their texture placement shown in Entity Info.
pub(super) fn with_texture_position(faces: Vec<EntityId>, placement: TexturePlacement) -> Fixture {
    Fixture {
        info: vec![format!("{} faces", faces.len())],
        texture_position: Some(TexturePosition { placement, faces }),
        ..Default::default()
    }
}
