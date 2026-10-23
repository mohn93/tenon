//! The sun: NOAA's simplified algorithm against the full NOAA solar calculator algorithm
//! (Meeus: Julian day, geometric mean longitude, equation of center, nutation-corrected
//! obliquity), computed independently with a script for each case below and rounded to 0.001
//! degrees. The simplified Fourier series is good to about 0.2 degrees; the tests allow 0.5.
//! The reference script's arguments are (year, month, day, local minutes, latitude, longitude,
//! UTC offset).

use kernel::{Location, Shadows};
use session::sun::{day_of_year, sun};

fn at(year: u16, month: u8, day: u8, minutes: u16) -> Shadows {
    Shadows {
        on: true,
        year,
        month,
        day,
        minutes,
        ..Shadows::default()
    }
}

fn place(latitude: f64, longitude: f64, utc_offset: f64) -> Location {
    Location {
        latitude,
        longitude,
        north_angle: 0.0,
        utc_offset,
    }
}

/// The angle between two azimuths, in degrees.
fn apart(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

fn check(label: &str, s: Shadows, l: Location, elevation: f64, azimuth: f64) {
    let got = sun(&s, &l);
    assert!(
        (got.elevation - elevation).abs() < 0.5,
        "{label}: elevation {} vs {elevation}",
        got.elevation
    );
    // Near the zenith the azimuth is ill-defined and hardly matters.
    if elevation.abs() < 85.0 {
        assert!(
            apart(got.azimuth, azimuth) < 0.5,
            "{label}: azimuth {} vs {azimuth}",
            got.azimuth
        );
    }
    assert!((got.direction.length() - 1.0).abs() < 1e-12);
    assert!((got.direction.z - elevation.to_radians().sin()).abs() < 0.01);
    assert_eq!(got.is_up(), got.elevation > 0.0);
}

#[test]
fn equator_at_the_equinox_noon() {
    check(
        "equator equinox",
        at(2026, 3, 20, 720),
        place(0.0, 0.0, 0.0),
        88.141,
        91.335,
    );
}

#[test]
fn the_default_place_and_time() {
    check(
        "boulder",
        at(2026, 11, 8, 13 * 60 + 30),
        Location::DEFAULT,
        28.174,
        208.754,
    );
    assert_eq!(Shadows::default().minutes, 13 * 60 + 30);
}

#[test]
fn sixty_north_in_summer_and_winter() {
    let l = place(60.0, 15.0, 1.0);
    check("60N summer 13:00", at(2026, 6, 21, 780), l, 52.047, 202.001);
    check(
        "60N summer 18:00",
        at(2026, 6, 21, 1080),
        l,
        20.378,
        281.843,
    );
    check("60N winter 11:00", at(2026, 12, 21, 660), l, 5.720, 166.643);
}

#[test]
fn the_southern_hemisphere_has_the_sun_in_the_north() {
    let got = sun(&at(2026, 1, 15, 15 * 60), &place(-33.8688, 151.2093, 11.0));
    check(
        "sydney",
        at(2026, 1, 15, 15 * 60),
        place(-33.8688, 151.2093, 11.0),
        61.524,
        289.077,
    );
    // Noon in the south: the sun is north of the zenith.
    let noon = sun(&at(2026, 1, 15, 13 * 60), &place(-33.8688, 151.2093, 11.0));
    assert!(noon.azimuth < 90.0 || noon.azimuth > 270.0, "{noon:?}");
    assert!(got.is_up());
}

#[test]
fn longitudes_near_the_date_line() {
    check(
        "fiji",
        at(2026, 7, 1, 9 * 60),
        place(-17.7, 178.0, 12.0),
        27.868,
        50.572,
    );
    check(
        "samoa",
        at(2026, 7, 1, 15 * 60),
        place(-13.8, -172.0, 13.0),
        38.264,
        315.116,
    );
    let edge_east = sun(&at(2026, 7, 1, 12 * 60), &place(0.0, 180.0, 12.0));
    let edge_west = sun(&at(2026, 7, 1, 12 * 60), &place(0.0, -180.0, 12.0));
    // The same meridian: the same sun.
    assert!((edge_east.elevation - edge_west.elevation).abs() < 1e-9);
    assert!(apart(edge_east.azimuth, edge_west.azimuth) < 1e-9);
}

#[test]
fn a_fractional_utc_offset() {
    check(
        "mumbai +5.5",
        at(2026, 4, 10, 10 * 60 + 30),
        place(19.076, 72.8777, 5.5),
        56.609,
        104.993,
    );
    check(
        "tehran +3.5 near sunrise",
        at(2026, 9, 23, 6 * 60),
        place(35.6892, 51.389, 3.5),
        0.593,
        90.473,
    );
    // A negative fractional offset moves the sun as the clock does: an hour later on the
    // clock is 15 degrees of hour angle.
    let a = sun(&at(2026, 4, 10, 600), &place(19.0, 72.0, 5.5));
    let b = sun(&at(2026, 4, 10, 600), &place(19.0, 72.0, 5.25));
    assert!(a.elevation != b.elevation);
}

#[test]
fn a_leap_day() {
    check(
        "cairo leap day",
        at(2028, 2, 29, 720),
        place(30.0444, 31.2357, 2.0),
        52.224,
        176.975,
    );
}

#[test]
fn night_is_below_the_horizon() {
    let s = sun(&at(2026, 11, 8, 23 * 60), &Location::DEFAULT);
    assert!((s.elevation - -64.888).abs() < 0.5, "{s:?}");
    assert!(!s.is_up());
    assert!(s.direction.z < 0.0);
}

#[test]
fn day_of_year_counts_leap_years() {
    assert_eq!(day_of_year(2026, 1, 1), 1);
    assert_eq!(day_of_year(2026, 3, 1), 60);
    assert_eq!(day_of_year(2026, 12, 31), 365);
    assert_eq!(day_of_year(2028, 3, 1), 61);
    assert_eq!(day_of_year(2028, 12, 31), 366);
    assert_eq!(day_of_year(1900, 3, 1), 60);
    assert_eq!(day_of_year(2000, 3, 1), 61);
}

#[test]
fn the_north_angle_turns_the_sun_in_the_model() {
    let s = at(2026, 3, 20, 720);
    let l = place(40.0, 0.0, 0.0);
    let plain = sun(&s, &l);
    // Near solar noon at 40N the sun is in the south: -y with north up the green axis.
    assert!(plain.direction.y < -0.5, "{plain:?}");
    assert!(plain.direction.x.abs() < 0.2);
    // North a quarter turn clockwise from +y is +x: the sun, in the south, is at -x.
    let turned = sun(
        &s,
        &Location {
            north_angle: 90.0,
            ..l
        },
    );
    assert!(turned.direction.x < -0.5, "{turned:?}");
    assert!(turned.direction.y.abs() < 0.2);
    assert!((turned.direction.z - plain.direction.z).abs() < 1e-12);
    assert_eq!(turned.azimuth, plain.azimuth);
    // And a negative angle the other way.
    let back = sun(
        &s,
        &Location {
            north_angle: -90.0,
            ..l
        },
    );
    assert!(back.direction.x > 0.5);
    // A full turn changes nothing.
    let full = sun(
        &s,
        &Location {
            north_angle: 360.0,
            ..l
        },
    );
    assert!((full.direction - plain.direction).length() < 1e-12);
}

#[test]
fn the_morning_sun_is_in_the_east() {
    let l = Location {
        north_angle: 0.0,
        ..place(0.0, 0.0, 0.0)
    };
    let morning = sun(&at(2026, 6, 21, 9 * 60), &l);
    let evening = sun(&at(2026, 6, 21, 15 * 60), &l);
    assert!(morning.direction.x > 0.3, "east is +x: {morning:?}");
    assert!(evening.direction.x < -0.3, "west is -x: {evening:?}");
}
