//! What the phone viewer does with touches: orbit, pan, pinch, tap and double tap, in a model
//! that cannot be edited (`set_read_only`).

use super::*;
use crate::touch::Gesture;

impl Editor {
    /// Makes the editor a viewer (or not): the document refuses every command that changes the
    /// model, and only the Select tool can be chosen, so the model is looked at and parts are
    /// selected, nothing more.
    pub fn set_read_only(&mut self, on: bool) {
        self.doc.set_read_only(on);
        if on {
            self.select_tool(ToolKind::Select);
        }
    }

    /// Acts on a touch gesture (positions and deltas in view pixels). True if anything visible
    /// may have changed.
    pub fn touch(&mut self, gesture: Gesture) -> bool {
        match gesture {
            Gesture::Anchor { at } => {
                self.touch_pivot = Some(self.anchor_at_pixel(at));
                false
            }
            Gesture::Orbit { delta } => {
                let pivot = self.touch_pivot.unwrap_or(self.doc.camera.target);
                self.doc.camera.orbit(pivot, delta / self.scale_factor);
                self.moved_camera();
                true
            }
            Gesture::Pan { at, delta } => {
                let pivot = match self.touch_pivot {
                    Some(p) => p,
                    None => self.anchor_at_pixel(at),
                };
                self.doc.camera.pan(pivot, delta);
                self.moved_camera();
                true
            }
            Gesture::Zoom { at, ratio } => {
                if !ratio.is_finite() || ratio <= 0.0 {
                    return false;
                }
                self.cursor_moved(at);
                self.pinch(ratio - 1.0)
            }
            Gesture::Tap { at } => self.select_part(at),
            Gesture::DoubleTap { at } => {
                let selected = self.select_part(at);
                let framed = if self.doc.selection().is_empty() {
                    self.zoom_extents()
                } else {
                    self.frame_selection()
                };
                selected || framed
            }
        }
    }

    fn moved_camera(&mut self) {
        self.navigated = true;
        self.invalidate_hover();
    }

    /// The point to turn or pan about under a pixel: the model's surface there, else the
    /// ground, else a point as deep as the target.
    fn anchor_at_pixel(&mut self, at: DVec2) -> DVec3 {
        let layout = self.layout();
        let to_root = self.doc.context_transform();
        let scenery = scenery_in(&layout, &self.doc);
        let camera = &self.doc.camera;
        camera.anchor_for(at, pick(&scenery, to_root, camera, at))
    }

    /// Selects the part (top-level group or component) under the pixel, or nothing when there
    /// is none: the Select tool's click, then only instances may stay selected. True if
    /// anything visible may have changed.
    pub fn select_part(&mut self, at: DVec2) -> bool {
        self.cursor_moved(at);
        self.button(Button::Left, true);
        self.button(Button::Left, false);
        let loose = self
            .doc
            .selection()
            .iter()
            .any(|id| !matches!(id, EntityId::Instance(_)));
        if loose {
            self.select_none();
        }
        self.invalidate_hover();
        true
    }

    /// Moves the camera, keeping its direction, to fit the selected parts in the view. False
    /// when nothing is selected or the selection has no extent.
    pub fn frame_selection(&mut self) -> bool {
        let scene = self.doc.scene();
        let root = scene.root();
        let mut lo = DVec3::splat(f64::INFINITY);
        let mut hi = DVec3::splat(f64::NEG_INFINITY);
        for id in self.doc.selection() {
            let EntityId::Instance(id) = id else { continue };
            let Some(instance) = root.instance(*id) else {
                continue;
            };
            if let Some([a, b]) = scene.instance_bounds(instance) {
                lo = lo.min(a);
                hi = hi.max(b);
            }
        }
        if !lo.is_finite() || !hi.is_finite() {
            return false;
        }
        let before = self.doc.camera.clone();
        self.doc.camera.fit(lo, hi);
        let moved = self.doc.camera != before;
        if moved {
            self.moved_camera();
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A viewer holding one group (a 2 m square face) well inside the view.
    fn viewer() -> Editor {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 2.0);
        e.select_tool(ToolKind::Line);
        for (x, y) in [(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0), (1.0, 1.0)] {
            let px = e.doc().camera.project(DVec3::new(x, y, 0.0)).unwrap();
            e.cursor_moved(px);
            e.button(Button::Left, true);
            e.button(Button::Left, false);
        }
        e.select_all();
        assert!(e.make_group());
        e.select_none();
        e.set_read_only(true);
        e
    }

    fn square_center(e: &Editor) -> DVec2 {
        e.doc()
            .camera
            .project(DVec3::new(2.0, 2.0, 0.0))
            .expect("in view")
    }

    fn selected_instances(e: &Editor) -> usize {
        e.doc()
            .selection()
            .iter()
            .filter(|id| matches!(id, EntityId::Instance(_)))
            .count()
    }

    #[test]
    fn a_viewer_cannot_edit_or_switch_tools() {
        let mut e = viewer();
        assert_eq!(e.tool_kind(), ToolKind::Select);
        for kind in ToolKind::ALL {
            e.select_tool(kind);
            assert_eq!(e.tool_kind(), ToolKind::Select, "{kind:?}");
        }
        let revision = e.doc().revision();
        let error = e
            .execute(Command::AddEdge {
                start: [0.0, 0.0, 0.0],
                end: [1.0, 0.0, 0.0],
            })
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidInput);
        let steps = e.doc().undo_label().map(str::to_string);
        assert!(steps.is_some(), "there was a step to undo");
        e.undo();
        e.redo();
        e.select_all();
        e.erase_selection();
        assert_eq!(e.doc().revision(), revision);
        assert_eq!(e.doc().undo_label().map(str::to_string), steps);
    }

