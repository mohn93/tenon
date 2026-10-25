//! Building a mesh straight from polygons read from a file (OBJ, STL): points closer than
//! `LENGTH_TOL` are welded, each polygon becomes a face as it is wound, and neighbouring faces
//! in one plane can merge into one. No sticky geometry: nothing is split at crossings and no
//! face is added that the file does not have.

use std::collections::{BTreeMap, HashMap, HashSet};

use glam::{DAffine3, DVec2, DVec3};

use crate::edit::KernelError;
use crate::geom::{LENGTH_TOL, Plane, signed_area2, vector_area};
use crate::ids::{InstanceId, MaterialId, VertexId};
use crate::import_tree::{ImportEdge, ImportFace, ImportGeometry};
use crate::model::{Face, Instance, Model};
use crate::regions::PlaneGraph;
use crate::scene::{DefinitionKind, MeshId, Scene, SceneChange};
use crate::texture::{TexturePlacement, check_placement};

/// A polygon of an imported mesh: its corners as indices into the mesh's points, wound
/// counter-clockwise around its front, and the material of its front side.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshPolygon {
    pub corners: Vec<usize>,
    pub material: Option<MaterialId>,
}

/// What building a mesh from polygons made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshReport {
    /// Vertices made: the points faces use, coincident points welded into one.
    pub vertices: usize,
    /// Faces made.
    pub faces: usize,
    /// Polygons that made no face: fewer than three distinct corners after welding, no area,
    /// a corner index out of range, or the same corners as an earlier polygon.
    pub skipped: usize,
}

/// Most faces one import may make (before merging): a bent polygon is cut into a triangle
/// per corner, so a few polygons could otherwise make far more faces than the file has.
pub const MAX_IMPORT_FACES: usize = 200_000;

/// Corners within this distance of a polygon's plane count as on it. Half the kernel's
/// tolerance, so the faces made pass `Model::validate` with room for rounding.
pub(crate) const FLAT_TOL: f64 = 0.5 * LENGTH_TOL;

/// Most corners of a flat polygon that are checked for crossing sides.
const MAX_SIMPLE_CHECK: usize = 256;

impl Scene {
    /// Builds a new definition from `points` and `polygons` (see [`build_mesh`]) and places
    /// one instance of it, with an identity transform, in mesh `parent`. With `merge`, faces
    /// that share an edge, lie in one plane, face the same way and have the same material
    /// become one face (interior edges and the vertices only they used go). Fails with
    /// `NotFinite` for a point that is not finite, `MaterialNotFound` for a material the scene
    /// lacks and `NothingToImport` when no polygon makes a face; nothing changes then.
    pub fn import_mesh(
        &mut self,
        parent: MeshId,
        points: &[DVec3],
        polygons: &[MeshPolygon],
        merge: bool,
        kind: DefinitionKind,
        name: &str,
    ) -> Result<(InstanceId, MeshReport, SceneChange), KernelError> {
        if self.mesh(parent).is_none() {
            return Err(KernelError::DefinitionNotFound);
        }
        if points.iter().any(|p| !p.is_finite()) {
            return Err(KernelError::NotFinite);
        }
        if polygons
            .iter()
            .any(|p| p.material.is_some_and(|m| self.material(m).is_none()))
        {
            return Err(KernelError::MaterialNotFound);
        }
        let built = build_mesh(points, polygons, merge)?;
        if built.faces.is_empty() {
            return Err(KernelError::NothingToImport);
        }
        let (definition, added) = self.add_definition(name, kind);
        let mut vertices = 0;
        let filled = self.edit(MeshId::Definition(definition), |m| {
            vertices = m.add_built(&built);
            Ok(m.take_changes())
        })?;
        let mut placed = None;
        let left = self.edit(parent, |m| {
            placed = Some(m.insert_instance(Instance::new(definition, DAffine3::IDENTITY)));
            Ok(m.take_changes())
        })?;
        let report = MeshReport {
            vertices,
            faces: built.faces.len(),
            skipped: built.skipped,
        };
        let change = SceneChange::join([SceneChange::from(added), filled, left]);
        Ok((placed.expect("instance placed"), report, change))
    }
}

impl Model {
    /// Inserts the built faces with their vertices and edges; returns how many vertices.
    fn add_built(&mut self, built: &Built) -> usize {
        let mut vertex: HashMap<usize, VertexId> = HashMap::new();
        let mut edges: HashSet<(usize, usize)> = HashSet::new();
        for f in &built.faces {
            let mut loops: Vec<Vec<VertexId>> = Vec::new();
            for l in &f.loops {
                let ids = l
                    .iter()
                    .map(|&p| {
                        *vertex
                            .entry(p)
                            .or_insert_with(|| self.insert_vertex(built.points[p]))
                    })
                    .collect();
                for i in 0..l.len() {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    if edges.insert((a.min(b), a.max(b))) {
                        self.insert_edge(vertex[&a], vertex[&b]);
                    }
                }
                loops.push(ids);
            }
            let outer = loops.remove(0);
            let mut face = Face::new(outer, loops, f.normal);
            face.front = f.material;
            face.texture = f.texture;
            self.insert_face(face);
        }
        vertex.len()
    }
}

