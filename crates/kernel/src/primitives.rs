//! Solid primitives: boxes, cylinders, tubes, cones and prisms built as closed meshes with
//! faces pointing outward, in a local frame (curved shapes about +Z with the base centre at the
//! origin, a box with its corner at the origin). A [`Solid`] is the shared description: points,
//! face loops, soft edges and curve rings. It goes into a fresh definition (`Scene::add_solid`)
//! or into an existing mesh with the usual merge rules (`Scene::draw_solid`).

use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{PI, TAU};
use std::fmt;

use glam::{DAffine3, DVec3};

use crate::edit::KernelError;
use crate::geom::{LENGTH_TOL, vector_area};
use crate::group::whole;
use crate::ids::{EdgeId, FaceId, InstanceId, MaterialId, VertexId};
use crate::model::{Edge, Face, Instance, Model};
use crate::scene::{DefinitionKind, MeshId, Scene, SceneChange};

/// Shortest length a primitive's dimensions and edges may have: ten times the merge tolerance.
pub const MIN_LENGTH: f64 = 10.0 * LENGTH_TOL;
/// Sides or segments a primitive gets when none are asked for.
pub const DEFAULT_SEGMENTS: u32 = 24;
/// Sides a prism gets when none are asked for.
pub const DEFAULT_SIDES: u32 = 6;
/// Fewest and most segments or sides around.
pub const MIN_SEGMENTS: u32 = 3;
pub const MAX_SEGMENTS: u32 = 360;

/// A refusal of a primitive's dimensions; the text names the field.
#[derive(Clone, Debug, PartialEq)]
pub struct PrimitiveError(pub String);

impl fmt::Display for PrimitiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PrimitiveError {}

fn invalid<T>(message: impl Into<String>) -> Result<T, PrimitiveError> {
    Err(PrimitiveError(message.into()))
}

/// A closed solid, described by index into `points`. Every face loop winds counter-clockwise
/// seen from outside the solid (a hole in either direction); every edge between two faces is
/// shared by exactly two of them.
#[derive(Clone, Debug, Default)]
pub struct Solid {
    points: Vec<DVec3>,
    faces: Vec<SolidFace>,
    /// Closed chains of edges that form one curve each, with their polygon flag.
    rings: Vec<(Vec<usize>, bool)>,
    /// Edges (as ordered index pairs, smaller first) that are soft.
    soft: BTreeSet<(usize, usize)>,
}

#[derive(Clone, Debug)]
struct SolidFace {
    outer: Vec<usize>,
    holes: Vec<Vec<usize>>,
}

