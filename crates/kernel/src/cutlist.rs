//! The cut list: the parts of a mesh's instances with the size of each part's own smallest box, its
//! material and whether it is a solid, and identical parts grouped with a quantity.
//!
//! A part is a definition's own faces placed by an instance. A group that holds instances
//! lists the instances inside it rather than itself (to any depth); its own loose faces, if
//! it has any, are one more part.

use std::collections::{BTreeSet, HashMap};

use glam::{DAffine3, DMat3, DVec3};

use crate::geom::vector_area;
use crate::ids::{DefinitionId, InstanceId, MaterialId, TagId, VertexId};
use crate::mass::dominant_material;
use crate::model::{Instance, Model};
use crate::scene::{DefinitionKind, MeshId, Scene};
use crate::solid::{open_reason, signed_volume};

/// The deepest nesting `cut_list` follows.
const MAX_DEPTH: usize = 64;

/// Sizes closer than this (meters) are the same size when parts are grouped.
const SIZE_STEP: f64 = 1e-7;

/// How parts become rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grouping {
    /// Parts of one definition, material and size are one row with a quantity.
    Definition,
    /// One row per part.
    None,
}

/// One part as found.
#[derive(Clone, Debug, PartialEq)]
pub struct CutPart {
    /// The definition's name ("Group" or "Component" when it has none).
    pub name: String,
    pub definition: DefinitionId,
    pub material: Option<MaterialId>,
    /// Longest to shortest, in meters, in the part's own axes.
    pub size: [f64; 3],
    pub solid: bool,
    /// Why it is not a solid.
    pub note: Option<String>,
    /// The listed instance that holds it (itself, or the group around it).
    pub root: InstanceId,
}

/// One row of the cut list.
#[derive(Clone, Debug, PartialEq)]
pub struct CutRow {
    pub name: String,
    pub definition: DefinitionId,
    pub material: Option<MaterialId>,
    pub size: [f64; 3],
    pub quantity: usize,
    pub solid: bool,
    pub note: Option<String>,
    /// The listed instances the parts belong to, sorted, once each.
    pub ids: Vec<InstanceId>,
}

/// The name a definition is shown under.
fn shown_name(kind: DefinitionKind, name: &str) -> String {
    match (name.trim().is_empty(), kind) {
        (false, _) => name.to_string(),
        (true, DefinitionKind::Group) => "Group".to_string(),
        (true, DefinitionKind::Component) => "Component".to_string(),
    }
}

/// The material that enclosing instances (`context`, from the root) give: the innermost
/// one's own.
pub fn context_material(scene: &Scene, context: &[InstanceId]) -> Option<MaterialId> {
    let mut mesh = MeshId::Root;
    let mut inherited = None;
    for id in context {
        let Some(inst) = scene.mesh(mesh).and_then(|m| m.instance(*id)) else {
            break;
        };
        inherited = inst.material.or(inherited);
        mesh = MeshId::Definition(inst.definition);
    }
    inherited
}

/// The corners of a part and the faces that show, in its placed (final) space.
#[derive(Default)]
struct Shape {
    points: Vec<DVec3>,
    /// Each face's unit normal, its area, and the unit directions of its sides.
    faces: Vec<(DVec3, f64, Vec<DVec3>)>,
}

/// Most point-frame products `smallest_box` measures (frames times points).
const WORK: usize = 2_000_000;

