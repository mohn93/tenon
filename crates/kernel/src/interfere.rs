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

use std::collections::{BTreeMap, HashMap};

use glam::{DAffine3, DVec3};

use crate::boolean::{BooleanOp, combine, interior_point, placed_copy, point_inside};
use crate::bvh::{Bvh, Closest};
use crate::edit::KernelError;
use crate::geom::LENGTH_TOL;
use crate::ids::{DefinitionId, InstanceId, JointId};
use crate::index::ray_hits_triangle;
use crate::model::Model;
use crate::scene::{MeshId, Scene};
use crate::solid::{open_reason, signed_volume};

/// Overlaps of at most this many cubic meters are touching, not interference.
pub const MIN_VOLUME: f64 = 1e-12;

/// The fewest and most steps of a sweep.
pub const MIN_SWEEP_STEPS: usize = 2;
pub const MAX_SWEEP_STEPS: usize = 10_000;

/// A sweep reports the pairs that come within this many meters (by default).
pub const SWEEP_REPORT_WITHIN: f64 = 0.005;

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
enum Probe {
    /// Their surfaces are this far apart (0 when touching), within the report distance.
    Near(f64),
    Overlap(f64),
    Failed(String),
}

/// Triangle trees and solid checks per definition, kept across queries and poses. The
/// definitions' geometry must not change while it is kept (moving instances is fine).
#[derive(Debug, Default)]
pub struct Checker {
    trees: HashMap<DefinitionId, Bvh>,
    solid: HashMap<DefinitionId, Result<(), String>>,
}

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
            .or_insert_with(|| {
                let mesh = &scene.definition(definition).expect("exists").mesh;
                if let Some(why) = open_reason(mesh) {
                    return Err(why);
                }
                if signed_volume(mesh).abs() <= MIN_VOLUME {
                    return Err("it encloses no volume".into());
                }
                Ok(())
            })
            .clone()
    }

    /// The tree of `definition`, built on first use.
    pub fn tree(&mut self, scene: &Scene, definition: DefinitionId) -> &Bvh {
        self.trees.entry(definition).or_insert_with(|| {
            let mut tris = Vec::new();
            if let Some(d) = scene.definition(definition) {
                for (f, _) in d.mesh.faces() {
                    tris.extend(d.mesh.triangulate(f).unwrap_or_default());
                }
            }
            Bvh::new(tris)
        })
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
        let mut volume = 0.0;
        let mut deepest: Option<(f64, DVec3)> = None;
        let mut touch = None;
        for sa in &a.shells {
            for sb in &b.shells {
                let Some(meet) = self.shells_meet(scene, sa, sb) else {
                    continue;
                };
                let ma = placed_copy(mesh_of(scene, sa), sa.place, None);
                let mb = placed_copy(mesh_of(scene, sb), sb.place, None);
                match combine(&ma, &mb, BooleanOp::Intersect) {
                    Ok(Some(m)) => {
                        let v = signed_volume(&m);
                        if v > MIN_VOLUME {
                            volume += v;
                            if deepest.is_none_or(|(w, _)| v > w) {
                                let p = inside_point(&m).unwrap_or(meet);
                                deepest = Some((v, p));
                            }
                        } else {
                            touch.get_or_insert(meet);
                        }
                    }
                    Ok(None) => {
                        touch.get_or_insert(meet);
                    }
                    Err(e) => return Contact::Failed(e.to_string()),
                }
            }
        }
        match (deepest, touch) {
            (Some((_, point)), _) if volume > MIN_VOLUME => Contact::Overlap { volume, point },
            (_, Some(point)) => Contact::Touching { point },
            _ => Contact::Apart,
        }
    }

    /// Where shells `sa` and `sb` meet: a point where their surfaces come within
    /// `LENGTH_TOL`, or a vertex of one lying inside the other. None when they lie apart.
    fn shells_meet(&mut self, scene: &Scene, sa: &Shell, sb: &Shell) -> Option<DVec3> {
        let (ba, bb) = (
            self.tree(scene, sa.definition).placed_bounds(&sa.place)?,
            self.tree(scene, sb.definition).placed_bounds(&sb.place)?,
        );
        if !boxes_meet(ba, bb, LENGTH_TOL) {
            return None;
        }
        if let Some(c) = self.shell_distance(scene, sa, sb, 2.0 * LENGTH_TOL, LENGTH_TOL)
            && c.distance <= LENGTH_TOL
        {
            return Some(c.a);
        }
        self.contained(scene, sa, sb)
            .or_else(|| self.contained(scene, sb, sa))
    }

    /// A vertex of `inner` when it lies inside `outer` (their surfaces apart).
    fn contained(&mut self, scene: &Scene, inner: &Shell, outer: &Shell) -> Option<DVec3> {
        let (bi, bo) = (
            self.tree(scene, inner.definition)
                .placed_bounds(&inner.place)?,
            self.tree(scene, outer.definition)
                .placed_bounds(&outer.place)?,
        );
        if !boxes_meet(bi, bo, 0.0) {
            return None;
        }
        let inner_mesh = &scene.definition(inner.definition)?.mesh;
        let outer_mesh = &scene.definition(outer.definition)?.mesh;
        let back = outer.place.inverse();
        // Any vertex answers (the surfaces are apart); a few are tried in case rays graze.
        for (_, v) in inner_mesh.vertices().take(8) {
            let p = inner.place.transform_point3(v.pos);
            if let Some(inside) = point_inside(back.transform_point3(p), outer_mesh) {
                return inside.then_some(p);
            }
        }
        None
    }

    fn shell_distance(
        &mut self,
        scene: &Scene,
        sa: &Shell,
        sb: &Shell,
        bound: f64,
        stop: f64,
    ) -> Option<Closest> {
        self.tree(scene, sa.definition);
        self.tree(scene, sb.definition);
        let (ta, tb) = (&self.trees[&sa.definition], &self.trees[&sb.definition]);
        ta.distance(&sa.place, tb, &sb.place, bound, stop)
    }

    /// Every overlapping and touching pair among `parts` (pairs whose boxes meet).
    pub fn interference(&mut self, scene: &Scene, parts: &[Part]) -> InterferenceReport {
        let boxes: Vec<(DVec3, DVec3)> = parts.iter().map(|p| self.bounds(scene, p)).collect();
        let mut order: Vec<usize> = (0..parts.len()).collect();
        order.sort_by(|i, j| boxes[*i].0.x.total_cmp(&boxes[*j].0.x));
        let mut report = InterferenceReport::default();
        for (k, &i) in order.iter().enumerate() {
            for &j in &order[k + 1..] {
                if boxes[j].0.x > boxes[i].1.x + LENGTH_TOL {
                    break;
                }
                if !boxes_meet(boxes[i], boxes[j], LENGTH_TOL) {
                    continue;
                }
                let (a, b) = if parts[i].id <= parts[j].id {
                    (&parts[i], &parts[j])
                } else {
                    (&parts[j], &parts[i])
                };
                match self.contact(scene, a, b) {
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
        let mut best: Option<Closest> = None;
        for sa in &a.shells {
            for sb in &b.shells {
                let bound = best.map_or(f64::INFINITY, |c| c.distance);
                if let Some(c) = self.shell_distance(scene, sa, sb, bound, 0.0) {
                    best = Some(c);
                }
            }
        }
        let best = best?;
        if best.distance <= LENGTH_TOL {
            let overlap = matches!(self.contact(scene, a, b), Contact::Overlap { .. });
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
    fn probe(&mut self, scene: &Scene, a: &Part, b: &Part, within: f64) -> Option<Probe> {
        let reach = within + LENGTH_TOL;
        let mut best: Option<f64> = None;
        let mut measure = false;
        for sa in &a.shells {
            for sb in &b.shells {
                let (Some(ba), Some(bb)) = (
                    self.tree(scene, sa.definition).placed_bounds(&sa.place),
                    self.tree(scene, sb.definition).placed_bounds(&sb.place),
                ) else {
                    continue;
                };
                if !boxes_meet(ba, bb, reach) {
                    continue;
                }
                if !measure {
                    measure = self.shells_cross(scene, sa, sb)
                        || self.encloses(scene, sa, sb) != Some(false)
                        || self.encloses(scene, sb, sa) != Some(false);
                }
                if best.is_none_or(|d| d > LENGTH_TOL) {
                    let bound = best.map_or(reach, |d| d.min(reach));
                    if let Some(c) = self.shell_distance(scene, sa, sb, bound, LENGTH_TOL) {
                        best = Some(c.distance);
                    }
                }
            }
        }
        if measure {
            match self.contact(scene, a, b) {
                Contact::Overlap { volume, .. } => return Some(Probe::Overlap(volume)),
                Contact::Touching { .. } => return Some(Probe::Near(0.0)),
                Contact::Failed(why) => return Some(Probe::Failed(why)),
                Contact::Apart => {}
            }
        }
        let d = best.filter(|d| *d <= within)?;
        Some(Probe::Near(if d <= LENGTH_TOL { 0.0 } else { d }))
    }

    /// Whether the surfaces of shells `sa` and `sb` cross (see [`Bvh::crosses`]).
    fn shells_cross(&mut self, scene: &Scene, sa: &Shell, sb: &Shell) -> bool {
        self.tree(scene, sa.definition);
        self.tree(scene, sb.definition);
        let (ta, tb) = (&self.trees[&sa.definition], &self.trees[&sb.definition]);
        ta.crosses(&sa.place, tb, &sb.place, LENGTH_TOL)
    }

    /// Whether shell `inner` lies inside `outer`, their surfaces not crossing: told by the
    /// first of its vertices that is not on `outer`'s surface; None when none tried is.
    fn encloses(&mut self, scene: &Scene, inner: &Shell, outer: &Shell) -> Option<bool> {
        let (Some(bi), Some(bo)) = (
            self.tree(scene, inner.definition)
                .placed_bounds(&inner.place),
            self.tree(scene, outer.definition)
                .placed_bounds(&outer.place),
        ) else {
            return Some(false);
        };
        if !boxes_meet(bi, bo, 0.0) {
            return Some(false);
        }
        let inner_mesh = &scene.definition(inner.definition)?.mesh;
        let outer_mesh = &scene.definition(outer.definition)?.mesh;
        let back = outer.place.inverse() * inner.place;
        inner_mesh
            .vertices()
            .take(32)
            .find_map(|(_, v)| point_inside(back.transform_point3(v.pos), outer_mesh))
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
                let Some(probe) = self.probe(scene, a, b, within) else {
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
                };
                if distance < e.clearance {
                    e.clearance = distance;
                    e.clearance_at = value;
                }
            }
        }
    }
}

/// Drives `joint` of a copy of `scene` from `from` to `to` (radians or meters) in `steps`
/// evenly spaced values, the ends included (`slide` held, for a cylindrical joint), and at
/// each checks every pair of the root instances `ids` (every one when empty) whose boxes
/// come within `within` meters. Pairs that never come within `within` are left out. A value
/// the solver cannot meet stops the sweep there. The scene is never changed. The triangle
/// trees are built once per definition and placed anew at each step.
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
    let mut scene = scene.clone();
    let mut checker = Checker::new();
    let (parts, skipped) = checker.parts(&scene, MeshId::Root, ids);
    let solid: Vec<InstanceId> = parts.iter().map(|p| p.id).collect();
    let mut report = SweepReport {
        skipped,
        ..SweepReport::default()
    };
    let mut found = BTreeMap::new();
    for k in 0..steps {
        let value = if k + 1 == steps {
            to
        } else {
            from + (to - from) * (k as f64 / (steps - 1) as f64)
        };
        if let Err(e) = scene.drive_joint(joint, value, slide) {
            report.stopped = Some((value, e.to_string()));
            break;
        }
        let parts: Vec<Part> = solid
            .iter()
            .filter_map(|i| checker.part(&scene, MeshId::Root, *i).ok())
            .collect();
        checker.sweep_step(&scene, &parts, within, value, &mut found);
        report.steps += 1;
    }
    report.pairs = found.into_values().collect();
    Ok(report)
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
