//! Interference and clearance between parts: which solids overlap and by how much, and how
//! near parts come.
//!
//! A part is an instance of one mesh (the open context) with everything it places: its own
//! geometry and that of its nested instances, each a shell (one definition's closed mesh
//! placed by a transform). Each definition gets one triangle tree ([`Bvh`]), built once and
//! placed by transforms, so a checker kept across poses (a sweep moving instances) reuses it.
//!
//! Two shells meet when their surfaces come within `LENGTH_TOL` (triangle against triangle,
//! pruned by the trees) or one lies inside the other (ray parity from one of its vertices).
//! Shells that meet are intersected by the solid boolean on copies, never in the scene: a
//! volume above `MIN_VOLUME` is interference, anything less (shared faces or edges) is
//! touching.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use glam::{DAffine3, DVec3};

use crate::boolean::{BooleanOp, combine, interior_point, placed_copy, point_inside};
use crate::bvh::{Bvh, Closest};
use crate::edit::KernelError;
use crate::geom::LENGTH_TOL;
use crate::ids::{DefinitionId, InstanceId, JointId};
use crate::index::ray_hits_triangle;
use crate::model::Model;
use crate::parallel::each_in_parallel;
use crate::scene::{MeshId, Scene};
use crate::solid::{open_reason, signed_volume};
use crate::solver::CamTrees;

/// Overlaps of at most this many cubic meters are touching, not interference.
pub const MIN_VOLUME: f64 = 1e-12;

/// The fewest and most steps of a sweep.
pub const MIN_SWEEP_STEPS: usize = 2;
pub const MAX_SWEEP_STEPS: usize = 10_000;

/// A sweep reports the pairs that come within this many meters (by default).
pub const SWEEP_REPORT_WITHIN: f64 = 0.005;

/// A sweep stops once it has run this many boolean volume computations.
pub const MAX_SWEEP_BOOLEANS: usize = 20_000;

/// Why a sweep stopped for running too many booleans.
pub const SWEEP_TOO_COSTLY: &str =
    "sweep stopped: too many overlapping steps; use fewer steps or ids";

/// One definition's closed mesh placed into the coordinates of the parts' mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shell {
    pub definition: DefinitionId,
    pub place: DAffine3,
}

/// An instance and every shell it places.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub id: InstanceId,
    pub shells: Vec<Shell>,
}

/// How two parts lie against each other.
#[derive(Clone, Debug, PartialEq)]
pub enum Contact {
    Apart,
    /// Their surfaces meet (at `point`) with no volume in common.
    Touching {
        point: DVec3,
    },
    /// They share `volume` cubic meters; `point` lies inside what they share.
    Overlap {
        volume: f64,
        point: DVec3,
    },
    /// The boolean that measures the overlap failed; the text says why.
    Failed(String),
    /// Some shell pairs share `volume` (a lower bound) but another pair's boolean failed.
    Partial {
        volume: f64,
        point: DVec3,
        why: String,
    },
}

/// Two overlapping parts.
#[derive(Clone, Debug, PartialEq)]
pub struct Overlap {
    pub a: InstanceId,
    pub b: InstanceId,
    pub volume: f64,
    pub point: DVec3,
}

/// Two touching parts.
#[derive(Clone, Debug, PartialEq)]
pub struct Touch {
    pub a: InstanceId,
    pub b: InstanceId,
    pub point: DVec3,
}

/// Every overlapping pair, largest volume first; touching pairs; pairs whose overlap could
/// not be measured, with why.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InterferenceReport {
    pub overlaps: Vec<Overlap>,
    pub touching: Vec<Touch>,
    pub failed: Vec<(InstanceId, InstanceId, String)>,
}

/// The nearest points of two parts' surfaces. `distance` is 0 when they touch or overlap
/// (`overlap` then says which); a part inside another is 0 apart with both points at a
/// vertex of the inner one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clearance {
    pub distance: f64,
    pub a: DVec3,
    pub b: DVec3,
    pub overlap: bool,
}

/// One pair of parts over a sweep, joint values in radians or meters.
#[derive(Clone, Debug, PartialEq)]
pub struct SweepPair {
    pub a: InstanceId,
    pub b: InstanceId,
    /// The first and last values where they overlap; None when they never do.
    pub interferes: Option<(f64, f64)>,
    /// The largest volume they share (0 when they never overlap), and where.
    pub volume: f64,
    pub volume_at: Option<f64>,
    /// The smallest clearance (0 when they touch or overlap) and the first value where it
    /// occurs.
    pub clearance: f64,
    pub clearance_at: f64,
    /// The first value where their overlap could not be measured, and why.
    pub failed: Option<(f64, String)>,
}

