//! Touch gestures as a pure state machine: touch events in (start, move, end, with ids and
//! positions in physical pixels and a time in milliseconds), camera and selection gestures out.
//!
//! One finger dragging orbits; two fingers pan (by the midpoint's movement) and zoom (by the
//! ratio of the distance between them); a short touch that barely moves is a tap, and two taps
//! close in time and place a double tap. A lifted finger of a pair does not start an orbit with
//! the finger left; a third finger is ignored.

use glam::DVec2;

/// Most movement, in logical pixels, a touch may make and still be a tap.
pub const TAP_SLOP: f64 = 10.0;
/// Longest a touch may last and still be a tap, in milliseconds.
pub const TAP_MAX_MS: u64 = 300;
/// Longest between two taps of a double tap (the end of the first to the end of the second), in
/// milliseconds.
pub const DOUBLE_TAP_MS: u64 = 300;
/// Furthest apart two taps of a double tap may be, in logical pixels.
pub const DOUBLE_TAP_DISTANCE: f64 = 30.0;

/// What a touch did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Start,
    Move,
    End,
    Cancel,
}

/// One touch event: the finger `id`, where it is (physical pixels) and when (milliseconds from
/// any fixed start).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchEvent {
    pub phase: Phase,
    pub id: u64,
    pub pos: DVec2,
    pub at_ms: u64,
}

/// What the touches ask of the view. Positions and deltas are physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    /// A drag begins at `at`: the point to turn or pan about is the one under it.
    Anchor { at: DVec2 },
    /// One finger moved by `delta`.
    Orbit { delta: DVec2 },
    /// Two fingers moved their midpoint (now at `at`) by `delta`.
    Pan { at: DVec2, delta: DVec2 },
    /// Two fingers changed their distance by `ratio` (above 1: apart) about `at`.
    Zoom { at: DVec2, ratio: f64 },
    /// A tap at `at`.
    Tap { at: DVec2 },
    /// The second tap of a double tap at `at`.
    DoubleTap { at: DVec2 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// No finger is down.
    Idle,
    /// One finger; `dragging` once it moved past the tap slop.
    One {
        start: DVec2,
        started_ms: u64,
        last: DVec2,
        dragging: bool,
    },
    /// Two fingers, from their ids: the last midpoint and distance.
    Two { mid: DVec2, distance: f64 },
    /// A pair was split or a third finger came: nothing more until every finger is up.
    Spent,
}

/// The gesture recognizer.
#[derive(Debug)]
pub struct Touches {
    mode: Mode,
    /// The fingers down, in the order they came.
    fingers: Vec<(u64, DVec2)>,
    last_tap: Option<(u64, DVec2)>,
    /// Physical pixels per logical pixel.
    scale: f64,
}

impl Default for Touches {
    fn default() -> Self {
        Self {
            mode: Mode::Idle,
            fingers: Vec::new(),
            last_tap: None,
            scale: 1.0,
        }
    }
}

impl Touches {
    pub fn set_scale_factor(&mut self, scale: f64) {
        if scale.is_finite() && scale > 0.0 {
            self.scale = scale;
        }
    }

    /// Whether any finger is down.
    #[cfg(test)]
    pub fn active(&self) -> bool {
        !self.fingers.is_empty()
    }

    /// Forgets every finger (the window lost focus; their ends may never come).
    pub fn reset(&mut self) {
        self.mode = Mode::Idle;
        self.fingers.clear();
        self.last_tap = None;
    }

    /// Takes one touch event; the gestures it makes are returned in order.
    pub fn event(&mut self, event: TouchEvent) -> Vec<Gesture> {
        let mut out = Vec::new();
        match event.phase {
            Phase::Start => self.start(event, &mut out),
            Phase::Move => self.moved(event, &mut out),
            Phase::End | Phase::Cancel => self.end(event, &mut out),
        }
        out
    }

