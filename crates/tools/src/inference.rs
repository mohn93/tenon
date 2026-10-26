//! Snapping the cursor to meaningful 3D points.

use std::cell::Cell;

use glam::{DMat3, DVec2, DVec3};
use kernel::geom::LENGTH_TOL;
use kernel::{EdgeId, EntityId, FaceId, Guide, GuideId, Model, Shape, VertexId};
use session::{Camera, Ray};

use crate::scenery::{Label, Owner, Part, Scenery, Stretch};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    /// The unit direction of the axis in drawing axes `axes` (their directions as columns, as
    /// a frame's `matrix3`; `DMat3::IDENTITY` for the world axes).
    pub fn dir_in(self, axes: DMat3) -> DVec3 {
        match self {
            Axis::X => axes.x_axis,
            Axis::Y => axes.y_axis,
            Axis::Z => axes.z_axis,
        }
    }
}

/// What the cursor snapped to, from strongest to weakest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapKind {
    Endpoint,
    Midpoint,
    Origin,
    /// A guide point.
    GuidePoint,
    /// Where two edges or guide lines cross on screen (or in space): the point on the nearer
    /// one.
    Intersection,
    OnEdge,
    /// On a guide line.
    OnGuide,
    /// Along a red/green/blue axis through the point the tool is drawing from.
    Axis(Axis),
    /// Parallel to the reference edge, through the point the tool is drawing from.
    Parallel,
    /// Perpendicular to the reference edge, through the point the tool is drawing from.
    Perpendicular,
    OnFace,
    Ground,
    /// Nothing to snap to: a point in front of the camera under the cursor.
    Free,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inference {
    pub point: DVec3,
    pub kind: SnapKind,
    /// The nearest face under the cursor, whatever the point snapped to, if it belongs to the
    /// open context (a face inside an instance or outside the open instance is not given).
    pub face: Option<FaceId>,
    /// Under a direction lock, the kind of point snap (an endpoint, a midpoint...) the locked
    /// point was projected from, so the point still counts as one of the model's although
    /// `kind` reports the lock. None otherwise.
    pub locked: Option<SnapKind>,
}

impl Inference {
    /// The kind of point snap this is, whatever the lock: an endpoint, midpoint, origin, guide
    /// point, intersection, point on an edge or on a guide line. None for the rest.
    pub fn point_snap(&self) -> Option<SnapKind> {
        let kind = self.locked.unwrap_or(self.kind);
        matches!(
            kind,
            SnapKind::Endpoint
                | SnapKind::Midpoint
                | SnapKind::Origin
                | SnapKind::GuidePoint
                | SnapKind::Intersection
                | SnapKind::OnEdge
                | SnapKind::OnGuide
        )
        .then_some(kind)
    }
}

/// What inference can follow from the point a tool is drawing from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Guides {
    /// The point a tool is drawing from; directions are measured from it.
    pub anchor: Option<DVec3>,
    /// Unit direction of the last edge the cursor rested on: snaps parallel and perpendicular
    /// to it are offered from the anchor.
    pub reference: Option<DVec3>,
    /// A locked direction (Shift or an arrow key): the point stays on the line through the
    /// anchor, and points the cursor snaps to are projected onto it.
    pub lock: Option<Lock>,
}

/// A direction inference is held to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lock {
    /// Unit direction.
    pub dir: DVec3,
    /// How the locked point is reported (an axis, parallel or perpendicular).
    pub kind: SnapKind,
}

/// Finds the 3D point under `cursor` (physical pixels) on one mesh, with no guides but
/// `anchor`, which enables axis inference. See [`infer_in`].
pub fn infer(
    model: &Model,
    camera: &Camera,
    cursor: DVec2,
    anchor: Option<DVec3>,
    radius_px: f64,
) -> Inference {
    let guides = Guides {
        anchor,
        ..Guides::default()
    };
    infer_with(model, camera, cursor, &guides, radius_px)
}

/// [`infer_in`] on one mesh.
pub fn infer_with(
    model: &Model,
    camera: &Camera,
    cursor: DVec2,
    guides: &Guides,
    radius_px: f64,
) -> Inference {
    infer_in(&Scenery::single(model), camera, cursor, guides, radius_px)
}

/// Finds the 3D point under `cursor` (physical pixels) among everything `scenery` holds, in
/// the open context's coordinates (`camera` must be in them too). Snap candidates count only
/// within `radius_px` of the cursor.
///
/// Ranking: endpoint, midpoint, origin and guide point (nearest on screen wins) > intersection
/// (of edges and guide lines) > on-edge > on-guide-line > directions from the anchor (axes,
/// parallel or perpendicular to the reference edge; nearest wins) > on-face > ground > free.
/// Points and edges hidden behind a face are skipped. With a lock, the result is always on
/// the locked line.
pub fn infer_in(
    scenery: &Scenery,
    camera: &Camera,
    cursor: DVec2,
    guides: &Guides,
    radius_px: f64,
) -> Inference {
    infer_detail(scenery, camera, cursor, guides, radius_px).0
}

/// Entities a snap involves, each with the index of its part in the `Scenery`.
pub type Involved = Vec<(usize, EntityId)>;

/// [`infer_in`], also giving the entities the point snapped to: the vertex of an endpoint;
/// the edge of a midpoint or of a point on an edge; the guide of a guide point or of a point on
/// a guide line; the two edges or guide lines that cross at an intersection, the one the
/// point lies on first; the face of a point on a face. None for the origin, a direction from
/// the anchor, the ground, a free point or a locked direction.
pub fn infer_detail(
    scenery: &Scenery,
    camera: &Camera,
    cursor: DVec2,
    guides: &Guides,
    radius_px: f64,
) -> (Inference, Involved) {
    // Inference never snaps to what is hidden, whether or not picking reveals it.
    let strict;
    let scenery = if scenery.reveals_hidden() {
        strict = scenery.seen_strictly();
        &strict
    } else {
        scenery
    };
    let ray = camera.ray(cursor);
    let face_hit = nearest_face_part(scenery, &ray);
    let face = face_hit.and_then(|(k, f, _)| (scenery.part(k).owner == Owner::Open).then_some(f));
    let projector = camera.projector();
    let screen_dist = |p: DVec3| projector.project(p).map(|s| s.distance(cursor));
    let snapped = snap_point(scenery, camera, cursor, radius_px);

    if let (Some(anchor), Some(lock)) = (guides.anchor, guides.lock) {
        let point = match snapped {
            Some((p, _, _)) => anchor + lock.dir * (p - anchor).dot(lock.dir),
            None => closest_on_line_to_ray(anchor, lock.dir, &ray, false),
        };
        let hit = Inference {
            point,
            kind: lock.kind,
            face,
            locked: snapped.as_ref().map(|(_, kind, _)| *kind),
        };
        return (hit, Vec::new());
    }
    if let Some((point, kind, involved)) = snapped {
        return (
            Inference {
                point,
                kind,
                face,
                locked: None,
            },
            involved,
        );
    }

    // Along a direction from the anchor. Direction lines are guides, so faces do not hide them.
    if let Some(anchor) = guides.anchor {
        let mut along: Option<(f64, Inference)> = None;
        for (dir, kind) in directions(scenery.axes().matrix3, guides.reference) {
            let p = closest_on_line_to_ray(anchor, dir, &ray, false);
            if let Some(d) = screen_dist(p).filter(|d| *d <= radius_px)
                && p.distance(anchor) > 0.0
                && scenery.slab().is_none_or(|s| s.holds(p))
                && along.is_none_or(|(bd, _)| d < bd)
            {
                along = Some((
                    d,
                    Inference {
                        point: p,
                        kind,
                        face,
                        locked: None,
                    },
                ));
            }
        }
        if let Some((_, hit)) = along {
            return (hit, Vec::new());
        }
    }

    // On a face (nearest along the ray).
    if let Some((k, f, point)) = face_hit {
        let hit = Inference {
            point,
            kind: SnapKind::OnFace,
            face,
            locked: None,
        };
        return (hit, vec![(k, EntityId::Face(f))]);
    }

    // Within a slab, the cursor stands for its point: on the ground there or in free space.
    if let Some(slab) = scenery.slab() {
        let kind = if slab.point.z.abs() <= LENGTH_TOL {
            SnapKind::Ground
        } else {
            SnapKind::Free
        };
        let hit = Inference {
            point: slab.point,
            kind,
            face,
            locked: None,
        };
        return (hit, Vec::new());
    }
    let hit = match camera.ground_at(cursor) {
        Some(point) => Inference {
            point,
            kind: SnapKind::Ground,
            face,
            locked: None,
        },
        None => Inference {
            point: camera.target_depth_at(cursor),
            kind: SnapKind::Free,
            face,
            locked: None,
        },
    };
    (hit, Vec::new())
}

/// The visible point snap under the cursor, if any, with the entities it involves (see
/// `infer_detail`): endpoint, midpoint, origin or guide point (nearest on screen), else the
/// nearest apparent intersection of two edges or guide lines both visible there, else the
/// nearest point on an edge, else on a guide line.
fn snap_point(
    scenery: &Scenery,
    camera: &Camera,
    cursor: DVec2,
    radius_px: f64,
) -> Option<(DVec3, SnapKind, Involved)> {
    let ray = camera.ray(cursor);
    let projector = camera.projector();
    // How far from the cursor a point is on screen, if within the radius.
    let near_enough = |p: DVec3| {
        projector
            .project(p)
            .map(|s| s.distance(cursor))
            .filter(|d| *d <= radius_px)
    };
    let sight = Sight::new(scenery, camera);
    let seen = |p: &DVec3| sight.sees(*p);
    let near = Near::around(scenery, camera, cursor, radius_px);
    let mut points: Vec<(f64, (DVec3, SnapKind, Involved))> = Vec::new();
    let mut offer = |p: DVec3, kind: SnapKind, involved: Involved| {
        if let Some(d) = near_enough(p) {
            points.push((d, (p, kind, involved)));
        }
    };
    for &(k, v, p) in &near.points {
        offer(p, SnapKind::Endpoint, vec![(k, EntityId::Vertex(v))]);
    }
    for &(k, e, a, b) in &near.segments {
        let middle = (a + b) * 0.5;
        if scenery.part(k).kept(middle) {
            offer(middle, SnapKind::Midpoint, vec![(k, EntityId::Edge(e))]);
        }
    }
    offer(scenery.axes().translation, SnapKind::Origin, Vec::new());
    let guides = scenery.guides();
    for &(k, id, g) in guides {
        if let Guide::Point { position, .. } = g {
            offer(
                position,
                SnapKind::GuidePoint,
                vec![(k, EntityId::Guide(id))],
            );
        }
    }
    if let Some((_, snap)) = nearest_visible(points, |(p, _, _)| seen(p)) {
        return Some(snap);
    }

    // Guide lines passing near the cursor, as the part of them in view, each with its part
    // and id.
    let guide_lines: Vec<(usize, GuideId, DVec3, DVec3)> = guides
        .iter()
        .filter_map(|&(k, id, g)| match g {
            Guide::Line {
                point, direction, ..
            } => Some((k, id, point, direction)),
            Guide::Point { .. } => None,
        })
        .filter(|&(_, _, point, dir)| {
            let p = closest_on_line_to_ray(point, dir, &ray, false);
            projector
                .project(p)
                .is_some_and(|s| s.distance(cursor) <= radius_px)
        })
        .collect();
    // Two lines cross within the radius only where both pass that near the cursor on screen,
    // so only lines passing within twice the radius (a margin far above any rounding) are
    // paired, each projected once.
    let lines: Vec<ScreenLine> = near
        .segments
        .iter()
        .map(|&(k, e, a, b)| ((k, EntityId::Edge(e)), (a, b)))
        .chain(guide_lines.iter().filter_map(|&(k, id, point, dir)| {
            let [a, b] = camera.line_span(point, dir)?;
            Some(((k, EntityId::Guide(id)), (a, b)))
        }))
        .filter_map(|(who, (a, b))| {
            let (sa, sb) = (projector.project(a)?, projector.project(b)?);
            (distance_to_segment(cursor, sa, sb) <= 2.0 * radius_px).then_some(ScreenLine {
                who,
                ends: (a, b),
                screen: (sa, sb),
            })
        })
        .collect();
    let mut crossings = Vec::new();
    for (i, ab) in lines.iter().enumerate() {
        for cd in &lines[i + 1..] {
            if let Some((p, far, ab_nearer)) = apparent_crossing(camera, ab, cd)
                && let Some(dist) = near_enough(p)
                && [ab.who, cd.who]
                    .iter()
                    .all(|&(k, id)| !matches!(id, EntityId::Edge(_)) || scenery.part(k).kept(p))
            {
                let involved = if ab_nearer {
                    vec![ab.who, cd.who]
                } else {
                    vec![cd.who, ab.who]
                };
                crossings.push((dist, (p, far, involved)));
            }
        }
    }
    if let Some((_, (p, _, involved))) =
        nearest_visible(crossings, |(p, far, _)| seen(far) && seen(p))
    {
        return Some((p, SnapKind::Intersection, involved));
    }

    let on_edge = near
        .segments
        .iter()
        .filter_map(|&(k, e, a, b)| {
            let p = closest_on_line_to_ray(a, b - a, &ray, true);
            if !scenery.part(k).kept(p) {
                return None;
            }
            near_enough(p).map(|d| (d, (p, (k, EntityId::Edge(e)))))
        })
        .collect();
    if let Some((_, (p, who))) = nearest_visible(on_edge, |(p, _)| seen(p)) {
        return Some((p, SnapKind::OnEdge, vec![who]));
    }

    let on_guide = guide_lines
        .iter()
        .filter_map(|&(k, id, point, dir)| {
            let p = closest_on_line_to_ray(point, dir, &ray, false);
            near_enough(p).map(|d| (d, (p, (k, EntityId::Guide(id)))))
        })
        .collect();
    nearest_visible(on_guide, |(p, _)| seen(p))
        .map(|(_, (p, who))| (p, SnapKind::OnGuide, vec![who]))
}

/// The nearest of `candidates` (each with its distance from the cursor on screen) that `seen`
/// holds for: the least distance, the first given among equally near ones. Seeing is the
/// dearest test, so candidates are tried nearest first and those after the answer never are.
fn nearest_visible<T>(
    mut candidates: Vec<(f64, T)>,
    mut seen: impl FnMut(&T) -> bool,
) -> Option<(f64, T)> {
    // A stable sort: equally near candidates stay in the order given.
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    candidates.into_iter().find(|(_, c)| seen(c))
}