/// Faces ready to insert: corner indices into `points` (welded positions).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Built {
    pub points: Vec<DVec3>,
    pub faces: Vec<BuiltFace>,
    pub skipped: usize,
    /// For each given point, the index of the welded point it became.
    pub map: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BuiltFace {
    /// The outer loop (counter-clockwise around `normal`), then holes (clockwise).
    pub loops: Vec<Vec<usize>>,
    pub normal: DVec3,
    pub material: Option<MaterialId>,
    pub texture: Option<TexturePlacement>,
    /// The polygon it was made from (of a merged face, the largest one's).
    pub source: usize,
}

/// Welds the points, turns each polygon into a face (a polygon that is not flat, or whose
/// sides cross, into triangles), drops the ones that make none, and with `merge` joins
/// neighbouring faces in one plane. Fails with `TooManyFaces` past `MAX_IMPORT_FACES` faces.
pub(crate) fn build_mesh(
    points: &[DVec3],
    polygons: &[MeshPolygon],
    merge: bool,
) -> Result<Built, KernelError> {
    build_placed(points, polygons, &[], merge)
}

/// `build_mesh` with a texture placement for each polygon (`textures` parallel to `polygons`,
/// or empty for none): faces with different placements do not merge.
fn build_placed(
    points: &[DVec3],
    polygons: &[MeshPolygon],
    textures: &[Option<TexturePlacement>],
    merge: bool,
) -> Result<Built, KernelError> {
    let (welded, map) = weld(points);
    let mut faces: Vec<BuiltFace> = Vec::new();
    let mut seen: HashSet<Vec<usize>> = HashSet::new();
    let mut skipped = 0;
    for (source, polygon) in polygons.iter().enumerate() {
        let Some(corners) = polygon
            .corners
            .iter()
            .map(|&c| map.get(c).copied())
            .collect::<Option<Vec<usize>>>()
        else {
            skipped += 1;
            continue;
        };
        let before = faces.len();
        let cut = pieces(&welded, corners);
        if faces.len() + cut.len() > MAX_IMPORT_FACES {
            return Err(KernelError::TooManyFaces);
        }
        for (l, normal) in cut {
            let mut key = l.clone();
            key.sort_unstable();
            if seen.insert(key) {
                faces.push(BuiltFace {
                    loops: vec![l],
                    normal,
                    material: polygon.material,
                    texture: textures.get(source).copied().flatten(),
                    source,
                });
            }
        }
        if faces.len() == before {
            skipped += 1;
        }
    }
    if merge {
        faces = merge_coplanar(&welded, faces);
    }
    Ok(Built {
        points: welded,
        faces,
        skipped,
        map,
    })
}

/// Positions with points closer than `LENGTH_TOL` to an earlier kept one dropped, and for
/// each point the index of the position it became. Kept positions are at least `LENGTH_TOL`
/// apart: a grid of `LENGTH_TOL` cells finds the near ones.
pub(crate) fn weld(points: &[DVec3]) -> (Vec<DVec3>, Vec<usize>) {
    let cell = |p: DVec3| (p / LENGTH_TOL).floor().to_array().map(|c| c as i64);
    let mut grid: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    let mut kept: Vec<DVec3> = Vec::new();
    let mut map = Vec::with_capacity(points.len());
    for &p in points {
        let [x, y, z] = cell(p);
        let mut near = None;
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let key = [
                        x.saturating_add(dx),
                        y.saturating_add(dy),
                        z.saturating_add(dz),
                    ];
                    if let Some(k) = grid
                        .get(&key)
                        .into_iter()
                        .flatten()
                        .find(|k| kept[**k].distance(p) < LENGTH_TOL)
                    {
                        near = Some(*k);
                        break 'search;
                    }
                }
            }
        }
        let k = near.unwrap_or_else(|| {
            kept.push(p);
            grid.entry([x, y, z]).or_default().push(kept.len() - 1);
            kept.len() - 1
        });
        map.push(k);
    }
    (kept, map)
}

