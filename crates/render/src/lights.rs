//! Point lights from glowing faces.
//!
//! Each placement of an instance whose own faces glow (wherever it is placed, at any depth) is
//! one light: at the area-weighted center of those faces, colored like them, as strong as their
//! emission times their area. The loose geometry's glowing faces (the root's) give one light per
//! connected group of them (faces sharing a vertex or an edge), so lamps drawn loose in two
//! rooms light each room rather than one light between them. Faces of nested instances belong
//! to the instances' own lights, so a lamp deep inside a house still gives its own. What a mesh
//! gives is worked out once per mesh and inherited material (see `Glow`), however often it is
//! placed.

use std::collections::HashMap;

use glam::{DAffine3, DVec3};
use kernel::VertexId;

/// The most lights the renderer uses at once: the strongest at the camera (see `strongest`).
pub const MAX_LIGHTS: usize = 128;

/// How far a light of strength 1 (1 m² glowing at emission 1) reaches, in meters; the reach
/// grows with the square root of the strength.
pub const LIGHT_REACH: f64 = 6.0;

/// A point light in root coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub position: DVec3,
    /// sRGB, 0 to 1: the glowing faces' colors, weighted by emission and area.
    pub color: [f32; 3],
    /// The faces' emission times their area in square meters, summed.
    pub strength: f64,
}

impl Light {
    /// How far the light reaches, in meters: its light falls to zero there.
    pub fn range(&self) -> f64 {
        LIGHT_REACH * self.strength.sqrt()
    }
}

/// What a mesh's own glowing faces give, in the mesh's coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Glow {
    /// Sums over the glowing faces: area times center, area, emission times area times color,
    /// emission times area.
    weighted_center: DVec3,
    area: f64,
    weighted_color: DVec3,
    strength: f64,
}

impl Glow {
    /// Adds a triangle glowing at `emission` in `color` (sRGB, 0 to 1).
    pub(crate) fn add_triangle(&mut self, [a, b, c]: [DVec3; 3], emission: f32, color: [f32; 3]) {
        let area = 0.5 * (b - a).cross(c - a).length();
        if emission <= 0.0 || area <= 0.0 {
            return;
        }
        let strength = f64::from(emission) * area;
        self.weighted_center += area * (a + b + c) / 3.0;
        self.area += area;
        self.weighted_color += strength * DVec3::from(color.map(f64::from));
        self.strength += strength;
    }

    /// Whether anything glows.
    pub(crate) fn glows(&self) -> bool {
        self.strength > 0.0
    }

    /// Adds another glow's faces to this one.
    pub(crate) fn add(&mut self, other: &Glow) {
        self.weighted_center += other.weighted_center;
        self.area += other.area;
        self.weighted_color += other.weighted_color;
        self.strength += other.strength;
    }

    /// The light this glow gives placed by `t` (None when nothing glows). The strength grows
    /// with the area a scaling placement gives the faces.
    pub(crate) fn placed(&self, t: DAffine3) -> Option<Light> {
        if self.strength <= 0.0 {
            return None;
        }
        let scale = t.matrix3.determinant().abs().powf(2.0 / 3.0);
        Some(Light {
            position: t.transform_point3(self.weighted_center / self.area),
            color: (self.weighted_color / self.strength).as_vec3().to_array(),
            strength: self.strength * scale,
        })
    }
}

/// Glowing faces' glows, each with the vertices of its face, joined into one glow per connected
/// group: faces sharing a vertex (so also those sharing an edge) are one group. In the order of
/// each group's first face.
pub(crate) fn connected(faces: &[(Glow, Vec<VertexId>)]) -> Vec<Glow> {
    fn root(parent: &mut [usize], mut k: usize) -> usize {
        while parent[k] != k {
            parent[k] = parent[parent[k]];
            k = parent[k];
        }
        k
    }
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    let mut first: HashMap<VertexId, usize> = HashMap::new();
    for (k, (_, vertices)) in faces.iter().enumerate() {
        for v in vertices {
            let other = *first.entry(*v).or_insert(k);
            let (a, b) = (root(&mut parent, k), root(&mut parent, other));
            // The earlier face roots the group, so groups keep their first face's order.
            parent[a.max(b)] = a.min(b);
        }
    }
    let mut groups: Vec<Glow> = Vec::new();
    let mut slot: HashMap<usize, usize> = HashMap::new();
    for (k, (glow, _)) in faces.iter().enumerate() {
        let r = root(&mut parent, k);
        let at = *slot.entry(r).or_insert_with(|| {
            groups.push(Glow::default());
            groups.len() - 1
        });
        groups[at].add(glow);
    }
    groups
}