/// The visible guide nearest the cursor on screen whose owner passes `keep`, if one lies
/// within `radius_px`: whose it is, the guide (an id in its own mesh) and the guide placed in
/// the open context's coordinates.
pub fn guide_under_where(
    scenery: &Scenery,
    camera: &Camera,
    cursor: DVec2,
    radius_px: f64,
    keep: impl Fn(Owner) -> bool,
) -> Option<(Owner, GuideId, Guide)> {
    let ray = camera.ray(cursor);
    let projector = camera.projector();
    // Guides are few, so they are scanned, not indexed.
    let candidates = scenery
        .pick_guides()
        .filter_map(|&(part, id, g)| {
            let owner = scenery.part(part).owner;
            if !keep(owner) {
                return None;
            }
            let p = match g {
                Guide::Point { position, .. } => position,
                Guide::Line {
                    point, direction, ..
                } => closest_on_line_to_ray(point, direction, &ray, false),
            };
            let d = projector.project(p)?.distance(cursor);
            (d <= radius_px).then_some((d, (owner, id, g, p)))
        })
        .collect();
    let strict;
    let strict = if scenery.reveals_hidden() {
        strict = scenery.seen_strictly();
        &strict
    } else {
        scenery
    };
    let sight = Sight::new(strict, camera);
    nearest_visible(candidates, |&(.., p)| sight.sees(p))
        .map(|(_, (owner, id, g, _))| (owner, id, g))
}

/// The topmost label under `cursor` whose owner passes `keep` (labels drawn later lie on
/// top): whose it is and the dimension or note.
pub fn label_under(
    labels: &[Label],
    cursor: DVec2,
    keep: impl Fn(Owner) -> bool,
) -> Option<(Owner, EntityId)> {
    labels
        .iter()
        .rev()
        .find(|l| keep(l.owner) && l.contains(cursor))
        .map(|l| (l.owner, l.id))
}

/// The visible dimension, leader note or section plane whose lines pass nearest the cursor on screen, within
/// `radius_px`, among those whose owner passes `keep`: whose it is and the annotation (an id
/// in its own mesh). Screen notes have no lines; their labels pick them (`label_under`).
pub fn annotation_under_where(
    scenery: &Scenery,
    camera: &Camera,
    cursor: DVec2,
    radius_px: f64,
    keep: impl Fn(Owner) -> bool,
) -> Option<(Owner, EntityId)> {
    let ray = camera.ray(cursor);
    let projector = camera.projector();
    // Annotations and section planes (by their outlines) are few, so they are scanned, not
    // indexed.
    let lines = scenery
        .annotations()
        .iter()
        .map(|(part, id, shape)| (*part, *id, shape.segments()))
        .chain(
            scenery
                .sections()
                .iter()
                .map(|(part, id, shape)| (*part, *id, shape.segments().to_vec())),
        );
    let candidates = lines
        .filter(|(part, ..)| keep(scenery.part(*part).owner))
        .flat_map(|(part, id, segments)| {
            let owner = scenery.part(part).owner;
            let ray = &ray;
            segments
                .into_iter()
                .map(move |[a, b]| (closest_on_line_to_ray(a, b - a, ray, true), owner, id))
        })
        .filter_map(|(p, owner, id)| {
            let d = projector.project(p)?.distance(cursor);
            (d <= radius_px).then_some((d, (owner, id, p)))
        })
        .collect();
    let sight = Sight::new(scenery, camera);
    nearest_visible(candidates, |&(.., p)| sight.sees(p)).map(|(_, (owner, id, _))| (owner, id))
}

/// A segment, where its ends are on screen, and the edge or guide line it is (with its part).
struct ScreenLine {
    who: (usize, EntityId),
    ends: (DVec3, DVec3),
    screen: (DVec2, DVec2),
}

/// Distance on screen from `p` to the segment `a`-`b`.
fn distance_to_segment(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 > 0.0 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

/// Where segments a-b and c-d cross on screen away from their ends: the points under the
/// crossing on the nearer and the farther segment, nearer first, and whether a-b is the nearer.
/// Edges that meet in space share a vertex (the kernel splits crossing edges), so their
/// crossing is an endpoint, not this.
fn apparent_crossing(
    camera: &Camera,
    ab: &ScreenLine,
    cd: &ScreenLine,
) -> Option<(DVec3, DVec3, bool)> {
    let ((a, b), (sa, sb)) = (ab.ends, ab.screen);
    let ((c, d), (sc, sd)) = (cd.ends, cd.screen);
    let (r, q) = (sb - sa, sd - sc);
    let denom = r.perp_dot(q);
    if denom.abs() < 1e-12 {
        return None;
    }
    let t = (sc - sa).perp_dot(q) / denom;
    let u = (sc - sa).perp_dot(r) / denom;
    let inside = |k: f64| k > 1e-6 && k < 1.0 - 1e-6;
    if !inside(t) || !inside(u) {
        return None;
    }
    let ray = camera.ray(sa + r * t);
    let on_ab = closest_on_line_to_ray(a, b - a, &ray, true);
    let on_cd = closest_on_line_to_ray(c, d - c, &ray, true);
    Some(
        if on_ab.distance(ray.origin) <= on_cd.distance(ray.origin) {
            (on_ab, on_cd, true)
        } else {
            (on_cd, on_ab, false)
        },
    )
}

/// Unit directions offered from the anchor: the three drawing axes `axes` (directions as
/// columns; `DMat3::IDENTITY` for the world axes), plus, for a reference direction not along
/// an axis, the direction itself (parallel) and two perpendiculars (one horizontal, one in the
/// vertical plane through the reference; up is the blue axis) when they are not along an axis
/// either.
pub fn directions(axes: DMat3, reference: Option<DVec3>) -> Vec<(DVec3, SnapKind)> {
    let mut out: Vec<(DVec3, SnapKind)> = Axis::ALL
        .into_iter()
        .map(|a| (a.dir_in(axes), SnapKind::Axis(a)))
        .collect();
    let along_axis = |d: DVec3| {
        Axis::ALL
            .iter()
            .any(|a| d.cross(a.dir_in(axes)).length() < 1e-6)
    };
    let Some(r) = reference.and_then(|r| r.try_normalize()) else {
        return out;
    };
    if along_axis(r) {
        return out;
    }
    out.push((r, SnapKind::Parallel));
    let horizontal = r.cross(axes.z_axis).try_normalize();
    let upright = horizontal.and_then(|h| r.cross(h).try_normalize());
    for p in [horizontal, upright].into_iter().flatten() {
        if !along_axis(p) {
            out.push((p, SnapKind::Perpendicular));
        }
    }
    out
}

/// The nearest face of one mesh under the cursor and the point where the cursor's ray meets it.
pub fn face_under(model: &Model, camera: &Camera, cursor: DVec2) -> Option<(FaceId, DVec3)> {
    face_under_in(&Scenery::single(model), camera, cursor).map(|(_, f, p)| (f, p))
}

/// The nearest face under the cursor among everything `scenery` holds: whose it is, the face
/// (an id in its own mesh) and where the cursor's ray meets it.
pub fn face_under_in(
    scenery: &Scenery,
    camera: &Camera,
    cursor: DVec2,
) -> Option<(Owner, FaceId, DVec3)> {
    nearest_face_hit(scenery, &camera.ray(cursor))
}

/// The visible edge of one mesh nearest the cursor on screen, if one lies within `radius_px`.
pub fn edge_under(model: &Model, camera: &Camera, cursor: DVec2, radius_px: f64) -> Option<EdgeId> {
    edge_under_in(&Scenery::single(model), camera, cursor, radius_px).map(|(_, e, _, _)| e)
}

/// The visible edge nearest the cursor on screen among everything `scenery` holds, if one
/// lies within `radius_px`: whose it is, the edge (an id in its own mesh) and its ends.
pub fn edge_under_in(
    scenery: &Scenery,
    camera: &Camera,
    cursor: DVec2,
    radius_px: f64,
) -> Option<(Owner, EdgeId, DVec3, DVec3)> {
    // Inference goes by this, so it never meets a hidden edge: that is `edge_under_where`.
    let strict;
    let scenery = if scenery.reveals_hidden() {
        strict = scenery.seen_strictly();
        &strict
    } else {
        scenery
    };
    edge_under_where(scenery, camera, cursor, radius_px, |_| true)
}

/// Like `edge_under_in`, but only edges whose owner passes `keep` compete, and with hidden
/// entities revealed in `scenery`, edges hidden by their own flag compete too (picking).
/// Visibility is still tested against every part seen strictly, so an edge hidden behind other
/// geometry is not found.
pub fn edge_under_where(
    scenery: &Scenery,
    camera: &Camera,
    cursor: DVec2,
    radius_px: f64,
    keep: impl Fn(Owner) -> bool,
) -> Option<(Owner, EdgeId, DVec3, DVec3)> {
    let ray = camera.ray(cursor);
    let projector = camera.projector();
    let candidates = Near::around(scenery, camera, cursor, radius_px)
        .segments
        .into_iter()
        .filter_map(|(part, id, a, b)| {
            let owner = scenery.part(part).owner;
            if !keep(owner) {
                return None;
            }
            let p = closest_on_line_to_ray(a, b - a, &ray, true);
            if !scenery.part(part).kept(p) {
                return None;
            }
            let d = projector.project(p)?.distance(cursor);
            (d <= radius_px).then_some((d, (owner, id, a, b, p)))
        })
        .collect();
    let strict;
    let strict = if scenery.reveals_hidden() {
        strict = scenery.seen_strictly();
        &strict
    } else {
        scenery
    };
    let sight = Sight::new(strict, camera);
    nearest_visible(candidates, |&(.., p)| sight.sees(p))
        .map(|(_, (owner, id, a, b, _))| (owner, id, a, b))
}

/// The vertices and edges that may lie within `radius_px` of the cursor on screen (or in a
/// cone), with the index of the part each comes from and positions in the open context's
/// coordinates, in part and id order so that ties between equally near candidates always go
/// the same way. Soft edges are hidden, so they are left out: nothing snaps to them or picks
/// them (their ends are vertices of the edges around them and still snap). Edges on hidden
/// tags are left out too, and vertices no shown edge or face uses.
pub(crate) struct Near {
    pub(crate) points: Vec<(usize, VertexId, DVec3)>,
    pub(crate) segments: Vec<(usize, EdgeId, DVec3, DVec3)>,
}

impl Near {
    fn around(scenery: &Scenery, camera: &Camera, cursor: DVec2, radius_px: f64) -> Near {
        Near::in_cone(scenery, Cone::around(camera, cursor, radius_px))
    }

    /// Those that may lie in `cone`, in the open context's coordinates.
    pub(crate) fn in_cone(scenery: &Scenery, cone: Cone) -> Near {
        let mut near = Near {
            points: Vec::new(),
            segments: Vec::new(),
        };
        for (k, part) in scenery.parts().enumerate() {
            // A part whose box misses the cone has nothing in it.
            if !part.may_meet_cone(cone.origin, cone.dir, cone.r0, cone.slope) {
                continue;
            }
            let local = cone.carried(part.back, part.stretch());
            let at = |p: DVec3| part.place.transform_point3(p);
            part.model.index().near_ray(
                local.origin,
                local.dir,
                local.r0,
                local.slope,
                |s| match *s {
                    Shape::Point(v, p) => {
                        if part.shows_vertex(v) && part.kept(at(p)) {
                            near.points.push((k, v, at(p)));
                        }
                    }
                    Shape::Segment(e, a, b) => {
                        if part.model.edge(e).is_some_and(|edge| !edge.soft)
                            && part.shows_edge(e)
                            && !part.cuts_away(&[at(a), at(b)])
                        {
                            near.segments.push((k, e, at(a), at(b)));
                        }
                    }
                    Shape::Triangle(..) => {}
                },
            );
        }
        near.points.sort_by_key(|(k, v, _)| (*k, *v));
        near.segments.sort_by_key(|(k, e, _, _)| (*k, *e));
        near
    }
}

/// The points within `r0 + slope * t` of the ray `origin + t * dir` (unit `dir`; `t` counted
/// from 0, and `r0` behind the origin): what `Index::near_ray` and `Placed::may_meet_cone`
/// search.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Cone {
    origin: DVec3,
    dir: DVec3,
    r0: f64,
    slope: f64,
}

impl Cone {
    /// A cone around the ray through `cursor` holding every ray through the square of pixels
    /// within `radius_px` of it across and down, so everything within `radius_px` of it on
    /// screen, at any field of view and anywhere on screen.
    ///
    /// The rays through the square are those through its four corners and in between: a
    /// pixel of the square is a weighted mean of the corners, and `Camera::ray` takes the
    /// pixel to a direction before scaling it to unit length (perspective) or to a starting
    /// point (parallel) by a map that keeps weighted means. In perspective every ray starts
    /// at the eye: a corner ray leans from the cursor's by an angle whose tangent the slope
    /// takes, and the directions leaning no more than the most leaning corner's form a convex
    /// cone (that angle being under a right angle), which therefore holds the direction of
    /// every ray through the square. In a parallel view every ray runs along the view: a ray
    /// lies as far from the cursor's as its start does, a convex function of the pixel, so no
    /// farther than the farthest corner's, which `r0` takes. The pick disk lies inside the
    /// square and touches it only at the middles of its sides, which lie strictly inside the
    /// cone (its rim, an ellipse or other conic in perspective and a circle in a parallel
    /// view, holds no straight piece between two corners), so rounding in where a point lands
    /// on screen cannot carry a candidate out of it.
    pub(crate) fn around(camera: &Camera, cursor: DVec2, radius_px: f64) -> Cone {
        let ray = camera.ray(cursor);
        let (mut r0, mut slope) = (0.0_f64, 0.0_f64);
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let corner = camera.ray(cursor + DVec2::new(x, y) * radius_px);
            r0 = r0.max(corner.origin.distance(ray.origin));
            let cos = corner.dir.dot(ray.dir);
            if cos.is_nan() || cos <= 0.0 {
                // A square so large on so small a view that it reaches a right angle off the
                // cursor's ray: no narrower cone holds it.
                return Cone::whole(ray.origin, ray.dir);
            }
            slope = slope.max(corner.dir.cross(ray.dir).length() / cos);
        }
        Cone {
            origin: ray.origin,
            dir: ray.dir,
            r0,
            slope,
        }
    }

    /// The points within `radius` of the ray `origin + t * dir` (unit `dir`), and `radius`
    /// behind its origin.
    pub(crate) fn along(origin: DVec3, dir: DVec3, radius: f64) -> Cone {
        Cone {
            origin,
            dir,
            r0: radius,
            slope: 0.0,
        }
    }

    /// A cone holding everything.
    fn whole(origin: DVec3, dir: DVec3) -> Cone {
        Cone {
            origin,
            dir,
            r0: f64::INFINITY,
            slope: 0.0,
        }
    }

    /// A cone in the coordinates `back` maps into holding everything `back` maps this one's
    /// points to: the cone a part's index searches, `back` being the inverse of the part's
    /// placement and `stretch` how it stretches lengths. A placement that turns, moves,
    /// mirrors and scales evenly carries the cone as it is, its `r0` scaled; one that
    /// stretches unevenly or shears needs a wider one.
    ///
    /// With `B` the linear part of `back`, a point `origin + dir * t + w` (`w` across `dir`)
    /// lands at `B w` from the carried axis's point at `lambda * t`, where `lambda = |B dir|`.
    /// Across the carried axis that is at most `|B w| <= sigma * |w|` off it, `sigma` being
    /// `stretch.most`; along it, at most `mu * |w|` from `lambda * t`, where `mu = stretch.off /
    /// lambda` (`B w` and `B dir` have a dot product of at most `stretch.off * |w|`). For `t <=
    /// 0`, `|w| <= r0`. For `t > 0`, `|w| <= r0 + slope * t` and `lambda * t <= t' + mu * |w|`
    /// at the carried distance `t'`, so `|w| * k <= r0 + slope * max(t', 0) / lambda` with `k =
    /// 1 - slope * mu / lambda`. Hence the point lies within `sigma / k * (r0 + slope / lambda
    /// * max(t', 0))` of the carried axis.
    fn carried(&self, back: glam::DAffine3, stretch: Stretch) -> Cone {
        let origin = back.transform_point3(self.origin);
        let axis = back.transform_vector3(self.dir);
        let lambda = axis.length();
        let dir = axis / lambda;
        let (sigma, mu) = (stretch.most, stretch.off / lambda);
        let k = 1.0 - self.slope * mu / lambda;
        if k.is_nan() || k <= 0.0 || !sigma.is_finite() {
            return Cone::whole(origin, dir);
        }
        Cone {
            origin,
            dir,
            r0: sigma * self.r0 / k,
            slope: sigma * self.slope / (lambda * k),
        }
    }
}