    #[test]
    fn a_tap_selects_the_part_and_a_tap_on_nothing_clears_it() {
        let mut e = viewer();
        let at = square_center(&e);
        e.touch(Gesture::Tap { at });
        assert_eq!(selected_instances(&e), 1);
        assert_eq!(e.doc().selection().len(), 1, "the part alone, not its face");
        e.touch(Gesture::Tap {
            at: DVec2::new(5.0, 5.0),
        });
        assert!(e.doc().selection().is_empty());
        assert_eq!(e.doc().context().len(), 0, "taps never open a group");
    }

    #[test]
    fn a_double_tap_frames_the_part_or_the_whole_model() {
        let mut e = viewer();
        let at = square_center(&e);
        let before = e.doc().camera.clone();
        e.touch(Gesture::DoubleTap { at });
        assert_eq!(selected_instances(&e), 1);
        assert!(
            e.doc().camera.distance() < before.distance(),
            "closer to the small part"
        );
        // On empty space it fits everything.
        e.touch(Gesture::DoubleTap {
            at: DVec2::new(3.0, 3.0),
        });
        assert!(e.doc().selection().is_empty());
        let mut whole = viewer();
        whole.zoom_extents();
        let (a, b) = (&e.doc().camera, &whole.doc().camera);
        assert!(a.target.abs_diff_eq(b.target, 1e-6), "fits the whole model");
        assert!((a.distance() - b.distance()).abs() < 1e-6);
    }

    #[test]
    fn one_finger_orbits_two_pan_and_pinch_zooms() {
        let mut e = viewer();
        let at = square_center(&e);
        let start = e.doc().camera.clone();

        e.touch(Gesture::Anchor { at });
        e.touch(Gesture::Orbit {
            delta: DVec2::new(80.0, 0.0),
        });
        let orbited = e.doc().camera.clone();
        assert_ne!(orbited.eye, start.eye);
        assert!(
            (orbited.distance() - start.distance()).abs() < 0.5,
            "orbiting keeps the distance roughly"
        );

        e.touch(Gesture::Anchor { at });
        e.touch(Gesture::Pan {
            at,
            delta: DVec2::new(40.0, 20.0),
        });
        let panned = e.doc().camera.clone();
        assert_ne!(panned.target, orbited.target);
        assert!(panned.forward().abs_diff_eq(orbited.forward(), 1e-9));

        e.touch(Gesture::Zoom { at, ratio: 2.0 });
        assert!(
            e.doc().camera.distance() < panned.distance(),
            "apart zooms in"
        );
        let closer = e.doc().camera.distance();
        e.touch(Gesture::Zoom { at, ratio: 0.5 });
        assert!(e.doc().camera.distance() > closer, "together zooms out");
    }

    #[test]
    fn the_model_is_unchanged_by_all_of_it() {
        let mut e = viewer();
        let revision = e.doc().revision();
        let at = square_center(&e);
        for gesture in [
            Gesture::Tap { at },
            Gesture::DoubleTap { at },
            Gesture::Anchor { at },
            Gesture::Orbit {
                delta: DVec2::new(30.0, 10.0),
            },
            Gesture::Zoom { at, ratio: 1.5 },
        ] {
            e.touch(gesture);
        }
        assert_eq!(e.doc().revision(), revision);
    }
}
