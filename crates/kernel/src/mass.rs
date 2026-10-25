//! Mass properties of solids: volume, centroid and inertia of closed meshes by the divergence
//! theorem (second moments summed over the tetrahedra spanned by each triangle and a point of
//! the mesh), carried through instance transforms (scale and mirroring included) and summed
//! over nested instances. Every part keeps its moments about its own centroid and parts are
//! combined by the parallel-axis theorem, so parts far from the origin stay precise.

use std::collections::{BTreeMap, BTreeSet};

use glam::{DAffine3, DMat3, DVec3};

use crate::ids::{DefinitionId, InstanceId, MaterialId};
use crate::model::{Instance, Model};
use crate::parallel::each_in_parallel;
use crate::scene::{MeshId, Scene};
use crate::solid::open_reason;

/// The deepest nesting of groups and components `mass_report` follows.
const MAX_DEPTH: usize = 64;

/// The zeroth and second moments of a region (or of a density over it) and its centroid:
/// `v` the volume (or mass), `g` the centroid, `c` the integral of (x - g)(x - g)^T.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Moments {
    v: f64,
    g: DVec3,
    c: DMat3,
}

impl Default for Moments {
    // glam's matrix default is the identity, not zero.
    fn default() -> Self {
        Moments {
            v: 0.0,
            g: DVec3::ZERO,
            c: DMat3::ZERO,
        }
    }
}

fn outer(a: DVec3, b: DVec3) -> DMat3 {
    DMat3::from_cols(a * b.x, a * b.y, a * b.z)
}

impl Moments {
    /// Adds a region by the parallel-axis theorem.
    fn add(&mut self, o: &Moments) {
        if o.v == 0.0 {
            return;
        }
        if self.v == 0.0 {
            *self = *o;
            return;
        }
        let v = self.v + o.v;
        let g = self.g + (o.g - self.g) * (o.v / v);
        let (d1, d2) = (self.g - g, o.g - g);
        self.c = self.c + o.c + outer(d1, d1) * self.v + outer(d2, d2) * o.v;
        self.v = v;
        self.g = g;
    }

    fn scaled(&self, k: f64) -> Moments {
        Moments {
            v: self.v * k,
            g: self.g,
            c: self.c * k,
        }
    }

    /// The moments of the region mapped by `a`: x' = L x + t, with the density per unit of
    /// volume unchanged, so every moment scales by |det L|.
    fn transformed(&self, a: &DAffine3) -> Moments {
        let l = a.matrix3;
        let d = l.determinant().abs();
        Moments {
            v: d * self.v,
            g: a.transform_point3(self.g),
            c: l * self.c * l.transpose() * d,
        }
    }
}

/// The moments of a closed mesh (volume positive whichever way its faces point) and whether
/// its faces point inward, or the face that cannot be triangulated.
fn mesh_moments(m: &Model) -> Result<(Moments, bool), String> {
    let mut tris = Vec::new();
    for (id, _) in m.faces() {
        let t = m
            .triangulate(id)
            .ok_or_else(|| format!("face f{} cannot be triangulated", id.0))?;
        tris.extend(t);
    }
    // Sum about a point of the mesh, not the origin, so a mesh far away stays precise.
    let p0 = tris.first().map_or(DVec3::ZERO, |t| t[0]);
    let (mut v, mut s1, mut s2) = (0.0, DVec3::ZERO, DMat3::ZERO);
    for t in &tris {
        let [a, b, c] = t.map(|p| p - p0);
        let w = a.dot(b.cross(c)) / 6.0;
        let s = a + b + c;
        v += w;
        s1 += s * (w / 4.0);
        s2 += (outer(a, a) + outer(b, b) + outer(c, c) + outer(s, s)) * (w / 20.0);
    }
    if v == 0.0 {
        return Ok((Moments::default(), false));
    }
    let local = s1 / v;
    // The signs of v, s1 and s2 flip together, so the centroid and the moments are the same.
    let k = v.signum();
    Ok((
        Moments {
            v: v * k,
            g: p0 + local,
            c: (s2 - outer(local, local) * v) * k,
        },
        v < 0.0,
    ))
}

