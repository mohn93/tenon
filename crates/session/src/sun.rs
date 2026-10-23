//! Where the sun is: the NOAA solar calculator's formulas (Meeus) from a date, a clock time, a place and the
//! clock's UTC offset, turned by the model's north angle into the model's axes. The date
//! arithmetic is done here by hand: there is no date or time dependency.

use std::f64::consts::{PI, TAU};

use glam::DVec3;
use kernel::{Location, Shadows};

/// The sun's place in the sky and the direction towards it in the model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    /// Degrees clockwise from true north, 0 to 360.
    pub azimuth: f64,
    /// Degrees above the horizon (negative below it), geometric: no refraction.
    pub elevation: f64,
    /// The unit vector from the model towards the sun, in the model's x, y and z: z up, and
    /// true north along the green axis (+y) turned clockwise by the location's north angle.
    pub direction: DVec3,
}

impl Sun {
    /// Whether the sun is above the horizon.
    pub fn is_up(&self) -> bool {
        self.elevation > 0.0
    }
}

/// The day of the year of a date, 1 for January 1st. The date must be a real one (see
/// [`Shadows::check`]).
pub fn day_of_year(year: u16, month: u8, day: u8) -> u16 {
    let before: u16 = (1..month)
        .map(|m| u16::from(Shadows::days_in_month(year, m)))
        .sum();
    before + u16::from(day)
}

/// The Julian day of the instant `minutes` after local midnight on a date, at the clock's
/// `utc_offset` hours east of UTC (so the day number is of UTC, and may fall on the day
/// before or after the date). Gregorian calendar.
fn julian_day(year: u16, month: u8, day: u8, minutes: f64, utc_offset: f64) -> f64 {
    let (y, m) = if month <= 2 {
        (i64::from(year) - 1, i64::from(month) + 12)
    } else {
        (i64::from(year), i64::from(month))
    };
    let a = y.div_euclid(100);
    let b = 2 - a + a.div_euclid(4);
    let midnight = (365.25 * (y + 4716) as f64).floor()
        + (30.6001 * (m + 1) as f64).floor()
        + f64::from(day)
        + b as f64
        - 1524.5;
    midnight + (minutes - 60.0 * utc_offset) / 1440.0
}

/// The sun at the date and time of `shadows`, for a model at `location`: the NOAA solar
/// calculator's formulas (Meeus): the Julian century of the instant (UTC), the Sun's mean
/// longitude and anomaly, the equation of center, the apparent longitude, the corrected
/// obliquity, the declination, the equation of time and the hour angle.
pub fn sun(shadows: &Shadows, location: &Location) -> Sun {
    let minutes = f64::from(shadows.minutes);
    let jd = julian_day(
        shadows.year,
        shadows.month,
        shadows.day,
        minutes,
        location.utc_offset,
    );
    let t = (jd - 2_451_545.0) / 36525.0;
    let rad = f64::to_radians;
    // Geometric mean longitude and anomaly of the Sun (degrees), orbit eccentricity.
    let l0 = (280.46646 + t * (36000.76983 + t * 0.0003032)).rem_euclid(360.0);
    let m = 357.52911 + t * (35999.05029 - 0.0001537 * t);
    let e = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
    let center = rad(m).sin() * (1.914602 - t * (0.004817 + 0.000014 * t))
        + (2.0 * rad(m)).sin() * (0.019993 - 0.000101 * t)
        + (3.0 * rad(m)).sin() * 0.000289;
    let omega = 125.04 - 1934.136 * t;
    let apparent = l0 + center - 0.00569 - 0.00478 * rad(omega).sin();
    let mean_obliquity =
        23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.0) / 60.0;
    let obliquity = mean_obliquity + 0.00256 * rad(omega).cos();
    let decl = (rad(obliquity).sin() * rad(apparent).sin()).asin();
    let y = (rad(obliquity) / 2.0).tan().powi(2);
    // Equation of time, minutes.
    let eqtime = 4.0
        * (y * (2.0 * rad(l0)).sin() - 2.0 * e * rad(m).sin()
            + 4.0 * e * y * rad(m).sin() * (2.0 * rad(l0)).cos()
            - 0.5 * y * y * (4.0 * rad(l0)).sin()
            - 1.25 * e * e * (2.0 * rad(m)).sin())
        .to_degrees();
    // True solar time, minutes; the hour angle is 15 degrees an hour from solar noon,
    // positive in the afternoon.
    let solar_time = minutes + eqtime + 4.0 * location.longitude - 60.0 * location.utc_offset;
    let hour_angle = rad(solar_time / 4.0 - 180.0);
    let lat = location.latitude.to_radians();
    let sin_elevation = lat.sin() * decl.sin() + lat.cos() * decl.cos() * hour_angle.cos();
    let elevation = sin_elevation.clamp(-1.0, 1.0).asin();
    // Azimuth measured from south towards west, then from north clockwise.
    let from_south = hour_angle
        .sin()
        .atan2(hour_angle.cos() * lat.sin() - decl.tan() * lat.cos());
    let azimuth = (from_south + PI).rem_euclid(TAU);
    // The model's axes: north is the green axis turned clockwise by the north angle.
    let model = azimuth + location.north_angle.to_radians();
    let direction = DVec3::new(
        elevation.cos() * model.sin(),
        elevation.cos() * model.cos(),
        elevation.sin(),
    );
    Sun {
        azimuth: azimuth.to_degrees(),
        elevation: elevation.to_degrees(),
        direction,
    }
}
