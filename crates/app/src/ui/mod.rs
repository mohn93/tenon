//! The window's panels: the context bar, the tool rail, the status bar with the measurements
//! box, on the right the dock: Entity Info, Materials, Tags and Scenes as tabs over the
//! Outliner, and the scene tabs above the 3D view. Over the 3D view, the tools' snap marker,
//! preview lines and chip, the tool's cursor and the orientation gizmo.

use std::collections::BTreeSet;
use std::path::PathBuf;

use egui::{Panel, Ui};
use kernel::{
    DefinitionId, EntityId, InstanceId, PageId, StyleId, StyleSettings, TagId, TexturePlacement,
};
use session::{
    Camera, FlipAxis, FlipFrame, IntersectScope, LibraryKind, Lighting, MaterialId, OutlinerTable,
    Projection, SectionDisplay, StandardView, Units,
};
use tools::ToolKind;

use crate::editor::{ImportOptions, PrintOptions, RenameTarget, SelectionTag, Status};
use crate::feedback::Hud;
use crate::navigation::TrackpadScroll;
use crate::platform::Features;
use theme::{ThemeChoice, UiScale};

pub(crate) mod context_bar;
mod dialogs;
mod dock;
mod fonts;
mod icons;
mod library;
mod materials;
mod menu_bar;
mod outliner;
mod rail;
mod recover;
mod right;
mod scene_tabs;
mod scenes;
mod settings;
mod shadows;
mod sheets;
mod start;
mod status;
mod styles;
mod tags;
#[cfg(test)]
mod tests;
pub mod theme;
pub(crate) mod viewer_bar;
pub use viewer_bar::{ViewerPanel, ViewerView};
pub(crate) mod viewport;
pub(crate) use context_bar::step_name;
pub use dock::DockLayout;
pub use dock::DockTab;
#[cfg(not(target_arch = "wasm32"))]
pub use sheets::{PhoneShare, PhoneView};
pub use sheets::{SHADOWS_OFF, Sheet};
pub(crate) use styles::edge_effect_patch;
pub(crate) mod widgets;

