//! What tools see: the open context's mesh plus every other placed mesh of the scene, all in
//! the open context's coordinates.

use glam::{DAffine3, DVec3};
use kernel::{InstanceId, MeshId, Model, Scene};

/// Whose geometry a part is, seen from the open context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The open context's own mesh: editable, its entities can be picked.
    Open,
    /// Geometry placed (at any depth) by this instance of the open context: picking it picks
    /// the instance.
    Instance(InstanceId),
    /// Geometry outside the open context: seen and snapped to, never picked.
    Outside,
}

/// One placed mesh.
#[derive(Clone, Copy, Debug)]
pub struct Part<'a> {
    pub model: &'a Model,
    /// Maps the mesh's coordinates into the open context's. Rigid (placements are moves).
    pub place: DAffine3,
    pub owner: Owner,
}

/// Every placed mesh, the open context's first.
#[derive(Clone, Debug)]
pub struct Scenery<'a> {
    pub parts: Vec<Part<'a>>,
}

impl<'a> Scenery<'a> {
    /// Just one mesh, open, in its own coordinates.
    pub fn single(model: &'a Model) -> Self {
        Scenery {
            parts: vec![Part {
                model,
                place: DAffine3::IDENTITY,
                owner: Owner::Open,
            }],
        }
    }

    /// The whole scene seen from the open context `context` (a path of instances from the
    /// root; see `session::Document::context`). None if the path does not resolve.
    pub fn of(scene: &'a Scene, context: &[InstanceId]) -> Option<Self> {
        let (_, to_root) = scene.resolve(context)?;
        let from_root = to_root.inverse();
        let mut parts = Vec::new();
        let mut path = Vec::new();
        collect(
            scene,
            MeshId::Root,
            from_root,
            context,
            &mut path,
            &mut parts,
        );
        // The open mesh first, then the others in walk order.
        parts.sort_by_key(|p: &Part| p.owner != Owner::Open);
        Some(Scenery { parts })
    }

    /// The open context's own mesh.
    pub fn open(&self) -> &'a Model {
        self.parts[0].model
    }

    /// True when an instance is open (there is geometry outside the open context or the root
    /// is not what is open).
    pub fn inside_instance(&self) -> bool {
        self.parts.iter().any(|p| p.owner == Owner::Outside)
    }

    /// The box around everything an instance of the open context places, in the open
    /// context's coordinates. None when it places nothing.
    pub fn instance_box(&self, id: InstanceId) -> Option<[DVec3; 2]> {
        let mut points = self
            .parts
            .iter()
            .filter(|p| p.owner == Owner::Instance(id))
            .flat_map(|p| {
                p.model
                    .vertices()
                    .map(move |(_, v)| p.place.transform_point3(v.pos))
            });
        let first = points.next()?;
        Some(points.fold([first, first], |[lo, hi], q| [lo.min(q), hi.max(q)]))
    }
}

/// Adds the mesh `id`, placed by `place` (into the open context's coordinates) and reached by
/// `path`, and everything it places.
fn collect<'a>(
    scene: &'a Scene,
    id: MeshId,
    place: DAffine3,
    context: &[InstanceId],
    path: &mut Vec<InstanceId>,
    parts: &mut Vec<Part<'a>>,
) {
    let Some(model) = scene.mesh(id) else { return };
    let owner = if path.as_slice() == context {
        Owner::Open
    } else if path.starts_with(context) {
        Owner::Instance(path[context.len()])
    } else {
        Owner::Outside
    };
    parts.push(Part {
        model,
        place,
        owner,
    });
    for (iid, instance) in model.instances() {
        path.push(iid);
        collect(
            scene,
            MeshId::Definition(instance.definition),
            place * instance.transform,
            context,
            path,
            parts,
        );
        path.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::DefinitionKind;

    /// A root holding a square at the origin, grouped, plus a loose edge; the group is moved
    /// by (5, 0, 0). Returns the scene and the instance.
    fn scene() -> (Scene, InstanceId) {
        let mut m = Model::new();
        let p = |x, y| DVec3::new(x, y, 0.0);
        m.add_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)])
            .unwrap();
        let mut s = Scene::from_root(m);
        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
        let (i, _) = s
            .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
            .unwrap();
        s.edit(MeshId::Root, |m| {
            let mut c = m.add_edge(p(0.0, 3.0), p(1.0, 3.0))?;
            c.ops
                .extend(m.move_instances(&[i], DVec3::new(5.0, 0.0, 0.0)).ops);
            Ok(c)
        })
        .unwrap();
        (s, i)
    }

    #[test]
    fn from_the_root_instances_are_placed_where_they_are_drawn() {
        let (s, i) = scene();
        let v = Scenery::of(&s, &[]).unwrap();
        assert_eq!(v.parts.len(), 2);
        assert_eq!(v.parts[0].owner, Owner::Open);
        assert_eq!(v.parts[1].owner, Owner::Instance(i));
        assert!(!v.inside_instance());
        assert_eq!(
            v.instance_box(i),
            Some([DVec3::new(5.0, 0.0, 0.0), DVec3::new(6.0, 1.0, 0.0)])
        );
    }

    #[test]
    fn inside_an_instance_the_rest_is_outside_and_shifted() {
        let (s, i) = scene();
        let v = Scenery::of(&s, &[i]).unwrap();
        assert_eq!(v.parts[0].owner, Owner::Open);
        assert_eq!(v.parts[0].place, DAffine3::IDENTITY);
        assert_eq!(v.parts[1].owner, Owner::Outside);
        // The root's loose edge at x 0..1 lies at x -5..-4 in the group's coordinates.
        assert_eq!(
            v.parts[1].place,
            DAffine3::from_translation(DVec3::new(-5.0, 0.0, 0.0))
        );
        assert!(v.inside_instance());
        assert!(Scenery::of(&s, &[InstanceId(999)]).is_none());
    }
}
