use bytemuck::{Pod, Zeroable};
use glam::{DAffine3, DVec3};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LineVertex {
    pub pos: [f32; 3],
    pub color: [f32; 4],
}

// sRGB-authored colors.
pub const BACKGROUND: [f32; 4] = [0.96, 0.96, 0.96, 1.0];
/// The background when the view is lit by night. scene.wgsl fades lines toward it.
pub const NIGHT_BACKGROUND: [f32; 4] = [0.05, 0.06, 0.09, 1.0];
pub const GRID_COLOR: [f32; 4] = [0.84, 0.84, 0.84, 1.0];
pub const AXIS_X: [f32; 4] = [0.85, 0.15, 0.15, 1.0];
pub const AXIS_Y: [f32; 4] = [0.15, 0.65, 0.20, 1.0];
pub const AXIS_Z: [f32; 4] = [0.15, 0.30, 0.85, 1.0];

/// `c` mixed 60% toward `toward`.
fn faded(c: [f32; 4], toward: [f32; 4]) -> [f32; 4] {
    let mix = |a: f32, b: f32| a + (b - a) * 0.6;
    [
        mix(c[0], toward[0]),
        mix(c[1], toward[1]),
        mix(c[2], toward[2]),
        1.0,
    ]
}

/// Ground grid on z = 0 as line-list vertex pairs. Lines on the axes are omitted.
pub fn grid_lines(half_extent: f32, spacing: f32) -> Vec<LineVertex> {
    grid_lines_with(half_extent, spacing, false)
}

/// `grid_lines`, with the lines through the origin too when `through_origin` (for axes that
/// are not the world's: the grid then has no axes to leave a gap for).
fn grid_lines_with(half_extent: f32, spacing: f32, through_origin: bool) -> Vec<LineVertex> {
    assert!(spacing > 0.0 && half_extent >= spacing);
    let n = (half_extent / spacing).floor() as i32;
    let e = n as f32 * spacing;
    let v = |x: f32, y: f32| LineVertex {
        pos: [x, y, 0.0],
        color: GRID_COLOR,
    };
    let mut out = Vec::with_capacity(8 * n as usize + 4);
    for i in (-n..=n).filter(|&i| through_origin || i != 0) {
        let o = i as f32 * spacing;
        out.extend([v(o, -e), v(o, e), v(-e, o), v(e, o)]);
    }
    out
}

/// Red/green/blue axes from the origin; negative halves are faded.
pub fn axis_lines(length: f32) -> Vec<LineVertex> {
    axis_lines_in(length, CLASSIC_AXES, BACKGROUND)
}

/// The classic axis colors, red, green and blue.
const CLASSIC_AXES: [[f32; 4]; 3] = [AXIS_X, AXIS_Y, AXIS_Z];

/// The X, Y and Z axes from the origin in `colors`; negative halves mix 60% toward `toward`.
pub fn axis_lines_in(length: f32, colors: [[f32; 4]; 3], toward: [f32; 4]) -> Vec<LineVertex> {
    axis_lines_at(length, colors, toward, DAffine3::IDENTITY)
}

/// `axis_lines_in` for the drawing axes `frame` (origin and directions in world space): the
/// segments run from the frame's origin along its x, y and z axes.
pub fn axis_lines_at(
    length: f32,
    colors: [[f32; 4]; 3],
    toward: [f32; 4],
    frame: DAffine3,
) -> Vec<LineVertex> {
    let dirs = [DVec3::X, DVec3::Y, DVec3::Z];
    let world = frame == DAffine3::IDENTITY;
    let place = |p: DVec3| {
        if world { p } else { frame.transform_point3(p) }
            .as_vec3()
            .to_array()
    };
    let mut out = Vec::with_capacity(12);
    for (dir, color) in dirs.into_iter().zip(colors) {
        for (sign, c) in [(1.0, color), (-1.0, faded(color, toward))] {
            out.push(LineVertex {
                pos: place(DVec3::ZERO),
                color: c,
            });
            out.push(LineVertex {
                pos: place(dir * f64::from(length) * sign),
                color: c,
            });
        }
    }
    out
}

/// Lines lying flat on z = 0: the ground grid and the X and Y axes.
/// Drawn before faces, so faces always cover them even at grazing angles.
pub fn flat_ground_lines() -> Vec<LineVertex> {
    flat_ground_lines_in(CLASSIC_AXES, BACKGROUND)
}

/// `flat_ground_lines` with the axes in `colors`, their negative halves faded toward `toward`.
pub fn flat_ground_lines_in(colors: [[f32; 4]; 3], toward: [f32; 4]) -> Vec<LineVertex> {
    flat_ground_lines_at(colors, toward, DAffine3::IDENTITY)
}

/// `flat_ground_lines_in` with the axes of the drawing axes `frame`: the grid stays on the
/// world's z = 0 (its lines through the world origin are drawn when the axes are not the
/// world's), and the axis segments lying on z = 0 are drawn with it.
pub fn flat_ground_lines_at(
    colors: [[f32; 4]; 3],
    toward: [f32; 4],
    frame: DAffine3,
) -> Vec<LineVertex> {
    let mut lines = grid_lines_with(100.0, 1.0, frame != DAffine3::IDENTITY);
    lines.extend(flat_axis_lines_at(colors, toward, frame));
    lines
}