    fn start(&mut self, event: TouchEvent, out: &mut Vec<Gesture>) {
        if self.fingers.iter().any(|(id, _)| *id == event.id) {
            return;
        }
        self.fingers.push((event.id, event.pos));
        match self.fingers.len() {
            1 => {
                self.mode = Mode::One {
                    start: event.pos,
                    started_ms: event.at_ms,
                    last: event.pos,
                    dragging: false,
                };
            }
            2 => {
                let (mid, distance) = self.pair();
                self.mode = Mode::Two { mid, distance };
                self.last_tap = None;
                out.push(Gesture::Anchor { at: mid });
            }
            _ => {
                self.mode = Mode::Spent;
                self.last_tap = None;
            }
        }
    }

    /// The midpoint of, and distance between, the first two fingers.
    fn pair(&self) -> (DVec2, f64) {
        let (a, b) = (self.fingers[0].1, self.fingers[1].1);
        ((a + b) * 0.5, a.distance(b))
    }

    fn moved(&mut self, event: TouchEvent, out: &mut Vec<Gesture>) {
        let Some(finger) = self.fingers.iter_mut().find(|(id, _)| *id == event.id) else {
            return;
        };
        finger.1 = event.pos;
        match self.mode {
            Mode::One {
                start,
                started_ms,
                last,
                dragging,
            } => {
                let mut dragging = dragging;
                let mut last = last;
                if !dragging && event.pos.distance(start) > TAP_SLOP * self.scale {
                    dragging = true;
                    self.last_tap = None;
                    out.push(Gesture::Anchor { at: start });
                }
                if dragging && event.pos != last {
                    out.push(Gesture::Orbit {
                        delta: event.pos - last,
                    });
                    last = event.pos;
                }
                self.mode = Mode::One {
                    start,
                    started_ms,
                    last,
                    dragging,
                };
            }
            Mode::Two { mid, distance } => {
                let (now_mid, now_distance) = self.pair();
                if now_mid != mid {
                    out.push(Gesture::Pan {
                        at: now_mid,
                        delta: now_mid - mid,
                    });
                }
                let mut distance_kept = distance;
                if distance > 0.0 && now_distance > 0.0 && now_distance != distance {
                    out.push(Gesture::Zoom {
                        at: now_mid,
                        ratio: now_distance / distance,
                    });
                }
                if now_distance > 0.0 {
                    distance_kept = now_distance;
                }
                self.mode = Mode::Two {
                    mid: now_mid,
                    distance: distance_kept,
                };
            }
            Mode::Idle | Mode::Spent => {}
        }
    }

    fn end(&mut self, event: TouchEvent, out: &mut Vec<Gesture>) {
        let Some(index) = self.fingers.iter().position(|(id, _)| *id == event.id) else {
            return;
        };
        self.fingers.remove(index);
        match self.mode {
            Mode::One {
                start,
                started_ms,
                dragging,
                ..
            } => {
                self.mode = Mode::Idle;
                let held = event.at_ms.saturating_sub(started_ms);
                let tapped = event.phase == Phase::End
                    && !dragging
                    && held <= TAP_MAX_MS
                    && event.pos.distance(start) <= TAP_SLOP * self.scale;
                if tapped {
                    self.tapped(event.pos, event.at_ms, out);
                }
            }
            Mode::Two { .. } | Mode::Spent => {
                self.mode = if self.fingers.is_empty() {
                    Mode::Idle
                } else {
                    Mode::Spent
                };
            }
            Mode::Idle => {}
        }
        if self.fingers.is_empty() {
            self.mode = Mode::Idle;
        }
    }