impl Shape {
    /// Adds the shown geometry of `m`, `place` taking its points to the final space. The
    /// facets of a smooth surface (a face with a soft edge) and the sides of circles and arcs
    /// give no frame: they approximate a round part, whose box follows the round, not its
    /// flats.
    fn add(&mut self, scene: &Scene, m: &Model, hidden: &BTreeSet<TagId>, place: &DAffine3) {
        self.points
            .extend(shown_points(m, hidden).map(|p| place.transform_point3(p)));
        let edge = |a: VertexId, b: VertexId| m.edge_between(a, b).and_then(|e| m.edge(e));
        for (_, f) in m.faces().filter(|(_, f)| scene.face_shown(f)) {
            let n = f.outer.len();
            let ring = |i: usize| (f.outer[i], f.outer[(i + 1) % n]);
            if std::iter::once(&f.outer)
                .chain(&f.holes)
                .flat_map(|l| (0..l.len()).map(move |i| (l[i], l[(i + 1) % l.len()])))
                .any(|(a, b)| edge(a, b).is_some_and(|e| e.soft))
            {
                continue;
            }
            let pts: Vec<DVec3> = f
                .outer
                .iter()
                .map(|v| place.transform_point3(m.pos(*v)))
                .collect();
            let area = vector_area(&pts);
            let Some(normal) = area.try_normalize() else {
                continue;
            };
            let sides: Vec<DVec3> = (0..n)
                .filter(|i| {
                    let (a, b) = ring(*i);
                    !edge(a, b).is_some_and(|e| e.curve.is_some() && !e.polygon)
                })
                .filter_map(|i| (pts[(i + 1) % n] - pts[i]).try_normalize())
                .collect();
            self.faces.push((normal, area.length(), sides));
        }
    }

    /// The size of the smallest of the boxes the shape is measured in: the frame `base` (the
    /// placement's own axes, which wins a tie) and, for each face (the largest first), the
    /// frames with its normal and each of its sides. The least volume wins, then the least
    /// area (a flat part). Longest first.
    fn smallest_box(&self, base: [DVec3; 3]) -> Option<[f64; 3]> {
        if self.points.is_empty() {
            return None;
        }
        let measure = |axes: &[DVec3; 3]| -> [f64; 3] {
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for p in &self.points {
                for k in 0..3 {
                    let d = axes[k].dot(*p);
                    lo[k] = lo[k].min(d);
                    hi[k] = hi[k].max(d);
                }
            }
            std::array::from_fn(|k| hi[k] - lo[k])
        };
        let cost = |s: &[f64; 3]| (s[0] * s[1] * s[2], s[0] * s[1] + s[1] * s[2] + s[0] * s[2]);
        let mut best = measure(&base);
        let mut best_cost = cost(&best);
        let budget = (WORK / self.points.len()).clamp(8, 4096);
        let mut order: Vec<usize> = (0..self.faces.len()).collect();
        order.sort_by(|a, b| self.faces[*b].1.total_cmp(&self.faces[*a].1));
        let mut tried: BTreeSet<[i64; 6]> = BTreeSet::new();
        let key = |v: DVec3| {
            // Either way along an axis is the same axis.
            let flip = v
                .to_array()
                .iter()
                .find(|c| c.abs() > 1e-9)
                .is_some_and(|c| *c < 0.0);
            let v = if flip { -v } else { v };
            v.to_array().map(|c| (c * 1e6).round() as i64)
        };
        'faces: for k in order {
            let (normal, _, sides) = &self.faces[k];
            for side in sides {
                if tried.len() >= budget {
                    break 'faces;
                }
                let along = (*side - *normal * side.dot(*normal)).try_normalize();
                let Some(along) = along else { continue };
                let (kn, ka) = (key(*normal), key(along));
                let mut id = [0; 6];
                id[..3].copy_from_slice(&kn.min(ka));
                id[3..].copy_from_slice(&kn.max(ka));
                if !tried.insert(id) {
                    continue;
                }
                let size = measure(&[along, normal.cross(along), *normal]);
                let c = cost(&size);
                let tol = 1e-9 * best_cost.0.max(c.0);
                if c.0 < best_cost.0 - tol
                    || (c.0 <= best_cost.0 + tol && c.1 < best_cost.1 * (1.0 - 1e-9))
                {
                    best = size;
                    best_cost = c;
                }
            }
        }
        best.sort_by(|a, b| b.total_cmp(a));
        Some(best)
    }
}

/// The orthonormal frame of `linear`'s columns (Gram-Schmidt), the placement's own axes when
/// it has no shear; None when the placement is flattened along an axis.
fn own_axes(linear: DMat3) -> Option<[DVec3; 3]> {
    let x = linear.x_axis.try_normalize()?;
    let y = (linear.y_axis - x * linear.y_axis.dot(x)).try_normalize()?;
    let z =
        (linear.z_axis - x * linear.z_axis.dot(x) - y * linear.z_axis.dot(y)).try_normalize()?;
    Some([x, y, z])
}

