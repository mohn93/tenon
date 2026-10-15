//! Shadows of the lights (see `lights`): the strongest lights at the camera each get a cube
//! shadow map, a layer of one cube-map array, holding how far the nearest caster is in every
//! direction. A map is kept between frames and drawn again only when the casters change or
//! its slot is given to another light (see `ShadowSlots`).

use glam::{Mat4, Vec3, Vec4};

use crate::lights::Light;

/// The most lights that cast shadows: the strongest at the camera of those lit.
pub const MAX_SHADOWS: usize = 64;

/// The side of a cube shadow map's faces, in pixels.
pub const SHADOW_SIZE: u32 = 256;

/// Shadow maps hold the distance to the nearest caster over the light's range, 0 to 1.
pub const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;

/// How near a caster can come to its light, in meters, and still cast.
pub const SHADOW_NEAR: f32 = 0.02;

/// A light as its shadow map was drawn for it: where it is and how far it reaches, as the
/// renderer uploads them. A light that keeps both keeps its map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowKey {
    pub position: [f32; 3],
    pub range: f32,
}

impl ShadowKey {
    pub fn of(light: &Light) -> Self {
        Self {
            position: light.position.as_vec3().to_array(),
            range: light.range() as f32,
        }
    }
}

/// Which slot of the cube-map array holds which light's shadow map, and for which casters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShadowSlots {
    /// What each slot holds: the light and the casters' generation it was drawn for.
    held: Vec<Option<(ShadowKey, u64)>>,
}

/// What `ShadowSlots::assign` decided for a frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Assignment {
    /// Each shadowed light's slot, in the order given.
    pub slots: Vec<usize>,
    /// The slots to draw again, in increasing order.
    pub redraw: Vec<usize>,
}

impl ShadowSlots {
    /// `capacity` empty slots.
    pub fn new(capacity: usize) -> Self {
        Self {
            held: vec![None; capacity],
        }
    }

    pub fn capacity(&self) -> usize {
        self.held.len()
    }

    /// Gives each of `lights` (at most the capacity) a slot for casters of `generation`. A
    /// light keeps the slot already holding it; the others take slots no light of this frame
    /// holds, empty ones first. A slot is drawn again when it changes hands or was drawn for
    /// other casters. Slots left over keep their maps, so a light that comes back finds its
    /// map unless the slot was taken meanwhile.
    pub fn assign(&mut self, lights: &[ShadowKey], generation: u64) -> Assignment {
        assert!(
            lights.len() <= self.capacity(),
            "more shadowed lights than slots"
        );
        let mut slots: Vec<Option<usize>> = vec![None; lights.len()];
        let mut taken = vec![false; self.capacity()];
        // Lights already held keep their slots (a light given twice shares one).
        for (k, key) in lights.iter().enumerate() {
            if let Some(s) = self
                .held
                .iter()
                .position(|h| h.is_some_and(|(h, _)| h == *key))
            {
                slots[k] = Some(s);
                taken[s] = true;
            }
        }
        for k in 0..lights.len() {
            if slots[k].is_some() {
                continue;
            }
            if let Some(j) = (0..k).find(|&j| lights[j] == lights[k]) {
                slots[k] = slots[j];
                continue;
            }
            let free = (0..self.capacity())
                .filter(|&s| !taken[s])
                .min_by_key(|&s| self.held[s].is_some())
                .expect("a free slot");
            slots[k] = Some(free);
            taken[free] = true;
        }
        let slots: Vec<usize> = slots.into_iter().map(|s| s.expect("assigned")).collect();
        let mut redraw = Vec::new();
        for (key, &s) in lights.iter().zip(&slots) {
            if self.held[s] != Some((*key, generation)) {
                self.held[s] = Some((*key, generation));
                redraw.push(s);
            }
        }
        redraw.sort_unstable();
        redraw.dedup();
        Assignment { slots, redraw }
    }
}

/// How many slots to make room for `needed` shadowed lights: the next power of two, at most
/// `max`, so a scene gaining lights reallocates the array only a few times.
pub fn capacity_for(needed: usize, max: usize) -> usize {
    needed.max(1).next_power_of_two().min(max)
}

