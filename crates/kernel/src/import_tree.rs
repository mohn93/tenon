//! Building an import tree read from a file (SketchUp): definitions with faces that may have
//! holes, soft and loose edges, guides and nested instances, each definition built once and
//! shared by all its instances. Points are welded and faces made as `build_mesh` does, with no
//! sticky geometry: nothing is split at crossings and no face is added that the file lacks.

use std::collections::{HashMap, HashSet};

use glam::{DAffine3, DVec3};

use crate::MIN_DETERMINANT;
use crate::annotation::{Anchor, Dimension, Note};
use crate::edit::KernelError;
use crate::geom::{LENGTH_TOL, vector_area};
use crate::ids::{DefinitionId, EdgeId, InstanceId, MaterialId, TagId, VertexId};
use crate::import::{FLAT_TOL, MAX_IMPORT_FACES, normal_of, pieces, weld};
use crate::model::{ChangeSet, Edge, Face, Guide, Instance, Model};
use crate::scene::{DefinitionKind, MeshId, Scene, SceneChange};
use crate::texture::TexturePlacement;
use crate::transform::rings_are_valid;

/// Deepest nesting of instances an import tree may have (the root's instances are depth 1).
pub const MAX_IMPORT_DEPTH: usize = 32;

/// Most definitions an import tree may have.
pub const MAX_IMPORT_DEFINITIONS: usize = 10_000;

/// Most instances an import tree may have, over all its geometries.
pub const MAX_IMPORT_INSTANCES: usize = 100_000;

/// Most placements an import tree may resolve to, counting every instance once per placement
/// of the definition holding it (a component of 10 placed in one placed 10 times is 110).
pub const MAX_IMPORT_PLACEMENTS: u64 = 1_000_000;

/// Most faces an import tree may show, counting a definition's faces once per placement.
pub const MAX_IMPORT_PLACED_FACES: u64 = 20_000_000;

/// Most dimensions and notes an import tree may have, over all its geometries.
pub const MAX_IMPORT_ANNOTATIONS: usize = 10_000;

/// Most pairs of loop sides one import checks for crossings (the check is quadratic in a face's
/// corners). Faces past it are made from their outer loop alone, as a bent face is.
const MAX_RING_CHECKS: usize = 20_000_000;

/// Definitions read from a file and the root geometry that places them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportTree {
    pub definitions: Vec<ImportDefinition>,
    pub root: ImportGeometry,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportDefinition {
    pub name: String,
    pub kind: DefinitionKind,
    pub geometry: ImportGeometry,
}

/// One mesh's contents: faces and edges index into `points`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportGeometry {
    pub points: Vec<DVec3>,
    pub faces: Vec<ImportFace>,
    pub edges: Vec<ImportEdge>,
    pub guides: Vec<Guide>,
    pub instances: Vec<ImportInstance>,
    pub dimensions: Vec<ImportDimension>,
    pub notes: Vec<ImportNote>,
}

/// A linear dimension between two points of the mesh, its line moved by `offset`. Each end is
/// anchored to the vertex made at its point (within `LENGTH_TOL`), else fixed there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImportDimension {
    pub a: DVec3,
    pub b: DVec3,
    pub offset: DVec3,
    pub tag: Option<TagId>,
}

/// A leader note: text at `position`, its leader ending at `anchor` (anchored to the vertex
/// at that point when there is one, else fixed there). Without text the label is the
/// anchored point's coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportNote {
    pub anchor: DVec3,
    pub position: DVec3,
    pub text: Option<String>,
    pub tag: Option<TagId>,
}

/// A face: loop 0 is the outer loop, the others holes, each in any winding. The face looks
/// the way its outer loop winds counter-clockwise around.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportFace {
    pub loops: Vec<Vec<usize>>,
    pub front: Option<MaterialId>,
    pub back: Option<MaterialId>,
    /// Where the texture lies on the face (see `Face::texture`).
    pub texture: Option<TexturePlacement>,
    pub tag: Option<TagId>,
    pub hidden: bool,
}

