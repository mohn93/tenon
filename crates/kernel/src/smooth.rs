//! Smooth shading across soft edges: the normal each face is shaded with at each corner.
//!
//! At a vertex, the faces around it that soft edges join form fans. Every face of a fan is
//! shaded there with the fan's area-weighted average normal, each face counted turned over
//! where it is oriented against the face it was reached from.

use std::collections::{BTreeSet, HashMap};

use glam::DVec3;

use crate::ids::{FaceId, VertexId};
use crate::model::Model;

/// A face's triangle with the normal it is shaded with at each corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadedTriangle {
    /// Wound counter-clockwise around the face's normal.
    pub corners: [DVec3; 3],
    pub normals: [DVec3; 3],
}

/// The corner normals of the faces in fans (faces that soft edges join at a vertex). An entry
/// can equal its face's normal, as in a coplanar fan. Worked out once for the whole mesh; see
/// `Model::smooth_normals`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SmoothNormals(HashMap<(FaceId, VertexId), DVec3>);

impl SmoothNormals {
    /// The normal face `face` is shaded with at vertex `v`, if it is not the face's own.
    pub fn get(&self, face: FaceId, v: VertexId) -> Option<DVec3> {
        self.0.get(&(face, v)).copied()
    }

    /// True when no face is in a fan.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A fan: faces with the sign that turns each to agree with the first.
type Fan = Vec<(FaceId, f64)>;

impl Model {
    /// The smooth corner normals of every face (see the module comment). Linear in the faces
    /// around the ends of soft edges; empty, after one pass over the edges, without them.
    pub fn smooth_normals(&self) -> SmoothNormals {
        let ends: BTreeSet<VertexId> = self
            .edges()
            .filter(|(_, e)| e.soft)
            .flat_map(|(_, e)| e.v)
            .collect();
        let mut areas: HashMap<FaceId, f64> = HashMap::new();
        let mut out = HashMap::new();
        for v in ends {
            for fan in self.fans_at(v) {
                let mut area =
                    |f: FaceId| *areas.entry(f).or_insert_with(|| self.face_area(f).abs());
                let sum: DVec3 = fan
                    .iter()
                    .map(|&(f, sign)| self.face(f).unwrap().normal * area(f) * sign)
                    .sum();
                for &(f, sign) in &fan {
                    let own = self.face(f).unwrap().normal;
                    if let Some(n) = (sum * sign).try_normalize().filter(|n| n.dot(own) > 0.0) {
                        out.insert((f, v), n);
                    }
                }
            }
        }
        SmoothNormals(out)
    }

    /// The normal face `id` is shaded with at its corner `v`: the average normal of its fan
    /// there, or its own normal when it is in none (or the average leans to its back). None
    /// for an unknown face. For many corners, `smooth_normals` is faster.
    pub fn corner_normal(&self, id: FaceId, v: VertexId) -> Option<DVec3> {
        let own = self.face(id)?.normal;
        let Some(fan) = self
            .fans_at(v)
            .into_iter()
            .find(|fan| fan.iter().any(|(f, _)| *f == id))
        else {
            return Some(own);
        };
        let sign = fan.iter().find(|(f, _)| *f == id).unwrap().1;
        let sum: DVec3 = fan
            .iter()
            .map(|&(f, s)| self.face(f).unwrap().normal * self.face_area(f).abs() * s)
            .sum();
        Some(
            (sum * sign)
                .try_normalize()
                .filter(|n| n.dot(own) > 0.0)
                .unwrap_or(own),
        )
    }

    /// `triangulate`, with the normal at each corner: from `smooth` (this mesh's
    /// `smooth_normals`), else the face's own.
    pub fn shaded_triangles(
        &self,
        id: FaceId,
        smooth: &SmoothNormals,
    ) -> Option<Vec<ShadedTriangle>> {
        let own = self.face(id)?.normal;
        let tris = self.triangle_vertices(id)?;
        Some(
            tris.iter()
                .map(|t| ShadedTriangle {
                    corners: t.map(|v| self.pos(v)),
                    normals: t.map(|v| smooth.get(id, v).unwrap_or(own)),
                })
                .collect(),
        )
    }

    /// The fans of two or more faces at `v`, joined through soft edges at `v`.
    fn fans_at(&self, v: VertexId) -> Vec<Fan> {
        let soft: Vec<VertexId> = self
            .edges_at(v)
            .into_iter()
            .filter_map(|e| self.edge(e).filter(|e| e.soft))
            .map(|e| if e.v[0] == v { e.v[1] } else { e.v[0] })
            .collect();
        if soft.is_empty() {
            return Vec::new();
        }
        let faces: Vec<FaceId> = self.faces_at(v).collect();
        // Each use of a soft side v-w by a face: (w, face index, runs from v to w).
        let mut uses: Vec<(VertexId, usize, bool)> = Vec::new();
        for (i, f) in faces.iter().enumerate() {
            let face = self.face(*f).unwrap();
            for l in std::iter::once(&face.outer).chain(&face.holes) {
                let n = l.len();
                for k in (0..n).filter(|&k| l[k] == v) {
                    let (prev, next) = (l[(k + n - 1) % n], l[(k + 1) % n]);
                    if soft.contains(&next) {
                        uses.push((next, i, true));
                    }
                    if soft.contains(&prev) {
                        uses.push((prev, i, false));
                    }
                }
            }
        }
        uses.sort_unstable();
        // Faces sharing a soft side, with whether they are oriented alike (they then run
        // along it in opposite directions).
        let mut links: Vec<Vec<(usize, bool)>> = vec![Vec::new(); faces.len()];
        for side in uses.chunk_by(|a, b| a.0 == b.0) {
            for (k, &(_, i, di)) in side.iter().enumerate() {
                for &(_, j, dj) in &side[k + 1..] {
                    if i != j {
                        links[i].push((j, di != dj));
                        links[j].push((i, di != dj));
                    }
                }
            }
        }
        let mut sign = vec![0.0; faces.len()];
        let mut fans = Vec::new();
        for start in 0..faces.len() {
            if sign[start] != 0.0 || links[start].is_empty() {
                continue;
            }
            sign[start] = 1.0;
            let mut fan = vec![start];
            let mut k = 0;
            while let Some(&i) = fan.get(k) {
                k += 1;
                for &(j, alike) in &links[i] {
                    if sign[j] == 0.0 {
                        sign[j] = if alike { sign[i] } else { -sign[i] };
                        fan.push(j);
                    }
                }
            }
            fans.push(fan.into_iter().map(|i| (faces[i], sign[i])).collect());
        }
        fans
    }
}
