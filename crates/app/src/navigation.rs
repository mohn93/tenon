use glam::{DVec2, DVec3};
use session::Camera;
use session::camera::ZOOM_STEP;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    Orbit { pivot: DVec3 },
    Pan { anchor: DVec3 },
}

/// A scroll event's delta, as the window reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scroll {
    /// Notches of a classic mouse wheel; positive y is away from the user.
    Lines(DVec2),
    /// Physical pixels from a trackpad (or another smooth-scrolling device); positive is the
    /// content moving right and down.
    Pixels(DVec2),
}

/// Logical pixels of Cmd/Ctrl + trackpad scroll per zoom step.
pub const LOGICAL_PX_PER_ZOOM_STEP: f64 = 100.0;
/// Most zoom steps one scroll or pinch event may apply, so a hard flick or an accelerated
/// wheel cannot jump.
pub const MAX_ZOOM_STEPS_PER_EVENT: f64 = 3.0;

/// Turns raw pointer input into camera moves.
///
/// Mouse: middle drag orbits, Shift+middle pans, Option/Alt+left (Shift+Option+left) does the
/// same without a middle button, and the wheel zooms toward the cursor.
///
/// Trackpad (Blender style): two-finger scroll orbits as if dragging the scene, Shift+scroll
/// pans, Cmd/Ctrl+scroll and pinch zoom toward the cursor.
///
/// Every move centers on what is under the cursor: the model point the caller picks there,
/// else the ground within reach, else the point as deep as the target (see
/// `Camera::anchor_for`). Orbit speed is per logical pixel, the same at any display scale.
pub struct Navigation {
    cursor: DVec2,
    shift: bool,
    alt: bool,
    /// Cmd or Ctrl.
    command: bool,
    /// Physical pixels per logical pixel.
    scale: f64,
    drag: Option<(Button, Drag)>,
    /// The pivot or anchor of the trackpad scroll in progress. The cursor does not move while
    /// two fingers scroll, so it holds until the cursor moves, a button or zoom acts, or the
    /// scroll switches between orbiting and panning.
    gesture: Option<Drag>,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            cursor: DVec2::ZERO,
            shift: false,
            alt: false,
            command: false,
            scale: 1.0,
            drag: None,
            gesture: None,
        }
    }
}

impl Navigation {
    pub fn set_modifiers(&mut self, shift: bool, alt: bool) {
        self.shift = shift;
        self.alt = alt;
    }

    /// Whether Cmd or Ctrl is held: trackpad scrolling then zooms.
    pub fn set_command(&mut self, held: bool) {
        self.command = held;
    }

    pub fn set_scale_factor(&mut self, scale: f64) {
        if scale.is_finite() && scale > 0.0 {
            self.scale = scale;
        }
    }

    /// Drops any drag and modifier state, e.g. when the window loses focus and release
    /// events would never arrive. The cursor position is kept.
    pub fn reset(&mut self) {
        self.drag = None;
        self.gesture = None;
        self.shift = false;
        self.alt = false;
        self.command = false;
    }

    /// Handles a button press or release. `pick` finds the model point under the cursor; it
    /// is called only when a drag starts. Returns true if navigation used the event, so it
    /// must not also go to the active tool.
    pub fn button(
        &mut self,
        cam: &Camera,
        button: Button,
        pressed: bool,
        pick: impl FnOnce() -> Option<DVec3>,
    ) -> bool {
        if !pressed {
            if self
                .drag
                .is_some_and(|(started_by, _)| started_by == button)
            {
                self.drag = None;
                return true;
            }
            return false;
        }
        if self.drag.is_some() {
            return true;
        }
        let navigates = button == Button::Middle || (button == Button::Left && self.alt);
        if !navigates {
            return false;
        }
        self.gesture = None;
        let point = cam.anchor_for(self.cursor, pick());
        let drag = if self.shift {
            Drag::Pan { anchor: point }
        } else {
            Drag::Orbit { pivot: point }
        };
        self.drag = Some((button, drag));
        true
    }

    pub fn cursor_moved(&mut self, cam: &mut Camera, pos: DVec2) -> bool {
        let delta = pos - self.cursor;
        self.cursor = pos;
        if delta != DVec2::ZERO {
            self.gesture = None;
        }
        match self.drag {
            Some((_, drag)) => self.apply(cam, drag, delta),
            None => return false,
        }
        true
    }

