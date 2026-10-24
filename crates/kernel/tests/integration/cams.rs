//! Cam followers: a prismatic joint whose value comes from where its axis line meets a cam.

use std::f64::consts::PI;

use glam::{DAffine3, DVec3};
use kernel::{
    CamFollower, CamFollowerId, Coupling, DefinitionKind, EntityId, InstanceId, JointId, JointKind,
    MeshId, Scene, SceneData,
};

use crate::solver::{join, p, parts, transform, value};

/// The base circle's radius.
const RB: f64 = 0.03;
/// The lobe: a circle of radius `RL` whose center is `C` from the cam's axis, so its tip is
/// `C + RL` out.
const C: f64 = 0.022;
const RL: f64 = 0.018;
/// The lobe's height over the base circle.
const H: f64 = C + RL - RB;
/// Profile corners: one every 5 degrees.
const N: usize = 72;
/// The cam's width along Y.
const W: f64 = 0.01;

fn deg(x: f64) -> f64 {
    x.to_radians()
}

/// The outer edge of the union of the base circle and the lobe circle, `alpha` from the
/// lobe's direction.
fn radius(alpha: f64) -> f64 {
    let s = RL * RL - (C * alpha.sin()).powi(2);
    let lobe = if s >= 0.0 && alpha.cos() > 0.0 {
        C * alpha.cos() + s.sqrt()
    } else {
        0.0
    };
    lobe.max(RB)
}

/// How far a chord of the profile may sit inside the round cam it samples: a base circle
/// chord's sag, with room for the lobe's sharper curve.
fn facet_error() -> f64 {
    (C + RL) * (1.0 - (PI / N as f64).cos())
}

struct Rig {
    scene: Scene,
    cam: InstanceId,
    follower: InstanceId,
    crank_joint: JointId,
    cam_joint: JointId,
    follower_joint: JointId,
    /// The Y of the cam's middle.
    y: f64,
}

/// A cam turning about Y through the origin, its lobe pointing down (-Z), coupled 0.5 to a
/// crank turning about Y at x = 0.2; a follower on a prismatic joint along +Z sits on the
/// cam's top with its frame origin at (0, y, `RB`) (so `follower_x` and `follower_z` move
/// where its axis line runs: a line along `axis` through (`follower_x`, y, `follower_z`)).
fn rig(follower_at: DVec3, axis: DVec3) -> Rig {
    let (mut s, ids) = parts(2);
    let (crank, follower) = (ids[0], ids[1]);
    // The cam: a profile in the XZ plane (angle from +X toward +Z), pulled along Y.
    let profile: Vec<DVec3> = (0..N)
        .map(|k| {
            let phi = 2.0 * PI * k as f64 / N as f64;
            let r = radius(phi + PI / 2.0);
            p(r * phi.cos(), 0.0, r * phi.sin())
        })
        .collect();
    let change = s.edit(MeshId::Root, |m| m.add_polygon(&profile)).unwrap();
    let face = change
        .summary()
        .created
        .iter()
        .find_map(|id| match id {
            EntityId::Face(f) => Some(*f),
            _ => None,
        })
        .unwrap();
    s.edit(MeshId::Root, |m| m.push_pull(face, W)).unwrap();
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
    let (cam, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &faces,
            &[],
            DefinitionKind::Component,
            "Cam",
        )
        .unwrap();
    assert_eq!(s.root().faces().count(), 0, "the root keeps no faces");
    let d = s.root().instance(cam).unwrap().definition;
    let (lo, hi) = s
        .definition(d)
        .unwrap()
        .mesh
        .vertices()
        .map(|(_, v)| v.pos.y)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), y| {
            (lo.min(y), hi.max(y))
        });
    assert!((hi - lo - W).abs() < 1e-12);
    let y = (lo + hi) / 2.0;
    let at = DVec3::new(follower_at.x, y, follower_at.z);
    s.set_instance_transform(
        MeshId::Root,
        crank,
        DAffine3::from_translation(p(0.2, 0.0, 0.0)),
    )
    .unwrap();
    s.set_instance_transform(
        MeshId::Root,
        follower,
        DAffine3::from_translation(at - p(0.005, 0.005, 0.0)),
    )
    .unwrap();
    let crank_joint = join(
        &mut s,
        JointKind::Revolute,
        None,
        crank,
        p(0.2, 0.0, 0.0),
        DVec3::Y,
        DVec3::X,
    );
    let cam_joint = join(
        &mut s,
        JointKind::Revolute,
        None,
        cam,
        DVec3::ZERO,
        DVec3::Y,
        DVec3::X,
    );
    let reference = if axis.cross(DVec3::Y).length() > 0.5 {
        DVec3::Y
    } else {
        DVec3::X
    };
    let follower_joint = join(
        &mut s,
        JointKind::Prismatic,
        None,
        follower,
        at,
        axis,
        reference,
    );
    s.add_coupling(Coupling {
        driver: crank_joint,
        driven: cam_joint,
        ratio: 0.5,
        offset: 0.0,
    })
    .unwrap();
    Rig {
        scene: s,
        cam,
        follower,
        crank_joint,
        cam_joint,
        follower_joint,
        y,
    }
}