/// The cube faces in array-layer order (+x, -x, +y, -y, +z, -z): the face's axis, then the
/// directions a lookup reads as the face's s (right) and t (down) coordinates, as WebGPU
/// picks cube faces.
const CUBE_FACES: [[Vec3; 3]; 6] = [
    [Vec3::X, Vec3::NEG_Z, Vec3::NEG_Y],
    [Vec3::NEG_X, Vec3::Z, Vec3::NEG_Y],
    [Vec3::Y, Vec3::X, Vec3::Z],
    [Vec3::NEG_Y, Vec3::X, Vec3::NEG_Z],
    [Vec3::Z, Vec3::X, Vec3::NEG_Y],
    [Vec3::NEG_Z, Vec3::NEG_X, Vec3::NEG_Y],
];

/// The view-projections drawing the six faces of a cube shadow map around a light at
/// `position` reaching `range`, in array-layer order. Each puts a point where a cube-map
/// lookup in its direction from the light reads; clip depth runs from 0 at SHADOW_NEAR along
/// the face's axis to 1 at the range, so casters nearer or beyond are clipped.
pub fn cube_faces(position: Vec3, range: f32) -> [Mat4; 6] {
    let (n, f) = (SHADOW_NEAR, range.max(2.0 * SHADOW_NEAR));
    let (a, b) = (f / (f - n), -f * n / (f - n));
    CUBE_FACES.map(|[axis, s, t]| {
        let row = |v: Vec3, w: f32| Vec4::new(v.x, v.y, v.z, w - v.dot(position));
        // x = s, y = -t (clip y runs up, t down), w = the distance along the axis.
        Mat4::from_cols(row(s, 0.0), row(-t, 0.0), row(a * axis, b), row(axis, 0.0)).transpose()
    })
}