/// At most `n` of `lights`, the strongest as seen from `eye`: by strength over (1 + d²), d being
/// the light's distance from the eye. With `n` or fewer lights, all of them, in order.
pub fn strongest(lights: &[Light], eye: DVec3, n: usize) -> Vec<Light> {
    if lights.len() <= n {
        return lights.to_vec();
    }
    let score = |l: &Light| l.strength / (1.0 + l.position.distance_squared(eye));
    let mut ranked: Vec<&Light> = lights.iter().collect();
    ranked.sort_by(|a, b| score(b).total_cmp(&score(a)));
    ranked.into_iter().take(n).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(x: f64, strength: f64) -> Light {
        Light {
            position: DVec3::new(x, 0.0, 0.0),
            color: [1.0; 3],
            strength,
        }
    }

    #[test]
    fn a_glow_is_centered_on_its_area_and_colored_by_its_strength() {
        let mut g = Glow::default();
        assert_eq!(g.placed(DAffine3::IDENTITY), None);
        // A 2 m² red triangle at emission 1 and a 1 m² blue one at emission 4, apart along x.
        let tri = |x: f64, size: f64| {
            [
                DVec3::new(x, 0.0, 0.0),
                DVec3::new(x + 2.0 * size, 0.0, 0.0),
                DVec3::new(x, 2.0 * size, 0.0),
            ]
        };
        g.add_triangle(tri(0.0, 1.0), 1.0, [1.0, 0.0, 0.0]);
        g.add_triangle(tri(10.0, 0.5_f64.sqrt()), 4.0, [0.0, 0.0, 1.0]);
        g.add_triangle(tri(100.0, 1.0), 0.0, [0.0, 1.0, 0.0]);
        let l = g.placed(DAffine3::IDENTITY).unwrap();
        assert!((l.strength - 6.0).abs() < 1e-9);
        // Centers (2/3, 2/3) and (10 + 0.47, 0.47), weighted 2 : 1 by area.
        let s = 0.5_f64.sqrt() * 2.0 / 3.0;
        let expected =
            (2.0 * DVec3::new(2.0 / 3.0, 2.0 / 3.0, 0.0) + DVec3::new(10.0 + s, s, 0.0)) / 3.0;
        assert!(l.position.distance(expected) < 1e-9, "{}", l.position);
        // Red 2 : blue 4 by emission times area.
        assert!((l.color[0] - 1.0 / 3.0).abs() < 1e-6 && (l.color[2] - 2.0 / 3.0).abs() < 1e-6);
        assert!((l.range() - LIGHT_REACH * 6.0_f64.sqrt()).abs() < 1e-9);

        // Placed moved and doubled in size: four times the area.
        let t = DAffine3::from_translation(DVec3::Z) * DAffine3::from_scale(DVec3::splat(2.0));
        let placed = g.placed(t).unwrap();
        assert!((placed.strength - 24.0).abs() < 1e-9);
        assert!(placed.position.distance(2.0 * expected + DVec3::Z) < 1e-9);
    }

    #[test]
    fn the_strongest_lights_at_the_eye_are_kept() {
        let lights = [light(1.0, 1.0), light(10.0, 1.0), light(10.0, 1000.0)];
        assert_eq!(strongest(&lights, DVec3::ZERO, 3), lights.to_vec());
        // Near and weak (1 / 2) loses to far and strong (1000 / 101); far and weak goes.
        assert_eq!(
            strongest(&lights, DVec3::ZERO, 2),
            vec![lights[2], lights[0]]
        );
        assert_eq!(
            strongest(&lights, DVec3::new(10.0, 0.0, 0.0), 1),
            vec![lights[2]]
        );
    }
}
