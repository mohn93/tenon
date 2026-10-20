//! The Follow Me tool: select the path edges, then click the profile face to sweep it along
//! them. With one face selected (and nothing else), clicking another face sweeps that face
//! around the selected face's outer loop (the lathe gesture). The command refuses what the
//! sweep cannot take; the editor shows its message.

use std::collections::BTreeSet;

use glam::DVec3;
use kernel::{EntityId, FaceId, Model};
use session::Command;

use crate::inference::face_under_in;
use crate::scenery::Owner;
use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};

const NO_PATH: &str = "Select the path edges, then click the profile face";

#[derive(Clone, Debug, Default)]
pub struct FollowMeTool {
    /// The face a click would sweep.
    hovered: Option<EntityId>,
}

impl FollowMeTool {
    pub fn new() -> Self {
        Self::default()
    }
}

/// The edges of a face's outer loop, in order.
fn outer_loop(model: &Model, face: FaceId) -> Option<Vec<EntityId>> {
    let outer = &model.face(face)?.outer;
    (0..outer.len())
        .map(|i| {
            model
                .edge_between(outer[i], outer[(i + 1) % outer.len()])
                .map(EntityId::Edge)
        })
        .collect()
}

/// The path the selection gives for a click on `profile`: the selected edges (only edges
/// selected), or the outer loop of the one other face selected (nothing else selected).
fn path(model: &Model, selection: &BTreeSet<EntityId>, profile: FaceId) -> Option<Vec<EntityId>> {
    if !selection.is_empty() && selection.iter().all(|e| matches!(e, EntityId::Edge(_))) {
        return Some(selection.iter().copied().collect());
    }
    match selection.iter().collect::<Vec<_>>()[..] {
        [EntityId::Face(f)] if *f != profile => outer_loop(model, *f),
        _ => None,
    }
}

fn face_under(ctx: &ToolContext) -> Option<FaceId> {
    match face_under_in(ctx.scenery, ctx.camera, ctx.cursor)? {
        (Owner::Open, f, _) => Some(f),
        _ => None,
    }
}

impl Tool for FollowMeTool {
    fn name(&self) -> &'static str {
        "Follow Me"
    }

    fn prompt(&self) -> &'static str {
        "Select the path edges first, then click the face to sweep along them"
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
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
            ToolEvent::Move(_) => self.hovered = face_under(ctx).map(EntityId::Face),
            ToolEvent::Click(_) => {
                self.hovered = None;
                let Some(profile) = face_under(ctx) else {
                    out.info = true;
                    out.message = Some("Click the profile face".into());
                    return out;
                };
                match path(ctx.model, ctx.selection, profile) {
                    Some(path) => out.commands.push(Command::FollowMe {
                        face: EntityId::Face(profile),
                        path,
                    }),
                    None => out.message = Some(NO_PATH.into()),
                }
            }
            ToolEvent::Value(_) => {
                out.message = Some("The Follow Me tool takes no typed values".into());
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
    use kernel::Scene;
    use session::Camera;

    fn hit() -> Inference {
        Inference {
            point: DVec3::ZERO,
            kind: SnapKind::Free,
            face: None,
        }
    }

    /// Two 2 x 2 squares on the ground, at x 0..2 and x 3..5.
    fn scene() -> Scene {
        let mut m = Model::new();
        for x in [0.0, 3.0] {
            m.add_polygon(&[
                DVec3::new(x, 0.0, 0.0),
                DVec3::new(x + 2.0, 0.0, 0.0),
                DVec3::new(x + 2.0, 2.0, 0.0),
                DVec3::new(x, 2.0, 0.0),
            ])
            .unwrap();
        }
        Scene::from_root(m)
    }

    fn click(scene: &Scene, selection: &BTreeSet<EntityId>, at: DVec3) -> ToolOutput {
        let camera = Camera::new(DVec2::new(800.0, 600.0));
        let scenery = Scenery::of(scene, &[]).unwrap();
        let ctx = ToolContext {
            model: scene.root(),
            scenery: &scenery,
            camera: &camera,
            cursor: camera.project(at).unwrap(),
            pick_radius: 10.0,
            drag_threshold: 4.0,
            quick_press: false,
            shift: false,
            selection,
            material: None,
            sample: false,
        };
        FollowMeTool::new().handle(ToolEvent::Click(hit()), &ctx)
    }

    fn face_at(scene: &Scene, x: f64) -> FaceId {
        let m = scene.root();
        m.faces()
            .find(|(_, f)| (m.position(f.outer[0]).unwrap().x - x).abs() < 0.1)
            .unwrap()
            .0
    }

    const LEFT: DVec3 = DVec3::new(1.0, 1.0, 0.0);
    const RIGHT: DVec3 = DVec3::new(4.0, 1.0, 0.0);

    #[test]
    fn selected_edges_are_the_path() {
        let s = scene();
        let edges: BTreeSet<EntityId> = s
            .root()
            .edges()
            .filter(|(_, e)| e.v.iter().all(|v| s.root().position(*v).unwrap().x >= 3.0))
            .map(|(e, _)| EntityId::Edge(e))
            .collect();
        assert_eq!(edges.len(), 4);
        let out = click(&s, &edges, LEFT);
        assert_eq!(
            out.commands,
            vec![Command::FollowMe {
                face: EntityId::Face(face_at(&s, 0.0)),
                path: edges.into_iter().collect(),
            }]
        );
    }

    #[test]
    fn a_selected_face_lends_its_outer_loop() {
        let s = scene();
        let lathe = face_at(&s, 3.0);
        let selection = BTreeSet::from([EntityId::Face(lathe)]);
        let out = click(&s, &selection, LEFT);
        let [Command::FollowMe { face, path }] = &out.commands[..] else {
            panic!("{:?}", out.commands);
        };
        assert_eq!(*face, EntityId::Face(face_at(&s, 0.0)));
        assert_eq!(path.len(), 4);
        assert!(path.iter().all(|e| matches!(e, EntityId::Edge(_))));
        // Clicking the selected face itself is no gesture.
        let out = click(&s, &selection, RIGHT);
        assert!(out.commands.is_empty());
        assert_eq!(out.message.as_deref(), Some(NO_PATH));
    }

    #[test]
    fn without_a_usable_selection_nothing_runs() {
        let s = scene();
        let face = EntityId::Face(face_at(&s, 3.0));
        let edge = s
            .root()
            .edges()
            .next()
            .map(|(e, _)| EntityId::Edge(e))
            .unwrap();
        for selection in [
            BTreeSet::new(),
            BTreeSet::from([face, edge]),
            BTreeSet::from([face, EntityId::Face(face_at(&s, 0.0))]),
        ] {
            let out = click(&s, &selection, LEFT);
            assert!(out.commands.is_empty(), "{selection:?}");
            assert_eq!(out.message.as_deref(), Some(NO_PATH));
            assert!(!out.info);
        }
    }
}
