//! Push/pull: move a face along its normal, extruding new sides or stretching the faces around it.

use std::collections::{BTreeMap, BTreeSet};

use glam::{DVec2, DVec3};

use crate::edit::{KernelError, loop_has_side};
use crate::geom::{
    LENGTH_TOL, PLANE_ANGLE_TOL, Plane, point_in_polygon, project_on_segment, segment_distance_2d,
};
use crate::ids::{CurveId, EdgeId, EntityId, FaceId, VertexId};
use crate::model::{ChangeSet, Face, Model};

impl Model {
    /// Pushes or pulls face `id` by `distance` meters along its normal (negative: against it).
    ///
    /// If every face touching it runs along the push direction (the top of a box), the face
    /// moves and those faces stretch. Otherwise it is extruded: a side face for every side of
    /// every loop, plus a cap. The original face stays as the base when it lies in a flat sheet
    /// and is removed when it is part of a solid's surface. Extruded faces point out of the solid.
    /// A push into a solid that ends exactly on a parallel face holding the whole pushed face
    /// (the back of a wall) cuts an opening through to it. Extruded faces (cap and sides) take
    /// the face's front and back materials; a base that turns over keeps each material on the
    /// side it was on.
    pub fn push_pull(&mut self, id: FaceId, distance: f64) -> Result<ChangeSet, KernelError> {
        let face = self.face(id).ok_or(KernelError::FaceNotFound)?.clone();
        if !distance.is_finite() {
            return Err(KernelError::NotFinite);
        }
        if distance.abs() < LENGTH_TOL {
            return Err(KernelError::ZeroDistance);
        }
        if self.stretches_around(id, &face) {
            if self.recess_floor_through(id, &face, distance) {
                // The floor of a recess pushed on to the back of the solid: cut through from
                // the floor as from the surface, then merge the new reveals into the recess
                // walls they continue.
                self.extrude(id, &face, distance, true);
                self.merge_seams(loop_vertices(&face));
            } else {
                self.move_face(id, &face, face.normal * distance)?;
            }
        } else {
            let through = self.check_path(id, &face, distance)?;
            self.extrude(id, &face, distance, through);
        }
        Ok(self.take_changes())
    }

    /// True if moving the face along its normal keeps every touching face planar: each face
    /// sharing a vertex either moves with it entirely or contains the normal direction, every
    /// edge leaving the face runs along the normal, and at least one face stretches.
    fn stretches_around(&self, id: FaceId, face: &Face) -> bool {
        let moving = loop_vertices(face);
        let n = face.normal;
        let mut stretching = false;
        for (fid, f) in self.faces() {
            if fid == id {
                continue;
            }
            let verts = loop_vertices(f);
            if verts.is_disjoint(&moving) || verts.is_subset(&moving) {
                continue;
            }
            if f.normal.dot(n).abs() > PLANE_ANGLE_TOL {
                return false;
            }
            stretching = true;
        }
        let along_normal = self.edges().all(|(_, e)| {
            let [a, b] = e.v;
            if moving.contains(&a) == moving.contains(&b) {
                return true;
            }
            (self.pos(b) - self.pos(a)).normalize().cross(n).length() < PLANE_ANGLE_TOL
        });
        stretching && along_normal
    }

