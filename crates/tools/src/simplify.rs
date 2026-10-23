//! Douglas-Peucker polyline simplification, for the Freehand tool.

use glam::DVec3;

/// The distance from `p` to the segment `a`-`b` (to `a` when they coincide).
pub(crate) fn segment_distance(p: DVec3, a: DVec3, b: DVec3) -> f64 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < 1e-24 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// The points of `path` that keep its shape within `tolerance` (model units): the ends, and
/// recursively the point farthest from the segment between the kept points around it while that
/// is more than `tolerance` away. Iterative, so a long path does not deepen the stack.
pub fn douglas_peucker(path: &[DVec3], tolerance: f64) -> Vec<DVec3> {
    let n = path.len();
    if n <= 2 {
        return path.to_vec();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut ranges = vec![(0, n - 1)];
    while let Some((lo, hi)) = ranges.pop() {
        let (mut far, mut far_distance) = (lo, 0.0);
        for (i, p) in path.iter().enumerate().take(hi).skip(lo + 1) {
            let d = segment_distance(*p, path[lo], path[hi]);
            if d > far_distance {
                (far, far_distance) = (i, d);
            }
        }
        if far_distance > tolerance {
            keep[far] = true;
            ranges.push((lo, far));
            ranges.push((far, hi));
        }
    }
    path.iter()
        .zip(&keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn p(x: f64, y: f64) -> DVec3 {
        DVec3::new(x, y, 0.0)
    }

    #[test]
    fn collinear_points_collapse_to_the_ends() {
        let line: Vec<DVec3> = (0..=20).map(|i| p(i as f64 * 0.1, 0.0)).collect();
        assert_eq!(douglas_peucker(&line, 0.01), vec![line[0], line[20]]);
    }

    #[test]
    fn a_corner_is_kept() {
        let mut path: Vec<DVec3> = (0..=10).map(|i| p(i as f64 * 0.2, 0.0)).collect();
        path.extend((1..=10).map(|i| p(2.0, i as f64 * 0.2)));
        assert_eq!(
            douglas_peucker(&path, 0.01),
            vec![p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0)]
        );
    }

    #[test]
    fn every_dropped_point_stays_within_the_tolerance() {
        let path: Vec<DVec3> = (0..500)
            .map(|i| {
                let t = i as f64 * 0.05;
                p(t, (t * 1.3).sin() + 0.2 * (t * 7.0).sin())
            })
            .collect();
        let tol = 0.05;
        let kept = douglas_peucker(&path, tol);
        assert!(kept.len() < path.len() / 2);
        assert_eq!((kept[0], *kept.last().unwrap()), (path[0], path[499]));
        for q in &path {
            let d = kept
                .windows(2)
                .map(|w| segment_distance(*q, w[0], w[1]))
                .fold(f64::INFINITY, f64::min);
            assert!(d <= tol + 1e-12, "{q:?} is {d} from the result");
        }
    }

    #[test]
    fn a_closed_path_keeps_its_far_side() {
        let mut ring: Vec<DVec3> = (0..=100)
            .map(|i| {
                let t = std::f64::consts::TAU * i as f64 / 100.0;
                p(t.cos(), t.sin())
            })
            .collect();
        *ring.last_mut().unwrap() = ring[0];
        let kept = douglas_peucker(&ring, 0.01);
        assert!(kept.len() > 8, "{}", kept.len());
        assert_eq!(kept[0], *kept.last().unwrap());
    }

    #[test]
    fn short_inputs_come_back_unchanged() {
        assert!(douglas_peucker(&[], 0.1).is_empty());
        assert_eq!(douglas_peucker(&[p(1.0, 1.0)], 0.1), vec![p(1.0, 1.0)]);
        let two = [p(0.0, 0.0), p(1.0, 0.0)];
        assert_eq!(douglas_peucker(&two, 0.1), two.to_vec());
    }

    #[test]
    fn ten_thousand_samples_are_fast() {
        let path: Vec<DVec3> = (0..10_000)
            .map(|i| {
                let t = i as f64 * 0.01;
                let r = 1.0 + t * 0.02 + 0.003 * (t * 50.0).sin();
                p(r * t.cos(), r * t.sin())
            })
            .collect();
        let start = Instant::now();
        let kept = douglas_peucker(&path, 0.002);
        assert!(start.elapsed().as_secs_f64() < 2.0, "{:?}", start.elapsed());
        assert!(kept.len() > 50 && kept.len() < path.len());
    }
}