/// What the user asked for through the panels this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum UiAction {
    /// The viewer: open a list over the view (Parts, Recent), or close it (None).
    ViewerPanel(Option<ViewerPanel>),
    /// The viewer: select these parts and frame them (a row of the parts list).
    SelectParts(Vec<InstanceId>),
    /// The viewer: hide every part but the selected ones.
    Isolate,
    /// The viewer: show every part again.
    ShowAll,
    /// The viewer: turn Measure on or off.
    MeasureMode(bool),
    /// The viewer: keep the open model on this phone (Recent list).
    KeepModel,
    /// The viewer: open the kept model with this id (Recent list).
    OpenKept(String),
    /// The viewer: forget the kept model with this id.
    DeleteKept(String),
    Tool(ToolKind),
    Undo,
    Redo,
    /// Select everything in the open context, keeping the tool.
    SelectAll,
    /// Select nothing, keeping the tool.
    SelectNone,
    /// Copy the selection with Move, the base point under the cursor.
    Duplicate,
    /// Put the selection on the clipboard and erase it.
    Cut,
    /// Put the selection on the clipboard.
    Copy,
    /// Paste the clipboard's clip with the Paste tool, under the cursor.
    Paste,
    /// Paste the clipboard's clip where it was copied from.
    PasteInPlace,
    EraseSelection,
    MakeGroup,
    MakeComponent,
    Explode,
    /// Group > Make Unique.
    MakeUnique,
    /// Group > Flip Along: mirror the selection along an axis, of the drawing axes or the
    /// one selected component's own.
    Flip(FlipAxis, FlipFrame),
    /// Put the model axes back to the world axes.
    ResetAxes,
    CloseInstance,
    NewModel,
    OpenFile,
    SaveFile,
    SaveFileAs,
    ImportFile,
    /// Import the file the Import Options sheet was opened for, with these options (kept in
    /// the preferences).
    ConfirmImport(ImportOptions),
    /// Close the open sheet without doing anything.
    CloseSheet,
    /// Open the Rename sheet for the one selected group or component.
    RenameComponent,
    /// Rename a definition (Entity Info's Name field).
    RenameDefinition(DefinitionId, String),
    /// The name typed in the Make Component or Rename sheet.
    ConfirmName(String),
    /// Remove the definitions no instance places.
    PurgeUnused,
    /// Open the Field of View sheet.
    ShowFieldOfView,
    /// The Field of View sheet's value, in whole degrees.
    ConfirmFieldOfView(f64),
    ExportFile,
    /// Open the Print sheet to export a PDF (a file is asked for after it).
    ExportPdf,
    /// Open the Print sheet to print and open the PDF in the system's viewer.
    Print,
    /// Print as the Print sheet's values say (kept in the preferences).
    ConfirmPrint(PrintOptions),
    /// Export with the format (an extension) chosen in the Export sheet.
    ConfirmExport(String),
    DeleteGuides,
    ZoomExtents,
    StandardView(StandardView),
    SetProjection(Projection),
    /// Light the view by day or by night.
    SetLighting(Lighting),
    /// Choose the material to paint with (None: the default material), keeping the tool.
    ChooseMaterial(Option<MaterialId>),
    /// Choose the material and the Paint tool (a material tile double-clicked).
    PaintWithMaterial(Option<MaterialId>),
    AddMaterial,
    DeleteMaterial(MaterialId),
    RenameMaterial(MaterialId, String),
    RecolorMaterial(MaterialId, [u8; 3]),
    /// Set a material's opacity, in percent (0 to 100).
    SetMaterialOpacity(MaterialId, u8),
    /// Set how strongly a material glows, in tenths (0 to 100: emission 0 to 10).
    SetMaterialEmission(MaterialId, u8),
    /// Set a material's metalness, in percent (0 to 100).
    SetMaterialMetalness(MaterialId, u8),
    /// Set a material's roughness, in percent (0 to 100: 100 is matte).
    SetMaterialRoughness(MaterialId, u8),
    /// Choose an image file for a material's texture.
    LoadTexture(MaterialId),
    /// Take a material's texture off.
    RemoveTexture(MaterialId),
    /// Set the width and height a material's texture covers, in millimeters.
    SetTextureSize(MaterialId, [u32; 2]),
    /// Draw faces in their tags' colors, or by their materials.
    SetColorByTag(bool),
    /// Show or hide the section planes (View > Section Planes).
    SetSectionPlanes(bool),
    /// Cut or not the geometry the active section planes cut away (View > Section Cuts).
    SetSectionCuts(bool),
    /// Fill the cuts or not (View > Section Fill).
    SetSectionFill(bool),
    /// The selected section plane's context menu: negate its normal.
    FlipSection,
    /// The selected section plane's context menu: make it the active cut, or clear it.
    SetSectionActive(bool),
    /// Close the viewport's right-click menu.
    CloseViewportMenu,
    /// Edit > Hide: hide the selection.
    Hide,
    /// Edit > Unhide > Selected.
    UnhideSelected,
    /// Edit > Unhide > Last: what the latest Hide hid.
    UnhideLast,
    /// Edit > Unhide > All: everything hidden in the open context.
    UnhideAll,
    /// Entity Info's Hidden checkbox: hide (true) or show the selection.
    SetHidden(bool),
    /// Entity Info's "Cut by section planes" checkbox: whether the selected instances are cut
    /// by the section planes of the contexts outside them (`set_section_cut`).
    SetSectionCut(bool),
    /// An Outliner row's menu: hide (true) or show the instance at the path.
    OutlinerSetHidden {
        path: Vec<EntityId>,
        hidden: bool,
    },
    /// View > X-ray: draw every face as glass.
    SetXRay(bool),
    /// View > Hidden Geometry: draw hidden entities ghosted and let them be picked.
    SetHiddenGeometry(bool),
    /// View > Shadows and the Shadows tab's On toggle: sun shadows on or off.
    SetShadowsOn(bool),
    /// The Shadows tab's date, time, levels and ground toggle.
    SetShadows(ShadowChange),
    /// The Shadows tab's location fields: the whole location, one undo step.
    SetLocation(kernel::Location),
    /// Edit > Reverse Faces: turn the selected faces over.
    ReverseFaces,
    /// Orient Faces: make the faces joined to the face (None: the lowest-id selected face) agree
    /// with it.
    OrientFaces(Option<EntityId>),
    AddTag,
    /// Delete a tag: at once when nothing is on it, otherwise after asking where what is on it
    /// goes (the Delete Tag sheet).
    DeleteTag(TagId),
    /// The Delete Tag sheet's destination for what is on the tag (None: Untagged).
    ConfirmDeleteTag(Option<TagId>),
    RenameTag(TagId, String),
    RecolorTag(TagId, [u8; 3]),
    /// Show (true) or hide what is on a tag.
    SetTagVisible(TagId, bool),
    /// Put the selection on a tag (None: Untagged); Entity Info's Tag field.
    SetSelectionTag(Option<TagId>),
    /// Add a style (a copy of this one, or the default settings) and choose it.
    AddStyle(Option<StyleId>),
    DeleteStyle(StyleId),
    RenameStyle(StyleId, String),
    /// Choose the style the view draws with (None: the default look).
    ChooseStyle(Option<StyleId>),
    /// Change parts of a style's settings: one `change_style`, one undo step.
    ChangeStyle(StyleId, session::StylePatch),
    /// View > Edge Style: turn one edge effect of the chosen style on or off.
    ToggleEdgeEffect(EdgeEffect),
    /// Save the current view and tag visibility as a new scene, after the last one.
    AddScene,
    /// Store the current view and tag visibility in a scene again (the parts it stores).
    UpdateScene(PageId),
    /// Start (true: from the current view) or stop storing the camera in a scene; tag
    /// visibility keeps what it stores (`change_scene`).
    StoreSceneCamera(PageId, bool),
    /// Start (true: from the current visibility) or stop storing tag visibility in a scene;
    /// the camera keeps what it stores.
    StoreSceneTags(PageId, bool),
    /// Start (true: the selected instances of the root, or all of them when none is
    /// selected) or stop storing instance positions in a scene.
    StoreScenePositions(PageId, bool),
    RenameScene(PageId, String),
    /// Move a scene to this place in the order (0: first).
    MoveScene(PageId, usize),
    /// Delete a scene, at once (it is undoable).
    DeleteScene(PageId),
    GoToScene(PageId),
    /// Go to the scene before the selected one, wrapping around (Page Up).
    PreviousScene,
    /// Go to the scene after the selected one, wrapping around (Page Down).
    NextScene,
    /// Select the outliner node at `path` (instances from the root down to it); `toggle`
    /// (Shift, Cmd or Ctrl held) adds or removes it instead of replacing the selection.
    OutlinerSelect {
        path: Vec<EntityId>,
        toggle: bool,
    },
    /// Open the instances along `path` for editing (empty: back to the root).
    OpenPath(Vec<EntityId>),
    /// Turn fly mode on or off.
    ToggleFly,
    /// Ask about unsaved changes, then quit.
    Quit,
    /// Open or close the Agent Activity window.
    ShowAgentActivity(bool),
    /// Draw the window in this theme, and keep it in the preferences.
    SetTheme(ThemeChoice),
    /// Open or close the Settings window.
    ShowSettings(bool),
    /// Draw the panels at this scale, and keep it in the preferences.
    SetUiScale(UiScale),
    /// Make plain trackpad scrolling do this, and keep it in the preferences.
    SetTrackpad(TrackpadScroll),
    /// Lay the dock out so (a resize, a divider drag or a tab chosen), and keep it in the
    /// preferences.
    SetDock(DockLayout),
    /// Open or close the Keyboard Shortcuts window.
    ShowShortcuts(bool),
    /// Open the About sheet.
    ShowAbout,
    /// File > Open on Phone: share a copy of the model on the local network and open the sheet
    /// with its QR code.
    OpenOnPhone,
    /// The Open on Phone sheet's Share the new version: seal the model as it is now.
    ShareNewVersion,
    /// The Open on Phone sheet's Stop: stop sharing and close the sheet.
    StopSharing,
    /// Answer the unsaved-changes sheet.
    UnsavedAnswer(crate::platform::UnsavedChoice),
    /// Open or close the Soften Edges window.
    ShowSoften(bool),
    /// Set the Soften Edges window's angle, in degrees (0 to 180).
    SetSoftenAngle(u8),
    /// Soften the selection (or the open context) up to the window's angle.
    Soften,
    /// Harden the selection (or the open context).
    Harden,
    /// Edit > Intersect Faces: cross the selected faces and groups with the scope.
    IntersectFaces(IntersectScope),
    /// Tools > Solid Tools: combine or cut the selected solid groups and components.
    SolidTool(crate::editor::SolidOp),
    /// Make the selected edges soft or hard (Entity Info's Soft checkbox).
    SetSoft(bool),
    /// Set the texture offset of the faces (those selected when the field was edited), in
    /// millimeters along u and v.
    SetTextureOffset(Vec<EntityId>, [i32; 2]),
    /// Set the texture rotation of the faces, in tenths of a degree.
    SetTextureRotation(Vec<EntityId>, i32),
    /// Set the texture scale of the faces, in thousandths.
    SetTextureScale(Vec<EntityId>, [u32; 2]),
    /// Turn the texture of each selected face a further quarter counter-clockwise.
    RotateTexture90,
    /// Clear the texture placement of the selected faces.
    ResetTexturePosition,
    /// Open this recent file (after asking about unsaved changes).
    OpenRecent(PathBuf),
    /// Empty the recent files, and keep that in the preferences.
    ClearRecent,
    /// Hide the empty-model card for the rest of the session.
    DismissStart,
    /// Open (listing the recovery folder again, discarded work too) or close the Recover
    /// Models window.
    ShowRecover(bool),
    /// Open the recovered model whose copy is at this path (after asking about unsaved
    /// changes).
    OpenRecovered(PathBuf),
    /// Ask before discarding the recovered model whose copy is at this path (None: every
    /// one listed).
    DiscardRecovered(Option<PathBuf>),
    /// The Discard Recovered sheet's Discard: remove the copies it names.
    ConfirmDiscardRecovered,
    /// The note sheet's Add or Change: the note's text (blank: a leader note's automatic
    /// text).
    ConfirmNote(String),
    /// Open the 3D Text sheet (Tools > 3D Text...).
    Show3dText,
    /// The 3D Text sheet's Place: the text to place with the cursor.
    Confirm3dText(tools::TextSpec),
    /// Autosave every so many minutes (0: off), and keep it in the preferences.
    SetAutosave(u32),
    /// Set the model's display units (Settings' Model tab), as an undo step.
    SetUnits(Units),
    /// The Model tab's scene transition (on and seconds): one undo step.
    SetSceneTransition(kernel::SceneTransition),
    /// Choose the library folder in a dialog (Settings > Files, the Library tab).
    ChooseLibraryFolder,
    /// Forget the library folder.
    ClearLibraryFolder,
    /// A Library tile clicked: a component or model is placed with the Paste tool, a material
    /// is added and chosen to paint with.
    LibraryUse(PathBuf),
    /// A Library tile's Insert: a component or model at the open context's origin, a material
    /// as a click does.
    LibraryInsert(PathBuf),
    /// A Library tile's Open: the item's file becomes the open model (after asking about
    /// unsaved changes).
    LibraryOpen(PathBuf),
    /// A Library tile's Delete from Library: ask first.
    LibraryDelete(PathBuf),
    /// The Delete from Library sheet's Delete.
    ConfirmLibraryDelete,
    /// The tiles at these paths are in view without a thumbnail.
    ThumbnailsWanted(Vec<PathBuf>),
    /// Open the name sheet to save something to the library.
    SaveToLibrary(LibrarySave),
    /// The save sheet's Save: the name typed.
    ConfirmSaveToLibrary(String),
    /// The overwrite sheet's Replace.
    ConfirmOverwriteLibrary,
}

