//! Section fill in rooms and solids made of voxel parts: a room stays empty however its walls
//! are split into parts, and parts that share single faces are filled.

use glam::DVec3;
use kernel::{section_loops, triangulate_section};

fn fill_area(tris: &[[DVec3; 3]], n: DVec3) -> f64 {
    tris.iter()
        .map(|t| (t[1] - t[0]).cross(t[2] - t[0]).dot(n) / 2.0)
        .sum()
}

fn area(tris: &[[DVec3; 3]], point: DVec3, normal: DVec3) -> f64 {
    let r = section_loops(tris, point, normal);
    fill_area(&triangulate_section(&r.closed, normal), normal)
}

/// Voxels `part(i, j, k)` (0: empty). A face between two cells of different parts exists once:
/// between solid and empty its normal points out of the solid; between two solids it points
/// along +axis, or -axis with `flip_iface`.
fn voxels(n: [i32; 3], part: &dyn Fn(i32, i32, i32) -> u32, flip_iface: bool) -> Vec<[DVec3; 3]> {
    let get = |c: [i32; 3]| {
        if (0..3).all(|k| c[k] >= 0 && c[k] < n[k]) {
            part(c[0], c[1], c[2])
        } else {
            0
        }
    };
    let mut tris = Vec::new();
    for axis in 0..3 {
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        for level in 0..=n[axis] {
            for i in 0..n[a] {
                for j in 0..n[b] {
                    let mut lo = [0; 3];
                    lo[axis] = level - 1;
                    lo[a] = i;
                    lo[b] = j;
                    let mut hi = lo;
                    hi[axis] = level;
                    let (pa, pb) = (get(lo), get(hi));
                    if pa == pb {
                        continue;
                    }
                    let plus = if pa != 0 && pb != 0 {
                        !flip_iface
                    } else {
                        pa != 0
                    };
                    let at = |u: i32, v: i32| {
                        let mut c = [0.0; 3];
                        c[axis] = level as f64;
                        c[a] = u as f64;
                        c[b] = v as f64;
                        DVec3::from_array(c)
                    };
                    let q = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
                    if plus {
                        tris.push([q[0], q[1], q[2]]);
                        tris.push([q[0], q[2], q[3]]);
                    } else {
                        tris.push([q[0], q[2], q[1]]);
                        tris.push([q[0], q[3], q[2]]);
                    }
                }
            }
        }
    }
    tris
}

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn is(name: &str, got: f64, want: f64) {
    assert!((got - want).abs() < 1e-6, "{name}: got {got}, want {want}");
}

fn shell(i: i32, j: i32, k: i32, s: i32) -> bool {
    i == 0 || j == 0 || k == 0 || i == s - 1 || j == s - 1 || k == s - 1
}

#[test]
fn a_room_stays_empty_however_its_walls_are_split_into_parts() {
    let z = |t: &[[DVec3; 3]], h: f64| area(t, p(0., 0., h), DVec3::Z);
    let t = voxels([5, 5, 5], &|i, j, k| shell(i, j, k, 5) as u32, false);
    is("manifold, horizontal", z(&t, 2.5), 16.0);
    is(
        "manifold, vertical",
        area(&t, p(2.5, 0., 0.), DVec3::X),
        16.0,
    );
    let parts = |i: i32, j: i32, k: i32| -> u32 {
        match (k, i, j) {
            (0, ..) => 1,
            (4, ..) => 2,
            (_, 0, _) => 3,
            (_, 4, _) => 4,
            (_, _, 0) => 5,
            (_, _, 4) => 6,
            _ => 0,
        }
    };
    for flip in [false, true] {
        let t = voxels([5, 5, 5], &parts, flip);
        is("walls as separate boxes", z(&t, 2.5), 16.0);
        is(
            "walls as separate boxes, vertical",
            area(&t, p(2.5, 0., 0.), DVec3::X),
            16.0,
        );
    }
}

