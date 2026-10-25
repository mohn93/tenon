//! What tools draw on top of the scene, in screen space: the snap marker (`marker`), the
//! snap's name (`snap_label`) and the preview lines clipped and projected (`preview_lines`).

use glam::{DVec2, DVec3};
use session::{Camera, Projector};

use crate::inference::{Axis, Inference, SnapKind};

/// sRGB and alpha, 0 to 255.
pub type Rgba = [u8; 4];

/// The shared axis colors: red, green, blue.
pub const AXIS_X: Rgba = [0xE5, 0x48, 0x4D, 0xFF];
pub const AXIS_Y: Rgba = [0x30, 0xA4, 0x6C, 0xFF];
pub const AXIS_Z: Rgba = [0x3E, 0x63, 0xDD, 0xFF];
/// Preview lines parallel or perpendicular to the reference edge.
pub const PARALLEL: Rgba = [0xCC, 0x1A, 0xCC, 0xFF];
/// Preview lines on no axis or edge: on a light canvas, and on a dark one.
pub const LINE_DAY: Rgba = [0x1A, 0x1A, 0x1A, 0xFF];
pub const LINE_NIGHT: Rgba = [0xE8, 0xE9, 0xEC, 0xFF];
/// A placed part's ghost: its blue core, and the casing around it, light on a dark canvas
/// and dark on a light one.
pub const GHOST_CORE: Rgba = [0x2F, 0x8C, 0xFF, 0xFF];
pub const GHOST_CASING_DAY: Rgba = [0x10, 0x20, 0x40, 0xC0];
pub const GHOST_CASING_NIGHT: Rgba = [0xF4, 0xF6, 0xFA, 0xC0];
pub const ENDPOINT: Rgba = [0x2F, 0xBF, 0x4A, 0xFF];
pub const MIDPOINT: Rgba = [0x12, 0xB5, 0xCB, 0xFF];
pub const INTERSECTION: Rgba = [0xFF, 0x8A, 0x00, 0xFF];
pub const ON_EDGE: Rgba = [0xE5, 0x48, 0x4D, 0xFF];
pub const ON_FACE: Rgba = [0x3E, 0x63, 0xDD, 0xFF];

/// Marker size across, halo width, line widths, in points.
pub const MARKER_PT: f32 = 12.0;
pub const HALO_PT: f32 = 1.5;
pub const LINE_PT: f32 = 2.0;
pub const LOCKED_LINE_PT: f32 = 2.5;
/// The ghost's core and casing widths, in points.
pub const GHOST_CORE_PT: f32 = 2.0;
pub const GHOST_CASING_PT: f32 = 4.5;

/// How a snap marker is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A filled circle.
    Dot,
    /// An X of two strokes.
    Cross,
    /// A hollow square.
    Square,
    /// A filled diamond.
    Diamond,
    /// An upright cross of two strokes.
    Plus,
}

/// The marker drawn at a snap of `kind`, and its color; None for snaps with no marker (axes,
/// parallel, perpendicular, ground and free points).
pub fn marker(kind: SnapKind) -> Option<(Shape, Rgba)> {
    Some(match kind {
        SnapKind::Endpoint | SnapKind::Origin => (Shape::Dot, ENDPOINT),
        SnapKind::Midpoint => (Shape::Dot, MIDPOINT),
        SnapKind::Intersection => (Shape::Cross, INTERSECTION),
        SnapKind::OnEdge | SnapKind::OnGuide => (Shape::Square, ON_EDGE),
        SnapKind::OnFace => (Shape::Diamond, ON_FACE),
        SnapKind::GuidePoint => (Shape::Plus, ENDPOINT),
        SnapKind::Axis(_)
        | SnapKind::Parallel
        | SnapKind::Perpendicular
        | SnapKind::Ground
        | SnapKind::Free => return None,
    })
}

/// Where snapped geometry lies, seen from the open context.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Within {
    /// In the open context itself, or no geometry at all (an origin or axis snap).
    #[default]
    Open,
    /// Inside a group the open context places.
    Group,
    /// Inside a component the open context places.
    Component,
    /// Outside the open context.
    Outside,
}

