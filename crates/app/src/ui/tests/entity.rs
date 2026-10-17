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
        assert!(h.query_by_label(label).is_none(), "{label}");
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
        h.get_all_by_label(label).nth(k).unwrap().click();
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
        h.get_all_by_label(label).nth(k).unwrap().click();
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
