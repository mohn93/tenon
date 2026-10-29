//! Dimensions and text of a 2021+ file.
//!
//! Layouts measured on the corpus's files (SketchUp's own tag names, `tags.rs`): 13 linear
//! dimensions in `Untitled.skp` and the 3 text labels of the tutorial.
//!
//! ```text
//! ComponentDimensionsLinear -> DimensionLinear {
//!     Dimension { DrawingElement{..}, DimensionText (empty: the measured length), .. },
//!     DimensionPtRef1 / DimensionPtRef2 -> PointRef,
//!     DimensionNormal      3 x f64   the plane's normal,
//!     DimensionXAxis       3 x f64   in the plane, opposite to start -> end in every sample,
//!     DimensionType        u32       1 in every sample,
//!     DimensionYPos        f64       the dimension line's distance from the measured points,
//!     DimensionXPos        f64       0 in every sample (the text along the line),
//!     DimensionTextPos     u32       3 in every sample }
//! ComponentTexts -> Text {
//!     DrawingElement, TextString (UTF-8), TextXOffset / TextYOffset f64 (0 in the samples),
//!     TextLeaderPointRef -> PointRef, TextLeaderVec 3 x f64, .., TextLeaderType u32 }
//! PointRef -> { PointRefType u32, PointRefPoint 3 x f64, PointRefInstancePath ->
//!     InstancePath { InstancePathLeafPIDRef: the entity's pid,
//!                    InstancePathInstancePIDRefs: the instances to it, empty here } }
//! ```
//!
//! A `PointRef` of type 1 is a free point: its position is the point. Type 3 (a text label on
//! a guide point in the tutorial) is a point on the entity whose pid is the path's leaf, with
//! the position left at zero; it is resolved here for a vertex or a guide point of the same
//! component (an empty instance list) and is otherwise unread. Other types are unread.
//!
//! The dimension line is the measured segment moved by `YPos * (normal x x_axis)`: in the
//! samples this puts the line above a top edge and below a floor edge, where a dimension is
//! drawn, and the other sign would put it inside the wall.
//!
//! Not read, and counted instead (no file with one is in the corpus, so their layouts are not
//! established): radial dimensions (`ComponentDimensionsRadial`; the arc, the leader position
//! parameter and ratio are unverified), and text without a pushpin leader (leader type 0,
//! screen text, and 1, view-based: their position fields are zero in the samples that have
//! them, which are none).

use crate::error::Result;
use crate::geom::{common_of, layer_ref, spend, DefGeometry, NO_LAYER};
use crate::tags as t;
use crate::tlv::{Record, Tlv};
use alloc::string::String;
use alloc::vec::Vec;

/// A dimension or text label as the file states it, in inches.
#[derive(Debug, Clone, PartialEq)]
pub enum RawAnnotation {
    /// A linear dimension between two points, its line moved by `offset` (perpendicular to
    /// the points, in the dimension's plane).
    Linear {
        start: [f64; 3],
        end: [f64; 3],
        offset: [f64; 3],
        /// A layer id as `DefGeometry::guide_layer` states it.
        layer: u32,
        /// True when the dimension carries text of its own, which a dimension here cannot.
        custom_text: bool,
    },
    /// Text with a pushpin leader: the leader ends at `anchor`, the text at `anchor + vector`.
    Leader {
        anchor: [f64; 3],
        vector: [f64; 3],
        /// Empty: the file's automatic text.
        text: String,
        layer: u32,
    },
}

/// What was in a component's annotation lists and was not read as an annotation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnnotationSkips {
    /// Radial dimensions (layout not established).
    pub radial: u32,
    /// Text without a pushpin leader (screen text, view-based leaders).
    pub screen_text: u32,
    /// Hidden dimensions and text.
    pub hidden: u32,
    /// Records that were not read: a missing field, a point on something that is not a
    /// vertex or guide point of the component, a kind of point not known.
    pub unread: u32,
}

impl AnnotationSkips {
    pub fn total(&self) -> u64 {
        u64::from(self.radial)
            + u64::from(self.screen_text)
            + u64::from(self.hidden)
            + u64::from(self.unread)
    }
}

