//! 3D text: the outlines of a string's letters in Inter Regular, as flat faces with holes (or
//! as their outline edges), laid out left to right along +X with the first line's baseline at
//! the origin and each further line 1.2 heights lower (-Y).
//!
//! Curves are flattened to at most `MAX_CURVE_SEGMENTS` straight segments each, within 1% of
//! the height. A letter's contours are nested by containment (even-odd): a contour inside an
//! even number of others is the outer loop of a face, one inside an odd number a hole of the
//! contour just around it. Characters the font has no glyph for, and letters whose contours
//! cross or touch, are left out and listed in `skipped`; spaces only move the next letter on.

use std::collections::{BTreeMap, BTreeSet};

use glam::{DVec2, DVec3};
use kernel::{DefinitionId, EntityId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, MetadataProvider};

/// Inter Regular (Inter 4.1, SIL Open Font License 1.1; see `assets/fonts/README.md`): the
/// font 3D text is drawn in, and the app's text font.
pub const INTER_REGULAR_TTF: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// The most straight segments one curve of an outline becomes.
pub const MAX_CURVE_SEGMENTS: usize = 12;

/// How far apart the baselines of two lines are, in heights.
pub const LINE_SPACING: f64 = 1.2;

/// The smallest 3D text height, in meters: below it the letters' points come closer than the
/// kernel tells points apart.
pub const MIN_TEXT_HEIGHT: f64 = 0.001;

/// How far a flattened curve may stray from the real one, as a fraction of the height.
const FLATTENING: f64 = 0.01;

/// Points of a contour closer than this fraction of the height (or `MIN_GAP`) become one.
const MERGE: f64 = 1e-3;

/// The least distance, in meters, between two points or sides of a letter that are not
/// neighbours: closer than this the kernel would weld them.
const MIN_GAP: f64 = 1e-5;

/// A character 3D text leaves out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkippedCharacter {
    /// The character.
    pub character: String,
    /// Its position in the text, counting characters (line breaks included) from 0.
    pub index: usize,
    /// Why it was left out.
    pub reason: String,
}

/// What `add_3d_text` made and left out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Text3dReport {
    /// The new group's instance.
    pub instance: EntityId,
    /// The new group's definition.
    pub definition: DefinitionId,
    /// The faces in the group: the letters', and with depth their sides and tops.
    pub faces: usize,
    /// The characters left out, in order.
    pub skipped: Vec<SkippedCharacter>,
}

/// A flat face of a letter: its outer loop counter-clockwise seen from +Z, its holes
/// clockwise.
#[derive(Clone, Debug, PartialEq)]
pub struct TextFace {
    /// The character the face belongs to (its position in the text, as `SkippedCharacter`).
    pub index: usize,
    /// That character.
    pub character: char,
    pub outer: Vec<DVec2>,
    pub holes: Vec<Vec<DVec2>>,
}

/// A text's letters, laid out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextShape {
    pub faces: Vec<TextFace>,
    /// The characters whose outlines touch or come within `MIN_GAP` of another character's:
    /// built apart, since their points would be welded into one.
    pub touching: BTreeSet<usize>,
    pub skipped: Vec<SkippedCharacter>,
}

impl TextShape {
    /// Every loop's sides, at height 0: what the text looks like before it is placed.
    pub fn segments(&self) -> Vec<[DVec3; 2]> {
        let mut out = Vec::new();
        for face in &self.faces {
            for ring in std::iter::once(&face.outer).chain(&face.holes) {
                for (i, a) in ring.iter().enumerate() {
                    let b = ring[(i + 1) % ring.len()];
                    out.push([a.extend(0.0), b.extend(0.0)]);
                }
            }
        }
        out
    }

    /// The smallest box holding every loop, or None with no faces.
    pub fn bounds(&self) -> Option<[DVec2; 2]> {
        let mut points = self.faces.iter().flat_map(|f| &f.outer);
        let first = *points.next()?;
        Some(points.fold([first, first], |[lo, hi], p| [lo.min(*p), hi.max(*p)]))
    }
}

