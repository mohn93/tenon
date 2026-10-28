//! Spatial chunks of a model's face and edge vertices, so a frame draws only those in view.
//!
//! A `ChunkPlan` reorders a vertex list so that the vertices of nearby placements lie together,
//! chunk after chunk, and notes each chunk's box. A frame tests the boxes against the camera
//! and draws the vertex ranges of the chunks that may show. The reordering is kept as
//! `Segment`s, so a flag buffer in the old order (a selection, a hover) can be written in the
//! new one.

use std::ops::Range;

use glam::{DVec3, Vec3, Vec4};
use session::Camera;

/// About how many primitives (triangles, or edges) a chunk holds, by default: a few thousand.
pub const CHUNK_PRIMITIVES: usize = 4096;
/// Chunks smaller than this many pixels across (the box's diagonal) are not drawn.
pub const SMALL_PIXELS: f64 = 1.0;
/// How far outside the view a chunk may lie and still be drawn (a fraction of the view's half
/// width, height and depth), so wide lines and depth offsets at the border are not clipped.
const GUARD: f32 = 0.1;

/// A run of vertices that moved together: `len` vertices from `old` in the old order to `new`
/// in the new one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub old: usize,
    pub len: usize,
    pub new: usize,
}

/// One chunk: its vertices in the new order and the box around them.
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub vertices: Range<u32>,
    pub lo: Vec3,
    pub hi: Vec3,
}

/// The chunks of one vertex list.
#[derive(Debug, Default)]
pub struct ChunkPlan {
    /// Where every vertex went, in new order; they cover the list.
    segments: Vec<Segment>,
    /// The same by old position.
    by_old: Vec<Segment>,
    /// The chunks, in new order: first those of the primitives `keep` listed (the first
    /// `solid_chunks`, holding `kept` vertices), then those of the others.
    pub chunks: Vec<Chunk>,
    pub solid_chunks: usize,
    pub kept: usize,
}

/// A run of vertices from one placement (or from between them) and its box.
struct Piece {
    range: Range<usize>,
    /// Whether every primitive of the range is in the class.
    whole: bool,
    lo: Vec3,
    hi: Vec3,
    key: u64,
}

impl ChunkPlan {
    /// Chunks the vertices `0..total` in `align`-vertex primitives (3 for triangles, 2 for line
    /// pairs). `groups` are ranges of vertices that belong together (a placement's), sorted;
    /// what they leave out is chunked as it comes. `pos` gives a vertex's position; `keep`
    /// says whether the primitive starting at a vertex is in the first class of chunks; the
    /// others make the second.
    pub fn new(
        total: usize,
        groups: &[Range<usize>],
        align: usize,
        chunk_primitives: usize,
        pos: &(impl Fn(usize) -> [f32; 3] + Sync),
        keep: &(impl Fn(usize) -> bool + Sync),
    ) -> ChunkPlan {
        // Placements bigger than half a chunk are split, so a chunk stays local.
        let piece_len = (chunk_primitives / 2).max(1) * align;
        let chunk_len = chunk_primitives.max(1) * align;
        let mut ranges: Vec<Range<usize>> = Vec::new();
        let mut cut = |from: usize, to: usize| {
            let mut a = from;
            while a < to {
                let b = (a + piece_len).min(to);
                ranges.push(a..b);
                a = b;
            }
        };
        let mut at = 0;
        for g in groups {
            let (start, end) = (g.start.max(at), g.end.min(total));
            if start >= end {
                continue;
            }
            cut(at, start);
            cut(start, end);
            at = end;
        }
        cut(at, total);

        let (solid, rest) = bounded(&ranges, align, pos, keep);
        let mut plan = ChunkPlan::default();
        plan.chunk_class(solid, align, chunk_len, true, keep);
        plan.kept = plan.end();
        plan.solid_chunks = plan.chunks.len();
        plan.chunk_class(rest, align, chunk_len, false, keep);
        plan.by_old = plan.segments.clone();
        plan.by_old.sort_by_key(|s| s.old);
        plan
    }

