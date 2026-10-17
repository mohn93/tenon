use super::*;
use crate::editor::MATERIAL_PALETTE;
use crate::ui::materials::palette_label;

/// Brick (chosen) and Glass.
fn two_materials() -> Fixture {
    let mut glass = brick();
    glass.id = MaterialId(32);
    glass.name = "Glass".into();
    glass.color = [90, 170, 170];
    glass.opacity = 0.4;
    Fixture {
        materials: vec![brick(), glass],
        material: Some(MaterialId(31)),
        ..Default::default()
    }
}

/// Brick, chosen, with `texture`.
fn brick_with(texture: Option<TextureRow>) -> Fixture {
    let mut row = brick();
    row.texture = texture;
    Fixture {
        materials: vec![row],
        material: Some(MaterialId(31)),
        ..Default::default()
    }
}

/// Clicks `along` (0 to 1) of the way across the slider named `label`.
fn click_slider(h: &mut H, label: &str, along: f32) -> Vec<UiAction> {
    let slider = rect_of(h, label);
    let width = h
        .ctx
        .global_style()
        .spacing
        .slider_width
        .min(slider.width());
    let at = Pos2::new(slider.min.x + along * width, slider.center().y);
    click_at(h, at, egui::Modifiers::NONE)
}

#[test]
fn the_materials_list_chooses_adds_renames_recolors_and_deletes() {
    let first = palette_label(MATERIAL_PALETTE[0]);
    for (label, action) in [
        ("Default", UiAction::ChooseMaterial(None)),
        ("Brick", UiAction::ChooseMaterial(Some(MaterialId(31)))),
        ("Glass", UiAction::ChooseMaterial(Some(MaterialId(32)))),
        ("Add", UiAction::AddMaterial),
        ("Delete", UiAction::DeleteMaterial(MaterialId(31))),
        (
            first.as_str(),
            UiAction::RecolorMaterial(MaterialId(31), MATERIAL_PALETTE[0]),
        ),
    ] {
        let mut h = harness(two_materials());
        assert_eq!(click(&mut h, label), [action], "{label}");
    }
    let mut h = harness(two_materials());
    let renamed = type_into(&mut h, "Material name", 0, "Old", egui::Key::Enter);
    assert!(
        matches!(renamed.as_slice(), [UiAction::RenameMaterial(MaterialId(31), name)] if name.contains("Old")),
        "{renamed:?}"
    );
    let mut h = harness(two_materials());
    let opacity = click_slider(&mut h, "Opacity", 0.04);
    assert!(
        matches!(opacity.as_slice(), [UiAction::SetMaterialOpacity(MaterialId(31), p)] if *p < 100),
        "{opacity:?}"
    );
    let mut h = harness(two_materials());
    let glow = click_slider(&mut h, "Glow", 0.75);
    assert!(
        matches!(glow.as_slice(), [UiAction::SetMaterialEmission(MaterialId(31), t)] if *t > 0),
        "{glow:?}"
    );
    // The default material has no name, colors or sliders, and cannot be deleted.
    let mut h = harness(Fixture {
        material: None,
        ..two_materials()
    });
    for label in ["Material name", first.as_str(), "Opacity", "Glow"] {
        assert!(h.query_by_label(label).is_none(), "{label}");
    }
    assert_eq!(click(&mut h, "Delete"), []);
}

#[test]
fn the_materials_list_loads_removes_and_sizes_textures() {
    let mut h = harness(brick_with(None));
    assert_eq!(
        click(&mut h, "Texture..."),
        [UiAction::LoadTexture(MaterialId(31))]
    );
    for label in ["Remove", "Size (m)"] {
        assert!(h.query_by_label(label).is_none(), "{label}");
    }
    for (label, action) in [
        ("Brick", UiAction::ChooseMaterial(Some(MaterialId(31)))),
        ("Texture...", UiAction::LoadTexture(MaterialId(31))),
        ("Remove", UiAction::RemoveTexture(MaterialId(31))),
    ] {
        let mut h = harness(brick_with(Some(small_texture())));
        assert_eq!(click(&mut h, label), [action], "{label}");
    }
    // Typing 2 into the width field, then the height field, sets the size in millimeters.
    for (k, size) in [(0, [2000, 500]), (1, [1000, 2000])] {
        let mut h = harness(brick_with(Some(small_texture())));
        assert_eq!(
            type_into(&mut h, "Size (m)", k, "2", egui::Key::Enter),
            [UiAction::SetTextureSize(MaterialId(31), size)]
        );
    }
}