    /// Moves the camera by a cursor (or scroll) delta in physical pixels: orbiting turns by
    /// the logical length, panning keeps the anchor under the cursor.
    fn apply(&self, cam: &mut Camera, drag: Drag, delta_px: DVec2) {
        match drag {
            Drag::Orbit { pivot } => cam.orbit(pivot, delta_px / self.scale),
            Drag::Pan { anchor } => cam.pan(anchor, delta_px),
        }
    }

    /// A scroll event. A mouse wheel, or Cmd/Ctrl with a trackpad, zooms toward the cursor;
    /// otherwise trackpad scrolling orbits like a middle drag of the same delta, or pans with
    /// Shift. `pick` is the model point under the cursor, if any. Returns true if the camera
    /// moved.
    pub fn scroll(&mut self, cam: &mut Camera, scroll: Scroll, pick: Option<DVec3>) -> bool {
        let delta = match scroll {
            Scroll::Lines(lines) => return self.zoom(cam, lines.y, pick),
            Scroll::Pixels(px) if self.command => {
                return self.zoom(cam, px.y / self.scale / LOGICAL_PX_PER_ZOOM_STEP, pick);
            }
            Scroll::Pixels(px) => px,
        };
        if self.drag.is_some() || !delta.is_finite() || delta == DVec2::ZERO {
            return false;
        }
        let pan = self.shift;
        let gesture = match self.gesture {
            Some(g @ Drag::Pan { .. }) if pan => g,
            Some(g @ Drag::Orbit { .. }) if !pan => g,
            _ => {
                let point = cam.anchor_for(self.cursor, pick);
                if pan {
                    Drag::Pan { anchor: point }
                } else {
                    Drag::Orbit { pivot: point }
                }
            }
        };
        self.gesture = Some(gesture);
        self.apply(cam, gesture, delta);
        true
    }

    /// A trackpad pinch: `delta` is the change in magnification (positive zooms in). Zooms
    /// toward the cursor; ignores deltas that are not finite or would invert the view.
    pub fn pinch(&mut self, cam: &mut Camera, delta: f64, pick: Option<DVec3>) -> bool {
        if !delta.is_finite() || delta <= -1.0 {
            return false;
        }
        self.zoom(cam, (1.0 + delta).ln() / -ZOOM_STEP.ln(), pick)
    }

    /// Zooms `steps` notches toward the cursor (at most `MAX_ZOOM_STEPS_PER_EVENT` either
    /// way). Returns true if the camera moved.
    pub fn zoom(&mut self, cam: &mut Camera, steps: f64, pick: Option<DVec3>) -> bool {
        if !steps.is_finite() || steps == 0.0 {
            return false;
        }
        let steps = steps.clamp(-MAX_ZOOM_STEPS_PER_EVENT, MAX_ZOOM_STEPS_PER_EVENT);
        self.gesture = None;
        cam.zoom_at(self.cursor, pick, steps);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Navigation, Camera) {
        let mut nav = Navigation::default();
        let mut cam = Camera::new(DVec2::new(800.0, 600.0));
        nav.cursor_moved(&mut cam, DVec2::new(400.0, 300.0));
        (nav, cam)
    }

    #[test]
    fn button_reports_whether_navigation_used_it() {
        let (mut nav, cam) = setup();
        assert!(
            !nav.button(&cam, Button::Left, true, || None),
            "plain left belongs to tools"
        );
        assert!(!nav.button(&cam, Button::Left, false, || None));
        assert!(nav.button(&cam, Button::Middle, true, || None));
        assert!(nav.button(&cam, Button::Middle, false, || None));
        nav.set_modifiers(false, true);
        assert!(
            nav.button(&cam, Button::Left, true, || None),
            "alt+left orbits"
        );
    }

    #[test]
    fn hover_without_buttons_does_nothing() {
        let (mut nav, mut cam) = setup();
        let before = cam.clone();
        assert!(!nav.cursor_moved(&mut cam, DVec2::new(450.0, 320.0)));
        assert_eq!(cam, before);
    }