/// The name of a snap, e.g. "Endpoint", with " in Group" or " in Component" when the snapped
/// geometry lies inside one; None for ground and free points.
pub fn snap_label(kind: SnapKind, within: Within) -> Option<String> {
    let name = match kind {
        SnapKind::Endpoint => "Endpoint",
        SnapKind::Midpoint => "Midpoint",
        SnapKind::Origin => "Origin",
        SnapKind::GuidePoint => "Guide Point",
        SnapKind::Intersection => "Intersection",
        SnapKind::OnEdge => "On Edge",
        SnapKind::OnGuide => "On Guide",
        SnapKind::Axis(Axis::X) => "On Red Axis",
        SnapKind::Axis(Axis::Y) => "On Green Axis",
        SnapKind::Axis(Axis::Z) => "On Blue Axis",
        SnapKind::Parallel => "Parallel to Edge",
        SnapKind::Perpendicular => "Perpendicular to Edge",
        SnapKind::OnFace => "On Face",
        SnapKind::Ground | SnapKind::Free => return None,
    };
    let suffix = match within {
        Within::Open | Within::Outside => "",
        Within::Group => " in Group",
        Within::Component => " in Component",
    };
    Some(format!("{name}{suffix}"))
}

/// A preview segment on screen, in view pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenLine {
    pub a: DVec2,
    pub b: DVec2,
    pub color: Rgba,
    /// In points.
    pub width: f32,
}

/// The part of `a`..`b` the camera sees (perspective: in front of the near plane; parallel:
/// within `depth_range`), projected; None when none of it is seen.
pub fn clip_segment(camera: &Camera, a: DVec3, b: DVec3) -> Option<[DVec2; 2]> {
    clip_with(camera, &camera.projector(), a, b)
}

/// `clip_segment` with the camera's projector made once.
fn clip_with(camera: &Camera, projector: &Projector, a: DVec3, b: DVec3) -> Option<[DVec2; 2]> {
    let (near, far) = seen_depths(camera);
    let (da, db) = (camera.depth(a), camera.depth(b));
    if (da < near && db < near) || (da > far && db > far) {
        return None;
    }
    // Where the segment reaches `depth`, as a fraction from `a` (linear in depth). Only asked
    // for when one end lies beyond `depth` and the other does not, so `db != da`.
    let cut = |depth: f64| (depth - da) / (db - da);
    let clamp = |d: f64, t: f64| {
        if d < near {
            cut(near)
        } else if d > far {
            cut(far)
        } else {
            t
        }
    };
    let (t0, t1) = (clamp(da, 0.0), clamp(db, 1.0));
    let at = |t: f64| a + (b - a) * t;
    Some([projector.project(at(t0))?, projector.project(at(t1))?])
}

/// The depths (see `Camera::depth`) a clipped end may take: the camera's, shrunk toward the
/// inside by 0.1% of their span so an end cut at a bound still projects. In perspective the
/// far bound is infinite, so the near one moves by 0.1% of itself.
fn seen_depths(camera: &Camera) -> (f64, f64) {
    if camera.is_parallel() {
        let (near, far) = camera.depth_range();
        let inset = 0.001 * (far - near).abs();
        (near + inset, far - inset)
    } else {
        let near = camera.near();
        (near + 0.001 * near, f64::INFINITY)
    }
}

/// The tool's preview segments as seen on screen. All take one color: the axis color when
/// `hit` follows an axis, magenta when parallel or perpendicular to an edge, otherwise dark on
/// a light canvas and light on a dark one (`dark_canvas`). They are `LOCKED_LINE_PT` wide
/// while a lock is in force, else `LINE_PT`. Segments the camera does not see are left out.
pub fn preview_lines(
    preview: &[[DVec3; 2]],
    hit: Option<&Inference>,
    locked: bool,
    dark_canvas: bool,
    camera: &Camera,
) -> Vec<ScreenLine> {
    let color = match hit.map(|h| h.kind) {
        Some(SnapKind::Axis(Axis::X)) => AXIS_X,
        Some(SnapKind::Axis(Axis::Y)) => AXIS_Y,
        Some(SnapKind::Axis(Axis::Z)) => AXIS_Z,
        Some(SnapKind::Parallel | SnapKind::Perpendicular) => PARALLEL,
        _ if dark_canvas => LINE_NIGHT,
        _ => LINE_DAY,
    };
    let width = if locked { LOCKED_LINE_PT } else { LINE_PT };
    let projector = camera.projector();
    preview
        .iter()
        .filter_map(|&[a, b]| clip_with(camera, &projector, a, b))
        .map(|[a, b]| ScreenLine { a, b, color, width })
        .collect()
}

