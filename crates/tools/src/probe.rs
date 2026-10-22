//! What lies along a ray: every face it meets, and the edges, guides and annotation lines
//! passing near it, for agents asking what is under a pixel.

use glam::{DVec2, DVec3};
use kernel::{EntityId, Guide};
use session::{Camera, Ray};

use crate::inference::{Cone, Near, closest_on_line_to_ray};
use crate::scenery::Scenery;

/// A face, edge, guide, dimension or note a pick met.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// The entity, an id in the mesh of its part.
    pub id: EntityId,
    /// The index of its part in the `Scenery` (see `Scenery::path`).
    pub part: usize,
    /// Where the ray meets a face, or the point of an edge or guide nearest the ray, in the
    /// open context's coordinates.
    pub point: DVec3,
    /// A face's unit normal (the direction its front side faces) in the open context's
    /// coordinates; None for edges and guides.
    pub normal: Option<DVec3>,
    /// How far along the ray `point` lies.
    pub distance: f64,
}

/// How near the ray an edge or guide must pass to be met.
#[derive(Clone, Copy, Debug)]
pub enum Reach<'a> {
    /// Within `radius_px` of `cursor` on screen, as `camera` (whose ray through the cursor is
    /// the one picked along) shows it.
    Screen {
        camera: &'a Camera,
        cursor: DVec2,
        radius_px: f64,
    },
    /// Within `radius` (meters) of the ray.
    Around { radius: f64 },
}