/// What the Shadows tab changes of the shadow settings besides On: each field as `set_shadows`
/// takes it ("YYYY-MM-DD", "HH:MM", percentages); None leaves it as it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShadowChange {
    pub date: Option<String>,
    pub time: Option<String>,
    pub light: Option<u8>,
    pub dark: Option<u8>,
    pub on_ground: Option<bool>,
}

/// What a library save stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibrarySave {
    /// The component instance.
    Component(InstanceId),
    /// The material.
    Material(MaterialId),
    /// The whole model.
    Model,
}

impl LibrarySave {
    pub fn kind(self) -> LibraryKind {
        match self {
            LibrarySave::Component(_) => LibraryKind::Component,
            LibrarySave::Material(_) => LibraryKind::Material,
            LibrarySave::Model => LibraryKind::Model,
        }
    }
}

/// A library item's thumbnail as its tile shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Thumb {
    Ready(egui::TextureId),
    /// Not made yet: a tile in view asks for it.
    Pending,
    /// Could not be made: the tile shows an icon.
    Failed,
}

/// An item of the library folder as the Library tab lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryRow {
    pub path: PathBuf,
    pub name: String,
    pub kind: LibraryKind,
    /// Why the file cannot be used as an item.
    pub error: Option<String>,
    pub thumb: Thumb,
}

