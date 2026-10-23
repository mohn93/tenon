//! The parts of a model: its top-level groups and components (the root mesh's instances), each
//! with the name it is shown under and the size of the box around its placed geometry.
//!
//! Only top-level instances are parts: what is nested inside a part belongs to it and is not
//! listed or counted. A component's "1 of N" counts the top-level instances of its definition.

use std::collections::BTreeSet;

use glam::{DAffine3, DVec3};
use kernel::{DefinitionId, DefinitionKind, InstanceId, MeshId, Scene, TagId};
use session::{Units, format_length_in};

/// One part: a top-level group or component instance.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub id: InstanceId,
    pub definition: DefinitionId,
    pub kind: DefinitionKind,
    /// The group's name, or the component's definition name ("Group" and "Component" when it
    /// has none).
    pub name: String,
    /// Width (x), depth (y) and height (z) of the axis-aligned box, in model axes, around the
    /// geometry the part places, in meters. None when it places no geometry.
    pub size: Option<[f64; 3]>,
    /// For a component, which of its definition's top-level instances this is (1 first, in id
    /// order).
    pub number: usize,
    /// How many top-level instances share the definition.
    pub count: usize,
}

impl Part {
    /// "1 of 3" for a component (top-level instances only), nothing for a group.
    pub fn ordinal(&self) -> Option<String> {
        (self.kind == DefinitionKind::Component)
            .then(|| format!("{} of {}", self.number, self.count))
    }
}

/// Every shown top-level instance of `scene`, in id order. Instances on hidden tags or hidden
/// themselves are not parts (they are not seen).
pub fn list(scene: &Scene) -> Vec<Part> {
    let hidden = scene.hidden_tags();
    let root = scene.root();
    let mut counts: std::collections::BTreeMap<DefinitionId, usize> = Default::default();
    let mut parts = Vec::new();
    for (id, instance) in root.instances() {
        if !scene.placed_instance_shown(instance) {
            continue;
        }
        let Some(definition) = scene.definition(instance.definition) else {
            continue;
        };
        let n = counts.entry(instance.definition).or_default();
        *n += 1;
        let blank = definition.name.trim().is_empty();
        let name = match (blank, definition.kind) {
            (false, _) => definition.name.clone(),
            (true, DefinitionKind::Group) => "Group".to_string(),
            (true, DefinitionKind::Component) => "Component".to_string(),
        };
        parts.push(Part {
            id,
            definition: instance.definition,
            kind: definition.kind,
            name,
            size: extent(scene, &hidden, instance.definition, instance.transform)
                .map(|[lo, hi]| (hi - lo).to_array()),
            number: *n,
            count: 0,
        });
    }
    for part in &mut parts {
        part.count = counts[&part.definition];
    }
    parts
}

/// The box (in the placing mesh's axes) around the shown geometry of definition `definition`
/// placed by `place`: every vertex that shows, taken through the transforms of the instances
/// down to it (so a turned or scaled part gets the box of what it really places, not the box
/// of its own box turned).
fn extent(
    scene: &Scene,
    hidden: &BTreeSet<TagId>,
    definition: DefinitionId,
    place: DAffine3,
) -> Option<[DVec3; 2]> {
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = DVec3::splat(f64::NEG_INFINITY);
    let mut seen = 0usize;
    // Definitions do not nest cyclically (the kernel forbids it), but a walk that cannot loop
    // forever is cheap insurance against a damaged file.
    let mut stack = vec![(definition, place, 0usize)];
    while let Some((definition, place, depth)) = stack.pop() {
        let Some(mesh) = scene.mesh(MeshId::Definition(definition)) else {
            continue;
        };
        // Nothing to look up when nothing hides a vertex.
        let all_shown = hidden.is_empty() && !mesh.has_hidden_edge_or_face();
        for (id, v) in mesh.vertices() {
            if all_shown || mesh.vertex_shown(id, hidden, false) {
                let p = place.transform_point3(v.pos);
                lo = lo.min(p);
                hi = hi.max(p);
                seen += 1;
            }
        }
        if depth > 64 {
            continue;
        }
        for (_, inner) in mesh.instances() {
            if scene.placed_instance_shown(inner) {
                stack.push((inner.definition, place * inner.transform, depth + 1));
            }
        }
    }
    (seen > 0).then_some([lo, hi])
}

/// "1.2 m x 0.6 m x 2 m": width, depth and height in the document's units.
pub fn size_text(size: [f64; 3], units: &Units) -> String {
    size.map(|v| format_length_in(v, units)).join(" x ")
}

/// A row of the parts list: a group, or every instance of one component together.
#[derive(Clone, Debug, PartialEq)]
pub struct PartRow {
    pub name: String,
    pub kind: DefinitionKind,
    /// How many instances the row stands for (1 for a group).
    pub count: usize,
    /// The first instance's size.
    pub size: Option<[f64; 3]>,
    /// The instances, in id order: what tapping the row selects and frames.
    pub ids: Vec<InstanceId>,
}

/// The parts list: each group a row, each component's instances one row with their count, in
/// the order the parts first appear.
pub fn rows(parts: &[Part]) -> Vec<PartRow> {
    let mut rows: Vec<PartRow> = Vec::new();
    let mut by_definition: std::collections::BTreeMap<DefinitionId, usize> = Default::default();
    for part in parts {
        if part.kind == DefinitionKind::Component
            && let Some(&at) = by_definition.get(&part.definition)
        {
            rows[at].count += 1;
            rows[at].ids.push(part.id);
            continue;
        }
        if part.kind == DefinitionKind::Component {
            by_definition.insert(part.definition, rows.len());
        }
        rows.push(PartRow {
            name: part.name.clone(),
            kind: part.kind,
            count: 1,
            size: part.size,
            ids: vec![part.id],
        });
    }
    rows
}

#[cfg(test)]
mod tests;
