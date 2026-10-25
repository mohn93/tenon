//! Cam followers: a prismatic joint whose value comes from where its axis line meets a cam's
//! surface, so a valve lifts as the lobe of a turning cam passes under it.
//!
//! The follower's contact point is its joint's frame origin on the moving part, which slides
//! along the joint's axis line. The cam's triangles (one tree per drive, in the cam
//! definition's coordinates, placed by the cam's pose) are cut by that line; the follower is
//! pushed along its preload direction, so it meets the first surface that way: the least hit
//! along the line when pushed along the axis, the greatest when pushed against it. A roller
//! of some radius stands that far back along the line (an approximation of the offset
//! surface that is exact where the cam's surface is square to the line).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use glam::DAffine3;

use crate::bvh::Bvh;
use crate::edit::KernelError;
use crate::ids::{CamFollowerId, DefinitionId, InstanceId, JointId};
use crate::joint::{JointFrame, JointKind};
use crate::scene::{Scene, SceneChange, Step};

/// How deep nested instances of a cam are taken into its surface.
const MAX_DEPTH: usize = 64;

/// A prismatic joint (`follower`) whose value the solver sets so that its frame origin on the
/// moving part, less `roller_radius` along the line, touches the surface of root instance
/// `cam` along the joint's axis. `preload` is 1 when the follower is pushed onto the cam along
/// the joint's axis (as seen from part `a`), -1 when against it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CamFollower {
    pub cam: InstanceId,
    pub follower: JointId,
    /// Meters, not negative; 0 for a flat or knife follower touching at its axis line.
    pub roller_radius: f64,
    pub preload: f64,
}

/// The cam trees built so far, by cam definition, each with the definitions it was gathered
/// from and their meshes' stamps then (see `Model::content_stamp`): a tree is used again while every
/// one of them stands as it did. Not part of the model: a copy of the scene starts with the
/// same trees, and equality leaves them out.
#[derive(Default)]
pub(crate) struct CamTrees(Mutex<BTreeMap<DefinitionId, CamTree>>);

/// A built cam tree and the (definition, stamp) of each mesh it was gathered from; no stamp
/// for a definition that was missing.
#[derive(Clone)]
struct CamTree {
    tree: Arc<Bvh>,
    from: Vec<(DefinitionId, Option<u64>)>,
}

impl Clone for CamTrees {
    fn clone(&self) -> Self {
        CamTrees(Mutex::new(self.lock().clone()))
    }
}

impl fmt::Debug for CamTrees {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CamTrees({})", self.lock().len())
    }
}

impl CamTrees {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<DefinitionId, CamTree>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The follower joint's value at which it touches the cam whose tree `tree` is placed by
/// `cam_place`, the joint's frame on part `a` standing at `line` in the world. None when the
/// axis line misses the cam.
pub(crate) fn contact(
    tree: &Bvh,
    cam_place: &DAffine3,
    line: &JointFrame,
    preload: f64,
    roller_radius: f64,
) -> Option<f64> {
    let (lo, hi) = tree.line_hits(cam_place, line.origin, line.axis)?;
    let touch = if preload > 0.0 { lo } else { hi };
    Some(touch - preload * roller_radius)
}

impl Scene {
    pub fn cam_follower(&self, id: CamFollowerId) -> Option<&CamFollower> {
        self.cam_followers.get(&id)
    }

    /// Every cam follower, in id (creation) order.
    pub fn cam_followers(&self) -> impl Iterator<Item = (CamFollowerId, &CamFollower)> {
        self.cam_followers.iter().map(|(k, v)| (*k, v))
    }

    /// The cam follower of joint `joint`, if it follows a cam.
    pub fn follower_of(&self, joint: JointId) -> Option<(CamFollowerId, &CamFollower)> {
        self.cam_followers().find(|(_, c)| c.follower == joint)
    }

    /// Adds `follower` after checking it (see [`Scene::cam_follower_problem`]) and that its
    /// axis line meets the cam now. Adding one never moves anything: the next drive of the
    /// cam's assembly meets it.
    pub fn add_cam_follower(
        &mut self,
        follower: CamFollower,
    ) -> Result<(CamFollowerId, SceneChange), KernelError> {
        self.cam_follower_problem(&follower, None)
            .map_err(KernelError::Joint)?;
        if self.cam_contact(&follower).is_none() {
            let name = &self.joints[&follower.follower].name;
            return Err(KernelError::Joint(format!(
                "joint {name:?}'s axis line does not meet cam i{}",
                follower.cam.0
            )));
        }
        let id = self.alloc_cam_follower();
        let step = Step::AddCamFollower(id, follower);
        self.apply_step(&step);
        Ok((id, step.into()))
    }

    /// Removes a cam follower; undo puts it back. The joint keeps its value and slides free.
    pub fn delete_cam_follower(&mut self, id: CamFollowerId) -> Result<SceneChange, KernelError> {
        let follower = *self
            .cam_follower(id)
            .ok_or(KernelError::CamFollowerNotFound)?;
        let step = Step::RemoveCamFollower(id, follower);
        self.apply_step(&step);
        Ok(step.into())
    }

    /// The value at which `follower`'s joint touches its cam with the parts as they stand;
    /// None when its axis line misses the cam (or the cam or joint is missing).
    pub fn cam_contact(&self, follower: &CamFollower) -> Option<f64> {
        let joint = self.joint(follower.follower)?;
        let cam = self.root().instance(follower.cam)?;
        let (line, _) = self.joint_world_frames(joint);
        contact(
            &self.cam_tree(cam.definition),
            &cam.transform,
            &line,
            follower.preload,
            follower.roller_radius,
        )
    }