/// What the Library tab shows.
#[derive(Clone, Copy, Debug)]
pub struct LibraryView<'a> {
    /// The library folder; None: not chosen.
    pub folder: Option<&'a std::path::Path>,
    pub rows: &'a [LibraryRow],
    /// The folder holds more items than are listed.
    pub more: bool,
    /// Why the folder could not be listed.
    pub error: Option<&'a str>,
}

/// A material as the Materials list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialRow {
    pub id: MaterialId,
    pub name: String,
    pub color: [u8; 3],
    /// From 0 (invisible) to 1 (opaque).
    pub opacity: f32,
    /// From 0 (no glow) to 10.
    pub emission: f32,
    /// From 0 (not metallic) to 1.
    pub metalness: f32,
    /// From 0 (a mirror) to 1 (matte).
    pub roughness: f32,
    pub texture: Option<TextureRow>,
}

/// A tag as the Tags tab lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct TagRow {
    pub id: TagId,
    pub name: String,
    pub color: [u8; 3],
    pub visible: bool,
    /// How many edges, faces, instances and guides in the whole model are on it.
    pub count: usize,
}

/// An edge effect View > Edge Style toggles on the chosen style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeEffect {
    Profiles,
    Sketchy,
    Endpoints,
}

/// A style as the Styles tab lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleRow {
    pub id: StyleId,
    pub name: String,
    pub settings: StyleSettings,
}