/// The X and Y axes alone, as in `flat_ground_lines_in`.
pub fn flat_axis_lines_in(colors: [[f32; 4]; 3], toward: [f32; 4]) -> Vec<LineVertex> {
    flat_axis_lines_at(colors, toward, DAffine3::IDENTITY)
}

/// Whether an axis segment lies on the ground, give or take what a moved origin's rounding
/// leaves (a micron); such segments are drawn flat, at z = 0 exactly.
fn is_flat(seg: &[LineVertex]) -> bool {
    seg[1].pos[2].abs() < 1e-6
}

/// The axis segments of `frame` lying on z = 0, as in `flat_ground_lines_at`.
pub fn flat_axis_lines_at(
    colors: [[f32; 4]; 3],
    toward: [f32; 4],
    frame: DAffine3,
) -> Vec<LineVertex> {
    // Axis segments come as (origin, end) pairs; keep the ones lying on z = 0.
    axis_lines_at(1000.0, colors, toward, frame)
        .chunks(2)
        .filter(|seg| is_flat(seg))
        .flatten()
        .map(|v| LineVertex {
            pos: [v.pos[0], v.pos[1], 0.0],
            ..*v
        })
        .collect()
}

/// The Z axis segments only, drawn with the depth bias that keeps them visible against faces.
pub fn vertical_axis_lines() -> Vec<LineVertex> {
    vertical_axis_lines_in(CLASSIC_AXES, BACKGROUND)
}

/// `vertical_axis_lines` in `colors`, the negative half faded toward `toward`.
pub fn vertical_axis_lines_in(colors: [[f32; 4]; 3], toward: [f32; 4]) -> Vec<LineVertex> {
    vertical_axis_lines_at(colors, toward, DAffine3::IDENTITY)
}