/// Tells which points the camera sees.
struct Sight<'s> {
    scenery: &'s Scenery<'s>,
    camera: &'s Camera,
    /// The part that last hid a point. Points near the cursor are often hidden by the same
    /// thing, so it is tried first.
    blocker: Cell<Option<usize>>,
}

impl<'s> Sight<'s> {
    fn new(scenery: &'s Scenery<'s>, camera: &'s Camera) -> Self {
        Sight {
            scenery,
            camera,
            blocker: Cell::new(None),
        }
    }

    /// True if no face lies between where the camera's sight of `p` starts (the eye, in
    /// perspective) and `p` (faces through `p` itself do not hide it). Within a slab, only
    /// points in it are seen, and only faces through its point hide them.
    fn sees(&self, p: DVec3) -> bool {
        let slab = self.scenery.slab();
        if slab.is_some_and(|s| !s.holds(p)) {
            return false;
        }
        let eye = self.camera.sight_origin(p);
        let to = p - eye;
        let dist = to.length();
        if dist < 1e-12 {
            return true;
        }
        let dir = to / dist;
        let reach = dist * (1.0 - 1e-6);
        let from = slab
            .and_then(|s| s.face_span(eye, dir))
            .map_or(0.0, |(from, _)| from);
        if from >= reach {
            return true;
        }
        // The fill of a section cut hides all that lies behind it.
        if self.scenery.fill_hit(eye, dir, from, reach).is_some() {
            return false;
        }
        let hides = |part: Part| {
            part.may_meet_ray(eye, dir, reach)
                && part
                    .model
                    .index()
                    .ray_hit_at(
                        part.back.transform_point3(eye),
                        part.back.transform_vector3(dir),
                        from,
                        reach,
                        |f, t| part.shows_face(f) && part.kept(eye + dir * t),
                    )
                    .is_some()
        };
        if let Some(k) = self.blocker.get()
            && hides(self.scenery.part(k))
        {
            return false;
        }
        match self.scenery.parts().position(hides) {
            Some(k) => {
                self.blocker.set(Some(k));
                false
            }
            None => true,
        }
    }
}

/// Point on the line `origin + t * dir` closest to the ray; `t` is clamped to [0, 1] when
/// `segment` is true (dir is then the segment vector).
pub(crate) fn closest_on_line_to_ray(origin: DVec3, dir: DVec3, ray: &Ray, segment: bool) -> DVec3 {
    let w0 = origin - ray.origin;
    let a = dir.dot(dir);
    let b = dir.dot(ray.dir);
    let c = ray.dir.dot(ray.dir);
    let d = dir.dot(w0);
    let e = ray.dir.dot(w0);
    let denom = a * c - b * b;
    let mut t = if denom.abs() < 1e-12 * a * c {
        0.0
    } else {
        (b * e - c * d) / denom
    };
    if segment {
        t = t.clamp(0.0, 1.0);
    }
    origin + dir * t
}

/// How much nearer (meters along the ray) one part's face must be than another's to win; within
/// it the later part wins, so a picture or other group lying flush on a face of the context is
/// the one met, not the face below it.
const FACE_TIE: f64 = kernel::geom::LENGTH_TOL;

/// The nearest face the ray meets in front of its origin: whose it is, the face, and where.
/// Faces on hidden tags are passed through. Faces equally near (see `FACE_TIE`) go to the later
/// part.
fn nearest_face_hit(scenery: &Scenery, ray: &Ray) -> Option<(Owner, FaceId, DVec3)> {
    nearest_face_part(scenery, ray).map(|(k, f, p)| (scenery.part(k).owner, f, p))
}

