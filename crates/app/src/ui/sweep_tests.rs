//! The panels' tests as coordinate sweeps; replaced by `ui/tests/` in the next task.

use super::outliner::{OUTLINER_MIN_HEIGHT, outliner_label};
use super::*;
use crate::editor::MATERIAL_PALETTE;
use egui::{Pos2, Rect, Vec2};
use kernel::DefinitionKind;
use session::OutlinerRow;

use super::tests::{Recorded, recorded_id};

static NO_ROWS: OutlinerTable = OutlinerTable {
    roots: Vec::new(),
    by_definition: std::collections::BTreeMap::new(),
};
static NO_SELECTION: BTreeSet<EntityId> = BTreeSet::new();

fn run(ctx: &egui::Context, input: egui::RawInput, view: &View) -> (Vec<UiAction>, Rect) {
    let mut out = None;
    ctx.run_ui(input, |ui| out = Some(draw(ui, view)))
        .drop_without_applying_deltas();
    out.unwrap()
}

/// The window's default size (see `app.rs`); the toolbar fits one row of it.
fn screen() -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
        ..Default::default()
    }
}

fn status() -> Status {
    Status {
        tool: "Line",
        prompt: "Click to start a line".into(),
        label: "Length",
        value: Some("2.5_".into()),
        message: Some("Not a length: x".into()),
        lock: None,
    }
}

/// One frame with a primary click (press, then release in the next frame) at `pos`.
fn click(ctx: &egui::Context, view: &View, pos: Pos2) -> Vec<UiAction> {
    let pointer = |pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let mut press = screen();
    press.events = vec![egui::Event::PointerMoved(pos), pointer(true)];
    let mut actions = run(ctx, press, view).0;
    let mut release = screen();
    release.events = vec![pointer(false)];
    actions.extend(run(ctx, release, view).0);
    actions
}

#[test]
fn the_file_menu_reports_its_items() {
    let ctx = egui::Context::default();
    let status = status();
    let info = Vec::new();
    let view = View {
        tool: ToolKind::Line,
        status: &status,
        info: &info,
        has_selection: false,
        has_instance_selected: false,
        inside_instance: false,
        has_guides: false,
        projection: Projection::Perspective,
        lighting: Lighting::Day,
        flying: false,
        import_units: ImportUnits::Meters,
        materials: &[],
        material: None,
        outliner: &NO_ROWS,
        context: &[],
        selection: &NO_SELECTION,
        agents: no_agents(),
        soften: None,
        edges_soft: None,
        texture_position: None,
    };
    let (_, central) = run(&ctx, screen(), &view);
    let file = Pos2::new(14.0, central.min.y / 2.0);
    let mut clicked = Vec::new();
    // Open the menu, then click one spot below it; repeat down the menu.
    for y in (0..160).step_by(6) {
        click(&ctx, &view, file);
        let _ = run(&ctx, screen(), &view);
        clicked.extend(click(
            &ctx,
            &view,
            Pos2::new(40.0, central.min.y + y as f32),
        ));
        // Close the menu if the click missed every item.
        let mut escape = screen();
        escape.events = vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }];
        let _ = run(&ctx, escape, &view);
    }
    for action in [
        UiAction::NewModel,
        UiAction::OpenFile,
        UiAction::SaveFile,
        UiAction::SaveFileAs,
        UiAction::ImportFile,
        UiAction::ExportFile,
    ] {
        assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
    }
}

/// The actions produced by opening the menu under each x in `xs` on the toolbar row and
/// clicking one spot below it, down the menu.
fn menu_actions(view: &View, xs: std::ops::Range<i32>) -> Vec<UiAction> {
    let ctx = egui::Context::default();
    let (_, central) = run(&ctx, screen(), view);
    let row = central.min.y / 2.0;
    let mut clicked = Vec::new();
    for x in xs.step_by(12) {
        for y in (0..300).step_by(8) {
            click(&ctx, view, Pos2::new(x as f32, row));
            let _ = run(&ctx, screen(), view);
            clicked.extend(click(
                &ctx,
                view,
                Pos2::new(x as f32 + 10.0, central.min.y + y as f32),
            ));
            let mut escape = screen();
            escape.events = vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }];
            let _ = run(&ctx, escape, view);
        }
    }
    clicked
}

fn no_agents() -> Agents<'static> {
    Agents {
        connected: 0,
        log: None,
        recovered: &[],
    }
}

