//! Telling whether two component definitions, possibly of different scenes, are the same
//! component: same name (but for a trailing `#<digits>`), kind, geometry, materials and nested definitions, whatever the ids.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use glam::{DAffine3, DVec3};

use crate::model::Model;
use crate::scene::{DefinitionKind, Scene};
use crate::{DefinitionId, MaterialId, TagId};

/// Fingerprints of the definitions of one scene, worked out on demand and kept.
pub struct Fingerprints<'a> {
    scene: &'a Scene,
    memo: HashMap<DefinitionId, Option<u64>>,
}

impl<'a> Fingerprints<'a> {
    pub fn new(scene: &'a Scene) -> Self {
        Self {
            scene,
            memo: HashMap::new(),
        }
    }

    /// A number equal for definitions with the same name (but for a trailing `#<digits>`), kind, vertices, edges, faces (with
    /// their materials and tags by name), instances (their transforms and what they place)
    /// and nothing else; the numbers of two definitions differ when their content does (but
    /// for hash collisions). None for a definition holding guides, dimensions, notes or
    /// section planes, or placing one that does, or a missing one: those are never taken as
    /// the same.
    pub fn of(&mut self, id: DefinitionId) -> Option<u64> {
        if let Some(known) = self.memo.get(&id) {
            return *known;
        }
        // Children first, without recursion: nesting can be deep.
        let mut stack = vec![(id, false)];
        while let Some((d, children_done)) = stack.pop() {
            if self.memo.contains_key(&d) {
                continue;
            }
            let Some(def) = self.scene.definition(d) else {
                self.memo.insert(d, None);
                continue;
            };
            if !children_done {
                stack.push((d, true));
                for (_, i) in def.mesh.instances() {
                    if !self.memo.contains_key(&i.definition) {
                        stack.push((i.definition, false));
                    }
                }
                continue;
            }
            let value = self.compute(d);
            self.memo.insert(d, value);
        }
        self.memo.get(&id).copied().flatten()
    }

    fn compute(&self, id: DefinitionId) -> Option<u64> {
        let def = self.scene.definition(id)?;
        let m = &def.mesh;
        if m.guides().next().is_some()
            || m.dimensions().next().is_some()
            || m.notes().next().is_some()
            || m.sections().next().is_some()
        {
            return None;
        }
        let mut h = DefaultHasher::new();
        base_name(&def.name).hash(&mut h);
        (def.kind == DefinitionKind::Component).hash(&mut h);
        sorted(vertices(m)).hash(&mut h);
        sorted(self.edges(m)).hash(&mut h);
        sorted(self.faces(m)).hash(&mut h);
        let mut placed = Vec::new();
        for (_, i) in m.instances() {
            // A child that cannot be fingerprinted makes this one unfit too.
            let child = self.memo.get(&i.definition).copied().flatten()?;
            let mut ih = DefaultHasher::new();
            child.hash(&mut ih);
            transform_bits(&i.transform).hash(&mut ih);
            self.material(i.material).hash(&mut ih);
            self.tag(i.tag).hash(&mut ih);
            (i.hidden, i.section_cut).hash(&mut ih);
            placed.push(ih.finish());
        }
        sorted(placed).hash(&mut h);
        Some(h.finish())
    }

    fn edges(&self, m: &Model) -> Vec<u64> {
        m.edges()
            .filter_map(|(_, e)| {
                let a = bits(m.position(e.v[0])?);
                let b = bits(m.position(e.v[1])?);
                let mut h = DefaultHasher::new();
                (a.min(b), a.max(b)).hash(&mut h);
                (e.soft, e.polygon, e.hidden, e.curve.is_some()).hash(&mut h);
                self.tag(e.tag).hash(&mut h);
                Some(h.finish())
            })
            .collect()
    }

    fn faces(&self, m: &Model) -> Vec<u64> {
        let ring = |l: &[crate::VertexId]| -> Vec<[u64; 3]> {
            let mut p: Vec<[u64; 3]> = l
                .iter()
                .map(|v| bits(m.position(*v).unwrap_or(DVec3::NAN)))
                .collect();
            if let Some(first) = (0..p.len()).min_by_key(|i| p[*i]) {
                p.rotate_left(first);
            }
            p
        };
        m.faces()
            .map(|(_, f)| {
                let mut h = DefaultHasher::new();
                ring(&f.outer).hash(&mut h);
                sorted(f.holes.iter().map(|l| {
                    let mut x = DefaultHasher::new();
                    ring(l).hash(&mut x);
                    x.finish()
                }))
                .hash(&mut h);
                bits(f.normal).hash(&mut h);
                self.material(f.front).hash(&mut h);
                self.material(f.back).hash(&mut h);
                format!("{:?}", f.texture).hash(&mut h);
                self.tag(f.tag).hash(&mut h);
                f.hidden.hash(&mut h);
                h.finish()
            })
            .collect()
    }

    fn material(&self, id: Option<MaterialId>) -> Option<String> {
        let m = self.scene.material(id?)?;
        Some(format!(
            "{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            m.name, m.color, m.opacity, m.emission, m.metalness, m.roughness, m.density
        ))
        .map(|s| match &m.texture {
            Some(t) => format!("{s}|{}x{}|{:?}", t.width, t.height, t.image.encoded),
            None => s,
        })
    }

    fn tag(&self, id: Option<TagId>) -> Option<String> {
        self.scene.tag(id?).map(|t| t.name.clone())
    }
}

/// `name` without a trailing `#<digits>`, as `Scene::unique_name` numbers copies.
pub fn base_name(name: &str) -> &str {
    match name.rsplit_once('#') {
        Some((base, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => name,
    }
}

fn vertices(m: &Model) -> Vec<[u64; 3]> {
    m.vertices().map(|(_, v)| bits(v.pos)).collect()
}

fn sorted<T: Ord>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut v: Vec<T> = items.into_iter().collect();
    v.sort();
    v
}

/// The bits of a position, with -0 as 0.
fn bits(p: DVec3) -> [u64; 3] {
    [p.x, p.y, p.z].map(|c| (c + 0.0).to_bits())
}

fn transform_bits(t: &DAffine3) -> [u64; 12] {
    t.to_cols_array().map(|c| (c + 0.0).to_bits())
}