    /// Moves every vertex of face `id` by `offset`, after checking that no edge leaving the face
    /// flips, no vertex lands on another and the face passes through no other face. An edge
    /// leaving the face may shrink to nothing exactly (a recess pushed back flush): the face
    /// then lands on the geometry around it and is welded to it.
    fn move_face(&mut self, id: FaceId, face: &Face, offset: DVec3) -> Result<(), KernelError> {
        let moving = loop_vertices(face);
        // Moving vertices that land on the far end of an edge leaving the face.
        let mut meets: BTreeSet<(VertexId, VertexId)> = BTreeSet::new();
        for (_, e) in self.edges() {
            let [a, b] = e.v;
            let (m, fixed) = match (moving.contains(&a), moving.contains(&b)) {
                (true, false) => (a, b),
                (false, true) => (b, a),
                _ => continue,
            };
            let before = self.pos(m) - self.pos(fixed);
            let after = before + offset;
            if after.length() < LENGTH_TOL {
                meets.insert((m, fixed));
            } else if after.dot(before) <= 0.0 {
                return Err(KernelError::Collapse);
            }
        }
        let flush = !meets.is_empty();
        // Every face that stretches must keep its fixed vertices on the side of the face they
        // started on; otherwise its loops fold over (a hole could end up outside its outer loop).
        let n = face.normal;
        let origin = self.pos(face.outer[0]);
        let d = offset.dot(n);
        for (_, f) in self.faces() {
            let verts = loop_vertices(f);
            if verts.is_disjoint(&moving) || verts.is_subset(&moving) {
                continue;
            }
            for v in verts.difference(&moving) {
                let h = (self.pos(*v) - origin).dot(n);
                let level = (h - d).abs() < LENGTH_TOL;
                if (level && !flush) || (!level && h * (h - d) <= 0.0) {
                    return Err(KernelError::Collapse);
                }
            }
        }
        for &m in &moving {
            let to = self.pos(m) + offset;
            let lands_on_other = self.vertices().any(|(v, vert)| {
                !moving.contains(&v)
                    && vert.pos.distance(to) < LENGTH_TOL
                    && !meets.contains(&(m, v))
            });
            if lands_on_other {
                return Err(KernelError::Collapse);
            }
        }
        // A face carried flush onto another face (a box top pushed down to its bottom) would
        // flatten the solid: the edges joining them collapse.
        self.check_unobstructed(id, offset.dot(n))
            .map_err(|e| if flush { KernelError::Collapse } else { e })?;
        for &m in &moving {
            let to = self.pos(m) + offset;
            self.move_vertex(m, to);
        }
        if flush {
            self.weld(&moving);
        }
        Ok(())
    }

