//! Material finish: metalness and roughness set when adding or changing a material, checked,
//! undone and kept in plain data.

use kernel::{
    DEFAULT_METALNESS, DEFAULT_ROUGHNESS, KernelError, Material, MaterialEdit, Scene, SceneData,
};

fn metal(metalness: f32, roughness: f32) -> Material {
    Material {
        name: "Steel".into(),
        color: [200, 200, 210],
        opacity: 1.0,
        texture: None,
        emission: 0.0,
        metalness,
        roughness,
    }
}

#[test]
fn new_materials_are_matte_and_not_metallic() {
    let mut s = Scene::new();
    let (id, _) = s.add_material("Plain", [1, 2, 3]).unwrap();
    let m = s.material(id).unwrap();
    assert_eq!(m.metalness, DEFAULT_METALNESS);
    assert_eq!(m.roughness, DEFAULT_ROUGHNESS);
    assert_eq!((m.metalness, m.roughness), (0.0, 1.0));
}

#[test]
fn finish_is_added_and_changed_as_exact_undo_steps() {
    let mut s = Scene::new();
    let empty = s.clone();
    let (id, added) = s.add_material_from(metal(1.0, 0.2)).unwrap();
    assert_eq!(s.material(id).unwrap().metalness, 1.0);
    assert_eq!(s.material(id).unwrap().roughness, 0.2);
    let before = s.clone();
    let edit = MaterialEdit {
        roughness: Some(0.6),
        ..MaterialEdit::default()
    };
    let changed = s.change_material_with(id, edit.clone()).unwrap();
    assert_eq!(s.material(id).unwrap().roughness, 0.6);
    assert_eq!(s.material(id).unwrap().metalness, 1.0);
    assert!(s.change_material_with(id, edit).unwrap().is_empty());
    let mut undone = s.clone();
    undone.apply(&changed.inverse());
    assert_eq!(undone, before);
    // Other edits keep the finish.
    s.change_material(id, Some("Chrome"), None, None).unwrap();
    assert_eq!(s.material(id).unwrap().metalness, 1.0);
    s = before;
    s.apply(&added.inverse());
    assert_eq!(s, empty);
}

#[test]
fn finish_outside_zero_to_one_is_refused_and_changes_nothing() {
    let mut s = Scene::new();
    let (id, _) = s.add_material_from(metal(0.5, 0.5)).unwrap();
    let before = s.clone();
    for bad in [-0.1, 1.01, f32::NAN, f32::INFINITY] {
        for (m, r) in [(bad, 0.5), (0.5, bad)] {
            let other = Material {
                name: "Other".into(),
                ..metal(m, r)
            };
            assert!(matches!(
                s.add_material_from(other),
                Err(KernelError::MaterialFinish(_))
            ));
        }
        for edit in [
            MaterialEdit {
                metalness: Some(bad),
                ..MaterialEdit::default()
            },
            MaterialEdit {
                roughness: Some(bad),
                ..MaterialEdit::default()
            },
        ] {
            assert!(matches!(
                s.change_material_with(id, edit),
                Err(KernelError::MaterialFinish(_))
            ));
        }
        assert!(Scene::check_material_ranges(None, None, Some(bad), None).is_err());
        assert!(Scene::check_material_ranges(None, None, None, Some(bad)).is_err());
    }
    assert_eq!(s, before);
}

#[test]
fn finish_is_kept_in_plain_data_and_absent_when_default() {
    let mut s = Scene::new();
    s.add_material("Plain", [9, 9, 9]).unwrap();
    let json = serde_json::to_string(&s.to_data()).unwrap();
    assert!(
        !json.contains("metalness") && !json.contains("roughness"),
        "{json}"
    );
    s.add_material_from(metal(0.75, 0.25)).unwrap();
    let json = serde_json::to_string(&s.to_data()).unwrap();
    assert_eq!(json.matches("metalness").count(), 1, "{json}");
    assert_eq!(json.matches("roughness").count(), 1, "{json}");
    let back: SceneData = serde_json::from_str(&json).unwrap();
    assert_eq!(Scene::from_data(&back).unwrap(), s);
}

#[test]
fn data_without_finish_loads_with_defaults() {
    let json = r#"{"id": 1, "name": "Old", "color": [1, 2, 3]}"#;
    let m: kernel::MaterialData = serde_json::from_str(json).unwrap();
    assert_eq!((m.metalness, m.roughness), (0.0, 1.0));
}