fn follow(r: &mut Rig, roller_radius: f64, preload: f64) -> CamFollowerId {
    r.scene
        .add_cam_follower(CamFollower {
            cam: r.cam,
            follower: r.follower_joint,
            roller_radius,
            preload,
        })
        .unwrap()
        .0
}

/// A follower above the cam pushed down onto it.
fn valve() -> (Rig, CamFollowerId) {
    let mut r = rig(p(0.0, 0.0, RB), DVec3::Z);
    let w = follow(&mut r, 0.0, -1.0);
    (r, w)
}

#[test]
fn a_cam_turning_once_in_two_crank_turns_lifts_the_valve_once_by_the_lobe_height() {
    let (mut r, w) = valve();
    assert!(r.scene.cam_follower(w).is_some());
    assert_eq!(r.scene.cam_followers().count(), 1);
    let sag = facet_error();
    let mut lifts = Vec::new();
    for k in 0..=144 {
        let crank = deg(5.0 * k as f64);
        r.scene.drive_joint(r.crank_joint, crank, None).unwrap();
        let lift = value(&r.scene, r.follower_joint);
        // The cam turns half as fast, and the follower stays on its line.
        assert!((value(&r.scene, r.cam_joint) - crank / 2.0).abs() < 1e-9);
        let foot = transform(&r.scene, r.follower).transform_point3(p(0.005, 0.005, 0.0));
        assert!(foot.abs_diff_eq(p(0.0, r.y, RB + lift), 1e-9), "{foot}");
        // At every whole 5 degrees of cam turn a profile corner stands under the follower:
        // the lift is that corner's exactly (the profile is symmetric about the lobe).
        if k % 2 == 0 {
            let up = radius(PI + crank / 2.0) - RB;
            assert!((lift - up).abs() < 1e-9, "at {k}: {lift} vs {up}");
        }
        lifts.push(lift);
    }
    let peak = lifts.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!((peak - H).abs() <= sag, "peak {peak} vs {H}");
    let (top, _) = lifts
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .unwrap();
    assert_eq!(top, 72, "the peak comes at 360 degrees of crank");
    // One lift event: the lift rises past half the lobe once and comes back once.
    let up: Vec<bool> = lifts.iter().map(|l| *l > H / 2.0).collect();
    let rises = up.windows(2).filter(|w| !w[0] && w[1]).count();
    let falls = up.windows(2).filter(|w| w[0] && !w[1]).count();
    assert_eq!((rises, falls), (1, 1));
    // On the base circle the lift is 0 within the facets' error, chords dipping below it.
    for (k, l) in lifts.iter().enumerate() {
        let alpha = PI + deg(2.5 * k as f64);
        if radius(alpha - deg(5.0)) == RB && radius(alpha + deg(5.0)) == RB {
            assert!(*l <= 1e-12 && *l >= -sag, "base lift at {k}: {l}");
        }
    }
}

#[test]
fn the_follower_meets_the_cam_on_the_first_drive_and_undo_puts_it_back() {
    let (mut r, _) = valve();
    let before = r.scene.clone();
    let change = r
        .scene
        .drive_joint(r.crank_joint, deg(360.0), None)
        .unwrap();
    assert!((value(&r.scene, r.follower_joint) - H).abs() < 1e-9);
    assert!(change.summary().joints.contains(&r.follower_joint));
    r.scene.apply(&change.inverse());
    assert_eq!(r.scene, before);
}

#[test]
fn a_roller_stands_its_radius_back_along_the_line() {
    let mut r = rig(p(0.0, 0.0, RB), DVec3::Z);
    follow(&mut r, 0.005, -1.0);
    r.scene.drive_joint(r.crank_joint, deg(10.0), None).unwrap();
    assert!(
        (value(&r.scene, r.follower_joint) - (radius(PI + deg(5.0)) - RB + 0.005)).abs() < 1e-9
    );
    r.scene
        .drive_joint(r.crank_joint, deg(360.0), None)
        .unwrap();
    assert!((value(&r.scene, r.follower_joint) - (H + 0.005)).abs() < 1e-9);
    // A follower below the cam pushed up meets its underside: the lobe's tip at the start.
    let mut r = rig(p(0.0, 0.0, -0.06), DVec3::Z);
    follow(&mut r, 0.002, 1.0);
    r.scene.drive_joint(r.crank_joint, deg(2.0), None).unwrap();
    r.scene.drive_joint(r.crank_joint, 0.0, None).unwrap();
    let tip = -(C + RL);
    assert!((value(&r.scene, r.follower_joint) - (tip - 0.002 + 0.06)).abs() < 1e-9);
}