/// An edge of the file: face sides and loose edges alike, so loose ones are kept and face
/// sides can be softened and tagged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportEdge {
    pub a: usize,
    pub b: usize,
    pub soft: bool,
    pub tag: Option<TagId>,
    pub hidden: bool,
}

/// A placement of definition `definition` (an index into `ImportTree::definitions`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImportInstance {
    pub definition: usize,
    pub transform: DAffine3,
    pub material: Option<MaterialId>,
    pub tag: Option<TagId>,
    pub hidden: bool,
}

/// What building an import tree made. Counts are over every mesh built, each shared
/// definition once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeReport {
    /// Definitions made from the tree's definitions (not counting a wrapper).
    pub definitions: usize,
    /// Instances made from the tree's instances (not counting a wrapper's).
    pub instances: usize,
    pub faces: usize,
    pub vertices: usize,
    /// Faces that made no face: no outer loop with three distinct corners in range, no area,
    /// or the same corners as an earlier face of the mesh.
    pub skipped: usize,
    /// Faces that could not be made as given (bent, sides crossing, or holes that do not fit)
    /// and were made from their outer loop alone, cut into triangles where needed.
    pub split: usize,
    /// Holes dropped from faces made from their outer loop alone (counted in `split`).
    pub holes_dropped: usize,
    /// Edges marked soft that stay hard because they are not between exactly two faces.
    pub hard: usize,
    /// Edges with an end out of range or both ends welded into one point.
    pub skipped_edges: usize,
    /// Guides that are not finite or have no direction.
    pub skipped_guides: usize,
    /// Definitions no instance reaches from the root: not built.
    pub unused: usize,
    /// Dimensions made.
    pub dimensions: usize,
    /// Notes made.
    pub notes: usize,
    /// Dimensions and notes the mesh refused (a dimension of no length, a note text that is
    /// not valid).
    pub skipped_annotations: usize,
}

impl Scene {
    /// Builds every definition of `tree` that the root reaches into a new definition (in the
    /// tree's order), then the root geometry into one new definition named `name` of `kind`,
    /// placed once with the identity in mesh `parent`. Fails with `InvalidImport` for a
    /// malformed tree (see `check_tree`), `NotFinite` for a point that is not finite,
    /// `MaterialNotFound` for a material the scene lacks, `TagNotFound` for a face, edge,
    /// instance or guide on a tag the scene lacks, `TooManyFaces` past
    /// `MAX_IMPORT_FACES` and `NothingToImport` when the root makes nothing; nothing changes
    /// then.
    pub fn import_tree(
        &mut self,
        parent: MeshId,
        tree: &ImportTree,
        kind: DefinitionKind,
        name: &str,
    ) -> Result<(InstanceId, TreeReport, SceneChange), KernelError> {
        let (placed, report, change) = self.build_tree(parent, tree, Some((kind, name)))?;
        Ok((placed.expect("wrapper placed"), report, change))
    }

    /// As `import_tree`, but the root geometry goes straight into mesh `parent`, which must be
    /// empty (no vertices, instances or guides; else `InvalidImport`). A root that makes
    /// nothing is no error.
    pub fn import_tree_into_root(
        &mut self,
        parent: MeshId,
        tree: &ImportTree,
    ) -> Result<(TreeReport, SceneChange), KernelError> {
        let m = self.mesh(parent).ok_or(KernelError::DefinitionNotFound)?;
        if !m.is_empty() || m.guides().next().is_some() {
            return Err(KernelError::InvalidImport(
                "the mesh to import into is not empty".into(),
            ));
        }
        let (_, report, change) = self.build_tree(parent, tree, None)?;
        Ok((report, change))
    }