/// What a sweep found: every pair that came within the report distance at some step, in
/// order of (a, b); the parts left out as not solid; how many steps were checked; and the
/// value the solver could not meet, with why, when it stopped the sweep there.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SweepReport {
    pub pairs: Vec<SweepPair>,
    pub skipped: Vec<(InstanceId, String)>,
    pub steps: usize,
    pub stopped: Option<(f64, String)>,
}

/// How two parts stand at one step of a sweep.
#[derive(Clone, Debug)]
enum Probe {
    /// Their surfaces are this far apart (0 when touching), within the report distance.
    Near(f64),
    Overlap(f64),
    Failed(String),
    /// A volume found, and a pair whose boolean failed.
    Partial(f64, String),
}

/// Triangle trees and solid checks per definition, kept across queries and poses. The
/// definitions' geometry must not change while it is kept (moving instances is fine).
///
/// The pairs of an interference check or a sweep step are checked on a few threads (where
/// threads can be started); each pair's result is the same either way.
#[derive(Debug, Default)]
pub struct Checker {
    trees: HashMap<DefinitionId, Bvh>,
    solid: HashMap<DefinitionId, Result<(), String>>,
    /// How many boolean volume computations ran.
    booleans: AtomicUsize,
    /// The last probe of each pair of a sweep, with the shells it was made with: a pair
    /// neither of whose parts moved since (two parts the joint does not move) stands as it
    /// did, so a sweep checks it once.
    probes: HashMap<(InstanceId, InstanceId), KeptProbe>,
    /// Placed copies of shells for the overlap booleans, with their face indexes built on
    /// first use: a shell that meets several others (a block against every part fitted into
    /// it) or stands still over a sweep is copied once. Each check or sweep step is a pose;
    /// copies not used at a pose are dropped at the next.
    copies: Mutex<Copies>,
}

/// Placed copies of shells: those used at this pose and those of the pose before.
#[derive(Debug, Default)]
struct Copies {
    now: HashMap<CopyKey, Arc<Model>>,
    before: HashMap<CopyKey, Arc<Model>>,
}

/// A shell's definition and the bits of its placement.
type CopyKey = (DefinitionId, [u64; 12]);

/// A pair's probe and the shells of its two parts when it was made.
type KeptProbe = (Vec<Shell>, Vec<Shell>, Option<Probe>);

impl Checker {
    pub fn new() -> Checker {
        Checker::default()
    }

    /// The parts for instances `ids` of mesh `parent` (every instance when empty), in
    /// order, and the ones that are not solid parts with why.
    pub fn parts(
        &mut self,
        scene: &Scene,
        parent: MeshId,
        ids: &[InstanceId],
    ) -> (Vec<Part>, Vec<(InstanceId, String)>) {
        let Some(mesh) = scene.mesh(parent) else {
            return (Vec::new(), Vec::new());
        };
        let ids: Vec<InstanceId> = if ids.is_empty() {
            mesh.instances().map(|(id, _)| id).collect()
        } else {
            ids.to_vec()
        };
        self.prepare(
            scene,
            ids.iter()
                .filter_map(|id| Some(mesh.instance(*id)?.definition))
                .collect(),
        );
        let (mut parts, mut skipped) = (Vec::new(), Vec::new());
        for id in ids {
            match self.part(scene, parent, id) {
                Ok(p) => parts.push(p),
                Err(why) => skipped.push((id, why)),
            }
        }
        (parts, skipped)
    }

    /// Instance `id` of `parent` as a part, or why it is not a solid part: its own geometry,
    /// when it has faces, and every nested instance at any depth must be closed and enclose
    /// a volume, and there must be at least one such shell.
    pub fn part(&mut self, scene: &Scene, parent: MeshId, id: InstanceId) -> Result<Part, String> {
        let instance = scene
            .mesh(parent)
            .and_then(|m| m.instance(id))
            .ok_or_else(|| "it is missing".to_string())?;
        let mut shells = Vec::new();
        self.gather(
            scene,
            instance.definition,
            instance.transform,
            None,
            &mut shells,
            0,
        )?;
        if shells.is_empty() {
            return Err("it has no faces".into());
        }
        Ok(Part { id, shells })
    }