/// Every text the frame draws, with the middle of where it lies.
fn texts(ctx: &egui::Context, view: &View) -> Vec<(String, Pos2)> {
    fn collect(shape: &egui::Shape, out: &mut Vec<(String, Pos2)>) {
        match shape {
            egui::Shape::Text(t) => {
                let middle = t.pos + t.galley.rect.center().to_vec2();
                out.push((t.galley.text().to_string(), middle));
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    let output = ctx.run_ui(screen(), |ui| {
        draw(ui, view);
    });
    for clipped in &output.shapes {
        collect(&clipped.shape, &mut found);
    }
    output.drop_without_applying_deltas();
    found
}

fn text_at(ctx: &egui::Context, view: &View, text: &str) -> Option<Pos2> {
    texts(ctx, view)
        .into_iter()
        .find(|(t, _)| t == text)
        .map(|(_, p)| p)
}

#[test]
fn the_view_menu_and_the_indicator_open_agent_activity() {
    let status = status();
    let info = Vec::new();
    let mut view = soften_view(&status, &info, None, None);
    // View menu, right of Camera.
    let ctx = egui::Context::default();
    let _ = run(&ctx, screen(), &view);
    let camera = text_at(&ctx, &view, "Camera").expect("the Camera menu");
    let at = text_at(&ctx, &view, "View").expect("the View menu");
    assert!(at.x > camera.x, "{at:?} right of {camera:?}");
    // Open the menu, then click its item, which lies just below the toolbar row.
    click(&ctx, &view, at);
    let _ = run(&ctx, screen(), &view);
    let item = text_at(&ctx, &view, "Agent Activity").expect("the menu's item");
    let clicked = click(&ctx, &view, item);
    assert!(
        clicked.contains(&UiAction::ShowAgentActivity(true)),
        "{clicked:?}"
    );

    // Status bar: nothing at 0 agents, "Agents: 2" at 2, and a click on it opens the log.
    let ctx = egui::Context::default();
    let _ = run(&ctx, screen(), &view);
    let drawn = texts(&ctx, &view);
    assert!(drawn.len() > 10, "a laid-out frame: {drawn:?}");
    assert!(
        !drawn.iter().any(|(t, _)| t.starts_with("Agents:")),
        "{drawn:?}"
    );
    view.agents.connected = 2;
    let ctx = egui::Context::default();
    let _ = run(&ctx, screen(), &view);
    let at = text_at(&ctx, &view, "Agents: 2").expect("the indicator is drawn");
    assert!(at.y > 700.0, "{at:?}");
    let clicked = click(&ctx, &view, at);
    assert_eq!(clicked, vec![UiAction::ShowAgentActivity(true)]);

    // The window lists the log and the recovery files, and closes.
    let lines = ["5 s ago  #1  execute save /a.skr  ok".to_string()];
    let recovered = [PathBuf::from("/r/untitled-1.skr")];
    view.agents = Agents {
        connected: 2,
        log: Some(&lines),
        recovered: &recovered,
    };
    let ctx = egui::Context::default();
    let _ = run(&ctx, screen(), &view);
    let shown = texts(&ctx, &view);
    for want in [
        "2 agents connected",
        "5 s ago  #1  execute save /a.skr  ok",
        "/r/untitled-1.skr",
    ] {
        assert!(shown.iter().any(|(t, _)| t == want), "{want} in {shown:?}");
    }
    // The close button sits at the right end of the title bar, whose text is at y 33.
    let title = text_at(&ctx, &view, "Agent Activity").expect("the window's title");
    let mut clicked = Vec::new();
    for y in (title.y as i32 - 8..title.y as i32 + 8).step_by(4) {
        for x in (title.x as i32..700).step_by(6) {
            clicked.extend(click(&ctx, &view, Pos2::new(x as f32, y as f32)));
        }
    }
    assert!(
        clicked.contains(&UiAction::ShowAgentActivity(false)),
        "{clicked:?}"
    );
}

#[test]
fn the_camera_menu_has_views_zoom_extents_and_projections() {
    let status = status();
    let info = Vec::new();
    let mut view = View {
        tool: ToolKind::Line,
        status: &status,
        info: &info,
        has_selection: false,
        has_instance_selected: false,
        inside_instance: false,
        has_guides: false,
        projection: Projection::Perspective,
        lighting: Lighting::Day,
        flying: false,
        import_units: ImportUnits::Meters,
        materials: &[],
        material: None,
        outliner: &NO_ROWS,
        context: &[],
        selection: &NO_SELECTION,
        agents: no_agents(),
        soften: None,
        edges_soft: None,
        texture_position: None,
    };
    // The Camera menu sits right after File and Edit.
    let clicked = menu_actions(&view, 80..140);
    assert!(clicked.contains(&UiAction::ZoomExtents), "{clicked:?}");
    for v in StandardView::ALL {
        assert!(
            clicked.contains(&UiAction::StandardView(v)),
            "{v:?} in {clicked:?}"
        );
    }
    for p in [Projection::Perspective, Projection::Parallel] {
        assert!(
            clicked.contains(&UiAction::SetProjection(p)),
            "{p:?} in {clicked:?}"
        );
    }
    assert!(clicked.contains(&UiAction::ToggleFly), "{clicked:?}");
    assert!(
        clicked.contains(&UiAction::SetLighting(Lighting::Night)),
        "{clicked:?}"
    );
    view.projection = Projection::Parallel;
    view.lighting = Lighting::Night;
    let clicked = menu_actions(&view, 80..140);
    assert!(clicked.contains(&UiAction::SetProjection(Projection::Perspective)));
    // Unchecking Night lighting goes back to day.
    assert!(
        clicked.contains(&UiAction::SetLighting(Lighting::Day)),
        "{clicked:?}"
    );
}

fn edit_menu_actions(has_selection: bool, has_guides: bool) -> Vec<UiAction> {
    let status = status();
    let info = Vec::new();
    let view = View {
        tool: ToolKind::Line,
        status: &status,
        info: &info,
        has_selection,
        has_instance_selected: false,
        inside_instance: false,
        has_guides,
        projection: Projection::Perspective,
        lighting: Lighting::Day,
        flying: false,
        import_units: ImportUnits::Meters,
        materials: &[],
        material: None,
        outliner: &NO_ROWS,
        context: &[],
        selection: &NO_SELECTION,
        agents: no_agents(),
        soften: None,
        edges_soft: None,
        texture_position: None,
    };
    // The Edit button sits right after File (which ends before x = 40).
    menu_actions(&view, 40..80)
}

#[test]
fn the_edit_menu_erases_the_selection_and_deletes_guides() {
    let clicked = edit_menu_actions(true, true);
    assert!(clicked.contains(&UiAction::EraseSelection), "{clicked:?}");
    assert!(clicked.contains(&UiAction::DeleteGuides), "{clicked:?}");
    assert!(clicked.contains(&UiAction::ShowSoften(true)), "{clicked:?}");
}

#[test]
fn the_edit_menu_items_need_a_selection_and_guides() {
    let clicked = edit_menu_actions(false, false);
    assert!(!clicked.contains(&UiAction::EraseSelection), "{clicked:?}");
    assert!(!clicked.contains(&UiAction::DeleteGuides), "{clicked:?}");
    assert!(
        clicked.contains(&UiAction::ShowSoften(true)),
        "Soften Edges... works on the open context: {clicked:?}"
    );
}

/// A view with nothing selected, the Soften Edges window as `soften` says and the Soft
/// checkbox as `edges_soft` says.
fn soften_view<'a>(
    status: &'a Status,
    info: &'a [String],
    soften: Option<u8>,
    edges_soft: Option<bool>,
) -> View<'a> {
    View {
        tool: ToolKind::Line,
        status,
        info,
        has_selection: false,
        has_instance_selected: false,
        inside_instance: false,
        has_guides: false,
        projection: Projection::Perspective,
        lighting: Lighting::Day,
        flying: false,
        import_units: ImportUnits::Meters,
        materials: &[],
        material: None,
        outliner: &NO_ROWS,
        context: &[],
        selection: &NO_SELECTION,
        agents: no_agents(),
        soften,
        edges_soft,
        texture_position: None,
    }
}

#[test]
fn the_soften_window_sets_its_angle_softens_hardens_and_closes() {
    let status = status();
    let info = Vec::new();
    let view = soften_view(&status, &info, Some(20), None);
    let ctx = egui::Context::default();
    let _ = run(&ctx, screen(), &view);
    let mut clicked = Vec::new();
    // The window opens around the middle of the 1280 x 800 window.
    for y in (300..500).step_by(4) {
        for x in (480..800).step_by(8) {
            clicked.extend(click(&ctx, &view, Pos2::new(x as f32, y as f32)));
        }
    }
    for action in [
        UiAction::Soften,
        UiAction::Harden,
        UiAction::ShowSoften(false),
    ] {
        assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
    }
    let angles: Vec<u8> = clicked
        .iter()
        .filter_map(|a| match a {
            UiAction::SetSoftenAngle(angle) => Some(*angle),
            _ => None,
        })
        .collect();
    assert!(angles.iter().any(|a| *a < 20), "{angles:?}");
    assert!(angles.iter().any(|a| *a > 90), "{angles:?}");
    assert!(angles.iter().all(|a| *a <= 180), "{angles:?}");

    let closed = soften_view(&status, &info, None, None);
    let ctx = egui::Context::default();
    let mut clicked = Vec::new();
    for y in (300..500).step_by(8) {
        for x in (480..800).step_by(16) {
            clicked.extend(click(&ctx, &closed, Pos2::new(x as f32, y as f32)));
        }
    }
    assert!(
        !clicked.iter().any(|a| matches!(
            a,
            UiAction::Soften | UiAction::Harden | UiAction::SetSoftenAngle(_)
        )),
        "{clicked:?}"
    );
}

#[test]
fn entity_info_shows_a_soft_checkbox_for_selected_edges() {
    let status = status();
    let info = vec!["1 edge".to_string()];
    let hard = soften_view(&status, &info, None, Some(false));
    let clicked = panel_actions(&hard, 1280.0 - 160.0, None);
    assert!(clicked.contains(&UiAction::SetSoft(true)), "{clicked:?}");
    let soft = soften_view(&status, &info, None, Some(true));
    let clicked = panel_actions(&soft, 1280.0 - 160.0, None);
    assert!(clicked.contains(&UiAction::SetSoft(false)), "{clicked:?}");
    let none = soften_view(&status, &info, None, None);
    let clicked = panel_actions(&none, 1280.0 - 160.0, None);
    assert!(
        !clicked.iter().any(|a| matches!(a, UiAction::SetSoft(_))),
        "{clicked:?}"
    );
}

#[test]
fn entity_info_shows_texture_position_fields_for_selected_faces() {
    let status = status();
    let info = vec!["2 faces".to_string()];
    let mut view = soften_view(&status, &info, None, None);
    let faces = vec![
        EntityId::Face(kernel::FaceId(3)),
        EntityId::Face(kernel::FaceId(4)),
    ];
    view.texture_position = Some(TexturePosition {
        placement: TexturePlacement::default(),
        faces: faces.clone(),
    });
    let columns = [160.0, 120.0, 80.0];
    let clicked: Vec<UiAction> = columns
        .into_iter()
        .flat_map(|x| panel_actions(&view, 1280.0 - x, None))
        .collect();
    for action in [UiAction::RotateTexture90, UiAction::ResetTexturePosition] {
        assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
    }
    // Typing 2 into each field sets that field alone, in its own units.
    let typed: Vec<UiAction> = columns
        .into_iter()
        .flat_map(|x| panel_actions(&view, 1280.0 - x, Some("2")))
        .collect();
    for action in [
        UiAction::SetTextureOffset(faces.clone(), [2000, 0]),
        UiAction::SetTextureOffset(faces.clone(), [0, 2000]),
        UiAction::SetTextureRotation(faces.clone(), 20),
        UiAction::SetTextureScale(faces.clone(), [2000, 1000]),
        UiAction::SetTextureScale(faces.clone(), [1000, 2000]),
    ] {
        assert!(typed.contains(&action), "{action:?} in {typed:?}");
    }
    // Without faces selected there are no texture fields.
    view.texture_position = None;
    let clicked: Vec<UiAction> = columns
        .into_iter()
        .flat_map(|x| panel_actions(&view, 1280.0 - x, Some("2")))
        .collect();
    assert!(
        !clicked.iter().any(|a| matches!(
            a,
            UiAction::SetTextureOffset(..)
                | UiAction::SetTextureRotation(..)
                | UiAction::SetTextureScale(..)
                | UiAction::RotateTexture90
                | UiAction::ResetTexturePosition
        )),
        "{clicked:?}"
    );
}

#[test]
fn a_texture_field_applies_to_the_faces_it_was_edited_for() {
    // Typing into a field with face 3 selected, then entering it once face 4 (placed
    // otherwise) is selected, sets face 3 from face 3's values.
    let status = status();
    let info = vec!["1 face".to_string()];
    let mut before = soften_view(&status, &info, None, None);
    let mut after = soften_view(&status, &info, None, None);
    let (three, four) = (
        vec![EntityId::Face(kernel::FaceId(3))],
        vec![EntityId::Face(kernel::FaceId(4))],
    );
    before.texture_position = Some(TexturePosition {
        placement: TexturePlacement::default(),
        faces: three.clone(),
    });
    after.texture_position = Some(TexturePosition {
        placement: TexturePlacement {
            offset: [0.5, 0.75],
            rotation_deg: 30.0,
            scale: [3.0, 3.0],
        },
        faces: four.clone(),
    });
    let mut typed = Vec::new();
    for x in [160.0, 120.0, 80.0] {
        let ctx = egui::Context::default();
        for y in (0..800).step_by(4) {
            typed.extend(click(&ctx, &before, Pos2::new(1280.0 - x, y as f32)));
            let mut typing = screen();
            typing.events = vec![egui::Event::Text("2".into())];
            typed.extend(run(&ctx, typing, &before).0);
            let mut ending = screen();
            ending.events = vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }];
            typed.extend(run(&ctx, ending, &after).0);
            typed.extend(run(&ctx, screen(), &after).0);
        }
    }
    for action in [
        UiAction::SetTextureOffset(three.clone(), [2000, 0]),
        UiAction::SetTextureOffset(three.clone(), [0, 2000]),
        UiAction::SetTextureRotation(three.clone(), 20),
        UiAction::SetTextureScale(three.clone(), [2000, 1000]),
        UiAction::SetTextureScale(three.clone(), [1000, 2000]),
    ] {
        assert!(typed.contains(&action), "{action:?} in {typed:?}");
    }
    assert!(
        !typed.iter().any(|a| matches!(
            a,
            UiAction::SetTextureOffset(f, _)
                | UiAction::SetTextureRotation(f, _)
                | UiAction::SetTextureScale(f, _) if *f == four
        )),
        "{typed:?}"
    );
}