/// The letters of `text`, capital letters `height` meters tall (a positive, finite height).
pub fn shape(text: &str, height: f64) -> TextShape {
    let font = FontRef::new(INTER_REGULAR_TTF).expect("the built-in font reads");
    let location = LocationRef::default();
    let metrics = font.metrics(Size::unscaled(), location);
    let cap = metrics
        .cap_height
        .map(f64::from)
        .filter(|c| *c > 0.0)
        .unwrap_or(0.7 * f64::from(metrics.units_per_em));
    let scale = height / cap;
    let tolerance = FLATTENING * cap;
    let merge = (MERGE * height).max(MIN_GAP) / scale;
    let gap = MIN_GAP / scale;
    let charmap = font.charmap();
    let advances = font.glyph_metrics(Size::unscaled(), location);
    let outlines = font.outline_glyphs();

    let mut out = TextShape::default();
    let (mut x, mut line) = (0.0, 0usize);
    for (index, c) in text.chars().enumerate() {
        match c {
            '\n' => {
                x = 0.0;
                line += 1;
                continue;
            }
            '\r' => continue,
            _ => {}
        }
        let skip = |reason: &str| SkippedCharacter {
            character: c.to_string(),
            index,
            reason: reason.to_string(),
        };
        let Some(glyph) = charmap.map(c).filter(|g| g.to_u32() != 0) else {
            out.skipped.push(skip("the font has no glyph for it"));
            continue;
        };
        let mut pen = Flattener::new(tolerance);
        if let Some(outline) = outlines.get(glyph) {
            // A glyph that cannot be drawn shows nothing, like a space.
            let _ = outline.draw(DrawSettings::unhinted(Size::unscaled(), location), &mut pen);
        }
        let contours: Vec<Vec<DVec2>> = pen
            .finish()
            .into_iter()
            .filter_map(|c| clean(c, merge))
            .collect();
        if !contours.is_empty() {
            match nest(&contours, gap) {
                Some(faces) => {
                    let origin = DVec2::new(x, 0.0);
                    let baseline = -(line as f64) * LINE_SPACING * height;
                    let place = |ring: &Vec<DVec2>| -> Vec<DVec2> {
                        ring.iter()
                            .map(|p| DVec2::new((p.x + origin.x) * scale, p.y * scale + baseline))
                            .collect()
                    };
                    out.faces.extend(faces.iter().map(|f| TextFace {
                        index,
                        character: c,
                        outer: place(&f.outer),
                        holes: f.holes.iter().map(place).collect(),
                    }));
                }
                None => out.skipped.push(skip("its outlines cross or touch")),
            }
        }
        x += f64::from(advances.advance_width(glyph).unwrap_or(0.0));
    }
    out.touching = touching(&out.faces, MIN_GAP);
    out
}

/// The characters of `faces` with a side within `gap` of a side of another character.
/// Characters are compared by bounding box first; a pair whose boxes meet costs its sides
/// squared.
fn touching(faces: &[TextFace], gap: f64) -> BTreeSet<usize> {
    let mut glyphs: BTreeMap<usize, Vec<[DVec2; 2]>> = BTreeMap::new();
    for f in faces {
        let sides = glyphs.entry(f.index).or_default();
        for ring in std::iter::once(&f.outer).chain(&f.holes) {
            sides.extend((0..ring.len()).map(|i| [ring[i], ring[(i + 1) % ring.len()]]));
        }
    }
    let boxes: Vec<(usize, DVec2, DVec2)> = glyphs
        .iter()
        .map(|(&i, sides)| {
            let (lo, hi) = sides.iter().flatten().fold(
                (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
                |(lo, hi), p| (lo.min(*p), hi.max(*p)),
            );
            (i, lo - gap, hi + gap)
        })
        .collect();
    let mut out = BTreeSet::new();
    for (k, &(i, lo1, hi1)) in boxes.iter().enumerate() {
        for &(j, lo2, hi2) in &boxes[k + 1..] {
            if lo2.x > hi1.x || hi2.x < lo1.x || lo2.y > hi1.y || hi2.y < lo1.y {
                continue;
            }
            let close = glyphs[&i].iter().any(|&[a, b]| {
                glyphs[&j]
                    .iter()
                    .any(|&[p, q]| segment_distance(a, b, p, q) < gap)
            });
            if close {
                out.extend([i, j]);
            }
        }
    }
    out
}

/// Collects a glyph's contours, flattening its curves.
struct Flattener {
    tolerance: f64,
    contours: Vec<Vec<DVec2>>,
    current: Vec<DVec2>,
}

impl Flattener {
    fn new(tolerance: f64) -> Self {
        Self {
            tolerance,
            contours: Vec::new(),
            current: Vec::new(),
        }
    }

    fn last(&self) -> DVec2 {
        self.current.last().copied().unwrap_or(DVec2::ZERO)
    }

    /// Segments for a curve whose second differences reach `bend` (font units) with an error
    /// of `bend / (factor n²)` at n segments: the fewest within the tolerance, at most
    /// `MAX_CURVE_SEGMENTS`.
    fn segments(&self, bend: f64, factor: f64) -> usize {
        let n = (bend / (factor * self.tolerance)).sqrt().ceil();
        (n as usize).clamp(1, MAX_CURVE_SEGMENTS)
    }

    fn finish(mut self) -> Vec<Vec<DVec2>> {
        self.close();
        self.contours
    }
}

impl OutlinePen for Flattener {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        self.current.push(point(x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.current.push(point(x, y));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (p0, p1, p2) = (self.last(), point(cx0, cy0), point(x, y));
        // Chords of a quadratic stray at most |p0 - 2 p1 + p2| / (4 n²).
        let n = self.segments((p0 - 2.0 * p1 + p2).length(), 4.0);
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            self.current
                .push(u * u * p0 + 2.0 * u * t * p1 + t * t * p2);
        }
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (p0, p1, p2, p3) = (self.last(), point(cx0, cy0), point(cx1, cy1), point(x, y));
        // Chords of a cubic stray at most 3 max|second difference| / (4 n²).
        let bend = (p0 - 2.0 * p1 + p2)
            .length()
            .max((p1 - 2.0 * p2 + p3).length());
        let n = self.segments(3.0 * bend, 4.0);
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            self.current.push(
                u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3,
            );
        }
    }

    fn close(&mut self) {
        if !self.current.is_empty() {
            self.contours.push(std::mem::take(&mut self.current));
        }
    }
}