/// The faces one polygon makes, as (loop, unit normal): itself when it is flat and simple,
/// else the triangles of its projection; none when it has no area.
pub(crate) fn pieces(points: &[DVec3], mut corners: Vec<usize>) -> Vec<(Vec<usize>, DVec3)> {
    corners.dedup();
    while corners.len() > 1 && corners.first() == corners.last() {
        corners.pop();
    }
    let mut distinct = corners.clone();
    distinct.sort_unstable();
    distinct.dedup();
    if corners.len() < 3 || distinct.len() != corners.len() {
        return Vec::new();
    }
    let pts: Vec<DVec3> = corners.iter().map(|&c| points[c]).collect();
    let Some(normal) = normal_of(&pts) else {
        return Vec::new();
    };
    let flat = pts
        .iter()
        .all(|p| (*p - pts[0]).dot(normal).abs() <= FLAT_TOL);
    let plane = Plane::through(pts[0], normal);
    let flat2: Vec<DVec2> = pts.iter().map(|p| plane.project(*p)).collect();
    // The simplicity check is quadratic in the corners, so a long polygon skips it and goes
    // to the triangulation, which cuts it into triangles even when it was simple.
    if flat && (pts.len() == 3 || (pts.len() <= MAX_SIMPLE_CHECK && is_simple(&flat2))) {
        return vec![(corners, normal)];
    }
    let coords: Vec<f64> = flat2.iter().flat_map(|q| [q.x, q.y]).collect();
    let Ok(indices) = earcutr::earcut(&coords, &[], 2) else {
        return Vec::new();
    };
    indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(|t| {
            let mut tri = vec![corners[t[0]], corners[t[1]], corners[t[2]]];
            let tp: Vec<DVec3> = tri.iter().map(|&c| points[c]).collect();
            let n = normal_of(&tp)?;
            if n.dot(normal) < 0.0 {
                tri.swap(1, 2);
                return Some((tri, -n));
            }
            Some((tri, n))
        })
        .collect()
}

/// The unit normal of a polygon by its vector area, or None when it is narrower than
/// `LENGTH_TOL` (twice its area over its longest side).
pub(crate) fn normal_of(pts: &[DVec3]) -> Option<DVec3> {
    let area = vector_area(pts);
    let longest = (0..pts.len())
        .map(|i| pts[i].distance(pts[(i + 1) % pts.len()]))
        .fold(0.0, f64::max);
    (2.0 * area.length() > LENGTH_TOL * longest).then(|| area.normalize())
}

/// True when no two sides of the closed 2D polygon cross or touch, apart from neighbours
/// meeting at their shared corner.
fn is_simple(poly: &[DVec2]) -> bool {
    let n = poly.len();
    for i in 0..n {
        for j in i + 1..n {
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            let (c, d) = (poly[j], poly[(j + 1) % n]);
            if segments_meet(a, b, c, d) {
                return false;
            }
        }
    }
    true
}