    /// Extrudes face `id` by `distance`. With `through`, the push ends on the back of the solid:
    /// the cap and the part of the back face it covers are removed, leaving an opening. The cap
    /// and the side faces get the face's front and back materials.
    fn extrude(&mut self, id: FaceId, face: &Face, distance: f64, through: bool) {
        let n = face.normal;
        let offset = n * distance;
        let in_solid = self.in_solid(id);
        let loops: Vec<&Vec<VertexId>> = std::iter::once(&face.outer).chain(&face.holes).collect();
        let points: Vec<Vec<DVec3>> = loops
            .iter()
            .map(|l| l.iter().map(|v| self.pos(*v)).collect())
            .collect();
        // Every loop side (a, b) of the face with its positions; each sweeps the rectangle
        // a, b, b + offset, a + offset.
        let sides: Vec<(VertexId, VertexId, DVec3, DVec3)> = loops
            .iter()
            .zip(&points)
            .flat_map(|(l, ps)| {
                (0..l.len()).map(move |i| {
                    let j = (i + 1) % l.len();
                    (l[i], l[j], ps[i], ps[j])
                })
            })
            .collect();
        // The moved loops, to tell the cap apart from the rest of a back face in its plane.
        let cap_plane = Plane::through(points[0][0] + offset, n);
        let ring = |l: &Vec<DVec3>| -> Vec<DVec2> {
            l.iter().map(|p| cap_plane.project(*p + offset)).collect()
        };
        let cap = Polygon {
            plane: cap_plane,
            outer: ring(&points[0]),
            holes: points[1..].iter().map(ring).collect(),
        };
        // The curve of each loop side with its polygon flag, to carry over to the moved sides
        // (the cap) and to soften the swept edges between two sides of one non-polygon curve.
        let side_curves: Vec<Vec<Option<(CurveId, bool)>>> = loops
            .iter()
            .map(|l| {
                (0..l.len())
                    .map(|i| {
                        self.edge_between(l[i], l[(i + 1) % l.len()])
                            .and_then(|e| self.edge(e))
                            .and_then(|e| Some((e.curve?, e.polygon)))
                    })
                    .collect()
            })
            .collect();
        let before: BTreeSet<FaceId> = self.faces().map(|(f, _)| f).collect();
        let vertices_before: BTreeSet<VertexId> = self.vertices().map(|(v, _)| v).collect();
        // A cut opens a piece of a side only where a face that existed before covered it (a
        // notch at a box edge). The faces in each side's plane are recorded before the
        // extrusion splits them.
        let covering: Vec<Vec<Polygon>> = sides
            .iter()
            .map(|&(_, _, pa, pb)| {
                let Some(sp) = Plane::from_points(pa, pb, pa + offset) else {
                    return Vec::new();
                };
                self.faces()
                    .filter(|(f, _)| *f != id)
                    .filter_map(|(_, g)| self.polygon_in(g, &sp))
                    .collect()
            })
            .collect();

        if in_solid {
            self.remove_face(id);
        }
        for l in &points {
            for p in l {
                self.draw_segment(*p, *p + offset);
            }
        }
        for l in &points {
            for i in 0..l.len() {
                self.draw_segment(l[i] + offset, l[(i + 1) % l.len()] + offset);
            }
        }

        // Each hole's cap loop closes inside the cap, which fills it like any loop drawn on a
        // face; that fill is removed so the hole stays open. Through a solid, the back face
        // already fills it.
        let hole_caps: Vec<BTreeSet<VertexId>> = points[1..]
            .iter()
            .filter(|_| !through)
            .filter_map(|l| l.iter().map(|p| self.find_vertex(*p + offset)).collect())
            .collect();
        if through {
            // The cap lands on the back face: every face there inside the cap opens. `holds`
            // guarantees the moved loop splits the back face cleanly, so a face is either wholly
            // inside the cap or wholly outside, and one interior point tells which.
            let inside: Vec<FaceId> = self
                .faces()
                .map(|(f, _)| f)
                .filter(|f| {
                    let g = self.face(*f).unwrap();
                    g.normal.cross(n).length() < PLANE_ANGLE_TOL
                        && cap_plane.contains(self.pos(g.outer[0]))
                        && self.interior_point(*f).is_some_and(|p| cap.contains(p))
                })
                .collect();
            for f in inside {
                self.remove_face(f);
            }
        }
        // A solid grows outward when pulled along the normal and is cut when pushed against it.
        let cut = in_solid && distance < 0.0;
        let cap_normal = if in_solid || distance > 0.0 { n } else { -n };
        let cap_origin = points[0][0] + offset;
        // A flat sheet pulled against its normal turns over: the side facing the pull direction
        // is the one that ends up outside, so the cap and sides take the materials swapped.
        // The cap is the pushed face moved: it keeps the texture placement too.
        let (new_paint, cap_texture) = if cap_normal == -n {
            let r = face.reversed();
            (r.paint(), r.texture)
        } else {
            (face.paint(), face.texture)
        };
        // The cap is the pushed face moved: it keeps the face's tag and hidden flag. The sides
        // are new geometry: Untagged and shown, whatever the pushed face (or a coplanar face
        // they were cut from) is on.
        let cap_tag = face.tag;
        let cap_hidden = face.hidden;
        let created: Vec<FaceId> = self
            .faces()
            .map(|(f, _)| f)
            .filter(|f| !before.contains(f))
            .collect();
        for f in created {
            let g = self.face(f).unwrap().clone();
            let outer: BTreeSet<VertexId> = g.outer.iter().copied().collect();
            if hole_caps.contains(&outer) {
                self.remove_face(f);
                continue;
            }
            let in_cap = g.normal.cross(n).length() < PLANE_ANGLE_TOL
                && (self.pos(g.outer[0]) - cap_origin).dot(n).abs() < LENGTH_TOL;
            if in_cap && through {
                // The rest of the back face keeps its orientation.
                continue;
            }
            if in_cap {
                self.orient(f, cap_normal);
                self.set_paint(f, new_paint);
                self.set_texture(f, cap_texture);
                self.set_tag(EntityId::Face(f), cap_tag);
                self.set_hidden(EntityId::Face(f), cap_hidden);
                continue;
            }
            // A side face lies inside a side's swept rectangle. Other new faces in a side's
            // plane (the rest of a wall split by the extrusion) keep their orientation.
            let Some(inside) = self.interior_point(f) else {
                continue;
            };
            let side = sides
                .iter()
                .position(|&(_, _, pa, pb)| in_swept(inside, pa, pb, offset));
            let Some(i) = side else {
                continue;
            };
            if cut && covering[i].iter().any(|poly| poly.contains(inside)) {
                // The cut reaches an existing face in this side's plane: leave it open.
                self.remove_face(f);
                continue;
            }
            let (_, _, pa, pb) = sides[i];
            let out = (pb - pa).cross(n).normalize();
            self.orient(f, if cut { -out } else { out });
            // The sides take the pushed face's materials, as the cap does, but no placement
            // (one made inside a coplanar old face inherited that face's).
            self.set_paint(f, new_paint);
            self.set_texture(f, None);
            self.set_tag(EntityId::Face(f), None);
            self.set_hidden(EntityId::Face(f), false);
        }
        if cut {
            self.remove_strays(&vertices_before, &sides, offset);
        }
        if !in_solid {
            self.orient(id, if distance > 0.0 { -n } else { n });
        }
        self.mark_swept_curves(&points, &side_curves, offset);
    }

