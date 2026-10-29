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
//! masts, strips), which clustering would drop whole. These are the same in every level, so
//! they are kept once per chunk as full vertices in a buffer of their own, drawn with the
//! ordinary face pipeline (and a buffer of zero flags) beside the chunk's simplified level.
//!
//! Cracks: each chunk is simplified alone, so the cell averages of two neighbours differ and a
//! surface running from one into the other would tear open. Vertices that must not move are
//! therefore pinned, each its own cluster at its exact position: those at a place where
//! vertices of two or more chunks lie (`Borders`), and those where a triangle kept whole has a
//! corner. Every edge shared by two chunks, or by a kept and a simplified triangle, has
//! pinned ends, so it is the same segment in a chunk at any level and in a full one.
//!
//! T-junctions: a vertex of one chunk can lie in the middle of an edge of another (a long wall
//! edge beside a subdivided neighbour), so the position is not shared. The ends of each chunk's
//! open edges (those one of its triangles uses alone: where its surface goes on in another chunk)
//! are put in grids, every edge of every chunk is looked up among them, and a point of another
//! chunk lying on the edge is pinned with the edge's ends. The tolerance is 0.5 mm and positions
//! are f32, so it stops working a few km from the origin. A dense model is searched up to a bound
//! of work and then left as it is (with a line on stderr).
//!
//! The simplified levels have their own small vertex (`LodVertex`): nothing in them is
//! textured, glowing or see-through, and a chunk with something selected or hovered is drawn
//! whole, so they carry no flags.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use crate::chunks::ChunkPlan;
use crate::scene::BaseFaceVertex;

/// A corner of a simplified chunk, 28 bytes: where it is, which way the surface faces, the
/// colors of its two sides and their finishes (metalness and roughness, front then back).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LodVertex {
    pub pos: [f32; 3],
    /// Signed bytes, a unit vector scaled by 127 (the fourth is 0).
    pub normal: [i8; 4],
    /// Bytes, 255 for 1 (the fourth is 0).
    pub front: [u8; 4],
    pub back: [u8; 4],
    pub finish: [u8; 4],
}

impl LodVertex {
    /// `v` as a simplified chunk keeps it.
    pub fn pack(v: &BaseFaceVertex) -> LodVertex {
        let unit = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        let n = Vec3::from(v.normal).normalize_or_zero() * 127.0;
        LodVertex {
            pos: v.pos,
            normal: [n.x.round() as i8, n.y.round() as i8, n.z.round() as i8, 0],
            front: [unit(v.front[0]), unit(v.front[1]), unit(v.front[2]), 0],
            back: [unit(v.back[0]), unit(v.back[1]), unit(v.back[2]), 0],
            finish: v.finish.map(unit),
        }
    }
}

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
    /// The triangles each chunk keeps whole, full vertices, in one buffer.
    kept: Option<wgpu::Buffer>,
    kept_ranges: Vec<Option<Range<u32>>>,
    /// Zero flags for `kept`'s vertices (nothing in them is selected or hovered).
    zeros: Option<wgpu::Buffer>,
}

/// What one chunk simplifies to at each level: the vertices, or None when it does not help.
type ChunkLevels = Vec<Option<Vec<LodVertex>>>;