#[test]
fn a_typed_texture_size_is_one_change_when_entered() {
    for (k, size) in [(0, [250, 500]), (1, [1000, 250])] {
        let mut h = harness(brick_with(Some(small_texture())));
        h.get_all_by_label("Size (m)").nth(k).unwrap().click();
        h.run_ok();
        for key in ["0", ".", "2", "5"] {
            h.event(egui::Event::Text(key.into()));
            h.run_ok();
        }
        assert_eq!(h.state_mut().take(), [], "nothing while typing");
        h.key_press(egui::Key::Enter);
        h.run_ok();
        assert_eq!(
            h.state_mut().take(),
            [UiAction::SetTextureSize(MaterialId(31), size)]
        );
    }
}

#[test]
fn a_dragged_texture_size_applies_to_its_material_when_another_is_chosen() {
    for k in 0..2 {
        let mut h = harness(brick_with(Some(small_texture())));
        let at = h
            .get_all_by_label("Size (m)")
            .nth(k)
            .unwrap()
            .rect()
            .center();
        h.event(egui::Event::PointerMoved(at));
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_ok();
        for dx in [10.0, 20.0, 40.0] {
            h.event(egui::Event::PointerMoved(at + Vec2::new(dx, 0.0)));
            h.run_ok();
        }
        h.state_mut().take();
        // Still holding the button, another material is chosen (as the API or a shortcut might).
        h.state_mut().material = None;
        h.run_ok();
        let switched = h.state_mut().take();
        assert!(
            matches!(
                switched.as_slice(),
                [UiAction::SetTextureSize(MaterialId(31), _)]
            ),
            "field {k}: {switched:?}"
        );
        h.event(egui::Event::PointerButton {
            pos: at + Vec2::new(40.0, 0.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_ok();
        h.state_mut().material = Some(MaterialId(31));
        h.run_ok();
        assert_eq!(h.state_mut().take(), [], "field {k}: nothing applies later");
    }
}

#[test]
fn dragging_the_opacity_slider_sets_the_opacity_once_when_it_ends() {
    let mut h = harness(brick_with(None));
    let slider = rect_of(&h, "Opacity");
    let width = h
        .ctx
        .global_style()
        .spacing
        .slider_width
        .min(slider.width());
    let start = Pos2::new(slider.min.x + 0.9 * width, slider.center().y);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    h.event(egui::Event::PointerMoved(start));
    h.event(button(start, true));
    h.run_ok();
    for k in 1..=10 {
        h.event(egui::Event::PointerMoved(
            start - Vec2::new(3.0 * k as f32, 0.0),
        ));
        h.run_ok();
    }
    assert_eq!(h.state_mut().take(), [], "nothing while dragging");
    h.event(button(start - Vec2::new(30.0, 0.0), false));
    h.run_ok();
    let actions = h.state_mut().take();
    assert!(
        matches!(actions.as_slice(), [UiAction::SetMaterialOpacity(MaterialId(31), p)] if *p < 100),
        "{actions:?}"
    );
}

#[test]
fn escape_in_the_material_name_field_discards_the_edit() {
    let mut h = harness(brick_with(None));
    let entered = type_into(&mut h, "Material name", 0, "Old", egui::Key::Enter);
    assert!(
        entered
            .iter()
            .any(|a| matches!(a, UiAction::RenameMaterial(..))),
        "Enter renames: {entered:?}"
    );
    let mut h = harness(brick_with(None));
    let escaped = type_into(&mut h, "Material name", 0, "Old", egui::Key::Escape);
    assert!(
        !escaped
            .iter()
            .any(|a| matches!(a, UiAction::RenameMaterial(..))),
        "{escaped:?}"
    );
}

#[test]
fn deleting_a_material_does_not_also_rename_it() {
    // Type into the name field, then press and release on Delete in one frame.
    let mut h = harness(brick_with(None));
    h.get_by_label("Material name").click();
    h.run_ok();
    h.event(egui::Event::Text("Old".into()));
    h.run_ok();
    let actions = click(&mut h, "Delete");
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