/// The lines of a part being placed (a pasted clip, a library item, 3D text): each in a blue core
/// over a wider casing that contrasts with the canvas (`dark_canvas`), so the ghost shows on
/// light and dark grounds alike. All casings come first, so no core is hidden by another
/// line's casing.
pub fn ghost_lines(preview: &[[DVec3; 2]], dark_canvas: bool, camera: &Camera) -> Vec<ScreenLine> {
    let projector = camera.projector();
    let seen: Vec<[DVec2; 2]> = preview
        .iter()
        .filter_map(|&[a, b]| clip_with(camera, &projector, a, b))
        .collect();
    let casing = if dark_canvas {
        GHOST_CASING_NIGHT
    } else {
        GHOST_CASING_DAY
    };
    let line = |[a, b]: [DVec2; 2], color, width| ScreenLine { a, b, color, width };
    seen.iter()
        .map(|s| line(*s, casing, GHOST_CASING_PT))
        .chain(seen.iter().map(|s| line(*s, GHOST_CORE, GHOST_CORE_PT)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;

    fn cam() -> Camera {
        Camera::new(DVec2::new(800.0, 600.0))
    }

    #[test]
    fn parallel_and_perpendicular_previews_are_magenta() {
        let c = cam();
        for kind in [SnapKind::Parallel, SnapKind::Perpendicular] {
            let hit = Inference {
                point: DVec3::new(1.0, 2.0, 0.0),
                kind,
                face: None,
                locked: None,
            };
            let lines = preview_lines(&[[DVec3::ZERO, hit.point]], Some(&hit), false, false, &c);
            assert_eq!(lines[0].color, PARALLEL, "{kind:?}");
        }
    }

    #[test]
    fn each_snap_kind_has_its_shape_and_color() {
        use SnapKind::*;
        assert_eq!(marker(Endpoint), Some((Shape::Dot, ENDPOINT)));
        assert_eq!(marker(Origin), Some((Shape::Dot, ENDPOINT)));
        assert_eq!(marker(Midpoint), Some((Shape::Dot, MIDPOINT)));
        assert_eq!(marker(Intersection), Some((Shape::Cross, INTERSECTION)));
        assert_eq!(marker(OnEdge), Some((Shape::Square, ON_EDGE)));
        assert_eq!(marker(OnGuide), Some((Shape::Square, ON_EDGE)));
        assert_eq!(marker(OnFace), Some((Shape::Diamond, ON_FACE)));
        assert_eq!(marker(GuidePoint), Some((Shape::Plus, ENDPOINT)));
        for k in [Axis(super::Axis::X), Parallel, Perpendicular, Ground, Free] {
            assert_eq!(marker(k), None, "{k:?}");
        }
        assert_eq!(ON_EDGE, [0xE5, 0x48, 0x4D, 0xFF]);
    }

    #[test]
    fn snap_labels_name_the_snap_and_where_it_lies() {
        use SnapKind::*;
        assert_eq!(
            snap_label(Endpoint, Within::Open).as_deref(),
            Some("Endpoint")
        );
        assert_eq!(
            snap_label(Midpoint, Within::Group).as_deref(),
            Some("Midpoint in Group")
        );
        assert_eq!(
            snap_label(OnFace, Within::Component).as_deref(),
            Some("On Face in Component")
        );
        assert_eq!(
            snap_label(OnEdge, Within::Outside).as_deref(),
            Some("On Edge")
        );
        assert_eq!(
            snap_label(Axis(super::Axis::X), Within::Open).as_deref(),
            Some("On Red Axis")
        );
        assert_eq!(
            snap_label(Axis(super::Axis::Y), Within::Open).as_deref(),
            Some("On Green Axis")
        );
        assert_eq!(
            snap_label(Axis(super::Axis::Z), Within::Open).as_deref(),
            Some("On Blue Axis")
        );
        assert_eq!(
            snap_label(Parallel, Within::Open).as_deref(),
            Some("Parallel to Edge")
        );
        assert_eq!(
            snap_label(Perpendicular, Within::Open).as_deref(),
            Some("Perpendicular to Edge")
        );
        assert_eq!(
            snap_label(GuidePoint, Within::Open).as_deref(),
            Some("Guide Point")
        );
        assert_eq!(snap_label(Ground, Within::Open), None);
        assert_eq!(snap_label(Free, Within::Open), None);
    }

    #[test]
    fn a_ghost_is_a_core_over_a_casing_that_contrasts_with_the_canvas() {
        let c = cam();
        let seg = [[DVec3::ZERO, DVec3::new(0.0, 3.0, 0.0)]];
        let day = ghost_lines(&seg, false, &c);
        assert_eq!(day.len(), 2);
        assert_eq!(
            (day[0].color, day[0].width),
            (GHOST_CASING_DAY, GHOST_CASING_PT)
        );
        assert_eq!((day[1].color, day[1].width), (GHOST_CORE, GHOST_CORE_PT));
        assert_eq!(ghost_lines(&seg, true, &c)[0].color, GHOST_CASING_NIGHT);
        assert!(ghost_lines(&[], false, &c).is_empty());
    }

    #[test]
    fn preview_lines_take_the_axis_color_and_widen_when_locked() {
        let c = cam();
        let hit = Inference {
            point: DVec3::new(0.0, 3.0, 0.0),
            kind: SnapKind::Axis(Axis::Y),
            face: None,
            locked: None,
        };
        let seg = [[DVec3::ZERO, hit.point]];
        let free = preview_lines(&seg, Some(&hit), false, false, &c);
        assert_eq!(
            (free.len(), free[0].color, free[0].width),
            (1, AXIS_Y, LINE_PT)
        );
        let locked = preview_lines(&seg, Some(&hit), true, false, &c);
        assert_eq!(locked[0].width, LOCKED_LINE_PT);
        assert_eq!(free[0].a, c.project(DVec3::ZERO).unwrap());
        let plain = Inference {
            kind: SnapKind::Free,
            ..hit
        };
        assert_eq!(
            preview_lines(&seg, Some(&plain), false, false, &c)[0].color,
            LINE_DAY
        );
        assert_eq!(
            preview_lines(&seg, Some(&plain), false, true, &c)[0].color,
            LINE_NIGHT
        );
        let par = Inference {
            kind: SnapKind::Parallel,
            ..hit
        };
        assert_eq!(
            preview_lines(&seg, Some(&par), false, false, &c)[0].color,
            PARALLEL
        );
    }

    #[test]
    fn lines_crossing_the_near_plane_are_clipped_not_flipped() {
        let c = cam();
        // Both ends off to the side, so the line misses the eye and its clipped part has
        // length on screen.
        let ahead = c.target + c.right();
        let behind = c.eye - c.forward() * 5.0 + c.right() * 3.0;
        let [a, b] = clip_segment(&c, ahead, behind).expect("the part in front is seen");
        assert_eq!(a, c.project(ahead).unwrap());
        // The clipped end lies on the same side of the screen as the line heads toward the eye,
        // and stays finite.
        assert!(b.is_finite() && b.distance(a) > 1.0);
        let toward = c.project(ahead + (behind - ahead) * 0.1).unwrap() - a;
        assert!((b - a).dot(toward) > 0.0, "not flipped: {a} -> {b}");
        assert_eq!(clip_segment(&c, behind, behind - c.forward()), None);
        // Parallel: clipped to the depth range the same way.
        let mut p = cam();
        p.projection = session::Projection::Parallel;
        let (_, far) = p.depth_range();
        let out = p.eye + p.forward() * (far + 10.0);
        let [a, b] = clip_segment(&p, p.target, out).unwrap();
        assert_eq!(a, p.project(p.target).unwrap());
        assert!(b.is_finite());
        assert_eq!(clip_segment(&p, out, out + p.forward()), None);
    }
}