/// A chunk's levels, and the triangles it keeps whole.
type Simplified = (ChunkLevels, Vec<BaseFaceVertex>);

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
        let borders = Borders::find(plan, faces, threads, cancel);
        let chunk_levels =
            |k: usize| -> Simplified { simplify_chunk(plan, faces, k, &borders, cancel) };
        let mut results: Vec<Simplified> = if threads < 2 || count < 16 {
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
        // The kept triangles go into one buffer, with the flags that go with them.
        let full = std::mem::size_of::<BaseFaceVertex>();
        let kept_total: usize = results.iter().map(|c| c.1.len()).sum();
        if (kept_total * (full + std::mem::size_of::<f32>())) as u64 > limit {
            return None;
        }
        let mut kept_ranges = vec![None; count];
        let (mut kept, mut zeros) = (None, None);
        if kept_total > 0 {
            let b = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lod kept faces"),
                size: (kept_total * full) as u64,
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
                    let v = std::mem::take(&mut chunk.1);
                    if !v.is_empty() {
                        view.slice(at * full..(at + v.len()) * full)
                            .copy_from_slice(bytemuck::cast_slice(&v));
                        kept_ranges[k] = Some(at as u32..(at + v.len()) as u32);
                        at += v.len();
                    }
                }
            }
            b.unmap();
            kept = Some(b);
            zeros = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lod flags"),
                size: (kept_total * std::mem::size_of::<f32>()) as u64,
                usage: wgpu::BufferUsages::VERTEX,
                mapped_at_creation: false,
            }));
        }
        // One level at a time goes into its buffer, and its chunks' vertices are freed.
        let stride = std::mem::size_of::<LodVertex>();
        let mut levels = Vec::new();
        for (l, &cell) in CELLS.iter().enumerate() {
            let total: usize = results
                .iter()
                .map(|c| c.0[l].as_ref().map_or(0, Vec::len))
                .sum();
            let mut ranges = vec![None; count];
            let mut buffer = None;
            // A level too big for a buffer is left out.
            let total = if (total * stride) as u64 > limit {
                for chunk in &mut results {
                    chunk.0[l] = None;
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
                        if let Some(v) = chunk.0[l].take() {
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
        Some(Lod {
            levels,
            kept,
            kept_ranges,
            zeros,
        })
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

    /// The triangles chunk `k` keeps whole, drawn with its simplified level.
    pub fn kept(&self, k: usize) -> Option<Range<u32>> {
        self.kept_ranges.get(k)?.clone()
    }

    /// The buffer of kept triangles and the zero flags to draw it with.
    pub fn kept_buffers(&self) -> Option<(&wgpu::Buffer, &wgpu::Buffer)> {
        Some((self.kept.as_ref()?, self.zeros.as_ref()?))
    }

    /// The vertex buffer of `level`.
    pub fn buffer(&self, level: usize) -> Option<&wgpu::Buffer> {
        self.levels.get(level)?.buffer.as_ref()
    }

    /// The bytes the copies take on the GPU.
    pub fn gpu_bytes(&self) -> u64 {
        self.levels
            .iter()
            .filter_map(|l| l.buffer.as_ref())
            .chain(self.kept.as_ref())
            .chain(self.zeros.as_ref())
            .map(|b| b.size())
            .sum()
    }

    /// The bytes of vertices each level holds.
    pub fn level_bytes(&self) -> Vec<u64> {
        let stride = std::mem::size_of::<LodVertex>() as u64;
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
    borders: &Borders,
    cancel: &AtomicBool,
) -> Simplified {
    let none = || (CELLS.iter().map(|_| None).collect(), Vec::new());
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
    // Where the kept triangles have corners, and where the chunk meets others, nothing moves.
    let corners: Map<[u32; 3], ()> = kept.iter().map(|v| (bits(v.pos), ())).collect();
    let pinned = |v: &BaseFaceVertex| borders.contains(v.pos) || corners.contains_key(&bits(v.pos));
    let mut before = (plain.len() + kept.len()) / 3;
    let levels = CELLS
        .iter()
        .map(|&cell| {
            plain = cluster_pinned(&plain, cell, &pinned);
            let now = (plain.len() + kept.len()) / 3;
            let worth = now as f32 <= WORTH * before as f32;
            if worth {
                before = now;
            }
            (worth && !plain.is_empty())
                .then(|| plain.iter().map(LodVertex::pack).collect::<Vec<_>>())
        })
        .collect();
    (levels, kept)
}

fn bits(p: [f32; 3]) -> [u32; 3] {
    p.map(f32::to_bits)
}

/// The places where vertices of more than one chunk lie.
pub struct Borders {
    places: Map<[u32; 3], ()>,
}

impl Borders {
    /// The places shared by the chunks whose vertex places are `chunks` (see `places`).
    pub fn new(chunks: impl IntoIterator<Item = Vec<[u32; 3]>>) -> Borders {
        let mut all: Vec<[u32; 3]> = chunks.into_iter().flatten().collect();
        all.sort_unstable();
        let mut places = Map::default();
        for pair in all.windows(2) {
            if pair[0] == pair[1] {
                places.insert(pair[0], ());
            }
        }
        Borders { places }
    }

    /// The places shared by the chunks of `plan` (the solid ones and the others), found on
    /// `threads` threads. Partial when `cancel` is set meanwhile (the build is then dropped).
    fn find(
        plan: &ChunkPlan,
        faces: &[BaseFaceVertex],
        threads: usize,
        cancel: &AtomicBool,
    ) -> Borders {
        let corners = |k: usize| -> Vec<[f32; 3]> {
            let c = &plan.chunks[k];
            let mut at: Vec<[f32; 3]> = Vec::new();
            for run in plan.sources(c.vertices.start as usize..c.vertices.end as usize) {
                at.extend(faces[run].iter().map(|v| v.pos));
            }
            at
        };
        let count = plan.chunks.len();
        let shared = par_map(count, threads, &|k| {
            if cancel.load(Ordering::Relaxed) {
                return Vec::new();
            }
            places(corners(k).into_iter())
        });
        let mut borders = Borders::new(shared);
        borders.pin_t_junctions(count, &corners, threads, cancel);
        borders
    }

    /// Pins the T-junctions between chunks (see the module doc): `corners(k)` gives the
    /// triangle corners of chunk `k` of `count`, three to a triangle. Gives up, leaving the
    /// pins found so far, when `cancel` is set or the work passes a bound.
    pub fn pin_t_junctions(
        &mut self,
        count: usize,
        corners: &(dyn Fn(usize) -> Vec<[f32; 3]> + Sync),
        threads: usize,
        cancel: &AtomicBool,
    ) {
        for p in t_junctions(count, corners, threads, cancel, WORK_LIMIT) {
            self.places.insert(p, ());
        }
    }

    /// True when vertices of more than one chunk lie at `p`.
    fn contains(&self, p: [f32; 3]) -> bool {
        !self.places.is_empty() && self.places.contains_key(&bits(p))
    }
}

/// `f(0)` to `f(count - 1)`, in order, computed on `threads` threads.
fn par_map<T: Send>(count: usize, threads: usize, f: &(dyn Fn(usize) -> T + Sync)) -> Vec<T> {
    if threads < 2 || count < 16 {
        return (0..count).map(f).collect();
    }
    let per = count.div_ceil(threads);
    std::thread::scope(|s| {
        let jobs: Vec<_> = (0..count)
            .step_by(per)
            .map(|from| s.spawn(move || (from..(from + per).min(count)).map(f).collect::<Vec<_>>()))
            .collect();
        jobs.into_iter()
            .flat_map(|j| j.join().expect("a border finding thread panicked"))
            .collect()
    })
}

/// A straight edge, its two ends.
pub type Seg = [[f32; 3]; 2];

/// How far from an edge a vertex may be and still count as on it, in meters. Positions are
/// f32, so this stops working a few km from the origin (a step of the last bit is then
/// 0.5 mm and more): vertices there are not found on edges.
const ON_EDGE: f64 = 5e-4;

/// The smallest cell of the grids T-junctions are looked up in, in meters, and how many
/// grids there are (each cell four times the one before: the last is about 17 km). An edge is
/// looked up in the finest grid whose cell is at least its size plus the tolerance on both
/// sides, so a few cells hold it, and the points in them are about as dense as the edges.
const FINEST: f32 = 1e-3;
const LEVELS: usize = 13;

/// The most points tested against edges before the search is given up.
const WORK_LIMIT: u64 = 400_000_000;

/// How many tests are made between looks at the bound and at `cancel`.
const CHECK_EVERY: u64 = 65_536;

/// The grid for an edge with a box of `extent` m at most: `LEVELS` for one too long for any.
fn level_of(extent: f32) -> usize {
    let need = extent + 2.0 * ON_EDGE as f32;
    let mut cell = FINEST;
    for level in 0..LEVELS {
        if cell >= need {
            return level;
        }
        cell *= 4.0;
    }
    LEVELS
}

fn cell_size(level: usize) -> f32 {
    FINEST * 4f32.powi(level as i32)
}

fn cell_at(p: f32, size: f32) -> i32 {
    (p / size).floor() as i32
}

/// The edges of the triangles whose corners are `corners` that only one triangle uses (by
/// position): where a chunk's surface ends, or the other side is in another chunk.
pub fn open_edges(corners: &[[f32; 3]]) -> Vec<Seg> {
    let mut all: Vec<([u32; 3], [u32; 3])> = Vec::with_capacity(corners.len());
    for t in corners.as_chunks::<3>().0 {
        for i in 0..3 {
            let (a, b) = (bits(t[i]), bits(t[(i + 1) % 3]));
            if a != b {
                all.push(if a < b { (a, b) } else { (b, a) });
            }
        }
    }
    all.sort_unstable();
    let mut out = Vec::new();
    let mut i = 0;
    while i < all.len() {
        let mut j = i + 1;
        while j < all.len() && all[j] == all[i] {
            j += 1;
        }
        if j == i + 1 {
            let unbits = |b: [u32; 3]| b.map(f32::from_bits);
            out.push([unbits(all[i].0), unbits(all[i].1)]);
        }
        i = j;
    }
    out
}

/// What a chunk contributes to the search: the ends of its open edges (each once), and the
/// grids its edges would be looked up in.
fn ends_and_levels(corners: &[[f32; 3]]) -> (Vec<[f32; 3]>, u32) {
    let mut ends: Vec<[f32; 3]> = open_edges(corners).into_iter().flatten().collect();
    ends.sort_unstable_by_key(|p| bits(*p));
    ends.dedup_by_key(|p| bits(*p));
    let mut levels = 0u32;
    for t in corners.as_chunks::<3>().0 {
        for i in 0..3 {
            let (a, b) = (t[i], t[(i + 1) % 3]);
            let extent = (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0f32, f32::max);
            levels |= 1 << level_of(extent);
        }
    }
    (ends, levels)
}

/// The places to pin so that chunks stay together where a vertex of one lies on an edge of
/// another: such a vertex ends an open edge of its chunk (the surface goes on in the other
/// chunk), and the edge it lies on is any edge of the other chunk (it may be used by two
/// triangles there, a wall standing on a seam). Pinned are the vertex and that edge's ends.
fn t_junctions(
    count: usize,
    corners: &(dyn Fn(usize) -> Vec<[f32; 3]> + Sync),
    threads: usize,
    cancel: &AtomicBool,
    limit: u64,
) -> Vec<[u32; 3]> {
    let found = par_map(count, threads, &|k| {
        if cancel.load(Ordering::Relaxed) {
            (Vec::new(), 0)
        } else {
            ends_and_levels(&corners(k))
        }
    });
    let mut levels = 0u32;
    let mut points: Vec<(u32, [f32; 3])> = Vec::new();
    for (k, (ends, l)) in found.into_iter().enumerate() {
        levels |= l;
        points.extend(ends.into_iter().map(|p| (k as u32, p)));
    }
    if points.is_empty() || cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }
    // Every point in each grid some edge is looked up in.
    let mut grids: Vec<Map<[i32; 3], Vec<u32>>> = (0..LEVELS).map(|_| Map::default()).collect();
    for (level, grid) in grids.iter_mut().enumerate() {
        if levels & (1 << level) != 0 {
            let size = cell_size(level);
            for (n, (_, p)) in points.iter().enumerate() {
                let key = [0, 1, 2].map(|i| cell_at(p[i], size));
                grid.entry(key).or_default().push(n as u32);
            }
        }
    }
    let work = AtomicU64::new(0);
    let pins = par_map(count, threads, &|k| {
        let mut pins = Vec::new();
        if work.load(Ordering::Relaxed) > limit || cancel.load(Ordering::Relaxed) {
            return pins;
        }
        let corners = corners(k);
        let tested = std::cell::Cell::new(0u64);
        let stop = std::cell::Cell::new(false);
        // Adds the tests since the last call to the total; true when the bound or cancel
        // says to stop.
        let flush = || {
            let done = tested.replace(0);
            if work.fetch_add(done, Ordering::Relaxed) + done > limit
                || cancel.load(Ordering::Relaxed)
            {
                stop.set(true);
            }
            stop.get()
        };
        let test = |n: u32, a: [f32; 3], b: [f32; 3], pins: &mut Vec<[u32; 3]>| {
            if stop.get() {
                return;
            }
            let (c, p) = points[n as usize];
            tested.set(tested.get() + 1);
            if tested.get() >= CHECK_EVERY && flush() {
                return;
            }
            if c as usize != k && p != a && p != b && distance_to_segment(p, [a, b]) <= ON_EDGE {
                pins.extend([bits(p), bits(a), bits(b)]);
            }
        };
        for (i, t) in corners.as_chunks::<3>().0.iter().enumerate() {
            if i % 4096 == 0 && flush() {
                return pins;
            }
            for j in 0..3 {
                let (a, b) = (t[j], t[(j + 1) % 3]);
                if a == b {
                    continue;
                }
                let extent = (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0f32, f32::max);
                let level = level_of(extent);
                if level == LEVELS {
                    // Longer than any grid: every point is looked at.
                    for n in 0..points.len() as u32 {
                        test(n, a, b, &mut pins);
                        if stop.get() {
                            return pins;
                        }
                    }
                    continue;
                }
                let size = cell_size(level);
                let reach = ON_EDGE as f32;
                let lo = [0, 1, 2].map(|i| cell_at(a[i].min(b[i]) - reach, size));
                let hi = [0, 1, 2].map(|i| cell_at(a[i].max(b[i]) + reach, size));
                for x in lo[0]..=hi[0] {
                    for y in lo[1]..=hi[1] {
                        for z in lo[2]..=hi[2] {
                            if let Some(list) = grids[level].get(&[x, y, z]) {
                                for &n in list {
                                    test(n, a, b, &mut pins);
                                }
                                if stop.get() {
                                    return pins;
                                }
                            }
                        }
                    }
                }
            }
        }
        flush();
        pins
    });
    if work.load(Ordering::Relaxed) > limit {
        eprintln!("lod: T-junction search given up after {limit} tests (dense geometry)");
    }
    pins.into_iter().flatten().collect()
}

/// How far `p` is from the segment `e`.
fn distance_to_segment(p: [f32; 3], e: Seg) -> f64 {
    let d = |v: [f32; 3]| glam::DVec3::new(f64::from(v[0]), f64::from(v[1]), f64::from(v[2]));
    let (a, b, p) = (d(e[0]), d(e[1]), d(p));
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 > 0.0 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p - (a + ab * t)).length()
}

/// The places `positions` are at, each once, sorted.
pub fn places(positions: impl Iterator<Item = [f32; 3]>) -> Vec<[u32; 3]> {
    let mut keys: Vec<[u32; 3]> = positions.map(bits).collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// True when the box `lo` to `hi` lies within the largest cell of one of `planes`, or crosses it:
/// where the simplified surface could part from what is built from the whole one.
pub fn near_planes(lo: Vec3, hi: Vec3, planes: &[kernel::SectionPlane]) -> bool {
    let reach = f64::from(CELLS[CELLS.len() - 1]);
    let (mid, half) = ((lo + hi).as_dvec3() * 0.5, (hi - lo).as_dvec3() * 0.5);
    planes.iter().any(|p| {
        let n = p.normal;
        let spread = half.x * n.x.abs() + half.y * n.y.abs() + half.z * n.z.abs();
        (n.dot(mid - p.point)).abs() <= spread + reach
    })
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
    /// Where the cell's one vertex is, when it is pinned there.
    exact: Option<Vec3>,
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
        self.exact.unwrap_or(self.pos / self.corners as f32)
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
#[cfg(test)]
fn cluster(tris: &[BaseFaceVertex], cell: f32) -> Vec<BaseFaceVertex> {
    cluster_pinned(tris, cell, &|_| false)
}

/// Like `cluster`, but a vertex for which `pinned` is true stays where it is, as its own
/// cluster with the others at the same place.
pub fn cluster_pinned(
    tris: &[BaseFaceVertex],
    cell: f32,
    pinned: &impl Fn(&BaseFaceVertex) -> bool,
) -> Vec<BaseFaceVertex> {
    let inv = 1.0 / cell;
    let low = tris
        .iter()
        .fold(Vec3::splat(f32::MAX), |m, v| m.min(Vec3::from(v.pos)));
    let base = (low * inv).floor();
    let mut index: Map<u64, u32> = Map::default();
    let mut fixed: Map<[u32; 3], u32> = Map::default();
    let mut cells: Vec<Cell> = Vec::new();
    let mut cell_of: Vec<u32> = Vec::with_capacity(tris.len());
    for v in tris {
        let p = Vec3::from(v.pos);
        if pinned(v) {
            let next = cells.len() as u32;
            let i = *fixed.entry(bits(v.pos)).or_insert(next);
            if i == next {
                cells.push(Cell {
                    exact: Some(p),
                    ..Cell::default()
                });
            }
            let c = &mut cells[i as usize];
            c.pos += p;
            c.corners += 1;
            cell_of.push(i);
            continue;
        }
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
    use std::collections::BTreeSet;

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
        let borders = Borders::new([]);
        let (levels, kept) = simplify_chunk(&plan, &tris, 0, &borders, &AtomicBool::new(false));
        let level = levels[0].as_ref().expect("the first level helps");
        assert!(level.len() * 4 < tris.len());
        // They are kept as they were, whole vertices, outside the levels.
        assert_eq!(kept.len(), 6);
        let has = |positions: &[[f32; 3]]| {
            kept.as_chunks::<3>()
                .0
                .iter()
                .any(|t| t.iter().map(|v| v.pos).eq(positions.iter().copied()))
        };
        assert!(has(&lit) && has(&textured));
        let lit_vertex = kept.iter().find(|v| v.glow[0] > 0.5).unwrap();
        assert_eq!(lit_vertex.glow, [1.0, 0.0]);
        assert!(kept.iter().any(|v| v.layer[0] == 2.0));
    }

    #[test]
    fn a_simplified_vertex_is_28_bytes_and_keeps_what_the_shader_shows() {
        assert_eq!(std::mem::size_of::<LodVertex>(), 28);
        let mut v = corner([1.0, 2.0, 3.0], [1.0, 0.5, 0.0]);
        v.normal = [0.0, 0.6, -0.8];
        v.finish = [0.0, 1.0, 1.0, 0.25];
        let p = LodVertex::pack(&v);
        assert_eq!(p.pos, [1.0, 2.0, 3.0]);
        assert_eq!(p.front, [255, 128, 0, 0]);
        assert_eq!(p.back, [128, 128, 128, 0]);
        assert_eq!(p.finish, [0, 255, 255, 64]);
        let n = p.normal.map(|b| f32::from(b) / 127.0);
        assert!((n[1] - 0.6).abs() < 0.01 && (n[2] + 0.8).abs() < 0.01);
    }

    /// A bumpy sheet, `n` x `n` squares of `step` m from `x0`, two triangles each; the bumps
    /// make the average of a cell differ from its neighbours' by chunk.
    fn sheet(x0: f32, n: usize, step: f32) -> Vec<BaseFaceVertex> {
        let z = |x: f32, y: f32| 0.9 * (x * 1.3).sin() * (y * 0.7).cos();
        let mut out = Vec::new();
        for i in 0..n {
            for j in 0..n {
                let (x, y) = (x0 + i as f32 * step, j as f32 * step);
                let c = |x: f32, y: f32| corner([x, y, z(x, y)], [0.8, 0.8, 0.8]);
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

    /// The edges of `tris` that lie on the plane x = `x`, as pairs of positions.
    fn edges_on(tris: &[BaseFaceVertex], x: f32) -> BTreeSet<([u32; 3], [u32; 3])> {
        let mut out = BTreeSet::new();
        for t in tris.as_chunks::<3>().0 {
            for i in 0..3 {
                let (a, b) = (t[i].pos, t[(i + 1) % 3].pos);
                if a[0] == x && b[0] == x {
                    out.insert(if bits(a) < bits(b) {
                        (bits(a), bits(b))
                    } else {
                        (bits(b), bits(a))
                    });
                }
            }
        }
        out
    }

    #[test]
    fn chunks_simplified_to_different_levels_meet_along_their_border_without_a_gap() {
        // Two sheets side by side, meeting along x = 20, each its own chunk.
        let (left, right) = (sheet(0.0, 40, 0.5), sheet(20.0, 40, 0.5));
        let shared = edges_on(&left, 20.0);
        assert_eq!(shared.len(), 40);
        assert_eq!(shared, edges_on(&right, 20.0));
        let borders = Borders::new([
            places(left.iter().map(|v| v.pos)),
            places(right.iter().map(|v| v.pos)),
        ]);
        let pinned = |v: &BaseFaceVertex| borders.contains(v.pos);
        for (a, b) in [(2.0, 8.0), (8.0, 2.0), (4.0, 4.0)] {
            let l = cluster_pinned(&left, a, &pinned);
            let r = cluster_pinned(&right, b, &pinned);
            assert!(l.len() * 3 < left.len() && r.len() * 3 < right.len());
            assert_eq!(edges_on(&l, 20.0), shared, "left at {a}, right at {b}");
            assert_eq!(edges_on(&r, 20.0), shared, "left at {a}, right at {b}");
        }
        // Without pins the borders do not agree (the cell averages differ).
        let l = cluster(&left, 2.0);
        let r = cluster(&right, 8.0);
        assert_ne!(edges_on(&l, 20.0), edges_on(&r, 20.0));
    }

    /// A sheet of `nx` x `ny` squares of `step` m from `x0`, flat (z = 0) along x = 20.
    fn flat_edged_sheet(x0: f32, nx: usize, ny: usize, step: f32) -> Vec<BaseFaceVertex> {
        let z = |x: f32, y: f32| 0.9 * (1.3 * (x - 20.0)).sin() * (y * 0.7).cos();
        let mut out = Vec::new();
        for i in 0..nx {
            for j in 0..ny {
                let (x, y) = (x0 + i as f32 * step, j as f32 * step);
                let c = |x: f32, y: f32| corner([x, y, z(x, y)], [0.8, 0.8, 0.8]);
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
    fn a_vertex_in_the_middle_of_a_neighbours_edge_stays_on_it_at_every_level() {
        // The left sheet's border along x = 20 has a vertex every half meter, the right
        // sheet's a vertex every meter: half of the left's lie inside the right's edges.
        let (left, right) = (
            flat_edged_sheet(0.0, 40, 40, 0.5),
            flat_edged_sheet(20.0, 20, 20, 1.0),
        );
        let (shared_left, shared_right) = (edges_on(&left, 20.0), edges_on(&right, 20.0));
        assert_eq!((shared_left.len(), shared_right.len()), (40, 20));
        let at = |t: &[BaseFaceVertex]| places(t.iter().map(|v| v.pos));
        let corners = |t: &[BaseFaceVertex]| t.iter().map(|v| v.pos).collect::<Vec<_>>();
        let mut borders = Borders::new([at(&left), at(&right)]);
        // With only the exactly shared places pinned, the half-meter vertices drift off.
        {
            let pinned = |v: &BaseFaceVertex| borders.contains(v.pos);
            let l = cluster_pinned(&left, 8.0, &pinned);
            assert_ne!(edges_on(&l, 20.0), shared_left);
        }
        let both = [corners(&left), corners(&right)];
        borders.pin_t_junctions(2, &|k| both[k].clone(), 1, &AtomicBool::new(false));
        let pinned = |v: &BaseFaceVertex| borders.contains(v.pos);
        for (a, b) in [(2.0, 8.0), (8.0, 2.0), (4.0, 4.0)] {
            let l = cluster_pinned(&left, a, &pinned);
            let r = cluster_pinned(&right, b, &pinned);
            assert!(l.len() * 2 < left.len() && r.len() < right.len());
            // Every piece of both borders is still there, so the left's pieces end where they
            // did, on the right's edges.
            assert_eq!(edges_on(&l, 20.0), shared_left, "left at {a}, right at {b}");
            assert_eq!(
                edges_on(&r, 20.0),
                shared_right,
                "left at {a}, right at {b}"
            );
        }
    }

    fn pins_of(chunks: &[Vec<[f32; 3]>]) -> Vec<[u32; 3]> {
        t_junctions(
            chunks.len(),
            &|k| chunks[k].clone(),
            2,
            &AtomicBool::new(false),
            WORK_LIMIT,
        )
    }

    #[test]
    fn a_vertex_on_an_edge_used_by_two_triangles_of_the_other_chunk_is_pinned() {
        // Chunk A: a floor with its edge on x = 20, and a wall standing on that edge, so the
        // edge has two triangles. Chunk B continues the floor, with vertices at y = 1, 2.5.
        let a = vec![
            [10.0, 0.0, 0.0],
            [20.0, 0.0, 0.0],
            [20.0, 5.0, 0.0],
            [10.0, 0.0, 0.0],
            [20.0, 5.0, 0.0],
            [10.0, 5.0, 0.0],
            [20.0, 0.0, 0.0],
            [20.0, 0.0, 3.0],
            [20.0, 5.0, 3.0],
            [20.0, 0.0, 0.0],
            [20.0, 5.0, 3.0],
            [20.0, 5.0, 0.0],
        ];
        let b = vec![
            [20.0, 0.0, 0.0],
            [30.0, 0.0, 0.0],
            [20.0, 1.0, 0.0],
            [20.0, 1.0, 0.0],
            [30.0, 0.0, 0.0],
            [20.0, 2.5, 0.0],
            [20.0, 2.5, 0.0],
            [30.0, 0.0, 0.0],
            [20.0, 5.0, 0.0],
        ];
        let pins = pins_of(&[a, b]);
        for y in [1.0, 2.5] {
            assert!(pins.contains(&bits([20.0, y, 0.0])), "y = {y}");
        }
        // And the ends of the wall's edge the vertices lie on.
        for p in [[20.0, 0.0, 0.0], [20.0, 5.0, 0.0]] {
            assert!(pins.contains(&bits(p)), "{p:?}");
        }
    }

    /// Two chunks of `n` single-sided triangles of 1 mm packed in a cube of 5 mm.
    fn dense(n: usize) -> [Vec<[f32; 3]>; 2] {
        let mut seed = 12345u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 40) as f32 / (1u64 << 24) as f32 * 0.005
        };
        let mut chunk = || {
            let mut out = Vec::new();
            for _ in 0..n {
                let c = [next(), next(), next()];
                for d in [[0.0, 0.0, 0.0], [0.001, 0.0, 0.0], [0.0, 0.001, 0.0]] {
                    out.push([c[0] + d[0], c[1] + d[1], c[2] + d[2]]);
                }
            }
            out
        };
        [chunk(), chunk()]
    }

    #[test]
    fn dense_geometry_hits_the_bound_and_returns_a_safe_partial_result() {
        // Every edge is near every point: without a bound this is 20,000 x 3 x 40,000 tests.
        let chunks = dense(20_000);
        let all: BTreeSet<[u32; 3]> = chunks.iter().flatten().map(|p| bits(*p)).collect();
        let started = std::time::Instant::now();
        let pins = t_junctions(
            2,
            &|k| chunks[k].clone(),
            1,
            &AtomicBool::new(false),
            3_000_000,
        );
        assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
        // What it found is positions of the input.
        assert!(pins.iter().all(|p| all.contains(p)));
    }

    #[test]
    fn a_cancel_set_mid_search_ends_it_within_an_edge() {
        let chunks = dense(20_000);
        let cancel = AtomicBool::new(false);
        let started = std::time::Instant::now();
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(200));
                cancel.store(true, Ordering::Relaxed);
            });
            // The bound is out of reach, so only the cancel can end it.
            let _ = t_junctions(2, &|k| chunks[k].clone(), 1, &cancel, u64::MAX);
        });
        assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
    }

    #[test]
    fn a_cancel_set_before_the_search_finds_nothing() {
        let a = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let b = vec![[0.5, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 2.0, 0.0]];
        let cancel = AtomicBool::new(true);
        let pins = t_junctions(
            2,
            &|k| [a.clone(), b.clone()][k].clone(),
            1,
            &cancel,
            WORK_LIMIT,
        );
        assert!(pins.is_empty());
    }

    #[test]
    fn a_vertex_on_an_edge_longer_than_the_largest_grid_is_pinned() {
        // A 100 km edge (beyond the 17 km cells) with another chunk's open end in its middle.
        let a = vec![[0.0, 0.0, 0.0], [100_000.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let b = vec![
            [50_000.0, 0.0, 0.0],
            [50_000.0, 5.0, 0.0],
            [50_001.0, 5.0, 0.0],
        ];
        let pins = pins_of(&[a, b]);
        for p in [[50_000.0, 0.0, 0.0], [0.0, 0.0, 0.0], [100_000.0, 0.0, 0.0]] {
            assert!(pins.contains(&bits(p)), "{p:?}");
        }
    }

    #[test]
    fn the_corners_of_a_triangle_kept_whole_stay_where_they_are_in_the_simplified_ones() {
        // A glowing triangle (kept whole) standing on a corner of a fine sheet: the sheet's
        // vertex there must not move, or a crack opens between them.
        let mut tris = sheet(0.0, 40, 0.5);
        let at = tris[0].pos;
        let mut lit = [
            corner(at, [0.8; 3]),
            corner([at[0] + 0.5, at[1], tris[1].pos[2]], [0.8; 3]),
            corner([at[0], at[1], at[2] + 5.0], [0.8; 3]),
        ];
        for v in &mut lit {
            v.glow = [1.0, 0.0];
        }
        tris.extend(lit);
        let plan = ChunkPlan::new(tris.len(), &[], 3, 100_000, &|i| tris[i].pos, &|_| true);
        let (levels, kept) =
            simplify_chunk(&plan, &tris, 0, &Borders::new([]), &AtomicBool::new(false));
        let level = levels[0].as_ref().expect("the first level helps");
        assert!(kept.iter().any(|v| v.pos == at));
        let there = level.iter().filter(|v| v.pos == at).count();
        assert!(there >= 1, "the sheet's corner at the lit triangle's moved");
    }

    #[test]
    fn a_box_crossing_a_cutting_plane_or_within_a_cell_of_it_is_near() {
        let plane = kernel::SectionPlane::new(glam::DVec3::new(100.0, 0.0, 0.0), glam::DVec3::X);
        let near = |lo: [f32; 3], hi: [f32; 3]| near_planes(lo.into(), hi.into(), &[plane]);
        assert!(near([90.0, 0.0, 0.0], [110.0, 5.0, 5.0]), "across it");
        assert!(
            near([93.0, 0.0, 0.0], [95.0, 5.0, 5.0]),
            "within the largest cell"
        );
        assert!(
            !near([80.0, 0.0, 0.0], [90.0, 5.0, 5.0]),
            "farther than that"
        );
        assert!(!near([0.0; 3], [5.0; 3]) && !near_planes(Vec3::ZERO, Vec3::ONE, &[]));
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
            kept: None,
            kept_ranges: Vec::new(),
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
