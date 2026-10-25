use glam::DVec3;
use kernel::health::{check, repair};
use kernel::{Model, Solid};

fn cube() -> Model {
    Solid::cuboid(DVec3::ONE).unwrap().to_model()
}

#[test]
fn erasing_a_face_and_making_solid_again_restores_the_box() {
    let mut m = cube();
    let top = m
        .faces()
        .find(|(_, f)| f.normal.z > 0.5)
        .map(|(id, _)| id)
        .unwrap();
    m.erase(&[], &[top]);
    let h = check(&m);
    assert!(!h.is_solid());
    assert_eq!(h.open_edges.len(), 4);
    let r = repair(&m);
    let mut fixed = r.model.expect("closed");
    let changes = fixed.take_changes();
    assert!(!changes.is_empty());
    assert!(check(&fixed).is_solid());
    fixed.validate().unwrap();
    // The change undoes to the open box.
    fixed.apply(&changes.inverse());
    assert_eq!(fixed, m);
}

#[test]
fn a_solid_is_not_reported_and_not_repaired() {
    let m = Solid::torus(2.0, 0.5, 12, 8).unwrap().to_model();
    assert!(check(&m).is_solid());
    assert!(repair(&m).model.is_none());
}
