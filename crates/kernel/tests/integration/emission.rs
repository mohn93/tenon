//! Glowing materials: emission set when adding or changing a material, checked, undone and
//! kept in plain data.

use kernel::{KernelError, MAX_EMISSION, Material, MaterialEdit, Scene, SceneData};

const WHITE: [u8; 3] = [255, 250, 240];

fn lamp(emission: f32) -> Material {
    Material {
        name: "Lamp".into(),
        color: WHITE,
        opacity: 1.0,
        texture: None,
        emission,
        metalness: 0.0,
        roughness: 1.0,
        density: None,
    }
}

#[test]
fn materials_glow_from_zero_to_the_maximum_as_exact_undo_steps() {
    let mut s = Scene::new();
    let (plain, _) = s.add_material("Plain", WHITE).unwrap();
    assert_eq!(s.material(plain).unwrap().emission, 0.0);
    assert!(!s.material(plain).unwrap().glows());

    let empty = s.clone();
    let (id, added) = s.add_material_from(lamp(4.0)).unwrap();
    assert_eq!(s.material(id).unwrap().emission, 4.0);
    assert!(s.material(id).unwrap().glows());
    assert_eq!(added.summary().materials, vec![id]);

    let before = s.clone();
    let edit = MaterialEdit {
        emission: Some(MAX_EMISSION),
        ..MaterialEdit::default()
    };
    let changed = s.change_material_with(id, edit.clone()).unwrap();
    assert_eq!(s.material(id).unwrap().emission, MAX_EMISSION);
    assert_eq!(changed.summary().materials, vec![id]);
    // The same emission again changes nothing.
    assert!(s.change_material_with(id, edit).unwrap().is_empty());
    let mut undone = s.clone();
    undone.apply(&changed.inverse());
    assert_eq!(undone, before);
    // Other parts keep the emission.
    s.change_material(id, Some("Bright lamp"), None, None)
        .unwrap();
    assert_eq!(s.material(id).unwrap().emission, MAX_EMISSION);
    s = before;
    s.apply(&added.inverse());
    assert_eq!(s, empty);
}

#[test]
fn emission_outside_zero_to_the_maximum_is_refused_and_changes_nothing() {
    let mut s = Scene::new();
    let (id, _) = s.add_material_from(lamp(1.0)).unwrap();
    let before = s.clone();
    for bad in [-0.5, MAX_EMISSION + 0.01, f32::NAN, f32::INFINITY] {
        let other = Material {
            name: "Other".into(),
            ..lamp(bad)
        };
        let err = s.add_material_from(other).unwrap_err();
        assert!(
            matches!(&err, KernelError::MaterialEmission(m) if m.contains("emission")),
            "{bad}: {err:?}"
        );
        let edit = MaterialEdit {
            emission: Some(bad),
            ..MaterialEdit::default()
        };
        assert!(matches!(
            s.change_material_with(id, edit),
            Err(KernelError::MaterialEmission(_))
        ));
    }
    assert_eq!(s, before);
}

#[test]
fn emission_is_kept_in_plain_data_and_absent_when_zero() {
    let mut s = Scene::new();
    s.add_material("Plain", WHITE).unwrap();
    s.add_material_from(lamp(2.5)).unwrap();
    let data = s.to_data();
    let json = serde_json::to_string(&data).unwrap();
    // Only the glowing material writes it, so files without glow read as before.
    assert_eq!(json.matches("emission").count(), 1, "{json}");
    let back: SceneData = serde_json::from_str(&json).unwrap();
    assert_eq!(Scene::from_data(&back).unwrap(), s);

    let mut bad = data.clone();
    bad.materials[1].emission = 11.0;
    assert!(Scene::from_data(&bad).unwrap_err().contains("emission"));
}
