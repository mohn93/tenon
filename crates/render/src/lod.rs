//! Simplified copies of a model's opaque chunks, drawn in place of the full ones when their
//! detail would be about a pixel.
//!
//! Each chunk is simplified by vertex clustering (Rossignac and Borrel): space is cut into
//! cubic cells, the vertices of a cell become one, and the triangles that then have two
//! corners in one cell are dropped. The cells lie on one grid for all chunks and the sizes of
//! the levels nest, so each level is clustered from the one before it. What the dropped
//! triangles looked like is not lost: a cell's colors and finish are the area-weighted average
//! of everything in it, kept apart by the way the surface faces, and each kept triangle takes
//! its corners' from their cells.
//!
//! Left as they are, in every level: triangles that glow or show a texture (an average would
//! make lit windows vanish or the texture wrong), and triangles much longer than a cell (spires,
//! masts, strips), which clustering would drop whole.
//!
//! The cells of neighbouring chunks of different levels are not stitched: a gap up to the cell
//! size may open between them, which the choice of level keeps to about a pixel.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use glam::Vec3;

use crate::chunks::ChunkPlan;
use crate::scene::BaseFaceVertex;

/// The cell size of each level in meters, finest first. Each is a whole multiple of the one
/// before, so the cells nest.
pub const CELLS: [f32; 3] = [2.0, 4.0, 8.0];

/// A level is kept for a chunk only when it has at most this share of the triangles of the
/// level before it that is kept (or of the chunk itself): otherwise it costs memory and saves
/// little.
const WORTH: f32 = 0.65;

/// Triangles with a side this many cells long are kept as they are.
const LONG: f32 = 3.0;

/// How many pixels across a level's cells may be on the screen, by default.
pub const CELL_PIXELS: f32 = 2.0;

/// A level is entered at this share of the limit and left at the limit, so the choice does not
/// flicker as the view moves.
pub const ENTER: f32 = 0.8;

/// One level of simplification of every solid chunk.
struct Level {
    /// The cell size, in meters.
    cell: f32,
    /// Each chunk's vertices in `buffer`; None for a chunk this level does not help.
    ranges: Vec<Option<Range<u32>>>,
    buffer: Option<wgpu::Buffer>,
    vertices: u32,
}

/// The simplified solid chunks of a model, on the GPU, one vertex buffer a level.
pub struct Lod {
    levels: Vec<Level>,
    /// Zero flags for every level's vertices (nothing in them is selected or hovered).
    zeros: Option<wgpu::Buffer>,
}

/// What one chunk simplifies to at each level: the vertices, or None when it does not help.
type ChunkLevels = Vec<Option<Vec<BaseFaceVertex>>>;