    fn gather(
        &mut self,
        scene: &Scene,
        definition: DefinitionId,
        place: DAffine3,
        nested: Option<InstanceId>,
        out: &mut Vec<Shell>,
        depth: usize,
    ) -> Result<(), String> {
        let inside = |why: String| match nested {
            Some(i) => format!("i{} inside it: {why}", i.0),
            None => why,
        };
        if depth > 64 {
            return Err(inside("it is nested too deeply".into()));
        }
        let d = scene
            .definition(definition)
            .ok_or_else(|| inside("its definition is missing".into()))?;
        if d.mesh.faces().next().is_some() {
            self.solid_check(scene, definition).map_err(inside)?;
            out.push(Shell { definition, place });
        }
        for (i, child) in d.mesh.instances() {
            self.gather(
                scene,
                child.definition,
                place * child.transform,
                nested.or(Some(i)),
                out,
                depth + 1,
            )?;
        }
        Ok(())
    }

    fn solid_check(&mut self, scene: &Scene, definition: DefinitionId) -> Result<(), String> {
        self.solid
            .entry(definition)
            .or_insert_with(|| solid_or_why(&scene.definition(definition).expect("exists").mesh))
            .clone()
    }

    /// Checks every definition with faces placed by `roots` (definitions), at any depth, and
    /// builds its tree, those not done before, all at once on a few threads.
    fn prepare(&mut self, scene: &Scene, mut roots: Vec<DefinitionId>) {
        let mut seen = std::collections::BTreeSet::new();
        let mut todo = Vec::new();
        while let Some(d) = roots.pop() {
            let Some(def) = scene.definition(d) else {
                continue;
            };
            if !seen.insert(d) {
                continue;
            }
            roots.extend(def.mesh.instances().map(|(_, i)| i.definition));
            let fresh = !self.solid.contains_key(&d) || !self.trees.contains_key(&d);
            if fresh && def.mesh.faces().next().is_some() {
                todo.push(d);
            }
        }
        let done = each_in_parallel(todo.len(), |k| {
            let d = todo[k];
            (
                solid_or_why(&scene.definition(d).expect("found").mesh),
                tree_of(scene, d),
            )
        });
        for (d, (solid, tree)) in todo.into_iter().zip(done) {
            self.solid.entry(d).or_insert(solid);
            self.trees.entry(d).or_insert(tree);
        }
    }

    /// The tree of `definition`, built on first use.
    pub fn tree(&mut self, scene: &Scene, definition: DefinitionId) -> &Bvh {
        self.trees
            .entry(definition)
            .or_insert_with(|| tree_of(scene, definition))
    }

    /// How many boolean volume computations ran.
    pub fn booleans(&self) -> usize {
        self.booleans.load(Ordering::Relaxed)
    }

    /// The tree of `definition`, which must have been built.
    fn built(&self, definition: DefinitionId) -> &Bvh {
        self.trees.get(&definition).expect("trees are built first")
    }

    /// Builds the trees of every shell of `parts`.
    fn build_trees(&mut self, scene: &Scene, parts: &[&Part]) {
        for s in parts.iter().flat_map(|p| &p.shells) {
            self.tree(scene, s.definition);
        }
    }

    /// Shell `s` copied to where it is placed, made once per pose. Two threads asking for
    /// the same new copy may both make it; either is kept.
    fn copy(&self, scene: &Scene, s: &Shell) -> Arc<Model> {
        let key = (s.definition, s.place.to_cols_array().map(f64::to_bits));
        {
            let mut copies = self.copies.lock().expect("copies lock");
            if let Some(m) = copies.now.get(&key) {
                return m.clone();
            }
            if let Some(m) = copies.before.remove(&key) {
                copies.now.insert(key, m.clone());
                return m;
            }
        }
        let m = Arc::new(placed_copy(mesh_of(scene, s), s.place, None));
        let mut copies = self.copies.lock().expect("copies lock");
        copies.now.entry(key).or_insert(m).clone()
    }

    /// Moves on to a new pose: copies not used at the pose before are dropped.
    fn next_pose(&mut self) {
        let copies = self.copies.get_mut().expect("copies lock");
        copies.before = std::mem::take(&mut copies.now);
    }

    /// The box around `part` in the parts' coordinates.
    pub fn bounds(&mut self, scene: &Scene, part: &Part) -> (DVec3, DVec3) {
        let mut lo = DVec3::splat(f64::INFINITY);
        let mut hi = DVec3::splat(f64::NEG_INFINITY);
        for s in &part.shells {
            if let Some((l, h)) = self.tree(scene, s.definition).placed_bounds(&s.place) {
                lo = lo.min(l);
                hi = hi.max(h);
            }
        }
        (lo, hi)
    }

    /// How `a` and `b` lie against each other.
    pub fn contact(&mut self, scene: &Scene, a: &Part, b: &Part) -> Contact {
        self.next_pose();
        self.build_trees(scene, &[a, b]);
        self.contact_built(scene, a, b, true)
    }

