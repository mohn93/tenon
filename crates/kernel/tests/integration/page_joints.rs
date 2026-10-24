//! Scenes that store joint values: kept in files, dropped with the joint, and driven to.

use std::collections::BTreeMap;
use std::f64::consts::FRAC_PI_2;

use glam::DVec3;
use kernel::{Coupling, JointId, JointKind, KernelError, Page, PageId, Scene, SceneData};

use crate::solver::{join, parts, value};

fn page(joints: Option<BTreeMap<JointId, (f64, f64)>>) -> Page {
    Page {
        name: "A".into(),
        view: None,
        tags: None,
        positions: None,
        joints,
    }
}

/// Two parts on revolute joints to the ground about Z, coupled 1:1.
fn two() -> (Scene, JointId, JointId) {
    let (mut s, ids) = parts(2);
    let a = join(
        &mut s,
        JointKind::Revolute,
        None,
        ids[0],
        DVec3::ZERO,
        DVec3::Z,
        DVec3::X,
    );
    let b = join(
        &mut s,
        JointKind::Revolute,
        None,
        ids[1],
        DVec3::new(0.1, 0.0, 0.0),
        DVec3::Z,
        DVec3::X,
    );
    (s, a, b)
}

fn add(s: &mut Scene, joints: &[(JointId, f64)]) -> Result<PageId, KernelError> {
    let map = joints.iter().map(|(j, v)| (*j, (*v, 0.0))).collect();
    s.add_page(page(Some(map))).map(|(id, _)| id)
}

#[test]
fn a_page_stores_joint_values_and_keeps_them_in_files() {
    let (mut s, a, b) = two();
    let id = add(&mut s, &[(a, 1.0), (b, -0.5)]).unwrap();
    let json = serde_json::to_string(&s.to_data()).unwrap();
    let read: SceneData = serde_json::from_str(&json).unwrap();
    let back = Scene::from_data(&read).unwrap();
    assert!(back == s);
    assert_eq!(back.page(id).unwrap().joints.as_ref().unwrap().len(), 2);
    // A page without them stores none, and writes nothing.
    let mut plain = Scene::new();
    plain.add_page(page(None)).unwrap();
    assert!(
        !serde_json::to_string(&plain.to_data())
            .unwrap()
            .contains("joints")
    );
}

#[test]
fn a_page_refuses_joints_it_cannot_store() {
    let (mut s, a, _) = two();
    let err = |r: Result<PageId, KernelError>| r.unwrap_err().to_string();
    assert!(err(add(&mut s, &[(JointId(9999), 0.0)])).contains("not a joint"));
    assert!(err(add(&mut s, &[(a, f64::NAN)])).contains("finite"));
    let (mut s2, ids) = parts(2);
    let fixed = join(
        &mut s2,
        JointKind::Fixed,
        None,
        ids[0],
        DVec3::ZERO,
        DVec3::Z,
        DVec3::X,
    );
    assert!(err(add(&mut s2, &[(fixed, 0.0)])).contains("no value"));
}

#[test]
fn deleting_a_joint_drops_its_stored_values_and_undo_restores_them() {
    let (mut s, a, b) = two();
    let id = add(&mut s, &[(a, 1.0), (b, 2.0)]).unwrap();
    let only = add(&mut s, &[(b, 2.0)]);
    assert!(only.is_err(), "the name is taken");
    let before = s.clone();
    let change = s.delete_joint(b).unwrap();
    let kept = s.page(id).unwrap().joints.clone().unwrap();
    assert_eq!(kept.keys().copied().collect::<Vec<_>>(), [a]);
    s.validate().unwrap();
    // The last joint's values going leaves the page storing none.
    let change2 = s.delete_joint(a).unwrap();
    assert!(s.page(id).unwrap().joints.is_none());
    s.apply(&change2.inverse());
    s.apply(&change.inverse());
    assert!(s == before);
}

#[test]
fn drive_joints_drives_in_turn_through_couplings_and_is_all_or_nothing() {
    let (mut s, a, b) = two();
    s.add_coupling(Coupling {
        driver: a,
        driven: b,
        ratio: 2.0,
        offset: 0.0,
    })
    .unwrap();
    let before = s.clone();
    let change = s
        .drive_joints(&[(a, FRAC_PI_2, None), (b, FRAC_PI_2 * 2.0, None)])
        .unwrap();
    assert!((value(&s, a) - FRAC_PI_2).abs() < 1e-9);
    assert!((value(&s, b) - 2.0 * FRAC_PI_2).abs() < 1e-9);
    s.apply(&change.inverse());
    assert!(s == before);
    // The second asks for more than the first joint's limit allows: nothing stays driven.
    let mut limited = s.joint(a).unwrap().clone();
    limited.limits = Some((-2.0, 2.0));
    s.change_joint(a, limited).unwrap();
    let before = s.clone();
    let err = s.drive_joints(&[(a, 1.0, None), (b, 5.0, None)]);
    assert!(err.is_err());
    assert!(s == before);
}

#[test]
fn joint_parts_are_the_parts_driving_moves() {
    let (s, a, b) = two();
    let parts_a = s.joint_parts([a]);
    let parts_b = s.joint_parts([b]);
    assert_eq!(parts_a.len(), 1);
    assert_eq!(parts_b.len(), 1);
    assert_ne!(parts_a, parts_b);
    assert_eq!(s.joint_parts([a, b]).len(), 2);
}