    /// Checks the tree, shapes every mesh and only then changes the scene, so a failure
    /// changes nothing. With `wrapper`, the root geometry goes into a new definition placed
    /// in `parent`; without, into `parent` itself.
    fn build_tree(
        &mut self,
        parent: MeshId,
        tree: &ImportTree,
        wrapper: Option<(DefinitionKind, &str)>,
    ) -> Result<(Option<InstanceId>, TreeReport, SceneChange), KernelError> {
        if self.mesh(parent).is_none() {
            return Err(KernelError::DefinitionNotFound);
        }
        let reached = check_tree(tree)?;
        let geometries: Vec<&ImportGeometry> = reached
            .iter()
            .map(|&i| &tree.definitions[i].geometry)
            .chain([&tree.root])
            .collect();
        let points = || geometries.iter().flat_map(|g| &g.points);
        if points().any(|p| !p.is_finite()) {
            return Err(KernelError::NotFinite);
        }
        if geometries.iter().any(|g| {
            g.dimensions
                .iter()
                .any(|d| !(d.a.is_finite() && d.b.is_finite() && d.offset.is_finite()))
                || g.notes
                    .iter()
                    .any(|n| !(n.anchor.is_finite() && n.position.is_finite()))
        }) {
            return Err(KernelError::NotFinite);
        }
        let missing = |m: &Option<MaterialId>| m.is_some_and(|m| self.material(m).is_none());
        if geometries.iter().any(|g| {
            g.faces
                .iter()
                .any(|f| missing(&f.front) || missing(&f.back))
                || g.instances.iter().any(|i| missing(&i.material))
        }) {
            return Err(KernelError::MaterialNotFound);
        }
        // A tag the scene lacks would leave an entity on a dangling tag.
        let lacks = |t: Option<TagId>| t.is_some_and(|t| self.tag(t).is_none());
        if geometries.iter().any(|g| {
            g.faces.iter().any(|f| lacks(f.tag))
                || g.edges.iter().any(|e| lacks(e.tag))
                || g.instances.iter().any(|i| lacks(i.tag))
                || g.guides.iter().any(|g| lacks(g.tag()))
                || g.dimensions.iter().any(|d| lacks(d.tag))
                || g.notes.iter().any(|n| lacks(n.tag))
        }) {
            return Err(KernelError::TagNotFound);
        }
        let given: usize = geometries.iter().map(|g| g.faces.len()).sum();
        if given > MAX_IMPORT_FACES {
            return Err(KernelError::TooManyFaces);
        }
        let mut report = TreeReport {
            unused: tree.definitions.len() - reached.len(),
            ..TreeReport::default()
        };
        let mut budget = MAX_RING_CHECKS;
        let mut made = 0;
        let shaped = geometries
            .iter()
            .map(|g| shape(g, &mut budget, &mut made, &mut report))
            .collect::<Result<Vec<Shaped>, KernelError>>()?;
        let root = shaped.last().expect("the root is shaped");
        if wrapper.is_some()
            && root.faces.is_empty()
            && root.edges.is_empty()
            && root.guides.is_empty()
            && root.instances.is_empty()
        {
            return Err(KernelError::NothingToImport);
        }

        let mut changes = Vec::new();
        let mut ours: Vec<Option<DefinitionId>> = vec![None; tree.definitions.len()];
        for &i in &reached {
            let d = &tree.definitions[i];
            let (id, step) = self.add_definition(&d.name, d.kind);
            ours[i] = Some(id);
            changes.push(SceneChange::from(step));
        }
        report.definitions = reached.len();
        let target = match wrapper {
            Some((kind, name)) => {
                let (id, step) = self.add_definition(name, kind);
                changes.push(SceneChange::from(step));
                MeshId::Definition(id)
            }
            None => parent,
        };
        let meshes = reached
            .iter()
            .map(|&i| MeshId::Definition(ours[i].expect("definition made")))
            .chain([target]);
        for (mesh, s) in meshes.zip(&shaped) {
            let filled = self.edit(mesh, |m| Ok(m.add_shaped(s, &ours, &mut report)))?;
            changes.push(filled);
        }
        let mut placed = None;
        if let MeshId::Definition(id) = target
            && wrapper.is_some()
        {
            changes.push(self.edit(parent, |m| {
                placed = Some(m.insert_instance(Instance::new(id, DAffine3::IDENTITY)));
                Ok(m.take_changes())
            })?);
        }
        Ok((placed, report, SceneChange::join(changes)))
    }
}

