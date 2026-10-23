//! What the phone viewer does with touches: orbit, pan, pinch, tap and double tap, in a model
//! that cannot be edited (`set_read_only`).

use super::*;
use crate::touch::Gesture;
use crate::viewer::measure::{self, Measure};
use crate::viewer::parts::{self, Part, PartRow};
use session::SelectMode;
use tools::infer_in;
use tools::overlay::{ON_FACE, ScreenLine, clip_segment};

/// The card shown for the selected part or parts.
#[derive(Clone, Debug, PartialEq)]
pub struct PartCard {
    /// The part's name, or "N parts" (or "Name x N") for several.
    pub title: String,
    /// "width x depth x height" in the document's units; for one part only.
    pub size: Option<String>,
    /// "1 of 3" for one component.
    pub ordinal: Option<String>,
    /// Whether some parts are hidden (Show all is offered).
    pub isolated: bool,
}

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
            // Measuring: every tap, a quick second one too, is a point.
            Gesture::Tap { at } | Gesture::DoubleTap { at } if self.measure.is_some() => {
                self.measure_tap(at)
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

    /// The model's parts (top-level groups and components) with their sizes, kept until the
    /// model, its revision or the document changes.
    pub fn parts(&self) -> Rc<Vec<Part>> {
        let key = (self.documents, self.doc.revision());
        if let Some((documents, revision, parts)) = self.parts.borrow().as_ref()
            && (*documents, *revision) == key
        {
            return parts.clone();
        }
        let parts = Rc::new(parts::list(self.doc.scene()));
        *self.parts.borrow_mut() = Some((key.0, key.1, parts.clone()));
        parts
    }

    /// The parts list's rows: a component's instances together.
    pub fn part_rows(&self) -> Vec<PartRow> {
        parts::rows(&self.parts())
    }

    /// The selected parts, in id order.
    fn selected_parts(&self) -> Vec<Part> {
        self.parts()
            .iter()
            .filter(|p| self.doc.selection().contains(&EntityId::Instance(p.id)))
            .cloned()
            .collect()
    }

    /// What the card says of the selection; None when no part is selected.
    pub fn part_card(&self) -> Option<PartCard> {
        let chosen = self.selected_parts();
        let first = chosen.first()?;
        let isolated = self.doc.isolated();
        Some(match chosen.as_slice() {
            [one] => PartCard {
                title: one.name.clone(),
                size: one.size.map(|s| parts::size_text(s, &self.doc.units())),
                ordinal: one.ordinal(),
                isolated,
            },
            many => PartCard {
                title: if many.iter().all(|p| p.definition == first.definition) {
                    format!("{} x {}", first.name, many.len())
                } else {
                    format!("{} parts", many.len())
                },
                size: None,
                ordinal: None,
                isolated,
            },
        })
    }

    /// Selects `ids` (parts) and frames them, as a tap on a row of the parts list does. Parts
    /// that were hidden by Isolate come back first. False when none of them exists.
    pub fn select_parts(&mut self, ids: &[InstanceId]) -> bool {
        let known: Vec<EntityId> = self
            .parts()
            .iter()
            .filter(|p| ids.contains(&p.id))
            .map(|p| EntityId::Instance(p.id))
            .collect();
        if known.is_empty() {
            return false;
        }
        self.doc.show_all();
        self.run(Command::Select {
            ids: known,
            mode: SelectMode::Replace,
            all: false,
        });
        self.invalidate_hover();
        self.frame_selection();
        true
    }

    /// Hides every part but the selected ones, for looking only (nothing is saved or undone),
    /// and frames what is left. False when no part is selected.
    pub fn isolate_selection(&mut self) -> bool {
        let keep: Vec<InstanceId> = self.selected_parts().iter().map(|p| p.id).collect();
        if !self.doc.isolate(&keep) {
            return false;
        }
        self.invalidate_hover();
        self.frame_selection();
        true
    }

    /// Shows every part again. False when none was hidden.
    pub fn show_all(&mut self) -> bool {
        let shown = self.doc.show_all();
        if shown {
            self.invalidate_hover();
        }
        shown
    }

    /// Whether some parts are hidden by Isolate.
    pub fn isolated(&self) -> bool {
        self.doc.isolated()
    }

    /// Turns Measure on: taps now pick points. Nothing stays selected.
    pub fn start_measure(&mut self) {
        self.select_none();
        self.measure = Some(Measure::default());
    }

    /// Turns Measure off (Done).
    pub fn stop_measure(&mut self) {
        self.measure = None;
    }

    /// The measurement while Measure is on.
    pub fn measurement(&self) -> Option<&Measure> {
        self.measure.as_ref()
    }

    /// Picks the point under `at` (view pixels) for the measurement: the nearest vertex, edge
    /// middle or edge within a finger's reach (`measure::TOUCH_RADIUS_PX`), else the face
    /// under it. A tap on nothing is ignored. True if the measurement changed.
    pub fn measure_tap(&mut self, at: DVec2) -> bool {
        if self.measure.is_none() {
            return false;
        }
        let to_root = self.doc.context_transform();
        let layout = self.layout();
        let scenery = scenery_in(&layout, &self.doc);
        let camera = self.doc.camera.placed(to_root.inverse());
        let hit = infer_in(
            &scenery,
            &camera,
            at,
            &Guides::default(),
            measure::snap_radius(self.scale_factor),
        );
        if !measure::on_model(hit.kind) {
            return false;
        }
        let point = to_root.transform_point3(hit.point);
        if let Some(m) = &mut self.measure {
            m.tap(point);
        }
        true
    }

    /// The measurement as lines over the view: a cross at each point and the line between them.
    pub(super) fn measure_lines(&self) -> Vec<ScreenLine> {
        let Some(m) = &self.measure else {
            return Vec::new();
        };
        let camera = &self.doc.camera;
        let arm = 8.0 * self.scale_factor;
        let line = |a: DVec2, b: DVec2, width: f32| ScreenLine {
            a,
            b,
            color: ON_FACE,
            width,
        };
        let mut out = Vec::new();
        if let (Some(a), Some(b)) = (m.first(), m.second())
            && let Some([a, b]) = clip_segment(camera, a, b)
        {
            out.push(line(a, b, 3.0));
        }
        for p in [m.first(), m.second()].into_iter().flatten() {
            if let Some(c) = camera.project(p) {
                let (dx, dy) = (DVec2::new(arm, 0.0), DVec2::new(0.0, arm));
                out.push(line(c - dx, c + dx, 3.0));
                out.push(line(c - dy, c + dy, 3.0));
            }
        }
        out
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

    /// Draws a 2 m square at `x`..`x`+2 (y 1..3) with the Line tool and makes it a group, or a
    /// component named `component`.
    fn add_part(e: &mut Editor, x: f64, component: Option<&str>) {
        let before: Vec<EntityId> = {
            let m = e.doc().model();
            m.edges()
                .map(|(id, _)| EntityId::Edge(id))
                .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
                .collect()
        };
        e.select_tool(ToolKind::Line);
        for (px, py) in [(x, 1.0), (x + 2.0, 1.0), (x + 2.0, 3.0), (x, 3.0), (x, 1.0)] {
            let px = e.doc().camera.project(DVec3::new(px, py, 0.0)).unwrap();
            e.cursor_moved(px);
            e.button(Button::Left, true);
            e.button(Button::Left, false);
        }
        let new: Vec<EntityId> = {
            let m = e.doc().model();
            m.edges()
                .map(|(id, _)| EntityId::Edge(id))
                .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
                .filter(|id| !before.contains(id))
                .collect()
        };
        e.execute(Command::Select {
            ids: new.clone(),
            mode: session::SelectMode::Replace,
            all: false,
        })
        .unwrap();
        match component {
            Some(name) => {
                e.execute(Command::MakeComponent {
                    ids: new,
                    name: name.into(),
                })
                .unwrap();
            }
            None => assert!(e.make_group()),
        }
        e.select_none();
    }

    fn add_group(e: &mut Editor, x: f64) {
        add_part(e, x, None);
    }

    /// A viewer holding two 2 m square groups, side by side (x 1..3 and x 5..7), both in view.
    fn two_parts() -> Editor {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 2.0);
        add_group(&mut e, 1.0);
        add_group(&mut e, 5.0);
        e.set_read_only(true);
        // The squares fill the view, so a finger's reach is a small part of one.
        e.zoom_extents();
        e
    }

    fn center(e: &Editor, x: f64) -> DVec2 {
        e.doc()
            .camera
            .project(DVec3::new(x, 2.0, 0.0))
            .expect("in view")
    }

    fn tap(e: &mut Editor, at: DVec2) {
        e.touch(Gesture::Tap { at });
    }

    fn selected_ids(e: &Editor) -> Vec<InstanceId> {
        e.doc()
            .selection()
            .iter()
            .filter_map(|id| match id {
                EntityId::Instance(i) => Some(*i),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_card_names_the_tapped_part_and_gives_its_size() {
        let mut e = two_parts();
        assert_eq!(e.part_card(), None);
        let at = center(&e, 2.0);
        tap(&mut e, at);
        let card = e.part_card().unwrap();
        assert_eq!(card.title, "Group");
        assert_eq!(card.size.as_deref(), Some("2 m x 2 m x 0 m"));
        assert_eq!(card.ordinal, None);
        assert!(!card.isolated);
        let at = center(&e, 6.0);
        tap(&mut e, at);
        assert_eq!(selected_ids(&e).len(), 1, "the tap replaced the selection");
        tap(&mut e, DVec2::new(5.0, 5.0));
        assert_eq!(e.part_card(), None);
    }

    #[test]
    fn the_card_says_one_of_n_for_a_component() {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 2.0);
        add_part(&mut e, 1.0, Some("Crate"));
        let first = e.parts()[0].id;
        e.execute(Command::Copy {
            ids: vec![EntityId::Instance(first)],
            offset: [4.0, 0.0, 0.0],
            count: Some(1),
        })
        .unwrap();
        e.select_none();
        e.set_read_only(true);
        let at = center(&e, 2.0);
        tap(&mut e, at);
        let card = e.part_card().unwrap();
        assert_eq!(card.title, "Crate");
        assert_eq!(card.ordinal.as_deref(), Some("1 of 2"));
        let at = center(&e, 6.0);
        tap(&mut e, at);
        assert_eq!(e.part_card().unwrap().ordinal.as_deref(), Some("2 of 2"));
        // Both together, from the list's one row for the component.
        let rows = e.part_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 2);
        e.select_parts(&rows[0].ids);
        let card = e.part_card().unwrap();
        assert_eq!(card.title, "Crate x 2");
        assert_eq!((card.size, card.ordinal), (None, None));
    }

    #[test]
    fn a_row_selects_and_frames_its_parts() {
        let mut e = two_parts();
        let rows = e.part_rows();
        assert_eq!(rows.len(), 2);
        let far = e.doc().camera.distance();
        assert!(e.select_parts(&rows[1].ids));
        assert_eq!(selected_ids(&e), rows[1].ids);
        assert!(e.doc().camera.distance() < far, "framed closer");
        let c = e.doc().camera.target;
        assert!((c.x - 6.0).abs() < 0.5 && (c.y - 2.0).abs() < 0.5, "{c:?}");
        assert!(!e.select_parts(&[InstanceId(9999)]));
    }

    #[test]
    fn isolate_hides_every_other_part_and_show_all_brings_them_back() {
        let mut e = two_parts();
        let left = center(&e, 2.0);
        let revision = e.doc().revision();
        let undo = e.doc().undo_label().map(str::to_string);
        let modified = e.doc().is_modified();
        tap(&mut e, left);
        assert!(e.isolate_selection());
        assert!(e.isolated());
        assert!(e.part_card().unwrap().isolated);
        // The hidden part cannot be hit; the kept one still can.
        e.select_none();
        let hidden_at = e.doc().camera.project(DVec3::new(6.0, 2.0, 0.0)).unwrap();
        tap(&mut e, hidden_at);
        assert!(
            selected_ids(&e).is_empty(),
            "nothing is where the hidden part was"
        );
        let shown_at = e.doc().camera.project(DVec3::new(2.0, 2.0, 0.0)).unwrap();
        tap(&mut e, shown_at);
        assert_eq!(selected_ids(&e).len(), 1);
        // View only: no undo step, still saved, the document's own edit state untouched.
        assert_eq!(e.doc().undo_label().map(str::to_string), undo);
        assert_eq!(e.doc().is_modified(), modified);
        assert!(e.doc().revision() > revision);
        // Zoom extents fits what is shown.
        e.zoom_extents();
        assert!(e.doc().camera.target.x < 4.0, "{:?}", e.doc().camera.target);
        assert!(e.show_all());
        assert!(!e.isolated());
        assert!(!e.show_all());
        e.select_none();
        let at = e.doc().camera.project(DVec3::new(6.0, 2.0, 0.0)).unwrap();
        tap(&mut e, at);
        assert_eq!(selected_ids(&e).len(), 1, "the other part is back");
    }

    #[test]
    fn isolate_needs_a_selected_part() {
        let mut e = two_parts();
        assert!(!e.isolate_selection());
        assert!(!e.isolated());
    }

    #[test]
    fn a_row_tap_brings_back_hidden_parts_first() {
        let mut e = two_parts();
        let at = center(&e, 2.0);
        tap(&mut e, at);
        e.isolate_selection();
        let rows = e.part_rows();
        assert_eq!(rows.len(), 2, "the list keeps every part");
        assert!(e.select_parts(&rows[1].ids));
        assert!(!e.isolated());
    }

    /// The pixel of a model point.
    fn px(e: &Editor, p: DVec3) -> DVec2 {
        e.doc().camera.project(p).unwrap()
    }

    #[test]
    fn measure_gives_the_distance_between_two_snapped_points() {
        let mut e = two_parts();
        e.start_measure();
        assert!(e.measurement().is_some());
        // Corners (1, 1) and (7, 3), about 10 logical px off (20 physical at 2x): beyond the
        // mouse's 8 px but inside a finger's 24.
        let off = DVec2::new(14.0, 14.0);
        let a = px(&e, DVec3::new(1.0, 1.0, 0.0));
        let b = px(&e, DVec3::new(7.0, 3.0, 0.0));
        assert!(e.touch(Gesture::Tap { at: a + off }));
        assert_eq!(e.measurement().unwrap().distance(), None);
        assert!(e.touch(Gesture::Tap { at: b + off }));
        let d = e.measurement().unwrap().distance().unwrap();
        assert!((d - (36.0f64 + 4.0).sqrt()).abs() < 1e-9, "{d}");
        // A third tap starts over.
        e.touch(Gesture::Tap { at: a });
        assert_eq!(e.measurement().unwrap().distance(), None);
        assert!(
            e.measurement()
                .unwrap()
                .first()
                .unwrap()
                .abs_diff_eq(DVec3::new(1.0, 1.0, 0.0), 1e-9)
        );
    }

    #[test]
    fn the_snap_reaches_24_logical_pixels_and_no_further() {
        let mut e = two_parts();
        e.start_measure();
        let corner = DVec3::new(1.0, 1.0, 0.0);
        let c = px(&e, corner);
        // Away from the origin (a snap too), so only the corner is near.
        let dir = (c - px(&e, DVec3::ZERO)).normalize();
        let scale = 2.0;
        e.touch(Gesture::Tap {
            at: c + dir * 22.0 * scale,
        });
        let hit = e.measurement().unwrap().first();
        assert!(
            hit.is_some_and(|p| p.abs_diff_eq(corner, 1e-9)),
            "22 logical px snaps: {hit:?}"
        );
        e.stop_measure();
        e.start_measure();
        e.touch(Gesture::Tap {
            at: c + dir * 40.0 * scale,
        });
        let far = e.measurement().unwrap().first();
        assert!(
            far.is_none_or(|p| !p.abs_diff_eq(corner, 1e-6)),
            "40 logical px away is out of the corner's reach: {far:?}"
        );
    }

    #[test]
    fn measure_mode_takes_taps_for_points_and_done_gives_them_back() {
        let mut e = two_parts();
        let revision = e.doc().revision();
        e.start_measure();
        let at = center(&e, 2.0);
        e.touch(Gesture::Tap { at });
        assert!(
            selected_ids(&e).is_empty(),
            "taps measure, they do not select"
        );
        assert!(e.measurement().unwrap().first().is_some());
        // A quick second tap arrives as a double tap and is the second point, not a zoom.
        let before = e.doc().camera.clone();
        let other = center(&e, 6.0);
        e.touch(Gesture::DoubleTap { at: other });
        assert_eq!(e.doc().camera, before);
        assert!(e.measurement().unwrap().distance().is_some());
        // The model never changes.
        assert_eq!(e.doc().revision(), revision);
        e.stop_measure();
        assert!(e.measurement().is_none());
        e.touch(Gesture::Tap { at });
        assert_eq!(selected_ids(&e).len(), 1);
    }

    #[test]
    fn the_measurement_is_drawn_and_a_new_model_forgets_it() {
        let mut e = two_parts();
        assert!(e.measure_lines().is_empty());
        e.start_measure();
        let a = px(&e, DVec3::new(1.0, 1.0, 0.0));
        let b = px(&e, DVec3::new(7.0, 3.0, 0.0));
        e.touch(Gesture::Tap { at: a });
        assert_eq!(e.measure_lines().len(), 2, "a cross");
        e.touch(Gesture::Tap { at: b });
        assert_eq!(e.measure_lines().len(), 5, "two crosses and the line");
        assert_eq!(e.hud(false).lines.len(), 5);
        let bytes = e.doc().encode_bytes().unwrap();
        assert!(e.open_bytes("again", &bytes));
        assert!(e.measurement().is_none());
    }

    #[test]
    fn a_tap_on_nothing_does_not_measure() {
        let mut e = two_parts();
        e.start_measure();
        assert!(!e.touch(Gesture::Tap {
            at: DVec2::new(3.0, 3.0)
        }));
        assert_eq!(e.measurement().unwrap().first(), None);
    }
}