    /// [`Checker::contact`] once the parts' trees are built; with `point` false, an overlap's
    /// point is any point where the shells meet (a sweep has no use for it).
    fn contact_built(&self, scene: &Scene, a: &Part, b: &Part, point: bool) -> Contact {
        let mut volume = 0.0;
        let mut deepest: Option<(f64, DVec3)> = None;
        let mut touch = None;
        let mut failure: Option<String> = None;
        for sa in &a.shells {
            for sb in &b.shells {
                let Some(meet) = self.shells_meet(scene, sa, sb) else {
                    continue;
                };
                let (ma, mb) = (self.copy(scene, sa), self.copy(scene, sb));
                self.booleans.fetch_add(1, Ordering::Relaxed);
                match combine(&ma, &mb, BooleanOp::Intersect) {
                    Ok(Some(m)) => {
                        let v = signed_volume(&m);
                        if v > MIN_VOLUME {
                            volume += v;
                            if deepest.is_none_or(|(w, _)| v > w) {
                                let p = if point {
                                    inside_point(&m).unwrap_or(meet)
                                } else {
                                    meet
                                };
                                deepest = Some((v, p));
                            }
                        } else {
                            touch.get_or_insert(meet);
                        }
                    }
                    Ok(None) => {
                        touch.get_or_insert(meet);
                    }
                    Err(e) => {
                        failure.get_or_insert(e.to_string());
                    }
                }
            }
        }
        match (deepest, touch) {
            (Some((_, point)), _) if volume > MIN_VOLUME => match failure {
                Some(why) => Contact::Partial { volume, point, why },
                None => Contact::Overlap { volume, point },
            },
            _ if failure.is_some() => Contact::Failed(failure.unwrap_or_default()),
            (_, Some(point)) => Contact::Touching { point },
            _ => Contact::Apart,
        }
    }

    /// Where shells `sa` and `sb` meet: a point where their surfaces come within
    /// `LENGTH_TOL`, or a vertex of one lying inside the other. None when they lie apart.
    fn shells_meet(&self, scene: &Scene, sa: &Shell, sb: &Shell) -> Option<DVec3> {
        let (ba, bb) = (
            self.built(sa.definition).placed_bounds(&sa.place)?,
            self.built(sb.definition).placed_bounds(&sb.place)?,
        );
        if !boxes_meet(ba, bb, LENGTH_TOL) {
            return None;
        }
        if let Some(c) = self.shell_distance(sa, sb, 2.0 * LENGTH_TOL, LENGTH_TOL)
            && c.distance <= LENGTH_TOL
        {
            return Some(c.a);
        }
        self.contained(scene, sa, sb)
            .or_else(|| self.contained(scene, sb, sa))
    }

    /// The first of `inner`'s vertices (up to `tries`) told inside or outside `outer`: its
    /// placed position and whether it is inside. None when the boxes do not meet or no
    /// vertex tried gave an answer.
    fn locate(
        &self,
        scene: &Scene,
        inner: &Shell,
        outer: &Shell,
        tries: usize,
    ) -> Option<(DVec3, bool)> {
        let (bi, bo) = (
            self.built(inner.definition).placed_bounds(&inner.place)?,
            self.built(outer.definition).placed_bounds(&outer.place)?,
        );
        if !boxes_meet(bi, bo, 0.0) {
            return None;
        }
        let inner_mesh = &scene.definition(inner.definition)?.mesh;
        let outer_mesh = &scene.definition(outer.definition)?.mesh;
        let back = outer.place.inverse();
        inner_mesh.vertices().take(tries).find_map(|(_, v)| {
            let p = inner.place.transform_point3(v.pos);
            point_inside(back.transform_point3(p), outer_mesh).map(|inside| (p, inside))
        })
    }

    /// A vertex of `inner` when it lies inside `outer` (their surfaces apart).
    fn contained(&self, scene: &Scene, inner: &Shell, outer: &Shell) -> Option<DVec3> {
        self.locate(scene, inner, outer, 8)
            .and_then(|(p, inside)| inside.then_some(p))
    }

    fn shell_distance(&self, sa: &Shell, sb: &Shell, bound: f64, stop: f64) -> Option<Closest> {
        let (ta, tb) = (self.built(sa.definition), self.built(sb.definition));
        ta.distance(&sa.place, tb, &sb.place, bound, stop)
    }