/// A frame pressing `key`.
fn key_frame(key: egui::Key) -> egui::RawInput {
    let mut frame = screen();
    frame.events = vec![egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }];
    frame
}

#[test]
fn a_stepped_texture_value_applies_when_its_fields_go_away() {
    // Stepping a field with the arrow keys on face 3, then selecting something else (the
    // fields go away), applies the stepped value to face 3.
    let status = status();
    let info = vec!["1 face".to_string()];
    let mut faces = soften_view(&status, &info, None, None);
    let three = vec![EntityId::Face(kernel::FaceId(3))];
    faces.texture_position = Some(TexturePosition {
        placement: TexturePlacement::default(),
        faces: three.clone(),
    });
    let other = soften_view(&status, &info, None, None);
    let mut acted = Vec::new();
    for x in [160.0, 120.0, 80.0] {
        let ctx = egui::Context::default();
        for y in (0..800).step_by(4) {
            click(&ctx, &faces, Pos2::new(1280.0 - x, y as f32));
            for _ in 0..2 {
                run(&ctx, key_frame(egui::Key::ArrowUp), &faces);
            }
            let mut after = run(&ctx, screen(), &other).0;
            after.extend(run(&ctx, screen(), &other).0);
            // Faces selected again later: nothing left over applies then.
            let later = run(&ctx, screen(), &faces).0;
            assert!(
                !later.iter().any(|a| matches!(
                    a,
                    UiAction::SetTextureOffset(..)
                        | UiAction::SetTextureRotation(..)
                        | UiAction::SetTextureScale(..)
                )),
                "{later:?}"
            );
            acted.extend(after.into_iter().filter(|a| {
                matches!(
                    a,
                    UiAction::SetTextureOffset(..)
                        | UiAction::SetTextureRotation(..)
                        | UiAction::SetTextureScale(..)
                )
            }));
        }
    }
    assert!(
        acted.contains(&UiAction::SetTextureRotation(three.clone(), 5)),
        "{acted:?}"
    );
    assert!(
        acted.iter().all(|a| matches!(
            a,
            UiAction::SetTextureOffset(f, _)
                | UiAction::SetTextureRotation(f, _)
                | UiAction::SetTextureScale(f, _) if *f == three
        )),
        "{acted:?}"
    );
}