#[test]
fn a_partition_a_pillar_and_furniture_leave_the_rooms_empty() {
    let two = |i: i32, j: i32, k: i32| {
        (i == 0 || i == 3 || i == 6 || j == 0 || j == 4 || k == 0 || k == 4) as u32
    };
    let t = voxels([7, 5, 5], &two, false);
    is(
        "two rooms, manifold",
        area(&t, p(0., 0., 2.5), DVec3::Z),
        23.0,
    );
    let two_p = |i: i32, j: i32, k: i32| -> u32 {
        match (k, i) {
            (0, _) => 1,
            (4, _) => 2,
            (_, 3) => 7,
            _ => two(i, j, k) * 3,
        }
    };
    let t = voxels([7, 5, 5], &two_p, false);
    is(
        "partition as a separate box",
        area(&t, p(0., 0., 2.5), DVec3::Z),
        23.0,
    );

    let furn = |i: i32, j: i32, k: i32| -> u32 {
        if shell(i, j, k, 5) {
            1
        } else if i == 2 && j == 2 && k == 1 {
            2
        } else {
            0
        }
    };
    let t = voxels([5, 5, 5], &furn, false);
    is(
        "box on the floor, cut through it",
        area(&t, p(0., 0., 1.5), DVec3::Z),
        17.0,
    );
    is(
        "box on the floor, cut above it",
        area(&t, p(0., 0., 2.5), DVec3::Z),
        16.0,
    );
    is(
        "box on the floor, vertical",
        area(&t, p(2.5, 0., 0.), DVec3::X),
        17.0,
    );
    let pil = |i: i32, j: i32, k: i32| -> u32 {
        if shell(i, j, k, 5) {
            1
        } else if i == 2 && j == 2 {
            2
        } else {
            0
        }
    };
    let t = voxels([5, 5, 5], &pil, false);
    is("pillar", area(&t, p(0., 0., 2.5), DVec3::Z), 17.0);
    is("pillar, vertical", area(&t, p(2.5, 0., 0.), DVec3::X), 19.0);
    let fl = |i: i32, j: i32, k: i32| (shell(i, j, k, 5) || (i == 2 && j == 2 && k == 2)) as u32;
    let t = voxels([5, 5, 5], &fl, false);
    is("floating box", area(&t, p(0., 0., 2.5), DVec3::Z), 17.0);
}

#[test]
fn courtyards_rings_and_edge_touching_cubes() {
    let u = |i: i32, j: i32, k: i32| -> u32 {
        match (k, i, j) {
            (0, ..) => 1,
            (_, 0, _) => 2,
            (_, 4, _) => 3,
            (_, _, 0) => 4,
            _ => 0,
        }
    };
    let t = voxels([5, 5, 3], &u, false);
    is("U shape", area(&t, p(0., 0., 1.5), DVec3::Z), 13.0);
    let c = |i: i32, j: i32, k: i32| -> u32 {
        match (k, i, j) {
            (0, ..) => 1,
            (_, 0, _) => 2,
            (_, 4, _) => 3,
            (_, _, 0) => 4,
            (_, _, 4) => 5,
            _ => 0,
        }
    };
    let t = voxels([5, 5, 3], &c, false);
    is("open courtyard", area(&t, p(0., 0., 1.5), DVec3::Z), 16.0);
    let ring = |i: i32, j: i32, _k: i32| (i == 0 || j == 0 || i == 4 || j == 4) as u32;
    let t = voxels([5, 5, 1], &ring, false);
    is("ring", area(&t, p(0., 0., 0.5), DVec3::Z), 16.0);
    let bars = |i: i32, j: i32, _k: i32| -> u32 {
        match (i, j) {
            (0, _) => 1,
            (4, _) => 2,
            (_, 0) => 3,
            (_, 4) => 4,
            _ => 0,
        }
    };
    let t = voxels([5, 5, 1], &bars, false);
    is("ring of bars", area(&t, p(0., 0., 0.5), DVec3::Z), 16.0);
    let e = |i: i32, j: i32, _k: i32| -> u32 {
        match (i, j) {
            (0, 0) => 1,
            (1, 1) => 2,
            _ => 0,
        }
    };
    let t = voxels([2, 2, 1], &e, false);
    is("cubes at an edge", area(&t, p(0., 0., 0.5), DVec3::Z), 2.0);
    let plus = |i: i32, j: i32, k: i32| -> u32 {
        if k == 0 {
            1
        } else if k == 2 {
            2
        } else if (i == 1) != (j == 1) {
            3 + (i * 3 + j) as u32
        } else {
            0
        }
    };
    let t = voxels([3, 3, 3], &plus, false);
    is(
        "four cubes round an empty cell",
        area(&t, p(0., 0., 1.5), DVec3::Z),
        4.0,
    );
}

#[test]
fn grids_of_parts_are_filled_but_a_missing_cell_stays_empty() {
    let g = |miss: Option<(i32, i32, i32)>| {
        move |i: i32, j: i32, k: i32| -> u32 {
            if Some((i, j, k)) == miss {
                0
            } else {
                1 + (i * 9 + j * 3 + k) as u32
            }
        }
    };
    for flip in [false, true] {
        let t = voxels([3, 3, 3], &g(None), flip);
        is("full grid", area(&t, p(0., 0., 1.5), DVec3::Z), 9.0);
    }
    for miss in [(1, 1, 1), (0, 0, 1), (1, 0, 1)] {
        let t = voxels([3, 3, 3], &g(Some(miss)), false);
        is(
            &format!("grid missing {miss:?}"),
            area(&t, p(0., 0., 1.5), DVec3::Z),
            8.0,
        );
    }
    let t = voxels([1, 1, 1], &|_, _, _| 1, false);
    is("one box", area(&t, p(0., 0., 0.5), DVec3::Z), 1.0);
    let t = voxels([2, 1, 1], &|i, _, _| 1 + i as u32, false);
    is("two boxes", area(&t, p(0., 0., 0.5), DVec3::Z), 2.0);
}