/// The lengths of a box `extent` long along the axes of its mesh, once `linear` places the
/// mesh: each extent times the length of that axis, longest first. Used for a placement that
/// is flattened along an axis.
fn sized(extent: DVec3, linear: DMat3) -> [f64; 3] {
    let axes = [linear.x_axis, linear.y_axis, linear.z_axis];
    let extent = extent.to_array();
    let mut size: [f64; 3] = std::array::from_fn(|i| extent[i] * axes[i].length());
    size.sort_by(|a, b| b.total_cmp(a));
    size
}

/// The size of `shape` (placed by `linear`, with `extent` its extent along its own axes)
/// as `Shape::smallest_box` finds it.
fn shape_size(shape: &Shape, extent: DVec3, linear: DMat3) -> [f64; 3] {
    own_axes(linear)
        .and_then(|axes| shape.smallest_box(axes))
        .unwrap_or_else(|| sized(extent, linear))
}

/// The positions of the vertices of `m` that show (on a shown edge or face, with `hidden` the
/// hidden tags).
fn shown_points<'a>(m: &'a Model, hidden: &'a BTreeSet<TagId>) -> impl Iterator<Item = DVec3> + 'a {
    let all_shown = hidden.is_empty() && !m.has_hidden_edge_or_face();
    m.vertices()
        .filter(move |(id, _)| all_shown || m.vertex_shown(*id, hidden, false))
        .map(|(_, v)| v.pos)
}

/// The extent of the shown vertices of `m` along its axes.
fn extent(m: &Model, hidden: &BTreeSet<TagId>) -> Option<DVec3> {
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = DVec3::splat(f64::NEG_INFINITY);
    let mut any = false;
    for p in shown_points(m, hidden) {
        lo = lo.min(p);
        hi = hi.max(p);
        any = true;
    }
    any.then(|| hi - lo)
}

/// The own box of `definition` placed by `linear`, longest first: the smallest box around its
/// shown geometry and the shown instances it places inside it (see `Shape::smallest_box`),
/// measured as `linear` places it. None when nothing of it shows.
pub fn own_size(scene: &Scene, definition: DefinitionId, linear: DMat3) -> Option<[f64; 3]> {
    let hidden = scene.hidden_tags();
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = DVec3::splat(f64::NEG_INFINITY);
    let mut shape = Shape::default();
    let mut stack = vec![(definition, DAffine3::IDENTITY, 0usize)];
    while let Some((d, place, depth)) = stack.pop() {
        let Some(mesh) = scene.mesh(MeshId::Definition(d)) else {
            continue;
        };
        for p in shown_points(mesh, &hidden) {
            let p = place.transform_point3(p);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        shape.add(scene, mesh, &hidden, &(DAffine3::from_mat3(linear) * place));
        if depth < MAX_DEPTH {
            for (_, inner) in mesh.instances() {
                if scene.placed_instance_shown(inner) {
                    stack.push((inner.definition, place * inner.transform, depth + 1));
                }
            }
        }
    }
    (!shape.points.is_empty()).then(|| shape_size(&shape, hi - lo, linear))
}

struct Walk<'a> {
    scene: &'a Scene,
    hidden: BTreeSet<TagId>,
    parts: Vec<CutPart>,
    /// Sizes found, by definition and the shape of the placement (its columns' dot products,
    /// which a turn does not change).
    sizes: HashMap<(DefinitionId, [i64; 6]), [f64; 3]>,
}

