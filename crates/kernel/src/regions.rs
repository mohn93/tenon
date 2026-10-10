//! Bounded regions of a planar 2D graph: an outer loop (counter-clockwise) plus holes (clockwise).

use std::collections::{BTreeMap, BTreeSet};

use glam::DVec2;

use crate::geom::{LENGTH_TOL, point_in_polygon, segment_distance_2d, signed_area2};
use crate::ids::VertexId;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Region {
    /// Counter-clockwise outer loop.
    pub outer: Vec<VertexId>,
    /// Clockwise hole loops.
    pub holes: Vec<Vec<VertexId>>,
}

/// Vertices projected into a plane and the edges joining them. Edges must not cross.
pub(crate) struct PlaneGraph {
    pub pos: BTreeMap<VertexId, DVec2>,
    pub edges: Vec<[VertexId; 2]>,
}

impl PlaneGraph {
    /// Every bounded region of the graph, each with its holes.
    pub fn regions(&self) -> Vec<Region> {
        let mut regions: Vec<(Region, f64)> = Vec::new();
        let mut loose: Vec<Vec<VertexId>> = Vec::new();
        for mut walk in self.walks() {
            remove_spurs(&mut walk);
            let mut outers: Vec<(Vec<VertexId>, f64)> = Vec::new();
            let mut holes = Vec::new();
            for l in split_pinches(&walk) {
                let area = self.area(&l);
                if area.abs() <= LENGTH_TOL * LENGTH_TOL {
                    continue;
                }
                if area > 0.0 {
                    outers.push((l, area));
                } else {
                    holes.push(l);
                }
            }
            // A walk around a bounded region yields its outer loop plus any holes touching it.
            outers.sort_by(|a, b| b.1.total_cmp(&a.1));
            let mut outers = outers.into_iter();
            match outers.next() {
                Some((outer, area)) => regions.push((Region { outer, holes }, area)),
                None => loose.extend(holes),
            }
            for (outer, area) in outers {
                let holes = Vec::new();
                regions.push((Region { outer, holes }, area));
            }
        }
        // The outside walk of a separate component is a hole of the smallest region around it.
        for h in loose {
            let container = (0..regions.len())
                .filter(|&i| {
                    let outer: BTreeSet<_> = regions[i].0.outer.iter().collect();
                    h.iter().find(|v| !outer.contains(v)).is_some_and(|v| {
                        point_in_polygon(self.pos[v], &self.poly(&regions[i].0.outer))
                    })
                })
                .min_by(|&a, &b| regions[a].1.total_cmp(&regions[b].1));
            if let Some(i) = container {
                regions[i].0.holes.push(h);
            }
        }
        regions.into_iter().map(|(r, _)| r).collect()
    }

    /// A point strictly inside the region: just inside the middle of one of its sides, closer to
    /// that side than to any other edge of the graph.
    pub fn interior_point(&self, r: &Region) -> Option<DVec2> {
        let outer = self.poly(&r.outer);
        let holes: Vec<Vec<DVec2>> = r.holes.iter().map(|h| self.poly(h)).collect();
        let inside =
            |p: DVec2| point_in_polygon(p, &outer) && !holes.iter().any(|h| point_in_polygon(p, h));
        for l in std::iter::once(&r.outer).chain(&r.holes) {
            for i in 0..l.len() {
                let (a, b) = (l[i], l[(i + 1) % l.len()]);
                let (pa, pb) = (self.pos[&a], self.pos[&b]);
                let mid = (pa + pb) * 0.5;
                let clearance = self
                    .edges
                    .iter()
                    .filter(|e| **e != [a, b] && **e != [b, a])
                    .map(|e| segment_distance_2d(mid, self.pos[&e[0]], self.pos[&e[1]]))
                    .fold(f64::INFINITY, f64::min);
                let side = pb - pa;
                let eps = (1e-4 * side.length()).min(0.5 * clearance);
                // The region lies left of every side: left of a CCW outer and of a CW hole.
                let p = mid + side.perp().normalize() * eps;
                if inside(p) {
                    return Some(p);
                }
            }
        }
        None
    }

    pub fn poly(&self, l: &[VertexId]) -> Vec<DVec2> {
        l.iter().map(|v| self.pos[v]).collect()
    }

    fn area(&self, l: &[VertexId]) -> f64 {
        signed_area2(&self.poly(l)) * 0.5
    }

    /// Traces every directed side once, always turning as far left as possible. Each walk is a
    /// closed vertex sequence that may contain spurs and repeated vertices.
    fn walks(&self) -> Vec<Vec<VertexId>> {
        let mut nbrs: BTreeMap<VertexId, Vec<VertexId>> = BTreeMap::new();
        for &[a, b] in &self.edges {
            nbrs.entry(a).or_default().push(b);
            nbrs.entry(b).or_default().push(a);
        }
        for (v, list) in nbrs.iter_mut() {
            let o = self.pos[v];
            list.sort_by(|x, y| {
                let ax = (self.pos[x] - o).to_angle();
                let ay = (self.pos[y] - o).to_angle();
                ax.total_cmp(&ay)
            });
        }
        let mut visited: BTreeSet<(VertexId, VertexId)> = BTreeSet::new();
        let mut walks = Vec::new();
        for (&start, list) in &nbrs {
            for &next in list {
                if visited.contains(&(start, next)) {
                    continue;
                }
                let mut walk = Vec::new();
                let (mut u, mut w) = (start, next);
                loop {
                    visited.insert((u, w));
                    walk.push(u);
                    let around = &nbrs[&w];
                    let i = around.iter().position(|x| *x == u).unwrap();
                    let turn = around[(i + around.len() - 1) % around.len()];
                    (u, w) = (w, turn);
                    if (u, w) == (start, next) {
                        break;
                    }
                }
                walks.push(walk);
            }
        }
        walks
    }
}

