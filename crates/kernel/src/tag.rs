//! Tags: named, colored groupings of edges, faces, instances and guides, shown or hidden
//! together. An entity is on at most one tag; on none it is "Untagged". Tags are shared by
//! every mesh of a scene.

use std::collections::BTreeSet;

use crate::edit::KernelError;
use crate::ids::{EntityId, TagId};
use crate::model::{ChangeSet, Model};
use crate::scene::{MeshId, Scene, SceneChange, Step};

/// The longest tag name, in characters.
pub const MAX_TAG_NAME: usize = 256;

/// The name of the absence of a tag; no tag may take it.
pub const UNTAGGED: &str = "Untagged";

/// The colors new tags take, in order: the first no tag has yet, or after all twelve, the
/// next in turn.
pub const TAG_COLORS: [[u8; 3]; 12] = [
    [230, 25, 75],
    [60, 180, 75],
    [0, 130, 200],
    [245, 130, 48],
    [145, 30, 180],
    [70, 200, 200],
    [240, 50, 230],
    [170, 110, 40],
    [128, 128, 0],
    [0, 0, 128],
    [255, 200, 25],
    [128, 128, 128],
];

/// A named color entities can be put on; hidden, everything on it is hidden.
#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    pub name: String,
    /// sRGB red, green and blue.
    pub color: [u8; 3],
    pub visible: bool,
}

/// The parts of a tag `Scene::change_tag` sets; None keeps that part.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TagEdit<'a> {
    pub name: Option<&'a str>,
    pub color: Option<[u8; 3]>,
    pub visible: Option<bool>,
}

/// Fails, saying why, when a tag name is blank, longer than [`MAX_TAG_NAME`] characters, or
/// [`UNTAGGED`].
pub(crate) fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("a tag needs a name".into());
    }
    if name.chars().count() > MAX_TAG_NAME {
        return Err(format!(
            "a tag name may have at most {MAX_TAG_NAME} characters"
        ));
    }
    if name == UNTAGGED {
        return Err(format!("{UNTAGGED:?} is not a tag: it is having none"));
    }
    Ok(())
}

impl Model {
    /// Puts edges, faces, instances and guides on `tag` (None: Untagged) as one ChangeSet.
    /// Vertices, unknown ids and entities already on it are skipped.
    pub fn set_tags(&mut self, ids: &[EntityId], tag: Option<TagId>) -> ChangeSet {
        for &id in ids.iter().collect::<BTreeSet<_>>() {
            self.set_tag(id, tag);
        }
        self.take_changes()
    }

    /// Moves everything on tag `from` to `to` (None: Untagged), as one ChangeSet.
    pub(crate) fn retag(&mut self, from: TagId, to: Option<TagId>) -> ChangeSet {
        let on: Vec<EntityId> = self.tagged(from).collect();
        for id in on {
            self.set_tag(id, to);
        }
        self.take_changes()
    }

    /// The edges, faces, instances and guides on `tag`, in kind then id order.
    pub fn tagged(&self, tag: TagId) -> impl Iterator<Item = EntityId> + '_ {
        let on = move |t: Option<TagId>| t == Some(tag);
        self.edges()
            .filter(move |(_, e)| on(e.tag))
            .map(|(id, _)| EntityId::Edge(id))
            .chain(
                self.faces()
                    .filter(move |(_, f)| on(f.tag))
                    .map(|(id, _)| EntityId::Face(id)),
            )
            .chain(
                self.instances()
                    .filter(move |(_, i)| on(i.tag))
                    .map(|(id, _)| EntityId::Instance(id)),
            )
            .chain(
                self.guides()
                    .filter(move |(_, g)| on(g.tag()))
                    .map(|(id, _)| EntityId::Guide(id)),
            )
    }
}

impl Scene {
    /// Adds `tag` after checking its name (valid, see [`check_name`], and not taken).
    pub fn add_tag(&mut self, tag: Tag) -> Result<(TagId, SceneChange), KernelError> {
        self.check_free_tag_name(&tag.name, None)?;
        let id = self.alloc_tag();
        let step = Step::AddTag(id, tag);
        self.apply_step(&step);
        Ok((id, step.into()))
    }