impl Lod {
    /// Simplifies the solid chunks of `plan`, which orders `faces`, and uploads them, a level
    /// at a time, leaving out one over `limit` bytes; None when `cancel` is set meanwhile.
    /// Written for a worker thread.
    pub fn build(
        device: &wgpu::Device,
        plan: &ChunkPlan,
        faces: &[BaseFaceVertex],
        cancel: &AtomicBool,
        limit: u64,
    ) -> Option<Lod> {
        let count = plan.solid_chunks;
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(16);
        let chunk_levels = |k: usize| -> ChunkLevels { simplify_chunk(plan, faces, k, cancel) };
        let mut results: Vec<ChunkLevels> = if threads < 2 || count < 16 {
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
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        // One level at a time goes into its buffer, and its chunks' vertices are freed.
        let stride = std::mem::size_of::<BaseFaceVertex>();
        let mut levels = Vec::new();
        for (l, &cell) in CELLS.iter().enumerate() {
            let total: usize = results
                .iter()
                .map(|c| c[l].as_ref().map_or(0, Vec::len))
                .sum();
            let mut ranges = vec![None; count];
            let mut buffer = None;
            // A level too big for a buffer is left out.
            let total = if (total * stride) as u64 > limit {
                for chunk in &mut results {
                    chunk[l] = None;
                }
                0
            } else {
                total
            };
            if total > 0 {
                let b = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("lod faces"),
                    size: (total * stride) as u64,
                    usage: wgpu::BufferUsages::VERTEX,
                    mapped_at_creation: true,
                });
                {
                    let mut view = b
                        .slice(..)
                        .get_mapped_range_mut()
                        .expect("the buffer was created mapped");
                    let mut at = 0;
                    for (k, chunk) in results.iter_mut().enumerate() {
                        if let Some(v) = chunk[l].take() {
                            view.slice(at * stride..(at + v.len()) * stride)
                                .copy_from_slice(bytemuck::cast_slice(&v));
                            ranges[k] = Some(at as u32..(at + v.len()) as u32);
                            at += v.len();
                        }
                    }
                }
                b.unmap();
                buffer = Some(b);
            }
            levels.push(Level {
                cell,
                ranges,
                buffer,
                vertices: total as u32,
            });
        }
        let most = levels.iter().map(|l| l.vertices).max().unwrap_or(0);
        let zeros = (most > 0 && u64::from(most) * 4 <= limit).then(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lod flags"),
                size: u64::from(most) * std::mem::size_of::<f32>() as u64,
                usage: wgpu::BufferUsages::VERTEX,
                mapped_at_creation: false,
            })
        });
        Some(Lod { levels, zeros })
    }

    /// The level to draw chunk `k` at, with its vertices, where a pixel is `per_pixel` meters
    /// long and cells may be `pixels` across; and the level to remember for the next frame
    /// (0: none, else the level plus one). `last` is the one remembered.
    ///
    /// A level is chosen when its cells are at most `ENTER` of the limit, and kept while they
    /// are within the limit.
    pub fn pick(
        &self,
        k: usize,
        per_pixel: f32,
        pixels: f32,
        last: u8,
    ) -> (u8, Option<(usize, Range<u32>)>) {
        let limit = pixels * per_pixel;
        let at = |l: usize| self.levels[l].ranges.get(k)?.clone();
        let fresh = (0..self.levels.len())
            .rev()
            .find(|&l| self.levels[l].cell <= ENTER * limit && at(l).is_some());
        let kept = usize::from(last)
            .checked_sub(1)
            .filter(|&l| self.levels[l].cell <= limit && at(l).is_some());
        let level = match (fresh, kept) {
            (Some(f), Some(c)) => Some(f.max(c)),
            (f, c) => f.or(c),
        };
        match level {
            Some(l) => (l as u8 + 1, at(l).map(|r| (l, r))),
            None => (0, None),
        }
    }

    /// The vertex buffer of `level` and the zero flag buffer to draw it with.
    pub fn buffers(&self, level: usize) -> Option<(&wgpu::Buffer, &wgpu::Buffer)> {
        Some((
            self.levels.get(level)?.buffer.as_ref()?,
            self.zeros.as_ref()?,
        ))
    }

    /// The bytes of vertices each level holds.
    pub fn level_bytes(&self) -> Vec<u64> {
        let stride = std::mem::size_of::<BaseFaceVertex>() as u64;
        self.levels
            .iter()
            .map(|l| u64::from(l.vertices) * stride)
            .collect()
    }
}

/// Simplifies chunk `k` to each level.
fn simplify_chunk(
    plan: &ChunkPlan,
    faces: &[BaseFaceVertex],
    k: usize,
    cancel: &AtomicBool,
) -> ChunkLevels {
    let none = || CELLS.iter().map(|_| None).collect();
    if cancel.load(Ordering::Relaxed) {
        return none();
    }
    let chunk = &plan.chunks[k];
    let (mut plain, mut kept) = (Vec::new(), Vec::new());
    for run in plan.sources(chunk.vertices.start as usize..chunk.vertices.end as usize) {
        for tri in faces[run].as_chunks::<3>().0 {
            let list = if stays_whole(tri) {
                &mut kept
            } else {
                &mut plain
            };
            list.extend_from_slice(tri);
        }
    }
    let mut before = (plain.len() + kept.len()) / 3;
    CELLS
        .iter()
        .map(|&cell| {
            plain = cluster(&plain, cell);
            let now = (plain.len() + kept.len()) / 3;
            let worth = now as f32 <= WORTH * before as f32;
            if worth {
                before = now;
            }
            worth.then(|| {
                let mut all = plain.clone();
                all.extend_from_slice(&kept);
                all
            })
        })
        .collect()
}

/// True for a triangle that glows or shows a texture on either side.
fn stays_whole(tri: &[BaseFaceVertex; 3]) -> bool {
    tri.iter()
        .any(|v| v.glow[0] > 0.0 || v.glow[1] > 0.0 || v.layer[0] >= 0.0 || v.layer[1] >= 0.0)
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
    /// The looks of the surfaces facing each of the six ways.
    looks: [Look; 6],
}

/// Sums of looks, each weighted by the area its corner stands for.
#[derive(Clone, Copy, Default)]
struct Look {
    weight: f32,
    front: [f32; 3],
    back: [f32; 3],
    finish: [f32; 4],
}

