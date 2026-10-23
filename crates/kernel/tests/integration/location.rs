//! The model's location and the shadow settings' validation.

use kernel::{KernelError, Location, Scene, SceneData, Shadows};

#[test]
fn the_default_location_is_boulder_with_north_up_the_green_axis() {
    let l = Scene::new().location();
    assert_eq!(l, Location::DEFAULT);
    assert_eq!(
        (l.latitude, l.longitude, l.north_angle, l.utc_offset),
        (40.0167, -105.2833, 0.0, -7.0)
    );
    assert!(l.is_default());
}

#[test]
fn set_location_is_one_undoable_step_and_a_no_op_when_unchanged() {
    let mut s = Scene::new();
    let to = Location {
        latitude: -12.5,
        ..Location::DEFAULT
    };
    let change = s.set_location(to).unwrap();
    assert_eq!(change.steps.len(), 1);
    assert_eq!(s.location(), to);
    s.apply(&change.inverse());
    assert_eq!(s.location(), Location::DEFAULT);
    s.apply(&change);
    assert_eq!(s.location(), to);
    assert!(s.set_location(to).unwrap().steps.is_empty());
}

#[test]
fn bad_locations_are_refused() {
    let mut s = Scene::new();
    for bad in [
        Location {
            latitude: f64::NAN,
            ..Location::DEFAULT
        },
        Location {
            latitude: 91.0,
            ..Location::DEFAULT
        },
        Location {
            longitude: -180.5,
            ..Location::DEFAULT
        },
        Location {
            north_angle: 400.0,
            ..Location::DEFAULT
        },
        Location {
            utc_offset: 15.0,
            ..Location::DEFAULT
        },
        Location {
            utc_offset: 1.1,
            ..Location::DEFAULT
        },
    ] {
        assert!(matches!(
            s.set_location(bad),
            Err(KernelError::BadLocation(_))
        ));
    }
    assert_eq!(s.location(), Location::DEFAULT);
}

#[test]
fn the_location_round_trips_through_scene_data_and_is_omitted_when_default() {
    let mut s = Scene::new();
    assert!(s.to_data().location.is_none());
    let to = Location {
        latitude: 1.0,
        longitude: 2.0,
        north_angle: 3.0,
        utc_offset: 4.25,
    };
    s.set_location(to).unwrap();
    let data: SceneData = s.to_data();
    assert!(data.location.is_some());
    let json = serde_json::to_string(&data).unwrap();
    let back = Scene::from_data(&serde_json::from_str(&json).unwrap()).unwrap();
    assert_eq!(back.location(), to);
    let mut bad = data;
    bad.location.as_mut().unwrap().latitude = 100.0;
    assert!(Scene::from_data(&bad).is_err());
}

#[test]
fn shadow_dates_times_and_levels_are_checked() {
    assert!(Shadows::default().check().is_ok());
    let at = |year, month, day| Shadows {
        year,
        month,
        day,
        ..Shadows::default()
    };
    assert!(at(2028, 2, 29).check().is_ok());
    assert!(at(2026, 2, 29).check().is_err());
    assert!(at(1900, 2, 29).check().is_err());
    assert!(at(2000, 2, 29).check().is_ok());
    assert!(at(2026, 4, 31).check().is_err());
    assert!(at(2026, 0, 1).check().is_err());
    assert!(at(2026, 13, 1).check().is_err());
    assert!(at(2026, 1, 0).check().is_err());
    assert!(at(1799, 1, 1).check().is_err());
    assert!(at(2201, 1, 1).check().is_err());
    let s = Shadows {
        minutes: 1440,
        ..Shadows::default()
    };
    assert!(s.check().is_err());
    let s = Shadows {
        light: 101,
        ..Shadows::default()
    };
    assert!(s.check().is_err());
    assert!(Shadows::default().is_default());
    assert!(
        !Shadows {
            on: true,
            ..Shadows::default()
        }
        .is_default()
    );
}
