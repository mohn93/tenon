//! Material density: checked, undone and kept in plain data only when set.

use kernel::{KernelError, Material, MaterialEdit, Scene, SceneData};

fn with(density: Option<f64>) -> Material {
    Material {
        name: "Steel".into(),
        color: [200, 200, 210],
        opacity: 1.0,
        texture: None,
        emission: 0.0,
        metalness: 0.0,
        roughness: 1.0,
        density,
    }
}

#[test]
fn density_is_set_changed_and_cleared_as_exact_undo_steps() {
    let mut s = Scene::new();
    let (plain, _) = s.add_material("Plain", [1, 2, 3]).unwrap();
    assert_eq!(s.material(plain).unwrap().density, None);
    let empty = s.clone();
    let (id, added) = s.add_material_from(with(Some(7850.0))).unwrap();
    let before = s.clone();
    let edit = MaterialEdit {
        density: Some(Some(2700.0)),
        ..MaterialEdit::default()
    };
    let changed = s.change_material_with(id, edit.clone()).unwrap();
    assert_eq!(s.material(id).unwrap().density, Some(2700.0));
    assert!(s.change_material_with(id, edit).unwrap().is_empty());
    let cleared = s
        .change_material_with(
            id,
            MaterialEdit {
                density: Some(None),
                ..MaterialEdit::default()
            },
        )
        .unwrap();
    assert_eq!(s.material(id).unwrap().density, None);
    s.apply(&cleared.inverse());
    assert_eq!(s.material(id).unwrap().density, Some(2700.0));
    s.apply(&changed.inverse());
    assert_eq!(s, before);
    // Other edits keep the density.
    s.change_material(id, Some("Iron"), None, None).unwrap();
    assert_eq!(s.material(id).unwrap().density, Some(7850.0));
    s = before;
    s.apply(&added.inverse());
    assert_eq!(s, empty);
}

#[test]
fn densities_outside_the_range_are_refused() {
    let mut s = Scene::new();
    let (id, _) = s.add_material_from(with(Some(100.0))).unwrap();
    let before = s.clone();
    for bad in [0.0, 0.99, 30_000.5, f64::NAN, f64::INFINITY, -1.0] {
        assert!(matches!(
            s.add_material_from(Material {
                name: "Other".into(),
                ..with(Some(bad))
            }),
            Err(KernelError::MaterialDensity(_))
        ));
        assert!(matches!(
            s.change_material_with(
                id,
                MaterialEdit {
                    density: Some(Some(bad)),
                    ..MaterialEdit::default()
                }
            ),
            Err(KernelError::MaterialDensity(_))
        ));
    }
    for ok in [1.0, 30_000.0] {
        assert!(kernel::check_density(ok).is_ok());
    }
    assert_eq!(s, before);
}

#[test]
fn density_is_kept_in_plain_data_and_absent_when_unset() {
    let mut s = Scene::new();
    s.add_material("Plain", [9, 9, 9]).unwrap();
    let json = serde_json::to_string(&s.to_data()).unwrap();
    assert!(!json.contains("density"), "{json}");
    s.add_material_from(with(Some(7850.0))).unwrap();
    let json = serde_json::to_string(&s.to_data()).unwrap();
    assert_eq!(json.matches("density").count(), 1, "{json}");
    let back: SceneData = serde_json::from_str(&json).unwrap();
    assert_eq!(Scene::from_data(&back).unwrap(), s);
    let bad = json.replace("7850.0", "0.5");
    let back: SceneData = serde_json::from_str(&bad).unwrap();
    assert!(Scene::from_data(&back).is_err());
}