impl Walk<'_> {
    fn instance(
        &mut self,
        root: InstanceId,
        instance: &Instance,
        linear: DMat3,
        inherited: Option<MaterialId>,
        depth: usize,
    ) {
        let scene = self.scene;
        if depth > MAX_DEPTH || !scene.placed_instance_shown(instance) {
            return;
        }
        let Some(def) = scene.definition(instance.definition) else {
            return;
        };
        let m = &def.mesh;
        let linear = linear * instance.transform.matrix3;
        let material = instance.material.or(inherited);
        // A part is its shown faces; hidden geometry does not count in its size.
        if m.faces().any(|(_, f)| scene.face_shown(f))
            && let Some(extent) = extent(m, &self.hidden)
        {
            let gram = linear.transpose() * linear;
            let [a, b, c, d, e, g] = [
                gram.x_axis.x,
                gram.x_axis.y,
                gram.x_axis.z,
                gram.y_axis.y,
                gram.y_axis.z,
                gram.z_axis.z,
            ];
            let key = (
                instance.definition,
                [a, b, c, d, e, g].map(|x| (x * 1e9).round() as i64),
            );
            let size = *self.sizes.entry(key).or_insert_with(|| {
                let mut shape = Shape::default();
                shape.add(scene, m, &self.hidden, &DAffine3::from_mat3(linear));
                shape_size(&shape, extent, linear)
            });
            let (solid, note) = match open_reason(m) {
                Some(why) => (false, Some(why)),
                None if signed_volume(m).abs() <= 1e-12 => {
                    (false, Some("it encloses no volume".to_string()))
                }
                None => (true, None),
            };
            // Faces pointing inward show their back on the outside.
            let side = usize::from(solid && signed_volume(m) < 0.0);
            self.parts.push(CutPart {
                name: shown_name(def.kind, &def.name),
                definition: instance.definition,
                material: dominant_material(m, side).or(material),
                size,
                solid,
                note,
                root,
            });
        }
        for (_, inner) in m.instances() {
            self.instance(root, inner, linear, material, depth + 1);
        }
    }
}

/// The parts of `ids` (instances of `mesh`, repeats counted once; empty: every instance of
/// it), found through nesting. `inherited` is the material of the instances enclosing `mesh`
/// (see `context_material`). Unknown ids are an error.
pub fn cut_parts(
    scene: &Scene,
    mesh: &Model,
    ids: &[InstanceId],
    inherited: Option<MaterialId>,
) -> Result<Vec<CutPart>, String> {
    let mut ids: Vec<InstanceId> = if ids.is_empty() {
        mesh.instances().map(|(id, _)| id).collect()
    } else {
        ids.to_vec()
    };
    let mut seen = BTreeSet::new();
    ids.retain(|id| seen.insert(*id));
    let mut walk = Walk {
        scene,
        hidden: scene.hidden_tags(),
        parts: Vec::new(),
        sizes: HashMap::new(),
    };
    for id in ids {
        let instance = mesh
            .instance(id)
            .ok_or_else(|| format!("no instance i{}", id.0))?;
        walk.instance(id, instance, DMat3::IDENTITY, inherited, 0);
    }
    Ok(walk.parts)
}