    #[test]
    fn middle_drag_orbits_around_point_under_cursor() {
        let (mut nav, mut cam) = setup();
        let pivot = cam.anchor_at(DVec2::new(400.0, 300.0));
        let d = (cam.eye - pivot).length();
        nav.button(&cam, Button::Middle, true, || None);
        assert!(nav.cursor_moved(&mut cam, DVec2::new(460.0, 300.0)));
        assert!(((cam.eye - pivot).length() - d).abs() < 1e-9);
        assert!(!cam.eye.abs_diff_eq(Camera::new(cam.viewport).eye, 1e-6));
    }

    #[test]
    fn shift_middle_drag_pans_without_rotating() {
        let (mut nav, mut cam) = setup();
        let forward = cam.forward();
        let target = cam.target;
        nav.set_modifiers(true, false);
        nav.button(&cam, Button::Middle, true, || None);
        assert!(nav.cursor_moved(&mut cam, DVec2::new(430.0, 280.0)));
        assert!(cam.forward().abs_diff_eq(forward, 1e-12));
        assert!(!cam.target.abs_diff_eq(target, 1e-6));
    }

    #[test]
    fn plain_left_drag_is_left_for_tools() {
        let (mut nav, mut cam) = setup();
        let before = cam.clone();
        nav.button(&cam, Button::Left, true, || None);
        assert!(!nav.cursor_moved(&mut cam, DVec2::new(500.0, 350.0)));
        assert_eq!(cam, before);
    }

    #[test]
    fn alt_left_drag_orbits_for_trackpads() {
        let (mut nav, mut cam) = setup();
        let before = cam.clone();
        nav.set_modifiers(false, true);
        nav.button(&cam, Button::Left, true, || None);
        assert!(nav.cursor_moved(&mut cam, DVec2::new(500.0, 300.0)));
        assert_ne!(cam, before);
    }

    #[test]
    fn release_ends_drag() {
        let (mut nav, mut cam) = setup();
        nav.button(&cam, Button::Middle, true, || None);
        nav.button(&cam, Button::Middle, false, || None);
        let before = cam.clone();
        assert!(!nav.cursor_moved(&mut cam, DVec2::new(500.0, 300.0)));
        assert_eq!(cam, before);
    }

    #[test]
    fn other_button_release_does_not_end_middle_drag() {
        let (mut nav, mut cam) = setup();
        nav.button(&cam, Button::Middle, true, || None);
        nav.button(&cam, Button::Left, true, || None);
        nav.button(&cam, Button::Left, false, || None);
        let before = cam.clone();
        assert!(nav.cursor_moved(&mut cam, DVec2::new(460.0, 300.0)));
        assert_ne!(cam, before);
    }

    #[test]
    fn other_presses_during_a_drag_are_consumed() {
        let (mut nav, mut cam) = setup();
        nav.button(&cam, Button::Middle, true, || None);
        assert!(
            nav.button(&cam, Button::Left, true, || None),
            "no stray tool click"
        );
        let before = cam.clone();
        assert!(nav.cursor_moved(&mut cam, DVec2::new(460.0, 300.0)));
        assert_ne!(cam, before, "drag still orbits");
        assert!(nav.button(&cam, Button::Middle, false, || None));
        let after = cam.clone();
        assert!(!nav.cursor_moved(&mut cam, DVec2::new(500.0, 300.0)));
        assert_eq!(cam, after);
    }

    #[test]
    fn middle_release_does_not_end_alt_left_orbit() {
        let (mut nav, mut cam) = setup();
        nav.set_modifiers(false, true);
        nav.button(&cam, Button::Left, true, || None);
        nav.button(&cam, Button::Middle, false, || None);
        let before = cam.clone();
        assert!(nav.cursor_moved(&mut cam, DVec2::new(460.0, 300.0)));
        assert_ne!(cam, before);
    }

    #[test]
    fn reset_clears_drag_and_modifiers() {
        let (mut nav, mut cam) = setup();
        nav.set_modifiers(true, false);
        nav.button(&cam, Button::Middle, true, || None);
        nav.reset();
        let before = cam.clone();
        assert!(!nav.cursor_moved(&mut cam, DVec2::new(460.0, 300.0)));
        assert_eq!(cam, before);
        nav.button(&cam, Button::Middle, true, || None);
        let forward = cam.forward();
        assert!(nav.cursor_moved(&mut cam, DVec2::new(500.0, 300.0)));
        // Orbit rotates the view direction; pan would not.
        assert!(!cam.forward().abs_diff_eq(forward, 1e-9));
    }