/// The axis segments of `frame` not lying on z = 0, as in `vertical_axis_lines_in`.
pub fn vertical_axis_lines_at(
    colors: [[f32; 4]; 3],
    toward: [f32; 4],
    frame: DAffine3,
) -> Vec<LineVertex> {
    axis_lines_at(1000.0, colors, toward, frame)
        .chunks(2)
        .filter(|seg| !is_flat(seg))
        .flatten()
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_has_two_lines_per_nonzero_offset() {
        // offsets -10..=10 excluding 0 -> 20 offsets, 2 lines each, 2 vertices per line
        assert_eq!(grid_lines(10.0, 1.0).len(), 80);
    }

    #[test]
    fn grid_lies_on_ground_and_spans_extent() {
        let v = grid_lines(10.0, 1.0);
        assert!(v.iter().all(|p| p.pos[2] == 0.0));
        assert!(
            v.iter()
                .all(|p| p.pos[0].abs() <= 10.0 && p.pos[1].abs() <= 10.0)
        );
        assert!(v.iter().any(|p| p.pos[0] == 10.0) && v.iter().any(|p| p.pos[1] == -10.0));
    }

    #[test]
    fn grid_skips_axis_lines() {
        for seg in grid_lines(10.0, 1.0).chunks(2) {
            let (a, b) = (seg[0].pos, seg[1].pos);
            assert!(!(a[0] == 0.0 && b[0] == 0.0), "line on x = 0");
            assert!(!(a[1] == 0.0 && b[1] == 0.0), "line on y = 0");
        }
    }

    #[test]
    fn flat_ground_is_on_z0_and_vertical_axis_is_only_z() {
        let flat = flat_ground_lines();
        assert!(flat.iter().all(|v| v.pos[2] == 0.0));
        // grid plus the X and Y axes, each as a positive and a negative half (8 vertices)
        assert_eq!(flat.len(), grid_lines(100.0, 1.0).len() + 8);

        let vertical = vertical_axis_lines();
        assert_eq!(vertical.len(), 4);
        assert!(vertical.iter().all(|v| v.pos[0] == 0.0 && v.pos[1] == 0.0));
    }

    /// Bit patterns, so a sign or rounding change shows.
    fn bits(v: &[LineVertex]) -> Vec<[u32; 7]> {
        v.iter()
            .map(|v| {
                let [x, y, z] = v.pos.map(f32::to_bits);
                let [r, g, b, a] = v.color.map(f32::to_bits);
                [x, y, z, r, g, b, a]
            })
            .collect()
    }

    #[test]
    fn the_classic_axes_are_unchanged() {
        // The vertices `axis_lines(1000.0)` gave before the axes took a style's colors.
        let x = |x: f32, y: f32, z: f32| [x, y, z];
        let expected: Vec<LineVertex> = [
            (x(0.0, 0.0, 0.0), [0.85, 0.15, 0.15, 1.0]),
            (x(1000.0, 0.0, 0.0), [0.85, 0.15, 0.15, 1.0]),
            (x(0.0, 0.0, 0.0), [0.916, 0.636, 0.636, 1.0]),
            (x(-1000.0, -0.0, -0.0), [0.916, 0.636, 0.636, 1.0]),
            (x(0.0, 0.0, 0.0), [0.15, 0.65, 0.2, 1.0]),
            (x(0.0, 1000.0, 0.0), [0.15, 0.65, 0.2, 1.0]),
            (x(0.0, 0.0, 0.0), [0.636, 0.83599997, 0.656, 1.0]),
            (x(-0.0, -1000.0, -0.0), [0.636, 0.83599997, 0.656, 1.0]),
            (x(0.0, 0.0, 0.0), [0.15, 0.3, 0.85, 1.0]),
            (x(0.0, 0.0, 1000.0), [0.15, 0.3, 0.85, 1.0]),
            (x(0.0, 0.0, 0.0), [0.636, 0.696, 0.916, 1.0]),
            (x(-0.0, -0.0, -1000.0), [0.636, 0.696, 0.916, 1.0]),
        ]
        .map(|(pos, color)| LineVertex { pos, color })
        .to_vec();
        let classic = [AXIS_X, AXIS_Y, AXIS_Z];
        assert_eq!(bits(&axis_lines(1000.0)), bits(&expected));
        assert_eq!(
            bits(&axis_lines_in(1000.0, classic, BACKGROUND)),
            bits(&expected)
        );
        let mut flat = grid_lines(100.0, 1.0);
        flat.extend(expected.chunks(2).take(4).flatten());
        assert_eq!(bits(&flat_ground_lines()), bits(&flat));
        assert_eq!(
            bits(&flat_ground_lines_in(classic, BACKGROUND)),
            bits(&flat)
        );
        let vertical = &expected[8..];
        assert_eq!(bits(&vertical_axis_lines()), bits(vertical));
        assert_eq!(
            bits(&vertical_axis_lines_in(classic, BACKGROUND)),
            bits(vertical)
        );
        assert_eq!(
            bits(&flat_axis_lines_in(classic, BACKGROUND)),
            bits(&expected[..8])
        );
    }

    #[test]
    fn axes_have_full_positive_and_faded_negative_halves() {
        let v = axis_lines(5.0);
        assert_eq!(v.len(), 12);
        let pos_x = v.chunks(2).find(|s| s[1].pos == [5.0, 0.0, 0.0]).unwrap();
        assert_eq!(pos_x[1].color, AXIS_X);
        let neg_x = v.chunks(2).find(|s| s[1].pos == [-5.0, 0.0, 0.0]).unwrap();
        assert_ne!(neg_x[1].color, AXIS_X);
        assert!(v.chunks(2).all(|s| s[0].pos == [0.0, 0.0, 0.0]));
    }

    #[test]
    fn axes_a_hair_off_the_ground_still_lie_flat() {
        let frame = DAffine3::from_translation(DVec3::new(2.0, 3.0, 1e-9));
        let flat = flat_axis_lines_at([AXIS_X, AXIS_Y, AXIS_Z], BACKGROUND, frame);
        assert_eq!(flat.len(), 8);
        assert!(flat.iter().all(|v| v.pos[2] == 0.0), "snapped to z = 0");
        let vertical = vertical_axis_lines_at([AXIS_X, AXIS_Y, AXIS_Z], BACKGROUND, frame);
        assert_eq!(vertical.len(), 4);
    }

    #[test]
    fn moved_axes_start_at_their_origin_and_split_by_the_ground() {
        // Red along world y, green along world -x, from (2, 3, 1): one meter up, so no axis
        // lies flat on z = 0 but the blue one is the only one leaving the plane z = 1.
        let frame = DAffine3::from_mat3_translation(
            glam::DMat3::from_cols(DVec3::Y, -DVec3::X, DVec3::Z),
            DVec3::new(2.0, 3.0, 1.0),
        );
        let lines = axis_lines_at(10.0, [AXIS_X, AXIS_Y, AXIS_Z], BACKGROUND, frame);
        assert_eq!(lines.len(), 12);
        assert_eq!(lines[0].pos, [2.0, 3.0, 1.0]);
        assert_eq!(lines[1].pos, [2.0, 13.0, 1.0], "red runs along world y");
        assert_eq!(lines[5].pos, [-8.0, 3.0, 1.0], "green runs along world -x");
        let flat = flat_axis_lines_at([AXIS_X, AXIS_Y, AXIS_Z], BACKGROUND, frame);
        assert!(flat.is_empty());
        assert_eq!(
            vertical_axis_lines_at([AXIS_X, AXIS_Y, AXIS_Z], BACKGROUND, frame).len(),
            12
        );
        // The grid keeps its lines through the world origin when the axes are elsewhere.
        let moved = flat_ground_lines_at([AXIS_X, AXIS_Y, AXIS_Z], BACKGROUND, frame);
        let world = flat_ground_lines_in([AXIS_X, AXIS_Y, AXIS_Z], BACKGROUND);
        // Four more vertices for the lines through the origin, eight fewer for the flat axes.
        assert_eq!(moved.len() + 4, world.len());
        // World axes: unchanged.
        let id = flat_ground_lines_at([AXIS_X, AXIS_Y, AXIS_Z], BACKGROUND, DAffine3::IDENTITY);
        assert_eq!(id, world);
    }
}