fn pair(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

impl Solid {
    /// The points, in the solid's own frame.
    pub fn points(&self) -> &[DVec3] {
        &self.points
    }

    /// Adds a point and returns its index.
    fn point(&mut self, p: DVec3) -> usize {
        self.points.push(p);
        self.points.len() - 1
    }

    fn face(&mut self, outer: Vec<usize>) {
        self.faces.push(SolidFace {
            outer,
            holes: Vec::new(),
        });
    }

    fn face_with_hole(&mut self, outer: Vec<usize>, hole: Vec<usize>) {
        self.faces.push(SolidFace {
            outer,
            holes: vec![hole],
        });
    }

    fn ring(&mut self, points: Vec<usize>, polygon: bool) {
        self.rings.push((points, polygon));
    }

    fn soften(&mut self, a: usize, b: usize) {
        self.soft.insert(pair(a, b));
    }

    /// The solid moved by `t`.
    pub fn placed(&self, t: &DAffine3) -> Solid {
        let mut s = self.clone();
        for p in &mut s.points {
            *p = t.transform_point3(*p);
        }
        s
    }

    /// Adds the solid to `m` as vertices, edges and faces, without merging with anything in
    /// it. Returns the new faces and edges.
    pub(crate) fn insert(&self, m: &mut Model) -> (Vec<FaceId>, Vec<EdgeId>) {
        let vertices: Vec<VertexId> = self.points.iter().map(|p| m.insert_vertex(*p)).collect();
        let mut sides: BTreeSet<(usize, usize)> = BTreeSet::new();
        for f in &self.faces {
            for l in std::iter::once(&f.outer).chain(&f.holes) {
                for i in 0..l.len() {
                    sides.insert(pair(l[i], l[(i + 1) % l.len()]));
                }
            }
        }
        let mut curve_of: BTreeMap<(usize, usize), (crate::ids::CurveId, bool)> = BTreeMap::new();
        for (ring, polygon) in &self.rings {
            let id = m.new_curve();
            for i in 0..ring.len() {
                curve_of.insert(pair(ring[i], ring[(i + 1) % ring.len()]), (id, *polygon));
            }
        }
        let mut edges = BTreeMap::new();
        for &(a, b) in &sides {
            let mut like = Edge::new(vertices[a], vertices[b]);
            if let Some((curve, polygon)) = curve_of.get(&(a, b)) {
                like.curve = Some(*curve);
                like.polygon = *polygon;
            }
            like.soft = self.soft.contains(&(a, b));
            edges.insert((a, b), m.insert_edge_like(vertices[a], vertices[b], &like));
        }
        let ids = |l: &[usize]| l.iter().map(|i| vertices[*i]).collect::<Vec<_>>();
        let mut faces = Vec::new();
        for f in &self.faces {
            let ring: Vec<DVec3> = f.outer.iter().map(|i| self.points[*i]).collect();
            let area = vector_area(&ring);
            let normal = area.normalize();
            let holes = f
                .holes
                .iter()
                .map(|h| {
                    let pts: Vec<DVec3> = h.iter().map(|i| self.points[*i]).collect();
                    // Holes wind the other way around the normal.
                    let mut h = h.clone();
                    if vector_area(&pts).dot(normal) > 0.0 {
                        h.reverse();
                    }
                    ids(&h)
                })
                .collect();
            faces.push(m.insert_face(Face::new(ids(&f.outer), holes, normal)));
        }
        (faces, edges.into_values().collect())
    }

    /// A model holding just the solid, for inspection and tests.
    pub fn to_model(&self) -> Model {
        let mut m = Model::new();
        self.insert(&mut m);
        m.take_changes();
        m
    }
}

/// The points of a regular polygon of `n` corners with `radius` at height `z`, the first at
/// angle `first` from +X, counter-clockwise around +Z.
fn ring_points(radius: f64, z: f64, n: u32, first: f64) -> Vec<DVec3> {
    (0..n)
        .map(|k| {
            let a = first + TAU * k as f64 / n as f64;
            DVec3::new(radius * a.cos(), radius * a.sin(), z)
        })
        .collect()
}

fn check_segments(name: &str, n: u32) -> Result<(), PrimitiveError> {
    if (MIN_SEGMENTS..=MAX_SEGMENTS).contains(&n) {
        Ok(())
    } else {
        invalid(format!(
            "{name} must be between {MIN_SEGMENTS} and {MAX_SEGMENTS}"
        ))
    }
}

fn check_length(name: &str, v: f64) -> Result<(), PrimitiveError> {
    if v.is_finite() && v >= MIN_LENGTH {
        Ok(())
    } else {
        invalid(format!(
            "{name} must be a finite number of at least {MIN_LENGTH} m"
        ))
    }
}

/// A radius whose neighbouring corners (`n` around) stay farther apart than the merge tolerance.
fn check_radius(name: &str, r: f64, n: u32) -> Result<(), PrimitiveError> {
    check_length(name, r)?;
    if 2.0 * r * (PI / n as f64).sin() < MIN_LENGTH {
        return invalid(format!("{name} is too small for that many segments"));
    }
    Ok(())
}

impl Solid {
    /// A box with one corner at the origin, `size` along the axes.
    pub fn cuboid(size: DVec3) -> Result<Solid, PrimitiveError> {
        for (name, v) in [
            ("size[0]", size.x),
            ("size[1]", size.y),
            ("size[2]", size.z),
        ] {
            check_length(name, v)?;
        }
        let mut s = Solid::default();
        // Index bit 0 is x, bit 1 y, bit 2 z.
        for i in 0..8 {
            s.point(DVec3::new(
                if i & 1 != 0 { size.x } else { 0.0 },
                if i & 2 != 0 { size.y } else { 0.0 },
                if i & 4 != 0 { size.z } else { 0.0 },
            ));
        }
        for loop_ in [
            [0, 2, 3, 1], // -z
            [4, 5, 7, 6], // +z
            [0, 1, 5, 4], // -y
            [2, 6, 7, 3], // +y
            [0, 4, 6, 2], // -x
            [1, 3, 7, 5], // +x
        ] {
            s.face(loop_.to_vec());
        }
        Ok(s)
    }

    /// A cylinder about +Z from z = 0 to `height`, `segments` sides; the vertical edges are
    /// soft.
    pub fn cylinder(radius: f64, height: f64, segments: u32) -> Result<Solid, PrimitiveError> {
        check_segments("segments", segments)?;
        check_radius("radius", radius, segments)?;
        check_length("height", height)?;
        Ok(Self::prism_like(radius, height, segments, 0.0, true))
    }

    /// A regular prism about +Z, `sides` corners on `radius` with the first at `first` radians
    /// from +X; every edge is hard and the end polygons are polygon curves.
    pub fn prism(
        radius: f64,
        height: f64,
        sides: u32,
        first: f64,
    ) -> Result<Solid, PrimitiveError> {
        check_segments("sides", sides)?;
        check_radius("radius", radius, sides)?;
        check_length("height", height)?;
        if !first.is_finite() {
            return invalid("direction must be finite");
        }
        Ok(Self::prism_like(radius, height, sides, first, false))
    }

    fn prism_like(radius: f64, height: f64, n: u32, first: f64, smooth: bool) -> Solid {
        let mut s = Solid::default();
        let bottom: Vec<usize> = ring_points(radius, 0.0, n, first)
            .into_iter()
            .map(|p| s.point(p))
            .collect();
        let top: Vec<usize> = ring_points(radius, height, n, first)
            .into_iter()
            .map(|p| s.point(p))
            .collect();
        s.walls(&bottom, &top, smooth, false);
        s.face(bottom.iter().rev().copied().collect());
        s.face(top.clone());
        s.ring(bottom, !smooth);
        s.ring(top, !smooth);
        s
    }

    /// Side faces between a lower and an upper ring; `inward` turns them to face the axis.
    fn walls(&mut self, bottom: &[usize], top: &[usize], smooth: bool, inward: bool) {
        let n = bottom.len();
        for k in 0..n {
            let j = (k + 1) % n;
            let quad = vec![bottom[k], bottom[j], top[j], top[k]];
            self.face(if inward {
                quad.into_iter().rev().collect()
            } else {
                quad
            });
            if smooth {
                self.soften(bottom[k], top[k]);
            }
        }
    }

    /// A tube about +Z from z = 0 to `height`: outer wall, inner wall and two ring caps; the
    /// wall edges between sides are soft.
    pub fn tube(
        radius: f64,
        inner_radius: f64,
        height: f64,
        segments: u32,
    ) -> Result<Solid, PrimitiveError> {
        check_segments("segments", segments)?;
        check_radius("radius", radius, segments)?;
        check_radius("inner_radius", inner_radius, segments)?;
        if radius - inner_radius < MIN_LENGTH {
            return invalid("inner_radius must be smaller than radius");
        }
        check_length("height", height)?;
        let mut s = Solid::default();
        let ring = |s: &mut Solid, r: f64, z: f64| -> Vec<usize> {
            ring_points(r, z, segments, 0.0)
                .into_iter()
                .map(|p| s.point(p))
                .collect()
        };
        let (ob, ot) = (ring(&mut s, radius, 0.0), ring(&mut s, radius, height));
        let (ib, it) = (
            ring(&mut s, inner_radius, 0.0),
            ring(&mut s, inner_radius, height),
        );
        s.walls(&ob, &ot, true, false);
        s.walls(&ib, &it, true, true);
        s.face_with_hole(ob.iter().rev().copied().collect(), ib.clone());
        s.face_with_hole(ot.clone(), it.clone());
        for r in [ob, ot, ib, it] {
            s.ring(r, false);
        }
        Ok(s)
    }

    /// A cone about +Z from the base circle at z = 0 to `height`, where it narrows to
    /// `top_radius` (0 for a point; may exceed `radius`); the sloping edges are soft.
    pub fn cone(
        radius: f64,
        top_radius: f64,
        height: f64,
        segments: u32,
    ) -> Result<Solid, PrimitiveError> {
        check_segments("segments", segments)?;
        check_radius("radius", radius, segments)?;
        check_length("height", height)?;
        if top_radius != 0.0 {
            check_radius("top_radius", top_radius, segments)?;
        }
        let mut s = Solid::default();
        let bottom: Vec<usize> = ring_points(radius, 0.0, segments, 0.0)
            .into_iter()
            .map(|p| s.point(p))
            .collect();
        s.face(bottom.iter().rev().copied().collect());
        if top_radius == 0.0 {
            let apex = s.point(DVec3::new(0.0, 0.0, height));
            let n = bottom.len();
            for k in 0..n {
                let j = (k + 1) % n;
                s.face(vec![bottom[k], bottom[j], apex]);
                s.soften(bottom[k], apex);
            }
        } else {
            let top: Vec<usize> = ring_points(top_radius, height, segments, 0.0)
                .into_iter()
                .map(|p| s.point(p))
                .collect();
            s.walls(&bottom, &top, true, false);
            s.face(top.clone());
            s.ring(top, false);
        }
        s.ring(bottom, false);
        Ok(s)
    }
}

impl Model {
    /// Draws `solid` placed by `place` into this mesh like new edges and faces, so it merges
    /// with what it touches; `front` paints the faces' front sides. Returns the changes.
    pub(crate) fn draw_solid(
        &mut self,
        solid: &Solid,
        place: &DAffine3,
        front: Option<MaterialId>,
    ) -> crate::model::ChangeSet {
        let mut scratch = Model::new();
        let (faces, _) = solid.insert(&mut scratch);
        for f in faces {
            scratch.set_paint(f, [front, None]);
        }
        scratch.take_changes();
        let stencil = whole(&scratch);
        self.draw_stencil(&stencil, &[*place]);
        self.take_changes()
    }
}

impl Scene {
    /// Makes a new definition holding `solid` (not merged with anything) and places one
    /// instance of it in mesh `parent` at `place`, painted with `material`. Returns the
    /// instance.
    pub fn add_solid(
        &mut self,
        parent: MeshId,
        solid: &Solid,
        kind: DefinitionKind,
        name: &str,
        place: DAffine3,
        material: Option<MaterialId>,
    ) -> Result<(InstanceId, SceneChange), KernelError> {
        if !place.is_finite() {
            return Err(KernelError::NotFinite);
        }
        if let Some(m) = material
            && self.material(m).is_none()
        {
            return Err(KernelError::MaterialNotFound);
        }
        if self.mesh(parent).is_none() {
            return Err(KernelError::DefinitionNotFound);
        }
        let (definition, added) = self.add_definition(name, kind);
        let filled = self.edit(MeshId::Definition(definition), |m| {
            solid.insert(m);
            Ok(m.take_changes())
        })?;
        let mut placed = None;
        let left = self.edit(parent, |m| {
            let mut instance = Instance::new(definition, place);
            instance.material = material;
            placed = Some(m.insert_instance(instance));
            Ok(m.take_changes())
        })?;
        let change = SceneChange::join([SceneChange::from(added), filled, left]);
        Ok((placed.expect("instance placed"), change))
    }

    /// Draws `solid` placed by `place` into mesh `parent` with the usual merge rules, its
    /// faces' front sides painted `material`.
    pub fn draw_solid(
        &mut self,
        parent: MeshId,
        solid: &Solid,
        place: DAffine3,
        material: Option<MaterialId>,
    ) -> Result<SceneChange, KernelError> {
        if !place.is_finite() {
            return Err(KernelError::NotFinite);
        }
        if let Some(m) = material
            && self.material(m).is_none()
        {
            return Err(KernelError::MaterialNotFound);
        }
        if self.mesh(parent).is_none() {
            return Err(KernelError::DefinitionNotFound);
        }
        self.edit(parent, |m| Ok(m.draw_solid(solid, &place, material)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks `m` is a closed, outward oriented 2-manifold and returns its volume.
    pub(crate) fn closed_volume(m: &Model) -> f64 {
        m.validate().unwrap();
        let mut sides: BTreeMap<(VertexId, VertexId), u32> = BTreeMap::new();
        for (_, f) in m.faces() {
            for l in std::iter::once(&f.outer).chain(&f.holes) {
                for i in 0..l.len() {
                    *sides.entry((l[i], l[(i + 1) % l.len()])).or_default() += 1;
                }
            }
        }
        for ((a, b), n) in &sides {
            assert_eq!(*n, 1, "directed side {a:?}-{b:?} used {n} times");
            assert_eq!(
                sides.get(&(*b, *a)),
                Some(&1),
                "side {a:?}-{b:?} has no opposite"
            );
        }
        for (id, _) in m.edges() {
            let [a, b] = m.edge(id).unwrap().v;
            assert!(sides.contains_key(&(a, b)), "edge {id:?} bounds no face");
        }
        let mut volume = 0.0;
        for (_, f) in m.faces() {
            let pts = |l: &[VertexId]| l.iter().map(|v| m.pos(*v)).collect::<Vec<_>>();
            let mut area = vector_area(&pts(&f.outer)).dot(f.normal);
            for h in &f.holes {
                area += vector_area(&pts(h)).dot(f.normal);
            }
            volume += f.normal.dot(m.pos(f.outer[0])) * area / 3.0;
        }
        volume
    }

    fn soft_count(m: &Model) -> usize {
        m.edges().filter(|(_, e)| e.soft).count()
    }

    #[test]
    fn box_is_closed_with_the_volume_of_its_size() {
        let m = Solid::cuboid(DVec3::new(2.0, 3.0, 4.0)).unwrap().to_model();
        assert_eq!(m.faces().count(), 6);
        assert_eq!(m.edges().count(), 12);
        assert!((closed_volume(&m) - 24.0).abs() < 1e-9);
        assert_eq!(soft_count(&m), 0);
    }

    #[test]
    fn cylinder_volume_and_soft_sides() {
        let n = 48;
        let m = Solid::cylinder(1.5, 2.0, n).unwrap().to_model();
        let analytic = 0.5 * n as f64 * 1.5f64.powi(2) * (TAU / n as f64).sin() * 2.0;
        assert!((closed_volume(&m) - analytic).abs() < 1e-9);
        assert_eq!(m.faces().count(), n as usize + 2);
        assert_eq!(soft_count(&m), n as usize);
        // The ring edges form two curves, none soft.
        assert!(
            m.edges()
                .filter(|(_, e)| e.curve.is_some())
                .all(|(_, e)| !e.soft && !e.polygon)
        );
    }

    #[test]
    fn tube_volume_and_walls() {
        let n = 36;
        let m = Solid::tube(2.0, 1.0, 3.0, n).unwrap().to_model();
        let ring = |r: f64| 0.5 * n as f64 * r * r * (TAU / n as f64).sin();
        assert!((closed_volume(&m) - (ring(2.0) - ring(1.0)) * 3.0).abs() < 1e-9);
        assert_eq!(m.faces().count(), 2 * n as usize + 2);
        assert_eq!(soft_count(&m), 2 * n as usize);
        assert_eq!(m.faces().filter(|(_, f)| !f.holes.is_empty()).count(), 2);
    }

    #[test]
    fn cone_with_a_point_and_a_frustum() {
        let n = 30;
        let m = Solid::cone(2.0, 0.0, 3.0, n).unwrap().to_model();
        let base = 0.5 * n as f64 * 4.0 * (TAU / n as f64).sin();
        assert!((closed_volume(&m) - base * 3.0 / 3.0).abs() < 1e-9);
        assert_eq!(m.faces().count(), n as usize + 1);
        assert_eq!(soft_count(&m), n as usize);

        let m = Solid::cone(2.0, 1.0, 3.0, n).unwrap().to_model();
        let ring = |r: f64| 0.5 * n as f64 * r * r * (TAU / n as f64).sin();
        let (a, b) = (ring(2.0), ring(1.0));
        let frustum = 3.0 / 3.0 * (a + b + (a * b).sqrt());
        assert!((closed_volume(&m) - frustum).abs() < 1e-9);
        assert_eq!(soft_count(&m), n as usize);

        // A top wider than the base still points outward.
        let m = Solid::cone(1.0, 2.0, 3.0, n).unwrap().to_model();
        assert!(closed_volume(&m) > 0.0);
    }

    #[test]
    fn prism_is_hard_with_polygon_rings() {
        let m = Solid::prism(1.0, 2.0, 6, 0.0).unwrap().to_model();
        let analytic = 1.5 * 3f64.sqrt() * 2.0;
        assert!((closed_volume(&m) - analytic).abs() < 1e-9);
        assert_eq!(soft_count(&m), 0);
        assert!(
            m.edges()
                .filter(|(_, e)| e.curve.is_some())
                .all(|(_, e)| e.polygon)
        );
        assert_eq!(m.edges().filter(|(_, e)| e.curve.is_some()).count(), 12);
        // A triangle and the first corner turned by a quarter turn.
        let m = Solid::prism(1.0, 1.0, 3, PI / 2.0).unwrap().to_model();
        assert!(closed_volume(&m) > 0.0);
        let first = m.vertices().map(|(_, v)| v.pos).next().unwrap();
        assert!(first.x.abs() < 1e-12 && (first.y - 1.0).abs() < 1e-12);
    }

    #[test]
    fn every_shape_is_closed_at_the_limits() {
        for n in [3, 4, 360] {
            closed_volume(&Solid::cylinder(1.0, 1.0, n).unwrap().to_model());
            closed_volume(&Solid::tube(1.0, 0.5, 1.0, n).unwrap().to_model());
            closed_volume(&Solid::cone(1.0, 0.0, 1.0, n).unwrap().to_model());
            closed_volume(&Solid::cone(1.0, 0.25, 1.0, n).unwrap().to_model());
            closed_volume(&Solid::prism(1.0, 1.0, n, 0.3).unwrap().to_model());
        }
    }

    #[test]
    fn bad_dimensions_are_refused_naming_the_field() {
        let nan = f64::NAN;
        let msg = |r: Result<Solid, PrimitiveError>| r.unwrap_err().0;
        assert!(msg(Solid::cuboid(DVec3::new(1.0, 0.0, 1.0))).contains("size[1]"));
        assert!(msg(Solid::cuboid(DVec3::new(1.0, 1.0, nan))).contains("size[2]"));
        assert!(msg(Solid::cylinder(0.0, 1.0, 24)).contains("radius"));
        assert!(msg(Solid::cylinder(1.0, -1.0, 24)).contains("height"));
        assert!(msg(Solid::cylinder(1.0, 1.0, 2)).contains("segments"));
        assert!(msg(Solid::cylinder(1.0, 1.0, 361)).contains("segments"));
        assert!(msg(Solid::cylinder(f64::INFINITY, 1.0, 24)).contains("radius"));
        assert!(msg(Solid::cylinder(1e-5, 1.0, 360)).contains("radius"));
        assert!(msg(Solid::tube(1.0, 1.0, 1.0, 24)).contains("inner_radius"));
        assert!(msg(Solid::tube(1.0, 2.0, 1.0, 24)).contains("inner_radius"));
        assert!(msg(Solid::tube(1.0, 0.0, 1.0, 24)).contains("inner_radius"));
        assert!(msg(Solid::cone(1.0, -1.0, 1.0, 24)).contains("top_radius"));
        assert!(msg(Solid::cone(1.0, 1e-9, 1.0, 24)).contains("top_radius"));
        assert!(msg(Solid::prism(1.0, 1.0, 2, 0.0)).contains("sides"));
        assert!(msg(Solid::prism(1.0, 1.0, 361, 0.0)).contains("sides"));
    }
}