#[test]
fn panels_leave_a_central_view_inside_the_window() {
    let ctx = egui::Context::default();
    let status = status();
    let info = vec!["Nothing selected".to_string()];
    let view = View {
        tool: ToolKind::Line,
        status: &status,
        info: &info,
        has_selection: false,
        has_instance_selected: false,
        inside_instance: false,
        has_guides: false,
        projection: Projection::Perspective,
        lighting: Lighting::Day,
        flying: false,
        import_units: ImportUnits::Meters,
        materials: &[],
        material: None,
        outliner: &NO_ROWS,
        context: &[],
        selection: &NO_SELECTION,
        agents: no_agents(),
        soften: None,
        edges_soft: None,
        texture_position: None,
    };
    let (actions, rect) = run(&ctx, screen(), &view);
    assert!(actions.is_empty());
    assert!(
        rect.min.x == 0.0 && rect.min.y > 0.0,
        "toolbar above: {rect:?}"
    );
    assert!(rect.max.x < 1280.0, "info panel on the right: {rect:?}");
    assert!(rect.max.y < 800.0, "status bar below: {rect:?}");
    assert!(rect.width() > 600.0 && rect.height() > 500.0, "{rect:?}");
}

/// A `width` x 800 point window.
fn screen_of(width: f32) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 800.0))),
        ..Default::default()
    }
}

/// Runs `f` with a view that has every toolbar button enabled.
fn with_toolbar_view<R>(f: impl FnOnce(&View) -> R) -> R {
    let status = status();
    let info = Vec::new();
    let view = View {
        tool: ToolKind::Line,
        status: &status,
        info: &info,
        has_selection: true,
        has_instance_selected: true,
        inside_instance: true,
        has_guides: true,
        projection: Projection::Perspective,
        lighting: Lighting::Day,
        flying: false,
        import_units: ImportUnits::Meters,
        materials: &[],
        material: None,
        outliner: &NO_ROWS,
        context: &[],
        selection: &NO_SELECTION,
        agents: no_agents(),
        soften: None,
        edges_soft: None,
        texture_position: None,
    };
    f(&view)
}

/// The 3D view's rectangle after one layout pass of a `width`-point window; its top is
/// the toolbar's height.
fn central_of(width: f32) -> Rect {
    with_toolbar_view(|view| run(&egui::Context::default(), screen_of(width), view).1)
}

/// Clicks over the whole toolbar of a `width`-point window, every 4 points across and 6
/// down. Returns the actions and the toolbar's height (the 3D view's top).
fn toolbar_clicks(width: f32) -> (Vec<UiAction>, f32) {
    with_toolbar_view(|view| toolbar_clicks_in(view, width))
}

fn toolbar_clicks_in(view: &View, width: f32) -> (Vec<UiAction>, f32) {
    let ctx = egui::Context::default();
    let (_, central) = run(&ctx, screen_of(width), view);
    let mut clicked = Vec::new();
    for y in (3..central.min.y as i32).step_by(6) {
        for x in (2..width as i32).step_by(4) {
            let pos = Pos2::new(x as f32, y as f32);
            let pointer = |pressed: bool| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            let mut press = screen_of(width);
            press.events = vec![egui::Event::PointerMoved(pos), pointer(true)];
            let _ = run(&ctx, press, view);
            let mut release = screen_of(width);
            release.events = vec![pointer(false)];
            clicked.extend(run(&ctx, release, view).0);
        }
    }
    (clicked, central.min.y)
}

/// Every toolbar button (not the menus) is in `clicked`.
fn assert_every_toolbar_button(clicked: &[UiAction], width: f32) {
    for kind in ToolKind::ALL {
        assert!(
            clicked.contains(&UiAction::Tool(kind)),
            "{kind:?} at {width} points: {clicked:?}"
        );
    }
    for action in [
        UiAction::Undo,
        UiAction::Redo,
        UiAction::MakeGroup,
        UiAction::MakeComponent,
        UiAction::Explode,
        UiAction::CloseInstance,
    ] {
        assert!(
            clicked.contains(&action),
            "{action:?} at {width} points: {clicked:?}"
        );
    }
}

#[test]
fn clicking_a_tool_button_reports_it() {
    let (clicked, height) = toolbar_clicks(1280.0);
    assert_every_toolbar_button(&clicked, 1280.0);
    // The default window keeps the toolbar on one row, as a very wide one does.
    let wide = central_of(2400.0).min.y;
    assert_eq!(height, wide, "one toolbar row at 1280 points");
}

#[test]
fn a_narrow_window_wraps_the_toolbar_and_keeps_every_button() {
    let one_row = central_of(2400.0).min.y;
    for width in [900.0, 640.0] {
        let (clicked, height) = toolbar_clicks(width);
        assert_every_toolbar_button(&clicked, width);
        assert!(
            height > one_row + 10.0,
            "the toolbar wraps at {width} points: {height} vs {one_row}"
        );
        assert!(height < 200.0, "{height} points of toolbar at {width}");
        let central = central_of(width);
        assert!(
            central.width() > 0.0 && central.height() > 0.0,
            "a 3D view is left at {width} points: {central:?}"
        );
    }
}

/// Clicks down the right panel at `x`, every 4 points; after each click, types `typed`
/// and presses Enter. Returns every action.
fn panel_actions(view: &View, x: f32, typed: Option<&str>) -> Vec<UiAction> {
    panel_actions_ending(view, x, typed, egui::Key::Enter)
}

/// `panel_actions`, ending what was typed with `end` instead of Enter.
fn panel_actions_ending(view: &View, x: f32, typed: Option<&str>, end: egui::Key) -> Vec<UiAction> {
    let ctx = egui::Context::default();
    let mut clicked = Vec::new();
    for y in (0..800).step_by(4) {
        clicked.extend(click(&ctx, view, Pos2::new(x, y as f32)));
        if let Some(text) = typed {
            let mut typing = screen();
            typing.events = vec![egui::Event::Text(text.into())];
            clicked.extend(run(&ctx, typing, view).0);
            let mut ending = screen();
            ending.events = vec![egui::Event::Key {
                key: end,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }];
            clicked.extend(run(&ctx, ending, view).0);
            clicked.extend(run(&ctx, screen(), view).0);
        }
    }
    clicked
}

