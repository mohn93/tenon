//! Fly mode: walking or flying through the model like in a game. The mouse turns the view and
//! held keys move the eye; this module turns that input into camera moves, independent of the
//! window.
//!
//! The camera's target stays `TARGET_AHEAD` in front of the eye, and moving carries the eye and
//! the target together, so the document camera (and the API's `camera` query) always shows
//! where the flyer is and which way it looks.

use std::f64::consts::FRAC_PI_2;
use std::time::Instant;

use glam::{DVec2, DVec3};
use session::Camera;
use session::camera::UP;

/// Base speed when fly mode is first used, in meters a second.
pub const DEFAULT_SPEED: f64 = 3.0;
pub const MIN_SPEED: f64 = 0.25;
pub const MAX_SPEED: f64 = 60.0;
/// Factor one speed step (a wheel notch, `[` or `]`) multiplies or divides the speed by.
pub const SPEED_STEP: f64 = 1.25;
/// Speed factor while Shift is held.
pub const FAST: f64 = 3.0;
/// Speed factor while Alt/Option is held.
pub const SLOW: f64 = 0.3;
/// About how long the speed takes to build up or die down, in seconds.
pub const RAMP: f64 = 0.15;
/// Longest frame a move covers, in seconds, so a stall does not jump the eye.
pub const MAX_FRAME: f64 = 0.1;
/// How far the view turns per logical pixel of mouse motion, in radians.
pub const LOOK_RAD_PER_PX: f64 = 0.0025;
/// Farthest the view tilts up or down from level, in radians: short of straight up or down.
pub const PITCH_LIMIT: f64 = FRAC_PI_2 - 0.02;
/// How far in front of the eye the camera's target stays, in meters.
pub const TARGET_AHEAD: f64 = 1.0;
/// Below this speed (m/s), with no key held, the eye stops.
const STOP_SPEED: f64 = 1e-3;

/// The keys that move the eye, by their place on a US keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlyKey {
    /// Forward.
    W,
    /// Left.
    A,
    /// Back.
    S,
    /// Right.
    D,
    /// Up.
    E,
    /// Down.
    Q,
    /// Up.
    Space,
    /// Down.
    Ctrl,
}

impl FlyKey {
    fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// Held keys, speed and velocity of fly mode.
#[derive(Clone, Debug)]
pub struct Fly {
    /// Bits of the `FlyKey`s held down.
    held: u8,
    /// Meters a second, in world coordinates.
    velocity: DVec3,
    /// Base speed, meters a second.
    speed: f64,
    fast: bool,
    slow: bool,
    /// When the last move was made while moving (None when at rest).
    last: Option<Instant>,
    /// Speed steps scrolled but not yet whole.
    scrolled: f64,
}

impl Default for Fly {
    fn default() -> Self {
        Self {
            held: 0,
            velocity: DVec3::ZERO,
            speed: DEFAULT_SPEED,
            fast: false,
            slow: false,
            last: None,
            scrolled: 0.0,
        }
    }
}

impl Fly {
    /// The base speed, meters a second.
    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// The speed moving keys give now: the base speed times `FAST` with Shift and `SLOW`
    /// with Alt.
    pub fn effective_speed(&self) -> f64 {
        let mut s = self.speed;
        if self.fast {
            s *= FAST;
        }
        if self.slow {
            s *= SLOW;
        }
        s
    }

    #[cfg(test)]
    pub fn velocity(&self) -> DVec3 {
        self.velocity
    }

    pub fn set_modifiers(&mut self, shift: bool, alt: bool) {
        self.fast = shift;
        self.slow = alt;
    }

    /// A moving key went down or up. Returns true if what is held changed.
    pub fn key(&mut self, key: FlyKey, pressed: bool) -> bool {
        let before = self.held;
        if pressed {
            self.held |= key.bit();
        } else {
            self.held &= !key.bit();
        }
        self.held != before
    }

    pub fn holds(&self, key: FlyKey) -> bool {
        self.held & key.bit() != 0
    }

    /// Lets go of every key and modifier (focus went elsewhere, so releases may never come).
    /// The eye slows to a stop.
    pub fn release_all(&mut self) {
        self.held = 0;
        self.fast = false;
        self.slow = false;
    }