/// Checks the tree's structure and returns the definitions the root reaches, in order:
/// `InvalidImport` for more than `MAX_IMPORT_DEFINITIONS` definitions or
/// `MAX_IMPORT_INSTANCES` instances, an instance of a definition out of range, a definition
/// that places itself (at any depth), nesting deeper than `MAX_IMPORT_DEPTH`, or a reached
/// instance whose transform is not finite or is flat (`|det| < MIN_DETERMINANT`).
fn check_tree(tree: &ImportTree) -> Result<Vec<usize>, KernelError> {
    let invalid = |why: String| Err(KernelError::InvalidImport(why));
    let n = tree.definitions.len();
    if n > MAX_IMPORT_DEFINITIONS {
        return invalid(format!(
            "the file has {n} definitions, more than {MAX_IMPORT_DEFINITIONS}"
        ));
    }
    let children = |d: Option<usize>| match d {
        Some(d) => &tree.definitions[d].geometry.instances,
        None => &tree.root.instances,
    };
    let instances: usize = std::iter::once(None)
        .chain((0..n).map(Some))
        .map(|d| children(d).len())
        .sum();
    if instances > MAX_IMPORT_INSTANCES {
        return invalid(format!(
            "the file has {instances} instances, more than {MAX_IMPORT_INSTANCES}"
        ));
    }
    let annotations: usize = std::iter::once(&tree.root)
        .chain(tree.definitions.iter().map(|d| &d.geometry))
        .map(|g| g.dimensions.len() + g.notes.len())
        .sum();
    if annotations > MAX_IMPORT_ANNOTATIONS {
        return invalid(format!(
            "the file has {annotations} dimensions and notes, more than {MAX_IMPORT_ANNOTATIONS}"
        ));
    }
    for d in std::iter::once(None).chain((0..n).map(Some)) {
        if let Some(i) = children(d).iter().find(|i| i.definition >= n) {
            let owner = d.map_or("the root".to_string(), |d| format!("definition {d}"));
            return invalid(format!(
                "{owner} places definition {}, which does not exist",
                i.definition
            ));
        }
    }
    // A depth-first search from the root's instances. `height[d]`: the most levels of nesting
    // from d down (1 for a definition placing nothing), once d is done.
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Unseen,
        Searching,
        Done,
    }
    let mut state = vec![State::Unseen; n];
    let mut height = vec![0; n];
    // Once d is done: the placements resolved inside one placement of d, and the faces one
    // placement of d shows (saturating).
    let mut placed = vec![0u64; n];
    let mut shown = vec![0u64; n];
    let totals = |instances: &[ImportInstance], faces: usize, placed: &[u64], shown: &[u64]| {
        instances.iter().fold((0u64, faces as u64), |(p, f), i| {
            (
                p.saturating_add(1).saturating_add(placed[i.definition]),
                f.saturating_add(shown[i.definition]),
            )
        })
    };
    let deep = || {
        invalid(format!(
            "instances are nested more than {MAX_IMPORT_DEPTH} deep"
        ))
    };
    for start in &tree.root.instances {
        let start = start.definition;
        if state[start] == State::Done {
            continue;
        }
        state[start] = State::Searching;
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(top) = stack.last_mut() {
            let d = top.0;
            let children_of_d = children(Some(d));
            if let Some(child) = children_of_d.get(top.1).map(|i| i.definition) {
                top.1 += 1;
                match state[child] {
                    State::Searching => {
                        return invalid(format!("definition {child} places itself"));
                    }
                    State::Done => {
                        if stack.len() + height[child] > MAX_IMPORT_DEPTH {
                            return deep();
                        }
                    }
                    State::Unseen => {
                        if stack.len() + 1 > MAX_IMPORT_DEPTH {
                            return deep();
                        }
                        state[child] = State::Searching;
                        stack.push((child, 0));
                    }
                }
            } else {
                height[d] = 1 + children_of_d
                    .iter()
                    .map(|i| height[i.definition])
                    .max()
                    .unwrap_or(0);
                (placed[d], shown[d]) = totals(
                    children_of_d,
                    tree.definitions[d].geometry.faces.len(),
                    &placed,
                    &shown,
                );
                state[d] = State::Done;
                stack.pop();
            }
        }
    }
    let (placements, faces) = totals(&tree.root.instances, tree.root.faces.len(), &placed, &shown);
    if placements > MAX_IMPORT_PLACEMENTS {
        return invalid(format!(
            "the file's instances resolve to more than {MAX_IMPORT_PLACEMENTS} placements \
             through their nesting"
        ));
    }
    if faces > MAX_IMPORT_PLACED_FACES {
        return invalid(format!(
            "the file shows more than {MAX_IMPORT_PLACED_FACES} faces with every placement \
             counted"
        ));
    }
    let reached: Vec<usize> = (0..n).filter(|&d| state[d] == State::Done).collect();
    for d in reached.iter().map(|&d| Some(d)).chain([None]) {
        if let Some(i) = children(d).iter().find(|i| {
            !i.transform.is_finite() || i.transform.matrix3.determinant().abs() < MIN_DETERMINANT
        }) {
            return invalid(format!(
                "an instance of definition {} has a transform that is not finite or is flat",
                i.definition
            ));
        }
    }
    Ok(reached)
}