/// A scene as the scene tabs and the Scenes tab list it.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneRow {
    pub id: PageId,
    pub name: String,
    /// Whether it stores the view (camera, projection, lighting, color by tag).
    pub camera: bool,
    /// Whether it stores which tags are hidden.
    pub tags: bool,
    /// Whether it stores instance positions.
    pub positions: bool,
    /// Whether it is the scene last added or gone to.
    pub selected: bool,
}

/// A material's texture as the Materials list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureRow {
    /// A small copy of the image for its swatch.
    pub swatch: egui::TextureId,
    /// The size one copy covers on the model, in meters.
    pub width: f64,
    pub height: f64,
    /// The image's size in pixels.
    pub pixels: [u32; 2],
}

/// What the window shows of the agents using the API.
pub struct Agents<'a> {
    /// Connections open now.
    pub connected: usize,
    /// The activity log's lines, newest first, while the Agent Activity window is open.
    pub log: Option<&'a [String]>,
    /// Recovery files written this session.
    pub recovered: &'a [PathBuf],
}

/// What the panels show.
pub struct View<'a> {
    pub agents: Agents<'a>,
    /// The label of the step Undo would undo (`Document::undo_label`); None: nothing to undo.
    pub undo: Option<&'a str>,
    /// The label of the step Redo would redo.
    pub redo: Option<&'a str>,
    pub tool: ToolKind,
    pub status: &'a Status,
    pub info: &'a [String],
    pub has_selection: bool,
    /// Whether the clipboard holds a clip (`Editor::clip_ready`): Paste is enabled then.
    pub has_clip: bool,
    /// Whether a group or component is selected.
    pub has_instance_selected: bool,
    /// Make Unique has something to do (`Editor::can_make_unique`).
    pub can_make_unique: bool,
    /// Exactly one instance is selected: Flip Along its own axes works.
    pub can_flip_own: bool,
    /// The Tools > Solid Tools items' state for the selection (`Editor::solid_tools`).
    pub solid_tools: crate::editor::SolidTools,
    /// Whether a group or component is open for editing.
    pub inside_instance: bool,
    /// Whether the open context has guides to delete.
    pub has_guides: bool,
    /// The camera's projection, checked in the Camera menu.
    pub projection: Projection,
    /// How the view is lit; Night lighting is checked in the Camera menu at night.
    pub lighting: Lighting,
    /// Whether fly mode is on, checked in the Camera menu.
    pub flying: bool,
    /// Every material, in id order.
    pub materials: &'a [MaterialRow],
    /// The material the Paint tool paints with (None: the default material).
    pub material: Option<MaterialId>,
    /// Every tag, in id order.
    pub tags: &'a [TagRow],
    /// Whether faces are drawn in their tags' colors.
    pub color_by_tag: bool,
    /// Every style, in the order they were made.
    pub styles: &'a [StyleRow],
    /// Whether the renderer can draw ambient occlusion (not in a browser's WebGL2): the Styles
    /// tab says so where it cannot.
    pub ambient_occlusion: bool,
    /// The style the view draws with (None: the default look).
    pub style: Option<StyleId>,
    /// Whether section planes are drawn, cut and filled (checked in the View menu).
    pub section_display: SectionDisplay,
    /// Whether every face is drawn as glass (checked in the View menu).
    pub x_ray: bool,
    /// Whether hidden entities are drawn ghosted and can be picked (checked in the View menu).
    pub hidden_geometry: bool,
    /// The sun shadow settings (the Shadows tab, checked in the View menu).
    pub shadows: kernel::Shadows,
    /// Where the model stands (the Shadows tab's Location fields).
    pub location: kernel::Location,
    /// The viewport's right-click menu, while it is open.
    pub viewport_menu: Option<ViewportMenu>,
    /// Every scene, in order: the scene tabs show while there is one.
    pub scenes: &'a [SceneRow],
    /// How many times the document was replaced (`Editor::documents`): what a panel holds by
    /// id from before names other things after.
    pub documents: u64,
    /// The selection's tag, when everything selected can be tagged (Entity Info's Tag field).
    pub selection_tag: Option<SelectionTag>,
    /// The placed groups and components (`Document::outliner_rows`).
    pub outliner: &'a OutlinerTable,
    /// The open context's instances, from the root down.
    pub context: &'a [EntityId],
    /// The selection, in the open context.
    pub selection: &'a BTreeSet<EntityId>,
    /// The Soften Edges window's angle in degrees while it is open.
    pub soften: Option<u8>,
    /// Whether the selected edges are all soft, when only edges are selected.
    pub edges_soft: Option<bool>,
    /// Whether everything selected that can be hidden is, None when nothing selected can be.
    pub selection_hidden: Option<bool>,
    /// Whether every selected instance is cut by section planes outside it, None when no
    /// instance is selected.
    pub selection_section_cut: Option<bool>,
    /// Whether a selected entity is hidden (Edit > Unhide > Selected).
    pub has_hidden_selected: bool,
    /// Whether the open context has a hidden entity (Edit > Unhide > All).
    pub has_hidden: bool,
    /// Whether Edit > Unhide > Last would show something.
    pub has_last_hidden: bool,
    /// Whether a face is selected (Reverse Faces, Orient Faces).
    pub has_face_selected: bool,
    /// The first selected face's texture placement, when only faces are selected.
    pub texture_position: Option<TexturePosition>,
    /// The theme chosen in View > Theme.
    pub theme: ThemeChoice,
    /// Whether the Keyboard Shortcuts window is open.
    pub shortcuts: bool,
    /// Whether the menus are drawn by egui (otherwise the window has native ones).
    pub egui_menus: bool,
    /// The dock's width, split and tab, as kept in the preferences.
    pub dock: DockLayout,
    /// What is drawn over the 3D view for the tools.
    pub hud: &'a Hud,
    /// The pointer over the 3D view (`viewport::cursor_icon`).
    pub cursor: egui::CursorIcon,
    /// The camera the orientation gizmo follows (its projection is `projection`).
    pub camera: &'a Camera,
    /// The drawing axes' directions in world space as columns: the gizmo's ends are the
    /// standard views along them.
    pub axes: glam::DMat3,
    /// The card over an empty, unedited model; None: hidden.
    pub start: Option<StartCard<'a>>,
    /// What the system offers; the panels hide what is off.
    pub features: Features,
    /// The name the viewer's top bar shows (`Features::read_only`).
    pub model_name: &'a str,
    /// The recent files as kept in the preferences, newest first (File > Open Recent).
    pub recent: &'a [PathBuf],
    /// What the phone viewer's bar and lists show (`Features::read_only`).
    pub viewer: ViewerView<'a>,
    /// The sheet open over the window, if any.
    pub sheet: Option<&'a Sheet>,
    /// The model's display units, which the sheets' lengths follow.
    pub units: Units,
    /// The Library tab's folder and items.
    pub library: LibraryView<'a>,
    /// The one selected component instance, which Save to Library saves.
    pub component: Option<InstanceId>,
    /// The one selected group or component (`Editor::rename_target`): Rename Component works
    /// on it and Entity Info shows its Name field.
    pub rename: Option<&'a RenameTarget>,
    /// What the Settings window shows; Some while it is open.
    pub settings: Option<SettingsView>,
    /// What the Recover Models window lists; Some while it is open.
    pub recover: Option<RecoverView<'a>>,
}