    /// Every overlapping and touching pair among `parts` (pairs whose boxes meet).
    pub fn interference(&mut self, scene: &Scene, parts: &[Part]) -> InterferenceReport {
        self.next_pose();
        let boxes: Vec<(DVec3, DVec3)> = parts.iter().map(|p| self.bounds(scene, p)).collect();
        let mut order: Vec<usize> = (0..parts.len()).collect();
        order.sort_by(|i, j| boxes[*i].0.x.total_cmp(&boxes[*j].0.x));
        let mut pairs = Vec::new();
        for (k, &i) in order.iter().enumerate() {
            for &j in &order[k + 1..] {
                if boxes[j].0.x > boxes[i].1.x + LENGTH_TOL {
                    break;
                }
                if !boxes_meet(boxes[i], boxes[j], LENGTH_TOL) {
                    continue;
                }
                pairs.push(if parts[i].id <= parts[j].id {
                    (&parts[i], &parts[j])
                } else {
                    (&parts[j], &parts[i])
                });
            }
        }
        let mut costly: Vec<usize> = (0..pairs.len()).collect();
        costly.sort_by_key(|k| Reverse(cost(pairs[*k].0, pairs[*k].1)));
        let this = &*self;
        let mut contacts: Vec<(usize, Contact)> = each_in_parallel(costly.len(), |k| {
            let (a, b) = pairs[costly[k]];
            (costly[k], this.contact_built(scene, a, b, true))
        });
        contacts.sort_by_key(|(k, _)| *k);
        let mut report = InterferenceReport::default();
        for (&(a, b), (_, contact)) in pairs.iter().zip(contacts) {
            match contact {
                Contact::Apart => {}
                Contact::Touching { point } => report.touching.push(Touch {
                    a: a.id,
                    b: b.id,
                    point,
                }),
                Contact::Overlap { volume, point } => report.overlaps.push(Overlap {
                    a: a.id,
                    b: b.id,
                    volume,
                    point,
                }),
                Contact::Failed(why) => report.failed.push((a.id, b.id, why)),
                Contact::Partial { volume, point, why } => {
                    report.overlaps.push(Overlap {
                        a: a.id,
                        b: b.id,
                        volume,
                        point,
                    });
                    report.failed.push((a.id, b.id, why));
                }
            }
        }
        report.overlaps.sort_by(|x, y| {
            y.volume
                .total_cmp(&x.volume)
                .then((x.a, x.b).cmp(&(y.a, y.b)))
        });
        report.touching.sort_by_key(|t| (t.a, t.b));
        report.failed.sort_by_key(|f| (f.0, f.1));
        report
    }

    /// The nearest points of `a` and `b` (see [`Clearance`]); None when a part has no
    /// triangles.
    pub fn clearance(&mut self, scene: &Scene, a: &Part, b: &Part) -> Option<Clearance> {
        self.next_pose();
        self.build_trees(scene, &[a, b]);
        let mut best: Option<Closest> = None;
        for sa in &a.shells {
            for sb in &b.shells {
                let bound = best.map_or(f64::INFINITY, |c| c.distance);
                if let Some(c) = self.shell_distance(sa, sb, bound, 0.0) {
                    best = Some(c);
                }
            }
        }
        let best = best?;
        if best.distance <= LENGTH_TOL {
            let overlap = matches!(
                self.contact_built(scene, a, b, true),
                Contact::Overlap { .. } | Contact::Partial { .. }
            );
            return Some(Clearance {
                distance: 0.0,
                a: best.a,
                b: best.b,
                overlap,
            });
        }
        for sa in &a.shells {
            for sb in &b.shells {
                if let Some(p) = self
                    .contained(scene, sa, sb)
                    .or_else(|| self.contained(scene, sb, sa))
                {
                    return Some(Clearance {
                        distance: 0.0,
                        a: p,
                        b: p,
                        overlap: true,
                    });
                }
            }
        }
        Some(Clearance {
            distance: best.distance,
            a: best.a,
            b: best.b,
            overlap: false,
        })
    }
}

impl Checker {
    /// How `a` and `b` stand, or None when they are farther apart than `within`. The overlap
    /// boolean runs only when their surfaces cross or one may lie inside the other; touching
    /// and near pairs are told apart by distance alone (a pin turning in its bore costs no
    /// boolean).
    fn probe(&self, scene: &Scene, a: &Part, b: &Part, within: f64) -> Option<Probe> {
        let reach = within + LENGTH_TOL;
        let mut best: Option<f64> = None;
        let mut measure = false;
        for sa in &a.shells {
            for sb in &b.shells {
                let (Some(ba), Some(bb)) = (
                    self.built(sa.definition).placed_bounds(&sa.place),
                    self.built(sb.definition).placed_bounds(&sb.place),
                ) else {
                    continue;
                };
                if !boxes_meet(ba, bb, reach) {
                    continue;
                }
                if !measure {
                    measure = self.shells_cross(sa, sb)
                        || self.encloses(scene, sa, sb) != Some(false)
                        || self.encloses(scene, sb, sa) != Some(false);
                }
                if best.is_none_or(|d| d > LENGTH_TOL) {
                    let bound = best.map_or(reach, |d| d.min(reach));
                    if let Some(c) = self.shell_distance(sa, sb, bound, LENGTH_TOL) {
                        best = Some(c.distance);
                    }
                }
            }
        }
        if measure {
            match self.contact_built(scene, a, b, false) {
                Contact::Overlap { volume, .. } => return Some(Probe::Overlap(volume)),
                Contact::Touching { .. } => return Some(Probe::Near(0.0)),
                Contact::Failed(why) => return Some(Probe::Failed(why)),
                Contact::Partial { volume, why, .. } => return Some(Probe::Partial(volume, why)),
                Contact::Apart => {}
            }
        }
        let d = best.filter(|d| *d <= within)?;
        Some(Probe::Near(if d <= LENGTH_TOL { 0.0 } else { d }))
    }