    #[test]
    fn scroll_zooms_toward_point_under_cursor() {
        let (mut nav, mut cam) = setup();
        let anchor = cam.anchor_at(DVec2::new(400.0, 300.0));
        let d = (cam.eye - anchor).length();
        assert!(nav.zoom(&mut cam, 1.0, None));
        assert!((cam.eye - anchor).length() < d);
        assert!(!nav.zoom(&mut cam, 0.0, None));
    }

    /// The angle the view direction turned through.
    fn turned(before: &Camera, after: &Camera) -> f64 {
        before.forward().angle_between(after.forward())
    }

    #[test]
    fn orbit_speed_is_per_logical_pixel() {
        let (mut nav1, mut cam1) = setup();
        nav1.button(&cam1, Button::Middle, true, || None);
        nav1.cursor_moved(&mut cam1, DVec2::new(450.0, 300.0));
        let (mut nav2, mut cam2) = setup();
        nav2.set_scale_factor(2.0);
        nav2.button(&cam2, Button::Middle, true, || None);
        nav2.cursor_moved(&mut cam2, DVec2::new(500.0, 300.0));
        let start = setup().1;
        let (a, b) = (turned(&start, &cam1), turned(&start, &cam2));
        assert!(a > 0.1 && (a - b).abs() < 1e-9, "{a} vs {b}");
    }

    #[test]
    fn pan_keeps_the_anchor_under_the_cursor_at_any_scale() {
        let (mut nav, mut cam) = setup();
        nav.set_scale_factor(2.0);
        nav.set_modifiers(true, false);
        let anchor = cam.anchor_at(DVec2::new(400.0, 300.0));
        nav.button(&cam, Button::Middle, true, || None);
        nav.cursor_moved(&mut cam, DVec2::new(440.0, 270.0));
        let at = cam.project(anchor).unwrap();
        assert!(at.abs_diff_eq(DVec2::new(440.0, 270.0), 1e-6), "{at}");
    }

    #[test]
    fn a_drag_orbits_about_the_picked_model_point() {
        let (mut nav, mut cam) = setup();
        let pick = cam.eye + cam.forward() * 4.0;
        let d = cam.eye.distance(pick);
        nav.button(&cam, Button::Middle, true, || Some(pick));
        nav.cursor_moved(&mut cam, DVec2::new(480.0, 330.0));
        assert!((cam.eye.distance(pick) - d).abs() < 1e-9);
        assert!(
            cam.project(pick)
                .unwrap()
                .abs_diff_eq(DVec2::new(400.0, 300.0), 1e-6)
        );
    }

    #[test]
    fn plain_presses_do_not_pick() {
        let (mut nav, cam) = setup();
        let mut picked = false;
        nav.button(&cam, Button::Left, true, || {
            picked = true;
            None
        });
        assert!(!picked, "a tool click needs no navigation raycast");
    }

    #[test]
    fn trackpad_scroll_orbits_like_a_middle_drag() {
        let (mut nav, mut dragged) = setup();
        nav.set_scale_factor(2.0);
        nav.button(&dragged, Button::Middle, true, || None);
        nav.cursor_moved(&mut dragged, DVec2::new(430.0, 290.0));
        let (mut nav, mut scrolled) = setup();
        nav.set_scale_factor(2.0);
        assert!(nav.scroll(&mut scrolled, Scroll::Pixels(DVec2::new(30.0, -10.0)), None));
        assert!(scrolled.eye.abs_diff_eq(dragged.eye, 1e-9));
        assert!(scrolled.target.abs_diff_eq(dragged.target, 1e-9));
    }

    #[test]
    fn trackpad_orbit_keeps_its_pivot_until_the_cursor_moves() {
        let (mut nav, mut cam) = setup();
        let pick = cam.eye + cam.forward() * 5.0;
        nav.scroll(&mut cam, Scroll::Pixels(DVec2::new(40.0, 0.0)), Some(pick));
        let d = cam.eye.distance(pick);
        // Later events pick something else under the cursor; the pivot holds.
        let other = Some(DVec3::new(100.0, 100.0, 0.0));
        for _ in 0..5 {
            nav.scroll(&mut cam, Scroll::Pixels(DVec2::new(15.0, 5.0)), other);
            assert!((cam.eye.distance(pick) - d).abs() < 1e-9);
        }
        nav.cursor_moved(&mut cam, DVec2::new(401.0, 300.0));
        let before = cam.eye.distance(DVec3::new(100.0, 100.0, 0.0));
        nav.scroll(&mut cam, Scroll::Pixels(DVec2::new(15.0, 0.0)), other);
        let after = cam.eye.distance(DVec3::new(100.0, 100.0, 0.0));
        assert!(
            (before - after).abs() < 1e-9,
            "a new pivot after the cursor moved"
        );
    }