    /// After an extrusion: the moved copy of each curve becomes a curve of its own (a polygon
    /// when the curve is one), and the swept edge from a point between two sides of one curve
    /// that is not a polygon is soft, so the sides of a pulled circle or arc read as one smooth
    /// surface while a pulled polygon keeps hard sides. (Upkeep hardens a swept edge that ends
    /// up bounding fewer than two faces.)
    fn mark_swept_curves(
        &mut self,
        points: &[Vec<DVec3>],
        side_curves: &[Vec<Option<(CurveId, bool)>>],
        offset: DVec3,
    ) {
        let mut renamed: BTreeMap<CurveId, CurveId> = BTreeMap::new();
        for (ps, curves) in points.iter().zip(side_curves) {
            let n = ps.len();
            for i in 0..n {
                let (before, after) = (curves[(i + n - 1) % n], curves[i]);
                if before.is_some_and(|(_, polygon)| !polygon) && before == after {
                    self.tag_along(ps[i], ps[i] + offset, None, true, false);
                }
                if let Some((c, polygon)) = after {
                    let moved = match renamed.get(&c) {
                        Some(m) => *m,
                        None => {
                            let m = self.new_curve();
                            renamed.insert(c, m);
                            m
                        }
                    };
                    let (a, b) = (ps[i] + offset, ps[(i + 1) % n] + offset);
                    self.tag_along(a, b, Some(moved), false, polygon);
                }
            }
        }
    }