/// Eigenvalues (ascending) and unit eigenvectors of a symmetric 3x3 matrix, by Jacobi
/// rotations. The vectors are right-handed.
pub fn symmetric_eigen(a: DMat3) -> ([f64; 3], [DVec3; 3]) {
    let mut a = [
        a.x_axis.to_array(),
        a.y_axis.to_array(),
        a.z_axis.to_array(),
    ];
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..64 {
        let off = a[0][1].abs() + a[0][2].abs() + a[1][2].abs();
        let scale = a[0][0].abs() + a[1][1].abs() + a[2][2].abs();
        if off <= 1e-15 * scale.max(f64::MIN_POSITIVE) {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-300 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let t = if theta == 0.0 { 1.0 } else { t };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for row in &mut a {
                let (akp, akq) = (row[p], row[q]);
                row[p] = c * akp - s * akq;
                row[q] = s * akp + c * akq;
            }
            let (rp, rq) = (a[p], a[q]);
            for k in 0..3 {
                a[p][k] = c * rp[k] - s * rq[k];
                a[q][k] = s * rp[k] + c * rq[k];
            }
            for row in &mut v {
                let (vp, vq) = (row[p], row[q]);
                row[p] = c * vp - s * vq;
                row[q] = s * vp + c * vq;
            }
        }
    }
    let mut order = [0usize, 1, 2];
    order.sort_by(|&i, &j| a[i][i].total_cmp(&a[j][j]));
    let vals = order.map(|i| a[i][i]);
    let mut vecs = order.map(|i| DVec3::new(v[0][i], v[1][i], v[2][i]).normalize());
    if vecs[0].cross(vecs[1]).dot(vecs[2]) < 0.0 {
        vecs[2] = -vecs[2];
    }
    (vals, vecs)
}

/// The principal moments of inertia (ascending, kg·m²) and their axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Principal {
    pub moments: [f64; 3],
    pub axes: [DVec3; 3],
}

/// Mass properties of one instance or of a sum of them, in the coordinates of the context
/// holding the instances.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MassProps {
    /// m³.
    pub volume: f64,
    /// kg; None when a contributing solid has no density.
    pub mass: Option<f64>,
    /// The centre of gravity when there is a mass, else the centroid of the volume.
    pub centroid: DVec3,
    /// About the centroid, kg·m²; None without a mass.
    pub inertia: Option<DMat3>,
    pub principal: Option<Principal>,
}

/// A loose-geometry or instance part left out of the sums, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Skipped {
    /// The instance path as "i3/i7", or "i3" for the instance's own loose geometry.
    pub path: String,
    pub reason: String,
}

/// What `mass_report` found.
#[derive(Clone, Debug, PartialEq)]
pub struct MassReport {
    /// Each asked instance that counts, in the order asked.
    pub instances: Vec<(InstanceId, MassProps)>,
    /// The sum of them (None when none counts).
    pub total: Option<MassProps>,
    pub skipped: Vec<Skipped>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Acc {
    vol: Moments,
    mass: Moments,
    /// A part without density went in.
    unweighted: bool,
    any: bool,
}

impl Acc {
    fn add(&mut self, o: &Acc) {
        self.vol.add(&o.vol);
        self.mass.add(&o.mass);
        self.unweighted |= o.unweighted;
        self.any |= o.any;
    }

    fn transformed(&self, a: &DAffine3) -> Acc {
        Acc {
            vol: self.vol.transformed(a),
            mass: self.mass.transformed(a),
            ..*self
        }
    }

