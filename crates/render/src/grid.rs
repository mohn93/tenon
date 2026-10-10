use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LineVertex {
    pub pos: [f32; 3],
    pub color: [f32; 4],
}

// sRGB-authored colors.
pub const BACKGROUND: [f32; 4] = [0.96, 0.96, 0.96, 1.0];
pub const GRID_COLOR: [f32; 4] = [0.84, 0.84, 0.84, 1.0];
pub const AXIS_X: [f32; 4] = [0.85, 0.15, 0.15, 1.0];
pub const AXIS_Y: [f32; 4] = [0.15, 0.65, 0.20, 1.0];
pub const AXIS_Z: [f32; 4] = [0.15, 0.30, 0.85, 1.0];

fn faded(c: [f32; 4]) -> [f32; 4] {
    let mix = |a: f32, b: f32| a + (b - a) * 0.6;
    [mix(c[0], BACKGROUND[0]), mix(c[1], BACKGROUND[1]), mix(c[2], BACKGROUND[2]), 1.0]
}

/// Ground grid on z = 0 as line-list vertex pairs. Lines on the axes are omitted.
pub fn grid_lines(half_extent: f32, spacing: f32) -> Vec<LineVertex> {
    assert!(spacing > 0.0 && half_extent >= spacing);
    let n = (half_extent / spacing).floor() as i32;
    let e = n as f32 * spacing;
    let v = |x: f32, y: f32| LineVertex { pos: [x, y, 0.0], color: GRID_COLOR };
    let mut out = Vec::with_capacity(8 * n as usize);
    for i in (-n..=n).filter(|&i| i != 0) {
        let o = i as f32 * spacing;
        out.extend([v(o, -e), v(o, e), v(-e, o), v(e, o)]);
    }
    out
}

/// Red/green/blue axes from the origin; negative halves are faded.
pub fn axis_lines(length: f32) -> Vec<LineVertex> {
    let axes = [([1.0, 0.0, 0.0], AXIS_X), ([0.0, 1.0, 0.0], AXIS_Y), ([0.0, 0.0, 1.0], AXIS_Z)];
    let mut out = Vec::with_capacity(12);
    for (dir, color) in axes {
        for (sign, c) in [(1.0, color), (-1.0, faded(color))] {
            let end = [dir[0] * length * sign, dir[1] * length * sign, dir[2] * length * sign];
            out.push(LineVertex { pos: [0.0, 0.0, 0.0], color: c });
            out.push(LineVertex { pos: end, color: c });
        }
    }
    out
}

/// Everything the M1 viewport draws.
pub fn scene_lines() -> Vec<LineVertex> {
    let mut lines = grid_lines(100.0, 1.0);
    lines.extend(axis_lines(1000.0));
    lines
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
        assert!(v.iter().all(|p| p.pos[0].abs() <= 10.0 && p.pos[1].abs() <= 10.0));
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
    fn axes_have_full_positive_and_faded_negative_halves() {
        let v = axis_lines(5.0);
        assert_eq!(v.len(), 12);
        let pos_x = v.chunks(2).find(|s| s[1].pos == [5.0, 0.0, 0.0]).unwrap();
        assert_eq!(pos_x[1].color, AXIS_X);
        let neg_x = v.chunks(2).find(|s| s[1].pos == [-5.0, 0.0, 0.0]).unwrap();
        assert_ne!(neg_x[1].color, AXIS_X);
        assert!(v.chunks(2).all(|s| s[0].pos == [0.0, 0.0, 0.0]));
    }
}