/// True when closed 2D segments a-b and c-d share a point.
fn segments_meet(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> bool {
    let side = |p: DVec2, q: DVec2, r: DVec2| (q - p).perp_dot(r - p);
    let (d1, d2) = (side(c, d, a), side(c, d, b));
    let (d3, d4) = (side(a, b, c), side(a, b, d));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    let on = |p: DVec2, q: DVec2, r: DVec2| crate::geom::segment_distance_2d(r, p, q) < LENGTH_TOL;
    on(c, d, a) || on(c, d, b) || on(a, b, c) || on(a, b, d)
}

/// Joins faces that share an edge used by no other face, lie in one plane, face the same way
/// and have the same material. A set of joined faces whose outline cannot be traced back
/// exactly (same area, every corner on the plane) stays as it was.
fn merge_coplanar(points: &[DVec3], faces: Vec<BuiltFace>) -> Vec<BuiltFace> {
    // Each side of each face, keyed by its ends in order: (face, forward).
    let mut users: HashMap<(usize, usize), Vec<(usize, bool)>> = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        let l = &f.loops[0];
        for k in 0..l.len() {
            let (a, b) = (l[k], l[(k + 1) % l.len()]);
            users
                .entry((a.min(b), a.max(b)))
                .or_default()
                .push((i, a < b));
        }
    }
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let on_plane = |f: &BuiltFace, g: &BuiltFace| {
        let o = points[f.loops[0][0]];
        g.loops[0]
            .iter()
            .all(|&c| (points[c] - o).dot(f.normal).abs() <= FLAT_TOL)
    };
    let mut keys: Vec<&(usize, usize)> = users.keys().collect();
    keys.sort_unstable();
    for key in keys {
        let [(f, df), (g, dg)] = users[key][..] else {
            continue;
        };
        let (ff, gg) = (&faces[f], &faces[g]);
        if df != dg
            && ff.material == gg.material
            && same_placement(ff.texture, gg.texture)
            && ff.normal.dot(gg.normal) > 0.0
            && on_plane(ff, gg)
            && on_plane(gg, ff)
        {
            let (rf, rg) = (root(&mut parent, f), root(&mut parent, g));
            if rf != rg {
                parent[rg.max(rf)] = rf.min(rg);
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..faces.len() {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut out = Vec::new();
    for members in groups.values() {
        if members.len() == 1 {
            out.push(faces[members[0]].clone());
            continue;
        }
        match merged(points, &faces, members, &users) {
            Some(joined) => out.extend(joined),
            None => out.extend(members.iter().map(|&i| faces[i].clone())),
        }
    }
    out
}

/// The faces covering the joined faces `members`, or None when they cannot be traced.
fn merged(
    points: &[DVec3],
    faces: &[BuiltFace],
    members: &[usize],
    users: &HashMap<(usize, usize), Vec<(usize, bool)>>,
) -> Option<Vec<BuiltFace>> {
    let area = |i: usize| {
        let pts: Vec<DVec3> = faces[i].loops[0].iter().map(|&c| points[c]).collect();
        vector_area(&pts).length()
    };
    let largest = *members
        .iter()
        .max_by(|a, b| area(**a).total_cmp(&area(**b)))?;
    let normal = faces[largest].normal;
    let plane = Plane::through(points[faces[largest].loops[0][0]], normal);
    let flipped = normal.dot(plane.normal) < 0.0;
    let inside: HashSet<usize> = members.iter().copied().collect();
    // A side is interior when exactly two faces use it and both are members.
    let mut boundary: Vec<[VertexId; 2]> = Vec::new();
    let mut pos: BTreeMap<VertexId, DVec2> = BTreeMap::new();
    let mut done: HashSet<(usize, usize)> = HashSet::new();
    for &i in members {
        let l = &faces[i].loops[0];
        for k in 0..l.len() {
            let (a, b) = (l[k], l[(k + 1) % l.len()]);
            let key = (a.min(b), a.max(b));
            let u = &users[&key];
            if u.len() == 2 && u.iter().all(|(f, _)| inside.contains(f)) {
                continue;
            }
            if done.insert(key) {
                boundary.push([VertexId(a as u64), VertexId(b as u64)]);
                for c in [a, b] {
                    pos.insert(VertexId(c as u64), plane.project(points[c]));
                }
            }
        }
    }
    let graph = PlaneGraph {
        pos,
        edges: boundary,
    };
    let total: f64 = members.iter().map(|&i| area(i)).sum();
    let mut covered = 0.0;
    let mut out = Vec::new();
    for r in graph.regions() {
        // Members wind counter-clockwise around `normal`, so a region is covered exactly when a
        // member runs along its first side in the same direction (the opposite one when the
        // plane's canonical normal points against `normal`).
        let (a, b) = (r.outer[0].0 as usize, r.outer[1].0 as usize);
        if !users[&(a.min(b), a.max(b))]
            .iter()
            .any(|(f, forward)| *forward == ((a < b) != flipped) && inside.contains(f))
        {
            continue;
        }
        covered += signed_area2(&graph.poly(&r.outer)) * 0.5;
        covered += r
            .holes
            .iter()
            .map(|h| signed_area2(&graph.poly(h)) * 0.5)
            .sum::<f64>();
        let index = |l: &Vec<VertexId>| -> Vec<usize> { l.iter().map(|v| v.0 as usize).collect() };
        let mut loops: Vec<Vec<usize>> = std::iter::once(&r.outer)
            .chain(&r.holes)
            .map(index)
            .collect();
        // Regions wind counter-clockwise around the plane's own normal.
        if flipped {
            loops.iter_mut().for_each(|l| l.reverse());
        }
        let o = points[loops[0][0]];
        if loops
            .iter()
            .flatten()
            .any(|&c| (points[c] - o).dot(normal).abs() > FLAT_TOL)
        {
            return None;
        }
        out.push(BuiltFace {
            loops,
            normal,
            material: faces[largest].material,
            texture: faces[largest].texture,
            source: faces[largest].source,
        });
    }
    ((covered - total).abs() <= 1e-9 * total.max(1.0)).then_some(out)
}

/// Most two texture placements may differ and still count as one (offsets in meters, turns
/// in degrees, scales as a fraction): neighbours one mapping covers differ only by rounding.
const PLACEMENT_TOL: f64 = 1e-4;

/// Most two unit normals given at a corner may differ and still count as one smooth normal.
const SMOOTH_TOL: f64 = 1e-3;

/// True when two faces' texture placements are the same, but for rounding.
fn same_placement(a: Option<TexturePlacement>, b: Option<TexturePlacement>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            let turn = (a.rotation_deg - b.rotation_deg).rem_euclid(360.0);
            (0..2).all(|i| {
                (a.offset[i] - b.offset[i]).abs() <= PLACEMENT_TOL
                    && (a.scale[i] - b.scale[i]).abs() <= PLACEMENT_TOL * a.scale[i].max(b.scale[i])
            }) && turn.min(360.0 - turn) <= PLACEMENT_TOL * 100.0
        }
        _ => false,
    }
}

/// A mesh read from a file as the faces and edges of an import tree geometry (see
/// [`Scene::import_tree`]): points are welded and polygons made into faces as
/// `import_mesh` does, and with `merge` neighbouring faces in one plane with the same material
/// and texture placement join into one. `textures` gives each polygon's texture placement
/// (parallel to `polygons`, or empty); a placement that is the default or out of range is
/// left off. `normals` gives the file's normal at each point (parallel to `points`, or empty
/// for a mesh shaded flat): an edge between two faces that stay apart and do not lie in one
/// plane is soft when both faces' normals at both its ends agree, so a smooth surface reads as
/// one. Returns the geometry (faces painted on the front only) and how many polygons made no
/// face. Fails with `TooManyFaces` past `MAX_IMPORT_FACES` faces.
pub fn polygon_geometry(
    points: &[DVec3],
    polygons: &[MeshPolygon],
    textures: &[Option<TexturePlacement>],
    normals: &[DVec3],
    merge: bool,
) -> Result<(ImportGeometry, usize), KernelError> {
    let textures: Vec<Option<TexturePlacement>> = textures
        .iter()
        .map(|t| t.filter(|t| !t.is_default() && check_placement(t).is_ok()))
        .collect();
    let built = build_placed(points, polygons, &textures, false)?;
    let mut soft: Vec<(usize, usize)> = Vec::new();
    if normals.len() == points.len() {
        // The file's normal at welded corner `c` of face `f`: that of the polygon's own point.
        let normal = |f: &BuiltFace, c: usize| {
            polygons[f.source]
                .corners
                .iter()
                .find(|&&k| built.map.get(k) == Some(&c))
                .map(|&k| normals[k].normalize_or_zero())
                .filter(|n| *n != DVec3::ZERO)
        };
        let mut users: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
        for (i, f) in built.faces.iter().enumerate() {
            let l = &f.loops[0];
            for k in 0..l.len() {
                let (a, b) = (l[k], l[(k + 1) % l.len()]);
                users.entry((a.min(b), a.max(b))).or_default().push(i);
            }
        }
        let mut keys: Vec<(&(usize, usize), &Vec<usize>)> = users.iter().collect();
        keys.sort_unstable();
        for (&(a, b), faces) in keys {
            let [f, g] = faces[..] else { continue };
            let (ff, gg) = (&built.faces[f], &built.faces[g]);
            if ff.normal.dot(gg.normal) >= 1.0 - 1e-9 {
                continue;
            }
            let agree = |c: usize| match (normal(ff, c), normal(gg, c)) {
                (Some(m), Some(n)) => m.distance(n) <= SMOOTH_TOL,
                _ => false,
            };
            if agree(a) && agree(b) {
                soft.push((a, b));
            }
        }
    }
    let faces = if merge {
        merge_coplanar(&built.points, built.faces)
    } else {
        built.faces
    };
    // A soft edge merging took away would come back as a loose edge.
    let mut sides: HashSet<(usize, usize)> = HashSet::new();
    for l in faces.iter().flat_map(|f| &f.loops) {
        for k in 0..l.len() {
            let (a, b) = (l[k], l[(k + 1) % l.len()]);
            sides.insert((a.min(b), a.max(b)));
        }
    }
    let edges = soft
        .into_iter()
        .filter(|e| sides.contains(e))
        .map(|(a, b)| ImportEdge {
            a,
            b,
            soft: true,
            tag: None,
            hidden: false,
        })
        .collect();
    let faces = faces
        .into_iter()
        .map(|f| ImportFace {
            loops: f.loops,
            front: f.material,
            back: None,
            texture: f.texture,
            tag: None,
            hidden: false,
        })
        .collect();
    Ok((
        ImportGeometry {
            points: built.points,
            faces,
            edges,
            ..ImportGeometry::default()
        },
        built.skipped,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn polys(list: &[&[usize]]) -> Vec<MeshPolygon> {
        list.iter()
            .map(|c| MeshPolygon {
                corners: c.to_vec(),
                material: None,
            })
            .collect()
    }

    fn import(points: &[DVec3], list: &[&[usize]], merge: bool) -> (Scene, MeshReport) {
        let mut s = Scene::new();
        let (_, report, _) = s
            .import_mesh(
                MeshId::Root,
                points,
                &polys(list),
                merge,
                DefinitionKind::Group,
                "",
            )
            .unwrap();
        s.validate().unwrap();
        (s, report)
    }

    /// The mesh of the only definition.
    fn mesh(s: &Scene) -> &Model {
        &s.definitions().next().unwrap().1.mesh
    }

    /// A unit cube: 8 corners, 12 triangles wound outward.
    fn cube() -> (Vec<DVec3>, Vec<[usize; 3]>) {
        let p = (0..8)
            .map(|i| DVec3::new((i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64))
            .collect();
        let quads = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        let tris = quads
            .iter()
            .flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]])
            .collect();
        (p, tris)
    }

    #[test]
    fn a_cube_of_triangles_merges_into_six_faces_facing_out() {
        let (p, tris) = cube();
        let list: Vec<&[usize]> = tris.iter().map(|t| &t[..]).collect();
        let (s, report) = import(&p, &list, true);
        let m = mesh(&s);
        assert_eq!(
            report,
            MeshReport {
                vertices: 8,
                faces: 6,
                skipped: 0
            }
        );
        assert_eq!((m.faces().count(), m.edges().count()), (6, 12));
        let centre = DVec3::splat(0.5);
        for (id, f) in m.faces() {
            let out = m.position(f.outer[0]).unwrap() - centre;
            assert!(f.normal.dot(out) > 0.0, "{id:?} faces in");
            assert_eq!(f.outer.len(), 4);
        }
        assert_eq!(s.root().instances().count(), 1);
    }

    #[test]
    fn without_merging_every_triangle_is_a_face() {
        let (p, tris) = cube();
        let list: Vec<&[usize]> = tris.iter().map(|t| &t[..]).collect();
        let (s, report) = import(&p, &list, false);
        assert_eq!(report.faces, 12);
        assert_eq!(mesh(&s).edges().count(), 18);
    }

    #[test]
    fn coincident_points_weld_and_degenerate_or_repeated_polygons_are_skipped() {
        let p = vec![
            DVec3::ZERO,
            DVec3::X,
            DVec3::Y,
            DVec3::new(1.0, 0.0, 0.0) + DVec3::splat(1e-8), // welds into X
            DVec3::new(2.0, 0.0, 0.0),                      // collinear with 0 and 1
        ];
        let list: &[&[usize]] = &[
            &[0, 1, 2],
            &[0, 3, 2], // the same corners once welded
            &[2, 1, 0], // the same corners, turned over
            &[0, 1, 4], // no area
            &[0, 1, 3], // two distinct corners
            &[0, 1, 9], // index out of range
        ];
        let (s, report) = import(&p, list, true);
        assert_eq!(
            report,
            MeshReport {
                vertices: 3,
                faces: 1,
                skipped: 5
            }
        );
        assert_eq!(mesh(&s).faces().next().unwrap().1.normal, DVec3::Z);
    }

    #[test]
    fn a_bent_quad_becomes_two_triangles_and_a_flat_one_stays_whole() {
        let p = vec![
            DVec3::ZERO,
            DVec3::X,
            DVec3::new(1.0, 1.0, 0.3),
            DVec3::Y,
            DVec3::new(0.0, 0.0, 1.0),
            DVec3::new(1.0, 0.0, 1.0),
        ];
        let (s, report) = import(&p, &[&[0, 1, 2, 3], &[0, 1, 5, 4]], false);
        assert_eq!(report.faces, 3);
        assert_eq!(
            mesh(&s).faces().filter(|(_, f)| f.outer.len() == 4).count(),
            1
        );
    }

    #[test]
    fn a_ring_of_flat_quads_merges_into_a_face_with_a_hole() {
        // A 3 x 3 square with the middle square left open, as 8 unit quads.
        let p: Vec<DVec3> = (0..16)
            .map(|i| DVec3::new((i % 4) as f64, (i / 4) as f64, 0.0))
            .collect();
        let at = |x: usize, y: usize| y * 4 + x;
        let mut quads = Vec::new();
        for y in 0..3 {
            for x in 0..3 {
                if (x, y) != (1, 1) {
                    quads.push(vec![at(x, y), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1)]);
                }
            }
        }
        let list: Vec<&[usize]> = quads.iter().map(|q| &q[..]).collect();
        let (s, report) = import(&p, &list, true);
        assert_eq!(report.faces, 1);
        let m = mesh(&s);
        let (id, f) = m.faces().next().unwrap();
        assert_eq!(f.holes.len(), 1);
        assert!((m.area(id).unwrap() - 8.0).abs() < 1e-12);
        // Vertices along the outline stay: 12 outside, 4 around the hole.
        assert_eq!(report.vertices, 16);
    }

    #[test]
    fn faces_with_different_materials_or_facing_apart_do_not_merge() {
        let mut s = Scene::new();
        let (red, _) = s.add_material("Red", [255, 0, 0]).unwrap();
        let p = vec![DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
        let polygons = vec![
            MeshPolygon {
                corners: vec![0, 1, 2],
                material: Some(red),
            },
            MeshPolygon {
                corners: vec![0, 2, 3],
                material: None,
            },
        ];
        let (_, report, _) = s
            .import_mesh(MeshId::Root, &p, &polygons, true, DefinitionKind::Group, "")
            .unwrap();
        assert_eq!(report.faces, 2);
        s.validate().unwrap();
        let fronts: Vec<_> = mesh(&s).faces().map(|(_, f)| f.front).collect();
        assert_eq!(fronts, vec![Some(red), None]);
        // Two triangles wound against each other share the diagonal the same way round.
        let (_, report) = import(&p, &[&[0, 1, 2], &[0, 3, 2]], true);
        assert_eq!(report.faces, 2);
    }

    #[test]
    fn nothing_to_import_and_bad_input_change_nothing() {
        let mut s = Scene::new();
        let p = [DVec3::ZERO, DVec3::X, DVec3::X * 2.0];
        let r = s.import_mesh(
            MeshId::Root,
            &p,
            &polys(&[&[0, 1, 2]]),
            true,
            DefinitionKind::Group,
            "",
        );
        assert_eq!(r.unwrap_err(), KernelError::NothingToImport);
        let bad = [DVec3::ZERO, DVec3::X, DVec3::new(f64::NAN, 0.0, 0.0)];
        let r = s.import_mesh(
            MeshId::Root,
            &bad,
            &polys(&[&[0, 1, 2]]),
            true,
            DefinitionKind::Group,
            "",
        );
        assert_eq!(r.unwrap_err(), KernelError::NotFinite);
        let missing = vec![MeshPolygon {
            corners: vec![0, 1, 2],
            material: Some(MaterialId(99)),
        }];
        let p = [DVec3::ZERO, DVec3::X, DVec3::Y];
        let r = s.import_mesh(MeshId::Root, &p, &missing, true, DefinitionKind::Group, "");
        assert_eq!(r.unwrap_err(), KernelError::MaterialNotFound);
        assert_eq!(s, Scene::new());
    }

    #[test]
    fn undoing_an_import_removes_the_definition_and_its_instance() {
        let (p, tris) = cube();
        let list: Vec<&[usize]> = tris.iter().map(|t| &t[..]).collect();
        let mut s = Scene::new();
        let (instance, _, change) = s
            .import_mesh(
                MeshId::Root,
                &p,
                &polys(&list),
                true,
                DefinitionKind::Component,
                "Cube",
            )
            .unwrap();
        assert_eq!(s.definitions().next().unwrap().1.name, "Cube");
        assert!(s.root().instance(instance).is_some());
        s.apply(&change.inverse());
        assert_eq!(s, Scene::new());
    }

    #[test]
    fn one_bent_polygon_cannot_make_more_faces_than_the_limit() {
        // A ring of corners alternately a little above and below its plane: bent, so it is
        // cut into one triangle per corner minus two.
        let n = MAX_IMPORT_FACES + 3;
        let p: Vec<DVec3> = (0..n)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                let z = if i % 2 == 0 { 0.01 } else { -0.01 };
                DVec3::new(1000.0 * a.cos(), 1000.0 * a.sin(), z)
            })
            .collect();
        let ring: Vec<usize> = (0..n).collect();
        let r = build_mesh(&p, &polys(&[&ring]), false);
        assert_eq!(r.unwrap_err(), KernelError::TooManyFaces);
    }

    #[test]
    fn a_self_crossing_flat_polygon_is_cut_into_triangles() {
        // Side 2-3 crosses side 0-1.
        let p = vec![
            DVec3::ZERO,
            DVec3::new(4.0, 0.0, 0.0),
            DVec3::new(4.0, 3.0, 0.0),
            DVec3::new(2.0, -1.0, 0.0),
            DVec3::new(0.0, 3.0, 0.0),
        ];
        let built = build_mesh(&p, &polys(&[&[0, 1, 2, 3, 4]]), false).unwrap();
        assert!(!built.faces.is_empty());
        assert!(built.faces.iter().all(|f| f.loops[0].len() == 3));
    }

    /// An n x n sheet of unit cells in the z = 0 plane as triangles, leaving out the cells
    /// `open` names.
    fn sheet(n: usize, open: impl Fn(usize, usize) -> bool) -> (Vec<DVec3>, Vec<Vec<usize>>) {
        let p = (0..(n + 1) * (n + 1))
            .map(|i| DVec3::new((i % (n + 1)) as f64, (i / (n + 1)) as f64, 0.0))
            .collect();
        let at = |x: usize, y: usize| y * (n + 1) + x;
        let mut tris = Vec::new();
        for y in 0..n {
            for x in 0..n {
                if !open(x, y) {
                    let (a, b, c, d) = (at(x, y), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1));
                    tris.push(vec![a, b, c]);
                    tris.push(vec![a, c, d]);
                }
            }
        }
        (p, tris)
    }

    #[test]
    fn a_pinched_hole_stays_one_hole() {
        // Cells (1,1) and (2,2) are open and meet at one corner.
        let (p, tris) = sheet(3, |x, y| (x, y) == (1, 1) || (x, y) == (2, 2));
        let list: Vec<&[usize]> = tris.iter().map(|t| &t[..]).collect();
        let (s, report) = import(&p, &list, true);
        assert_eq!(report.faces, 1);
        let m = mesh(&s);
        let (id, f) = m.faces().next().unwrap();
        assert_eq!(f.holes.len(), 1);
        assert!((m.area(id).unwrap() - 7.0).abs() < 1e-12);
    }

    #[test]
    fn squares_touching_at_a_corner_stay_two_faces() {
        let p = vec![
            DVec3::ZERO,
            DVec3::X,
            DVec3::new(1.0, 1.0, 0.0),
            DVec3::Y,
            DVec3::new(2.0, 1.0, 0.0),
            DVec3::new(2.0, 2.0, 0.0),
            DVec3::new(1.0, 2.0, 0.0),
        ];
        let (s, report) = import(&p, &[&[0, 1, 2, 3], &[2, 4, 5, 6]], true);
        assert_eq!((report.faces, report.vertices), (2, 7));
        assert_eq!(mesh(&s).faces().count(), 2);
    }

    #[test]
    fn a_strip_drifting_off_its_plane_is_left_unmerged_and_valid() {
        // Each cell is flat and the next lies within tolerance of its plane, but twelve cells
        // together bend further than the tolerance allows one face.
        let strip = |c: f64| {
            let n = 12;
            let mut p = Vec::new();
            for i in 0..=n {
                let z = c * (i * i) as f64;
                p.push(DVec3::new(100.0 * i as f64, 0.0, z));
                p.push(DVec3::new(100.0 * i as f64, 1.0, z));
            }
            let quads: Vec<Vec<usize>> = (0..n)
                .map(|i| vec![2 * i, 2 * i + 2, 2 * i + 3, 2 * i + 1])
                .collect();
            (p, quads, n)
        };
        let (p, quads, n) = strip(2e-7);
        let list: Vec<&[usize]> = quads.iter().map(|q| &q[..]).collect();
        let (s, report) = import(&p, &list, true);
        assert_eq!(report.faces, n);
        s.validate().unwrap();
        // The same strip without the bend is one face.
        let (p, quads, _) = strip(0.0);
        let list: Vec<&[usize]> = quads.iter().map(|q| &q[..]).collect();
        let (_, report) = import(&p, &list, true);
        assert_eq!(report.faces, 1);
    }

    #[test]
    fn a_self_crossing_polygon_imports_as_valid_triangles() {
        let p = vec![
            DVec3::ZERO,
            DVec3::new(4.0, 0.0, 0.0),
            DVec3::new(4.0, 3.0, 0.0),
            DVec3::new(2.0, -1.0, 0.0),
            DVec3::new(0.0, 3.0, 0.0),
        ];
        let (s, report) = import(&p, &[&[0, 1, 2, 3, 4]], false);
        assert!(report.faces >= 1);
        assert!(mesh(&s).faces().all(|(_, f)| f.outer.len() == 3));
    }

    #[test]
    fn a_missing_parent_changes_nothing() {
        let mut s = Scene::new();
        let p = [DVec3::ZERO, DVec3::X, DVec3::Y];
        let r = s.import_mesh(
            MeshId::Definition(crate::ids::DefinitionId(77)),
            &p,
            &polys(&[&[0, 1, 2]]),
            true,
            DefinitionKind::Group,
            "",
        );
        assert_eq!(r.unwrap_err(), KernelError::DefinitionNotFound);
        assert_eq!(s, Scene::new());
    }

    #[test]
    fn a_sheet_with_sixteen_hundred_holes_merges_into_one_face() {
        // 81 x 81 cells, a cell open wherever both coordinates are odd: 40 x 40 holes.
        let (p, tris) = sheet(81, |x, y| x % 2 == 1 && y % 2 == 1);
        let list: Vec<&[usize]> = tris.iter().map(|t| &t[..]).collect();
        let start = web_time::Instant::now();
        let (s, report) = import(&p, &list, true);
        let took = start.elapsed();
        assert!(took.as_secs() < 10, "{took:?}");
        assert_eq!(report.faces, 1);
        assert_eq!(mesh(&s).faces().next().unwrap().1.holes.len(), 1600);
    }
}