#[test]
fn the_materials_list_chooses_adds_renames_recolors_and_deletes() {
    let status = status();
    let info = vec!["Nothing selected".to_string()];
    let rows = [
        MaterialRow {
            id: MaterialId(31),
            name: "Brick".into(),
            color: [180, 60, 40],
            opacity: 1.0,
            emission: 0.0,
            texture: None,
        },
        MaterialRow {
            id: MaterialId(32),
            name: "Glass".into(),
            color: [90, 170, 170],
            opacity: 0.4,
            emission: 0.0,
            texture: None,
        },
    ];
    let mut view = View {
        tool: ToolKind::Line,
        status: &status,
        info: &info,
        has_selection: false,
        has_instance_selected: false,
        inside_instance: false,
        has_guides: false,
        projection: Projection::Perspective,
        lighting: Lighting::Day,
        flying: false,
        import_units: ImportUnits::Meters,
        materials: &rows,
        material: Some(MaterialId(31)),
        outliner: &NO_ROWS,
        context: &[],
        selection: &NO_SELECTION,
        agents: no_agents(),
        soften: None,
        edges_soft: None,
        texture_position: None,
    };
    // Down the panel's left part: swatches and names, Add, the name field, the palette.
    let clicked = panel_actions(&view, 1280.0 - 160.0, None);
    for action in [
        UiAction::ChooseMaterial(None),
        UiAction::ChooseMaterial(Some(MaterialId(31))),
        UiAction::ChooseMaterial(Some(MaterialId(32))),
        UiAction::AddMaterial,
        UiAction::RecolorMaterial(MaterialId(31), MATERIAL_PALETTE[0]),
    ] {
        assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
    }
    // Further right: Delete, and typing into the name field renames on Enter.
    let clicked = panel_actions(&view, 1280.0 - 120.0, Some("Old"));
    assert!(
        clicked.contains(&UiAction::DeleteMaterial(MaterialId(31))),
        "{clicked:?}"
    );
    assert!(
        clicked.iter().any(|a| matches!(
            a,
            UiAction::RenameMaterial(MaterialId(31), name) if name.contains("Old")
        )),
        "{clicked:?}"
    );
    // Clicking on the opacity slider sets the opacity, on the Glow slider the emission.
    assert!(
        clicked
            .iter()
            .any(|a| matches!(a, UiAction::SetMaterialOpacity(MaterialId(31), p) if *p < 100)),
        "{clicked:?}"
    );
    assert!(
        clicked
            .iter()
            .any(|a| matches!(a, UiAction::SetMaterialEmission(MaterialId(31), t) if *t > 0)),
        "{clicked:?}"
    );
    // With the default material chosen there is nothing to delete, rename or recolor.
    view.material = None;
    let clicked = panel_actions(&view, 1280.0 - 120.0, Some("Old"));
    assert!(
        !clicked.iter().any(|a| matches!(
            a,
            UiAction::DeleteMaterial(_)
                | UiAction::RenameMaterial(..)
                | UiAction::RecolorMaterial(..)
                | UiAction::SetMaterialOpacity(..)
                | UiAction::SetMaterialEmission(..)
        )),
        "{clicked:?}"
    );
}

#[test]
fn the_right_panel_keeps_its_width_with_a_texture_shown() {
    let status = status();
    let info = vec!["Nothing selected".to_string()];
    let plain = brick();
    let mut textured = brick();
    textured[0].texture = Some(TextureRow {
        swatch: egui::TextureId::default(),
        width: 123.456,
        height: 789.012,
        pixels: [4096, 4096],
    });
    let ctx = egui::Context::default();
    let width = |rows: &[MaterialRow]| {
        let view = name_field_view(&status, &info, rows);
        let mut w = 0.0;
        // A few frames, so egui settles any size it remembers.
        for _ in 0..3 {
            w = run(&ctx, screen(), &view).1.width();
        }
        w
    };
    let before = width(&plain);
    assert_eq!(width(&textured), before, "a texture shown");
    assert_eq!(width(&plain), before, "after the texture");
    // Long texture position values do not widen it either (extreme ones are clipped).
    let mut view = name_field_view(&status, &info, &textured);
    view.texture_position = Some(TexturePosition {
        placement: TexturePlacement {
            offset: [-123456.789, -987654.321],
            rotation_deg: 359.9,
            scale: [999.999, 999.999],
        },
        faces: vec![EntityId::Face(kernel::FaceId(3))],
    });
    for _ in 0..3 {
        assert_eq!(
            run(&ctx, screen(), &view).1.width(),
            before,
            "texture position"
        );
    }
}

#[test]
fn the_materials_list_loads_removes_and_sizes_textures() {
    let status = status();
    let info = vec!["Nothing selected".to_string()];
    let rows = brick();
    let view = name_field_view(&status, &info, &rows);
    // Without a texture: Texture... only.
    let clicked: Vec<UiAction> = [160.0, 120.0, 80.0]
        .into_iter()
        .flat_map(|x| panel_actions(&view, 1280.0 - x, None))
        .collect();
    assert!(
        clicked.contains(&UiAction::LoadTexture(MaterialId(31))),
        "{clicked:?}"
    );
    assert!(
        !clicked
            .iter()
            .any(|a| matches!(a, UiAction::RemoveTexture(_) | UiAction::SetTextureSize(..))),
        "{clicked:?}"
    );
    let mut textured = brick();
    textured[0].texture = Some(TextureRow {
        swatch: egui::TextureId::default(),
        width: 1.0,
        height: 0.5,
        pixels: [2, 1],
    });
    let view = name_field_view(&status, &info, &textured);
    let clicked: Vec<UiAction> = [160.0, 120.0, 80.0]
        .into_iter()
        .flat_map(|x| panel_actions(&view, 1280.0 - x, None))
        .collect();
    for action in [
        UiAction::ChooseMaterial(Some(MaterialId(31))),
        UiAction::LoadTexture(MaterialId(31)),
        UiAction::RemoveTexture(MaterialId(31)),
    ] {
        assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
    }
    // Typing 2 into the width field, then the height field, sets the size in millimeters.
    let typed: Vec<UiAction> = [160.0, 120.0, 80.0]
        .into_iter()
        .flat_map(|x| panel_actions(&view, 1280.0 - x, Some("2")))
        .collect();
    for size in [[2000, 500], [1000, 2000]] {
        let action = UiAction::SetTextureSize(MaterialId(31), size);
        assert!(typed.contains(&action), "{action:?} in {typed:?}");
    }
}

