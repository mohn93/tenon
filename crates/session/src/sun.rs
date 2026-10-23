//! Where the sun is: NOAA's simplified solar position algorithm (fractional year, equation of
//! time, declination, true solar time, hour angle) from a date, a clock time, a place and the
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

/// The sun at the date and time of `shadows`, for a model at `location`.
pub fn sun(shadows: &Shadows, location: &Location) -> Sun {
    let days_in_year = if Shadows::is_leap_year(shadows.year) {
        366.0
    } else {
        365.0
    };
    let hour = f64::from(shadows.minutes) / 60.0;
    // Fractional year, radians.
    let g = TAU / days_in_year
        * (f64::from(day_of_year(shadows.year, shadows.month, shadows.day)) - 1.0
            + (hour - 12.0) / 24.0);
    // Equation of time, minutes.
    let eqtime = 229.18
        * (0.000075 + 0.001868 * g.cos()
            - 0.032077 * g.sin()
            - 0.014615 * (2.0 * g).cos()
            - 0.040849 * (2.0 * g).sin());
    // Declination, radians.
    let decl = 0.006918 - 0.399912 * g.cos() + 0.070257 * g.sin() - 0.006758 * (2.0 * g).cos()
        + 0.000907 * (2.0 * g).sin()
        - 0.002697 * (3.0 * g).cos()
        + 0.00148 * (3.0 * g).sin();
    // True solar time, minutes; the hour angle is 15 degrees an hour from solar noon,
    // positive in the afternoon.
    let time_offset = eqtime + 4.0 * location.longitude - 60.0 * location.utc_offset;
    let solar_time = f64::from(shadows.minutes) + time_offset;
    let hour_angle = (solar_time / 4.0 - 180.0).to_radians();
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