    fn props(&self) -> Option<MassProps> {
        if !self.any || self.vol.v <= 0.0 {
            return None;
        }
        let volume = self.vol.v;
        let weighted = !self.unweighted && self.mass.v > 0.0;
        if !weighted {
            return Some(MassProps {
                volume,
                mass: None,
                centroid: self.vol.g,
                inertia: None,
                principal: None,
            });
        }
        let cc = self.mass.c;
        let trace = cc.x_axis.x + cc.y_axis.y + cc.z_axis.z;
        let mut inertia = DMat3::IDENTITY * trace - cc;
        // Symmetrise away rounding.
        inertia = (inertia + inertia.transpose()) * 0.5;
        let (moments, axes) = symmetric_eigen(inertia);
        Some(MassProps {
            volume,
            mass: Some(self.mass.v),
            centroid: self.mass.g,
            inertia: Some(inertia),
            principal: Some(Principal { moments, axes }),
        })
    }
}

/// The material most faces of `m` show on `side` (0 front, 1 back; more than half of them),
/// if any.
pub(crate) fn dominant_material(m: &Model, side: usize) -> Option<MaterialId> {
    let mut counts: BTreeMap<MaterialId, usize> = BTreeMap::new();
    let mut faces = 0usize;
    for (_, f) in m.faces() {
        faces += 1;
        if let Some(id) = f.paint()[side] {
            *counts.entry(id).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .find(|&(_, n)| n * 2 > faces)
        .map(|(id, _)| id)
}

/// What a definition's own geometry is to a mass sum, worked out once per definition.
#[derive(Clone, Debug)]
enum Part {
    /// No faces: only its nested instances count.
    Empty,
    /// Left out, and why.
    Skip(String),
    /// A closed mesh of unit density, and the material most of its faces show on the outside.
    Solid {
        vol: Moments,
        faces: Option<MaterialId>,
    },
}

fn part(m: &Model) -> Part {
    if m.faces().next().is_none() {
        return Part::Empty;
    }
    if let Some(why) = open_reason(m) {
        return Part::Skip(why);
    }
    match mesh_moments(m) {
        Err(why) => Part::Skip(why),
        Ok((vol, _)) if vol.v <= 1e-12 => Part::Skip("it encloses no volume".into()),
        Ok((vol, inward)) => Part::Solid {
            vol,
            // Faces pointing inward show their back on the outside.
            faces: dominant_material(m, usize::from(inward)),
        },
    }
}

fn density(scene: &Scene, m: Option<MaterialId>) -> Option<f64> {
    m.and_then(|m| scene.material(m)).and_then(|m| m.density)
}

/// The density the instances inside the open context inherit from the instances enclosing it
/// (`context`, from the root): the innermost one's own material with a density.
pub fn context_density(scene: &Scene, context: &[InstanceId]) -> Option<f64> {
    let mut mesh = MeshId::Root;
    let mut inherited = None;
    for id in context {
        let Some(inst) = scene.mesh(mesh).and_then(|m| m.instance(*id)) else {
            break;
        };
        inherited = density(scene, inst.material).or(inherited);
        mesh = MeshId::Definition(inst.definition);
    }
    inherited
}

struct Walk<'a> {
    scene: &'a Scene,
    parts: BTreeMap<DefinitionId, Part>,
    skipped: Vec<Skipped>,
}

impl Walk<'_> {
    /// Works out what every definition placed by `instances`, at any depth, is to the sum, all
    /// at once on a few threads.
    fn prepare<'i>(&mut self, instances: impl IntoIterator<Item = &'i Instance>) {
        let scene = self.scene;
        let mut found = Vec::new();
        let mut seen = BTreeSet::new();
        let mut next: Vec<DefinitionId> = instances.into_iter().map(|i| i.definition).collect();
        while let Some(d) = next.pop() {
            let Some(def) = scene.definition(d) else {
                continue;
            };
            if seen.insert(d) {
                found.push((d, &def.mesh));
                next.extend(def.mesh.instances().map(|(_, i)| i.definition));
            }
        }
        let parts = each_in_parallel(found.len(), |k| part(found[k].1));
        self.parts.extend(found.iter().map(|(d, _)| *d).zip(parts));
    }

    fn skip(&mut self, path: &str, reason: String) {
        self.skipped.push(Skipped {
            path: path.into(),
            reason,
        });
    }