    /// After a cut, removes the edges that border no face and lie on a swept vertical segment
    /// (p to p + offset for a loop point p), on a loop side of the pushed face, or on one of its
    /// moved sides (the rim of an opening on the back face). Then removes the loop vertices,
    /// their offset copies and the vertices this push/pull created that no edge uses any more.
    /// User lines the extrusion split lie on none of these, so they stay.
    fn remove_strays(
        &mut self,
        vertices_before: &BTreeSet<VertexId>,
        sides: &[(VertexId, VertexId, DVec3, DVec3)],
        offset: DVec3,
    ) {
        let mut bordered: BTreeSet<(VertexId, VertexId)> = BTreeSet::new();
        for (_, f) in self.faces() {
            for l in std::iter::once(&f.outer).chain(&f.holes) {
                for i in 0..l.len() {
                    bordered.insert(side(l[i], l[(i + 1) % l.len()]));
                }
            }
        }
        let segments: Vec<(DVec3, DVec3)> = sides
            .iter()
            .flat_map(|&(_, _, pa, pb)| [(pa, pa + offset), (pa, pb), (pa + offset, pb + offset)])
            .collect();
        let on_segment = |p: DVec3, (a, b): (DVec3, DVec3)| {
            let (t, d) = project_on_segment(p, a, b);
            let tol = LENGTH_TOL / (b - a).length();
            d < LENGTH_TOL && t > -tol && t < 1.0 + tol
        };
        let strays: Vec<EdgeId> = self
            .edges()
            .filter(|(_, e)| {
                let [a, b] = e.v;
                let (pa, pb) = (self.pos(a), self.pos(b));
                !bordered.contains(&side(a, b))
                    && segments
                        .iter()
                        .any(|&seg| on_segment(pa, seg) && on_segment(pb, seg))
            })
            .map(|(e, _)| e)
            .collect();
        for e in strays {
            self.remove_edge(e);
        }
        let mut candidates: BTreeSet<VertexId> = sides.iter().map(|&(a, _, _, _)| a).collect();
        candidates.extend(
            sides
                .iter()
                .filter_map(|&(_, _, pa, _)| self.find_vertex(pa + offset)),
        );
        candidates.extend(
            self.vertices()
                .map(|(v, _)| v)
                .filter(|v| !vertices_before.contains(v)),
        );
        for v in candidates {
            if self.edges_at(v).is_empty() {
                self.remove_vertex(v);
            }
        }
    }

    /// Face `g` as a polygon, if it lies in `plane`.
    fn polygon_in(&self, g: &Face, plane: &Plane) -> Option<Polygon> {
        let o = self.pos(g.outer[0]);
        let own = Plane {
            normal: g.normal,
            d: g.normal.dot(o),
        };
        if !own.same_as(plane) {
            return None;
        }
        let ring = |l: &Vec<VertexId>| -> Vec<DVec2> {
            l.iter().map(|v| own.project(self.pos(*v))).collect()
        };
        Some(Polygon {
            plane: own,
            outer: ring(&g.outer),
            holes: g.holes.iter().map(ring).collect(),
        })
    }

    /// A point strictly inside face `id`: the centroid of its first triangle.
    pub(crate) fn interior_point(&self, id: FaceId) -> Option<DVec3> {
        let [a, b, c] = *self.triangulate(id)?.first()?;
        Some((a + b + c) / 3.0)
    }

    /// Fails with `Obstructed` if face `id`, moved `distance` along its normal, would meet another
    /// face: a ray from a point inside it hits one within that distance.
    fn check_unobstructed(&self, id: FaceId, distance: f64) -> Result<(), KernelError> {
        if self.hits_along(id, distance).is_empty() {
            Ok(())
        } else {
            Err(KernelError::Obstructed)
        }
    }

    /// Checks the path of face `id` extruded by `distance`, like `check_unobstructed`, except
    /// that a push into a solid may end exactly on one face parallel to it that holds the whole
    /// moved face (the back of a wall). Returns true in that case: the push cuts through.
    fn check_path(&self, id: FaceId, face: &Face, distance: f64) -> Result<bool, KernelError> {
        let hits = self.hits_along(id, distance);
        let [(g, t)] = hits[..] else {
            return if hits.is_empty() {
                Ok(false)
            } else {
                Err(KernelError::Obstructed)
            };
        };
        let ends_on_back = distance < 0.0
            && (t - distance.abs()).abs() < LENGTH_TOL
            && self.holds(g, face, face.normal * distance)
            && self.in_solid(id)
            && !self.sweep_covers_a_face(id, face, face.normal * distance);
        if ends_on_back {
            Ok(true)
        } else {
            Err(KernelError::Obstructed)
        }
    }

