use glam::{DVec2, DVec3};
use render::Camera;

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

/// Turns raw pointer input into camera moves. Middle drag orbits, Shift+middle pans,
/// Alt+left does the same for trackpads, scroll zooms toward the cursor.
#[derive(Default)]
pub struct Navigation {
    cursor: DVec2,
    shift: bool,
    alt: bool,
    drag: Option<(Button, Drag)>,
}

impl Navigation {
    pub fn set_modifiers(&mut self, shift: bool, alt: bool) {
        self.shift = shift;
        self.alt = alt;
    }

    /// Drops any drag and modifier state, e.g. when the window loses focus and release
    /// events would never arrive. The cursor position is kept.
    pub fn reset(&mut self) {
        self.drag = None;
        self.shift = false;
        self.alt = false;
    }

    pub fn button(&mut self, cam: &Camera, button: Button, pressed: bool) {
        if !pressed {
            if self
                .drag
                .is_some_and(|(started_by, _)| started_by == button)
            {
                self.drag = None;
            }
            return;
        }
        let navigates = button == Button::Middle || (button == Button::Left && self.alt);
        if !navigates {
            return;
        }
        let point = cam.anchor_at(self.cursor);
        let drag = if self.shift {
            Drag::Pan { anchor: point }
        } else {
            Drag::Orbit { pivot: point }
        };
        self.drag = Some((button, drag));
    }

    pub fn cursor_moved(&mut self, cam: &mut Camera, pos: DVec2) -> bool {
        let delta = pos - self.cursor;
        self.cursor = pos;
        match self.drag {
            Some((_, Drag::Orbit { pivot })) => cam.orbit(pivot, delta),
            Some((_, Drag::Pan { anchor })) => cam.pan(anchor, delta),
            None => return false,
        }
        true
    }

    pub fn scroll(&mut self, cam: &mut Camera, steps: f64) -> bool {
        if steps == 0.0 {
            return false;
        }
        let anchor = cam.anchor_at(self.cursor);
        cam.zoom(anchor, steps);
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
        nav.button(&cam, Button::Middle, true);
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
        nav.button(&cam, Button::Middle, true);
        assert!(nav.cursor_moved(&mut cam, DVec2::new(430.0, 280.0)));
        assert!(cam.forward().abs_diff_eq(forward, 1e-12));
        assert!(!cam.target.abs_diff_eq(target, 1e-6));
    }

    #[test]
    fn plain_left_drag_is_left_for_tools() {
        let (mut nav, mut cam) = setup();
        let before = cam.clone();
        nav.button(&cam, Button::Left, true);
        assert!(!nav.cursor_moved(&mut cam, DVec2::new(500.0, 350.0)));
        assert_eq!(cam, before);
    }

    #[test]
    fn alt_left_drag_orbits_for_trackpads() {
        let (mut nav, mut cam) = setup();
        let before = cam.clone();
        nav.set_modifiers(false, true);
        nav.button(&cam, Button::Left, true);
        assert!(nav.cursor_moved(&mut cam, DVec2::new(500.0, 300.0)));
        assert_ne!(cam, before);
    }

    #[test]
    fn release_ends_drag() {
        let (mut nav, mut cam) = setup();
        nav.button(&cam, Button::Middle, true);
        nav.button(&cam, Button::Middle, false);
        let before = cam.clone();
        assert!(!nav.cursor_moved(&mut cam, DVec2::new(500.0, 300.0)));
        assert_eq!(cam, before);
    }

    #[test]
    fn other_button_release_does_not_end_middle_drag() {
        let (mut nav, mut cam) = setup();
        nav.button(&cam, Button::Middle, true);
        nav.button(&cam, Button::Left, true);
        nav.button(&cam, Button::Left, false);
        let before = cam.clone();
        assert!(nav.cursor_moved(&mut cam, DVec2::new(460.0, 300.0)));
        assert_ne!(cam, before);
    }

    #[test]
    fn middle_release_does_not_end_alt_left_orbit() {
        let (mut nav, mut cam) = setup();
        nav.set_modifiers(false, true);
        nav.button(&cam, Button::Left, true);
        nav.button(&cam, Button::Middle, false);
        let before = cam.clone();
        assert!(nav.cursor_moved(&mut cam, DVec2::new(460.0, 300.0)));
        assert_ne!(cam, before);
    }

    #[test]
    fn reset_clears_drag_and_modifiers() {
        let (mut nav, mut cam) = setup();
        nav.set_modifiers(true, false);
        nav.button(&cam, Button::Middle, true);
        nav.reset();
        let before = cam.clone();
        assert!(!nav.cursor_moved(&mut cam, DVec2::new(460.0, 300.0)));
        assert_eq!(cam, before);
        nav.button(&cam, Button::Middle, true);
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
        assert!(nav.scroll(&mut cam, 1.0));
        assert!((cam.eye - anchor).length() < d);
        assert!(!nav.scroll(&mut cam, 0.0));
    }
}