/// A resolved point ref: its position, or why not.
fn point_of(r: &Record, g: &DefGeometry, guide_points: &[(u64, [f64; 3])]) -> Option<[f64; 3]> {
    let pr = r.child(t::POINT_REF)?;
    let kind = pr.child(t::POINT_REF_TYPE)?.u32_at(0).ok()?;
    match kind {
        1 => pr.child(t::POINT_REF_POINT)?.vec3_at(0).ok(),
        3 => {
            let pid = connected_pid(&pr)?;
            if let Ok(i) = g.vertex_pid.binary_search(&pid) {
                let p = g.vertex_pos.get(3 * i..3 * i + 3)?;
                return Some([p[0], p[1], p[2]]);
            }
            let at = guide_points.binary_search_by_key(&pid, |(p, _)| *p).ok()?;
            Some(guide_points[at].1)
        }
        _ => None,
    }
}

/// The entity pid of a point ref's first connection.
fn connected_pid(pr: &Record) -> Option<u64> {
    let path = pr
        .child(t::POINT_REF_INSTANCE_PATH)?
        .child(t::INSTANCE_PATH)?;
    if path
        .child(t::INSTANCE_PATH_INSTANCE_PID_REFS)
        .is_some_and(|i| !i.payload.is_empty())
    {
        return None;
    }
    let pid = path.child(t::INSTANCE_PATH_LEAF_PID_REF)?;
    let n = pid.payload.len();
    if n == 0 || n > 8 {
        return None;
    }
    Some(
        pid.payload
            .iter()
            .enumerate()
            .fold(0u64, |v, (i, b)| v | (u64::from(*b) << (8 * i))),
    )
}

