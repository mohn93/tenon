//! Simplified copies of a model's opaque chunks, drawn in place of the full ones when their
//! detail would be about a pixel.
//!
//! Each chunk is simplified by vertex clustering (Rossignac and Borrel): space is cut into
//! cubic cells, the vertices of a cell become one, and the triangles that then have two
//! corners in one cell are dropped. The cells lie on one grid for all chunks, so neighbouring
//! chunks of a level meet. What the dropped triangles looked like is not lost: a cell's colors,
//! glow and finish are the area-weighted average of everything in it, and each kept triangle
//! takes its corners' from their cells. A frame draws a chunk's coarsest level whose cells are
//! small on the screen (see `Lod::pick`).

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::ops::Range;

use glam::Vec3;

use crate::chunks::ChunkPlan;
use crate::scene::BaseFaceVertex;

/// The cell size of each level in meters, finest first.
const CELLS: [f32; 4] = [1.0, 2.5, 6.0, 15.0];

/// A level is kept for a chunk only when it has at most this share of the triangles of the
/// level before it: otherwise it costs memory and saves little.
const WORTH: f32 = 0.65;

/// How many pixels across a level's cells may be on the screen, by default. The cells' colors are averages, so a little over a pixel is not seen.
pub const CELL_PIXELS: f32 = 1.5;

/// One level of simplification of every solid chunk.
struct Level {
    /// The cell size, in meters.
    cell: f32,
    /// Each chunk's vertices in `Lod::vertices`; None for a chunk this level does not help.
    ranges: Vec<Option<Range<u32>>>,
}

/// The simplified solid chunks of a model, in the order of the chunk plan's chunks.
#[derive(Default)]
pub struct Lod {
    /// The vertices of every level, level after level and chunk after chunk, so neighbours
    /// that share a level form one run.
    pub vertices: Vec<BaseFaceVertex>,
    levels: Vec<Level>,
}

/// What one chunk simplifies to at each level: the vertices, or None when it does not help.
type ChunkLevels = Vec<Option<Vec<BaseFaceVertex>>>;

impl Lod {
    /// Simplifies the solid chunks of `plan`, which orders `faces`.
    pub fn build(plan: &ChunkPlan, faces: &[BaseFaceVertex]) -> Lod {
        let count = plan.solid_chunks;
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(16);
        let chunk_levels = |k: usize| -> ChunkLevels { simplify_chunk(plan, faces, k) };
        let results: Vec<ChunkLevels> = if threads < 2 || count < 16 {
            (0..count).map(chunk_levels).collect()
        } else {
            let per = count.div_ceil(threads);
            std::thread::scope(|s| {
                let jobs: Vec<_> = (0..count)
                    .step_by(per)
                    .map(|from| {
                        let chunk_levels = &chunk_levels;
                        s.spawn(move || {
                            (from..(from + per).min(count))
                                .map(chunk_levels)
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                jobs.into_iter()
                    .flat_map(|j| j.join().expect("a simplifying thread panicked"))
                    .collect()
            })
        };
        let mut lod = Lod::default();
        for (l, &cell) in CELLS.iter().enumerate() {
            let ranges: Vec<Option<Range<u32>>> = results
                .iter()
                .map(|chunk| {
                    chunk[l].as_ref().map(|tris| {
                        let start = lod.vertices.len() as u32;
                        lod.vertices.extend_from_slice(tris);
                        start..lod.vertices.len() as u32
                    })
                })
                .collect();
            lod.levels.push(Level { cell, ranges });
        }
        lod
    }

    /// The vertices of the coarsest level of chunk `k` whose cells are at most `pixels` across
    /// where a pixel is `per_pixel` meters long, or None when none is.
    pub fn pick(&self, k: usize, per_pixel: f32, pixels: f32) -> Option<Range<u32>> {
        self.levels
            .iter()
            .rev()
            .filter(|level| level.cell <= pixels * per_pixel)
            .find_map(|level| level.ranges.get(k)?.clone())
    }
}

/// Simplifies chunk `k` to each level.
fn simplify_chunk(plan: &ChunkPlan, faces: &[BaseFaceVertex], k: usize) -> ChunkLevels {
    let chunk = &plan.chunks[k];
    let mut tris: Vec<BaseFaceVertex> =
        Vec::with_capacity((chunk.vertices.end - chunk.vertices.start) as usize);
    for run in plan.sources(chunk.vertices.start as usize..chunk.vertices.end as usize) {
        tris.extend_from_slice(&faces[run]);
    }
    let mut before = tris.len() / 3;
    CELLS
        .iter()
        .map(|&cell| {
            let simple = cluster(&tris, chunk.lo, cell);
            let now = simple.len() / 3;
            let worth = now as f32 <= WORTH * before as f32;
            before = now.min(before);
            worth.then_some(simple)
        })
        .collect()
}

/// A hasher for small integer keys: a multiplication, as `HashMap`'s default is slow for the
/// tens of millions of lookups a big model takes.
#[derive(Default)]
struct Mix(u64);

impl Hasher for Mix {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(u64::from(b));
        }
    }

    fn write_u32(&mut self, v: u32) {
        self.write_u64(u64::from(v));
    }

    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.0 ^= self.0 >> 29;
    }
}

type Map<K, V> = HashMap<K, V, BuildHasherDefault<Mix>>;

/// What the vertices in one cell add up to.
#[derive(Clone, Copy, Default)]
struct Cell {
    /// The sum of the positions and how many there are.
    pos: Vec3,
    corners: u32,
    /// The sums of the looks, each weighted by the area its corner stands for.
    weight: f32,
    front: [f32; 3],
    back: [f32; 3],
    finish: [f32; 4],
    glow: [f32; 2],
}

impl Cell {
    fn add(&mut self, v: &BaseFaceVertex, w: f32) {
        self.weight += w;
        for i in 0..3 {
            self.front[i] += w * v.front[i];
            self.back[i] += w * v.back[i];
        }
        for i in 0..4 {
            self.finish[i] += w * v.finish[i];
        }
        for i in 0..2 {
            self.glow[i] += w * v.glow[i];
        }
    }