    fn tapped(&mut self, at: DVec2, at_ms: u64, out: &mut Vec<Gesture>) {
        let second = self.last_tap.take().is_some_and(|(then, place)| {
            at_ms.saturating_sub(then) <= DOUBLE_TAP_MS
                && at.distance(place) <= DOUBLE_TAP_DISTANCE * self.scale
        });
        if second {
            out.push(Gesture::DoubleTap { at });
        } else {
            self.last_tap = Some((at_ms, at));
            out.push(Gesture::Tap { at });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> DVec2 {
        DVec2::new(x, y)
    }

    fn ev(phase: Phase, id: u64, pos: DVec2, at_ms: u64) -> TouchEvent {
        TouchEvent {
            phase,
            id,
            pos,
            at_ms,
        }
    }

    fn run(t: &mut Touches, events: &[TouchEvent]) -> Vec<Gesture> {
        events.iter().flat_map(|e| t.event(*e)).collect()
    }

    #[test]
    fn one_finger_drag_orbits_by_its_movement() {
        let mut t = Touches::default();
        let g = run(
            &mut t,
            &[
                ev(Phase::Start, 1, p(100.0, 100.0), 0),
                ev(Phase::Move, 1, p(104.0, 100.0), 10),
            ],
        );
        assert!(g.is_empty(), "inside the slop: {g:?}");
        let g = run(
            &mut t,
            &[
                ev(Phase::Move, 1, p(130.0, 100.0), 20),
                ev(Phase::Move, 1, p(130.0, 90.0), 30),
            ],
        );
        assert_eq!(
            g,
            vec![
                Gesture::Anchor {
                    at: p(100.0, 100.0)
                },
                Gesture::Orbit {
                    delta: p(30.0, 0.0)
                },
                Gesture::Orbit {
                    delta: p(0.0, -10.0)
                },
            ]
        );
        assert!(t.event(ev(Phase::End, 1, p(130.0, 90.0), 40)).is_empty());
        assert!(!t.active());
    }

    #[test]
    fn two_fingers_pan_by_the_midpoint() {
        let mut t = Touches::default();
        let g = run(
            &mut t,
            &[
                ev(Phase::Start, 1, p(100.0, 100.0), 0),
                ev(Phase::Start, 2, p(200.0, 100.0), 0),
            ],
        );
        assert_eq!(
            g,
            vec![Gesture::Anchor {
                at: p(150.0, 100.0)
            }]
        );
        // Both move by (20, 10): the distance is unchanged, only the midpoint moves.
        let g = run(
            &mut t,
            &[
                ev(Phase::Move, 1, p(120.0, 110.0), 10),
                ev(Phase::Move, 2, p(220.0, 110.0), 10),
            ],
        );
        let pans: Vec<_> = g
            .iter()
            .filter_map(|g| match g {
                Gesture::Pan { delta, .. } => Some(*delta),
                _ => None,
            })
            .collect();
        let total: DVec2 = pans.iter().sum();
        assert!((total - p(20.0, 10.0)).length() < 1e-9, "{pans:?}");
        assert!(
            g.iter().all(|g| !matches!(g, Gesture::Orbit { .. })),
            "two fingers never orbit"
        );
    }

    #[test]
    fn pinch_zooms_by_the_distance_ratio() {
        let mut t = Touches::default();
        run(
            &mut t,
            &[
                ev(Phase::Start, 1, p(100.0, 100.0), 0),
                ev(Phase::Start, 2, p(200.0, 100.0), 0),
            ],
        );
        let g = t.event(ev(Phase::Move, 2, p(300.0, 100.0), 10));
        let zooms: Vec<f64> = g
            .iter()
            .filter_map(|g| match g {
                Gesture::Zoom { ratio, .. } => Some(*ratio),
                _ => None,
            })
            .collect();
        assert_eq!(zooms.len(), 1);
        assert!((zooms[0] - 2.0).abs() < 1e-12, "100 apart to 200 apart");
        let g = t.event(ev(Phase::Move, 2, p(200.0, 100.0), 20));
        let zoom = g.iter().find_map(|g| match g {
            Gesture::Zoom { ratio, .. } => Some(*ratio),
            _ => None,
        });
        assert!((zoom.unwrap() - 0.5).abs() < 1e-12, "back together");
    }

    #[test]
    fn a_short_still_touch_is_a_tap() {
        let mut t = Touches::default();
        let g = run(
            &mut t,
            &[
                ev(Phase::Start, 1, p(50.0, 60.0), 1000),
                ev(Phase::Move, 1, p(53.0, 62.0), 1050),
                ev(Phase::End, 1, p(53.0, 62.0), 1100),
            ],
        );
        assert_eq!(g, vec![Gesture::Tap { at: p(53.0, 62.0) }]);
    }

    #[test]
    fn a_long_or_moved_or_cancelled_touch_is_not_a_tap() {
        let mut t = Touches::default();
        let g = run(
            &mut t,
            &[
                ev(Phase::Start, 1, p(50.0, 60.0), 0),
                ev(Phase::End, 1, p(50.0, 60.0), TAP_MAX_MS + 1),
            ],
        );
        assert!(g.is_empty(), "held too long: {g:?}");
        let g = run(
            &mut t,
            &[
                ev(Phase::Start, 1, p(50.0, 60.0), 1000),
                ev(Phase::Move, 1, p(90.0, 60.0), 1010),
                ev(Phase::Move, 1, p(50.0, 60.0), 1020),
                ev(Phase::End, 1, p(50.0, 60.0), 1030),
            ],
        );
        assert!(
            g.iter().all(|g| !matches!(g, Gesture::Tap { .. })),
            "a drag that came back is still a drag: {g:?}"
        );
        let g = run(
            &mut t,
            &[
                ev(Phase::Start, 1, p(50.0, 60.0), 2000),
                ev(Phase::Cancel, 1, p(50.0, 60.0), 2010),
            ],
        );
        assert!(g.is_empty(), "cancelled: {g:?}");
    }

    #[test]
    fn the_slop_grows_with_the_display_scale() {
        let mut t = Touches::default();
        t.set_scale_factor(3.0);
        // 20 physical pixels is under 10 logical ones at 3x.
        let g = run(
            &mut t,
            &[
                ev(Phase::Start, 1, p(0.0, 0.0), 0),
                ev(Phase::Move, 1, p(20.0, 0.0), 10),
                ev(Phase::End, 1, p(20.0, 0.0), 20),
            ],
        );
        assert_eq!(g, vec![Gesture::Tap { at: p(20.0, 0.0) }]);
    }

    fn tap(t: &mut Touches, at: DVec2, ms: u64) -> Vec<Gesture> {
        run(
            t,
            &[ev(Phase::Start, 1, at, ms), ev(Phase::End, 1, at, ms + 40)],
        )
    }

    #[test]
    fn two_close_taps_are_a_double_tap_once() {
        let mut t = Touches::default();
        assert_eq!(
            tap(&mut t, p(100.0, 100.0), 0),
            vec![Gesture::Tap {
                at: p(100.0, 100.0)
            }]
        );
        assert_eq!(
            tap(&mut t, p(110.0, 105.0), 200),
            vec![Gesture::DoubleTap {
                at: p(110.0, 105.0)
            }]
        );
        // A third tap starts over.
        assert_eq!(
            tap(&mut t, p(110.0, 105.0), 400),
            vec![Gesture::Tap {
                at: p(110.0, 105.0)
            }]
        );
    }

    #[test]
    fn taps_too_slow_or_too_far_are_two_taps() {
        let mut t = Touches::default();
        tap(&mut t, p(100.0, 100.0), 0);
        let g = tap(&mut t, p(100.0, 100.0), 40 + DOUBLE_TAP_MS + 1);
        assert_eq!(
            g,
            vec![Gesture::Tap {
                at: p(100.0, 100.0)
            }]
        );
        let mut t = Touches::default();
        tap(&mut t, p(100.0, 100.0), 0);
        let g = tap(&mut t, p(100.0 + DOUBLE_TAP_DISTANCE + 1.0, 100.0), 200);
        assert!(matches!(g.as_slice(), [Gesture::Tap { .. }]), "{g:?}");
    }

    #[test]
    fn a_drag_between_taps_does_not_make_a_double_tap() {
        let mut t = Touches::default();
        tap(&mut t, p(100.0, 100.0), 0);
        run(
            &mut t,
            &[
                ev(Phase::Start, 1, p(100.0, 100.0), 100),
                ev(Phase::Move, 1, p(160.0, 100.0), 110),
                ev(Phase::End, 1, p(160.0, 100.0), 120),
            ],
        );
        let g = tap(&mut t, p(100.0, 100.0), 200);
        assert_eq!(
            g,
            vec![Gesture::Tap {
                at: p(100.0, 100.0)
            }],
            "the drag ended the first tap's chance of a double"
        );
    }

    #[test]
    fn two_fingers_are_never_a_tap() {
        let mut t = Touches::default();
        let g = run(
            &mut t,
            &[
                ev(Phase::Start, 1, p(100.0, 100.0), 0),
                ev(Phase::Start, 2, p(140.0, 100.0), 5),
                ev(Phase::End, 2, p(140.0, 100.0), 30),
                ev(Phase::End, 1, p(100.0, 100.0), 40),
            ],
        );
        assert!(
            g.iter()
                .all(|g| !matches!(g, Gesture::Tap { .. } | Gesture::DoubleTap { .. })),
            "{g:?}"
        );
        assert!(!t.active());
    }

    #[test]
    fn the_finger_left_after_a_pair_does_not_orbit_or_tap() {
        let mut t = Touches::default();
        run(
            &mut t,
            &[
                ev(Phase::Start, 1, p(100.0, 100.0), 0),
                ev(Phase::Start, 2, p(200.0, 100.0), 0),
                ev(Phase::End, 2, p(200.0, 100.0), 50),
            ],
        );
        let g = run(
            &mut t,
            &[
                ev(Phase::Move, 1, p(150.0, 150.0), 60),
                ev(Phase::End, 1, p(150.0, 150.0), 70),
            ],
        );
        assert!(g.is_empty(), "{g:?}");
        // The next touch is fresh again.
        let g = run(
            &mut t,
            &[
                ev(Phase::Start, 3, p(10.0, 10.0), 1000),
                ev(Phase::Move, 3, p(60.0, 10.0), 1010),
            ],
        );
        assert!(g.iter().any(|g| matches!(g, Gesture::Orbit { .. })));
    }

    #[test]
    fn a_third_finger_and_unknown_ids_are_ignored() {
        let mut t = Touches::default();
        assert!(t.event(ev(Phase::Move, 9, p(1.0, 1.0), 0)).is_empty());
        assert!(t.event(ev(Phase::End, 9, p(1.0, 1.0), 0)).is_empty());
        run(
            &mut t,
            &[
                ev(Phase::Start, 1, p(100.0, 100.0), 0),
                ev(Phase::Start, 2, p(200.0, 100.0), 0),
                ev(Phase::Start, 3, p(300.0, 100.0), 0),
            ],
        );
        let g = t.event(ev(Phase::Move, 3, p(400.0, 100.0), 10));
        assert!(g.is_empty(), "{g:?}");
        run(
            &mut t,
            &[
                ev(Phase::End, 1, p(100.0, 100.0), 20),
                ev(Phase::End, 2, p(200.0, 100.0), 20),
                ev(Phase::End, 3, p(400.0, 100.0), 20),
            ],
        );
        assert!(!t.active());
    }

    #[test]
    fn reset_forgets_the_fingers() {
        let mut t = Touches::default();
        t.event(ev(Phase::Start, 1, p(0.0, 0.0), 0));
        assert!(t.active());
        t.reset();
        assert!(!t.active());
        assert!(t.event(ev(Phase::Move, 1, p(90.0, 0.0), 5)).is_empty());
    }
}