    fn instance(
        &mut self,
        instance: &Instance,
        path: &str,
        inherited: Option<f64>,
        depth: usize,
    ) -> Acc {
        let scene = self.scene;
        let Some(d) = scene.definition(instance.definition) else {
            self.skip(path, "its definition is missing".into());
            return Acc::default();
        };
        if depth > MAX_DEPTH {
            self.skip(path, "it is nested too deeply".into());
            return Acc::default();
        }
        // Painted faces show their own material, then the instance's, then an enclosing
        // instance's, as they render; materials without a density are passed over.
        let inherited = density(scene, instance.material).or(inherited);
        let part = self
            .parts
            .entry(instance.definition)
            .or_insert_with(|| part(&d.mesh))
            .clone();
        let mut acc = Acc::default();
        match part {
            Part::Empty => {}
            Part::Skip(why) => self.skip(path, why),
            Part::Solid { vol, faces } => {
                acc.any = true;
                match density(scene, faces).or(inherited) {
                    Some(rho) => acc.mass = vol.scaled(rho),
                    None => acc.unweighted = true,
                }
                acc.vol = vol;
            }
        }
        for (id, inner) in d.mesh.instances() {
            let sub = self.instance(inner, &format!("{path}/i{}", id.0), inherited, depth + 1);
            acc.add(&sub.transformed(&inner.transform));
        }
        acc
    }
}

/// The mass properties of one `instance` alone, in the coordinates of the context holding it
/// (None when no part of it is a solid). `inherited` is the density of the instances enclosing
/// that context (see `context_density`).
pub fn instance_mass(
    scene: &Scene,
    instance: &Instance,
    inherited: Option<f64>,
) -> Option<MassProps> {
    let mut walk = Walk {
        scene,
        parts: BTreeMap::new(),
        skipped: Vec::new(),
    };
    walk.prepare([instance]);
    walk.instance(instance, "", inherited, 0)
        .transformed(&instance.transform)
        .props()
}