/// One geometry ready to insert: corners index `points` (welded positions).
struct Shaped<'a> {
    points: Vec<DVec3>,
    faces: Vec<ShapedFace>,
    /// Each edge once: face sides first, then the file's edges.
    edges: Vec<ShapedEdge>,
    guides: Vec<Guide>,
    instances: &'a [ImportInstance],
    dimensions: &'a [ImportDimension],
    notes: &'a [ImportNote],
}

struct ShapedFace {
    /// The outer loop (counter-clockwise around `normal`), then holes (clockwise).
    loops: Vec<Vec<usize>>,
    normal: DVec3,
    front: Option<MaterialId>,
    back: Option<MaterialId>,
    texture: Option<TexturePlacement>,
    tag: Option<TagId>,
    hidden: bool,
}

/// An edge between two welded points: soft when the file marks it so, on the tag of the
/// file's edge between them (a face side the file has no edge for is Untagged).
struct ShapedEdge {
    a: usize,
    b: usize,
    soft: bool,
    tag: Option<TagId>,
    hidden: bool,
}

/// Welds a geometry's points and makes its faces, edges and guides, counting what is
/// skipped or split in `report`. `made` counts faces over the whole import (`TooManyFaces`
/// past `MAX_IMPORT_FACES`); `budget` is what is left of `MAX_RING_CHECKS`.
fn shape<'a>(
    g: &'a ImportGeometry,
    budget: &mut usize,
    made: &mut usize,
    report: &mut TreeReport,
) -> Result<Shaped<'a>, KernelError> {
    let (points, map) = weld(&g.points);
    let mut faces = Vec::new();
    let mut seen: HashSet<Vec<usize>> = HashSet::new();
    for f in &g.faces {
        let Some(outer) = f.loops.first().and_then(|l| clean(l, &map)) else {
            report.skipped += 1;
            continue;
        };
        let holes: Option<Vec<Vec<usize>>> = f.loops[1..].iter().map(|l| clean(l, &map)).collect();
        let whole = holes.and_then(|holes| whole_face(&points, outer.clone(), holes, budget));
        let split = whole.is_none();
        let cut: Vec<(Vec<Vec<usize>>, DVec3)> = match whole {
            Some(face) => vec![face],
            None => pieces(&points, outer)
                .into_iter()
                .map(|(l, n)| (vec![l], n))
                .collect(),
        };
        if *made + cut.len() > MAX_IMPORT_FACES {
            return Err(KernelError::TooManyFaces);
        }
        let before = faces.len();
        for (loops, normal) in cut {
            let mut key: Vec<usize> = loops.iter().flatten().copied().collect();
            key.sort_unstable();
            if seen.insert(key) {
                faces.push(ShapedFace {
                    loops,
                    normal,
                    front: f.front,
                    back: f.back,
                    texture: f.texture,
                    tag: f.tag,
                    hidden: f.hidden,
                });
            }
        }
        *made += faces.len() - before;
        if faces.len() == before {
            report.skipped += 1;
        } else if split {
            report.split += 1;
            report.holes_dropped += f.loops.len() - 1;
        }
    }
    let mut edges: Vec<ShapedEdge> = Vec::new();
    let mut index: HashMap<(usize, usize), usize> = HashMap::new();
    // The first of the file's edges between two points gives the edge its tag.
    let mut tagged: HashSet<usize> = HashSet::new();
    let mut add = |a: usize, b: usize, file: Option<&ImportEdge>| {
        let at = *index.entry((a.min(b), a.max(b))).or_insert_with(|| {
            edges.push(ShapedEdge {
                a,
                b,
                soft: false,
                tag: None,
                hidden: false,
            });
            edges.len() - 1
        });
        if let Some(e) = file {
            edges[at].soft |= e.soft;
            if tagged.insert(at) {
                edges[at].tag = e.tag;
                edges[at].hidden = e.hidden;
            }
        }
    };
    for l in faces.iter().flat_map(|f| &f.loops) {
        for i in 0..l.len() {
            add(l[i], l[(i + 1) % l.len()], None);
        }
    }
    for e in &g.edges {
        match (map.get(e.a), map.get(e.b)) {
            (Some(&a), Some(&b)) if a != b => add(a, b, Some(e)),
            _ => report.skipped_edges += 1,
        }
    }
    let guides: Vec<Guide> = g.guides.iter().copied().filter(guide_is_valid).collect();
    report.skipped_guides += g.guides.len() - guides.len();
    Ok(Shaped {
        points,
        faces,
        edges,
        guides,
        instances: &g.instances,
        dimensions: &g.dimensions,
        notes: &g.notes,
    })
}