/// The viewport's right-click menu: where it opened (points), the selected section plane's
/// entries (Some: whether the plane is the active cut, so Active Cut is checked) and the
/// Intersect Faces entries (for a selection with faces or groups), Hide (for what can be
/// hidden), Unhide (for a hidden selection) and Reverse Faces and Orient Faces (for faces).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewportMenu {
    pub at: egui::Pos2,
    pub section: Option<bool>,
    pub intersect: bool,
    pub hide: bool,
    pub unhide: bool,
    pub faces: bool,
    /// The selected face under the cursor when the menu opened: Orient Faces works from it.
    pub orient_from: Option<EntityId>,
    /// Make Unique: a selected instance shares its definition.
    pub make_unique: bool,
    /// Flip Along the drawing axes' Red, Green and Blue (a selection).
    pub flip: bool,
    /// Flip Along the component's own axes (one instance selected).
    pub flip_own: bool,
    /// Reset Axes: the model axes are moved and no context is open.
    pub reset_axes: bool,
}

/// What the Recover Models window lists.
#[derive(Clone, Copy, Debug)]
pub struct RecoverView<'a> {
    /// The newest models, newest first.
    pub rows: &'a [RecoverRow],
    /// How many older ones were left out.
    pub older: usize,
}

/// A model the Recover Models window lists.
#[derive(Clone, Debug, PartialEq)]
pub struct RecoverRow {
    /// Its copy in the recovery folder: what Open and Discard act on.
    pub model: PathBuf,
    /// The name it went by: its file's stem, or "Untitled".
    pub name: String,
    /// The file it was a copy of (None: never saved).
    pub original: Option<PathBuf>,
    /// When the copy was written, as "5 min ago".
    pub saved: String,
    /// Unsaved work an agent discarded, rather than an autosave.
    pub discarded: bool,
    /// Why it cannot be opened (it can only be discarded).
    pub error: Option<String>,
}