#[test]
fn a_typed_texture_size_is_one_change_when_entered() {
    let status = status();
    let info = vec!["Nothing selected".to_string()];
    let mut textured = brick();
    textured[0].texture = Some(TextureRow {
        swatch: egui::TextureId::default(),
        width: 1.0,
        height: 0.5,
        pixels: [2, 1],
    });
    let view = name_field_view(&status, &info, &textured);
    let sizes = |actions: &[UiAction]| {
        actions
            .iter()
            .filter(|a| matches!(a, UiAction::SetTextureSize(..)))
            .cloned()
            .collect::<Vec<_>>()
    };
    let mut entered = Vec::new();
    for x in [160.0, 120.0] {
        let ctx = egui::Context::default();
        for y in (0..800).step_by(4) {
            let mut typing = click(&ctx, &view, Pos2::new(1280.0 - x, y as f32));
            for key in ["0", ".", "2", "5"] {
                let mut frame = screen();
                frame.events = vec![egui::Event::Text(key.into())];
                typing.extend(run(&ctx, frame, &view).0);
            }
            assert!(sizes(&typing).is_empty(), "while typing: {typing:?}");
            let mut ending = screen();
            ending.events = vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }];
            let mut done = run(&ctx, ending, &view).0;
            done.extend(run(&ctx, screen(), &view).0);
            let done = sizes(&done);
            assert!(done.len() <= 1, "{done:?}");
            entered.extend(done);
        }
    }
    for size in [[250, 500], [1000, 250]] {
        let action = UiAction::SetTextureSize(MaterialId(31), size);
        assert!(entered.contains(&action), "{action:?} in {entered:?}");
    }
}

#[test]
fn a_dragged_texture_size_applies_to_its_material_when_another_is_chosen() {
    let status = status();
    let info = vec!["Nothing selected".to_string()];
    let mut textured = brick();
    textured[0].texture = Some(TextureRow {
        swatch: egui::TextureId::default(),
        width: 1.0,
        height: 0.5,
        pixels: [2, 1],
    });
    let chosen = name_field_view(&status, &info, &textured);
    let mut default = name_field_view(&status, &info, &textured);
    default.material = None;
    let sizes = |actions: Vec<UiAction>| {
        actions
            .into_iter()
            .filter(|a| matches!(a, UiAction::SetTextureSize(..)))
            .collect::<Vec<_>>()
    };
    let mut switched = Vec::new();
    for x in [160.0, 120.0] {
        let ctx = egui::Context::default();
        for y in (0..800).step_by(4) {
            // Drag a field 40 points and, still holding the button, choose another
            // material (as the API or a shortcut might).
            let at = Pos2::new(1280.0 - x, y as f32);
            let mut press = screen();
            press.events = vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ];
            run(&ctx, press, &chosen);
            for dx in [10.0, 20.0, 40.0] {
                let mut drag = screen();
                drag.events = vec![egui::Event::PointerMoved(at + Vec2::new(dx, 0.0))];
                run(&ctx, drag, &chosen);
            }
            switched.extend(sizes(run(&ctx, screen(), &default).0));
            let mut release = screen();
            release.events = vec![egui::Event::PointerButton {
                pos: at + Vec2::new(40.0, 0.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }];
            run(&ctx, release, &default);
            let later = sizes(run(&ctx, screen(), &chosen).0);
            assert!(later.is_empty(), "{later:?}");
        }
    }
    assert!(
        switched
            .iter()
            .any(|a| matches!(a, UiAction::SetTextureSize(MaterialId(31), _))),
        "{switched:?}"
    );
}

/// Two materials, the first chosen, for the name-field tests.
fn name_field_view<'a>(
    status: &'a Status,
    info: &'a [String],
    rows: &'a [MaterialRow],
) -> View<'a> {
    View {
        tool: ToolKind::Line,
        status,
        info,
        has_selection: false,
        has_instance_selected: false,
        inside_instance: false,
        has_guides: false,
        projection: Projection::Perspective,
        lighting: Lighting::Day,
        flying: false,
        import_units: ImportUnits::Meters,
        materials: rows,
        material: Some(MaterialId(31)),
        outliner: &NO_ROWS,
        context: &[],
        selection: &NO_SELECTION,
        agents: no_agents(),
        soften: None,
        edges_soft: None,
        texture_position: None,
    }
}

fn brick() -> [MaterialRow; 1] {
    [MaterialRow {
        id: MaterialId(31),
        name: "Brick".into(),
        color: [180, 60, 40],
        opacity: 1.0,
        emission: 0.0,
        texture: None,
    }]
}

#[test]
fn dragging_the_opacity_slider_sets_the_opacity_once_when_it_ends() {
    let status = status();
    let info = vec!["Nothing selected".to_string()];
    let rows = brick();
    let view = name_field_view(&status, &info, &rows);
    // The slider's row: where a click near the panel's right end sets an opacity.
    let y = (0..800)
        .step_by(2)
        .map(|y| y as f32)
        .find(|y| {
            let ctx = egui::Context::default();
            run(&ctx, screen(), &view);
            click(&ctx, &view, Pos2::new(1280.0 - 80.0, *y))
                .iter()
                .any(|a| matches!(a, UiAction::SetMaterialOpacity(..)))
        })
        .expect("found the slider")
        + 6.0;
    let ctx = egui::Context::default();
    run(&ctx, screen(), &view);
    let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let start = Pos2::new(1280.0 - 82.0, y);
    let mut actions = Vec::new();
    let mut input = screen();
    input.events = vec![egui::Event::PointerMoved(start), button(start, true)];
    actions.extend(run(&ctx, input, &view).0);
    for k in 1..=10 {
        let mut input = screen();
        let at = Pos2::new(start.x - 3.0 * k as f32, y);
        input.events = vec![egui::Event::PointerMoved(at)];
        actions.extend(run(&ctx, input, &view).0);
    }
    assert!(actions.is_empty(), "nothing while dragging: {actions:?}");
    let end = Pos2::new(start.x - 30.0, y);
    let mut input = screen();
    input.events = vec![button(end, false)];
    actions.extend(run(&ctx, input, &view).0);
    actions.extend(run(&ctx, screen(), &view).0);
    assert!(
        matches!(actions.as_slice(), [UiAction::SetMaterialOpacity(MaterialId(31), p)] if *p < 100),
        "{actions:?}"
    );
}

#[test]
fn escape_in_the_material_name_field_discards_the_edit() {
    let status = status();
    let info = vec!["Nothing selected".to_string()];
    let rows = brick();
    let view = name_field_view(&status, &info, &rows);
    let entered = panel_actions(&view, 1280.0 - 120.0, Some("Old"));
    assert!(
        entered
            .iter()
            .any(|a| matches!(a, UiAction::RenameMaterial(..))),
        "Enter renames: {entered:?}"
    );
    let escaped = panel_actions_ending(&view, 1280.0 - 120.0, Some("Old"), egui::Key::Escape);
    assert!(
        !escaped
            .iter()
            .any(|a| matches!(a, UiAction::RenameMaterial(..))),
        "{escaped:?}"
    );
}