    /// True if a face other than `id` lies wholly inside the rectangle a side of any of
    /// `face`'s loops (outer or hole) sweeps when moved by `offset`. A cut through would match
    /// such a face exactly, so it would never be split or removed and would be left floating.
    fn sweep_covers_a_face(&self, id: FaceId, face: &Face, offset: DVec3) -> bool {
        std::iter::once(&face.outer).chain(&face.holes).any(|l| {
            (0..l.len()).any(|i| {
                let (a, b) = (self.pos(l[i]), self.pos(l[(i + 1) % l.len()]));
                self.faces().any(|(f, g)| {
                    f != id
                        && loop_vertices(g)
                            .iter()
                            .all(|v| in_swept(self.pos(*v), a, b, offset))
                })
            })
        })
    }

    /// True if face `id` is the floor of a recess (a face on a side of its outer loop faces into
    /// the loop: a recess wall) and pushing it in by `distance` ends on the back of the solid as
    /// `check_path` allows. Faces on the other sides may face out (the top of a wall beside a
    /// slot). A face whose sides all face out (a box top) is no recess floor: it never cuts.
    fn recess_floor_through(&self, id: FaceId, face: &Face, distance: f64) -> bool {
        if distance >= 0.0 {
            return false;
        }
        let n = face.normal;
        let l = &face.outer;
        let recess_wall = (0..l.len()).any(|i| {
            let (a, b) = (l[i], l[(i + 1) % l.len()]);
            let out = (self.pos(b) - self.pos(a)).cross(n);
            self.faces().any(|(f, g)| {
                f != id
                    && std::iter::once(&g.outer)
                        .chain(&g.holes)
                        .any(|gl| loop_has_side(gl, a, b))
                    && g.normal.dot(out) < 0.0
            })
        });
        recess_wall && matches!(self.check_path(id, face, distance), Ok(true))
    }

    /// The faces a ray from a point inside face `id` meets beyond `LENGTH_TOL` and within
    /// `distance` along its normal, each with the distance to its nearest hit.
    fn hits_along(&self, id: FaceId, distance: f64) -> Vec<(FaceId, f64)> {
        let Some(tris) = self.triangulate(id) else {
            return Vec::new();
        };
        let Some([a, b, c]) = tris.first().copied() else {
            return Vec::new();
        };
        let from = (a + b + c) / 3.0;
        let dir = self.face(id).unwrap().normal * distance.signum();
        let reach = distance.abs() + LENGTH_TOL;
        let mut hits = self.index().ray_hits(from, dir, LENGTH_TOL, reach);
        hits.retain(|(f, _)| *f != id);
        hits
    }

    /// True if face `g` is parallel to `face` and, in its plane, holds `face` moved by `offset`
    /// cleanly: every corner and side midpoint of the moved outer loop lies inside `g` or on its
    /// boundary, no vertex of `g` lies strictly inside the moved loop (a hole, boss or notch) and
    /// no side of `g` properly crosses a side of the moved loop.
    fn holds(&self, g: FaceId, face: &Face, offset: DVec3) -> bool {
        let gf = self.face(g).unwrap();
        let moved: Vec<DVec3> = face.outer.iter().map(|v| self.pos(*v) + offset).collect();
        let plane = Plane::through(self.pos(gf.outer[0]), gf.normal);
        let Some(poly) = self.polygon_in(gf, &plane) else {
            return false;
        };
        let parallel = gf.normal.cross(face.normal).length() < PLANE_ANGLE_TOL;
        if !parallel {
            return false;
        }
        let covered = (0..moved.len()).all(|i| {
            let (a, b) = (moved[i], moved[(i + 1) % moved.len()]);
            plane.contains(a) && poly.holds(a) && poly.holds((a + b) * 0.5)
        });
        if !covered {
            return false;
        }
        let ring: Vec<DVec2> = moved.iter().map(|p| poly.plane.project(*p)).collect();
        let sides_of = |l: &Vec<DVec2>| -> Vec<(DVec2, DVec2)> {
            (0..l.len()).map(|i| (l[i], l[(i + 1) % l.len()])).collect()
        };
        let moved_sides = sides_of(&ring);
        let g_loops = std::iter::once(&poly.outer).chain(&poly.holes);
        for l in g_loops {
            let strictly_inside = |q: &DVec2| {
                point_in_polygon(*q, &ring)
                    && moved_sides
                        .iter()
                        .all(|&(a, b)| segment_distance_2d(*q, a, b) > LENGTH_TOL)
            };
            if l.iter().any(strictly_inside) {
                return false;
            }
            let crossing = sides_of(l)
                .iter()
                .any(|&(c, d)| moved_sides.iter().any(|&(a, b)| properly_cross(a, b, c, d)));
            if crossing {
                return false;
            }
        }
        true
    }

