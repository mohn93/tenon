//! The Paint tool (Paint Bucket): click a face to paint the side facing you with the current
//! material, or a group to paint the group. Clicking something selected paints every selected
//! face and group. With the sampling modifier (Cmd on macOS, Ctrl elsewhere) held, a click
//! picks up the material clicked instead. What is under the cursor is reported as hovered.

use glam::DVec3;
use kernel::{EntityId, FaceId, Side};
use session::Command;

use crate::inference::face_under_in;
use crate::scenery::Owner;
use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};

#[derive(Clone, Debug, Default)]
pub struct PaintTool {
    /// The face or group a click would paint.
    hovered: Option<EntityId>,
}

impl PaintTool {
    pub fn new() -> Self {
        Self::default()
    }
}

/// What a click reaches: the open context's face under the cursor (with the side facing the
/// viewer), or the instance whose geometry is under it. Geometry outside the open instance
/// hides what is behind it but is not painted.
enum Target {
    Face(FaceId, Side),
    Instance(EntityId),
}

fn target(ctx: &ToolContext) -> Option<Target> {
    match face_under_in(ctx.scenery, ctx.camera, ctx.cursor)? {
        (Owner::Open, f, _) => {
            let normal = ctx.model.face(f)?.normal;
            let side = if ctx.camera.ray(ctx.cursor).dir.dot(normal) > 0.0 {
                Side::Back
            } else {
                Side::Front
            };
            Some(Target::Face(f, side))
        }
        (Owner::Instance(i), _, _) => Some(Target::Instance(EntityId::Instance(i))),
        (Owner::Outside, _, _) => None,
    }
}

