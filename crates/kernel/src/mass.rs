//! Mass properties of solids: volume, centroid and inertia of closed meshes by the divergence
//! theorem (second moments summed over the tetrahedra spanned by each triangle and the origin),
//! carried through instance transforms (scale and mirroring included) and summed over nested
//! instances.

use std::collections::BTreeMap;

use glam::{DAffine3, DMat3, DVec3};

use crate::ids::{InstanceId, MaterialId};
use crate::model::{Instance, Model};
use crate::scene::Scene;
use crate::solid::{open_reason, signed_volume};

/// The deepest nesting of groups and components `mass_report` follows.
const MAX_DEPTH: usize = 64;

/// The zeroth, first and second moments of a region (or of a density over it) about the
/// origin: `v` the volume (or mass), `m` the integral of x, `c` the integral of x x^T.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Moments {
    v: f64,
    m: DVec3,
    c: DMat3,
}

impl Default for Moments {
    // glam's matrix default is the identity, not zero.
    fn default() -> Self {
        Moments {
            v: 0.0,
            m: DVec3::ZERO,
            c: DMat3::ZERO,
        }
    }
}

fn outer(a: DVec3, b: DVec3) -> DMat3 {
    DMat3::from_cols(a * b.x, a * b.y, a * b.z)
}

impl Moments {
    fn add(&mut self, o: &Moments) {
        self.v += o.v;
        self.m += o.m;
        self.c += o.c;
    }

    fn scaled(&self, k: f64) -> Moments {
        Moments {
            v: self.v * k,
            m: self.m * k,
            c: self.c * k,
        }
    }

    /// The moments of the region mapped by `a`: x' = L x + t, with the density per unit of
    /// volume unchanged, so every moment scales by |det L|.
    fn transformed(&self, a: &DAffine3) -> Moments {
        let l = a.matrix3;
        let t = a.translation;
        let d = l.determinant().abs();
        let lm = l * self.m;
        Moments {
            v: d * self.v,
            m: d * (lm + t * self.v),
            c: (l * self.c * l.transpose() + outer(lm, t) + outer(t, lm) + outer(t, t) * self.v)
                * d,
        }
    }
}

/// The moments of a closed mesh (volume positive whichever way its faces point).
fn mesh_moments(m: &Model) -> Moments {
    let sign = if signed_volume(m) < 0.0 { -1.0 } else { 1.0 };
    let mut out = Moments::default();
    for (id, _) in m.faces() {
        for [a, b, c] in m.triangulate(id).unwrap_or_default() {
            let v = a.dot(b.cross(c)) / 6.0 * sign;
            let s = a + b + c;
            out.v += v;
            out.m += s * (v / 4.0);
            out.c += (outer(a, a) + outer(b, b) + outer(c, c) + outer(s, s)) * (v / 20.0);
        }
    }
    out
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
        let centroid_of = |m: &Moments| m.m / m.v;
        if !weighted {
            return Some(MassProps {
                volume,
                mass: None,
                centroid: centroid_of(&self.vol),
                inertia: None,
                principal: None,
            });
        }
        let mass = self.mass.v;
        let g = centroid_of(&self.mass);
        let cc = self.mass.c - outer(g, g) * mass;
        let trace = cc.x_axis.x + cc.y_axis.y + cc.z_axis.z;
        let mut inertia = DMat3::IDENTITY * trace - cc;
        // Symmetrise away rounding.
        inertia = (inertia + inertia.transpose()) * 0.5;
        let (moments, axes) = symmetric_eigen(inertia);
        Some(MassProps {
            volume,
            mass: Some(mass),
            centroid: g,
            inertia: Some(inertia),
            principal: Some(Principal { moments, axes }),
        })
    }
}