    /// Chunks the primitives of `pieces` whose `keep` is `class`, after the chunks so far.
    fn chunk_class(
        &mut self,
        mut pieces: Vec<Piece>,
        align: usize,
        chunk_len: usize,
        class: bool,
        keep: &(impl Fn(usize) -> bool + Sync),
    ) {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &pieces {
            lo = lo.min(p.lo);
            hi = hi.max(p.hi);
        }
        let span = (hi - lo).max(Vec3::splat(1e-6));
        for p in &mut pieces {
            let unit = ((p.lo + p.hi) * 0.5 - lo) / span;
            p.key = morton(unit);
        }
        pieces.sort_by_key(|p| p.key);

        // The open chunk: where it starts, and its box.
        let mut open: Option<(usize, Vec3, Vec3)> = None;
        for p in &pieces {
            if let Some((first, ..)) = open
                && self.end() - first >= chunk_len
            {
                self.close(&mut open);
            }
            let (_, olo, ohi) = open.get_or_insert((self.end(), p.lo, p.hi));
            *olo = olo.min(p.lo);
            *ohi = ohi.max(p.hi);
            if p.whole {
                self.push(p.range.start, p.range.len());
            } else {
                for v in p.range.clone().step_by(align) {
                    if keep(v) == class {
                        self.push(v, align);
                    }
                }
            }
        }
        self.close(&mut open);
    }

    /// Where the next vertex goes.
    fn end(&self) -> usize {
        self.segments.last().map_or(0, |s| s.new + s.len)
    }

    /// Adds `len` vertices from `old` after the others, joining the last segment when they
    /// continue it.
    fn push(&mut self, old: usize, len: usize) {
        let new = self.end();
        match self.segments.last_mut() {
            Some(s) if s.old + s.len == old => s.len += len,
            _ => self.segments.push(Segment { old, len, new }),
        }
    }

    fn close(&mut self, open: &mut Option<(usize, Vec3, Vec3)>) {
        if let Some((first, lo, hi)) = open.take()
            && self.end() > first
        {
            self.chunks.push(Chunk {
                vertices: first as u32..self.end() as u32,
                lo,
                hi,
            });
        }
    }

    /// Where every vertex went, in the new order.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// How many vertices the plan places.
    pub fn total(&self) -> usize {
        self.end()
    }

    /// `list` in the new order; an empty list (of a kind of data a model may lack) stays empty.
    pub fn permute<T: bytemuck::Pod + Send + Sync>(&self, list: &[T]) -> Vec<T> {
        if list.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<T> = bytemuck::zeroed_vec(list.len());
        self.permute_into(list, &mut out);
        out
    }

    /// The vertices of `list` that come from `first` on in the new order, in that order.
    pub fn permute_tail<T: Copy>(&self, list: &[T], first: usize) -> Vec<T> {
        let mut out = Vec::new();
        for s in self.segments.iter().filter(|s| s.new + s.len > first) {
            let skip = first.saturating_sub(s.new);
            out.extend_from_slice(&list[s.old + skip..s.old + s.len]);
        }
        out
    }

    /// Writes `list` in the new order to `out`, which is as long.
    pub fn permute_into<T: bytemuck::Pod + Send + Sync>(&self, list: &[T], out: &mut [T]) {
        assert_eq!(list.len(), out.len());
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8);
        let copy = |s: &Segment, to: &mut [T]| to.copy_from_slice(&list[s.old..s.old + s.len]);
        if threads < 2 || list.len() < 1 << 16 {
            for s in &self.segments {
                copy(s, &mut out[s.new..s.new + s.len]);
            }
            return;
        }
        // Each thread fills a stretch of the output, cut at segment boundaries.
        std::thread::scope(|scope| {
            let mut rest: &mut [T] = out;
            let mut at = 0;
            for group in self.segments.chunks(self.segments.len().div_ceil(threads)) {
                let end = group.last().map_or(at, |s| s.new + s.len);
                let (mine, others) = rest.split_at_mut(end - at);
                rest = others;
                let from = std::mem::replace(&mut at, end);
                let copy = &copy;
                scope.spawn(move || {
                    for s in group {
                        copy(s, &mut mine[s.new - from..s.new - from + s.len]);
                    }
                });
            }
        });
    }

    /// Where the vertices `range` of the old order lie in the new one.
    pub fn map_range(&self, range: Range<usize>) -> Vec<Range<usize>> {
        let first = self
            .by_old
            .partition_point(|s| s.old + s.len <= range.start);
        let mut out = Vec::new();
        for s in &self.by_old[first..] {
            if s.old >= range.end {
                break;
            }
            let (a, b) = (range.start.max(s.old), range.end.min(s.old + s.len));
            out.push(s.new + (a - s.old)..s.new + (b - s.old));
        }
        out
    }

    /// The vertex ranges to draw for `camera`, adjacent ones merged: chunks that may show in
    /// its view and are not smaller than `SMALL_PIXELS`.
    pub fn visible(&self, camera: &Camera) -> Runs {
        let m = camera.view_proj_f32();
        let parallel = camera.is_parallel();
        // World size of a pixel at depth 1 (perspective) or anywhere (parallel).
        let unit = if parallel {
            camera.pixel_size(DVec3::ZERO)
        } else {
            camera.pixel_size(camera.eye + camera.forward())
        };
        let mut runs = Runs::default();
        for (k, c) in self.chunks.iter().enumerate() {
            let (mut out, mut w_min) = ([true; 6], f32::MAX);
            for k in 0..8 {
                let p = Vec3::new(
                    if k & 1 == 0 { c.lo.x } else { c.hi.x },
                    if k & 2 == 0 { c.lo.y } else { c.hi.y },
                    if k & 4 == 0 { c.lo.z } else { c.hi.z },
                );
                let q: Vec4 = m * p.extend(1.0);
                let g = GUARD * q.w;
                // Signed distances to the six planes of the view volume (0 <= z <= w),
                // loosened by the guard; inside where not negative.
                let d = [
                    q.x + q.w + g,
                    q.w + g - q.x,
                    q.y + q.w + g,
                    q.w + g - q.y,
                    q.z + g,
                    q.w + g - q.z,
                ];
                for (o, d) in out.iter_mut().zip(d) {
                    *o &= d < 0.0;
                }
                w_min = w_min.min(q.w);
            }
            if out.iter().any(|&o| o) {
                continue;
            }
            // The nearest corner gives the largest size, so only chunks small even there go.
            let depth = if parallel { 1.0 } else { f64::from(w_min) };
            if depth > 0.0 && f64::from((c.hi - c.lo).length()) < SMALL_PIXELS * unit * depth {
                continue;
            }
            let list = if k < self.solid_chunks {
                &mut runs.solid
            } else {
                &mut runs.rest
            };
            match list.last_mut() {
                Some(r) if r.end == c.vertices.start => r.end = c.vertices.end,
                _ => list.push(c.vertices.clone()),
            }
        }
        runs
    }

    /// How many vertices the ranges cover.
    pub fn covered(runs: &[Range<u32>]) -> u32 {
        runs.iter().map(|r| r.end - r.start).sum()
    }
}

