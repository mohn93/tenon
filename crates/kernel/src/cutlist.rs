//! The cut list: the parts of a mesh's instances with the size of each part's own box, its
//! material and whether it is a solid, and identical parts grouped with a quantity.
//!
//! A part is a definition's own faces placed by an instance. A group that holds instances
//! lists the instances inside it rather than itself (to any depth); its own loose faces, if
//! it has any, are one more part.

use std::collections::BTreeSet;

use glam::{DAffine3, DMat3, DVec3};

use crate::ids::{DefinitionId, InstanceId, MaterialId, TagId};
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

/// The lengths of a box `extent` long along the axes of its mesh, once `linear` places the
/// mesh: each extent times the length of that axis, longest first.
fn sized(extent: DVec3, linear: DMat3) -> [f64; 3] {
    let axes = [linear.x_axis, linear.y_axis, linear.z_axis];
    let extent = extent.to_array();
    let mut size: [f64; 3] = std::array::from_fn(|i| extent[i] * axes[i].length());
    size.sort_by(|a, b| b.total_cmp(a));
    size
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

/// The own box of `definition` placed by `linear`, longest first: the extent of its shown
/// geometry and of the shown instances it places inside it along its own axes, scaled by the
/// placement's axis lengths. None when nothing of it shows.
pub fn own_size(scene: &Scene, definition: DefinitionId, linear: DMat3) -> Option<[f64; 3]> {
    let hidden = scene.hidden_tags();
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = DVec3::splat(f64::NEG_INFINITY);
    let mut any = false;
    let mut stack = vec![(definition, DAffine3::IDENTITY, 0usize)];
    while let Some((d, place, depth)) = stack.pop() {
        let Some(mesh) = scene.mesh(MeshId::Definition(d)) else {
            continue;
        };
        for p in shown_points(mesh, &hidden) {
            let p = place.transform_point3(p);
            lo = lo.min(p);
            hi = hi.max(p);
            any = true;
        }
        if depth < MAX_DEPTH {
            for (_, inner) in mesh.instances() {
                if scene.placed_instance_shown(inner) {
                    stack.push((inner.definition, place * inner.transform, depth + 1));
                }
            }
        }
    }
    any.then(|| sized(hi - lo, linear))
}

struct Walk<'a> {
    scene: &'a Scene,
    hidden: BTreeSet<TagId>,
    parts: Vec<CutPart>,
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
                size: sized(extent, linear),
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
