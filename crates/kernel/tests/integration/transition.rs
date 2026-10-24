//! The scene transition setting.

use kernel::{KernelError, Scene, SceneData, SceneTransition};

#[test]
fn the_default_is_on_for_one_second() {
    let t = Scene::new().scene_transition();
    assert_eq!(t, SceneTransition::DEFAULT);
    assert!(t.on);
    assert_eq!(t.seconds, 1.0);
    assert!(t.is_default());
}

#[test]
fn setting_it_is_one_undoable_step_and_a_no_op_when_unchanged() {
    let mut s = Scene::new();
    let to = SceneTransition {
        on: false,
        seconds: 2.5,
    };
    let change = s.set_scene_transition(to).unwrap();
    assert_eq!(change.steps.len(), 1);
    assert_eq!(s.scene_transition(), to);
    s.apply(&change.inverse());
    assert_eq!(s.scene_transition(), SceneTransition::DEFAULT);
    s.apply(&change);
    assert_eq!(s.scene_transition(), to);
    assert!(s.set_scene_transition(to).unwrap().steps.is_empty());
}

#[test]
fn seconds_are_checked() {
    let mut s = Scene::new();
    for seconds in [-0.1, 10.5, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            s.set_scene_transition(SceneTransition { on: true, seconds }),
            Err(KernelError::BadTransition(_))
        ));
    }
    for seconds in [0.0, 10.0] {
        assert!(
            s.set_scene_transition(SceneTransition { on: true, seconds })
                .is_ok()
        );
    }
}

#[test]
fn it_round_trips_through_scene_data_and_is_omitted_when_default() {
    let mut s = Scene::new();
    assert!(s.to_data().transition.is_none());
    let to = SceneTransition {
        on: false,
        seconds: 3.0,
    };
    s.set_scene_transition(to).unwrap();
    let data: SceneData = s.to_data();
    assert!(data.transition.is_some());
    let json = serde_json::to_string(&data).unwrap();
    let back = Scene::from_data(&serde_json::from_str(&json).unwrap()).unwrap();
    assert_eq!(back.scene_transition(), to);
    assert_eq!(back, s);
    let mut bad = data;
    bad.transition.as_mut().unwrap().seconds = 11.0;
    assert!(Scene::from_data(&bad).is_err());
}