/// The preferences the Settings window shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SettingsView {
    pub ui_scale: UiScale,
    pub trackpad: TrackpadScroll,
    /// Minutes between autosaves (0: off).
    pub autosave_minutes: u32,
    /// The model's display units (saved with the model, not in the preferences).
    pub units: Units,
    /// How going to a scene animates (saved with the model).
    pub transition: kernel::SceneTransition,
}

/// What the empty-model card lists; `View::start` is None when the card is hidden.
pub struct StartCard<'a> {
    /// The recent files that exist, newest first (at most `prefs::CARD_RECENT`).
    pub recent: &'a [PathBuf],
}

/// What Entity Info's texture position fields show and apply to.
#[derive(Clone, Debug, PartialEq)]
pub struct TexturePosition {
    /// The first selected face's placement.
    pub placement: TexturePlacement,
    /// The selected faces.
    pub faces: Vec<EntityId>,
}

/// The fly mode controls, shown over the Camera menu's Fly mode item.
pub const FLY_HELP: &str = "Walk or fly through the model.\n\
    Mouse: look around (trackpad: two-finger scroll)\n\
    W A S D: forward, left, back, right\n\
    E or Space: up; Q or Ctrl: down\n\
    Wheel, [ ] or - =: slower, faster (trackpad: Cmd+scroll)\n\
    Shift: 3x faster; Option/Alt: slower\n\
    Esc or F: leave fly mode";

/// The dock panel's id.
const DOCK_ID: &str = "dock";

/// The id of the dock panel's resize edge, as egui 0.36 registers it (`Panel::resize_id`, not
/// public): the panel's id with "__resize".
fn dock_resize_id() -> egui::Id {
    egui::Id::new(DOCK_ID).with("__resize")
}