    #[test]
    fn shift_trackpad_scroll_pans_with_the_fingers() {
        let (mut nav, mut cam) = setup();
        nav.set_scale_factor(2.0);
        nav.set_modifiers(true, false);
        let forward = cam.forward();
        let anchor = cam.anchor_at(DVec2::new(400.0, 300.0));
        assert!(nav.scroll(&mut cam, Scroll::Pixels(DVec2::new(20.0, 10.0)), None));
        assert!(nav.scroll(&mut cam, Scroll::Pixels(DVec2::new(20.0, 10.0)), None));
        assert!(cam.forward().abs_diff_eq(forward, 1e-12));
        let at = cam.project(anchor).unwrap();
        assert!(at.abs_diff_eq(DVec2::new(440.0, 320.0), 1e-6), "{at}");
    }

    #[test]
    fn command_trackpad_scroll_zooms() {
        let (mut nav, mut cam) = setup();
        nav.set_scale_factor(2.0);
        nav.set_command(true);
        let forward = cam.forward();
        let d = cam.distance();
        let px = 2.0 * LOGICAL_PX_PER_ZOOM_STEP;
        assert!(nav.scroll(&mut cam, Scroll::Pixels(DVec2::new(0.0, px)), None));
        assert!(cam.forward().abs_diff_eq(forward, 1e-12));
        assert!((cam.distance() - d * ZOOM_STEP).abs() < 1e-9);
    }

    #[test]
    fn wheel_zooms_and_is_clamped_per_event() {
        let (mut nav, mut cam) = setup();
        let d = cam.distance();
        assert!(nav.scroll(&mut cam, Scroll::Lines(DVec2::new(0.0, 1.0)), None));
        assert!((cam.distance() - d * ZOOM_STEP).abs() < 1e-9);
        let (mut nav, mut cam) = setup();
        assert!(nav.scroll(&mut cam, Scroll::Lines(DVec2::new(0.0, 40.0)), None));
        let most = d * ZOOM_STEP.powf(MAX_ZOOM_STEPS_PER_EVENT);
        assert!((cam.distance() - most).abs() < 1e-9);
        assert!(!nav.scroll(&mut cam, Scroll::Lines(DVec2::ZERO), None));
    }

    #[test]
    fn zoom_goes_toward_the_picked_point() {
        let (mut nav, mut cam) = setup();
        let pick = cam.eye + cam.forward() * 3.0;
        assert!(nav.zoom(&mut cam, 1.0, Some(pick)));
        assert!((cam.eye.distance(pick) - 3.0 * ZOOM_STEP).abs() < 1e-9);
    }

    #[test]
    fn pinch_zooms_toward_the_cursor() {
        let (mut nav, mut cam) = setup();
        let d = cam.distance();
        assert!(nav.pinch(&mut cam, 0.1, None));
        assert!(
            (cam.distance() - d / 1.1).abs() < 1e-9,
            "magnifies by 1 + delta"
        );
        let d = cam.distance();
        assert!(nav.pinch(&mut cam, -0.1, None));
        assert!((cam.distance() - d / 0.9).abs() < 1e-9);
        let before = cam.clone();
        assert!(!nav.pinch(&mut cam, f64::NAN, None));
        assert!(!nav.pinch(&mut cam, f64::INFINITY, None));
        assert!(!nav.pinch(&mut cam, -1.0, None));
        assert!(!nav.pinch(&mut cam, 0.0, None));
        assert_eq!(cam, before);
        // A huge pinch is clamped like a scroll.
        assert!(nav.pinch(&mut cam, 50.0, None));
        let most = before.distance() * ZOOM_STEP.powf(MAX_ZOOM_STEPS_PER_EVENT);
        assert!((cam.distance() - most).abs() < 1e-9);
    }
}