impl Cell {
    fn add(&mut self, side: usize, v: &BaseFaceVertex, w: f32) {
        let l = &mut self.looks[side];
        l.weight += w;
        for i in 0..3 {
            l.front[i] += w * v.front[i];
            l.back[i] += w * v.back[i];
        }
        for i in 0..4 {
            l.finish[i] += w * v.finish[i];
        }
    }

    fn position(&self) -> Vec3 {
        self.pos / self.corners as f32
    }

    /// `v` with the cell's position and the look of the surfaces facing `side`.
    fn put(&self, side: usize, v: &mut BaseFaceVertex) {
        v.pos = self.position().to_array();
        let l = &self.looks[side];
        if l.weight > 0.0 {
            let k = 1.0 / l.weight;
            v.front = l.front.map(|x| x * k);
            v.back = l.back.map(|x| x * k);
            v.finish = l.finish.map(|x| x * k);
        }
    }
}

/// Which of the six directions (axis by axis, positive then negative) `n` points most to.
fn side_of(n: Vec3) -> usize {
    let a = n.abs();
    let axis = if a.x >= a.y && a.x >= a.z {
        0
    } else if a.y >= a.z {
        1
    } else {
        2
    };
    2 * axis + usize::from(n[axis] < 0.0)
}

/// The triangles `tris` (three vertices each) clustered on the grid of `cell`-sized cells
/// that has a corner at the origin.
pub fn cluster(tris: &[BaseFaceVertex], cell: f32) -> Vec<BaseFaceVertex> {
    let inv = 1.0 / cell;
    let low = tris
        .iter()
        .fold(Vec3::splat(f32::MAX), |m, v| m.min(Vec3::from(v.pos)));
    let base = (low * inv).floor();
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
    let area = |tri: &[BaseFaceVertex]| {
        let p = |v: &BaseFaceVertex| Vec3::from(v.pos);
        (p(&tri[1]) - p(&tri[0])).cross(p(&tri[2]) - p(&tri[0]))
    };
    // The side each triangle faces, by its front.
    let facing = |tri: &[BaseFaceVertex], was: Vec3| {
        let n = Vec3::from(tri[0].normal) + Vec3::from(tri[1].normal) + Vec3::from(tri[2].normal);
        if was.dot(n) < 0.0 { -was } else { was }
    };
    // Every triangle, kept or not, lends its looks to its corners' cells.
    let mut sides: Vec<u8> = Vec::with_capacity(tris.len() / 3);
    for (t, tri) in tris.as_chunks::<3>().0.iter().enumerate() {
        let was = area(tri);
        let side = side_of(facing(tri, was));
        sides.push(side as u8);
        let third = was.length() / 6.0;
        for (j, v) in tri.iter().enumerate() {
            cells[cell_of[3 * t + j] as usize].add(side, v, third);
        }
    }

    let mut out: Vec<BaseFaceVertex> = Vec::with_capacity(tris.len() / 2);
    // Triangles already kept, by their cells, and whether they run the way the cells sort.
    let mut kept: Map<u64, (usize, bool)> = Map::default();
    let too_thin = (cell * cell * 1e-4).powi(2);
    for (t, tri) in tris.as_chunks::<3>().0.iter().enumerate() {
        let c = [cell_of[3 * t], cell_of[3 * t + 1], cell_of[3 * t + 2]];
        let was = area(tri);
        if was.length_squared() <= 0.0 {
            continue;
        }
        if c[0] == c[1] || c[1] == c[2] || c[0] == c[2] {
            // A triangle with a long side would vanish: it stays as it is.
            let p = |i: usize| Vec3::from(tri[i].pos);
            let longest = (p(1) - p(0))
                .length()
                .max((p(2) - p(1)).length())
                .max((p(0) - p(2)).length());
            if longest >= LONG * cell {
                out.extend_from_slice(tri);
            }
            continue;
        }
        let at = |i: usize| cells[c[i] as usize].position();
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
        let flip = facing(tri, was).dot(was) < 0.0;
        let normal = (now.normalize() * if flip { -1.0 } else { 1.0 }).to_array();

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
            cells[c[i] as usize].put(usize::from(sides[t]), &mut v);
            v.normal = normal;
            out.push(v);
        }
    }
    out
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
            layer: [-1.0; 2],
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
        let simple = cluster(&tris, 1.0);
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
        assert!(cluster(&small, 1.0).is_empty());
        // Wound the other way round, facing down.
        let mut big = [
            corner([0.0, 0.0, 0.0], [1.0; 3]),
            corner([0.0, 5.0, 0.0], [1.0; 3]),
            corner([5.0, 0.0, 0.0], [1.0; 3]),
        ];
        for v in &mut big {
            v.normal = [0.0, 0.0, -1.0];
        }
        let kept = cluster(&big, 1.0);
        assert_eq!(kept.len(), 3);
        assert!(kept.iter().all(|v| v.normal[2] < -0.99));
    }

    #[test]
    fn a_thin_triangle_longer_than_a_few_cells_stays() {
        let spire = [
            corner([0.1, 0.1, 0.0], [1.0; 3]),
            corner([0.2, 0.1, 0.0], [1.0; 3]),
            corner([0.15, 0.1, 10.0], [1.0; 3]),
        ];
        let kept = cluster(&spire, 1.0);
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[2].pos, [0.15, 0.1, 10.0]);
    }

    #[test]
    fn glowing_and_textured_triangles_stay_whole_in_every_level() {
        let mut tris = grid(40, 0.25);
        for v in &mut tris[300..303] {
            v.glow = [1.0, 0.0];
        }
        for v in &mut tris[600..603] {
            v.layer = [2.0, -1.0];
        }
        let lit: Vec<_> = tris[300..303].iter().map(|v| v.pos).collect();
        let textured: Vec<_> = tris[600..603].iter().map(|v| v.pos).collect();
        let plan = ChunkPlan::new(tris.len(), &[], 3, 100_000, &|i| tris[i].pos, &|_| true);
        assert_eq!(plan.chunks.len(), 1);
        let levels = simplify_chunk(&plan, &tris, 0, &AtomicBool::new(false));
        let level = levels[0].as_ref().expect("the first level helps");
        assert!(level.len() * 4 < tris.len());
        let has = |positions: &[[f32; 3]]| {
            level
                .as_chunks::<3>()
                .0
                .iter()
                .any(|t| t.iter().map(|v| v.pos).eq(positions.iter().copied()))
        };
        assert!(has(&lit) && has(&textured));
        let lit_vertex = level.iter().find(|v| v.glow[0] > 0.5).unwrap();
        assert_eq!(lit_vertex.glow, [1.0, 0.0]);
    }

    fn levels_of_chunk_zero() -> Lod {
        let level = |cell: f32| Level {
            cell,
            ranges: vec![Some(0..3)],
            buffer: None,
            vertices: 3,
        };
        Lod {
            levels: vec![level(1.0), level(3.0), level(9.0)],
            zeros: None,
        }
    }

    #[test]
    fn a_level_is_entered_at_a_part_of_the_limit_and_kept_up_to_it() {
        let lod = levels_of_chunk_zero();
        // The limit is 1.5 cells per pixel of `per_pixel` meters.
        let level = |per_pixel: f32, last: u8| lod.pick(0, per_pixel, 1.5, last).0;
        assert_eq!(level(0.5, 0), 0, "a pixel of half a meter: nothing to gain");
        assert_eq!(
            level(2.0, 0),
            1,
            "the finest enters at 1.2 m cells (2.4 limit)"
        );
        assert_eq!(level(2.6, 0), 2, "the next enters at 3.12");
        // Once in, it stays while within the limit (3.15) and goes finer past it.
        assert_eq!(level(2.1, 2), 2);
        assert_eq!(level(1.9, 2), 1);
        // And a coarser one is chosen at once when it enters.
        assert_eq!(level(2.6, 1), 2);
    }

    #[test]
    fn surfaces_facing_different_ways_keep_their_own_colors() {
        // A red floor and a blue wall meeting in the same cells.
        let mut tris = Vec::new();
        let floor = |c: [f32; 3]| {
            let v = |x: f32, y: f32| corner([x, y, 0.0], c);
            [v(0.0, 0.0), v(2.0, 0.0), v(0.0, 2.0)]
        };
        let wall = |c: [f32; 3]| {
            let v = |x: f32, z: f32| {
                let mut v = corner([x, 0.0, z], c);
                v.normal = [0.0, -1.0, 0.0];
                v
            };
            // Wound to face -y.
            [v(0.0, 0.0), v(0.0, 2.0), v(2.0, 0.0)]
        };
        tris.extend(floor([1.0, 0.0, 0.0]));
        tris.extend(wall([0.0, 0.0, 1.0]));
        let simple = cluster(&tris, 1.0);
        let floors: Vec<_> = simple.iter().filter(|v| v.normal[2] > 0.5).collect();
        let walls: Vec<_> = simple.iter().filter(|v| v.normal[1] < -0.5).collect();
        assert_eq!((floors.len(), walls.len()), (3, 3));
        assert!(floors.iter().all(|v| v.front == [1.0, 0.0, 0.0]));
        assert!(walls.iter().all(|v| v.front == [0.0, 0.0, 1.0]));
    }
}