/// Removes out-and-back excursions (u, w, u -> u) until none remain.
fn remove_spurs(w: &mut Vec<VertexId>) {
    loop {
        let n = w.len();
        if n < 3 {
            w.clear();
            return;
        }
        let Some(i) = (0..n).find(|&i| w[(i + n - 1) % n] == w[(i + 1) % n]) else {
            return;
        };
        let j = (i + 1) % n;
        w.remove(i.max(j));
        w.remove(i.min(j));
    }
}

/// Splits a closed walk at repeated vertices into simple loops.
fn split_pinches(w: &[VertexId]) -> Vec<Vec<VertexId>> {
    let mut loops = Vec::new();
    let mut stack: Vec<VertexId> = Vec::new();
    for &v in w {
        if let Some(j) = stack.iter().position(|x| *x == v) {
            loops.push(stack.split_off(j));
        }
        stack.push(v);
    }
    if !stack.is_empty() {
        loops.push(stack);
    }
    loops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(pts: &[(f64, f64)], edges: &[(u64, u64)]) -> PlaneGraph {
        PlaneGraph {
            pos: pts
                .iter()
                .enumerate()
                .map(|(i, p)| (VertexId(i as u64), DVec2::new(p.0, p.1)))
                .collect(),
            edges: edges
                .iter()
                .map(|&(a, b)| [VertexId(a), VertexId(b)])
                .collect(),
        }
    }

    fn ids(l: &[u64]) -> Vec<VertexId> {
        l.iter().map(|&i| VertexId(i)).collect()
    }

    #[test]
    fn spurs_and_pinches_are_cleaned() {
        let mut w = ids(&[0, 1, 4, 5, 4, 1, 2, 3]);
        remove_spurs(&mut w);
        assert_eq!(w, ids(&[0, 1, 2, 3]));
        let loops = split_pinches(&ids(&[0, 1, 2, 9, 5, 6, 9, 3]));
        assert_eq!(loops, vec![ids(&[9, 5, 6]), ids(&[0, 1, 2, 9, 3])]);
    }

    #[test]
    fn square_with_dangling_edge_and_island() {
        // Square 0-3 with a dangling edge 0-4 and a separate triangle 5-7 inside.
        let g = graph(
            &[
                (0.0, 0.0),
                (4.0, 0.0),
                (4.0, 4.0),
                (0.0, 4.0),
                (1.0, 0.5),
                (2.0, 2.0),
                (3.0, 2.0),
                (3.0, 3.0),
            ],
            &[
                (0, 1),
                (1, 2),
                (2, 3),
                (3, 0),
                (0, 4),
                (5, 6),
                (6, 7),
                (7, 5),
            ],
        );
        let mut r = g.regions();
        r.sort_by_key(|r| r.outer.len());
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].outer.len(), 3);
        assert!(r[0].holes.is_empty());
        assert_eq!(r[1].outer.len(), 4);
        assert_eq!(r[1].holes.len(), 1);
        assert!(g.area(&r[1].holes[0]) < 0.0);
    }

    #[test]
    fn interior_point_fits_a_thin_sliver() {
        // Square 0-3 and a sliver below its bottom side: 0, 4 (just below the middle), 1.
        let g = graph(
            &[
                (0.0, 0.0),
                (10.0, 0.0),
                (10.0, 10.0),
                (0.0, 10.0),
                (5.0, -0.0005),
            ],
            &[(0, 1), (1, 2), (2, 3), (3, 0), (0, 4), (4, 1)],
        );
        let r = g.regions();
        assert_eq!(r.len(), 2);
        for region in &r {
            let p = g.interior_point(region).unwrap();
            let in_square = point_in_polygon(p, &g.poly(&ids(&[0, 1, 2, 3])));
            assert_eq!(in_square, region.outer.len() == 4, "{p} for {region:?}");
        }
    }

    #[test]
    fn interior_point_avoids_holes() {
        let g = graph(
            &[
                (0.0, 0.0),
                (4.0, 0.0),
                (4.0, 4.0),
                (0.0, 4.0),
                (1.0, 1.0),
                (1.0, 3.0),
                (3.0, 3.0),
                (3.0, 1.0),
            ],
            &[
                (0, 1),
                (1, 2),
                (2, 3),
                (3, 0),
                (4, 5),
                (5, 6),
                (6, 7),
                (7, 4),
            ],
        );
        let ring = Region {
            outer: ids(&[0, 1, 2, 3]),
            holes: vec![ids(&[4, 5, 6, 7])],
        };
        let p = g.interior_point(&ring).unwrap();
        assert!(point_in_polygon(p, &g.poly(&ring.outer)));
        assert!(!point_in_polygon(p, &g.poly(&ring.holes[0])));
    }
}