fn finite3(p: [f64; 3]) -> bool {
    p.iter().all(|x| x.is_finite())
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn linear(rec: &Record, g: &DefGeometry, points: &[(u64, [f64; 3])]) -> Option<RawAnnotation> {
    let start = point_of(&rec.child(t::DIMENSION_PT_REF1)?, g, points)?;
    let end = point_of(&rec.child(t::DIMENSION_PT_REF2)?, g, points)?;
    let normal = rec.child(t::DIMENSION_NORMAL)?.vec3_at(0).ok()?;
    let x_axis = rec.child(t::DIMENSION_X_AXIS)?.vec3_at(0).ok()?;
    let y_pos = rec.child(t::DIMENSION_Y_POS)?.f64_at(0).ok()?;
    let y_axis = cross(normal, x_axis);
    let offset = [y_axis[0] * y_pos, y_axis[1] * y_pos, y_axis[2] * y_pos];
    if !(finite3(start) && finite3(end) && finite3(offset)) {
        return None;
    }
    let base = rec.child(t::DIMENSION);
    let custom_text = base
        .as_ref()
        .and_then(|b| b.child(t::DIMENSION_TEXT))
        .is_some_and(|s| !s.payload.is_empty());
    let layer = base.as_ref().map_or(NO_LAYER, |b| layer_ref(&common_of(b)));
    Some(RawAnnotation::Linear {
        start,
        end,
        offset,
        layer,
        custom_text,
    })
}

/// The text's leader type: 2 is a pushpin leader (the tutorial's labels), 1 a view-based one
/// and 0 none (SketchUp's `SUTextLeaderType`).
const LEADER_PUSHPIN: u32 = 2;

enum Read {
    Note(RawAnnotation),
    Screen,
    Unread,
}

fn text(rec: &Record, g: &DefGeometry, points: &[(u64, [f64; 3])]) -> Read {
    let Some(kind) = rec
        .child(t::TEXT_LEADER_TYPE)
        .and_then(|k| k.u32_at(0).ok())
    else {
        return Read::Unread;
    };
    if kind != LEADER_PUSHPIN {
        return Read::Screen;
    }
    let (Some(anchor), Some(vector)) = (
        rec.child(t::TEXT_LEADER_POINT_REF)
            .and_then(|r| point_of(&r, g, points)),
        rec.child(t::TEXT_LEADER_VEC)
            .and_then(|v| v.vec3_at(0).ok()),
    ) else {
        return Read::Unread;
    };
    if !(finite3(anchor) && finite3(vector)) {
        return Read::Unread;
    }
    let string = rec
        .child(t::TEXT_STRING)
        .map(|s| String::from_utf8_lossy(s.payload).into_owned())
        .unwrap_or_default();
    let layer = layer_ref(&common_of(rec));
    Read::Note(RawAnnotation::Leader {
        anchor,
        vector,
        text: string,
        layer,
    })
}

/// Reads the component's dimension and text lists into `g`, counting every element against
/// `budget`. `guide_points` are the component's guide points sorted by pid, which a text label may
/// be attached to.
pub(crate) fn read_annotations(
    payload: &[u8],
    g: &mut DefGeometry,
    guide_points: &[(u64, [f64; 3])],
    budget: &mut u64,
) -> Result<()> {
    let mut found: Vec<RawAnnotation> = Vec::new();
    let mut skips = AnnotationSkips::default();
    for sec in Tlv::new(payload) {
        match sec.tag {
            t::COMPONENT_DIMENSIONS_RADIAL => {
                for _ in sec.children() {
                    spend(budget, 1)?;
                    skips.radial = skips.radial.saturating_add(1);
                }
            }
            t::COMPONENT_DIMENSIONS_LINEAR => {
                for r in sec.children() {
                    spend(budget, 1)?;
                    if r.tag != t::DIMENSION_LINEAR {
                        skips.unread = skips.unread.saturating_add(1);
                    } else if hidden(&r) {
                        skips.hidden = skips.hidden.saturating_add(1);
                    } else if let Some(a) = linear(&r, g, guide_points) {
                        found.push(a);
                    } else {
                        skips.unread = skips.unread.saturating_add(1);
                    }
                }
            }
            t::COMPONENT_TEXTS => {
                for r in sec.children() {
                    spend(budget, 1)?;
                    if r.tag != t::TEXT {
                        skips.unread = skips.unread.saturating_add(1);
                    } else if hidden(&r) {
                        skips.hidden = skips.hidden.saturating_add(1);
                    } else {
                        match text(&r, g, guide_points) {
                            Read::Note(a) => found.push(a),
                            Read::Screen => skips.screen_text = skips.screen_text.saturating_add(1),
                            Read::Unread => skips.unread = skips.unread.saturating_add(1),
                        }
                    }
                }
            }
            _ => {}
        }
    }
    g.annotations = found;
    g.annotation_skips = skips;
    Ok(())
}

/// Whether the element's drawing-element flags hide it.
fn hidden(rec: &Record) -> bool {
    let holder = rec.child(t::DIMENSION).unwrap_or(*rec);
    common_of(&holder).flags & 1 != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(tag: u16, payload: &[u8]) -> Vec<u8> {
        let mut v = tag.to_le_bytes().to_vec();
        v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn cat(parts: &[Vec<u8>]) -> Vec<u8> {
        parts.concat()
    }

    fn f64s(v: &[f64]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    fn point(kind: u32, p: [f64; 3], leaf: Option<u64>) -> Vec<u8> {
        let path = match leaf {
            Some(l) => rec(
                t::POINT_REF_INSTANCE_PATH,
                &rec(
                    t::INSTANCE_PATH,
                    &cat(&[
                        rec(t::INSTANCE_PATH_LEAF_PID_REF, &l.to_le_bytes()[..4]),
                        rec(t::INSTANCE_PATH_INSTANCE_PID_REFS, &[]),
                    ]),
                ),
            ),
            None => Vec::new(),
        };
        rec(
            t::POINT_REF,
            &cat(&[
                rec(t::POINT_REF_TYPE, &kind.to_le_bytes()),
                rec(t::POINT_REF_POINT, &f64s(&p)),
                path,
            ]),
        )
    }

    fn base(flags: u8, layer: u8) -> Vec<u8> {
        rec(
            t::DRAWING_ELEMENT,
            &cat(&[
                rec(t::ENTITY, &rec(t::ENTITY_PID, &[7, 0])),
                rec(t::DRAWING_ELEMENT_LAYER_PID_REF, &[layer]),
                rec(t::DRAWING_ELEMENT_FLAGS, &[flags]),
            ]),
        )
    }

    fn dimension(a: Vec<u8>, b: Vec<u8>, y_pos: f64, flags: u8, text: &[u8]) -> Vec<u8> {
        rec(
            t::DIMENSION_LINEAR,
            &cat(&[
                rec(
                    t::DIMENSION,
                    &cat(&[base(flags, 9), rec(t::DIMENSION_TEXT, text)]),
                ),
                rec(t::DIMENSION_PT_REF1, &a),
                rec(t::DIMENSION_PT_REF2, &b),
                rec(t::DIMENSION_NORMAL, &f64s(&[-1.0, 0.0, 0.0])),
                rec(t::DIMENSION_X_AXIS, &f64s(&[0.0, -1.0, 0.0])),
                rec(t::DIMENSION_TYPE, &1u32.to_le_bytes()),
                rec(t::DIMENSION_Y_POS, &y_pos.to_le_bytes()),
                rec(t::DIMENSION_X_POS, &0f64.to_le_bytes()),
                rec(t::DIMENSION_TEXT_POS, &3u32.to_le_bytes()),
            ]),
        )
    }

    fn text_rec(kind: u32, anchor: Vec<u8>, s: &str) -> Vec<u8> {
        rec(
            t::TEXT,
            &cat(&[
                base(6, 9),
                rec(t::TEXT_STRING, s.as_bytes()),
                rec(t::TEXT_LEADER_POINT_REF, &anchor),
                rec(t::TEXT_LEADER_VEC, &f64s(&[1.0, 2.0, 3.0])),
                rec(t::TEXT_LEADER_TYPE, &kind.to_le_bytes()),
            ]),
        )
    }

    fn read(payload: &[u8], g: &mut DefGeometry, points: &[(u64, [f64; 3])]) {
        let mut budget = 1000;
        read_annotations(payload, g, points, &mut budget).unwrap();
    }

    #[test]
    fn linear_dimensions_are_read_with_their_offset() {
        let a = point(1, [0.0, 0.0, 0.0], None);
        let b = point(1, [0.0, 90.0, 0.0], None);
        let payload = rec(
            t::COMPONENT_DIMENSIONS_LINEAR,
            &cat(&[
                dimension(a, b, -4.5, 6, b""),
                dimension(
                    point(1, [0.0; 3], None),
                    point(1, [0.0, 1.0, 0.0], None),
                    1.0,
                    6,
                    b"7 in",
                ),
            ]),
        );
        let mut g = DefGeometry::default();
        read(&payload, &mut g, &[]);
        assert_eq!(g.annotation_skips, AnnotationSkips::default());
        // normal (-1,0,0) x x_axis (0,-1,0) = (0,0,1); times -4.5.
        assert_eq!(
            g.annotations[0],
            RawAnnotation::Linear {
                start: [0.0, 0.0, 0.0],
                end: [0.0, 90.0, 0.0],
                offset: [0.0, 0.0, -4.5],
                layer: 9,
                custom_text: false,
            }
        );
        let RawAnnotation::Linear { custom_text, .. } = &g.annotations[1] else {
            panic!()
        };
        assert!(custom_text);
    }

    #[test]
    fn text_with_a_pushpin_leader_is_read_and_attached_points_resolve() {
        let mut g = DefGeometry::default();
        g.vertex_pid = vec![5, 9];
        g.vertex_pos = vec![0.0, 0.0, 0.0, 10.0, 20.0, 30.0];
        let payload = rec(
            t::COMPONENT_TEXTS,
            &cat(&[
                text_rec(2, point(1, [1.0, 1.0, 1.0], None), "free"),
                text_rec(2, point(3, [0.0; 3], Some(9)), "on a vertex"),
                text_rec(2, point(3, [0.0; 3], Some(77)), "on a guide point"),
                text_rec(2, point(3, [0.0; 3], Some(1234)), "on nothing known"),
                text_rec(2, point(2, [0.0; 3], None), "unknown point type"),
                text_rec(0, point(1, [0.0; 3], None), "screen"),
                text_rec(1, point(1, [0.0; 3], None), "view based"),
                text_rec(2, point(1, [0.0; 3], None), ""),
            ]),
        );
        read(&payload, &mut g, &[(77, [4.0, 5.0, 6.0])]);
        let anchors: Vec<([f64; 3], &str)> = g
            .annotations
            .iter()
            .map(|a| match a {
                RawAnnotation::Leader { anchor, text, .. } => (*anchor, text.as_str()),
                _ => panic!(),
            })
            .collect();
        assert_eq!(
            anchors,
            [
                ([1.0, 1.0, 1.0], "free"),
                ([10.0, 20.0, 30.0], "on a vertex"),
                ([4.0, 5.0, 6.0], "on a guide point"),
                ([0.0, 0.0, 0.0], ""),
            ]
        );
        assert_eq!(g.annotation_skips.screen_text, 2);
        assert_eq!(g.annotation_skips.unread, 2);
    }

    #[test]
    fn radial_dimensions_hidden_ones_and_broken_ones_are_counted() {
        let hidden = dimension(
            point(1, [0.0; 3], None),
            point(1, [0.0, 1.0, 0.0], None),
            1.0,
            7,
            b"",
        );
        let broken = rec(t::DIMENSION_LINEAR, &rec(t::DIMENSION_NORMAL, &[0; 24]));
        let payload = cat(&[
            rec(
                t::COMPONENT_DIMENSIONS_RADIAL,
                &cat(&[rec(t::DIMENSION_RADIAL, &[]), rec(t::DIMENSION_RADIAL, &[])]),
            ),
            rec(t::COMPONENT_DIMENSIONS_LINEAR, &cat(&[hidden, broken])),
        ]);
        let mut g = DefGeometry::default();
        read(&payload, &mut g, &[]);
        assert!(g.annotations.is_empty());
        assert_eq!(
            g.annotation_skips,
            AnnotationSkips {
                radial: 2,
                screen_text: 0,
                hidden: 1,
                unread: 1,
            }
        );
    }

    #[test]
    fn many_labels_on_many_guide_points_are_found_by_binary_search() {
        // 100,000 labels, each on its own guide point: a linear search per label would be
        // 10^10 steps.
        const N: u64 = 100_000;
        let points: Vec<(u64, [f64; 3])> = (1..=N).map(|p| (p, [p as f64, 0.0, 0.0])).collect();
        let labels: Vec<Vec<u8>> = (1..=N)
            .map(|p| text_rec(2, point(3, [0.0; 3], Some(p)), "x"))
            .collect();
        let payload = rec(t::COMPONENT_TEXTS, &labels.concat());
        let mut g = DefGeometry::default();
        let mut budget = 10 * N;
        let started = std::time::Instant::now();
        read_annotations(&payload, &mut g, &points, &mut budget).unwrap();
        assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
        assert_eq!(g.annotations.len(), N as usize);
        assert_eq!(g.annotation_skips.unread, 0);
        let RawAnnotation::Leader { anchor, .. } = &g.annotations[41] else {
            panic!()
        };
        assert_eq!(anchor[0], 42.0);
    }

    #[test]
    fn non_finite_numbers_are_unread_and_the_budget_binds() {
        let nan = point(1, [f64::NAN, 0.0, 0.0], None);
        let payload = rec(
            t::COMPONENT_DIMENSIONS_LINEAR,
            &dimension(nan, point(1, [1.0, 0.0, 0.0], None), 1.0, 6, b""),
        );
        let mut g = DefGeometry::default();
        read(&payload, &mut g, &[]);
        assert_eq!(g.annotation_skips.unread, 1);
        let mut budget = 0;
        assert!(read_annotations(&payload, &mut g, &[], &mut budget).is_err());
    }
}