    /// Runs `draw` (an edit that draws a shape through `point` in the plane with normal
    /// `normal`) and turns every face it newly makes in that plane to face along `normal`,
    /// keeping materials on their sides, as one ChangeSet. A new face lying inside a face
    /// that was in that plane before (a piece split from it) keeps that face's orientation.
    pub fn draw_facing(
        &mut self,
        point: DVec3,
        normal: DVec3,
        draw: impl FnOnce(&mut Model) -> Result<ChangeSet, KernelError>,
    ) -> Result<ChangeSet, KernelError> {
        let plane = Plane::through(point, normal);
        let in_plane = |m: &Model, f: &Face| {
            f.outer
                .first()
                .is_some_and(|v| Plane::through(m.pos(*v), f.normal).same_as(&plane))
        };
        let project = |m: &Model, l: &[VertexId]| -> Vec<DVec2> {
            l.iter().map(|v| plane.project(m.pos(*v))).collect()
        };
        // The outlines of the faces already in the plane: outer loop and holes.
        let before: Vec<(Vec<DVec2>, Vec<Vec<DVec2>>)> = self
            .faces()
            .filter(|(_, f)| in_plane(self, f))
            .map(|(_, f)| {
                (
                    project(self, &f.outer),
                    f.holes.iter().map(|h| project(self, h)).collect(),
                )
            })
            .collect();
        let mut changes = draw(self)?;
        let summary = changes.summary();
        for e in &summary.created {
            let EntityId::Face(id) = e else { continue };
            let Some(f) = self.face(*id) else { continue };
            if !in_plane(self, f) {
                continue;
            }
            // The centroid of the face's largest triangle lies inside its area; a face that
            // cannot be triangulated is taken as new.
            let inside = self.triangulate(*id).and_then(|tris| {
                tris.into_iter()
                    .max_by(|a, b| {
                        let area = |t: &[DVec3; 3]| (t[1] - t[0]).cross(t[2] - t[0]).length();
                        area(a).total_cmp(&area(b))
                    })
                    .map(|t| plane.project((t[0] + t[1] + t[2]) / 3.0))
            });
            if let Some(p) = inside
                && before.iter().any(|(o, hs)| {
                    point_in_polygon(p, o) && !hs.iter().any(|h| point_in_polygon(p, h))
                })
            {
                continue;
            }
            self.orient(*id, normal);
        }
        changes.ops.extend(self.take_changes().ops);
        Ok(changes)
    }

    /// Flips face `id` if its normal points away from `want`. Its materials stay on the sides
    /// they were on.
    pub(crate) fn orient(&mut self, id: FaceId, want: DVec3) {
        let f = self.face(id).unwrap();
        if f.normal.dot(want) >= 0.0 {
            return;
        }
        let flipped = f.reversed();
        self.replace_face(id, flipped);
    }