    /// The preload toward the cam for joint `joint`: 1 when the center of the cam's bounding
    /// box lies along the joint's axis from its frame origin on the moving part, -1 when
    /// against it; None when the center lies square to the axis (or the cam or joint is
    /// missing, or the cam has no faces).
    pub fn preload_toward(&self, cam: InstanceId, joint: JointId) -> Option<f64> {
        let joint = self.joint(joint)?;
        let cam = self.root().instance(cam)?;
        let (lo, hi) = self
            .cam_tree(cam.definition)
            .placed_bounds(&cam.transform)?;
        let (a, b) = self.joint_world_frames(joint);
        let along = ((lo + hi) / 2.0 - b.origin).dot(a.axis);
        let size = (hi - lo).length().max(1e-300);
        (along.abs() > 1e-9 * size).then(|| along.signum())
    }

    /// The triangles of definition `d` and the instances nested in it at any depth, in `d`'s
    /// coordinates: the tree built before while the meshes it came from are unchanged.
    pub(crate) fn cam_tree(&self, d: DefinitionId) -> Arc<Bvh> {
        let fresh = |t: &CamTree| {
            t.from
                .iter()
                .all(|(m, stamp)| self.definition(*m).map(|x| x.mesh.content_stamp()) == *stamp)
        };
        if let Some(t) = self.cam_trees.lock().get(&d).filter(|t| fresh(t)) {
            return t.tree.clone();
        }
        let mut tris = Vec::new();
        let mut from = Vec::new();
        self.gather_triangles(d, DAffine3::IDENTITY, &mut tris, 0, &mut from);
        let tree = Arc::new(Bvh::new(tris));
        let mut trees = self.cam_trees.lock();
        trees.retain(|k, _| self.definition(*k).is_some());
        trees.insert(
            d,
            CamTree {
                tree: tree.clone(),
                from,
            },
        );
        tree
    }

    fn gather_triangles(
        &self,
        d: DefinitionId,
        place: DAffine3,
        out: &mut Vec<[glam::DVec3; 3]>,
        depth: usize,
        from: &mut Vec<(DefinitionId, Option<u64>)>,
    ) {
        let def = self.definition(d);
        from.push((d, def.map(|x| x.mesh.content_stamp())));
        let Some(def) = def else {
            return;
        };
        if depth > MAX_DEPTH {
            return;
        }
        for (f, _) in def.mesh.faces() {
            for t in def.mesh.triangulate(f).unwrap_or_default() {
                out.push(t.map(|p| place.transform_point3(p)));
            }
        }
        for (_, child) in def.mesh.instances() {
            self.gather_triangles(
                child.definition,
                place * child.transform,
                out,
                depth + 1,
                from,
            );
        }
    }

    /// Why `follower` is not valid, if it is not: a roller radius that is not a finite number
    /// at least 0, a preload other than 1 or -1, a cam that is not an instance of the root, a
    /// joint that does not exist, is not prismatic, has the cam as a part, is coupled, or
    /// already follows a cam (but `except`).
    pub(crate) fn cam_follower_problem(
        &self,
        follower: &CamFollower,
        except: Option<CamFollowerId>,
    ) -> Result<(), String> {
        if !follower.roller_radius.is_finite() || follower.roller_radius < 0.0 {
            return Err("a roller radius must be a finite number, at least 0".into());
        }
        if follower.preload != 1.0 && follower.preload != -1.0 {
            return Err("a cam follower's preload must be 1 or -1 along its axis".into());
        }
        let cam = follower.cam;
        if self.root().instance(cam).is_none() {
            return Err(format!(
                "the cam (i{}) is not an instance of the root",
                cam.0
            ));
        }
        let id = follower.follower;
        let joint = self
            .joint(id)
            .ok_or_else(|| format!("the follower joint {id} does not exist"))?;
        if joint.kind != JointKind::Prismatic {
            return Err(format!(
                "the follower joint {id} must be prismatic: it slides along its axis"
            ));
        }
        if joint.b == cam || joint.a == Some(cam) {
            return Err(format!(
                "the cam (i{}) is one of the follower joint {id}'s parts",
                cam.0
            ));
        }
        if let Some((k, _)) = self
            .couplings()
            .find(|(_, c)| c.driver == id || c.driven == id)
        {
            return Err(format!(
                "the follower joint {id} is coupled by {k}: its value comes from the cam"
            ));
        }
        if let Some((other, _)) = self
            .cam_followers()
            .find(|(w, c)| Some(*w) != except && c.follower == id)
        {
            return Err(format!("joint {id} already follows a cam ({other})"));
        }
        Ok(())
    }

    /// The steps that remove every cam follower whose joint is in `joints` or whose cam is no
    /// longer an instance of the root, applied.
    pub(crate) fn drop_cam_followers_of(&mut self, joints: &BTreeSet<JointId>) -> SceneChange {
        let gone: Vec<(CamFollowerId, CamFollower)> = self
            .cam_followers()
            .filter(|(_, c)| joints.contains(&c.follower) || self.root().instance(c.cam).is_none())
            .map(|(id, c)| (id, *c))
            .collect();
        let mut steps = Vec::new();
        for (id, c) in gone {
            let step = Step::RemoveCamFollower(id, c);
            self.apply_step(&step);
            steps.push(step);
        }
        SceneChange { steps }
    }
}