    /// Stops at once: no keys held, no velocity. The speed and modifiers are kept.
    pub fn stop(&mut self) {
        self.held = 0;
        self.velocity = DVec3::ZERO;
        self.last = None;
        self.scrolled = 0.0;
    }

    /// Multiplies the base speed by `SPEED_STEP` `steps` times (negative divides), within
    /// `MIN_SPEED` and `MAX_SPEED`. Returns true if it changed.
    pub fn step_speed(&mut self, steps: i32) -> bool {
        let before = self.speed;
        self.speed = (self.speed * SPEED_STEP.powi(steps)).clamp(MIN_SPEED, MAX_SPEED);
        self.speed != before
    }

    /// Scrolls the speed by `steps` (positive faster), which may be fractional: whole steps
    /// apply once they add up. Returns true if the speed changed.
    pub fn scroll_speed(&mut self, steps: f64) -> bool {
        if !steps.is_finite() {
            return false;
        }
        self.scrolled += steps;
        let whole = self.scrolled.trunc();
        self.scrolled -= whole;
        whole != 0.0 && self.step_speed(whole.clamp(-10.0, 10.0) as i32)
    }

    /// True while a key is held or the eye still moves: frames must keep coming.
    pub fn moving(&self) -> bool {
        self.held != 0 || self.velocity != DVec3::ZERO
    }

    /// The velocity the held keys ask for: W/S along the view direction laid flat, A/D
    /// across it, E or Space up and Q or Ctrl down, at `effective_speed` (diagonals no
    /// faster).
    pub fn wish(&self, cam: &Camera) -> DVec3 {
        let axis = |plus: bool, minus: bool| f64::from(i8::from(plus) - i8::from(minus));
        let (yaw, _) = heading(cam);
        let forward = DVec3::new(yaw.cos(), yaw.sin(), 0.0);
        let right = forward.cross(UP);
        let h = |k| self.holds(k);
        let dir = forward * axis(h(FlyKey::W), h(FlyKey::S))
            + right * axis(h(FlyKey::D), h(FlyKey::A))
            + UP * axis(
                h(FlyKey::E) || h(FlyKey::Space),
                h(FlyKey::Q) || h(FlyKey::Ctrl),
            );
        dir.normalize_or_zero() * self.effective_speed()
    }

    /// Moves the eye and target over `dt` seconds (at most `MAX_FRAME`): the velocity eases
    /// toward `wish` over about `RAMP`, then carries the camera. Returns true if it moved.
    pub fn advance(&mut self, cam: &mut Camera, dt: f64) -> bool {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, MAX_FRAME)
        } else {
            0.0
        };
        let wish = self.wish(cam);
        // Exponential easing reaching 95% of the change in RAMP.
        let k = 1.0 - (-3.0 * dt / RAMP).exp();
        self.velocity += (wish - self.velocity) * k;
        if wish == DVec3::ZERO && self.velocity.length() < STOP_SPEED {
            self.velocity = DVec3::ZERO;
        }
        let step = self.velocity * dt;
        if step == DVec3::ZERO {
            return false;
        }
        cam.eye += step;
        cam.target += step;
        true
    }

    /// A frame at `now`: moves by the time since the last one (none after rest). Returns true
    /// while frames must keep coming (see `moving`).
    pub fn tick(&mut self, cam: &mut Camera, now: Instant) -> bool {
        let dt = frame_dt(self.last, now);
        self.advance(cam, dt);
        self.last = self.moving().then_some(now);
        self.moving()
    }
}

/// Seconds since `last` (0 for none), no more than `MAX_FRAME`.
pub fn frame_dt(last: Option<Instant>, now: Instant) -> f64 {
    last.map_or(0.0, |t| {
        now.saturating_duration_since(t)
            .as_secs_f64()
            .min(MAX_FRAME)
    })
}

/// Which way the camera looks: the angle about `UP` from +X (counter-clockwise seen from
/// above) and the angle above level. Looking straight down or up, the heading is the screen's
/// up direction.
pub fn heading(cam: &Camera) -> (f64, f64) {
    let f = cam.forward();
    let flat = DVec2::new(f.x, f.y);
    let flat = if flat.length() > 1e-9 {
        flat
    } else {
        DVec2::new(cam.up.x, cam.up.y)
    };
    let yaw = flat.y.atan2(flat.x);
    let pitch = f.z.clamp(-1.0, 1.0).asin();
    (yaw, pitch)
}