    /// True if the face is part of a solid's surface with the solid behind it: it is connected
    /// through shared sides to a face with a vertex behind its plane. Faces connected only to
    /// faces in its plane or in front of it (a flat sheet, the ground beside a box) are not.
    fn in_solid(&self, id: FaceId) -> bool {
        let mut by_side: BTreeMap<(VertexId, VertexId), Vec<FaceId>> = BTreeMap::new();
        for (fid, f) in self.faces() {
            for l in std::iter::once(&f.outer).chain(&f.holes) {
                for i in 0..l.len() {
                    by_side
                        .entry(side(l[i], l[(i + 1) % l.len()]))
                        .or_default()
                        .push(fid);
                }
            }
        }
        let n = self.face(id).unwrap().normal;
        let origin = self.pos(self.face(id).unwrap().outer[0]);
        let mut seen = BTreeSet::from([id]);
        let mut stack = vec![id];
        while let Some(fid) = stack.pop() {
            let f = self.face(fid).unwrap();
            let behind = loop_vertices(f)
                .iter()
                .any(|v| (self.pos(*v) - origin).dot(n) < -LENGTH_TOL);
            if behind {
                return true;
            }
            for l in std::iter::once(&f.outer).chain(&f.holes) {
                for i in 0..l.len() {
                    for &next in &by_side[&side(l[i], l[(i + 1) % l.len()])] {
                        if seen.insert(next) {
                            stack.push(next);
                        }
                    }
                }
            }
        }
        false
    }
}

/// A face's loops projected into its plane, recorded before an edit changes the face.
struct Polygon {
    plane: Plane,
    outer: Vec<DVec2>,
    holes: Vec<Vec<DVec2>>,
}

impl Polygon {
    /// True if `p` (assumed in the plane) is inside the outer loop and outside every hole.
    fn contains(&self, p: DVec3) -> bool {
        let q = self.plane.project(p);
        point_in_polygon(q, &self.outer) && !self.holes.iter().any(|h| point_in_polygon(q, h))
    }

    /// True if `p` (assumed in the plane) is inside or on the boundary.
    fn holds(&self, p: DVec3) -> bool {
        let q = self.plane.project(p);
        let on = |l: &Vec<DVec2>| {
            (0..l.len()).any(|i| segment_distance_2d(q, l[i], l[(i + 1) % l.len()]) < LENGTH_TOL)
        };
        on(&self.outer) || self.holes.iter().any(on) || self.contains(p)
    }
}

/// True if segments ab and cd cross at a point interior to both, beyond `LENGTH_TOL` from every
/// end (not collinear overlap, not touching at ends).
fn properly_cross(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> bool {
    let (r, q) = (b - a, d - c);
    let det = r.perp_dot(q);
    if det.abs() < 1e-12 {
        return false;
    }
    let t = (c - a).perp_dot(q) / det;
    let u = (c - a).perp_dot(r) / det;
    let (et, eu) = (LENGTH_TOL / r.length(), LENGTH_TOL / q.length());
    t > et && t < 1.0 - et && u > eu && u < 1.0 - eu
}

fn loop_vertices(f: &Face) -> BTreeSet<VertexId> {
    std::iter::once(&f.outer)
        .chain(&f.holes)
        .flatten()
        .copied()
        .collect()
}

/// True if `p` lies inside the rectangle a, b, b + offset, a + offset (in its plane, with
/// tolerance). `offset` is perpendicular to b - a.
fn in_swept(p: DVec3, a: DVec3, b: DVec3, offset: DVec3) -> bool {
    let (ab, rel) = (b - a, p - a);
    let u = rel.dot(ab) / ab.length_squared();
    let w = rel.dot(offset) / offset.length_squared();
    let off_plane = (rel - ab * u - offset * w).length();
    let (eu, ew) = (LENGTH_TOL / ab.length(), LENGTH_TOL / offset.length());
    off_plane < LENGTH_TOL && u > -eu && u < 1.0 + eu && w > -ew && w < 1.0 + ew
}

fn side(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    (a.min(b), a.max(b))
}