#[test]
fn deleting_a_material_does_not_also_rename_it() {
    let status = status();
    let info = vec!["Nothing selected".to_string()];
    let rows = brick();
    let view = name_field_view(&status, &info, &rows);
    let x = 1280.0 - 120.0;
    let row_of = |wanted: fn(&UiAction) -> bool, typed: Option<&str>| {
        (0..800)
            .step_by(4)
            .find(|y| {
                let ctx = egui::Context::default();
                run(&ctx, screen(), &view);
                let mut actions = click(&ctx, &view, Pos2::new(x, *y as f32));
                if let Some(text) = typed {
                    let mut typing = screen();
                    typing.events = vec![
                        egui::Event::Text(text.into()),
                        egui::Event::Key {
                            key: egui::Key::Enter,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Default::default(),
                        },
                    ];
                    actions.extend(run(&ctx, typing, &view).0);
                    actions.extend(run(&ctx, screen(), &view).0);
                }
                actions.iter().any(wanted)
            })
            .expect("found the row") as f32
    };
    let delete_y = row_of(|a| matches!(a, UiAction::DeleteMaterial(_)), None);
    let field_y = row_of(|a| matches!(a, UiAction::RenameMaterial(..)), Some("Old"));
    // Type into the field, then press and release on Delete in one frame.
    let ctx = egui::Context::default();
    run(&ctx, screen(), &view);
    let mut actions = click(&ctx, &view, Pos2::new(x, field_y));
    let mut typing = screen();
    typing.events = vec![egui::Event::Text("Old".into())];
    actions.extend(run(&ctx, typing, &view).0);
    let pos = Pos2::new(x, delete_y);
    let pointer = |pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let mut both = screen();
    both.events = vec![
        egui::Event::PointerMoved(pos),
        pointer(true),
        pointer(false),
    ];
    actions.extend(run(&ctx, both, &view).0);
    actions.extend(run(&ctx, screen(), &view).0);
    assert!(
        actions.contains(&UiAction::DeleteMaterial(MaterialId(31))),
        "{actions:?}"
    );
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, UiAction::RenameMaterial(..))),
        "{actions:?}"
    );
}

/// Lays out one frame and returns the Outliner's rows.
fn rows(ctx: &egui::Context, view: &View) -> Vec<Recorded> {
    run(ctx, screen(), view);
    ctx.data(|d| d.get_temp::<Vec<Recorded>>(recorded_id()))
        .unwrap_or_default()
}

/// The name row (not the expand button) at `path`.
fn row<'a>(rows: &'a [Recorded], path: &[EntityId]) -> Option<&'a Recorded> {
    rows.iter().find(|r| !r.toggle && r.path == path)
}

fn row_at(rows: &[Recorded], path: &[EntityId]) -> Pos2 {
    row(rows, path)
        .unwrap_or_else(|| panic!("a row for {path:?} in {rows:?}"))
        .rect
        .center()
}

/// One click at `pos` with `modifiers` held: press, then release in the next frame.
fn click_with(
    ctx: &egui::Context,
    view: &View,
    pos: Pos2,
    modifiers: egui::Modifiers,
) -> Vec<UiAction> {
    let pointer = |pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    };
    let mut press = screen();
    press.events = vec![
        egui::Event::ModifiersChanged(modifiers),
        egui::Event::PointerMoved(pos),
        pointer(true),
    ];
    let mut actions = run(ctx, press, view).0;
    let mut release = screen();
    release.events = vec![egui::Event::ModifiersChanged(modifiers), pointer(false)];
    actions.extend(run(ctx, release, view).0);
    actions
}

fn table_row(id: &str, definition: &str, kind: DefinitionKind, name: &str) -> OutlinerRow {
    OutlinerRow {
        id: id.parse().unwrap(),
        definition: definition.parse().unwrap(),
        kind,
        name: name.into(),
        material: None,
    }
}

/// Two "Box" components (definition d1) each placing the group i27 (definition d2).
fn boxes() -> OutlinerTable {
    let component = |id| table_row(id, "d1", DefinitionKind::Component, "Box");
    let group = table_row("i27", "d2", DefinitionKind::Group, "");
    OutlinerTable {
        roots: vec![component("i55"), component("i56")],
        by_definition: [
            ("d1".parse().unwrap(), vec![group]),
            ("d2".parse().unwrap(), Vec::new()),
        ]
        .into(),
    }
}

fn ids(ids: &[&str]) -> Vec<EntityId> {
    ids.iter().map(|id| id.parse().unwrap()).collect()
}

fn outliner_view<'a>(
    status: &'a Status,
    info: &'a [String],
    table: &'a OutlinerTable,
    context: &'a [EntityId],
    selection: &'a BTreeSet<EntityId>,
) -> View<'a> {
    View {
        tool: ToolKind::Line,
        status,
        info,
        has_selection: !selection.is_empty(),
        has_instance_selected: !selection.is_empty(),
        inside_instance: !context.is_empty(),
        has_guides: false,
        projection: Projection::Perspective,
        lighting: Lighting::Day,
        flying: false,
        import_units: ImportUnits::Meters,
        materials: &[],
        material: None,
        outliner: table,
        context,
        selection,
        agents: no_agents(),
        soften: None,
        edges_soft: None,
        texture_position: None,
    }
}

#[test]
fn outliner_labels_name_groups_and_components() {
    let table = boxes();
    assert_eq!(outliner_label(&table.roots[0]), "<Box>");
    assert_eq!(
        outliner_label(&table.children(table.roots[0].definition)[0]),
        "Group"
    );
    let named = table_row("i3", "d4", DefinitionKind::Group, "Wall");
    assert_eq!(outliner_label(&named), "Wall");
}

#[test]
fn outliner_rows_show_the_open_path_and_the_selection_and_select_their_paths() {
    let status = status();
    let info = vec!["1 group".to_string()];
    let table = boxes();
    let (context, selection) = (ids(&["i55"]), ids(&["i27"]).into_iter().collect());
    let view = outliner_view(&status, &info, &table, &context, &selection);
    let laid = rows(&egui::Context::default(), &view);
    // The first box is open, so expanded; the second starts collapsed.
    let names: Vec<&[EntityId]> = laid
        .iter()
        .filter(|r| !r.toggle)
        .map(|r| r.path.as_slice())
        .collect();
    assert_eq!(
        names,
        [ids(&[]), ids(&["i55"]), ids(&["i55", "i27"]), ids(&["i56"])]
    );
    let marks = |path: &[&str]| {
        let r = row(&laid, &ids(path)).unwrap();
        (r.open, r.selected)
    };
    assert_eq!(marks(&["i55"]), (true, false));
    assert_eq!(marks(&["i55", "i27"]), (false, true));
    assert_eq!(marks(&["i56"]), (false, false));

    for (path, modifiers, toggle) in [
        (&["i55"][..], egui::Modifiers::NONE, false),
        (&["i55", "i27"], egui::Modifiers::NONE, false),
        (&["i56"], egui::Modifiers::NONE, false),
        (&["i56"], egui::Modifiers::SHIFT, true),
        (&["i56"], egui::Modifiers::COMMAND, true),
        (&["i56"], egui::Modifiers::CTRL, true),
    ] {
        // A fresh context for each, so no two clicks make a double-click.
        let ctx = egui::Context::default();
        run(&ctx, screen(), &view);
        let clicked = click_with(&ctx, &view, row_at(&laid, &ids(path)), modifiers);
        let wanted = UiAction::OutlinerSelect {
            path: ids(path),
            toggle,
        };
        assert_eq!(clicked, [wanted], "{path:?} with {modifiers:?}");
    }
}

