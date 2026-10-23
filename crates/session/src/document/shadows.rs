//! The sun's shadow settings (a view setting) and the model's location.

use kernel::{Location, SceneChange, Shadows};

use super::Document;
use crate::command::{CommandError, ErrorKind};
use crate::sun::{self, Sun};

impl Document {
    /// The sun at the shadow settings' date and time, for the model's location.
    pub fn sun(&self) -> Sun {
        sun::sun(&self.shadows, &self.scene.location())
    }

    /// The shadow settings with the given fields changed, or `invalid_input` for a bad one.
    pub(super) fn shadows_with(
        &self,
        on: Option<bool>,
        date: Option<&str>,
        time: Option<&str>,
        light: Option<i64>,
        dark: Option<i64>,
        on_ground: Option<bool>,
    ) -> Result<Shadows, CommandError> {
        let bad = |why: String| CommandError::new(ErrorKind::InvalidInput, why);
        let mut s = self.shadows;
        if let Some(on) = on {
            s.on = on;
        }
        if let Some(date) = date {
            (s.year, s.month, s.day) = parse_date(date).ok_or_else(|| {
                bad(format!(
                    "date must be a real date written YYYY-MM-DD, not {date:?}"
                ))
            })?;
        }
        if let Some(time) = time {
            s.minutes = parse_time(time).ok_or_else(|| {
                bad(format!(
                    "time must be written HH:MM (00:00 to 23:59), not {time:?}"
                ))
            })?;
        }
        for (name, level, field) in [("light", light, &mut s.light), ("dark", dark, &mut s.dark)] {
            if let Some(level) = level {
                *field = u8::try_from(level)
                    .ok()
                    .filter(|l| *l <= 100)
                    .ok_or_else(|| bad(format!("{name} must be from 0 to 100")))?;
            }
        }
        if let Some(on_ground) = on_ground {
            s.on_ground = on_ground;
        }
        s.check().map_err(bad)?;
        Ok(s)
    }

    /// Moves the model on the Earth: the given fields change, the others stay.
    pub(super) fn set_location_changes(
        &mut self,
        latitude: Option<f64>,
        longitude: Option<f64>,
        north_angle: Option<f64>,
        utc_offset: Option<f64>,
    ) -> Result<SceneChange, CommandError> {
        let from = self.scene.location();
        let to = Location {
            latitude: latitude.unwrap_or(from.latitude),
            longitude: longitude.unwrap_or(from.longitude),
            north_angle: north_angle.unwrap_or(from.north_angle),
            utc_offset: utc_offset.unwrap_or(from.utc_offset),
        };
        self.scene
            .set_location(to)
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
    }
}

/// `YYYY-MM-DD` as a year, month and day, when it is written exactly so (digits only, the
/// widths shown); whether it is a real date is [`Shadows::check`]'s to say.
fn parse_date(text: &str) -> Option<(u16, u8, u8)> {
    let b = text.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<u16> {
        let digits = &text[range];
        digits
            .bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| digits.parse().ok())
            .flatten()
    };
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    Some((year, u8::try_from(month).ok()?, u8::try_from(day).ok()?))
}

/// `HH:MM` as minutes since midnight, when it is written exactly so and within a day.
fn parse_time(text: &str) -> Option<u16> {
    let b = text.as_bytes();
    if b.len() != 5 || b[2] != b':' || !text.is_char_boundary(2) {
        return None;
    }
    let number = |digits: &str| -> Option<u16> {
        digits
            .bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| digits.parse().ok())
            .flatten()
    };
    let (hours, minutes) = (number(&text[0..2])?, number(&text[3..5])?);
    (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
}
