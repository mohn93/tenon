//! Edges, prepared so a shader can decide what is a profile.
//!
//! No reader in any of openskp's five languages emits edges into anything
//! renderable - their scene builders output face triangles and the soft,
//! smooth and hidden flags never reach the output at all. Line art is edges,
//! so this module is the one that makes a hidden-line viewer possible.
//!
//! What a profile is, SketchUp decides like this
//! (`EdgeDrawingUtils::ComputeEdgeSilhouetteStatus`, recovered from the
//! binary - see viewer/reference/RENDER.md section 1):
//!
//! ```text
//! fewer than two adjacent faces      -> always a profile
//! otherwise, with v = (midpoint - eye) in perspective, or the eye
//! direction in parallel, cast to f32:
//!     d0 = dot(n0, v) ; d1 = dot(n1, v) ; tol = eps * |v|^2
//!     |d0|^2 < tol or |d1|^2 < tol    -> profile  (a face seen edge-on)
//!     otherwise                        -> d0 * d1 < 0
//! ```
//!
//! That test is per-edge and runs in the *definition's* local space with the
//! eye transformed into it, which is why the normals are prepared here, once
//! per definition, and shipped to the GPU rather than recomputed per
//! placement. A component placed a thousand times pays for this once.
//!
//! The two normals are not simply the two adjacent face normals. Each is taken
//! from the face through its edge *use* and negated when that use is reversed,
//! and the second is negated again on storage. Doing only half of that gives
//! an outline that is right for some surfaces and inside-out for others, which
//! reads as a shading bug rather than as a sign error.

use crate::geom::{DefGeometry, NO_LAYER};
use crate::model::INCH_TO_MM;
use crate::model::{edge_flag, EdgeBatch};
use alloc::vec;
use alloc::vec::Vec;

/// Per-edge adjacency, in the definition's own space.
pub struct EdgeAdjacency {
    /// How many faces use this edge. Under two means a boundary, which is
    /// always a profile.
    pub face_count: Vec<u8>,
    /// Two oriented normals per edge, six f32 each.
    pub normals: Vec<f32>,
}

pub fn build_adjacency(g: &DefGeometry) -> EdgeAdjacency {
    let ne = g.edges();
    let mut face_count = vec![0u8; ne];
    let mut normals = vec![0.0f32; ne * 6];

    for f in 0..g.faces() {
        let n = g.normal(f);
        let (l0, l1) = (g.face_loop_ofs[f] as usize, g.face_loop_ofs[f + 1] as usize);
        for lp in l0..l1 {
            let (c0, c1) = (g.loop_ofs[lp] as usize, g.loop_ofs[lp + 1] as usize);
            for c in c0..c1 {
                let e = g.coedge_edge[c];
                if e == u32::MAX {
                    continue;
                }
                let e = e as usize;
                let reversed = g.coedge_reversed[c] != 0;
                let sign = if reversed { -1.0 } else { 1.0 };
                let slot = face_count[e];
                if slot >= 2 {
                    // Three or more faces on an edge. SketchUp sorts by
                    // dihedral angle and keeps the extreme pair; keeping the
                    // first two is a knowing simplification, and it shows only
                    // on non-manifold geometry where the outline is ambiguous
                    // anyway. Recorded here rather than left implicit.
                    face_count[e] = face_count[e].saturating_add(1);
                    continue;
                }
                let store_sign = if slot == 1 { -sign } else { sign };
                let base = e * 6 + slot as usize * 3;
                normals[base] = (n[0] * store_sign) as f32;
                normals[base + 1] = (n[1] * store_sign) as f32;
                normals[base + 2] = (n[2] * store_sign) as f32;
                face_count[e] = slot + 1;
            }
        }
    }

    EdgeAdjacency {
        face_count,
        normals,
    }
}

/// Which edges to put in a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeFilter {
    /// Everything, flags included, so the renderer can honour a style that
    /// shows hidden geometry.
    All,
    /// Only what SketchUp would draw. Across this project's fixtures that
    /// drops 27% of edges on average and up to 66% on curved models, which is
    /// worth having when the model is large.
    DrawableOnly,
}