fn point(x: f32, y: f32) -> DVec2 {
    DVec2::new(f64::from(x), f64::from(y))
}

/// A contour without repeated points (closer than `merge`) or the closing repeat of its first
/// point, or None when fewer than three points or no area are left.
fn clean(contour: Vec<DVec2>, merge: f64) -> Option<Vec<DVec2>> {
    let mut ring: Vec<DVec2> = Vec::with_capacity(contour.len());
    for p in contour {
        if ring.last().is_none_or(|q| q.distance(p) > merge) {
            ring.push(p);
        }
    }
    while ring.len() > 1 && ring[0].distance(ring[ring.len() - 1]) <= merge {
        ring.pop();
    }
    // A point where the outline turns straight back (a spike) leaves two sides on top of
    // each other: it goes, and the check starts over.
    let mut i = 0;
    while ring.len() >= 3 && i < ring.len() {
        let n = ring.len();
        let (a, b, c) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
        let (u, v) = (b - a, c - b);
        if u.perp_dot(v).abs() <= 1e-9 * u.length() * v.length() && u.dot(v) < 0.0 {
            ring.remove(i);
            i = 0;
        } else {
            i += 1;
        }
    }
    if ring.len() < 3 {
        return None;
    }
    let size = ring
        .iter()
        .fold([ring[0], ring[0]], |[lo, hi], p| [lo.min(*p), hi.max(*p)]);
    let extent = (size[1] - size[0]).max_element();
    (area(&ring).abs() > 1e-6 * extent * extent).then_some(ring)
}

/// The signed area of a ring: positive when it winds counter-clockwise.
fn area(ring: &[DVec2]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| ring[i].perp_dot(ring[(i + 1) % n]))
        .sum::<f64>()
        / 2.0
}

/// Faces of a glyph's contours nested by containment (even-odd), or None when two of them
/// (or two sides of one that are not neighbours) cross or come closer than `gap`.
fn nest(contours: &[Vec<DVec2>], gap: f64) -> Option<Vec<TextFace>> {
    if crosses(contours, gap) {
        return None;
    }
    let n = contours.len();
    // Contours do not cross, so one point tells whether a contour lies inside another.
    let inside =
        |i: usize, j: usize| i != j && kernel::geom::point_in_polygon(contours[i][0], &contours[j]);
    let parents: Vec<Vec<usize>> = (0..n)
        .map(|i| (0..n).filter(|&j| inside(i, j)).collect())
        .collect();
    let mut faces: Vec<(usize, TextFace)> = Vec::new();
    for i in (0..n).filter(|&i| parents[i].len().is_multiple_of(2)) {
        let mut outer = contours[i].clone();
        if area(&outer) < 0.0 {
            outer.reverse();
        }
        faces.push((
            i,
            TextFace {
                // The caller sets the character.
                index: 0,
                character: ' ',
                outer,
                holes: Vec::new(),
            },
        ));
    }
    for i in (0..n).filter(|&i| !parents[i].len().is_multiple_of(2)) {
        // The contour just around a hole is the one inside all its other parents.
        let depth = parents[i].len() - 1;
        let owner = parents[i]
            .iter()
            .copied()
            .find(|&j| parents[j].len() == depth)?;
        let mut hole = contours[i].clone();
        if area(&hole) > 0.0 {
            hole.reverse();
        }
        faces
            .iter_mut()
            .find(|(o, _)| *o == owner)?
            .1
            .holes
            .push(hole);
    }
    Some(faces.into_iter().map(|(_, f)| f).collect())
}