/// A loop as welded point indices with consecutive repeats and a closing repeat dropped, or
/// None when a corner is out of range or fewer than three distinct corners remain or one
/// repeats.
fn clean(corners: &[usize], map: &[usize]) -> Option<Vec<usize>> {
    let mut l: Vec<usize> = corners
        .iter()
        .map(|&c| map.get(c).copied())
        .collect::<Option<_>>()?;
    l.dedup();
    while l.len() > 1 && l.first() == l.last() {
        l.pop();
    }
    let mut distinct = l.clone();
    distinct.sort_unstable();
    distinct.dedup();
    (l.len() >= 3 && distinct.len() == l.len()).then_some(l)
}

/// The face's loops oriented (the outer one counter-clockwise around its normal, from its
/// vector area; holes clockwise) and that normal, when every corner is within `FLAT_TOL` of
/// the outer loop's plane, no sides cross or touch and every hole lies inside. None when not,
/// or when checking would overrun `budget`.
fn whole_face(
    points: &[DVec3],
    outer: Vec<usize>,
    holes: Vec<Vec<usize>>,
    budget: &mut usize,
) -> Option<(Vec<Vec<usize>>, DVec3)> {
    let at = |l: &[usize]| -> Vec<DVec3> { l.iter().map(|&c| points[c]).collect() };
    let normal = normal_of(&at(&outer))?;
    let origin = points[outer[0]];
    let mut loops = vec![outer];
    loops.extend(holes);
    if loops
        .iter()
        .flatten()
        .any(|&c| (points[c] - origin).dot(normal).abs() > FLAT_TOL)
    {
        return None;
    }
    let corners: usize = loops.iter().map(Vec::len).sum();
    let cost = corners.saturating_mul(corners) / 2;
    *budget = budget.checked_sub(cost)?;
    for hole in &mut loops[1..] {
        let pts = at(hole);
        normal_of(&pts)?;
        if vector_area(&pts).dot(normal) > 0.0 {
            hole.reverse();
        }
    }
    let rings: Vec<Vec<DVec3>> = loops.iter().map(|l| at(l)).collect();
    rings_are_valid(&rings, normal).then_some((loops, normal))
}