pub fn build_batch(
    definition_id: u32,
    g: &DefGeometry,
    adj: &EdgeAdjacency,
    filter: EdgeFilter,
) -> EdgeBatch {
    let mut b = EdgeBatch {
        definition_id,
        ..Default::default()
    };
    let ne = g.edges();
    b.positions.reserve(ne * 6);
    b.face_normals.reserve(ne * 6);
    b.flags.reserve(ne);
    b.face_counts.reserve(ne);
    b.layer_ids.reserve(ne);

    // Drawable edges first, then the rest, so a hidden-line renderer can draw
    // a prefix instead of issuing every edge and discarding most of them in
    // the vertex shader. Two passes over the same list rather than a sort:
    // the order within each group does not matter and a sort would move six
    // floats per edge about for nothing.
    let push = |b: &mut EdgeBatch, e: usize| {
        let (a, c) = (g.edge_v[e * 2], g.edge_v[e * 2 + 1]);
        if a == u32::MAX || c == u32::MAX {
            return;
        }
        let pa = g.vertex(a);
        let pc = g.vertex(c);
        b.positions.extend_from_slice(&[
            (pa[0] * INCH_TO_MM) as f32,
            (pa[1] * INCH_TO_MM) as f32,
            (pa[2] * INCH_TO_MM) as f32,
            (pc[0] * INCH_TO_MM) as f32,
            (pc[1] * INCH_TO_MM) as f32,
            (pc[2] * INCH_TO_MM) as f32,
        ]);
        b.face_normals
            .extend_from_slice(&adj.normals[e * 6..e * 6 + 6]);
        b.flags.push(g.edge_flags[e]);
        b.face_counts.push(adj.face_count[e]);
        b.layer_ids.push(if g.edge_layer[e] == NO_LAYER {
            u32::MAX
        } else {
            g.edge_layer[e]
        });
    };

    for e in 0..ne {
        if edge_flag::is_drawable(g.edge_flags[e]) {
            push(&mut b, e);
        }
    }
    b.drawable = b.flags.len() as u32;
    if filter == EdgeFilter::All {
        // Softened before hidden: the middle range is drawn as a silhouette
        // and the last one is never drawn at all.
        for e in 0..ne {
            let f = g.edge_flags[e];
            if f & edge_flag::HIDDEN == 0 && f & (edge_flag::SOFT | edge_flag::SMOOTH) != 0 {
                push(&mut b, e);
            }
        }
        b.silhouette = b.flags.len() as u32;
        for e in 0..ne {
            if g.edge_flags[e] & edge_flag::HIDDEN != 0 {
                push(&mut b, e);
            }
        }
    } else {
        b.silhouette = b.drawable;
    }
    b
}

/// The profile test itself, for the CPU side: used by the headless renderer
/// and by tests that have to agree with the shader.
///
/// `eye_or_dir` is the eye position in perspective, or the view direction in
/// parallel. Everything is f32 because the original casts to f32 before
/// testing, and at long distances that cast is what decides borderline edges.
pub fn is_profile(
    face_count: u8,
    n0: [f32; 3],
    n1: [f32; 3],
    midpoint: [f32; 3],
    eye_or_dir: [f32; 3],
    perspective: bool,
) -> bool {
    if face_count < 2 {
        return true;
    }
    let v = if perspective {
        [
            midpoint[0] - eye_or_dir[0],
            midpoint[1] - eye_or_dir[1],
            midpoint[2] - eye_or_dir[2],
        ]
    } else {
        eye_or_dir
    };
    let d0 = n0[0] * v[0] + n0[1] * v[1] + n0[2] * v[2];
    let d1 = n1[0] * v[0] + n1[1] * v[1] + n1[2] * v[2];
    let len2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    // SketchUp compares the squared dot against a tolerance scaled by the view
    // vector's squared length, which is what makes the test scale-invariant.
    const EPS_SQ: f32 = 1.0e-10;
    let tol = EPS_SQ * len2;
    if d0 * d0 < tol || d1 * d1 < tol {
        return true;
    }
    d0 * d1 < 0.0
}

/// Put a batch's drawable edges first and record how many there are.
///
/// `build_batch` already emits them in that order, but a batch can come from
/// anywhere - the legacy reader builds its own, and so does the merge pass -
/// and a batch whose `drawable` is left at zero renders as no line art at
/// all. That failure is silent in every count: the edges are present, the
/// flags are right, and the screen is blank. Normalising here means a reader
/// cannot forget.
pub fn sort_drawable_first(b: &mut EdgeBatch) {
    let n = b.len();
    if n == 0 {
        b.drawable = 0;
        b.silhouette = 0;
        return;
    }
    // Three ranges, not two. SketchUp draws a softened edge when it lies on
    // the silhouette and silhouettes are on, and never draws a hidden one -
    // so "soft or smooth" and "hidden" are different populations and cannot
    // share a tail. See `EdgeBatch::silhouette` for the fixture that settled
    // it.
    let rank = |f: u8| -> u8 {
        if f & edge_flag::HIDDEN != 0 {
            2
        } else if f & (edge_flag::SOFT | edge_flag::SMOOTH) != 0 {
            1
        } else {
            0
        }
    };
    let first = b.flags.iter().filter(|f| rank(**f) == 0).count() as u32;
    let second = b.flags.iter().filter(|f| rank(**f) == 1).count() as u32;

    // Already partitioned - the common case, and free to check.
    if b.drawable == first
        && b.silhouette == first + second
        && b.flags.windows(2).all(|w| rank(w[0]) <= rank(w[1]))
    {
        return;
    }

    let mut order: Vec<u32> = (0..n as u32).collect();
    // Stable, so edges keep the order the reader gave them within a range -
    // which is the order their layer ids and normals are in, and those are
    // permuted alongside.
    order.sort_by_key(|&i| rank(b.flags[i as usize]));

    let mut pos = Vec::with_capacity(n * 6);
    let mut nrm = Vec::with_capacity(n * 6);
    let mut fl = Vec::with_capacity(n);
    let mut fc = Vec::with_capacity(n);
    let mut ly = Vec::with_capacity(n);
    for &i in &order {
        let k = i as usize;
        pos.extend_from_slice(&b.positions[k * 6..k * 6 + 6]);
        nrm.extend_from_slice(&b.face_normals[k * 6..k * 6 + 6]);
        fl.push(b.flags[k]);
        fc.push(b.face_counts[k]);
        ly.push(b.layer_ids.get(k).copied().unwrap_or(u32::MAX));
    }
    b.positions = pos;
    b.face_normals = nrm;
    b.flags = fl;
    b.face_counts = fc;
    b.layer_ids = ly;
    b.drawable = first;
    b.silhouette = first + second;
}