/// [`nearest_face_hit`] giving the index of the face's part instead of its owner.
fn nearest_face_part(scenery: &Scenery, ray: &Ray) -> Option<(usize, FaceId, DVec3)> {
    // Within a slab, only faces through its point are met.
    let (from, to) = match scenery.slab() {
        Some(slab) => slab.face_span(ray.origin, ray.dir)?,
        None => (0.0, f64::INFINITY),
    };
    // Nothing behind the fill of a section cut is met.
    let to = scenery
        .fill_hit(ray.origin, ray.dir, from, to)
        .map_or(to, |t| t + 1e-9);
    let mut best: Option<(f64, usize, FaceId)> = None;
    for (k, part) in scenery.parts().enumerate() {
        let reach = best.map_or(to, |(t, _, _)| t + FACE_TIE);
        if !part.may_meet_ray(ray.origin, ray.dir, reach) {
            continue;
        }
        if let Some((f, t)) = part.model.index().ray_hit_at(
            part.back.transform_point3(ray.origin),
            part.back.transform_vector3(ray.dir),
            from,
            reach,
            |f, t| part.shows_face(f) && part.kept(ray.origin + ray.dir * t),
        ) && best.is_none_or(|(bt, _, _)| t <= bt + FACE_TIE)
        {
            best = Some((t, k, f));
        }
    }
    best.map(|(t, k, f)| (k, f, ray.origin + ray.dir * t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenery::Slab;

    const R: f64 = 10.0;

    fn cam() -> Camera {
        Camera::new(DVec2::new(800.0, 600.0))
    }

    fn px(c: &Camera, p: DVec3) -> DVec2 {
        c.project(p).unwrap()
    }

    fn square(m: &mut Model, z: f64) {
        let c = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            m.add_edge(DVec3::new(a.0, a.1, z), DVec3::new(b.0, b.1, z))
                .unwrap();
        }
    }

    #[test]
    fn within_a_slab_a_hidden_point_is_found_where_it_is() {
        let c = cam();
        let mut m = Model::new();
        square(&mut m, 0.0);
        let floor = m.faces().next().unwrap().0;
        // A card between the eye and the floor's middle, square to the view.
        let p = DVec3::new(1.0, 1.0, 0.0);
        let mid = c.eye.lerp(p, 0.5);
        let (r, u) = (c.right(), c.up());
        m.add_polygon(&[mid - r - u, mid + r - u, mid + r + u, mid - r + u])
            .unwrap();
        let cursor = px(&c, p);
        let open = Scenery::single(&m);
        let hidden = infer_in(&open, &c, cursor, &Guides::default(), R);
        assert_eq!(hidden.kind, SnapKind::OnFace);
        assert!(hidden.point.distance(mid) < 1.5, "the card hides the floor");
        let at = |q: DVec3| {
            let scenery = Scenery::single(&m).within(Slab::around(&c, q, R));
            infer_detail(&scenery, &c, px(&c, q), &Guides::default(), R)
        };
        let (hit, involved) = at(p);
        assert_eq!((hit.kind, hit.face), (SnapKind::OnFace, Some(floor)));
        assert!(hit.point.distance(p) < 1e-9);
        assert_eq!(involved, vec![(0, EntityId::Face(floor))]);
        // A corner of the floor behind the card snaps as an endpoint.
        let corner = DVec3::new(2.0, 2.0, 0.0);
        assert_eq!(at(corner).0.kind, SnapKind::Endpoint);
        // A point in the air between the card and the floor is free space, exactly there.
        let air = c.eye.lerp(p, 0.75);
        let (hit, _) = at(air);
        assert_eq!((hit.kind, hit.point), (SnapKind::Free, air));
        // A card's back, just behind its front, is found too, not the front.
        let back = mid + (p - c.eye).normalize() * 0.01;
        let mut thin = Model::new();
        for d in [0.0, 0.01] {
            let o = mid + (p - c.eye).normalize() * d;
            thin.add_polygon(&[o - r - u, o + r - u, o + r + u, o - r + u])
                .unwrap();
        }
        let behind_face = thin
            .faces()
            .find(|(_, f)| {
                let at = thin.position(f.outer[0]).unwrap();
                (back - at).dot(f.normal).abs() < 1e-9
            })
            .unwrap()
            .0;
        let scenery = Scenery::single(&thin).within(Slab::around(&c, back, R));
        let hit = infer_in(&scenery, &c, px(&c, back), &Guides::default(), R);
        assert_eq!((hit.kind, hit.face), (SnapKind::OnFace, Some(behind_face)));
        // On the ground off the floor: the ground there.
        let ground = DVec3::new(-3.0, 1.0, 0.0);
        assert_eq!(at(ground).0.kind, SnapKind::Ground);
    }

    #[test]
    fn detailed_inference_names_what_was_snapped_to() {
        let (c, mut m) = (cam(), Model::new());
        square(&mut m, 0.0);
        m.add_guide(Guide::Line {
            point: DVec3::new(-5.0, 0.5, 0.0),
            direction: DVec3::X,
            tag: None,
            hidden: false,
        })
        .unwrap();
        let v = Scenery::single(&m);
        let none = Guides::default();
        let snap = |p: DVec3| infer_detail(&v, &c, px(&c, p), &none, R);
        let vertex = |p: DVec3| m.vertices().find(|(_, x)| x.pos == p).unwrap().0;
        let edge = |a: DVec3, b: DVec3| {
            m.edges()
                .find(|(_, e)| {
                    let ends = e.v.map(|id| m.vertex(id).unwrap().pos);
                    ends == [a, b] || ends == [b, a]
                })
                .unwrap()
                .0
        };
        let (o, x) = (DVec3::ZERO, DVec3::new(2.0, 0.0, 0.0));
        let (hit, involved) = snap(x);
        assert_eq!(hit.kind, SnapKind::Endpoint);
        assert_eq!(involved, vec![(0, EntityId::Vertex(vertex(x)))]);
        let (hit, involved) = snap(DVec3::new(1.0, 0.0, 0.0));
        assert_eq!(hit.kind, SnapKind::Midpoint);
        assert_eq!(involved, vec![(0, EntityId::Edge(edge(o, x)))]);
        let (hit, involved) = snap(DVec3::new(0.4, 0.0, 0.0));
        assert_eq!(hit.kind, SnapKind::OnEdge);
        assert_eq!(involved, vec![(0, EntityId::Edge(edge(o, x)))]);
        let guide = m.guides().next().unwrap().0;
        // The edge and the guide line cross in space here, so either may come first.
        let (hit, mut involved) = snap(DVec3::new(2.0, 0.5, 0.0));
        assert_eq!(hit.kind, SnapKind::Intersection);
        involved.sort();
        let side = edge(x, DVec3::new(2.0, 2.0, 0.0));
        assert_eq!(
            involved,
            vec![(0, EntityId::Edge(side)), (0, EntityId::Guide(guide))]
        );
        let (hit, involved) = snap(DVec3::new(-3.0, 0.5, 0.0));
        assert_eq!(hit.kind, SnapKind::OnGuide);
        assert_eq!(involved, vec![(0, EntityId::Guide(guide))]);
        let face = m.faces().next().unwrap().0;
        let (hit, involved) = snap(DVec3::new(1.3, 1.4, 0.0));
        assert_eq!(hit.kind, SnapKind::OnFace);
        assert_eq!(involved, vec![(0, EntityId::Face(face))]);
        let (hit, involved) = snap(DVec3::new(9.0, 9.0, 0.0));
        assert_eq!((hit.kind, involved), (SnapKind::Ground, vec![]));
    }

    #[test]
    fn snaps_to_a_nearby_endpoint_exactly() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(1.0, 1.0, 0.0), DVec3::new(3.0, 1.0, 0.0))
            .unwrap();
        let hit = infer(
            &m,
            &c,
            px(&c, DVec3::new(3.0, 1.0, 0.0)) + DVec2::new(4.0, -3.0),
            None,
            R,
        );
        assert_eq!(
            hit,
            Inference {
                point: DVec3::new(3.0, 1.0, 0.0),
                kind: SnapKind::Endpoint,
                face: None,
                locked: None
            }
        );
    }

    #[test]
    fn snaps_to_midpoints_and_the_origin() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(1.0, 1.0, 0.0), DVec3::new(3.0, 1.0, 0.0))
            .unwrap();
        let mid = infer(&m, &c, px(&c, DVec3::new(2.0, 1.0, 0.0)), None, R);
        assert_eq!(mid.kind, SnapKind::Midpoint);
        assert!(mid.point.abs_diff_eq(DVec3::new(2.0, 1.0, 0.0), 1e-12));
        let origin = infer(&m, &c, px(&c, DVec3::ZERO) + DVec2::new(2.0, 2.0), None, R);
        assert_eq!(origin.kind, SnapKind::Origin);
    }

    #[test]
    fn snaps_onto_an_edge_between_its_points() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(1.0, 1.0, 0.0), DVec3::new(5.0, 1.0, 0.0))
            .unwrap();
        let hit = infer(
            &m,
            &c,
            px(&c, DVec3::new(4.2, 1.0, 0.0)) + DVec2::new(0.0, 3.0),
            None,
            R,
        );
        assert_eq!(hit.kind, SnapKind::OnEdge);
        assert!((hit.point.y - 1.0).abs() < 1e-9 && hit.point.z.abs() < 1e-9);
        assert!((hit.point.x - 4.2).abs() < 0.1);
    }

    #[test]
    fn snaps_where_edges_cross_on_screen_to_the_nearer_edge() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(0.0, 1.0, 0.0), DVec3::new(4.0, 1.0, 0.0))
            .unwrap();
        // A second edge hovering between the eye and the first, across it, crossing it on
        // screen away from both midpoints.
        let behind = DVec3::new(2.7, 1.0, 0.0);
        let front = behind + (c.eye - behind) * 0.3;
        let across = DVec3::Z.cross(c.eye - behind).normalize() + DVec3::Z;
        m.add_edge(front - across * 0.5, front + across * 1.5)
            .unwrap();
        let hit = infer(&m, &c, px(&c, behind) + DVec2::new(1.0, 2.0), None, R);
        assert_eq!(hit.kind, SnapKind::Intersection);
        assert!(hit.point.distance(front) < 1e-6, "{:?}", hit.point);
        // Next to an end, the endpoint wins.
        let end = infer(&m, &c, px(&c, DVec3::new(4.0, 1.0, 0.0)), None, R);
        assert_eq!(end.kind, SnapKind::Endpoint);
    }

    #[test]
    fn snaps_find_their_edge_among_many() {
        let (c, mut m) = (cam(), Model::new());
        for i in -10..10 {
            for j in -10..10 {
                let a = DVec3::new(i as f64, j as f64, 0.0);
                m.add_edge(a, a + DVec3::new(0.4, 0.0, 0.0)).unwrap();
            }
        }
        let end = DVec3::new(2.4, 1.0, 0.0);
        let hit = infer(&m, &c, px(&c, end) + DVec2::new(2.0, -1.0), None, R);
        assert_eq!((hit.point, hit.kind), (end, SnapKind::Endpoint));
        let mid = infer(&m, &c, px(&c, DVec3::new(2.2, 1.0, 0.0)), None, R);
        assert_eq!(mid.kind, SnapKind::Midpoint);
        let e = edge_under(&m, &c, px(&c, DVec3::new(2.1, 1.0, 0.0)), R).unwrap();
        let ends = m.edge(e).unwrap().v.map(|v| m.position(v).unwrap());
        assert_eq!(ends, [DVec3::new(2.0, 1.0, 0.0), end]);
    }

    #[test]
    fn follows_an_axis_from_the_anchor() {
        let c = cam();
        let anchor = DVec3::new(1.0, 1.0, 0.0);
        let target = DVec3::new(1.0, 1.0, 2.5); // straight up (blue)
        let hit = infer(
            &Model::new(),
            &c,
            px(&c, target) + DVec2::new(5.0, 0.0),
            Some(anchor),
            R,
        );
        assert_eq!(hit.kind, SnapKind::Axis(Axis::Z));
        assert!(hit.point.x == 1.0 && hit.point.y == 1.0);
        assert!((hit.point.z - 2.5).abs() < 0.2);
    }

    #[test]
    fn points_beat_axes() {
        let (c, mut m) = (cam(), Model::new());
        let anchor = DVec3::new(0.0, 3.0, 0.0);
        m.add_edge(DVec3::new(2.0, 3.0, 0.0), DVec3::new(2.0, 5.0, 0.0))
            .unwrap();
        let hit = infer(&m, &c, px(&c, DVec3::new(2.0, 3.0, 0.0)), Some(anchor), R);
        assert_eq!(hit.kind, SnapKind::Endpoint);
    }

    #[test]
    fn lands_on_faces_then_ground_then_free() {
        let (c, mut m) = (cam(), Model::new());
        square(&mut m, 1.0);
        let on_face = infer(&m, &c, px(&c, DVec3::new(1.3, 0.7, 1.0)), None, R);
        assert_eq!(on_face.kind, SnapKind::OnFace);
        assert!((on_face.point.z - 1.0).abs() < 1e-9);

        let ground = infer(
            &Model::new(),
            &c,
            px(&c, DVec3::new(-3.0, 4.0, 0.0)),
            None,
            R,
        );
        assert_eq!(ground.kind, SnapKind::Ground);
        assert!(ground.point.z.abs() < 1e-9);

        let mut level = cam();
        level.eye = DVec3::new(0.0, -10.0, 1.0);
        level.target = DVec3::new(0.0, 0.0, 1.0);
        let sky = infer(&Model::new(), &level, DVec2::new(400.0, 0.0), None, R);
        assert_eq!(sky.kind, SnapKind::Free);
    }

    /// A parallel view from the front (looking along +Y), zoomed in so the eye is half a
    /// meter in front of the target.
    fn parallel_front() -> Camera {
        let mut c = cam();
        c.projection = session::Projection::Parallel;
        c.target = DVec3::new(1.0, 0.0, 1.0);
        c.eye = DVec3::new(1.0, -0.5, 1.0);
        c
    }

    #[test]
    fn a_parallel_view_lands_on_the_ground() {
        let mut c = cam();
        c.projection = session::Projection::Parallel;
        c.look_from(session::StandardView::Top);
        let at = DVec3::new(-3.0, 4.0, 0.0);
        let hit = infer(&Model::new(), &c, px(&c, at), None, R);
        assert_eq!(hit.kind, SnapKind::Ground);
        assert!(hit.point.abs_diff_eq(at, 1e-9), "{}", hit.point);
    }

    #[test]
    fn a_parallel_view_sees_and_hides_with_what_is_behind_the_eye() {
        let c = parallel_front();
        let mut m = Model::new();
        // A wall 2 m in front of the target: behind the eye, but in view.
        let w = [(-1.0, -1.0), (3.0, -1.0), (3.0, 3.0), (-1.0, 3.0)];
        for i in 0..4 {
            let (a, b) = (w[i], w[(i + 1) % 4]);
            m.add_edge(DVec3::new(a.0, -2.0, a.1), DVec3::new(b.0, -2.0, b.1))
                .unwrap();
        }
        // An edge behind the target, hidden by the wall.
        let end = DVec3::new(0.5, 1.0, 0.5);
        m.add_edge(end, DVec3::new(1.5, 1.0, 0.5)).unwrap();
        let hit = infer(&m, &c, px(&c, end), None, R);
        assert_eq!(hit.kind, SnapKind::OnFace, "{hit:?}");
        assert!((hit.point.y + 2.0).abs() < 1e-9);
        // The wall's own corner, behind the eye, snaps (seen 6 m wide).
        let corner = DVec3::new(3.0, -2.0, 3.0);
        let mut wide = c.clone();
        wide.eye = DVec3::new(1.0, -1.5, 1.0);
        wide.target = DVec3::new(1.0, 4.5, 1.0);
        let hit = infer(&m, &wide, px(&wide, corner) + DVec2::new(3.0, 2.0), None, R);
        assert_eq!((hit.kind, hit.point), (SnapKind::Endpoint, corner));
    }

    #[test]
    fn parallel_snaps_reach_as_far_on_screen_as_perspective_ones() {
        let mut c = cam();
        c.projection = session::Projection::Parallel;
        c.look_from(session::StandardView::Top);
        // A millimeter edge: the index's box around it is far smaller than the snap radius.
        let mut m = Model::new();
        let end = DVec3::new(2.001, 1.0, 0.0);
        m.add_edge(DVec3::new(2.0, 1.0, 0.0), end).unwrap();
        let hit = infer(&m, &c, px(&c, end) + DVec2::new(6.0, -5.0), None, R);
        assert_eq!((hit.point, hit.kind), (end, SnapKind::Endpoint));
        let e = edge_under(&m, &c, px(&c, end) + DVec2::new(0.0, 6.0), R);
        assert!(e.is_some());
    }

    #[test]
    fn inner_face_inside_another_faces_hole_is_hit() {
        let (c, mut m) = (cam(), Model::new());
        let pts = [(-2.0, -2.0), (4.0, -2.0), (4.0, 4.0), (-2.0, 4.0)];
        for i in 0..4 {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            m.add_edge(DVec3::new(a.0, a.1, 1.0), DVec3::new(b.0, b.1, 1.0))
                .unwrap();
        }
        square(&mut m, 1.0); // inner square (0,0)-(2,2): a face and a hole of the outer face
        let inside_inner = infer(&m, &c, px(&c, DVec3::new(1.2, 0.8, 1.0)), None, R);
        assert_eq!(inside_inner.kind, SnapKind::OnFace);
        assert!((inside_inner.point.z - 1.0).abs() < 1e-9);
    }

    /// A 2 m box on the ground at the origin, built through push/pull.
    fn cube() -> Model {
        let mut m = Model::new();
        let pts = [
            DVec3::ZERO,
            DVec3::new(2.0, 0.0, 0.0),
            DVec3::new(2.0, 2.0, 0.0),
            DVec3::new(0.0, 2.0, 0.0),
        ];
        m.add_polygon(&pts).unwrap();
        let base = m.faces().next().unwrap().0;
        m.push_pull(base, 2.0).unwrap();
        m
    }

    #[test]
    fn hidden_corners_are_not_snapped() {
        let (c, m) = (cam(), cube());
        // The camera looks from (-8, -12, 7): the far bottom corner is behind the box.
        let hidden = DVec3::new(2.0, 2.0, 0.0);
        let hit = infer(&m, &c, px(&c, hidden), None, R);
        assert_ne!(hit.kind, SnapKind::Endpoint);
        assert_eq!(hit.kind, SnapKind::OnFace);
        assert!(hit.face.is_some());

        let near_top = DVec3::new(2.0, 2.0, 2.0);
        let hit = infer(&m, &c, px(&c, near_top), None, R);
        assert_eq!(hit.kind, SnapKind::Endpoint);
        assert_eq!(hit.point, near_top);
    }

    #[test]
    fn hidden_edges_make_no_intersections() {
        let (c, m) = (cam(), cube());
        // The hidden back edge (2,2,z) crosses the visible top-front edge on screen.
        let front = (DVec3::new(0.0, 0.0, 2.0), DVec3::new(2.0, 0.0, 2.0));
        let back = (DVec3::new(2.0, 2.0, 0.0), DVec3::new(2.0, 2.0, 2.0));
        let (sa, sb) = (px(&c, front.0), px(&c, front.1));
        let (sc, sd) = (px(&c, back.0), px(&c, back.1));
        let (r, q) = (sb - sa, sd - sc);
        let t = (sc - sa).perp_dot(q) / r.perp_dot(q);
        assert!(t > 0.1 && t < 0.4, "{t}");
        let hit = infer(&m, &c, sa + r * t, None, R);
        assert_eq!(hit.kind, SnapKind::OnEdge, "{:?}", hit.point);
        assert!((hit.point.y).abs() < 1e-9 && (hit.point.z - 2.0).abs() < 1e-9);
    }

    #[test]
    fn reports_the_face_under_the_cursor() {
        let (c, m) = (cam(), cube());
        let top = m
            .faces()
            .find(|(_, f)| f.normal == DVec3::Z)
            .map(|(id, _)| id)
            .unwrap();
        let cursor = px(&c, DVec3::new(1.0, 1.2, 2.0));
        let hit = infer(&m, &c, cursor, None, R);
        assert_eq!((hit.kind, hit.face), (SnapKind::OnFace, Some(top)));
        let (face, point) = face_under(&m, &c, cursor).unwrap();
        assert_eq!(face, top);
        assert!((point.z - 2.0).abs() < 1e-9);
        assert!(face_under(&m, &c, px(&c, DVec3::new(-3.0, 4.0, 0.0))).is_none());
    }

    #[test]
    fn axis_guides_show_through_faces() {
        let (c, m) = (cam(), cube());
        let anchor = DVec3::new(3.0, 3.0, 0.0);
        let behind = DVec3::new(3.0, 3.0, 1.0);
        let hit = infer(&m, &c, px(&c, behind), Some(anchor), R);
        assert_eq!(hit.kind, SnapKind::Axis(Axis::Z));
    }

    #[test]
    fn edge_under_picks_the_nearest_visible_edge() {
        let (c, m) = (cam(), cube());
        let edge_at = |a: DVec3, b: DVec3| {
            m.edges()
                .find(|(_, e)| {
                    let (p, q) = (m.position(e.v[0]).unwrap(), m.position(e.v[1]).unwrap());
                    (p == a && q == b) || (p == b && q == a)
                })
                .unwrap()
                .0
        };
        let near_top = edge_at(DVec3::new(0.0, 0.0, 2.0), DVec3::new(2.0, 0.0, 2.0));
        let cursor = px(&c, DVec3::new(1.0, 0.0, 2.0)) + DVec2::new(0.0, 3.0);
        assert_eq!(edge_under(&m, &c, cursor, R), Some(near_top));
        // The far bottom edge lies behind the box.
        let far_bottom = edge_at(DVec3::new(0.0, 2.0, 0.0), DVec3::new(2.0, 2.0, 0.0));
        let behind = px(&c, DVec3::new(1.0, 2.0, 0.0));
        assert_ne!(edge_under(&m, &c, behind, R), Some(far_bottom));
        assert_eq!(
            edge_under(&m, &c, px(&c, DVec3::new(-3.0, 4.0, 0.0)), R),
            None
        );
    }

    fn guides(anchor: DVec3, reference: Option<DVec3>, lock: Option<Lock>) -> Guides {
        Guides {
            anchor: Some(anchor),
            reference,
            lock,
        }
    }

    #[test]
    fn follows_parallel_and_perpendicular_to_the_reference() {
        let c = cam();
        let anchor = DVec3::new(0.0, 3.0, 0.0);
        let r = DVec3::new(2.0, 1.0, 0.0).normalize();
        let g = guides(anchor, Some(r), None);
        let along = anchor + r * 2.0;
        let hit = infer_with(&Model::new(), &c, px(&c, along), &g, R);
        assert_eq!(hit.kind, SnapKind::Parallel);
        assert!(hit.point.abs_diff_eq(along, 1e-6));
        let across = anchor + DVec3::new(1.0, -2.0, 0.0).normalize() * 2.0;
        let hit = infer_with(&Model::new(), &c, px(&c, across), &g, R);
        assert_eq!(hit.kind, SnapKind::Perpendicular);
        assert!(hit.point.abs_diff_eq(across, 1e-6));
    }

    /// A scene whose axes start at (2, 3, 0) with red along world y and green along world -x.
    fn turned_scene() -> kernel::Scene {
        let mut s = kernel::Scene::new();
        let axes = kernel::Axes::new(DVec3::new(2.0, 3.0, 0.0), DVec3::Y, -DVec3::X).unwrap();
        s.set_axes(axes);
        s
    }

    #[test]
    fn the_origin_snap_is_at_the_axes_origin() {
        let (c, s) = (cam(), turned_scene());
        let scenery = Scenery::of(&s, &[]).unwrap();
        let near = DVec2::new(2.0, 2.0);
        let hit = infer_in(
            &scenery,
            &c,
            px(&c, DVec3::new(2.0, 3.0, 0.0)) + near,
            &Guides::default(),
            R,
        );
        assert_eq!(hit.kind, SnapKind::Origin);
        assert!(hit.point.abs_diff_eq(DVec3::new(2.0, 3.0, 0.0), 1e-12));
        let world = infer_in(
            &scenery,
            &c,
            px(&c, DVec3::ZERO) + near,
            &Guides::default(),
            R,
        );
        assert_ne!(world.kind, SnapKind::Origin, "the world origin is no snap");
    }

    #[test]
    fn axis_inference_follows_the_drawing_axes() {
        let (c, s) = (cam(), turned_scene());
        let scenery = Scenery::of(&s, &[]).unwrap();
        let anchor = DVec3::new(1.0, 1.0, 0.0);
        let g = guides(anchor, None, None);
        // Red runs along world y.
        let along_red = anchor + DVec3::Y * 3.0;
        let hit = infer_in(&scenery, &c, px(&c, along_red), &g, R);
        assert_eq!(hit.kind, SnapKind::Axis(Axis::X));
        assert!(hit.point.abs_diff_eq(along_red, 1e-6));
        // Green along world x.
        let along_green = anchor + DVec3::X * 3.0;
        let hit = infer_in(&scenery, &c, px(&c, along_green), &g, R);
        assert_eq!(hit.kind, SnapKind::Axis(Axis::Y));
        assert!(hit.point.abs_diff_eq(along_green, 1e-6));
    }

    #[test]
    fn horizontal_perpendiculars_are_horizontal_in_the_axes() {
        // Blue along world y (red along z, green along x).
        let axes = DMat3::from_cols(DVec3::Z, DVec3::X, DVec3::Y);
        let r = DVec3::new(1.0, 0.0, 1.0).normalize();
        let out = directions(axes, Some(r));
        assert_eq!(out[0], (DVec3::Z, SnapKind::Axis(Axis::X)));
        assert_eq!(out[2], (DVec3::Y, SnapKind::Axis(Axis::Z)));
        let across: Vec<DVec3> = out
            .iter()
            .filter(|(_, k)| *k == SnapKind::Perpendicular)
            .map(|(d, _)| *d)
            .collect();
        assert!(
            across.iter().any(|d| d.dot(axes.z_axis).abs() < 1e-9),
            "one perpendicular is flat in the axes: {across:?}"
        );
    }

    #[test]
    fn a_reference_along_an_axis_adds_nothing() {
        assert_eq!(
            directions(DMat3::IDENTITY, Some(DVec3::new(0.0, 3.0, 0.0))).len(),
            3
        );
        let tilted = directions(DMat3::IDENTITY, Some(DVec3::new(1.0, 1.0, 0.0)));
        // Parallel and the horizontal perpendicular; the other perpendicular is vertical (Z).
        assert_eq!(tilted.len(), 5);
        assert_eq!(directions(DMat3::IDENTITY, None).len(), 3);
    }

    #[test]
    fn a_lock_holds_the_point_on_its_line() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(3.0, 3.0, 2.0), DVec3::new(4.0, 3.0, 2.0))
            .unwrap();
        let anchor = DVec3::new(1.0, 1.0, 0.0);
        let lock = Lock {
            dir: DVec3::Z,
            kind: SnapKind::Axis(Axis::Z),
        };
        let g = guides(anchor, None, Some(lock));
        let far = infer_with(&m, &c, px(&c, DVec3::new(-4.0, 5.0, 0.0)), &g, R);
        assert_eq!(far.kind, SnapKind::Axis(Axis::Z));
        assert!(far.point.x == 1.0 && far.point.y == 1.0);
        // An endpoint under the cursor sets the height.
        let snapped = infer_with(&m, &c, px(&c, DVec3::new(3.0, 3.0, 2.0)), &g, R);
        assert_eq!(snapped.point, DVec3::new(1.0, 1.0, 2.0));
    }

    fn with_guides() -> Model {
        let mut m = Model::new();
        m.add_guide(Guide::Point {
            position: DVec3::new(2.0, 1.0, 0.0),
            tag: None,
            hidden: false,
        })
        .unwrap();
        m.add_guide(Guide::Line {
            point: DVec3::new(0.0, 3.0, 0.0),
            direction: DVec3::X,
            tag: None,
            hidden: false,
        })
        .unwrap();
        m.add_guide(Guide::Line {
            point: DVec3::new(5.0, 0.0, 0.0),
            direction: DVec3::Y,
            tag: None,
            hidden: false,
        })
        .unwrap();
        m
    }

    #[test]
    fn snaps_to_guide_points_and_onto_guide_lines() {
        let (c, m) = (cam(), with_guides());
        let point = DVec3::new(2.0, 1.0, 0.0);
        let hit = infer(&m, &c, px(&c, point) + DVec2::new(3.0, -2.0), None, R);
        assert_eq!((hit.point, hit.kind), (point, SnapKind::GuidePoint));
        let hit = infer(
            &m,
            &c,
            px(&c, DVec3::new(2.6, 3.0, 0.0)) + DVec2::new(0.0, 3.0),
            None,
            R,
        );
        assert_eq!(hit.kind, SnapKind::OnGuide);
        assert!((hit.point.y - 3.0).abs() < 1e-9 && hit.point.z.abs() < 1e-9);
        assert!((hit.point.x - 2.6).abs() < 0.1);
    }

    #[test]
    fn guide_lines_cross_each_other_and_edges() {
        let (c, mut m) = (cam(), with_guides());
        let both = DVec3::new(5.0, 3.0, 0.0);
        let hit = infer(&m, &c, px(&c, both) + DVec2::new(2.0, 1.0), None, R);
        assert_eq!(hit.kind, SnapKind::Intersection);
        assert!(hit.point.distance(both) < 1e-6, "{:?}", hit.point);
        m.add_edge(DVec3::new(1.0, -3.0, 0.0), DVec3::new(1.0, 7.0, 0.0))
            .unwrap();
        let cross = DVec3::new(1.0, 3.0, 0.0);
        let hit = infer(&m, &c, px(&c, cross) + DVec2::new(-1.0, 2.0), None, R);
        assert_eq!(hit.kind, SnapKind::Intersection);
        assert!(hit.point.distance(cross) < 1e-6, "{:?}", hit.point);
        // Along the edge away from the guide, the edge wins over nothing.
        let hit = infer(&m, &c, px(&c, DVec3::new(1.0, 0.5, 0.0)), None, R);
        assert_eq!(hit.kind, SnapKind::OnEdge);
    }

    #[test]
    fn finds_the_guide_under_the_cursor() {
        let (c, m) = (cam(), with_guides());
        let scenery = Scenery::single(&m);
        let ids: Vec<GuideId> = m.guides().map(|(id, _)| id).collect();
        let under = |p: DVec3| {
            guide_under_where(&scenery, &c, px(&c, p) + DVec2::new(2.0, 2.0), R, |_| true)
                .map(|(_, id, _)| id)
        };
        assert_eq!(under(DVec3::new(2.0, 1.0, 0.0)), Some(ids[0]));
        assert_eq!(under(DVec3::new(-2.0, 3.0, 0.0)), Some(ids[1]));
        assert_eq!(under(DVec3::new(5.0, -4.0, 0.0)), Some(ids[2]));
        assert_eq!(under(DVec3::new(-3.0, -3.0, 0.0)), None);
        // Guides behind a face are not found.
        let mut hidden = with_guides();
        let lid = [(-9.0, -9.0), (9.0, -9.0), (9.0, 9.0), (-9.0, 9.0)]
            .map(|(x, y)| DVec3::new(x, y, 1.0));
        hidden.add_polygon(&lid).unwrap();
        let scenery = Scenery::single(&hidden);
        let p = px(&c, DVec3::new(2.0, 1.0, 0.0));
        assert!(guide_under_where(&scenery, &c, p, R, |_| true).is_none());
    }

    /// A scene whose only instance, a group holding a unit square at the origin, also holds a
    /// guide point at (3, 1, 0).
    fn scene_with_a_guide_in_a_group() -> kernel::Scene {
        let mut m = Model::new();
        let p = |x, y| DVec3::new(x, y, 0.0);
        m.add_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)])
            .unwrap();
        let mut s = kernel::Scene::from_root(m);
        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
        let (i, _) = s
            .make_group(
                kernel::MeshId::Root,
                &edges,
                &faces,
                &[],
                kernel::DefinitionKind::Group,
                "",
            )
            .unwrap();
        let def = s.root().instance(i).unwrap().definition;
        s.edit(kernel::MeshId::Definition(def), |m| {
            m.add_guide(Guide::Point {
                position: p(3.0, 1.0),
                tag: None,
                hidden: false,
            })
        })
        .unwrap();
        s
    }

    #[test]
    fn a_group_flush_on_a_face_is_the_face_under_the_cursor() {
        // The unit square is in a group; the root gets the same square at the same place.
        let (c, mut s) = (cam(), scene_with_a_guide_in_a_group());
        let p = |x, y| DVec3::new(x, y, 0.0);
        s.edit(kernel::MeshId::Root, |m| {
            m.add_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)])
        })
        .unwrap();
        let scenery = Scenery::of(&s, &[]).unwrap();
        let (owner, ..) = face_under_in(&scenery, &c, px(&c, p(0.5, 0.5))).unwrap();
        assert!(matches!(owner, Owner::Instance(_)), "{owner:?}");
    }

    #[test]
    fn a_guide_inside_a_closed_group_is_snapped_to() {
        let (c, s) = (cam(), scene_with_a_guide_in_a_group());
        let scenery = Scenery::of(&s, &[]).unwrap();
        let g = Guides::default();
        let at = DVec3::new(3.0, 1.0, 0.0);
        let hit = infer_in(&scenery, &c, px(&c, at) + DVec2::new(2.0, 2.0), &g, R);
        assert_eq!((hit.point, hit.kind), (at, SnapKind::GuidePoint));
    }

    /// A deterministic stream of numbers in [0, 1).
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }

        fn within(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * self.next()
        }
    }

    /// Adds a box `size` wide with a corner at `at` to a mesh of `s`.
    fn add_box(s: &mut kernel::Scene, mesh: kernel::MeshId, at: DVec3, size: f64) {
        s.edit(mesh, |m| {
            let p = |x: f64, y: f64| at + DVec3::new(x, y, 0.0) * size;
            let mut c = m.add_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)])?;
            let f = m.faces().map(|(id, _)| id).max().unwrap();
            c.ops.extend(m.push_pull(f, size)?.ops);
            Ok(c)
        })
        .unwrap();
    }

    /// A crowd of placed boxes: a component (a box, a nested group with a smaller box, guides)
    /// copied 28 times turned, moved, mirrored and scaled, some of the copies gathered in a
    /// group, over a ground face, with crossing loose edges and a guide line. Returns the
    /// scene and the open contexts to look from: the root, the group, the nested group in a
    /// component in the group, and a component at the root.
    fn crowd() -> (kernel::Scene, Vec<Vec<kernel::InstanceId>>) {
        use kernel::{DefinitionKind, MeshId};
        let mut s = kernel::Scene::new();
        add_box(&mut s, MeshId::Root, DVec3::ZERO, 0.5);
        let all = |s: &kernel::Scene, mesh: MeshId| {
            let m = s.mesh(mesh).unwrap();
            let edges: Vec<EdgeId> = m.edges().map(|(id, _)| id).collect();
            let faces: Vec<FaceId> = m.faces().map(|(id, _)| id).collect();
            (edges, faces)
        };
        let (edges, faces) = all(&s, MeshId::Root);
        let (comp, _) = s
            .make_group(
                MeshId::Root,
                &edges,
                &faces,
                &[],
                DefinitionKind::Component,
                "C",
            )
            .unwrap();
        let def = MeshId::Definition(s.root().instance(comp).unwrap().definition);
        let (old_edges, old_faces) = all(&s, def);
        add_box(&mut s, def, DVec3::new(0.1, 0.15, 0.5), 0.2);
        let (edges, faces) = all(&s, def);
        let edges: Vec<EdgeId> = edges
            .into_iter()
            .filter(|e| !old_edges.contains(e))
            .collect();
        let faces: Vec<FaceId> = faces
            .into_iter()
            .filter(|f| !old_faces.contains(f))
            .collect();
        let (nested, _) = s
            .make_group(def, &edges, &faces, &[], DefinitionKind::Group, "")
            .unwrap();
        s.edit(def, |m| {
            let mut c = m.add_guide(Guide::Point {
                position: DVec3::new(0.25, 0.25, 0.9),
                tag: None,
                hidden: false,
            })?;
            c.ops.extend(
                m.add_guide(Guide::Line {
                    point: DVec3::new(0.0, 0.5, 0.25),
                    direction: DVec3::X,
                    tag: None,
                    hidden: false,
                })?
                .ops,
            );
            Ok(c)
        })
        .unwrap();

        let mut rng = Lcg(7);
        let places: Vec<glam::DAffine3> = (0..28)
            .map(|k| {
                let axis = DVec3::new(rng.within(-1.0, 1.0), rng.within(-1.0, 1.0), rng.next())
                    .normalize();
                let turn = glam::DQuat::from_axis_angle(axis, rng.within(-3.0, 3.0));
                let scale = match k % 7 {
                    0 => DVec3::new(-1.0, 1.0, 1.0),
                    3 => DVec3::splat(rng.within(0.5, 2.0)),
                    5 => DVec3::new(1.0, 1.5, 0.7),
                    _ => DVec3::ONE,
                };
                let at = DVec3::new(
                    rng.within(-5.0, 5.0),
                    rng.within(-5.0, 5.0),
                    rng.within(0.0, 2.0),
                );
                glam::DAffine3::from_scale_rotation_translation(scale, turn, at)
            })
            .collect();
        s.copy_instances_placed(MeshId::Root, &[comp], &places)
            .unwrap();
        let copies: Vec<kernel::InstanceId> = s
            .root()
            .instances()
            .map(|(id, _)| id)
            .filter(|id| *id != comp)
            .collect();
        let (group, _) = s
            .make_group(
                MeshId::Root,
                &[],
                &[],
                &copies[..8],
                DefinitionKind::Group,
                "",
            )
            .unwrap();
        s.edit(MeshId::Root, |m| {
            let mut c = m.add_polygon(&[
                DVec3::new(-6.0, -6.0, 0.0),
                DVec3::new(6.0, -6.0, 0.0),
                DVec3::new(6.0, 6.0, 0.0),
                DVec3::new(-6.0, 6.0, 0.0),
            ])?;
            for k in 0..6 {
                let y = -3.0 + k as f64;
                c.ops.extend(
                    m.add_edge(DVec3::new(-4.0, y, 1.0), DVec3::new(4.0, y + 1.5, 2.5))?
                        .ops,
                );
                c.ops.extend(
                    m.add_edge(DVec3::new(y, -4.0, 1.5), DVec3::new(y + 0.5, 4.0, 0.5))?
                        .ops,
                );
            }
            c.ops.extend(
                m.add_guide(Guide::Line {
                    point: DVec3::new(0.0, 0.0, 0.3),
                    direction: DVec3::new(1.0, 1.0, 0.0).normalize(),
                    tag: None,
                    hidden: false,
                })?
                .ops,
            );
            Ok(c)
        })
        .unwrap();
        let group_mesh = MeshId::Definition(s.root().instance(group).unwrap().definition);
        let inner = s.mesh(group_mesh).unwrap().instances().next().unwrap().0;
        let contexts = vec![
            vec![],
            vec![group],
            vec![group, inner, nested],
            vec![copies[20]],
        ];
        (s, contexts)
    }

    /// Cameras in root coordinates: perspective and parallel, from around and above.
    fn views() -> Vec<Camera> {
        let mut out = Vec::new();
        for (eye, target) in [
            (DVec3::new(-8.0, -12.0, 7.0), DVec3::ZERO),
            (DVec3::new(0.5, -2.0, 9.0), DVec3::new(0.5, 0.0, 0.0)),
            (DVec3::new(-1.0, 0.2, 1.2), DVec3::new(3.0, 1.0, 1.0)),
        ] {
            for projection in [
                session::Projection::Perspective,
                session::Projection::Parallel,
            ] {
                let mut c = Camera::new(DVec2::new(800.0, 600.0));
                c.eye = eye;
                c.target = target;
                c.projection = projection;
                out.push(c);
            }
        }
        out
    }

    /// `c` through a `fov_deg` degree field of view in a wide window, 1600 by 400 pixels.
    /// Toward the left and right ends of such a view the rays fan out much faster up and down
    /// than across, so the ray beside the cursor (across) spreads least from the cursor's.
    fn wide(mut c: Camera, fov_deg: f64) -> Camera {
        c.fov_y = fov_deg.to_radians();
        c.viewport = DVec2::new(1600.0, 400.0);
        c
    }

    /// The perspective `views` through 120 and 160 degree fields of view in a wide window,
    /// their eyes brought in to a quarter of the way to their targets so that the crowd still
    /// fills the view (a parallel view's field of view only sets how far it is zoomed out).
    fn wide_views() -> Vec<Camera> {
        views()
            .into_iter()
            .filter(|c| !c.is_parallel())
            .flat_map(|mut c| {
                c.eye = c.target.lerp(c.eye, 0.25);
                [wide(c.clone(), 120.0), wide(c, 160.0)]
            })
            .collect()
    }

    /// Cursors 12 pixels in from each corner of `c`'s view.
    fn near_corners(c: &Camera) -> [DVec2; 4] {
        let (w, h) = (c.viewport.x, c.viewport.y);
        [
            DVec2::new(12.0, 12.0),
            DVec2::new(w - 12.0, 12.0),
            DVec2::new(w - 12.0, h - 12.0),
            DVec2::new(12.0, h - 12.0),
        ]
    }

    /// The unit step on screen from `cursor` along which `c`'s rays lean fastest from the
    /// cursor's, out of a turn's worth in degree steps.
    fn steepest(c: &Camera, cursor: DVec2) -> DVec2 {
        let ray = c.ray(cursor);
        let lean = |u: DVec2| c.ray(cursor + u * R).dir.angle_between(ray.dir);
        (0..360)
            .map(|k| DVec2::from_angle((k as f64).to_radians()))
            .max_by(|a, b| lean(*a).total_cmp(&lean(*b)))
            .unwrap()
    }

    /// The ends of an edge seen from `c` (perspective) running from 0.9 radii off `cursor` to
    /// 1.2 radii off it, along `steepest`, 4 m from the eye: its near end lies within the
    /// radius of the cursor on screen, its middle and far end do not.
    fn edge_by(c: &Camera, cursor: DVec2) -> (DVec3, DVec3) {
        let u = steepest(c, cursor);
        let at = |px: f64| {
            let ray = c.ray(cursor + u * px);
            ray.origin + ray.dir * 4.0
        };
        (at(0.9 * R), at(1.2 * R))
    }

    /// Where `holding_edge` draws its edge.
    #[derive(Clone, Copy, Debug)]
    enum Holder {
        /// In the root, the open context.
        Root,
        /// In a component placed scaled up 3 times, turned and moved.
        Scaled,
        /// In a group turned and moved in a component turned and moved.
        Nested,
    }

    const HOLDERS: [Holder; 3] = [Holder::Root, Holder::Scaled, Holder::Nested];

    /// A scene holding only the edge from `p` to `q` (in root coordinates), drawn as
    /// `holder` says, and whose the edge is seen from the root.
    fn holding_edge(p: DVec3, q: DVec3, holder: Holder) -> (kernel::Scene, Owner) {
        use glam::{DAffine3, DQuat};
        use kernel::{DefinitionKind, MeshId};
        let turn = |axis: DVec3, angle: f64| DQuat::from_axis_angle(axis.normalize(), angle);
        let (outer, inner) = match holder {
            Holder::Root => (DAffine3::IDENTITY, DAffine3::IDENTITY),
            Holder::Scaled => (
                DAffine3::from_scale_rotation_translation(
                    DVec3::splat(3.0),
                    turn(DVec3::new(1.0, 2.0, 3.0), 0.7),
                    DVec3::new(0.5, -1.0, 0.3),
                ),
                DAffine3::IDENTITY,
            ),
            Holder::Nested => (
                DAffine3::from_rotation_translation(
                    turn(DVec3::new(1.0, 0.0, 0.5), -0.6),
                    DVec3::new(-2.0, 1.0, 0.5),
                ),
                DAffine3::from_rotation_translation(
                    turn(DVec3::new(0.0, 1.0, 1.0), 1.1),
                    DVec3::new(0.2, 0.1, 0.0),
                ),
            ),
        };
        let back = (outer * inner).inverse();
        let mut s = kernel::Scene::new();
        s.edit(MeshId::Root, |m| {
            m.add_edge(back.transform_point3(p), back.transform_point3(q))
        })
        .unwrap();
        let edges: Vec<EdgeId> = s.root().edges().map(|(id, _)| id).collect();
        let kind = match holder {
            Holder::Root => return (s, Owner::Open),
            Holder::Scaled => DefinitionKind::Component,
            Holder::Nested => DefinitionKind::Group,
        };
        let (first, _) = s
            .make_group(MeshId::Root, &edges, &[], &[], kind, "")
            .unwrap();
        let top = match holder {
            Holder::Nested => {
                let (top, _) = s
                    .make_group(
                        MeshId::Root,
                        &[],
                        &[],
                        &[first],
                        DefinitionKind::Component,
                        "N",
                    )
                    .unwrap();
                let def = MeshId::Definition(s.root().instance(top).unwrap().definition);
                s.edit(def, |m| Ok(m.transform_instances(&[first], inner)))
                    .unwrap();
                top
            }
            _ => first,
        };
        s.edit(MeshId::Root, |m| Ok(m.transform_instances(&[top], outer)))
            .unwrap();
        (s, Owner::Instance(top))
    }

    #[test]
    fn wide_views_find_points_and_edges_within_the_radius_near_corners() {
        let g = Guides::default();
        for fov in [120.0, 160.0] {
            let c = wide(cam(), fov);
            for cursor in near_corners(&c) {
                let (p, q) = edge_by(&c, cursor);
                // Both ends lie outside the cone searched before, sized from the ray beside the
                // cursor (across).
                let (r0, slope) = old::beside_cone(&c, cursor, R);
                let ray = c.ray(cursor);
                for end in [p, q] {
                    let t = (end - ray.origin).dot(ray.dir);
                    let off = (end - ray.origin - ray.dir * t).length();
                    assert!(off > r0 + slope * t, "{fov} {cursor}");
                }
                for holder in HOLDERS {
                    let at = format!("{fov} degrees, {cursor}, {holder:?}");
                    let (s, owner) = holding_edge(p, q, holder);
                    let scenery = Scenery::of(&s, &[]).unwrap();
                    let hit = infer_in(&scenery, &c, cursor, &g, R);
                    assert_eq!(hit.kind, SnapKind::Endpoint, "{at}");
                    assert!(hit.point.distance(p) < 1e-9, "{at}: {}", hit.point);
                    let (o, _, a, b) = edge_under_in(&scenery, &c, cursor, R).expect(&at);
                    assert_eq!(o, owner, "{at}");
                    assert!(a.distance(p).max(b.distance(q)) < 1e-9, "{at}");
                    // The code before found neither.
                    let before = old::Scenery::of(&s, &[]).unwrap();
                    let then = old::infer_in(&before, &c, cursor, &g, R, old::Gather::Cone);
                    assert_ne!(then.kind, SnapKind::Endpoint, "{at}");
                    let then = old::edge_under_in(&before, &c, cursor, R, old::Gather::Cone);
                    assert_eq!(then, None, "{at}");
                }
            }
        }
    }

    /// True if `x` lies in `cone`, but for rounding.
    fn in_cone(cone: &Cone, x: DVec3) -> bool {
        let rel = x - cone.origin;
        let t = rel.dot(cone.dir);
        let off = (rel - cone.dir * t).length();
        off <= (cone.r0 + cone.slope * t.max(0.0)) * (1.0 + 1e-9) + 1e-12 * rel.length()
    }

    #[test]
    fn the_cone_holds_every_ray_through_the_pick_square_in_every_part() {
        use glam::{DAffine3, DMat3, DQuat};
        let mut rng = Lcg(5);
        let mut cones = 0;
        for n in 0..300 {
            let mut c = Camera::new(DVec2::new(
                rng.within(100.0, 2400.0),
                rng.within(100.0, 1400.0),
            ));
            c.fov_y = rng.within(1.0, 170.0).to_radians();
            c.eye = DVec3::new(
                rng.within(-20.0, 20.0),
                rng.within(-20.0, 20.0),
                rng.within(-20.0, 20.0),
            );
            c.target = DVec3::new(
                rng.within(-5.0, 5.0),
                rng.within(-5.0, 5.0),
                rng.within(-5.0, 5.0),
            );
            if n % 3 == 0 {
                c.projection = session::Projection::Parallel;
            }
            let radius = rng.within(1.0, 30.0);
            let cursor = DVec2::new(
                rng.within(-20.0, c.viewport.x + 20.0),
                rng.within(-20.0, c.viewport.y + 20.0),
            );
            let cone = Cone::around(&c, cursor, radius);
            cones += usize::from(cone.r0.is_finite());
            // A placement turned, mirrored, stretched unevenly, sheared and moved.
            let axis = DVec3::new(rng.within(-1.0, 1.0), rng.within(-1.0, 1.0), 1.0);
            let turn = DMat3::from_quat(DQuat::from_axis_angle(
                axis.normalize(),
                rng.within(-3.0, 3.0),
            ));
            let stretch = DMat3::from_diagonal(DVec3::new(
                -rng.within(0.2, 4.0),
                rng.within(0.2, 4.0),
                rng.within(0.2, 4.0),
            ));
            let shear = DMat3::from_cols(
                DVec3::X,
                DVec3::new(rng.within(-0.5, 0.5), 1.0, 0.0),
                DVec3::Z,
            );
            let place = DAffine3::from_mat3_translation(
                turn * stretch * shear,
                DVec3::new(rng.within(-9.0, 9.0), rng.within(-9.0, 9.0), 0.0),
            );
            let back = place.inverse();
            let local = cone.carried(back, Stretch::of(back.matrix3));
            for i in 0..=8 {
                for j in 0..=8 {
                    let step = DVec2::new(i as f64, j as f64) / 4.0 - 1.0;
                    let ray = c.ray(cursor + step * radius);
                    for depth in [0.0, 0.01, 0.5, 3.0, 40.0, 1e3] {
                        let x = ray.origin + ray.dir * depth;
                        assert!(in_cone(&cone, x), "{n} {step} {depth}");
                        let y = back.transform_point3(x);
                        assert!(in_cone(&local, y), "{n} {step} {depth} in the part");
                    }
                }
            }
            // A rigid placement carries the cone as it is; an even scale only scales `r0`.
            let rigid = cone.carried(
                DAffine3::from_mat3_translation(turn, DVec3::X),
                Stretch::of(turn),
            );
            let scaled = cone.carried(
                DAffine3::from_mat3_translation(turn * 0.25, DVec3::Y),
                Stretch::of(turn * 0.25),
            );
            let near = |a: f64, b: f64| a == b || (a - b).abs() <= 1e-12 * a.abs().max(b.abs());
            assert!(
                near(rigid.r0, cone.r0) && near(rigid.slope, cone.slope),
                "{n}"
            );
            assert!(near(scaled.r0, cone.r0 * 0.25) && near(scaled.slope, cone.slope));
        }
        assert!(cones > 290, "{cones} of 300 cones are not the whole space");
    }

    #[test]
    fn the_cone_is_the_narrowest_holding_the_square() {
        // Straight ahead in perspective it leans by the angle to a corner of the square.
        let c = cam();
        let mid = c.viewport * 0.5;
        let cone = Cone::around(&c, mid, R);
        let corner = c
            .ray(mid + DVec2::splat(R))
            .dir
            .angle_between(c.ray(mid).dir);
        assert!((cone.slope - corner.tan()).abs() < 1e-12, "{cone:?}");
        assert_eq!(cone.r0, 0.0);
        // In a parallel view it is a cylinder through the corners' rays.
        let mut flat = cam();
        flat.projection = session::Projection::Parallel;
        let cone = Cone::around(&flat, mid, R);
        let across = flat.ray(mid + DVec2::splat(R)).origin - flat.ray(mid).origin;
        assert!((cone.r0 - across.length()).abs() < 1e-12, "{cone:?}");
        assert_eq!(cone.slope, 0.0);
        // A square reaching a right angle off the cursor's ray on a tiny wide view: everything.
        let mut tiny = cam();
        tiny.viewport = DVec2::new(8.0, 8.0);
        tiny.fov_y = 170f64.to_radians();
        assert_eq!(
            Cone::around(&tiny, DVec2::new(1.0, 1.0), R).r0,
            f64::INFINITY
        );
    }

    /// What the picking functions return at one cursor: `infer_in`, `edge_under_in`,
    /// `edge_under_where` keeping the open context's own edges, `face_under_in` and
    /// `guide_under_where`.
    type Picks = (
        Inference,
        Option<(Owner, EdgeId, DVec3, DVec3)>,
        Option<(Owner, EdgeId, DVec3, DVec3)>,
        Option<(Owner, FaceId, DVec3)>,
        Option<(Owner, GuideId, Guide)>,
    );

    fn picks(scenery: &Scenery, c: &Camera, cursor: DVec2, g: &Guides) -> Picks {
        (
            infer_in(scenery, c, cursor, g, R),
            edge_under_in(scenery, c, cursor, R),
            edge_under_where(scenery, c, cursor, R, |o| o == Owner::Open),
            face_under_in(scenery, c, cursor),
            guide_under_where(scenery, c, cursor, R, |_| true),
        )
    }

    /// The picking of 5d097e0, before parts were skipped by their boxes (06fb266), kept to
    /// check that the new code returns what it did: the same code, but for the camera's
    /// projection written out as it was, the size of its cone given a name (`beside_cone`) and
    /// where candidates come from (`Gather`) added.
    mod old {
        use glam::{DAffine3, DVec2, DVec3};
        use kernel::{
            EdgeId, FaceId, Guide, GuideId, InstanceId, MeshId, Model, Scene, Shape, VertexId,
        };
        use session::{Camera, Ray};

        use crate::inference::{Axis, Guides, Inference, SnapKind};
        use crate::scenery::Owner;

        /// One placed mesh.
        #[derive(Clone, Copy, Debug)]
        pub struct Part<'a> {
            pub model: &'a Model,
            /// Maps the mesh's coordinates into the open context's. Rigid (placements turn and
            /// move).
            pub place: DAffine3,
            pub owner: Owner,
        }

        /// Every placed mesh, the open context's first.
        #[derive(Clone, Debug)]
        pub struct Scenery<'a> {
            pub parts: Vec<Part<'a>>,
        }

        impl<'a> Scenery<'a> {
            /// The whole scene seen from the open context `context` (a path of instances from the
            /// root; see `session::Document::context`). None if the path does not resolve.
            pub fn of(scene: &'a Scene, context: &[InstanceId]) -> Option<Self> {
                let (_, to_root) = scene.resolve(context)?;
                let from_root = to_root.inverse();
                let mut parts = Vec::new();
                let mut path = Vec::new();
                collect(
                    scene,
                    MeshId::Root,
                    from_root,
                    context,
                    &mut path,
                    &mut parts,
                );
                // The open mesh first, then the others in walk order.
                parts.sort_by_key(|p: &Part| p.owner != Owner::Open);
                Some(Scenery { parts })
            }
        }

        /// Adds the mesh `id`, placed by `place` (into the open context's coordinates) and
        /// reached by `path`, and everything it places.
        fn collect<'a>(
            scene: &'a Scene,
            id: MeshId,
            place: DAffine3,
            context: &[InstanceId],
            path: &mut Vec<InstanceId>,
            parts: &mut Vec<Part<'a>>,
        ) {
            let Some(model) = scene.mesh(id) else { return };
            let owner = if path.as_slice() == context {
                Owner::Open
            } else if path.starts_with(context) {
                Owner::Instance(path[context.len()])
            } else {
                Owner::Outside
            };
            parts.push(Part {
                model,
                place,
                owner,
            });
            for (iid, instance) in model.instances() {
                path.push(iid);
                collect(
                    scene,
                    MeshId::Definition(instance.definition),
                    place * instance.transform,
                    context,
                    path,
                    parts,
                );
                path.pop();
            }
        }

        /// `Camera::project` as it was: pixel position of a world point, or None if the camera
        /// does not see its depth.
        pub fn project(camera: &Camera, p: DVec3) -> Option<DVec2> {
            if camera.is_parallel() && !camera.sees_depth(p) {
                return None;
            }
            let clip = camera.proj() * camera.view() * p.extend(1.0);
            if clip.w <= 0.0 {
                return None;
            }
            let ndc = clip.truncate() / clip.w;
            Some(DVec2::new(
                (ndc.x + 1.0) * 0.5 * camera.viewport.x,
                (1.0 - ndc.y) * 0.5 * camera.viewport.y,
            ))
        }

        /// Where the functions below get their candidates near the cursor: from their cone, as
        /// they did, or as given.
        #[derive(Clone, Copy)]
        pub enum Gather<'a> {
            Cone,
            Given(&'a Near),
        }

        impl Gather<'_> {
            fn near(
                self,
                scenery: &Scenery,
                camera: &Camera,
                cursor: DVec2,
                radius_px: f64,
            ) -> Near {
                match self {
                    Gather::Cone => Near::around(scenery, camera, cursor, radius_px),
                    Gather::Given(near) => near.clone(),
                }
            }
        }

        /// All of the picking functions at one cursor (see `super::picks`).
        pub fn picks(
            scenery: &Scenery,
            camera: &Camera,
            cursor: DVec2,
            guides: &Guides,
            radius_px: f64,
            gather: Gather,
        ) -> super::Picks {
            (
                infer_in(scenery, camera, cursor, guides, radius_px, gather),
                edge_under_in(scenery, camera, cursor, radius_px, gather),
                edge_under_where(
                    scenery,
                    camera,
                    cursor,
                    radius_px,
                    |o| o == Owner::Open,
                    gather,
                ),
                face_under_in(scenery, camera, cursor),
                guide_under_where(scenery, camera, cursor, radius_px, |_| true),
            )
        }

        pub fn infer_in(
            scenery: &Scenery,
            camera: &Camera,
            cursor: DVec2,
            guides: &Guides,
            radius_px: f64,
            gather: Gather,
        ) -> Inference {
            let ray = camera.ray(cursor);
            let face_hit = nearest_face_hit(scenery, &ray);
            let face = face_hit.and_then(|(owner, f, _)| (owner == Owner::Open).then_some(f));
            let screen_dist = |p: DVec3| project(camera, p).map(|s| s.distance(cursor));
            let snapped = snap_point(scenery, camera, cursor, radius_px, gather);

            if let (Some(anchor), Some(lock)) = (guides.anchor, guides.lock) {
                let point = match snapped {
                    Some((p, _)) => anchor + lock.dir * (p - anchor).dot(lock.dir),
                    None => closest_on_line_to_ray(anchor, lock.dir, &ray, false),
                };
                return Inference {
                    point,
                    kind: lock.kind,
                    face,
                    locked: snapped.map(|(_, kind)| kind),
                };
            }
            if let Some((point, kind)) = snapped {
                return Inference {
                    point,
                    kind,
                    face,
                    locked: None,
                };
            }

            // Along a direction from the anchor. Direction lines are guides, so faces do not
            // hide them.
            if let Some(anchor) = guides.anchor {
                let mut along: Option<(f64, Inference)> = None;
                for (dir, kind) in directions(guides.reference) {
                    let p = closest_on_line_to_ray(anchor, dir, &ray, false);
                    if let Some(d) = screen_dist(p).filter(|d| *d <= radius_px)
                        && p.distance(anchor) > 0.0
                        && along.is_none_or(|(bd, _)| d < bd)
                    {
                        along = Some((
                            d,
                            Inference {
                                point: p,
                                kind,
                                face,
                                locked: None,
                            },
                        ));
                    }
                }
                if let Some((_, hit)) = along {
                    return hit;
                }
            }

            // On a face (nearest along the ray).
            if let Some((_, _, point)) = face_hit {
                return Inference {
                    point,
                    kind: SnapKind::OnFace,
                    face,
                    locked: None,
                };
            }

            match camera.ground_at(cursor) {
                Some(point) => Inference {
                    point,
                    kind: SnapKind::Ground,
                    face,
                    locked: None,
                },
                None => Inference {
                    point: camera.target_depth_at(cursor),
                    kind: SnapKind::Free,
                    face,
                    locked: None,
                },
            }
        }

        fn snap_point(
            scenery: &Scenery,
            camera: &Camera,
            cursor: DVec2,
            radius_px: f64,
            gather: Gather,
        ) -> Option<(DVec3, SnapKind)> {
            let ray = camera.ray(cursor);
            let within = |p: DVec3| {
                project(camera, p)
                    .map(|s| s.distance(cursor))
                    .filter(|d| *d <= radius_px)
                    .filter(|_| visible(scenery, camera, p))
            };
            let near = gather.near(scenery, camera, cursor, radius_px);
            let mut best: Option<(f64, DVec3, SnapKind)> = None;
            let mut offer = |p: DVec3, kind: SnapKind| {
                if let Some(d) = within(p)
                    && best.is_none_or(|(bd, _, _)| d < bd)
                {
                    best = Some((d, p, kind));
                }
            };
            for &(_, _, p) in &near.points {
                offer(p, SnapKind::Endpoint);
            }
            for &(_, _, a, b) in &near.segments {
                offer((a + b) * 0.5, SnapKind::Midpoint);
            }
            offer(DVec3::ZERO, SnapKind::Origin);
            let guides = placed_guides(scenery);
            for &(_, _, g) in &guides {
                if let Guide::Point { position, .. } = g {
                    offer(position, SnapKind::GuidePoint);
                }
            }
            if let Some((_, p, kind)) = best {
                return Some((p, kind));
            }

            // Guide lines passing near the cursor, as the part of them in view.
            let guide_lines: Vec<(DVec3, DVec3)> = guides
                .iter()
                .filter_map(|&(_, _, g)| match g {
                    Guide::Line {
                        point, direction, ..
                    } => Some((point, direction)),
                    Guide::Point { .. } => None,
                })
                .filter(|&(point, dir)| {
                    let p = closest_on_line_to_ray(point, dir, &ray, false);
                    project(camera, p).is_some_and(|s| s.distance(cursor) <= radius_px)
                })
                .collect();
            let lines: Vec<(DVec3, DVec3)> = near
                .segments
                .iter()
                .map(|&(_, _, a, b)| (a, b))
                .chain(
                    guide_lines
                        .iter()
                        .filter_map(|&(point, dir)| camera.line_span(point, dir))
                        .map(|[a, b]| (a, b)),
                )
                .collect();
            let mut crossing: Option<(f64, DVec3)> = None;
            for (i, &(a, b)) in lines.iter().enumerate() {
                for &(c, d) in &lines[i + 1..] {
                    if let Some((p, far)) = apparent_crossing(camera, (a, b), (c, d))
                        && visible(scenery, camera, far)
                        && let Some(dist) = within(p)
                        && crossing.is_none_or(|(bd, _)| dist < bd)
                    {
                        crossing = Some((dist, p));
                    }
                }
            }
            if let Some((_, p)) = crossing {
                return Some((p, SnapKind::Intersection));
            }

            let mut on_edge: Option<(f64, DVec3)> = None;
            for &(_, _, a, b) in &near.segments {
                let p = closest_on_line_to_ray(a, b - a, &ray, true);
                if let Some(d) = within(p)
                    && on_edge.is_none_or(|(bd, _)| d < bd)
                {
                    on_edge = Some((d, p));
                }
            }
            if let Some((_, p)) = on_edge {
                return Some((p, SnapKind::OnEdge));
            }

            let mut on_guide: Option<(f64, DVec3)> = None;
            for &(point, dir) in &guide_lines {
                let p = closest_on_line_to_ray(point, dir, &ray, false);
                if let Some(d) = within(p)
                    && on_guide.is_none_or(|(bd, _)| d < bd)
                {
                    on_guide = Some((d, p));
                }
            }
            on_guide.map(|(_, p)| (p, SnapKind::OnGuide))
        }

        fn placed_guides(scenery: &Scenery) -> Vec<(usize, GuideId, Guide)> {
            scenery
                .parts
                .iter()
                .enumerate()
                .flat_map(|(k, part)| {
                    part.model
                        .guides()
                        .map(move |(id, g)| (k, id, g.transformed(part.place)))
                })
                .collect()
        }

        pub fn guide_under_where(
            scenery: &Scenery,
            camera: &Camera,
            cursor: DVec2,
            radius_px: f64,
            keep: impl Fn(Owner) -> bool,
        ) -> Option<(Owner, GuideId, Guide)> {
            let ray = camera.ray(cursor);
            let mut best: Option<(f64, (Owner, GuideId, Guide))> = None;
            for (part, id, g) in placed_guides(scenery) {
                let owner = scenery.parts[part].owner;
                if !keep(owner) {
                    continue;
                }
                let p = match g {
                    Guide::Point { position, .. } => position,
                    Guide::Line {
                        point, direction, ..
                    } => closest_on_line_to_ray(point, direction, &ray, false),
                };
                let Some(d) = project(camera, p).map(|s| s.distance(cursor)) else {
                    continue;
                };
                if d <= radius_px
                    && best.is_none_or(|(bd, _)| d < bd)
                    && visible(scenery, camera, p)
                {
                    best = Some((d, (owner, id, g)));
                }
            }
            best.map(|(_, hit)| hit)
        }

        fn apparent_crossing(
            camera: &Camera,
            (a, b): (DVec3, DVec3),
            (c, d): (DVec3, DVec3),
        ) -> Option<(DVec3, DVec3)> {
            let (sa, sb) = (project(camera, a)?, project(camera, b)?);
            let (sc, sd) = (project(camera, c)?, project(camera, d)?);
            let (r, q) = (sb - sa, sd - sc);
            let denom = r.perp_dot(q);
            if denom.abs() < 1e-12 {
                return None;
            }
            let t = (sc - sa).perp_dot(q) / denom;
            let u = (sc - sa).perp_dot(r) / denom;
            let inside = |k: f64| k > 1e-6 && k < 1.0 - 1e-6;
            if !inside(t) || !inside(u) {
                return None;
            }
            let ray = camera.ray(sa + r * t);
            let on_ab = closest_on_line_to_ray(a, b - a, &ray, true);
            let on_cd = closest_on_line_to_ray(c, d - c, &ray, true);
            Some(
                if on_ab.distance(ray.origin) <= on_cd.distance(ray.origin) {
                    (on_ab, on_cd)
                } else {
                    (on_cd, on_ab)
                },
            )
        }

        fn directions(reference: Option<DVec3>) -> Vec<(DVec3, SnapKind)> {
            let mut out: Vec<(DVec3, SnapKind)> = Axis::ALL
                .into_iter()
                .map(|a| (a.dir_in(glam::DMat3::IDENTITY), SnapKind::Axis(a)))
                .collect();
            let along_axis = |d: DVec3| {
                Axis::ALL
                    .iter()
                    .any(|a| d.cross(a.dir_in(glam::DMat3::IDENTITY)).length() < 1e-6)
            };
            let Some(r) = reference.and_then(|r| r.try_normalize()) else {
                return out;
            };
            if along_axis(r) {
                return out;
            }
            out.push((r, SnapKind::Parallel));
            let horizontal = r.cross(DVec3::Z).try_normalize();
            let upright = horizontal.and_then(|h| r.cross(h).try_normalize());
            for p in [horizontal, upright].into_iter().flatten() {
                if !along_axis(p) {
                    out.push((p, SnapKind::Perpendicular));
                }
            }
            out
        }

        pub fn face_under_in(
            scenery: &Scenery,
            camera: &Camera,
            cursor: DVec2,
        ) -> Option<(Owner, FaceId, DVec3)> {
            nearest_face_hit(scenery, &camera.ray(cursor))
        }

        pub fn edge_under_in(
            scenery: &Scenery,
            camera: &Camera,
            cursor: DVec2,
            radius_px: f64,
            gather: Gather,
        ) -> Option<(Owner, EdgeId, DVec3, DVec3)> {
            edge_under_where(scenery, camera, cursor, radius_px, |_| true, gather)
        }

        pub fn edge_under_where(
            scenery: &Scenery,
            camera: &Camera,
            cursor: DVec2,
            radius_px: f64,
            keep: impl Fn(Owner) -> bool,
            gather: Gather,
        ) -> Option<(Owner, EdgeId, DVec3, DVec3)> {
            let ray = camera.ray(cursor);
            let mut best: Option<(f64, (Owner, EdgeId, DVec3, DVec3))> = None;
            for (part, id, a, b) in gather.near(scenery, camera, cursor, radius_px).segments {
                let owner = scenery.parts[part].owner;
                if !keep(owner) {
                    continue;
                }
                let p = closest_on_line_to_ray(a, b - a, &ray, true);
                let Some(d) = project(camera, p).map(|s| s.distance(cursor)) else {
                    continue;
                };
                if d <= radius_px
                    && best.is_none_or(|(bd, _)| d < bd)
                    && visible(scenery, camera, p)
                {
                    best = Some((d, (owner, id, a, b)));
                }
            }
            best.map(|(_, hit)| hit)
        }

        #[derive(Clone, Debug, Default)]
        pub struct Near {
            pub points: Vec<(usize, VertexId, DVec3)>,
            pub segments: Vec<(usize, EdgeId, DVec3, DVec3)>,
        }

        /// The cone `Near::around` searched: twice the spread of the rays through the cursor and
        /// `radius_px` beside it (across).
        pub fn beside_cone(camera: &Camera, cursor: DVec2, radius_px: f64) -> (f64, f64) {
            let ray = camera.ray(cursor);
            let beside = camera.ray(cursor + DVec2::new(radius_px, 0.0));
            let slope = 2.0 * beside.dir.distance(ray.dir);
            let r0 = 2.0 * beside.origin.distance(ray.origin);
            (r0, slope)
        }

        impl Near {
            pub fn around(
                scenery: &Scenery,
                camera: &Camera,
                cursor: DVec2,
                radius_px: f64,
            ) -> Near {
                let ray = camera.ray(cursor);
                // The rays through the cursor and a point `radius_px` beside it spread apart by the
                // cone's slope (perspective) or lie that far apart (parallel); doubling covers the
                // diagonal of the pixel square around the cursor.
                let (r0, slope) = beside_cone(camera, cursor, radius_px);
                let mut near = Near {
                    points: Vec::new(),
                    segments: Vec::new(),
                };
                for (k, part) in scenery.parts.iter().enumerate() {
                    let back = part.place.inverse();
                    let (origin, dir) = (
                        back.transform_point3(ray.origin),
                        back.transform_vector3(ray.dir),
                    );
                    let at = |p: DVec3| part.place.transform_point3(p);
                    part.model
                        .index()
                        .near_ray(origin, dir, r0, slope, |s| match *s {
                            Shape::Point(v, p) => near.points.push((k, v, at(p))),
                            Shape::Segment(e, a, b) => {
                                if part.model.edge(e).is_some_and(|edge| !edge.soft) {
                                    near.segments.push((k, e, at(a), at(b)));
                                }
                            }
                            Shape::Triangle(..) => {}
                        });
                }
                near.sort();
                near
            }

            /// Puts the candidates in part and id order.
            pub fn sort(&mut self) {
                self.points.sort_by_key(|(k, v, _)| (*k, *v));
                self.segments.sort_by_key(|(k, e, _, _)| (*k, *e));
            }
        }

        fn visible(scenery: &Scenery, camera: &Camera, p: DVec3) -> bool {
            let eye = camera.sight_origin(p);
            let to = p - eye;
            let dist = to.length();
            if dist < 1e-12 {
                return true;
            }
            let dir = to / dist;
            scenery.parts.iter().all(|part| {
                let back = part.place.inverse();
                part.model
                    .index()
                    .ray_hit(
                        back.transform_point3(eye),
                        back.transform_vector3(dir),
                        0.0,
                        dist * (1.0 - 1e-6),
                    )
                    .is_none()
            })
        }

        pub fn closest_on_line_to_ray(
            origin: DVec3,
            dir: DVec3,
            ray: &Ray,
            segment: bool,
        ) -> DVec3 {
            let w0 = origin - ray.origin;
            let a = dir.dot(dir);
            let b = dir.dot(ray.dir);
            let c = ray.dir.dot(ray.dir);
            let d = dir.dot(w0);
            let e = ray.dir.dot(w0);
            let denom = a * c - b * b;
            let mut t = if denom.abs() < 1e-12 * a * c {
                0.0
            } else {
                (b * e - c * d) / denom
            };
            if segment {
                t = t.clamp(0.0, 1.0);
            }
            origin + dir * t
        }

        fn nearest_face_hit(scenery: &Scenery, ray: &Ray) -> Option<(Owner, FaceId, DVec3)> {
            let mut best: Option<(f64, Owner, FaceId)> = None;
            for part in &scenery.parts {
                let back = part.place.inverse();
                let reach = best.map_or(f64::INFINITY, |(t, _, _)| t + crate::inference::FACE_TIE);
                if let Some((f, t)) = part.model.index().ray_hit(
                    back.transform_point3(ray.origin),
                    back.transform_vector3(ray.dir),
                    0.0,
                    reach,
                ) && best.is_none_or(|(bt, _, _)| t <= bt + crate::inference::FACE_TIE)
                {
                    best = Some((t, part.owner, f));
                }
            }
            best.map(|(t, owner, f)| (owner, f, ray.origin + ray.dir * t))
        }
    }

    /// Distance on screen from `p` to the segment `a`-`b`, worked out apart from the code under
    /// test.
    fn screen_distance(p: DVec2, a: DVec2, b: DVec2) -> f64 {
        let along = b - a;
        if along == DVec2::ZERO {
            return p.distance(a);
        }
        let t = ((p - a).dot(along) / along.length_squared()).clamp(0.0, 1.0);
        p.distance(a.lerp(b, t))
    }

    /// Where the old projection puts both ends of an edge, if it sees them.
    type Ends = Option<(DVec2, DVec2)>;

    /// Every vertex and edge (not soft) of a scenery of the code before culling, placed, and
    /// where its old projection puts them on screen.
    struct Everything {
        points: Vec<(usize, VertexId, DVec3, Option<DVec2>)>,
        segments: Vec<(usize, EdgeId, DVec3, DVec3, Ends)>,
    }

    impl Everything {
        fn of(scenery: &old::Scenery, c: &Camera) -> Everything {
            let mut all = Everything {
                points: Vec::new(),
                segments: Vec::new(),
            };
            for (k, part) in scenery.parts.iter().enumerate() {
                let at = |p: DVec3| part.place.transform_point3(p);
                for (v, vertex) in part.model.vertices() {
                    let p = at(vertex.pos);
                    all.points.push((k, v, p, old::project(c, p)));
                }
                for (e, edge) in part.model.edges() {
                    if !edge.soft {
                        let [a, b] = edge.v.map(|v| at(part.model.position(v).unwrap()));
                        let screen = old::project(c, a).zip(old::project(c, b));
                        all.segments.push((k, e, a, b, screen));
                    }
                }
            }
            all
        }

        /// True if the edge `a`-`b`, whose ends land at `screen`, may come within `reach` of
        /// `cursor` on screen: when the camera sees both ends, if its picture does; when not
        /// (it then crosses no other edge on screen), if its middle or its point nearest the
        /// cursor's ray do, the only points of it offered then.
        fn edge_near(
            c: &Camera,
            cursor: DVec2,
            (a, b, screen): (DVec3, DVec3, Ends),
            reach: f64,
        ) -> bool {
            let close = |p: DVec3| old::project(c, p).is_some_and(|s| s.distance(cursor) <= reach);
            match screen {
                Some((sa, sb)) => screen_distance(cursor, sa, sb) <= reach,
                None => {
                    let ray = c.ray(cursor);
                    close((a + b) * 0.5) || close(old::closest_on_line_to_ray(a, b - a, &ray, true))
                }
            }
        }

        /// What `Near::around` of the code before gathers near `cursor` when its cone misses
        /// nothing: every vertex, and every edge, that may lie within the radius of the cursor
        /// on screen, give or take rounding. Everything the code snaps to or picks is a vertex
        /// or a point of an edge there (a middle, a crossing, a point nearest the cursor's
        /// ray); nothing else changes what is picked.
        fn near(&self, c: &Camera, cursor: DVec2, radius_px: f64) -> old::Near {
            let reach = radius_px + 1e-6;
            let mut near = old::Near {
                points: self
                    .points
                    .iter()
                    .filter(|(.., s)| s.is_some_and(|s| s.distance(cursor) <= reach))
                    .map(|&(k, v, p, _)| (k, v, p))
                    .collect(),
                segments: self
                    .segments
                    .iter()
                    .filter(|&&(_, _, a, b, s)| Self::edge_near(c, cursor, (a, b, s), reach))
                    .map(|&(k, e, a, b, _)| (k, e, a, b))
                    .collect(),
            };
            near.sort();
            near
        }

        /// The vertices and edges of `complete` within `radius_px` of `cursor` on screen that
        /// `cone` lacks, each as the index of its part and points along it.
        fn missed(
            &self,
            complete: &old::Near,
            cone: &old::Near,
            c: &Camera,
            cursor: DVec2,
            radius_px: f64,
        ) -> Vec<(usize, Vec<DVec3>)> {
            let mut out = Vec::new();
            for &(k, v, p) in &complete.points {
                let within = old::project(c, p).is_some_and(|s| s.distance(cursor) <= radius_px);
                if within && !cone.points.iter().any(|&(j, w, _)| (j, w) == (k, v)) {
                    out.push((k, vec![p]));
                }
            }
            for &(k, e, a, b) in &complete.segments {
                let screen = old::project(c, a).zip(old::project(c, b));
                if Self::edge_near(c, cursor, (a, b, screen), radius_px)
                    && !cone.segments.iter().any(|&(j, f, ..)| (j, f) == (k, e))
                {
                    out.push((k, (0..=32).map(|i| a.lerp(b, i as f64 / 32.0)).collect()));
                }
            }
            out
        }
    }

    /// Cursors to try in `c`'s view of `scenery`: a grid over the view, spots next to placed
    /// corners, the origin and guide points so that points snap, and in a wide view the
    /// corners of the view and spots 0.85 radii above and below placed corners (toward the
    /// ends of a wide view, rays fan out fastest up and down).
    fn cursors(scenery: &Scenery, c: &Camera, wide: bool) -> Vec<DVec2> {
        let size = c.viewport;
        let mut out: Vec<DVec2> = (0..7)
            .flat_map(|i| {
                (0..5).map(move |j| {
                    DVec2::new(
                        40.0 + (size.x - 80.0) * i as f64 / 6.0,
                        30.0 + (size.y - 60.0) * j as f64 / 4.0,
                    )
                })
            })
            .collect();
        let mut spots = vec![DVec3::ZERO];
        for part in scenery.parts().step_by(4) {
            for (_, v) in part.model.vertices().step_by(7) {
                spots.push(part.place.transform_point3(v.pos));
            }
        }
        for &(_, _, g) in scenery.guides() {
            if let Guide::Point { position, .. } = g {
                spots.push(position);
            }
        }
        for p in spots.iter().filter_map(|p| c.project(*p)) {
            out.push(p + DVec2::new(2.0, -1.5));
            if wide {
                out.push(p + DVec2::new(0.0, 0.85 * R));
                out.push(p - DVec2::new(0.0, 0.85 * R));
            }
        }
        if wide {
            out.extend(near_corners(c));
        }
        out
    }

    /// How the picks at the cursors tried compared with the code before culling: at how many
    /// cursors its cone missed no candidate within the radius (the picks were the same); and
    /// at how many it missed one in a rigid part (beyond the cone, in a wide view), or in a
    /// part placed with a scale, and the picks changed.
    #[derive(Debug, Default)]
    struct Tally {
        same: usize,
        narrow: usize,
        scaled: usize,
        changed: usize,
    }

    /// Checks the picks at `cursor` against the code before culling (`before`, whose
    /// vertices and edges are `all`) and returns the inference.
    ///
    /// Given every candidate within the radius, that code must pick exactly the same. With
    /// its own cone it must too, unless the cone missed a candidate within the radius, which
    /// the new cone is to find: one beyond the old cone, which in a rigid part happens only
    /// in a wide view; or one in a part placed with a scale, where the old cone was carried
    /// with its direction scaled and so searched around the wrong axis.
    #[allow(clippy::too_many_arguments)]
    fn compare(
        scenery: &Scenery,
        before: &old::Scenery,
        all: &Everything,
        c: &Camera,
        cursor: DVec2,
        g: &Guides,
        wide: bool,
        tally: &mut Tally,
    ) -> Inference {
        let now = picks(scenery, c, cursor, g);
        let complete = all.near(c, cursor, R);
        let given = old::picks(before, c, cursor, g, R, old::Gather::Given(&complete));
        assert_eq!(now, given, "{cursor} with every candidate");
        // Its own cone, gathered once: each of its functions gathered the same.
        let cone = old::Near::around(before, c, cursor, R);
        let then = old::picks(before, c, cursor, g, R, old::Gather::Given(&cone));
        let missed = all.missed(&complete, &cone, c, cursor, R);
        if missed.is_empty() {
            assert_eq!(now, then, "{cursor}");
            tally.same += 1;
            return now.0;
        }
        let (r0, slope) = old::beside_cone(c, cursor, R);
        let ray = c.ray(cursor);
        let (mut narrow, mut scaled) = (false, false);
        for (k, along) in missed {
            let place = before.parts[k].place.matrix3;
            if (place.transpose() * place).abs_diff_eq(glam::DMat3::IDENTITY, 1e-9) {
                assert!(
                    wide,
                    "{cursor}: the old cone missed {along:?} in a normal view"
                );
                for x in along {
                    let t = (x - ray.origin).dot(ray.dir);
                    let off = (x - ray.origin - ray.dir * t).length();
                    assert!(
                        off > r0 + slope * t.max(0.0),
                        "{cursor}: {x} in the old cone"
                    );
                }
                narrow = true;
            } else {
                scaled = true;
            }
        }
        tally.narrow += usize::from(narrow);
        tally.scaled += usize::from(scaled);
        tally.changed += usize::from(now != then);
        now.0
    }

    #[test]
    fn picking_returns_what_the_code_before_culling_did() {
        let (s, contexts) = crowd();
        let mut tally = Tally::default();
        let mut kinds = std::collections::BTreeSet::new();
        let mut skipped = 0;
        let mut checked = 0;
        for (k, context) in contexts.iter().enumerate() {
            let scenery = Scenery::of(&s, context).unwrap();
            let before = old::Scenery::of(&s, context).unwrap();
            let to_root = s.resolve(context).unwrap().1;
            let normal = views().into_iter().map(|v| (false, v));
            // Wide views from the root and from deepest in, to keep the test quick.
            let wide = wide_views().into_iter().filter(|_| k % 2 == 0);
            for (wide, view) in normal.chain(wide.map(|v| (true, v))) {
                let c = view.placed(to_root.inverse());
                let all = Everything::of(&before, &c);
                let middle = c.ray(c.viewport * 0.5);
                let anchor = middle.origin + middle.dir * 5.0;
                for (n, cursor) in cursors(&scenery, &c, wide).into_iter().enumerate() {
                    let ray = c.ray(cursor);
                    skipped += scenery
                        .parts()
                        .filter(|p| !p.may_meet_ray(ray.origin, ray.dir, f64::INFINITY))
                        .count();
                    checked += scenery.len();
                    // Every other cursor follows directions from an anchor.
                    let g = Guides {
                        anchor: (n % 2 == 1).then_some(anchor),
                        reference: Some(DVec3::new(1.0, 2.0, 0.5)),
                        lock: None,
                    };
                    let hit = compare(&scenery, &before, &all, &c, cursor, &g, wide, &mut tally);
                    kinds.insert(format!("{:?}", hit.kind));
                }
            }
        }
        // The edges near the corners of wide views, which the code before missed.
        let g = Guides::default();
        for fov in [120.0, 160.0] {
            let c = wide(cam(), fov);
            for cursor in near_corners(&c) {
                let (p, q) = edge_by(&c, cursor);
                for holder in HOLDERS {
                    let (s, _) = holding_edge(p, q, holder);
                    let scenery = Scenery::of(&s, &[]).unwrap();
                    let before = old::Scenery::of(&s, &[]).unwrap();
                    let all = Everything::of(&before, &c);
                    let mut spots = vec![cursor];
                    spots.extend(cursors(&scenery, &c, true));
                    for spot in spots {
                        compare(&scenery, &before, &all, &c, spot, &g, true, &mut tally);
                    }
                }
            }
        }
        // The check means something only if parts were skipped, many kinds of snap came up,
        // and the old cone both sufficed and fell short, in rigid and scaled parts.
        assert!(skipped * 2 > checked, "{skipped} of {checked} skipped");
        for kind in [
            "Endpoint",
            "Midpoint",
            "OnEdge",
            "OnFace",
            "Intersection",
            "GuidePoint",
            "OnGuide",
        ] {
            assert!(kinds.contains(kind), "no {kind} in {kinds:?}");
        }
        assert!(tally.same > 3 * tally.changed, "{tally:?}");
        assert!(
            tally.narrow > 0 && tally.scaled > 0 && tally.changed > 0,
            "{tally:?}"
        );
    }
}
