use super::*;

#[test]
fn entity_info_shows_a_soft_checkbox_for_selected_edges() {
    for (soft, wanted) in [
        (false, UiAction::SetSoft(true)),
        (true, UiAction::SetSoft(false)),
    ] {
        let mut h = harness(Fixture {
            info: vec!["1 edge".into()],
            edges_soft: Some(soft),
            ..Default::default()
        });
        assert_eq!(click(&mut h, "Soft"), [wanted]);
    }
    assert!(harness(Fixture::default()).query_by_label("Soft").is_none());
}

/// The texture position fields, each with the action typing 2 into it for `faces` makes.
fn texture_fields(faces: &[EntityId]) -> [(&'static str, usize, UiAction); 5] {
    let f = faces.to_vec();
    [
        (
            "Offset",
            0,
            UiAction::SetTextureOffset(f.clone(), [2000, 0]),
        ),
        (
            "Offset",
            1,
            UiAction::SetTextureOffset(f.clone(), [0, 2000]),
        ),
        ("Rotation", 0, UiAction::SetTextureRotation(f.clone(), 20)),
        (
            "Scale",
            0,
            UiAction::SetTextureScale(f.clone(), [2000, 1000]),
        ),
        ("Scale", 1, UiAction::SetTextureScale(f, [1000, 2000])),
    ]
}

#[test]
fn entity_info_shows_texture_position_fields_for_selected_faces() {
    let selected = faces(&[3, 4]);
    let fixture = || with_texture_position(selected.clone(), TexturePlacement::default());
    for (label, action) in [
        ("Rotate 90", UiAction::RotateTexture90),
        ("Reset", UiAction::ResetTexturePosition),
    ] {
        let mut h = harness(fixture());
        assert_eq!(click(&mut h, label), [action], "{label}");
    }
    // Typing 2 into each field sets that field alone, in its own units.
    for (label, k, action) in texture_fields(&selected) {
        let mut h = harness(fixture());
        assert_eq!(
            type_into(&mut h, label, k, "2", egui::Key::Enter),
            [action],
            "{label} {k}"
        );
    }
    // Without faces selected there are no texture fields.
    let h = harness(Fixture::default());
    for label in ["Offset", "Rotation", "Scale", "Rotate 90", "Reset"] {
        assert!(!has_field(&h, label), "{label}");
    }
}

#[test]
fn a_texture_field_applies_to_the_faces_it_was_edited_for() {
    // Typing into a field with face 3 selected, then entering it once face 4 (placed
    // otherwise) is selected, sets face 3 from face 3's values.
    let (three, four) = (faces(&[3]), faces(&[4]));
    let other = TexturePlacement {
        offset: [0.5, 0.75],
        rotation_deg: 30.0,
        scale: [3.0, 3.0],
    };
    for (label, k, action) in texture_fields(&three) {
        let mut h = harness(with_texture_position(
            three.clone(),
            TexturePlacement::default(),
        ));
        field(&h, label, k).click();
        h.run_ok();
        h.event(egui::Event::Text("2".into()));
        h.run_ok();
        h.state_mut().texture_position = Some(TexturePosition {
            placement: other,
            faces: four.clone(),
        });
        h.key_press(egui::Key::Enter);
        h.run_ok();
        assert_eq!(h.state_mut().take(), [action], "{label} {k}");
    }
}

#[test]
fn a_stepped_texture_value_applies_when_its_fields_go_away() {
    // Stepping a field with the arrow keys on face 3, then selecting something else (the
    // fields go away), applies the stepped value to face 3.
    let three = faces(&[3]);
    let texture = |a: &UiAction| {
        matches!(
            a,
            UiAction::SetTextureOffset(..)
                | UiAction::SetTextureRotation(..)
                | UiAction::SetTextureScale(..)
        )
    };
    let mut acted = Vec::new();
    for (label, k) in [
        ("Offset", 0),
        ("Offset", 1),
        ("Rotation", 0),
        ("Scale", 0),
        ("Scale", 1),
    ] {
        let mut h = harness(with_texture_position(
            three.clone(),
            TexturePlacement::default(),
        ));
        field(&h, label, k).click();
        h.run_ok();
        for _ in 0..2 {
            h.key_press(egui::Key::ArrowUp);
            h.run_ok();
        }
        h.state_mut().take();
        h.state_mut().texture_position = None;
        h.run_ok();
        acted.extend(h.state_mut().take().into_iter().filter(texture));
        // Faces selected again later: nothing left over applies then.
        h.state_mut().texture_position = Some(TexturePosition {
            placement: TexturePlacement::default(),
            faces: three.clone(),
        });
        h.run_ok();
        let later: Vec<_> = h.state_mut().take().into_iter().filter(texture).collect();
        assert!(later.is_empty(), "{label} {k}: {later:?}");
    }
    // Two steps of each field's speed, applied to face 3 alone.
    assert_eq!(
        acted,
        [
            UiAction::SetTextureOffset(three.clone(), [20, 0]),
            UiAction::SetTextureOffset(three.clone(), [0, 20]),
            UiAction::SetTextureRotation(three.clone(), 10),
            UiAction::SetTextureScale(three.clone(), [1020, 1000]),
            UiAction::SetTextureScale(three, [1000, 1020]),
        ]
    );
}

#[test]
fn an_edit_ended_in_a_discarded_pass_is_reported_once() {
    // Dragging an Offset field on face 3, then selecting something else in a frame egui lays
    // out twice: the fields go away and the first pass ends the edit; its action must survive
    // the second pass, and only once.
    let three = faces(&[3]);
    let mut h = harness(with_texture_position(
        three.clone(),
        TexturePlacement::default(),
    ));
    let field = h.get_all_by_label("Offset").next().unwrap().rect();
    let start = field.center();
    h.event(egui::Event::PointerMoved(start));
    h.event(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_ok();
    for step in 1..=4 {
        h.event(egui::Event::PointerMoved(
            start + Vec2::new(10.0 * step as f32, 0.0),
        ));
        h.run_ok();
    }
    assert_eq!(h.state_mut().take(), [], "nothing applies mid-drag");
    // One frame: the selection is no longer faces.
    h.state_mut().texture_position = None;
    h.state_mut().info = vec!["1 edge".into()];
    h.state_mut().discard_once = true;
    h.step();
    assert_eq!(
        h.output().platform_output.num_completed_passes,
        2,
        "the frame was laid out again"
    );
    let actions = h.state_mut().take();
    assert!(
        matches!(&actions[..], [UiAction::SetTextureOffset(f, [x, 0])] if *f == three && *x > 0),
        "{actions:?}"
    );
    // Letting go later reports nothing more.
    h.event(egui::Event::PointerButton {
        pos: start + Vec2::new(40.0, 0.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
}

#[test]
fn entity_info_renames_a_selected_component() {
    use crate::editor::RenameTarget;
    let d: kernel::DefinitionId = "d2".parse().unwrap();
    let fixture = || Fixture {
        info: vec!["1 component".into()],
        rename: Some(RenameTarget {
            definition: d,
            kind: kernel::DefinitionKind::Component,
            name: "Chair".into(),
        }),
        ..Default::default()
    };
    let mut h = harness(fixture());
    assert_eq!(
        h.get_by_label("Name").accesskit_node().value().as_deref(),
        Some("Chair")
    );
    // Clicking puts the cursor at the end; select it all first, as a user would.
    h.get_by_label("Name").click();
    h.run_ok();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_ok();
    h.event(egui::Event::Text("Stool".into()));
    h.run_ok();
    h.key_press(egui::Key::Enter);
    h.run_ok();
    assert_eq!(
        h.state_mut().take(),
        [UiAction::RenameDefinition(d, "Stool".into())]
    );

    let mut h = harness(fixture());
    assert_eq!(type_into(&mut h, "Name", 0, "s", egui::Key::Escape), []);

    let mut h = harness(fixture());
    h.get_by_label("Name").click();
    h.run_ok();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_ok();
    h.key_press(egui::Key::Backspace);
    h.run_ok();
    h.key_press(egui::Key::Enter);
    h.run_ok();
    assert_eq!(h.state_mut().take(), [], "a component needs a name");
}

#[test]
fn a_name_being_typed_goes_with_the_selection() {
    use crate::editor::RenameTarget;
    let target = RenameTarget {
        definition: "d2".parse().unwrap(),
        kind: kernel::DefinitionKind::Component,
        name: "Chair".into(),
    };
    let mut h = harness(Fixture {
        info: vec!["1 component".into()],
        rename: Some(target.clone()),
        ..Default::default()
    });
    h.get_by_label("Name").click();
    h.run_ok();
    h.event(egui::Event::Text("x".into()));
    h.run_ok();
    assert_eq!(
        h.get_by_label("Name").accesskit_node().value().as_deref(),
        Some("Chairx")
    );
    // The selection changes: the field goes, and with it the text typed.
    h.state_mut().rename = None;
    h.state_mut().info = vec!["Nothing selected".into()];
    h.run_ok();
    assert!(h.query_by_label("Name").is_none());
    assert_eq!(h.state_mut().take(), []);
    h.state_mut().rename = Some(target);
    h.state_mut().info = vec!["1 component".into()];
    h.run_ok();
    assert_eq!(
        h.get_by_label("Name").accesskit_node().value().as_deref(),
        Some("Chair")
    );
}