/// The material most faces of `m` show on their front (more than half of them), if any.
fn dominant_material(m: &Model) -> Option<MaterialId> {
    let mut counts: BTreeMap<MaterialId, usize> = BTreeMap::new();
    let mut faces = 0usize;
    for (_, f) in m.faces() {
        faces += 1;
        if let Some(id) = f.paint()[0] {
            *counts.entry(id).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .find(|&(_, n)| n * 2 > faces)
        .map(|(id, _)| id)
}

fn walk(
    scene: &Scene,
    instance: &Instance,
    path: &str,
    inherited: Option<MaterialId>,
    depth: usize,
    skipped: &mut Vec<Skipped>,
) -> Acc {
    let skip = |skipped: &mut Vec<Skipped>, path: &str, reason: String| {
        skipped.push(Skipped {
            path: path.into(),
            reason,
        });
    };
    let Some(d) = scene.definition(instance.definition) else {
        skip(skipped, path, "its definition is missing".into());
        return Acc::default();
    };
    if depth > MAX_DEPTH {
        skip(skipped, path, "it is nested too deeply".into());
        return Acc::default();
    }
    let material = instance.material.or(inherited);
    let mut acc = Acc::default();
    if d.mesh.faces().next().is_some() {
        match open_reason(&d.mesh) {
            Some(why) => skip(skipped, path, why),
            None => {
                let vol = mesh_moments(&d.mesh);
                if vol.v <= 1e-12 {
                    skip(skipped, path, "it encloses no volume".into());
                } else {
                    let density = material
                        .or_else(|| dominant_material(&d.mesh))
                        .and_then(|m| scene.material(m))
                        .and_then(|m| m.density);
                    acc.any = true;
                    match density {
                        Some(rho) => acc.mass = vol.scaled(rho),
                        None => acc.unweighted = true,
                    }
                    acc.vol = vol;
                }
            }
        }
    }
    for (id, inner) in d.mesh.instances() {
        let sub = walk(
            scene,
            inner,
            &format!("{path}/i{}", id.0),
            material,
            depth + 1,
            skipped,
        );
        acc.add(&sub.transformed(&inner.transform));
    }
    acc
}

/// The mass properties of one `instance` alone, in the coordinates of the context holding it
/// (None when no part of it is a solid).
pub fn instance_mass(scene: &Scene, instance: &Instance) -> Option<MassProps> {
    walk(scene, instance, "", None, 0, &mut Vec::new())
        .transformed(&instance.transform)
        .props()
}

/// The mass properties of `ids` (instances of `mesh`; empty: every instance of it), each in
/// the coordinates of `mesh`, and their sum. A part's density is its instance's own material's,
/// else an enclosing instance's, else the material most of its faces show on their front.
/// Loose geometry that is not closed is skipped with the reason; unknown ids are an error.
pub fn mass_report(scene: &Scene, mesh: &Model, ids: &[InstanceId]) -> Result<MassReport, String> {
    let ids: Vec<InstanceId> = if ids.is_empty() {
        mesh.instances().map(|(id, _)| id).collect()
    } else {
        ids.to_vec()
    };
    let mut skipped = Vec::new();
    let mut total = Acc::default();
    let mut instances = Vec::new();
    for id in ids {
        let instance = mesh
            .instance(id)
            .ok_or_else(|| format!("no instance i{}", id.0))?;
        let acc = walk(
            scene,
            instance,
            &format!("i{}", id.0),
            None,
            0,
            &mut skipped,
        )
        .transformed(&instance.transform);
        if let Some(p) = acc.props() {
            instances.push((id, p));
            total.add(&acc);
        }
    }
    Ok(MassReport {
        instances,
        total: total.props(),
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use glam::DQuat;

    use super::*;
    use crate::material::Material;
    use crate::primitives::Solid;
    use crate::scene::{DefinitionKind, MeshId};

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
        mass_report(scene, scene.mesh(MeshId::Root).unwrap(), ids).unwrap()
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
    fn density_comes_from_the_instance_then_the_faces_else_none() {
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