impl Tool for PaintTool {
    fn name(&self) -> &'static str {
        "Paint"
    }

    fn prompt(&self) -> &'static str {
        "Click a face or group to paint it; Cmd/Ctrl-click picks up a material"
    }

    fn measurement(&self, _units: &session::Units) -> (&'static str, Option<String>) {
        ("", None)
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    /// Each click acts at once.
    fn phase(&self) -> Phase {
        Phase::Idle
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(_) => {
                self.hovered = target(ctx).map(|t| match t {
                    Target::Face(f, _) => EntityId::Face(f),
                    Target::Instance(id) => id,
                });
            }
            ToolEvent::Click(_) => {
                self.hovered = None;
                let Some(target) = target(ctx) else {
                    out.info = true;
                    out.message =
                        Some("Paint works on faces and groups: click one to paint it".into());
                    return out;
                };
                let (id, side) = match target {
                    Target::Face(f, side) => (EntityId::Face(f), side),
                    Target::Instance(id) => (id, Side::Front),
                };
                if ctx.sample {
                    out.sampled = Some(match target {
                        Target::Face(f, side) => ctx.model.face(f).and_then(|face| match side {
                            Side::Back => face.back,
                            _ => face.front,
                        }),
                        Target::Instance(EntityId::Instance(i)) => {
                            ctx.model.instance(i).and_then(|i| i.material)
                        }
                        Target::Instance(_) => None,
                    });
                } else {
                    // A click on something selected paints the whole selection's faces (on the
                    // side clicked) and groups.
                    let ids = if ctx.selection.contains(&id) {
                        ctx.selection
                            .iter()
                            .copied()
                            .filter(|e| matches!(e, EntityId::Face(_) | EntityId::Instance(_)))
                            .collect()
                    } else {
                        vec![id]
                    };
                    out.commands.push(Command::Paint {
                        ids,
                        material: ctx.material,
                        side,
                    });
                }
            }
            ToolEvent::Value(_) => {
                out.message = Some("The Paint tool takes no typed values".into());
            }
            ToolEvent::Cancel => self.hovered = None,
            ToolEvent::Release(_)
            | ToolEvent::DoubleClick(_)
            | ToolEvent::TripleClick(_)
            | ToolEvent::ToggleCopy => {}
        }
        out
    }

    fn on_external_change(&mut self) {
        self.hovered = None;
    }

    fn hovered(&self) -> Option<EntityId> {
        self.hovered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind};
    use crate::scenery::Scenery;
    use glam::DVec2;
    use kernel::{DefinitionKind, MaterialId, MeshId, Model, Scene};
    use session::Camera;
    use std::collections::BTreeSet;

    fn hit() -> Inference {
        Inference {
            point: DVec3::ZERO,
            kind: SnapKind::Free,
            face: None,
            locked: None,
        }
    }

    /// A 2 x 2 square on the ground at x 0..2, and a grouped one at x 3..5.
    fn scene() -> Scene {
        let mut m = Model::new();
        let square = |x: f64| {
            [
                DVec3::new(x, 0.0, 0.0),
                DVec3::new(x + 2.0, 0.0, 0.0),
                DVec3::new(x + 2.0, 2.0, 0.0),
                DVec3::new(x, 2.0, 0.0),
            ]
        };
        m.add_polygon(&square(0.0)).unwrap();
        m.add_polygon(&square(3.0)).unwrap();
        let mut s = Scene::from_root(m);
        let far = |s: &Scene| {
            let m = s.root();
            let edges: Vec<_> = m
                .edges()
                .filter(|(_, e)| e.v.iter().all(|v| m.position(*v).unwrap().x >= 3.0))
                .map(|(e, _)| e)
                .collect();
            let faces: Vec<_> = m
                .faces()
                .filter(|(_, f)| m.position(f.outer[0]).unwrap().x >= 3.0)
                .map(|(f, _)| f)
                .collect();
            (edges, faces)
        };
        let (edges, faces) = far(&s);
        s.make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
            .unwrap();
        s
    }

    fn send(
        tool: &mut PaintTool,
        scene: &Scene,
        camera: &Camera,
        at: DVec3,
        sample: bool,
        selection: &BTreeSet<EntityId>,
        event: ToolEvent,
    ) -> ToolOutput {
        let scenery = Scenery::of(scene, &[]).unwrap();
        let ctx = ToolContext {
            model: scene.root(),
            scenery: &scenery,
            camera,
            cursor: camera.project(at).unwrap(),
            pick_radius: 10.0,
            drag_threshold: 4.0,
            scale_factor: 1.0,
            to_world: glam::DAffine3::IDENTITY,
            quick_press: false,
            units: session::Units::default(),
            shift: false,
            selection,
            material: Some(MaterialId(77)),
            sample,
            labels: &[],
        };
        tool.handle(event, &ctx)
    }

    fn face_at(scene: &Scene, x: f64) -> EntityId {
        let m = scene.root();
        let f = m
            .faces()
            .find(|(_, f)| (m.position(f.outer[0]).unwrap().x - x).abs() < 2.1)
            .unwrap()
            .0;
        EntityId::Face(f)
    }

    #[test]
    fn clicking_paints_the_side_facing_the_viewer_and_groups_whole() {
        let s = scene();
        let none = BTreeSet::new();
        let mut t = PaintTool::new();
        let above = Camera::new(DVec2::new(800.0, 600.0));
        let on_square = DVec3::new(1.0, 1.0, 0.0);
        let out = send(
            &mut t,
            &s,
            &above,
            on_square,
            false,
            &none,
            ToolEvent::Click(hit()),
        );
        let face = face_at(&s, 0.0);
        assert_eq!(
            out.commands,
            vec![Command::Paint {
                ids: vec![face],
                material: Some(MaterialId(77)),
                side: Side::Front,
            }]
        );
        // From below, the back is painted.
        let mut below = Camera::new(DVec2::new(800.0, 600.0));
        below.eye = DVec3::new(-8.0, -12.0, -7.0);
        let out = send(
            &mut t,
            &s,
            &below,
            on_square,
            false,
            &none,
            ToolEvent::Click(hit()),
        );
        assert!(matches!(
            &out.commands[..],
            [Command::Paint {
                side: Side::Back,
                ..
            }]
        ));
        // The grouped square paints its group.
        let group = EntityId::Instance(s.root().instances().next().unwrap().0);
        let on_group = DVec3::new(4.0, 1.0, 0.0);
        let out = send(
            &mut t,
            &s,
            &above,
            on_group,
            false,
            &none,
            ToolEvent::Click(hit()),
        );
        assert!(matches!(&out.commands[..], [Command::Paint { ids, .. }] if *ids == vec![group]));
        // Clicking a selected face paints the whole selection's faces and groups.
        let selection = BTreeSet::from([face, group]);
        let out = send(
            &mut t,
            &s,
            &above,
            on_square,
            false,
            &selection,
            ToolEvent::Click(hit()),
        );
        assert!(
            matches!(&out.commands[..], [Command::Paint { ids, .. }] if *ids == vec![face, group])
        );
        // Nothing under the cursor: nothing happens.
        let off = DVec3::new(-3.0, 6.0, 0.0);
        let out = send(
            &mut t,
            &s,
            &above,
            off,
            false,
            &none,
            ToolEvent::Click(hit()),
        );
        assert!(out.commands.is_empty() && out.sampled.is_none());
    }

    #[test]
    fn clicking_nothing_says_what_paint_works_on() {
        let s = scene();
        let cam = Camera::new(DVec2::new(800.0, 600.0));
        let mut t = PaintTool::new();
        let off = DVec3::new(-3.0, 4.0, 0.0);
        let none = BTreeSet::new();
        let out = send(&mut t, &s, &cam, off, false, &none, ToolEvent::Click(hit()));
        assert!(out.commands.is_empty() && out.info);
        assert_eq!(
            out.message.as_deref(),
            Some("Paint works on faces and groups: click one to paint it")
        );
    }

    #[test]
    fn hovering_reports_the_face_or_group_a_click_would_paint() {
        let s = scene();
        let none = BTreeSet::new();
        let mut t = PaintTool::new();
        let above = Camera::new(DVec2::new(800.0, 600.0));
        let hover = |t: &mut PaintTool, at: DVec3| {
            let out = send(t, &s, &above, at, false, &none, ToolEvent::Move(hit()));
            assert!(out.preview.is_empty(), "no outline at {at}");
            t.hovered()
        };
        let face = face_at(&s, 0.0);
        assert_eq!(hover(&mut t, DVec3::new(1.0, 1.0, 0.0)), Some(face));
        let group = EntityId::Instance(s.root().instances().next().unwrap().0);
        assert_eq!(hover(&mut t, DVec3::new(4.0, 1.0, 0.0)), Some(group));
        assert_eq!(hover(&mut t, DVec3::new(-3.0, 6.0, 0.0)), None);
    }

    #[test]
    fn the_sampling_modifier_picks_up_the_material_clicked() {
        let mut s = scene();
        let (red, _) = s.add_material("Red", [255, 0, 0]).unwrap();
        let (blue, _) = s.add_material("Blue", [0, 0, 255]).unwrap();
        let EntityId::Face(face) = face_at(&s, 0.0) else {
            unreachable!()
        };
        s.paint(MeshId::Root, &[face], &[], Side::Back, Some(red))
            .unwrap();
        let group = s.root().instances().next().unwrap().0;
        s.paint(MeshId::Root, &[], &[group], Side::Front, Some(blue))
            .unwrap();
        let none = BTreeSet::new();
        let mut t = PaintTool::new();
        let above = Camera::new(DVec2::new(800.0, 600.0));
        let on_square = DVec3::new(1.0, 1.0, 0.0);
        let out = send(
            &mut t,
            &s,
            &above,
            on_square,
            true,
            &none,
            ToolEvent::Click(hit()),
        );
        assert_eq!(
            out.sampled,
            Some(None),
            "the front has the default material"
        );
        assert!(out.commands.is_empty());
        let mut below = Camera::new(DVec2::new(800.0, 600.0));
        below.eye = DVec3::new(-8.0, -12.0, -7.0);
        let out = send(
            &mut t,
            &s,
            &below,
            on_square,
            true,
            &none,
            ToolEvent::Click(hit()),
        );
        assert_eq!(out.sampled, Some(Some(red)));
        let on_group = DVec3::new(4.0, 1.0, 0.0);
        let out = send(
            &mut t,
            &s,
            &above,
            on_group,
            true,
            &none,
            ToolEvent::Click(hit()),
        );
        assert_eq!(out.sampled, Some(Some(blue)));
    }
}