    /// Whether the surfaces of shells `sa` and `sb` cross (see [`Bvh::crosses`]).
    fn shells_cross(&self, sa: &Shell, sb: &Shell) -> bool {
        let (ta, tb) = (self.built(sa.definition), self.built(sb.definition));
        ta.crosses(&sa.place, tb, &sb.place, LENGTH_TOL)
    }

    /// Whether shell `inner` lies inside `outer`, their surfaces not crossing: told by the
    /// first of its vertices that is not on `outer`'s surface; None when none tried is.
    fn encloses(&self, scene: &Scene, inner: &Shell, outer: &Shell) -> Option<bool> {
        let bounds = |c: &Self, s: &Shell| c.built(s.definition).placed_bounds(&s.place);
        let (Some(bi), Some(bo)) = (bounds(self, inner), bounds(self, outer)) else {
            return Some(false);
        };
        if !boxes_meet(bi, bo, 0.0) {
            return Some(false);
        }
        self.locate(scene, inner, outer, 32)
            .map(|(_, inside)| inside)
    }

    /// Checks the pairs among `parts` whose boxes come within `within` at joint value
    /// `value`, recording each in `found`.
    fn sweep_step(
        &mut self,
        scene: &Scene,
        parts: &[Part],
        within: f64,
        value: f64,
        found: &mut BTreeMap<(InstanceId, InstanceId), SweepPair>,
    ) {
        let boxes: Vec<(DVec3, DVec3)> = parts.iter().map(|p| self.bounds(scene, p)).collect();
        let mut order: Vec<usize> = (0..parts.len()).collect();
        order.sort_by(|i, j| boxes[*i].0.x.total_cmp(&boxes[*j].0.x));
        let reach = within + LENGTH_TOL;
        // The pairs in reach, each with its kept probe when neither part moved since.
        let mut pairs = Vec::new();
        for (k, &i) in order.iter().enumerate() {
            for &j in &order[k + 1..] {
                if boxes[j].0.x > boxes[i].1.x + reach {
                    break;
                }
                if !boxes_meet(boxes[i], boxes[j], reach) {
                    continue;
                }
                let (a, b) = if parts[i].id <= parts[j].id {
                    (&parts[i], &parts[j])
                } else {
                    (&parts[j], &parts[i])
                };
                let kept = self
                    .probes
                    .get(&(a.id, b.id))
                    .filter(|(sa, sb, _)| *sa == a.shells && *sb == b.shells)
                    .map(|(_, _, probe)| probe.clone());
                pairs.push((a, b, kept));
            }
        }
        let mut fresh: Vec<usize> = (0..pairs.len()).filter(|k| pairs[*k].2.is_none()).collect();
        fresh.sort_by_key(|k| Reverse(cost(pairs[*k].0, pairs[*k].1)));
        let this = &*self;
        let probes = each_in_parallel(fresh.len(), |k| {
            let (a, b, _) = pairs[fresh[k]];
            this.probe(scene, a, b, within)
        });
        for (k, probe) in fresh.into_iter().zip(probes) {
            let (a, b, _) = pairs[k];
            self.probes.insert(
                (a.id, b.id),
                (a.shells.clone(), b.shells.clone(), probe.clone()),
            );
            pairs[k].2 = Some(probe);
        }
        for (a, b, probe) in pairs {
            let Some(probe) = probe.expect("every pair probed") else {
                continue;
            };
            let e = found.entry((a.id, b.id)).or_insert(SweepPair {
                a: a.id,
                b: b.id,
                interferes: None,
                volume: 0.0,
                volume_at: None,
                clearance: f64::INFINITY,
                clearance_at: value,
                failed: None,
            });
            let distance = match probe {
                Probe::Near(d) => d,
                Probe::Overlap(v) => {
                    let first = e.interferes.map_or(value, |(f, _)| f);
                    e.interferes = Some((first, value));
                    if v > e.volume {
                        e.volume = v;
                        e.volume_at = Some(value);
                    }
                    0.0
                }
                Probe::Failed(why) => {
                    e.failed.get_or_insert((value, why));
                    0.0
                }
                Probe::Partial(v, why) => {
                    let first = e.interferes.map_or(value, |(f, _)| f);
                    e.interferes = Some((first, value));
                    if v > e.volume {
                        e.volume = v;
                        e.volume_at = Some(value);
                    }
                    e.failed.get_or_insert((value, why));
                    0.0
                }
            };
            if distance < e.clearance {
                e.clearance = distance;
                e.clearance_at = value;
            }
        }
    }
}