/// The vertex ranges of the chunks in view, of the first class and of the second.
#[derive(Debug, Default, PartialEq)]
pub struct Runs {
    pub solid: Vec<Range<u32>>,
    pub rest: Vec<Range<u32>>,
}

/// `ranges` with the box of the primitives `keep` lists, and again with that of the others;
/// ranges with none of a kind are left out of that list.
fn bounded(
    ranges: &[Range<usize>],
    align: usize,
    pos: &(impl Fn(usize) -> [f32; 3] + Sync),
    keep: &(impl Fn(usize) -> bool + Sync),
) -> (Vec<Piece>, Vec<Piece>) {
    let measure = |range: Range<usize>| -> (Option<Piece>, Option<Piece>) {
        let empty = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let (mut boxes, mut counts) = ([empty; 2], [0usize; 2]);
        for v in range.clone().step_by(align) {
            let class = usize::from(!keep(v));
            counts[class] += 1;
            let (lo, hi) = &mut boxes[class];
            for i in v..v + align {
                let p = Vec3::from(pos(i));
                *lo = lo.min(p);
                *hi = hi.max(p);
            }
        }
        let total = counts[0] + counts[1];
        let piece = |class: usize| {
            (counts[class] > 0).then(|| Piece {
                range: range.clone(),
                whole: counts[class] == total,
                lo: boxes[class].0,
                hi: boxes[class].1,
                key: 0,
            })
        };
        (piece(0), piece(1))
    };
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(8);
    let collect = |parts: Vec<(Option<Piece>, Option<Piece>)>| {
        let (a, b): (Vec<_>, Vec<_>) = parts.into_iter().unzip();
        (
            a.into_iter().flatten().collect(),
            b.into_iter().flatten().collect(),
        )
    };
    if threads < 2 || ranges.len() < 64 {
        return collect(ranges.iter().cloned().map(measure).collect());
    }
    let per = ranges.len().div_ceil(threads);
    collect(std::thread::scope(|s| {
        let jobs: Vec<_> = ranges
            .chunks(per)
            .map(|part| {
                let measure = &measure;
                s.spawn(move || part.iter().cloned().map(measure).collect::<Vec<_>>())
            })
            .collect();
        jobs.into_iter()
            .flat_map(|j| j.join().expect("a chunk measuring thread panicked"))
            .collect()
    }))
}