/// Points the camera at `yaw` and `pitch` (see `heading`; the pitch held within
/// `PITCH_LIMIT`), with the target `TARGET_AHEAD` in front of the eye and `UP` up.
pub fn aim(cam: &mut Camera, yaw: f64, pitch: f64) {
    let pitch = pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);
    let dir = DVec3::new(
        pitch.cos() * yaw.cos(),
        pitch.cos() * yaw.sin(),
        pitch.sin(),
    );
    cam.target = cam.eye + dir * TARGET_AHEAD;
    cam.up = UP;
}

/// Turns the view by a mouse move of `delta` logical pixels: right turns right (clockwise
/// seen from above), down looks down.
pub fn look(cam: &mut Camera, delta: DVec2) {
    if !delta.is_finite() {
        return;
    }
    let (yaw, pitch) = heading(cam);
    aim(
        cam,
        yaw - delta.x * LOOK_RAD_PER_PX,
        pitch - delta.y * LOOK_RAD_PER_PX,
    );
}

/// Readies the camera for flying: the same eye and heading, the pitch within `PITCH_LIMIT`,
/// the target `TARGET_AHEAD` away and `UP` up.
pub fn level(cam: &mut Camera) {
    let (yaw, pitch) = heading(cam);
    aim(cam, yaw, pitch);
}