/// Whether any of the box from `lo` to `hi` lies in the view of cube face `face` (in
/// array-layer order) of a light at `light`: a box wholly outside one of the four planes
/// bounding the face's view is not, and need not be drawn into it.
pub fn face_sees(face: usize, light: Vec3, lo: Vec3, hi: Vec3) -> bool {
    let [axis, s, t] = CUBE_FACES[face];
    let corners: [Vec3; 8] = std::array::from_fn(|k| {
        let pick = |bit: usize, a: f32, b: f32| if k & bit == 0 { a } else { b };
        Vec3::new(
            pick(1, lo.x, hi.x),
            pick(2, lo.y, hi.y),
            pick(4, lo.z, hi.z),
        ) - light
    });
    // Inside the view: |s.v| <= axis.v and |t.v| <= axis.v.
    [s, -s, t, -t]
        .iter()
        .all(|&side| corners.iter().any(|&v| v.dot(axis) - v.dot(side) >= 0.0))
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::*;

    fn key(x: f32) -> ShadowKey {
        ShadowKey {
            position: [x, 0.0, 0.0],
            range: 5.0,
        }
    }

    #[test]
    fn lights_keep_their_slots_and_maps_are_drawn_only_when_stale() {
        let mut s = ShadowSlots::new(4);
        // Three new lights take three slots, each drawn.
        let a = s.assign(&[key(1.0), key(2.0), key(3.0)], 0);
        assert_eq!(a.slots, vec![0, 1, 2]);
        assert_eq!(a.redraw, vec![0, 1, 2]);
        // The same lights in another order: same slots, nothing drawn.
        let b = s.assign(&[key(3.0), key(1.0), key(2.0)], 0);
        assert_eq!(b.slots, vec![2, 0, 1]);
        assert!(b.redraw.is_empty());
        // Light 2 leaves and light 4 comes: it takes the empty slot, not light 2's.
        let c = s.assign(&[key(1.0), key(3.0), key(4.0)], 0);
        assert_eq!(c.slots, vec![0, 2, 3]);
        assert_eq!(c.redraw, vec![3]);
        // Light 2 comes back and finds its map.
        let d = s.assign(&[key(2.0), key(4.0)], 0);
        assert_eq!(d.slots, vec![1, 3]);
        assert!(d.redraw.is_empty());
        // With every slot held, a new light takes one no light of this frame holds.
        let e = s.assign(&[key(5.0), key(2.0)], 0);
        assert_eq!(e.slots[1], 1);
        assert_ne!(e.slots[0], 1);
        assert_eq!(e.redraw, vec![e.slots[0]]);
        // New casters: every light shown is drawn again, in its own slot.
        let f = s.assign(&[key(5.0), key(2.0)], 1);
        assert_eq!(f.slots, e.slots);
        let mut both = e.slots.clone();
        both.sort();
        assert_eq!(f.redraw, both);
        // A moved light is a new light; one given twice shares its slot.
        let g = s.assign(&[key(2.5), key(2.5)], 1);
        assert_eq!(g.slots[0], g.slots[1]);
        assert_eq!(g.redraw.len(), 1);
    }

    #[test]
    fn capacity_grows_by_powers_of_two_up_to_the_most() {
        assert_eq!(capacity_for(0, 64), 1);
        assert_eq!(capacity_for(3, 64), 4);
        assert_eq!(capacity_for(17, 64), 32);
        assert_eq!(capacity_for(64, 64), 64);
        assert_eq!(capacity_for(50, 42), 42);
    }

    #[test]
    fn each_direction_lands_on_its_cube_face_where_a_lookup_reads_it() {
        let light = Vec3::new(1.0, -2.0, 3.0);
        let faces = cube_faces(light, 10.0);
        // Directions from the light, each with its face and the (s, t) a lookup reads.
        let cases = [
            (Vec3::new(2.0, 0.5, -1.0), 0),
            (Vec3::new(-2.0, -0.5, 1.0), 1),
            (Vec3::new(0.5, 2.0, 1.0), 2),
            (Vec3::new(-0.5, -2.0, 1.0), 3),
            (Vec3::new(1.0, -0.5, 2.0), 4),
            (Vec3::new(1.0, 0.5, -2.0), 5),
        ];
        for (v, face) in cases {
            let [axis, s, t] = CUBE_FACES[face];
            let major = v.dot(axis);
            assert!(major > 0.0 && major >= v.abs().max_element() - 1e-6);
            let expected = Vec3::new(v.dot(s) / major, -v.dot(t) / major, 0.0);
            let clip = faces[face] * (light + v).extend(1.0);
            assert!((clip.w - major).abs() < 1e-5, "face {face}: w {}", clip.w);
            let ndc = clip.truncate() / clip.w;
            assert!(
                ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0,
                "face {face}: {ndc}"
            );
            assert!(
                (ndc.truncate() - expected.truncate()).length() < 1e-5,
                "face {face}"
            );
            assert!(ndc.z > 0.0 && ndc.z < 1.0, "face {face}: depth {}", ndc.z);
        }
        // Depth is 0 at the near distance and 1 at the range along the axis.
        let near = faces[4] * (light + Vec3::Z * SHADOW_NEAR).extend(1.0);
        let far = faces[4] * (light + Vec3::Z * 10.0).extend(1.0);
        assert!((near.z / near.w).abs() < 1e-4);
        assert!((far.z / far.w - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_face_sees_the_boxes_in_its_quarter_of_the_view() {
        let light = Vec3::new(10.0, 0.0, 0.0);
        let unit = |c: Vec3| (light + c - Vec3::splat(0.5), light + c + Vec3::splat(0.5));
        // A box 3 m along +x is seen by the +x face only.
        let (lo, hi) = unit(Vec3::new(3.0, 0.0, 0.0));
        let seen: Vec<usize> = (0..6).filter(|&f| face_sees(f, light, lo, hi)).collect();
        assert_eq!(seen, vec![0]);
        // One straddling the +x and +z views is seen by both.
        let (lo, hi) = unit(Vec3::new(3.0, 0.0, 3.0));
        let seen: Vec<usize> = (0..6).filter(|&f| face_sees(f, light, lo, hi)).collect();
        assert_eq!(seen, vec![0, 4]);
        // One around the light is seen by all six.
        let (lo, hi) = unit(Vec3::ZERO);
        assert!((0..6).all(|f| face_sees(f, light, lo, hi)));
    }

    #[test]
    fn a_shadow_key_is_the_light_as_uploaded() {
        let light = Light {
            position: DVec3::new(1.0, 2.0, 3.0),
            color: [1.0; 3],
            strength: 4.0,
        };
        assert_eq!(
            ShadowKey::of(&light),
            ShadowKey {
                position: [1.0, 2.0, 3.0],
                range: 12.0,
            }
        );
    }
}