/// Lays out the panels for one pass, appending the actions asked for to `actions`. Returns the
/// rectangle left for the 3D view, in points.
///
/// egui can lay a frame out more than once, and only the first pass sees the frame's input; so
/// a caller keeps every pass's actions and the last pass's rectangle.
pub fn draw(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) -> egui::Rect {
    if view.features.read_only {
        return viewer_bar::draw(ui, view, actions);
    }
    let t = theme::tokens(ui.ctx().theme());
    Panel::top("context")
        .resizable(false)
        .exact_size(theme::CONTEXT_BAR_HEIGHT)
        .frame(
            egui::Frame::new()
                .fill(t.bg_panel)
                .inner_margin(egui::Margin::symmetric(8, 0)),
        )
        .show(ui, |ui| context_bar::context_bar(ui, view, actions));
    Panel::bottom("status")
        .resizable(false)
        .exact_size(theme::STATUS_HEIGHT)
        .frame(
            egui::Frame::new()
                .fill(t.bg_panel)
                .inner_margin(egui::Margin::symmetric(8, 0)),
        )
        .show(ui, |ui| status::status_bar(ui, view, actions));
    Panel::left("rail")
        .resizable(false)
        .exact_size(theme::RAIL_WIDTH)
        .frame(
            egui::Frame::new()
                .fill(t.bg_panel)
                .inner_margin(egui::Margin::symmetric(6, 8)),
        )
        .show(ui, |ui| rail::rail(ui, view, actions));
    let panel = Panel::right(DOCK_ID)
        .resizable(true)
        .default_size(view.dock.width)
        .size_range(dock::DOCK_WIDTH)
        .frame(
            egui::Frame::new()
                .fill(t.bg_panel)
                .inner_margin(egui::Margin::same(dock::DOCK_MARGIN)),
        )
        .show(ui, |ui| dock::dock(ui, view, actions));
    // Between the rail and the dock, right over the 3D view.
    if view.scenes.is_empty() {
        scene_tabs::forget(ui, view);
    } else {
        Panel::top("scene-tabs")
            .resizable(false)
            .exact_size(theme::SCENE_TABS_HEIGHT)
            .frame(
                egui::Frame::new()
                    .fill(t.bg_panel)
                    .inner_margin(egui::Margin::symmetric(8, 0)),
            )
            .show(ui, |ui| scene_tabs::scene_tabs(ui, view, actions));
    }
    widgets::name_egui_widget(
        ui.ctx(),
        dock_resize_id(),
        egui::accesskit::Role::Splitter,
        || "Dock width".into(),
    );
    let mut layout = panel.inner;
    // The width the panel's edge was dragged to, once the drag ends: not while dragging, not a
    // width that only rounding moved, and not a width the panel was held to without a drag
    // (the window too narrow for the saved width).
    let width = panel.response.rect.width();
    let edge_dragged = ui
        .ctx()
        .read_response(dock_resize_id())
        .is_some_and(|r| r.drag_stopped());
    if edge_dragged && (width - view.dock.width).abs() > 0.5 {
        layout.width = width.round();
    }
    if layout != view.dock {
        actions.push(UiAction::SetDock(layout));
    }
    if let Some(angle) = view.soften {
        dialogs::soften_window(ui, angle, actions);
    }
    if let Some(menu) = view.viewport_menu {
        dialogs::viewport_menu(ui, menu, actions);
    }
    if let Some(log) = view.agents.log {
        dialogs::agent_activity(ui, &view.agents, log, actions);
    }
    if view.shortcuts {
        dialogs::shortcuts_window(ui, actions);
    }
    if let Some(recover) = view.recover {
        recover::recover_window(ui, recover.rows, recover.older, actions);
    }
    let central = ui.available_rect_before_wrap();
    // In the root's own layer, under every window and menu.
    viewport::hud(ui, central, view.hud);
    if let Some(card) = &view.start {
        start::start_card(ui, central, card, view.features.recent, actions);
    }
    if !view.flying {
        viewport::gizmo(
            ui,
            central,
            view.camera,
            view.axes,
            view.projection,
            actions,
        );
    }
    viewport::set_cursor(ui.ctx(), central, view.cursor);
    if let Some(settings) = &view.settings {
        settings::settings_window(ui, view, settings, actions);
    }
    // Last, over everything else.
    match view.sheet {
        Some(sheet) => sheets::sheet(ui, sheet, view.units, actions),
        None => sheets::forget(ui),
    }
    central
}