/// Everything `ray` (unit direction, in the open context's coordinates) meets in front of its
/// origin, nearest first: every face it crosses, whatever lies in front, and every edge (not
/// soft: soft edges are hidden), guide, and dimension or leader note (by its lines) passing
/// within `reach`. What hidden tags hide is not met (see `Scenery`).
pub fn pick(scenery: &Scenery, ray: &Ray, reach: Reach) -> Vec<Hit> {
    let mut hits = Vec::new();
    // Nothing behind the fill of a section cut is met.
    let behind = scenery
        .fill_hit(ray.origin, ray.dir, 0.0, f64::INFINITY)
        .map_or(f64::INFINITY, |t| t + 1e-9);
    for (k, part) in scenery.parts().enumerate() {
        if !part.may_meet_ray(ray.origin, ray.dir, f64::INFINITY) {
            continue;
        }
        let faces = part.model.index().ray_hits(
            part.back.transform_point3(ray.origin),
            part.back.transform_vector3(ray.dir),
            0.0,
            f64::INFINITY,
        );
        // Normals go into the open context's coordinates by the inverse transpose.
        let normals = part.back.matrix3.transpose();
        let faces = faces.into_iter().filter(|(f, t)| {
            *t <= behind && part.shows_face(*f) && part.kept(ray.origin + ray.dir * *t)
        });
        for (f, t) in faces {
            let normal = part
                .model
                .face(f)
                .and_then(|face| (normals * face.normal).try_normalize());
            hits.push(Hit {
                id: EntityId::Face(f),
                part: k,
                point: ray.origin + ray.dir * t,
                normal,
                distance: t,
            });
        }
    }

    let (cone, near_enough): (Cone, Box<dyn Fn(DVec3) -> bool>) = match reach {
        Reach::Screen {
            camera,
            cursor,
            radius_px,
        } => {
            let projector = camera.projector();
            (
                Cone::around(camera, cursor, radius_px),
                Box::new(move |p| {
                    projector
                        .project(p)
                        .is_some_and(|s| s.distance(cursor) <= radius_px)
                }),
            )
        }
        Reach::Around { radius } => {
            let (origin, dir) = (ray.origin, ray.dir);
            (
                Cone::along(origin, dir, radius),
                Box::new(move |p| {
                    let w = p - origin;
                    (w - dir * w.dot(dir)).length() <= radius
                }),
            )
        }
    };
    let mut offer = |id: EntityId, part: usize, point: DVec3| {
        let distance = (point - ray.origin).dot(ray.dir);
        if distance >= 0.0 && distance <= behind && near_enough(point) {
            hits.push(Hit {
                id,
                part,
                point,
                normal: None,
                distance,
            });
        }
    };
    for (k, e, a, b) in Near::in_cone(scenery, cone).segments {
        let point = closest_on_line_to_ray(a, b - a, ray, true);
        if scenery.part(k).kept(point) {
            offer(EntityId::Edge(e), k, point);
        }
    }
    for &(k, id, g) in scenery.guides() {
        let point = match g {
            Guide::Point { position, .. } => position,
            Guide::Line {
                point, direction, ..
            } => closest_on_line_to_ray(point, direction, ray, false),
        };
        offer(EntityId::Guide(id), k, point);
    }
    // An annotation is met once, at the point of its lines nearest the ray.
    for (k, id, shape) in scenery.annotations() {
        let off_ray = |p: &DVec3| {
            let t = (*p - ray.origin).dot(ray.dir);
            (*p - ray.origin - ray.dir * t).length()
        };
        let nearest = shape
            .segments()
            .into_iter()
            .map(|[a, b]| closest_on_line_to_ray(a, b - a, ray, true))
            .min_by(|p, q| off_ray(p).total_cmp(&off_ray(q)));
        if let Some(point) = nearest {
            offer(*id, *k, point);
        }
    }
    // A section plane is met once, at the point of its outline nearest the ray.
    for (k, id, shape) in scenery.sections() {
        let off_ray = |p: &DVec3| {
            let t = (*p - ray.origin).dot(ray.dir);
            (*p - ray.origin - ray.dir * t).length()
        };
        let nearest = shape
            .segments()
            .into_iter()
            .map(|[a, b]| closest_on_line_to_ray(a, b - a, ray, true))
            .min_by(|p, q| off_ray(p).total_cmp(&off_ray(q)));
        if let Some(point) = nearest {
            offer(*id, *k, point);
        }
    }
    hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::{DefinitionKind, InstanceId, MeshId, Model, Scene};

    /// A root holding a unit square at z = 0, another at z = 1 and a guide point at
    /// (0.25, 0.25, 5); and a group holding a unit square at z = 0 from x = 5 to 6.
    fn scene() -> (Scene, InstanceId) {
        let p = |x, y, z| DVec3::new(x, y, z);
        let square = |m: &mut Model, x: f64, z: f64| {
            m.add_polygon(&[
                p(x, 0.0, z),
                p(x + 1.0, 0.0, z),
                p(x + 1.0, 1.0, z),
                p(x, 1.0, z),
            ])
            .unwrap();
        };
        let mut m = Model::new();
        square(&mut m, 5.0, 0.0);
        let mut s = Scene::from_root(m);
        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
        let (i, _) = s
            .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
            .unwrap();
        s.edit(MeshId::Root, |m| {
            let mut c = m.add_polygon(&[
                p(0.0, 0.0, 0.0),
                p(1.0, 0.0, 0.0),
                p(1.0, 1.0, 0.0),
                p(0.0, 1.0, 0.0),
            ])?;
            c.ops.extend(
                m.add_polygon(&[
                    p(0.0, 0.0, 1.0),
                    p(1.0, 0.0, 1.0),
                    p(1.0, 1.0, 1.0),
                    p(0.0, 1.0, 1.0),
                ])?
                .ops,
            );
            c.ops.extend(
                m.add_guide(Guide::Point {
                    position: p(0.25, 0.25, 5.0),
                    tag: None,
                })?
                .ops,
            );
            Ok(c)
        })
        .unwrap();
        (s, i)
    }

    fn down_at(x: f64, y: f64) -> Ray {
        Ray {
            origin: DVec3::new(x, y, 10.0),
            dir: DVec3::NEG_Z,
        }
    }

    /// The kind letter of each hit's id ("f", "e", "g") with its distance.
    fn kinds(hits: &[Hit]) -> Vec<(char, f64)> {
        hits.iter()
            .map(|h| (h.id.to_string().chars().next().unwrap(), h.distance))
            .collect()
    }

    #[test]
    fn a_ray_meets_every_face_front_to_back() {
        let (s, _) = scene();
        let v = Scenery::of(&s, &[]).unwrap();
        let hits = pick(&v, &down_at(0.5, 0.5), Reach::Around { radius: 0.0 });
        assert_eq!(kinds(&hits), vec![('f', 9.0), ('f', 10.0)]);
        assert_eq!(hits[0].point, DVec3::new(0.5, 0.5, 1.0));
        assert!(hits.iter().all(|h| v.path(h.part).is_empty()));
        let normal = hits[0].normal.unwrap();
        assert!((normal.z.abs() - 1.0).abs() < 1e-12, "{normal}");
    }

    #[test]
    fn faces_in_groups_carry_their_path_and_placement() {
        let (s, i) = scene();
        let v = Scenery::of(&s, &[]).unwrap();
        let hits = pick(&v, &down_at(5.5, 0.5), Reach::Around { radius: 0.0 });
        assert_eq!(kinds(&hits), vec![('f', 10.0)]);
        assert_eq!(v.path(hits[0].part), &[i]);
        assert_eq!(hits[0].point, DVec3::new(5.5, 0.5, 0.0));
        // Seen from inside the group, the same face is the open context's.
        let inside = Scenery::of(&s, &[i]).unwrap();
        let hits = pick(&inside, &down_at(5.5, 0.5), Reach::Around { radius: 0.0 });
        assert_eq!(kinds(&hits), vec![('f', 10.0)]);
        assert_eq!(hits[0].part, 0);
        assert_eq!(inside.path(0), &[i]);
    }

    #[test]
    fn edges_and_guides_within_reach_are_met_too() {
        let (s, _) = scene();
        let v = Scenery::of(&s, &[]).unwrap();
        let hits = pick(&v, &down_at(0.255, 0.25), Reach::Around { radius: 0.01 });
        assert_eq!(kinds(&hits), vec![('g', 5.0), ('f', 9.0), ('f', 10.0)]);
        let hits = pick(&v, &down_at(0.995, 0.5), Reach::Around { radius: 0.01 });
        let mut found = kinds(&hits);
        found.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(
            found,
            vec![('e', 9.0), ('e', 10.0), ('f', 9.0), ('f', 10.0)]
        );
        // On screen: the edge passes within the radius of the cursor.
        let camera = Camera::new(DVec2::new(800.0, 600.0));
        let edge_mid = DVec3::new(1.0, 0.5, 1.0);
        let cursor = camera.project(edge_mid).unwrap() + DVec2::new(2.0, 0.0);
        let ray = camera.ray(cursor);
        let reach = |radius_px| Reach::Screen {
            camera: &camera,
            cursor,
            radius_px,
        };
        let near = pick(&v, &ray, reach(4.0));
        let edge = near.iter().find(|h| h.id.to_string().starts_with('e'));
        assert!(
            edge.is_some_and(|h| h.point.distance(edge_mid) < 0.05),
            "{near:?}"
        );
        let tight = pick(&v, &ray, reach(0.5));
        assert!(
            tight.iter().all(|h| !h.id.to_string().starts_with('e')),
            "{tight:?}"
        );
    }
}