    /// Changes the parts of a tag `edit` gives, checking a new name as `add_tag` does. One
    /// step, so undo restores every part; nothing to change changes nothing.
    pub fn change_tag(&mut self, id: TagId, edit: TagEdit) -> Result<SceneChange, KernelError> {
        let from = self.tag(id).ok_or(KernelError::TagNotFound)?.clone();
        if let Some(name) = edit.name {
            self.check_free_tag_name(name, Some(id))?;
        }
        let to = Tag {
            name: edit.name.map_or_else(|| from.name.clone(), str::to_string),
            color: edit.color.unwrap_or(from.color),
            visible: edit.visible.unwrap_or(from.visible),
        };
        if to == from {
            return Ok(SceneChange::default());
        }
        let step = Step::ChangeTag(id, from, to);
        self.apply_step(&step);
        Ok(step.into())
    }

    /// Removes a tag. Everything on it, in every mesh, moves to `move_to` (None: Untagged)
    /// first, so undo puts it back.
    pub fn delete_tag(
        &mut self,
        id: TagId,
        move_to: Option<TagId>,
    ) -> Result<SceneChange, KernelError> {
        let tag = self.tag(id).ok_or(KernelError::TagNotFound)?.clone();
        if move_to == Some(id) {
            return Err(KernelError::TagName(
                "entities cannot move to the tag being deleted".into(),
            ));
        }
        if move_to.is_some_and(|t| self.tag(t).is_none()) {
            return Err(KernelError::TagNotFound);
        }
        let meshes: Vec<MeshId> = self.meshes().map(|(m, _)| m).collect();
        let mut changes = Vec::new();
        for mesh in meshes {
            changes.push(self.edit(mesh, |m| Ok(m.retag(id, move_to)))?);
        }
        let step = Step::RemoveTag(id, tag);
        self.apply_step(&step);
        changes.push(step.into());
        Ok(SceneChange::join(changes))
    }

    /// Puts edges, faces, instances and guides of mesh `mesh` on `tag` (None: Untagged).
    /// Vertices and unknown ids are skipped.
    pub fn set_tags(
        &mut self,
        mesh: MeshId,
        ids: &[EntityId],
        tag: Option<TagId>,
    ) -> Result<SceneChange, KernelError> {
        if tag.is_some_and(|t| self.tag(t).is_none()) {
            return Err(KernelError::TagNotFound);
        }
        self.edit(mesh, |m| Ok(m.set_tags(ids, tag)))
    }

    /// How many edges, faces, instances and guides carry `tag`, in the whole model (each
    /// definition counted once).
    pub fn tag_count(&self, tag: TagId) -> usize {
        self.meshes().map(|(_, m)| m.tagged(tag).count()).sum()
    }

    /// `base` if no tag has that name, else `base` followed by the first free number from 2
    /// ("Tag 2").
    pub fn free_tag_name(&self, base: &str) -> String {
        let taken: BTreeSet<&str> = self.tags().map(|(_, t)| t.name.as_str()).collect();
        if !taken.contains(base) {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|n| !taken.contains(n.as_str()))
            .expect("a free name")
    }

    /// The color a new tag takes: the first of [`TAG_COLORS`] no tag has, or once every one
    /// is used, the next in turn.
    pub fn next_tag_color(&self) -> [u8; 3] {
        let used: BTreeSet<[u8; 3]> = self.tags().map(|(_, t)| t.color).collect();
        TAG_COLORS
            .iter()
            .copied()
            .find(|c| !used.contains(c))
            .unwrap_or(TAG_COLORS[self.tags().count() % TAG_COLORS.len()])
    }

    fn check_free_tag_name(&self, name: &str, except: Option<TagId>) -> Result<(), KernelError> {
        check_name(name).map_err(KernelError::TagName)?;
        let taken = self
            .tags()
            .any(|(id, t)| Some(id) != except && t.name == name);
        if taken {
            return Err(KernelError::TagName(format!(
                "a tag named {name:?} already exists"
            )));
        }
        Ok(())
    }
}