/// True when `add_guide` takes the guide: finite, and a line has a direction.
fn guide_is_valid(g: &Guide) -> bool {
    match *g {
        Guide::Line {
            point, direction, ..
        } => point.is_finite() && direction.try_normalize().is_some(),
        Guide::Point { position, .. } => position.is_finite(),
    }
}

impl Model {
    /// Inserts a shaped geometry: its vertices (those used), edges, faces, soft flags, guides
    /// and instances (of `ours[i]` for the tree's definition i), as one ChangeSet, counting
    /// in `report`.
    fn add_shaped(
        &mut self,
        s: &Shaped,
        ours: &[Option<DefinitionId>],
        report: &mut TreeReport,
    ) -> ChangeSet {
        let mut ops = Vec::new();
        let mut vertex: Vec<Option<VertexId>> = vec![None; s.points.len()];
        let mut vertex_of = |m: &mut Model, p: usize| {
            *vertex[p].get_or_insert_with(|| {
                report.vertices += 1;
                m.insert_vertex(s.points[p])
            })
        };
        let mut soft: Vec<EdgeId> = Vec::new();
        for e in &s.edges {
            let (va, vb) = (vertex_of(self, e.a), vertex_of(self, e.b));
            let id = self.insert_edge_like(
                va,
                vb,
                &Edge {
                    tag: e.tag,
                    hidden: e.hidden,
                    ..Edge::new(va, vb)
                },
            );
            if e.soft {
                soft.push(id);
            }
        }
        for f in &s.faces {
            let mut loops: Vec<Vec<VertexId>> = f
                .loops
                .iter()
                .map(|l| l.iter().map(|&p| vertex_of(self, p)).collect())
                .collect();
            let outer = loops.remove(0);
            let mut face = Face::new(outer, loops, f.normal);
            face.front = f.front;
            face.back = f.back;
            face.texture = f.texture;
            face.tag = f.tag;
            face.hidden = f.hidden;
            self.insert_face(face);
        }
        report.faces += s.faces.len();
        ops.extend(self.take_changes().ops);
        let (softened, r) = self.soften_edges(&soft, true);
        ops.extend(softened.ops);
        report.hard += r.skipped;
        for g in &s.guides {
            match self.add_guide(*g) {
                Ok(c) => ops.extend(c.ops),
                Err(_) => report.skipped_guides += 1,
            }
        }
        for i in s.instances {
            self.insert_instance(Instance {
                definition: ours[i.definition].expect("a reached definition is made"),
                transform: i.transform,
                material: i.material,
                tag: i.tag,
                hidden: i.hidden,
                section_cut: true,
            });
            report.instances += 1;
        }
        ops.extend(self.take_changes().ops);
        ops.extend(self.add_annotations(s, report).ops);
        ChangeSet { ops }
    }
}