/// Drives `joint` of a copy of `scene` from `from` to `to` (radians or meters) in `steps`
/// evenly spaced values, the ends included (`slide` held, for a cylindrical joint), and at
/// each checks every pair of the root instances `ids` (every one when empty) whose boxes
/// come within `within` meters. Pairs that never come within `within` are left out. A value
/// the solver cannot meet stops the sweep there. The scene is never changed. The triangle
/// trees are built once per definition and placed anew at each step; a pair neither of whose
/// parts moved since the step before (two parts the joint does not move) is checked once and
/// stands as it did. A step's pairs are checked on a few threads while the solver drives the
/// copy on to the next steps, where threads can be started; the report is the same either way.
///
/// A sweep that has run more than [`MAX_SWEEP_BOOLEANS`] boolean volume computations stops
/// after the step that passed it, with [`SWEEP_TOO_COSTLY`] as the reason.
#[allow(clippy::too_many_arguments)]
pub fn sweep_check(
    scene: &Scene,
    joint: JointId,
    from: f64,
    to: f64,
    steps: usize,
    slide: Option<f64>,
    ids: &[InstanceId],
    within: f64,
) -> Result<SweepReport, KernelError> {
    sweep_check_budget(
        scene,
        joint,
        from,
        to,
        steps,
        slide,
        ids,
        within,
        MAX_SWEEP_BOOLEANS,
    )
}

/// [`sweep_check`] with its own limit on boolean volume computations.
#[allow(clippy::too_many_arguments)]
pub fn sweep_check_budget(
    scene: &Scene,
    joint: JointId,
    from: f64,
    to: f64,
    steps: usize,
    slide: Option<f64>,
    ids: &[InstanceId],
    within: f64,
    budget: usize,
) -> Result<SweepReport, KernelError> {
    let refuse = |why: String| Err(KernelError::Joint(why));
    let kind = scene.joint(joint).ok_or(KernelError::JointNotFound)?.kind;
    if !kind.has_value() {
        return refuse("a fixed or ball joint cannot be driven".into());
    }
    if slide.is_some() && kind != crate::joint::JointKind::Cylindrical {
        return refuse("only a cylindrical joint slides".into());
    }
    if !from.is_finite() || !to.is_finite() || slide.is_some_and(|s| !s.is_finite()) {
        return refuse("from, to and slide must be finite numbers".into());
    }
    if !(MIN_SWEEP_STEPS..=MAX_SWEEP_STEPS).contains(&steps) {
        return refuse(format!(
            "steps must be from {MIN_SWEEP_STEPS} to {MAX_SWEEP_STEPS}"
        ));
    }
    if !within.is_finite() || within < 0.0 {
        return refuse("report_within must be a distance of 0 or more".into());
    }
    let mut checker = Checker::new();
    let (parts, skipped) = checker.parts(scene, MeshId::Root, ids);
    let solid: Vec<InstanceId> = parts.iter().map(|p| p.id).collect();
    let mut report = SweepReport {
        skipped,
        ..SweepReport::default()
    };
    let values: Vec<f64> = (0..steps)
        .map(|k| {
            if k + 1 == steps {
                to
            } else {
                from + (to - from) * (k as f64 / (steps - 1) as f64)
            }
        })
        .collect();
    let solid_checks = checker.solid.clone();
    let poses = || Poses {
        scene: scene.clone(),
        joint,
        slide,
        values: values.clone().into_iter(),
        solid: &solid,
        checker: Checker {
            solid: solid_checks.clone(),
            ..Checker::default()
        },
        cams: CamTrees::new(),
        stopped: false,
    };
    let mut found = BTreeMap::new();
    // Checks one pose; false once the sweep stops.
    let mut check = |(value, pose): (f64, Result<Vec<Part>, String>)| -> bool {
        let parts = match pose {
            Ok(parts) => parts,
            Err(why) => {
                report.stopped = Some((value, why));
                return false;
            }
        };
        // Only the instances moved: the definitions are those of `scene`.
        checker.next_pose();
        checker.sweep_step(scene, &parts, within, value, &mut found);
        report.steps += 1;
        if checker.booleans() > budget {
            report.stopped = Some((value, SWEEP_TOO_COSTLY.to_string()));
            return false;
        }
        true
    };
    // The solver drives the copy to the next values on a thread of its own while the poses
    // before are checked, where threads can be started.
    let threads = std::thread::available_parallelism().is_ok_and(|n| n.get() > 1);
    let driven = threads
        && std::thread::scope(|s| {
            let (send, poses_ahead) = std::sync::mpsc::sync_channel(POSES_AHEAD);
            let driver = std::thread::Builder::new().spawn_scoped(s, move || {
                for pose in poses() {
                    if send.send(pose).is_err() {
                        break;
                    }
                }
            });
            if driver.is_err() {
                return false;
            }
            for pose in poses_ahead {
                if !check(pose) {
                    break;
                }
            }
            true
        });
    if !driven {
        for pose in poses() {
            if !check(pose) {
                break;
            }
        }
    }
    report.pairs = found.into_values().collect();
    Ok(report)
}