/// The Morton code of a point of the unit cube, 21 bits an axis.
fn morton(p: Vec3) -> u64 {
    fn spread(v: u32) -> u64 {
        let mut x = u64::from(v) & 0x1F_FFFF;
        x = (x | x << 32) & 0x001F_0000_0000_FFFF;
        x = (x | x << 16) & 0x001F_0000_FF00_00FF;
        x = (x | x << 8) & 0x100F_00F0_0F00_F00F;
        x = (x | x << 4) & 0x10C3_0C30_C30C_30C3;
        x = (x | x << 2) & 0x1249_2492_4924_9249;
        x
    }
    let q = |v: f32| (v.clamp(0.0, 1.0) * 2_097_151.0) as u32;
    spread(q(p.x)) | spread(q(p.y)) << 1 | spread(q(p.z)) << 2
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` unit triangles in a row along x, a gap apart.
    fn row(n: usize) -> Vec<[f32; 3]> {
        (0..n)
            .flat_map(|k| {
                let x = 3.0 * k as f32;
                [[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x, 1.0, 0.0]]
            })
            .collect()
    }

    fn plan(v: &[[f32; 3]], keep: impl Fn(usize) -> bool + Sync) -> ChunkPlan {
        ChunkPlan::new(v.len(), &[], 3, CHUNK_PRIMITIVES, &|i| v[i], &keep)
    }

    #[test]
    fn every_vertex_moves_once_and_kept_ones_come_first() {
        let v = row(10_000);
        let solid = |first: usize| !(first / 3).is_multiple_of(4);
        let p = plan(&v, solid);
        let moved = p.permute(&(0..v.len()).collect::<Vec<_>>());
        let mut sorted = moved.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..v.len()).collect::<Vec<_>>());
        assert_eq!(p.kept, 3 * (10_000 - 2_500));
        assert!(moved[..p.kept].iter().all(|&i| solid(i / 3 * 3)));
        assert!(moved[p.kept..].iter().all(|&i| !solid(i / 3 * 3)));
        // Triangles stay whole and in order.
        for t in moved.chunks(3) {
            assert_eq!(t[0] % 3, 0);
            assert_eq!((t[1], t[2]), (t[0] + 1, t[0] + 2));
        }
        assert!(p.chunks.len() > 1);
        let mut at = 0;
        for c in &p.chunks {
            assert_eq!(c.vertices.start, at);
            at = c.vertices.end;
        }
        assert_eq!(at as usize, v.len());
    }

    #[test]
    fn chunk_boxes_hold_their_vertices() {
        let v = row(5_000);
        let p = plan(&v, |_| true);
        let moved = p.permute(&v);
        for c in &p.chunks {
            for q in &moved[c.vertices.start as usize..c.vertices.end as usize] {
                let q = Vec3::from(*q);
                assert!(q.cmpge(c.lo).all() && q.cmple(c.hi).all());
            }
        }
    }

    #[test]
    fn old_ranges_map_to_the_new_places_of_their_vertices() {
        let v = row(3_000);
        let p = plan(&v, |first| !(first / 3).is_multiple_of(5));
        let order = p.permute(&(0..v.len()).collect::<Vec<_>>());
        for range in [0..9, 5..40, 1000..4000, 8990..9000, 0..9000] {
            let mut got: Vec<usize> = p
                .map_range(range.clone())
                .into_iter()
                .flat_map(|r| order[r].to_vec())
                .collect();
            got.sort_unstable();
            assert_eq!(got, range.collect::<Vec<_>>());
        }
    }

    #[test]
    fn groups_stay_whole_and_gaps_are_listed() {
        let v = row(100);
        let p = ChunkPlan::new(v.len(), &[30..60, 120..150], 3, 16, &|i| v[i], &|_| true);
        assert_eq!(p.kept, v.len());
    }

    #[test]
    fn a_view_of_one_end_of_a_row_draws_one_chunk() {
        // 300 squares of two triangles, 6 m apart, each its own group.
        let v: Vec<[f32; 3]> = (0..300usize)
            .flat_map(|k| {
                let x = 6.0 * k as f32;
                let (a, b, c, d) = ([x, 0., 0.], [x + 4., 0., 0.], [x + 4., 4., 0.], [x, 4., 0.]);
                [a, b, d, b, c, d]
            })
            .collect();
        let groups: Vec<_> = (0..300).map(|k| 6 * k..6 * k + 6).collect();
        let p = ChunkPlan::new(v.len(), &groups, 3, 100, &|i| v[i], &|_| true);
        assert_eq!(p.chunks.len(), 6);
        for (k, c) in p.chunks.iter().enumerate() {
            assert_eq!(c.lo.x, 300.0 * k as f32, "chunks are runs of neighbours");
        }
        let mut cam = Camera::new(glam::DVec2::new(256.0, 256.0));
        cam.eye = DVec3::new(30.0, -60.0, 40.0);
        cam.target = DVec3::new(30.0, 0.0, 0.0);
        assert_eq!(p.visible(&cam).solid, vec![0..300]);
    }
}