/// The grid cell of a point, `LENGTH_TOL` on a side: a point's vertex within that distance is
/// in its cell or a neighbour.
fn cell(p: DVec3) -> [i64; 3] {
    let c = |x: f64| (x / LENGTH_TOL).floor() as i64;
    [c(p.x), c(p.y), c(p.z)]
}

impl Model {
    /// The dimensions and notes of `s`, anchored to the vertex at an end when there is one
    /// within `LENGTH_TOL`, else fixed there. One the mesh refuses is counted.
    fn add_annotations(&mut self, s: &Shaped, report: &mut TreeReport) -> ChangeSet {
        let mut ops = Vec::new();
        if s.dimensions.is_empty() && s.notes.is_empty() {
            return ChangeSet { ops };
        }
        let mut grid: HashMap<[i64; 3], Vec<(VertexId, DVec3)>> = HashMap::new();
        for (id, v) in self.vertices() {
            grid.entry(cell(v.pos)).or_default().push((id, v.pos));
        }
        let anchor = |p: DVec3| -> Anchor {
            let c = cell(p);
            let mut best: Option<(f64, VertexId)> = None;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let key = [c[0] + dx, c[1] + dy, c[2] + dz];
                        for (id, q) in grid.get(&key).into_iter().flatten() {
                            let d = q.distance(p);
                            if d < LENGTH_TOL && best.is_none_or(|(b, _)| d < b) {
                                best = Some((d, *id));
                            }
                        }
                    }
                }
            }
            best.map_or(Anchor::Point(p), |(_, id)| Anchor::Vertex(id))
        };
        for d in s.dimensions {
            let dimension = Dimension::Linear {
                a: anchor(d.a),
                b: anchor(d.b),
                offset: d.offset,
                tag: d.tag,
            };
            match self.add_dimension(dimension) {
                Ok(c) => {
                    ops.extend(c.ops);
                    report.dimensions += 1;
                }
                Err(_) => report.skipped_annotations += 1,
            }
        }
        for n in s.notes {
            let note = Note::Leader {
                anchor: anchor(n.anchor),
                position: n.position,
                text: n.text.clone(),
                tag: n.tag,
            };
            match self.add_note(note) {
                Ok(c) => {
                    ops.extend(c.ops);
                    report.notes += 1;
                }
                Err(_) => report.skipped_annotations += 1,
            }
        }
        ChangeSet { ops }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square with a square hole, flat and valid.
    fn holed() -> ImportGeometry {
        let p = |x: f64, y: f64| DVec3::new(x, y, 0.0);
        ImportGeometry {
            points: vec![
                p(0.0, 0.0),
                p(2.0, 0.0),
                p(2.0, 2.0),
                p(0.0, 2.0),
                p(0.5, 0.5),
                p(1.5, 0.5),
                p(1.5, 1.5),
                p(0.5, 1.5),
            ],
            faces: vec![ImportFace {
                loops: vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]],
                front: None,
                back: None,
                texture: None,
                tag: None,
                hidden: false,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn a_face_past_the_ring_check_budget_is_made_from_its_outer_loop() {
        let g = holed();
        let mut report = TreeReport::default();
        let mut budget = MAX_RING_CHECKS;
        let shaped = shape(&g, &mut budget, &mut 0, &mut report).unwrap();
        assert_eq!(shaped.faces.len(), 1);
        assert_eq!(shaped.faces[0].loops.len(), 2);
        assert_eq!((report.split, report.holes_dropped), (0, 0));

        // 8 corners cost 32 checks: a budget of 31 is not enough.
        let mut report = TreeReport::default();
        let shaped = shape(&g, &mut 31, &mut 0, &mut report).unwrap();
        assert_eq!(shaped.faces.len(), 1);
        assert_eq!(shaped.faces[0].loops.len(), 1);
        assert_eq!((report.split, report.holes_dropped), (1, 1));
        assert_eq!(report.skipped, 0);
    }
}
