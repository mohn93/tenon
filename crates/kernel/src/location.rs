//! Where the model stands on the Earth, for the sun: a model property.

use crate::edit::KernelError;
use crate::scene::{Scene, SceneChange, Step};

/// A place and a direction: the latitude and longitude in degrees, the angle from the model's
/// green axis clockwise (seen from above) to true north, and the clock's offset from UTC in
/// hours. The default is SketchUp's: Boulder, Colorado, north along the green axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Location {
    /// Degrees, -90 (south) to 90 (north).
    pub latitude: f64,
    /// Degrees, -180 (west) to 180 (east).
    pub longitude: f64,
    /// Degrees from the green axis clockwise to true north, -360 to 360.
    pub north_angle: f64,
    /// Hours east of UTC, -12 to 14 in steps of 0.25.
    pub utc_offset: f64,
}

impl Location {
    pub const DEFAULT: Location = Location {
        latitude: 40.0167,
        longitude: -105.2833,
        north_angle: 0.0,
        utc_offset: -7.0,
    };

    /// Fails when a number is not finite or out of its range, or the offset is not a
    /// multiple of a quarter hour.
    pub fn check(&self) -> Result<(), KernelError> {
        let bad = |why: &'static str| Err(KernelError::BadLocation(why));
        let all = [
            self.latitude,
            self.longitude,
            self.north_angle,
            self.utc_offset,
        ];
        if !all.iter().all(|x| x.is_finite()) {
            return bad("a location number is not finite");
        }
        if !(-90.0..=90.0).contains(&self.latitude) {
            return bad("latitude must be from -90 to 90");
        }
        if !(-180.0..=180.0).contains(&self.longitude) {
            return bad("longitude must be from -180 to 180");
        }
        if !(-360.0..=360.0).contains(&self.north_angle) {
            return bad("north_angle must be from -360 to 360");
        }
        if !(-12.0..=14.0).contains(&self.utc_offset) {
            return bad("utc_offset must be from -12 to 14");
        }
        if (self.utc_offset * 4.0).fract() != 0.0 {
            return bad("utc_offset must be a multiple of 0.25");
        }
        Ok(())
    }

    pub fn is_default(&self) -> bool {
        *self == Location::DEFAULT
    }
}

impl Default for Location {
    fn default() -> Self {
        Location::DEFAULT
    }
}

impl Scene {
    /// Where the model stands.
    pub fn location(&self) -> Location {
        self.location
    }

    /// Moves the model on the Earth, after checking `location`. Nothing to change changes
    /// nothing.
    pub fn set_location(&mut self, location: Location) -> Result<SceneChange, KernelError> {
        location.check()?;
        if location == self.location {
            return Ok(SceneChange::default());
        }
        let step = Step::SetLocation(self.location, location);
        self.apply_step(&step);
        Ok(step.into())
    }
}