/// The mass properties of `ids` (instances of `mesh`, repeats counted once; empty: every
/// instance of it), each in the coordinates of `mesh`, and their sum. `inherited` is the
/// density of the instances enclosing `mesh` (see `context_density`). A part's density is that
/// of the material most of its faces show on the outside, else its instance's own material's,
/// else the nearest enclosing instance's, passing over materials without a density. Loose
/// geometry that is not closed, or cannot be triangulated, is skipped with the reason; unknown
/// ids are an error.
pub fn mass_report(
    scene: &Scene,
    mesh: &Model,
    ids: &[InstanceId],
    inherited: Option<f64>,
) -> Result<MassReport, String> {
    let mut ids: Vec<InstanceId> = if ids.is_empty() {
        mesh.instances().map(|(id, _)| id).collect()
    } else {
        ids.to_vec()
    };
    let mut seen = BTreeSet::new();
    ids.retain(|id| seen.insert(*id));
    let mut walk = Walk {
        scene,
        parts: BTreeMap::new(),
        skipped: Vec::new(),
    };
    walk.prepare(ids.iter().filter_map(|id| mesh.instance(*id)));
    let mut total = Acc::default();
    let mut instances = Vec::new();
    for id in ids {
        let instance = mesh
            .instance(id)
            .ok_or_else(|| format!("no instance i{}", id.0))?;
        let acc = walk
            .instance(instance, &format!("i{}", id.0), inherited, 0)
            .transformed(&instance.transform);
        if let Some(p) = acc.props() {
            instances.push((id, p));
            total.add(&acc);
        }
    }
    Ok(MassReport {
        instances,
        total: total.props(),
        skipped: walk.skipped,
    })
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use glam::DQuat;

    use super::*;
    use crate::ids::FaceId;
    use crate::material::Material;
    use crate::primitives::Solid;
    use crate::scene::DefinitionKind;

    fn steel(scene: &mut Scene, density: Option<f64>) -> MaterialId {
        scene
            .add_material_from(Material {
                name: format!("M{}", scene.materials().count()),
                color: [1, 2, 3],
                opacity: 1.0,
                texture: None,
                emission: 0.0,
                metalness: 0.0,
                roughness: 1.0,
                density,
            })
            .unwrap()
            .0
    }

    fn place(
        scene: &mut Scene,
        solid: &Solid,
        at: DAffine3,
        material: Option<MaterialId>,
    ) -> InstanceId {
        scene
            .add_solid(
                MeshId::Root,
                solid,
                DefinitionKind::Group,
                "S",
                at,
                material,
            )
            .unwrap()
            .0
    }

    fn report(scene: &Scene, ids: &[InstanceId]) -> MassReport {
        mass_report(scene, scene.mesh(MeshId::Root).unwrap(), ids, None).unwrap()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
    }

    fn diag(p: &MassProps) -> [f64; 3] {
        let i = p.inertia.unwrap();
        [i.x_axis.x, i.y_axis.y, i.z_axis.z]
    }

    #[test]
    fn a_box_has_exact_mass_centroid_and_inertia() {
        let mut s = Scene::new();
        let m = steel(&mut s, Some(7850.0));
        let id = place(
            &mut s,
            &Solid::cuboid(DVec3::new(2.0, 3.0, 4.0)).unwrap(),
            DAffine3::from_translation(DVec3::new(10.0, 0.0, 0.0)),
            Some(m),
        );
        let p = report(&s, &[id]).total.unwrap();
        let mass = 24.0 * 7850.0;
        assert!(close(p.volume, 24.0) && close(p.mass.unwrap(), mass));
        assert!((p.centroid - DVec3::new(11.0, 1.5, 2.0)).length() < 1e-9);
        let d = diag(&p);
        assert!(close(d[0], mass * (9.0 + 16.0) / 12.0));
        assert!(close(d[1], mass * (4.0 + 16.0) / 12.0));
        assert!(close(d[2], mass * (4.0 + 9.0) / 12.0));
        let i = p.inertia.unwrap();
        assert!(i.x_axis.y.abs() < 1e-6 && i.x_axis.z.abs() < 1e-6 && i.y_axis.z.abs() < 1e-6);
        let pr = p.principal.unwrap();
        assert!(close(pr.moments[0], mass * 13.0 / 12.0));
        assert!(close(pr.moments[2], mass * 25.0 / 12.0));
    }

    #[test]
    fn a_cylinder_matches_the_faceted_analytic_values() {
        let n = 64u32;
        let (r, h) = (0.5, 2.0);
        let mut s = Scene::new();
        let m = steel(&mut s, Some(1000.0));
        let id = place(
            &mut s,
            &Solid::cylinder(r, h, n).unwrap(),
            DAffine3::IDENTITY,
            Some(m),
        );
        let p = report(&s, &[id]).total.unwrap();
        // The regular n-gon of circumradius r: area and polar second moment per unit height.
        let a = 0.5 * n as f64 * r * r * (2.0 * PI / n as f64).sin();
        let k = (2.0 * PI / n as f64).sin();
        let j = n as f64 * r.powi(4) / 12.0 * k * (2.0 + (2.0 * PI / n as f64).cos());
        let mass = a * h * 1000.0;
        assert!(close(p.mass.unwrap(), mass));
        let izz = 1000.0 * h * j;
        assert!(close(diag(&p)[2], izz), "{} vs {izz}", diag(&p)[2]);
        // Close to the true cylinder.
        assert!((izz / (0.5 * mass * r * r) - 1.0).abs() < 0.01);
        assert!((p.centroid - DVec3::new(0.0, 0.0, 1.0)).length() < 1e-9);
    }

    #[test]
    fn a_sphere_is_close_to_two_fifths_m_r_squared() {
        let mut s = Scene::new();
        let m = steel(&mut s, Some(2700.0));
        let id = place(
            &mut s,
            &Solid::sphere(1.0, 48).unwrap(),
            DAffine3::IDENTITY,
            Some(m),
        );
        let p = report(&s, &[id]).total.unwrap();
        let mass = p.mass.unwrap();
        assert!((p.volume / (4.0 / 3.0 * PI) - 1.0).abs() < 0.01);
        let d = diag(&p);
        for v in d {
            assert!((v / (0.4 * mass) - 1.0).abs() < 0.02, "{d:?}");
        }
    }

    #[test]
    fn rotation_scale_and_mirror_transform_the_inertia() {
        let mut s = Scene::new();
        let m = steel(&mut s, Some(1000.0));
        let b = Solid::cuboid(DVec3::new(2.0, 3.0, 4.0)).unwrap();
        // Rotated a quarter turn about z about the origin.
        let rot = DAffine3::from_quat(DQuat::from_rotation_z(PI / 2.0));
        let r = place(&mut s, &b, rot, Some(m));
        let p = report(&s, &[r]).total.unwrap();
        let mass = 24000.0;
        let d = diag(&p);
        assert!(close(d[0], mass * 20.0 / 12.0) && close(d[1], mass * 25.0 / 12.0));
        assert!((p.centroid - DVec3::new(-1.5, 1.0, 2.0)).length() < 1e-9);
        // Non-uniform scale: a 2x3x4 box scaled (2,1,0.5) is a 4x3x2 box.
        let sc = place(
            &mut s,
            &b,
            DAffine3::from_scale(DVec3::new(2.0, 1.0, 0.5)),
            Some(m),
        );
        let p = report(&s, &[sc]).total.unwrap();
        let d = diag(&p);
        assert!(close(p.volume, 24.0));
        assert!(close(d[0], mass * (9.0 + 4.0) / 12.0));
        assert!(close(d[1], mass * (16.0 + 4.0) / 12.0));
        assert!(close(d[2], mass * (16.0 + 9.0) / 12.0));
        // A mirror (negative determinant) gives the same, positive, numbers.
        let mir = place(
            &mut s,
            &b,
            DAffine3::from_scale(DVec3::new(-1.0, 1.0, 1.0)),
            Some(m),
        );
        let p = report(&s, &[mir]).total.unwrap();
        assert!(close(p.volume, 24.0) && close(p.mass.unwrap(), mass));
        assert!((p.centroid - DVec3::new(-1.0, 1.5, 2.0)).length() < 1e-9);
        assert!(close(diag(&p)[0], mass * 25.0 / 12.0));
    }

    #[test]
    fn the_parallel_axis_theorem_combines_parts() {
        let mut s = Scene::new();
        let m = steel(&mut s, Some(1000.0));
        let b = Solid::cuboid(DVec3::ONE).unwrap();
        let a = place(&mut s, &b, DAffine3::IDENTITY, Some(m));
        let c = place(
            &mut s,
            &b,
            DAffine3::from_translation(DVec3::new(3.0, 0.0, 0.0)),
            Some(m),
        );
        let r = report(&s, &[a, c]);
        assert_eq!(r.instances.len(), 2);
        let t = r.total.unwrap();
        assert!((t.centroid - DVec3::new(2.0, 0.5, 0.5)).length() < 1e-9);
        // Each cube: m/6 about its own centre; the offset 1.5 adds m d^2 in y and z.
        let each = 1000.0 / 6.0;
        assert!(close(diag(&t)[0], 2.0 * each));
        assert!(close(diag(&t)[1], 2.0 * (each + 1000.0 * 2.25)));
    }

    #[test]
    fn density_comes_from_the_faces_then_the_instance_else_none() {
        let mut s = Scene::new();
        let light = steel(&mut s, Some(100.0));
        let heavy = steel(&mut s, Some(8000.0));
        let b = Solid::cuboid(DVec3::ONE).unwrap();
        let own = place(&mut s, &b, DAffine3::IDENTITY, Some(light));
        let bare = place(&mut s, &b, DAffine3::IDENTITY, None);
        let faces: Vec<_> = {
            let def = s
                .mesh(MeshId::Root)
                .unwrap()
                .instance(bare)
                .unwrap()
                .definition;
            s.definition(def)
                .unwrap()
                .mesh
                .faces()
                .map(|(f, _)| f)
                .collect()
        };
        assert_eq!(report(&s, &[bare]).total.unwrap().mass, None);
        let def = s
            .mesh(MeshId::Root)
            .unwrap()
            .instance(bare)
            .unwrap()
            .definition;
        s.edit(MeshId::Definition(def), |m| {
            Ok(m.paint_faces(&faces[..4], crate::Side::Front, Some(heavy)))
        })
        .unwrap();
        assert!(close(
            report(&s, &[bare]).total.unwrap().mass.unwrap(),
            8000.0
        ));
        assert!(close(
            report(&s, &[own]).total.unwrap().mass.unwrap(),
            100.0
        ));
        // One part without density makes the sum massless; the volume still counts.
        let nodens = place(&mut s, &b, DAffine3::IDENTITY, None);
        let t = report(&s, &[own, nodens]).total.unwrap();
        assert!(t.mass.is_none() && t.inertia.is_none() && close(t.volume, 2.0));
    }

    fn definition_faces(s: &Scene, id: InstanceId) -> (crate::ids::DefinitionId, Vec<FaceId>) {
        let def = s
            .mesh(MeshId::Root)
            .unwrap()
            .instance(id)
            .unwrap()
            .definition;
        let faces = s
            .definition(def)
            .unwrap()
            .mesh
            .faces()
            .map(|(f, _)| f)
            .collect();
        (def, faces)
    }

    #[test]
    fn painted_faces_win_over_the_instance_material() {
        let mut s = Scene::new();
        let steel_id = steel(&mut s, Some(7850.0));
        let red = steel(&mut s, Some(100.0));
        let plain = steel(&mut s, None);
        let b = Solid::cuboid(DVec3::ONE).unwrap();
        let id = place(&mut s, &b, DAffine3::IDENTITY, Some(red));
        let (def, faces) = definition_faces(&s, id);
        s.edit(MeshId::Definition(def), |m| {
            Ok(m.paint_faces(&faces, crate::Side::Front, Some(steel_id)))
        })
        .unwrap();
        assert!(close(
            report(&s, &[id]).total.unwrap().mass.unwrap(),
            7850.0
        ));
        // Faces whose material has no density fall through to the instance.
        s.edit(MeshId::Definition(def), |m| {
            Ok(m.paint_faces(&faces, crate::Side::Front, Some(plain)))
        })
        .unwrap();
        assert!(close(report(&s, &[id]).total.unwrap().mass.unwrap(), 100.0));
    }

    #[test]
    fn inward_facing_solids_take_the_back_material() {
        let mut s = Scene::new();
        let heavy = steel(&mut s, Some(8000.0));
        let id = place(
            &mut s,
            &Solid::cuboid(DVec3::ONE).unwrap(),
            DAffine3::IDENTITY,
            None,
        );
        let (def, faces) = definition_faces(&s, id);
        s.edit(MeshId::Definition(def), |m| Ok(m.reverse_faces(&faces).0))
            .unwrap();
        let (_, faces) = definition_faces(&s, id);
        s.edit(MeshId::Definition(def), |m| {
            Ok(m.paint_faces(&faces, crate::Side::Back, Some(heavy)))
        })
        .unwrap();
        let p = report(&s, &[id]).total.unwrap();
        assert!(close(p.volume, 1.0) && close(p.mass.unwrap(), 8000.0));
    }

    #[test]
    fn enclosing_instances_and_the_open_context_pass_their_density_down() {
        let mut s = Scene::new();
        let heavy = steel(&mut s, Some(2000.0));
        let plain = steel(&mut s, None);
        let outer = place(
            &mut s,
            &Solid::cuboid(DVec3::ONE).unwrap(),
            DAffine3::IDENTITY,
            Some(heavy),
        );
        let (def, faces) = definition_faces(&s, outer);
        s.edit(MeshId::Definition(def), |m| Ok(m.erase(&[], &faces)))
            .unwrap();
        let edges: Vec<_> = s
            .definition(def)
            .unwrap()
            .mesh
            .edges()
            .map(|(e, _)| e)
            .collect();
        s.edit(MeshId::Definition(def), |m| Ok(m.erase(&edges, &[])))
            .unwrap();
        // An inner box painted with a material without density.
        let inner = s
            .add_solid(
                MeshId::Definition(def),
                &Solid::cuboid(DVec3::ONE).unwrap(),
                DefinitionKind::Group,
                "In",
                DAffine3::IDENTITY,
                Some(plain),
            )
            .unwrap()
            .0;
        let from_root = report(&s, &[outer]).total.unwrap();
        assert!(close(from_root.mass.unwrap(), 2000.0));
        let seed = context_density(&s, &[outer]);
        assert_eq!(seed, Some(2000.0));
        let inside = mass_report(&s, &s.definition(def).unwrap().mesh, &[inner], seed)
            .unwrap()
            .total
            .unwrap();
        assert!(close(inside.mass.unwrap(), from_root.mass.unwrap()));
    }

    #[test]
    fn repeated_ids_count_once() {
        let mut s = Scene::new();
        let m = steel(&mut s, Some(1000.0));
        let id = place(
            &mut s,
            &Solid::cuboid(DVec3::ONE).unwrap(),
            DAffine3::IDENTITY,
            Some(m),
        );
        let r = report(&s, &[id, id, id]);
        assert_eq!(r.instances.len(), 1);
        assert!(close(r.total.unwrap().mass.unwrap(), 1000.0));
    }

    #[test]
    fn parts_far_from_the_origin_stay_precise() {
        let mut s = Scene::new();
        let m = steel(&mut s, Some(1000.0));
        let far = DVec3::new(1.0e5, -2.0e5, 3.0e4);
        let b = Solid::cuboid(DVec3::new(2.0, 3.0, 4.0)).unwrap();
        // Placed far away by the transform, and a second one 1 m beside it.
        let a = place(&mut s, &b, DAffine3::from_translation(far), Some(m));
        let c = place(
            &mut s,
            &b,
            DAffine3::from_translation(far + DVec3::new(0.0, 0.0, 4.0)),
            Some(m),
        );
        let mass = 24000.0;
        let p = report(&s, &[a]).total.unwrap();
        let d = diag(&p);
        assert!(close(d[0], mass * 25.0 / 12.0), "{d:?}");
        assert!(close(d[2], mass * 13.0 / 12.0), "{d:?}");
        assert!((p.centroid - (far + DVec3::new(1.0, 1.5, 2.0))).length() < 1e-9);
        // Stacked: a 2x3x8 box.
        let t = report(&s, &[a, c]).total.unwrap();
        let d = diag(&t);
        assert!(close(d[2], 2.0 * mass * 13.0 / 12.0), "{d:?}");
        assert!(close(d[0], 2.0 * mass * (9.0 + 64.0) / 12.0), "{d:?}");
        // Geometry that is itself far away (an identity placement).
        let local = Solid::cuboid(DVec3::new(2.0, 3.0, 4.0)).unwrap();
        let g = place(&mut s, &local, DAffine3::IDENTITY, Some(m));
        let (def, _) = definition_faces(&s, g);
        s.edit(MeshId::Definition(def), |m| {
            let all: BTreeSet<_> = m.vertices().map(|(v, _)| v).collect();
            m.move_vertices(&all, far)
        })
        .unwrap();
        let p = report(&s, &[g]).total.unwrap();
        let d = diag(&p);
        assert!(close(d[0], mass * 25.0 / 12.0), "{d:?}");
        assert!(close(d[1], mass * 20.0 / 12.0), "{d:?}");
    }

    #[test]
    fn open_loose_geometry_is_skipped_with_the_reason() {
        let mut s = Scene::new();
        let m = steel(&mut s, Some(1000.0));
        let id = place(
            &mut s,
            &Solid::cuboid(DVec3::ONE).unwrap(),
            DAffine3::IDENTITY,
            Some(m),
        );
        let def = s
            .mesh(MeshId::Root)
            .unwrap()
            .instance(id)
            .unwrap()
            .definition;
        let f = s.definition(def).unwrap().mesh.faces().next().unwrap().0;
        s.edit(MeshId::Definition(def), |m| Ok(m.erase(&[], &[f])))
            .unwrap();
        let r = report(&s, &[id]);
        assert!(r.total.is_none() && r.instances.is_empty());
        assert_eq!(r.skipped.len(), 1);
        assert_eq!(r.skipped[0].path, format!("i{}", id.0));
        assert!(r.skipped[0].reason.contains("bounds 1 face"));
    }

    #[test]
    fn eigen_solver_finds_principal_axes() {
        let a = DMat3::from_cols(
            DVec3::new(2.0, 1.0, 0.0),
            DVec3::new(1.0, 2.0, 0.0),
            DVec3::new(0.0, 0.0, 5.0),
        );
        let (vals, vecs) = symmetric_eigen(a);
        assert!(close(vals[0], 1.0) && close(vals[1], 3.0) && close(vals[2], 5.0));
        for (v, x) in vals.iter().zip(vecs) {
            assert!(((a * x) - x * *v).length() < 1e-9);
        }
    }
}