/// The status bar's text in fly mode.
pub fn status_text(speed: f64) -> String {
    let speed = if speed < 1.0 {
        format!("{speed:.2}")
    } else {
        format!("{speed:.1}")
    };
    format!("Fly mode: {speed} m/s (wheel or [ ] to change, Shift faster, Esc to exit)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A camera at (0, 0, 1.7) looking along +X.
    fn camera() -> Camera {
        let mut cam = Camera::new(DVec2::new(800.0, 600.0));
        cam.eye = DVec3::new(0.0, 0.0, 1.7);
        cam.target = DVec3::new(5.0, 0.0, 1.7);
        level(&mut cam);
        cam
    }

    /// Runs `secs` of 60 Hz frames.
    fn run(fly: &mut Fly, cam: &mut Camera, secs: f64) {
        let frames = (secs * 60.0).round() as usize;
        for _ in 0..frames {
            fly.advance(cam, 1.0 / 60.0);
        }
    }

    #[test]
    fn level_keeps_the_eye_and_puts_the_target_a_meter_ahead() {
        let cam = camera();
        assert!(cam.eye.abs_diff_eq(DVec3::new(0.0, 0.0, 1.7), 1e-12));
        assert!(cam.target.abs_diff_eq(DVec3::new(1.0, 0.0, 1.7), 1e-12));
        assert_eq!(cam.up, UP);
    }

    #[test]
    fn keys_ask_for_velocity_along_the_ground() {
        let mut cam = camera();
        // Looking down a little: forward still moves level.
        look(&mut cam, DVec2::new(0.0, 100.0));
        let mut fly = Fly::default();
        assert_eq!(fly.wish(&cam), DVec3::ZERO);
        fly.key(FlyKey::W, true);
        assert!(
            fly.wish(&cam)
                .abs_diff_eq(DVec3::new(DEFAULT_SPEED, 0.0, 0.0), 1e-9)
        );
        fly.key(FlyKey::W, false);
        fly.key(FlyKey::S, true);
        assert!(fly.wish(&cam).x < 0.0);
        fly.key(FlyKey::S, false);
        // Looking along +X, right is -Y.
        fly.key(FlyKey::D, true);
        assert!(
            fly.wish(&cam)
                .abs_diff_eq(DVec3::new(0.0, -DEFAULT_SPEED, 0.0), 1e-9)
        );
        fly.key(FlyKey::D, false);
        fly.key(FlyKey::A, true);
        assert!(fly.wish(&cam).y > 0.0);
        fly.key(FlyKey::A, false);
        for up in [FlyKey::E, FlyKey::Space] {
            fly.key(up, true);
            assert!(fly.wish(&cam).abs_diff_eq(UP * DEFAULT_SPEED, 1e-9));
            fly.key(up, false);
        }
        for down in [FlyKey::Q, FlyKey::Ctrl] {
            fly.key(down, true);
            assert!(fly.wish(&cam).abs_diff_eq(-UP * DEFAULT_SPEED, 1e-9));
            fly.key(down, false);
        }
        // Diagonals are no faster; opposite keys cancel.
        fly.key(FlyKey::W, true);
        fly.key(FlyKey::D, true);
        assert!((fly.wish(&cam).length() - DEFAULT_SPEED).abs() < 1e-9);
        fly.key(FlyKey::D, false);
        fly.key(FlyKey::S, true);
        assert_eq!(fly.wish(&cam), DVec3::ZERO);
    }

    #[test]
    fn shift_and_alt_scale_the_speed() {
        let cam = camera();
        let mut fly = Fly::default();
        fly.key(FlyKey::W, true);
        fly.set_modifiers(true, false);
        assert!((fly.wish(&cam).length() - DEFAULT_SPEED * FAST).abs() < 1e-9);
        fly.set_modifiers(false, true);
        assert!((fly.wish(&cam).length() - DEFAULT_SPEED * SLOW).abs() < 1e-9);
        fly.set_modifiers(false, false);
        assert!((fly.effective_speed() - DEFAULT_SPEED).abs() < 1e-12);
    }

    #[test]
    fn speed_eases_in_and_out_over_the_ramp() {
        let mut cam = camera();
        let mut fly = Fly::default();
        fly.key(FlyKey::W, true);
        run(&mut fly, &mut cam, 0.05);
        let early = fly.velocity().length();
        assert!(
            early > 0.3 * DEFAULT_SPEED && early < 0.8 * DEFAULT_SPEED,
            "{early}"
        );
        run(&mut fly, &mut cam, 0.1);
        assert!(fly.velocity().length() > 0.94 * DEFAULT_SPEED);
        run(&mut fly, &mut cam, 1.0);
        let start = cam.eye;
        run(&mut fly, &mut cam, 1.0);
        assert!(
            ((cam.eye - start).length() - DEFAULT_SPEED).abs() < 1e-3,
            "full speed covers the speed in a second"
        );
        // The target travels with the eye.
        assert!(((cam.target - cam.eye).length() - TARGET_AHEAD).abs() < 1e-9);
        fly.key(FlyKey::W, false);
        assert!(fly.moving(), "still gliding");
        run(&mut fly, &mut cam, 0.05);
        assert!(fly.velocity().length() < 0.7 * DEFAULT_SPEED);
        run(&mut fly, &mut cam, 0.5);
        assert_eq!(fly.velocity(), DVec3::ZERO, "comes to rest");
        assert!(!fly.moving());
        let rest = cam.clone();
        assert!(!fly.advance(&mut cam, 1.0 / 60.0));
        assert_eq!(cam, rest);
    }

    #[test]
    fn speed_steps_and_clamps() {
        let mut fly = Fly::default();
        assert!(fly.step_speed(1));
        assert!((fly.speed() - DEFAULT_SPEED * SPEED_STEP).abs() < 1e-12);
        assert!(fly.step_speed(-2));
        assert!((fly.speed() - DEFAULT_SPEED / SPEED_STEP).abs() < 1e-12);
        for _ in 0..100 {
            fly.step_speed(1);
        }
        assert_eq!(fly.speed(), MAX_SPEED);
        assert!(!fly.step_speed(1));
        for _ in 0..100 {
            fly.step_speed(-1);
        }
        assert_eq!(fly.speed(), MIN_SPEED);
        assert!(!fly.step_speed(-1));
    }

    #[test]
    fn scrolled_speed_steps_add_up() {
        let mut fly = Fly::default();
        assert!(!fly.scroll_speed(0.4));
        assert!(!fly.scroll_speed(0.4));
        assert!(fly.scroll_speed(0.4), "1.2 steps");
        assert!((fly.speed() - DEFAULT_SPEED * SPEED_STEP).abs() < 1e-12);
        assert!(fly.scroll_speed(-1.3), "0.2 left over, less 1.3");
        assert!((fly.speed() - DEFAULT_SPEED).abs() < 1e-12);
        assert!(!fly.scroll_speed(f64::NAN));
    }

    #[test]
    fn looking_turns_right_and_down_and_stops_short_of_vertical() {
        let mut cam = camera();
        look(&mut cam, DVec2::new(100.0, 0.0));
        let f = cam.forward();
        assert!(f.y < 0.0, "turned right (toward -Y): {f}");
        assert!(f.z.abs() < 1e-12);
        let (yaw, _) = heading(&cam);
        assert!((yaw + 100.0 * LOOK_RAD_PER_PX).abs() < 1e-9);
        look(&mut cam, DVec2::new(0.0, 50.0));
        assert!(cam.forward().z < 0.0, "looks down");
        look(&mut cam, DVec2::new(0.0, 1e6));
        let (_, pitch) = heading(&cam);
        assert!((pitch + PITCH_LIMIT).abs() < 1e-9, "{pitch}");
        look(&mut cam, DVec2::new(0.0, -1e6));
        let (_, pitch) = heading(&cam);
        assert!((pitch - PITCH_LIMIT).abs() < 1e-9, "{pitch}");
        // The eye never moves when looking, and the target stays a meter ahead.
        assert!(cam.eye.abs_diff_eq(DVec3::new(0.0, 0.0, 1.7), 1e-12));
        assert!(((cam.target - cam.eye).length() - TARGET_AHEAD).abs() < 1e-9);
        let before = cam.clone();
        look(&mut cam, DVec2::new(f64::NAN, 0.0));
        assert_eq!(cam, before);
    }

    #[test]
    fn leveling_a_view_from_above_heads_up_the_screen() {
        let mut cam = Camera::new(DVec2::new(800.0, 600.0));
        cam.look_from(session::StandardView::Top);
        level(&mut cam);
        let f = cam.forward();
        assert!(f.y > 0.0 && f.z < 0.0, "{f}");
        assert!(!cam.is_vertical());
        assert_eq!(cam.up, UP);
    }

    #[test]
    fn frames_are_timed_and_clamped() {
        let now = Instant::now();
        assert_eq!(frame_dt(None, now), 0.0);
        let dt = frame_dt(Some(now), now + Duration::from_millis(16));
        assert!((dt - 0.016).abs() < 1e-9);
        assert_eq!(frame_dt(Some(now), now + Duration::from_secs(3)), MAX_FRAME);
        assert_eq!(frame_dt(Some(now + Duration::from_secs(1)), now), 0.0);
        // A long stall moves no farther than one clamped frame.
        let mut cam = camera();
        let mut fly = Fly::default();
        fly.key(FlyKey::W, true);
        run(&mut fly, &mut cam, 1.0);
        let start = cam.eye;
        fly.advance(&mut cam, 5.0);
        assert!((cam.eye - start).length() <= DEFAULT_SPEED * MAX_FRAME + 1e-9);
    }

    #[test]
    fn ticks_keep_frames_coming_only_while_moving() {
        let now = Instant::now();
        let mut cam = camera();
        let mut fly = Fly::default();
        assert!(!fly.tick(&mut cam, now), "idle stays idle");
        fly.key(FlyKey::W, true);
        assert!(fly.tick(&mut cam, now), "the first frame starts the clock");
        let before = cam.eye;
        assert!(fly.tick(&mut cam, now + Duration::from_millis(16)));
        assert!(cam.eye.x > before.x);
        fly.key(FlyKey::W, false);
        let mut t = now + Duration::from_millis(16);
        let mut frames = 0;
        while fly.tick(&mut cam, t) {
            t += Duration::from_millis(16);
            frames += 1;
            assert!(frames < 100, "comes to rest");
        }
    }

    #[test]
    fn releasing_everything_lets_the_eye_glide_to_a_stop() {
        let mut cam = camera();
        let mut fly = Fly::default();
        fly.key(FlyKey::W, true);
        fly.key(FlyKey::Ctrl, true);
        fly.set_modifiers(true, false);
        run(&mut fly, &mut cam, 0.5);
        fly.release_all();
        assert!(!fly.holds(FlyKey::W) && !fly.holds(FlyKey::Ctrl));
        assert_eq!(fly.effective_speed(), DEFAULT_SPEED);
        assert_eq!(fly.wish(&cam), DVec3::ZERO);
        fly.step_speed(2);
        fly.stop();
        assert!(!fly.moving());
        assert!(fly.speed() > DEFAULT_SPEED, "the speed is kept");
    }

    #[test]
    fn the_status_names_the_speed() {
        assert_eq!(
            status_text(3.0),
            "Fly mode: 3.0 m/s (wheel or [ ] to change, Shift faster, Esc to exit)"
        );
        assert!(status_text(0.25).starts_with("Fly mode: 0.25 m/s"));
    }
}