/// True when two sides of the contours that are not neighbours on one contour cross or come
/// closer than `gap`. Quadratic in the number of sides of one glyph (a bounding-box check
/// skips most pairs), which is fine for the few dozen to few hundred sides of a letter.
fn crosses(contours: &[Vec<DVec2>], gap: f64) -> bool {
    let sides: Vec<(usize, usize, DVec2, DVec2)> = contours
        .iter()
        .enumerate()
        .flat_map(|(c, ring)| {
            (0..ring.len()).map(move |i| (c, i, ring[i], ring[(i + 1) % ring.len()]))
        })
        .collect();
    for (k, &(c1, i1, a, b)) in sides.iter().enumerate() {
        let (lo1, hi1) = (a.min(b) - gap, a.max(b) + gap);
        for &(c2, i2, p, q) in &sides[k + 1..] {
            if c1 == c2 {
                let len = contours[c1].len();
                if (i1 + 1) % len == i2 || (i2 + 1) % len == i1 {
                    continue;
                }
            }
            let (lo2, hi2) = (p.min(q), p.max(q));
            if hi2.x < lo1.x || lo2.x > hi1.x || hi2.y < lo1.y || lo2.y > hi1.y {
                continue;
            }
            if segment_distance(a, b, p, q) < gap {
                return true;
            }
        }
    }
    false
}

/// The distance between segments a-b and p-q (0 when they cross).
fn segment_distance(a: DVec2, b: DVec2, p: DVec2, q: DVec2) -> f64 {
    let d1 = (b - a).perp_dot(p - a);
    let d2 = (b - a).perp_dot(q - a);
    let d3 = (q - p).perp_dot(a - p);
    let d4 = (q - p).perp_dot(b - p);
    if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
        return 0.0;
    }
    let to = |x: DVec2, s: DVec2, e: DVec2| {
        let d = e - s;
        let t = if d.length_squared() > 0.0 {
            ((x - s).dot(d) / d.length_squared()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        x.distance(s + d * t)
    };
    to(a, p, q)
        .min(to(b, p, q))
        .min(to(p, a, b))
        .min(to(q, a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_with_counters_have_holes() {
        let shape = shape("o8%", 1.0);
        assert!(shape.skipped.is_empty(), "{:?}", shape.skipped);
        let holes: Vec<usize> = shape.faces.iter().map(|f| f.holes.len()).collect();
        // o: one face, one hole; 8: one face, two holes; %: two rings and a slash.
        assert_eq!(holes.iter().sum::<usize>(), 1 + 2 + 2, "{holes:?}");
        for f in &shape.faces {
            assert!(area(&f.outer) > 0.0);
            assert!(f.holes.iter().all(|h| area(h) < 0.0));
        }
    }

    #[test]
    fn curves_are_flattened_within_the_tolerance() {
        let mut pen = Flattener::new(1.0);
        pen.move_to(0.0, 0.0);
        pen.quad_to(50.0, 100.0, 100.0, 0.0);
        let rings = pen.finish();
        // 1 + up to 12 segments.
        assert!(rings[0].len() <= 1 + MAX_CURVE_SEGMENTS);
        let mut pen = Flattener::new(1000.0);
        pen.move_to(0.0, 0.0);
        pen.quad_to(50.0, 100.0, 100.0, 0.0);
        assert_eq!(
            pen.finish()[0].len(),
            2,
            "a loose tolerance takes one chord"
        );
    }

    #[test]
    fn capital_letters_are_the_height_tall_and_lines_step_down() {
        let one = shape("H", 2.0);
        let [lo, hi] = one.bounds().expect("H has a face");
        assert!((hi.y - 2.0).abs() < 0.02 && lo.y.abs() < 1e-9, "{lo} {hi}");
        let two = shape("H\nH", 2.0);
        let [lo, _] = two.bounds().expect("faces");
        assert!((lo.y + LINE_SPACING * 2.0).abs() < 1e-9);
    }

    #[test]
    fn spaces_move_on_and_missing_glyphs_are_skipped() {
        let a = shape("HH", 1.0).bounds().unwrap();
        let b = shape("H H", 1.0).bounds().unwrap();
        assert!(b[1].x > a[1].x);
        let s = shape("H\u{4E2D}\u{10FFFD}", 1.0);
        assert_eq!(
            s.skipped.iter().map(|k| k.index).collect::<Vec<_>>(),
            [1, 2],
            "{:?}",
            s.skipped
        );
    }
}