/// The cut list: `cut_parts` grouped as `grouping` says, ordered by material name (case blind,
/// no material last), then size (length, width, thickness, largest first), then name.
pub fn cut_list(
    scene: &Scene,
    mesh: &Model,
    ids: &[InstanceId],
    inherited: Option<MaterialId>,
    grouping: Grouping,
) -> Result<Vec<CutRow>, String> {
    let parts = cut_parts(scene, mesh, ids, inherited)?;
    // Sizes are compared within `SIZE_STEP` rather than rounded to it, so float noise either
    // side of a rounding boundary does not split identical parts.
    let same = |r: &CutRow, p: &CutPart| {
        r.definition == p.definition
            && r.material == p.material
            && r.solid == p.solid
            && (0..3).all(|i| (r.size[i] - p.size[i]).abs() <= SIZE_STEP)
    };
    let mut rows: Vec<CutRow> = Vec::new();
    for p in parts {
        let at = (grouping == Grouping::Definition)
            .then(|| rows.iter().position(|r| same(r, &p)))
            .flatten();
        match at {
            Some(i) => {
                let row = &mut rows[i];
                row.quantity += 1;
                if let Err(at) = row.ids.binary_search(&p.root) {
                    row.ids.insert(at, p.root);
                }
            }
            None => rows.push(CutRow {
                name: p.name,
                definition: p.definition,
                material: p.material,
                size: p.size,
                quantity: 1,
                solid: p.solid,
                note: p.note,
                ids: vec![p.root],
            }),
        }
    }
    let material_name = |m: Option<MaterialId>| {
        m.and_then(|m| scene.material(m))
            .map(|m| m.name.to_lowercase())
    };
    rows.sort_by(|a, b| {
        let (ma, mb) = (material_name(a.material), material_name(b.material));
        // Rows with no material come last.
        ma.is_none()
            .cmp(&mb.is_none())
            .then_with(|| ma.cmp(&mb))
            .then_with(|| {
                (0..3)
                    .map(|i| b.size[i].total_cmp(&a.size[i]))
                    .find(|o| o.is_ne())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.ids.cmp(&b.ids))
    });
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plank `a` by `b` by `t` (z) as a shape: its corner points and its six faces.
    fn plank(a: DVec3, b: DVec3, t: f64) -> Shape {
        let z = DVec3::new(0.0, 0.0, t);
        let mut shape = Shape::default();
        for k in 0..8 {
            shape
                .points
                .push(a * (k & 1) as f64 + b * ((k >> 1) & 1) as f64 + z * ((k >> 2) & 1) as f64);
        }
        let n = a.cross(b).normalize();
        let dirs =
            |u: DVec3, v: DVec3| vec![u.normalize(), v.normalize(), -u.normalize(), -v.normalize()];
        shape.faces.push((n, a.cross(b).length(), dirs(a, b)));
        shape.faces.push((-n, a.cross(b).length(), dirs(a, b)));
        for (u, w) in [(a, z), (b, z)] {
            let m = u.cross(w).normalize();
            shape.faces.push((m, u.cross(w).length(), dirs(u, w)));
            shape.faces.push((-m, u.cross(w).length(), dirs(u, w)));
        }
        shape
    }

    fn close(a: [f64; 3], b: [f64; 3]) {
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-9, "{a:?} vs {b:?}");
        }
    }

    const WORLD: [DVec3; 3] = [DVec3::X, DVec3::Y, DVec3::Z];

    #[test]
    fn a_plank_square_to_its_axes_reads_its_axes() {
        let s = plank(DVec3::new(1.0, 0.0, 0.0), DVec3::new(0.0, 0.1, 0.0), 0.02);
        close(s.smallest_box(WORLD).unwrap(), [1.0, 0.1, 0.02]);
    }

    #[test]
    fn a_plank_modelled_turned_reads_its_own_sides_not_its_axes() {
        let turn = DMat3::from_rotation_z(30f64.to_radians());
        let s = plank(turn * DVec3::X, turn * (DVec3::Y * 0.1), 0.02);
        close(s.smallest_box(WORLD).unwrap(), [1.0, 0.1, 0.02]);
    }

    #[test]
    fn a_sheared_plank_reads_the_smallest_box_around_it() {
        // A 1 x 0.1 plank turned 30 degrees and stretched twice along x: a parallelogram.
        // Its smallest box lies along its long side, 1.875 x 0.111, where the axes box is
        // 1.83 x 0.587 and the old reading (side lengths) 1.803 x 0.132.
        let a = DVec3::new(
            2.0 * 30f64.to_radians().cos(),
            30f64.to_radians().sin(),
            0.0,
        );
        let b = DVec3::new(-0.1, 0.1 * 30f64.to_radians().cos(), 0.0);
        let s = plank(a, b, 0.02);
        let size = s.smallest_box(WORLD).unwrap();
        for (got, want) in size.iter().zip([1.874_86, 0.110_94, 0.02]) {
            assert!((got - want).abs() < 1e-4, "{size:?}");
        }
    }

    #[test]
    fn a_flat_part_picks_the_smallest_area() {
        let s = plank(DVec3::new(2.0, 0.0, 0.0), DVec3::new(0.0, 0.5, 0.0), 0.0);
        let turn = DMat3::from_rotation_z(0.3);
        let flat = Shape {
            points: s.points.iter().map(|p| turn * *p).collect(),
            faces: s
                .faces
                .iter()
                .map(|(n, a, d)| (turn * *n, *a, d.iter().map(|x| turn * *x).collect()))
                .collect(),
        };
        close(flat.smallest_box(WORLD).unwrap(), [2.0, 0.5, 0.0]);
    }

    #[test]
    fn a_part_with_no_points_has_no_box() {
        assert!(Shape::default().smallest_box(WORLD).is_none());
    }
}