    /// `v` with the cell's position and looks.
    fn put(&self, v: &mut BaseFaceVertex) {
        v.pos = (self.pos / self.corners as f32).to_array();
        if self.weight > 0.0 {
            let k = 1.0 / self.weight;
            v.front = self.front.map(|x| x * k);
            v.back = self.back.map(|x| x * k);
            v.finish = self.finish.map(|x| x * k);
            v.glow = self.glow.map(|x| x * k);
        }
    }
}

/// The triangles `tris` (three vertices each) clustered on the grid of `cell`-sized cells,
/// which `lo` (a corner of the triangles' box) fixes the numbering of.
pub fn cluster(tris: &[BaseFaceVertex], lo: Vec3, cell: f32) -> Vec<BaseFaceVertex> {
    let inv = 1.0 / cell;
    let base = (lo * inv).floor();
    let mut index: Map<u64, u32> = Map::default();
    let mut cells: Vec<Cell> = Vec::new();
    let mut cell_of: Vec<u32> = Vec::with_capacity(tris.len());
    for v in tris {
        let p = Vec3::from(v.pos);
        let q = (p * inv).floor() - base;
        let key = (q.x as i64 as u64 & 0x1F_FFFF)
            | (q.y as i64 as u64 & 0x1F_FFFF) << 21
            | (q.z as i64 as u64 & 0x1F_FFFF) << 42;
        let next = cells.len() as u32;
        let i = *index.entry(key).or_insert(next);
        if i == next {
            cells.push(Cell::default());
        }
        let c = &mut cells[i as usize];
        c.pos += p;
        c.corners += 1;
        cell_of.push(i);
    }
    // Every triangle, kept or not, lends its looks to its corners' cells.
    let area = |tri: &[BaseFaceVertex]| {
        let p = |v: &BaseFaceVertex| Vec3::from(v.pos);
        (p(&tri[1]) - p(&tri[0])).cross(p(&tri[2]) - p(&tri[0]))
    };
    for (t, tri) in tris.as_chunks::<3>().0.iter().enumerate() {
        let third = area(tri).length() / 6.0;
        for (j, v) in tri.iter().enumerate() {
            cells[cell_of[3 * t + j] as usize].add(v, third);
        }
    }

    let mut out: Vec<BaseFaceVertex> = Vec::with_capacity(tris.len() / 2);
    // Triangles already kept, by their cells, and whether they run the way the cells sort.
    let mut kept: Map<u64, (usize, bool)> = Map::default();
    let too_thin = (cell * cell * 1e-4).powi(2);
    for (t, tri) in tris.as_chunks::<3>().0.iter().enumerate() {
        let c = [cell_of[3 * t], cell_of[3 * t + 1], cell_of[3 * t + 2]];
        if c[0] == c[1] || c[1] == c[2] || c[0] == c[2] {
            continue;
        }
        let was = area(tri);
        if was.length_squared() <= 0.0 {
            continue;
        }
        let at = |i: usize| Vec3::from_array(position(&cells[c[i] as usize]));
        let mut now = (at(1) - at(0)).cross(at(2) - at(0));
        if now.length_squared() <= too_thin {
            continue;
        }
        // Keep the triangle facing the way it did: swap two corners if it has turned over.
        let mut order = [0usize, 1, 2];
        if now.dot(was) < 0.0 {
            order = [0, 2, 1];
            now = -now;
        }
        // The normal points to the side the old one did.
        let side =
            Vec3::from(tri[0].normal) + Vec3::from(tri[1].normal) + Vec3::from(tri[2].normal);
        let normal = (now.normalize() * if was.dot(side) < 0.0 { -1.0 } else { 1.0 }).to_array();

        let ids = order.map(|i| c[i]);
        let mut sorted = ids;
        sorted.sort_unstable();
        let forward = ids == sorted
            || ids == [sorted[1], sorted[2], sorted[0]]
            || ids == [sorted[2], sorted[0], sorted[1]];
        let key = u64::from(sorted[0]) << 42 | u64::from(sorted[1]) << 21 | u64::from(sorted[2]);
        if let Some((first, way)) = kept.get_mut(&key) {
            // Two triangles on the same cells: one stays. Opposite ones (a thin slab) leave
            // one that shows its front on both sides.
            if *way != forward {
                two_sided(&mut out[*first..*first + 3]);
                *way = forward;
            }
            continue;
        }
        kept.insert(key, (out.len(), forward));
        for i in order {
            let mut v = tri[i];
            cells[c[i] as usize].put(&mut v);
            v.normal = normal;
            out.push(v);
        }
    }
    out
}

fn position(cell: &Cell) -> [f32; 3] {
    (cell.pos / cell.corners as f32).to_array()
}

/// Makes the three vertices show their front side's look on the back too.
fn two_sided(tri: &mut [BaseFaceVertex]) {
    for v in tri {
        v.back = v.front;
        v.alpha[1] = v.alpha[0];
        v.uv = [v.uv[0], v.uv[1], v.uv[0], v.uv[1]];
        v.layer[1] = v.layer[0];
        v.glow[1] = v.glow[0];
        v.finish = [v.finish[0], v.finish[1], v.finish[0], v.finish[1]];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    fn corner(p: [f32; 3], front: [f32; 3]) -> BaseFaceVertex {
        BaseFaceVertex {
            pos: p,
            normal: [0.0, 0.0, 1.0],
            front,
            back: [0.5; 3],
            alpha: [1.0; 2],
            ..BaseFaceVertex::zeroed()
        }
    }

    /// `n` x `n` squares of `step` m on the ground, two triangles each, red where the square
    /// is in the first half along x and blue elsewhere.
    fn grid(n: usize, step: f32) -> Vec<BaseFaceVertex> {
        let mut out = Vec::new();
        for i in 0..n {
            for j in 0..n {
                let (x, y) = (i as f32 * step, j as f32 * step);
                let color = if i < n / 2 {
                    [1.0, 0.0, 0.0]
                } else {
                    [0.0, 0.0, 1.0]
                };
                let c = |x: f32, y: f32| corner([x, y, 0.0], color);
                let (a, b, d, e) = (
                    c(x, y),
                    c(x + step, y),
                    c(x + step, y + step),
                    c(x, y + step),
                );
                out.extend([a, b, d, a, d, e]);
            }
        }
        out
    }

    #[test]
    fn clustering_drops_degenerate_triangles_and_keeps_bounds_and_colors() {
        let tris = grid(40, 0.25);
        let simple = cluster(&tris, Vec3::ZERO, 1.0);
        assert!(!simple.is_empty());
        assert!(
            simple.len() * 8 < tris.len(),
            "{} of {}",
            simple.len(),
            tris.len()
        );
        for v in &simple {
            let p = Vec3::from(v.pos);
            assert!(p.cmpge(Vec3::splat(-1e-4)).all());
            assert!(p.cmple(Vec3::new(10.0, 10.0, 1e-4)).all());
            // Facing up, as the squares did.
            assert!(v.normal[2] > 0.99);
            // Red and blue only: no other color enters; a cell on the seam may mix them.
            assert!(v.front[1].abs() < 1e-4);
            assert!((v.front[0] + v.front[2] - 1.0).abs() < 1e-3);
        }
        // Far from the seam the colors are exact.
        let left = simple.iter().find(|v| v.pos[0] < 2.0).unwrap();
        assert!((left.front[0] - 1.0).abs() < 1e-4);
        let right = simple.iter().find(|v| v.pos[0] > 8.0).unwrap();
        assert!((right.front[2] - 1.0).abs() < 1e-4);
        // Every triangle still has three distinct places.
        for t in simple.chunks(3) {
            assert!(t[0].pos != t[1].pos && t[1].pos != t[2].pos && t[0].pos != t[2].pos);
        }
    }

    #[test]
    fn a_triangle_inside_one_cell_goes_and_a_big_one_stays_as_it_faced() {
        let small = [
            corner([0.1, 0.1, 0.0], [1.0; 3]),
            corner([0.2, 0.1, 0.0], [1.0; 3]),
            corner([0.1, 0.2, 0.0], [1.0; 3]),
        ];
        assert!(cluster(&small, Vec3::ZERO, 1.0).is_empty());
        // Wound the other way round, facing down.
        let mut big = [
            corner([0.0, 0.0, 0.0], [1.0; 3]),
            corner([0.0, 5.0, 0.0], [1.0; 3]),
            corner([5.0, 0.0, 0.0], [1.0; 3]),
        ];
        for v in &mut big {
            v.normal = [0.0, 0.0, -1.0];
        }
        let kept = cluster(&big, Vec3::ZERO, 1.0);
        assert_eq!(kept.len(), 3);
        assert!(kept.iter().all(|v| v.normal[2] < -0.99));
    }
}