#[test]
fn the_model_row_goes_back_to_the_root_only_from_inside() {
    let status = status();
    let info = Vec::new();
    let table = boxes();
    let selection = BTreeSet::new();
    let inside = ids(&["i55"]);
    for (context, wanted) in [
        (&inside[..], vec![UiAction::OpenPath(Vec::new())]),
        (&[][..], Vec::new()),
    ] {
        let view = outliner_view(&status, &info, &table, context, &selection);
        let ctx = egui::Context::default();
        let laid = rows(&ctx, &view);
        let model = row(&laid, &[]).unwrap();
        assert_eq!(model.open, context.is_empty(), "bold at the root");
        assert_eq!(click(&ctx, &view, model.rect.center()), wanted);
    }
}

#[test]
fn double_clicking_an_outliner_row_opens_it_and_expanding_shows_children() {
    let status = status();
    let info = Vec::new();
    let table = boxes();
    let (context, selection) = (ids(&["i55"]), BTreeSet::new());
    let view = outliner_view(&status, &info, &table, &context, &selection);
    let ctx = egui::Context::default();
    let laid = rows(&ctx, &view);
    let inner = row_at(&laid, &ids(&["i55", "i27"]));
    let mut actions = click(&ctx, &view, inner);
    actions.extend(click(&ctx, &view, inner));
    assert!(
        actions.contains(&UiAction::OpenPath(ids(&["i55", "i27"]))),
        "{actions:?}"
    );

    // The expand button left of the second box shows its group.
    let ctx = egui::Context::default();
    let laid = rows(&ctx, &view);
    let button = laid
        .iter()
        .find(|r| r.toggle && r.path == ids(&["i56"]))
        .expect("the second box has an expand button");
    assert_eq!(click(&ctx, &view, button.rect.center()), []);
    let laid = rows(&ctx, &view);
    let wanted = UiAction::OutlinerSelect {
        path: ids(&["i56", "i27"]),
        toggle: false,
    };
    // Let the opening animation finish before clicking the new row.
    let laid = (0..30).fold(laid, |_, _| rows(&ctx, &view));
    assert_eq!(
        click(&ctx, &view, row_at(&laid, &ids(&["i56", "i27"]))),
        [wanted]
    );
}

#[test]
fn a_newly_opened_path_expands_its_ancestors() {
    let status = status();
    let info = Vec::new();
    let table = boxes();
    let selection = BTreeSet::new();
    let ctx = egui::Context::default();
    let root = outliner_view(&status, &info, &table, &[], &selection);
    let laid = rows(&ctx, &root);
    assert!(row(&laid, &ids(&["i56", "i27"])).is_none(), "collapsed");
    let context = ids(&["i56", "i27"]);
    let inside = outliner_view(&status, &info, &table, &context, &selection);
    let laid = (0..30).fold(Vec::new(), |_, _| rows(&ctx, &inside));
    let group = row(&laid, &context).expect("the open group shows");
    assert!(group.open);
    // Collapsing it by hand sticks while the context stays the same.
    let button = laid
        .iter()
        .find(|r| r.toggle && r.path == ids(&["i56"]))
        .unwrap();
    click(&ctx, &inside, button.rect.center());
    let laid = (0..30).fold(Vec::new(), |_, _| rows(&ctx, &inside));
    assert!(row(&laid, &context).is_none(), "{laid:?}");
}

#[test]
fn a_short_window_leaves_the_outliner_its_minimum_height() {
    let status = status();
    let info = vec!["1 group".to_string()];
    let materials: Vec<MaterialRow> = (0..40)
        .map(|k| MaterialRow {
            id: MaterialId(100 + k),
            name: format!("Material {k}"),
            color: [10, 20, 30],
            opacity: 1.0,
            emission: 0.0,
            texture: None,
        })
        .collect();
    let table = OutlinerTable {
        roots: (0..40)
            .map(|k| table_row(&format!("i{}", 200 + k), "d1", DefinitionKind::Group, ""))
            .collect(),
        by_definition: Default::default(),
    };
    let selection = BTreeSet::new();
    let mut view = outliner_view(&status, &info, &table, &[], &selection);
    view.materials = &materials;
    view.material = Some(MaterialId(100));
    let height = 520.0;
    let ctx = egui::Context::default();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, height))),
        ..Default::default()
    };
    run(&ctx, input(), &view);
    run(&ctx, input(), &view);
    let laid = ctx
        .data(|d| d.get_temp::<Vec<Recorded>>(recorded_id()))
        .unwrap();
    let model = row(&laid, &[]).unwrap();
    assert!(
        model.rect.min.y <= height - OUTLINER_MIN_HEIGHT,
        "the Outliner starts at {} in a {height} point window",
        model.rect.min.y
    );
}

#[test]
fn a_very_short_window_scrolls_the_right_panel() {
    let status = status();
    let info = vec!["1 group".to_string()];
    let table = OutlinerTable {
        roots: vec![table_row("i200", "d1", DefinitionKind::Group, "")],
        by_definition: Default::default(),
    };
    let selection = BTreeSet::new();
    let materials: Vec<MaterialRow> = (0..40)
        .map(|k| MaterialRow {
            id: MaterialId(100 + k),
            name: format!("Material {k}"),
            color: [10, 20, 30],
            opacity: 1.0,
            emission: 0.0,
            texture: None,
        })
        .collect();
    let mut view = outliner_view(&status, &info, &table, &[], &selection);
    view.materials = &materials;
    view.material = Some(MaterialId(100));
    let height = 300.0;
    let ctx = egui::Context::default();
    let frame = |k: f64, events: Vec<egui::Event>| egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, height))),
        time: Some(k * 0.1),
        events,
        ..Default::default()
    };
    let model_y = |ctx: &egui::Context| {
        let laid = ctx
            .data(|d| d.get_temp::<Vec<Recorded>>(recorded_id()))
            .unwrap();
        row(&laid, &[]).unwrap().rect.min.y
    };
    run(&ctx, frame(0.0, vec![]), &view);
    run(&ctx, frame(1.0, vec![]), &view);
    let before = model_y(&ctx);
    assert!(
        before > height,
        "the Outliner is below the window at {before}"
    );
    // Scroll with the pointer over Entity Info.
    let over = Pos2::new(1200.0, 60.0);
    let wheel = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: Vec2::new(0.0, -400.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    };
    run(
        &ctx,
        frame(2.0, vec![egui::Event::PointerMoved(over), wheel]),
        &view,
    );
    for k in 3..30 {
        run(&ctx, frame(k as f64, vec![]), &view);
    }
    let after = model_y(&ctx);
    assert!(
        after < before - 100.0,
        "the panel scrolled from {before} to {after}"
    );
}