/// How many poses the solver may run ahead of the checks.
const POSES_AHEAD: usize = 2;

/// The parts `solid` at each of `values`, driving `joint` of a copy of a scene there in turn:
/// each value with the parts, or with why the solver could not meet it (the last one).
struct Poses<'a> {
    scene: Scene,
    joint: JointId,
    slide: Option<f64>,
    values: std::vec::IntoIter<f64>,
    solid: &'a [InstanceId],
    checker: Checker,
    cams: CamTrees,
    stopped: bool,
}

impl Iterator for Poses<'_> {
    type Item = (f64, Result<Vec<Part>, String>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.stopped {
            return None;
        }
        let value = self.values.next()?;
        if let Err(e) = self
            .scene
            .drive_joint_with(self.joint, value, self.slide, &mut self.cams)
        {
            self.stopped = true;
            return Some((value, Err(e.to_string())));
        }
        let parts = self
            .solid
            .iter()
            .filter_map(|i| self.checker.part(&self.scene, MeshId::Root, *i).ok())
            .collect();
        Some((value, Ok(parts)))
    }
}

/// About how costly checking parts `a` and `b` is, to start the costly pairs first so they do
/// not hold up the end of a parallel check.
fn cost(a: &Part, b: &Part) -> usize {
    a.shells.len() * b.shells.len()
}

/// Whether the closed mesh `mesh` is a solid part's shell, or why not.
fn solid_or_why(mesh: &Model) -> Result<(), String> {
    if let Some(why) = open_reason(mesh) {
        return Err(why);
    }
    if signed_volume(mesh).abs() <= MIN_VOLUME {
        return Err("it encloses no volume".into());
    }
    Ok(())
}

/// The triangle tree of `definition`'s own faces.
fn tree_of(scene: &Scene, definition: DefinitionId) -> Bvh {
    let mut tris = Vec::new();
    if let Some(d) = scene.definition(definition) {
        for (f, _) in d.mesh.faces() {
            tris.extend(d.mesh.triangulate(f).unwrap_or_default());
        }
    }
    Bvh::new(tris)
}

fn mesh_of<'s>(scene: &'s Scene, s: &Shell) -> &'s Model {
    &scene.definition(s.definition).expect("checked").mesh
}

fn boxes_meet((alo, ahi): (DVec3, DVec3), (blo, bhi): (DVec3, DVec3), pad: f64) -> bool {
    let pad = DVec3::splat(pad);
    alo.cmple(bhi + pad).all() && blo.cmple(ahi + pad).all()
}

/// A point inside the closed, outward mesh `m`: halfway along a ray from inside one of its
/// faces into the solid to the next face it meets, the deepest of a few faces tried.
fn inside_point(m: &Model) -> Option<DVec3> {
    let mut tris = Vec::new();
    for (f, _) in m.faces() {
        tris.extend(m.triangulate(f).unwrap_or_default());
    }
    let mut best: Option<(f64, DVec3)> = None;
    for (f, face) in m.faces().take(16) {
        let Some(p) = interior_point(m, f) else {
            continue;
        };
        let d = -face.normal;
        let hit = tris
            .iter()
            .filter_map(|t| ray_hits_triangle(p, d, *t))
            .filter(|t| *t > 1e-9)
            .min_by(f64::total_cmp);
        if let Some(t) = hit
            && best.is_none_or(|(b, _)| t > b)
        {
            best = Some((t, p + d * (t / 2.0)));
        }
    }
    best.map(|(_, p)| p)
}