#[test]
fn losing_contact_refuses_the_drive_and_changes_nothing() {
    // A follower running along X at a height only the lobe's tip reaches.
    let mut r = rig(p(0.08, 0.0, 0.035), -DVec3::X);
    // The lobe points down: the line misses the cam, so the follower cannot be added.
    let err = r
        .scene
        .add_cam_follower(CamFollower {
            cam: r.cam,
            follower: r.follower_joint,
            roller_radius: 0.0,
            preload: 1.0,
        })
        .unwrap_err();
    assert!(err.to_string().contains("does not meet cam"), "{err}");
    r.scene
        .drive_joint(r.crank_joint, deg(360.0), None)
        .unwrap();
    let w = follow(&mut r, 0.0, 1.0);
    r.scene
        .drive_joint(r.crank_joint, deg(350.0), None)
        .unwrap();
    let at = value(&r.scene, r.follower_joint);
    assert!(at > 0.0 && at < 0.08, "{at}");
    let before = r.scene.clone();
    let err = r
        .scene
        .drive_joint(r.crank_joint, deg(180.0), None)
        .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains(&w.to_string()) && text.contains("loses contact"),
        "{text}"
    );
    assert_eq!(r.scene, before);
}

#[test]
fn bad_followers_are_refused() {
    let (mut r, w) = valve();
    let base = CamFollower {
        cam: r.cam,
        follower: r.follower_joint,
        roller_radius: 0.0,
        preload: -1.0,
    };
    let refused = |s: &mut Scene, c: CamFollower, why: &str| {
        let e = s.add_cam_follower(c).unwrap_err().to_string();
        assert!(e.contains(why), "{e} lacks {why}");
    };
    refused(&mut r.scene, base, "already follows a cam");
    refused(
        &mut r.scene,
        CamFollower {
            follower: r.cam_joint,
            ..base
        },
        "must be prismatic",
    );
    r.scene.delete_cam_follower(w).unwrap();
    refused(
        &mut r.scene,
        CamFollower {
            roller_radius: -1.0,
            ..base
        },
        "roller radius",
    );
    refused(
        &mut r.scene,
        CamFollower {
            preload: 0.5,
            ..base
        },
        "preload",
    );
    refused(
        &mut r.scene,
        CamFollower {
            cam: r.follower,
            ..base
        },
        "one of the follower joint",
    );
    refused(
        &mut r.scene,
        CamFollower {
            follower: JointId(987654),
            ..base
        },
        "does not exist",
    );
    // A follower joint cannot be driven or coupled.
    let w = follow(&mut r, 0.0, -1.0);
    let e = r
        .scene
        .drive_joint(r.follower_joint, 0.01, None)
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("follows cam") && e.contains(&w.to_string()),
        "{e}"
    );
    let e = r
        .scene
        .add_coupling(Coupling {
            driver: r.crank_joint,
            driven: r.follower_joint,
            ratio: 1.0,
            offset: 0.0,
        })
        .unwrap_err()
        .to_string();
    assert!(e.contains("follows a cam"), "{e}");
}

#[test]
fn deleting_the_joint_or_erasing_the_cam_drops_the_follower() {
    let (mut r, w) = valve();
    let before = r.scene.clone();
    let change = r.scene.delete_joint(r.follower_joint).unwrap();
    assert!(r.scene.cam_follower(w).is_none());
    assert_eq!(change.summary().cam_followers, vec![w]);
    r.scene.apply(&change.inverse());
    assert_eq!(r.scene, before);
    let change = r.scene.erase_instances(MeshId::Root, &[r.cam]);
    assert!(r.scene.cam_follower(w).is_none());
    assert_eq!(change.summary().cam_followers, vec![w]);
    r.scene.validate().unwrap();
    r.scene.apply(&change.inverse());
    assert_eq!(r.scene, before);
}

#[test]
fn followers_round_trip_through_scene_data() {
    let (mut r, w) = valve();
    r.scene.delete_cam_follower(w).unwrap();
    let w = follow(&mut r, 0.004, -1.0);
    let data = r.scene.to_data();
    assert_eq!(data.cam_followers.len(), 1);
    assert_eq!(data.cam_followers[0].id, w.0);
    let back = Scene::from_data(&data).unwrap();
    assert_eq!(back, r.scene);
    let mut bad: SceneData = data.clone();
    bad.cam_followers[0].preload = 2.0;
    assert!(Scene::from_data(&bad).is_err());
    let mut bad = data;
    bad.cam_followers[0].follower = 987654;
    assert!(Scene::from_data(&bad).is_err());
}
