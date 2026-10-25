//! Tags: named, colored groupings of edges, faces, instances, guides, annotations and section
//! planes, shown or hidden together. An entity is on at most one tag; on none it is "Untagged". Tags are
//! shared by every mesh of a scene.

use std::collections::{BTreeSet, HashMap};

use glam::DVec3;

use crate::annotation::{Dimension, Note};
use crate::edit::KernelError;
use crate::ids::{DimensionId, EntityId, InstanceId, NoteId, TagId, VertexId};
use crate::model::{ChangeSet, Model};
use crate::model::{Edge, Face, Guide, Instance};
use crate::scene::{MeshId, Scene, SceneChange, Step, enclose, placed_box};

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

/// Whether what is on `tag` (None: Untagged) shows while the tags `hidden` are hidden: Untagged
/// always does. Every visibility check goes by this; an entity inside instances is hidden too
/// when one of them is on a hidden tag (callers walking down skip such instances). An entity
/// also has its own hidden flag (`Edge::hidden` and the like), checked beside this.
pub fn tag_shown_among(hidden: &BTreeSet<TagId>, tag: Option<TagId>) -> bool {
    tag.is_none_or(|t| !hidden.contains(&t))
}

impl Model {
    /// Whether a vertex shows while the tags `hidden` are hidden: when some shown edge or
    /// shown face uses it (a vertex has no tag of its own; one used by nothing shows). With
    /// `reveal`, an edge or face hidden by its own flag counts as shown (only tags hide).
    pub fn vertex_shown(&self, v: VertexId, hidden: &BTreeSet<TagId>, reveal: bool) -> bool {
        let shown = |tag, own: bool| (reveal || !own) && tag_shown_among(hidden, tag);
        let edges = self.edges_at(v);
        let mut faces = self.faces_at(v).peekable();
        (edges.is_empty() && faces.peek().is_none())
            || edges
                .iter()
                .filter_map(|e| self.edge(*e))
                .any(|e| shown(e.tag, e.hidden))
            || faces
                .filter_map(|f| self.face(f))
                .any(|f| shown(f.tag, f.hidden))
    }

    /// Whether some edge or face is hidden by its own flag.
    pub fn has_hidden_edge_or_face(&self) -> bool {
        self.edges().any(|(_, e)| e.hidden) || self.faces().any(|(_, f)| f.hidden)
    }

    /// Puts edges, faces, instances, guides and annotations on `tag` (None: Untagged) as one
    /// ChangeSet.
    /// Vertices, unknown ids and entities already on it are skipped.
    pub(crate) fn set_tags(&mut self, ids: &[EntityId], tag: Option<TagId>) -> ChangeSet {
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

    /// The edges, faces, instances, guides, dimensions, notes and section planes on `tag`, in
    /// kind then id order.
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
            .chain(
                self.dimensions()
                    .filter(move |(_, d)| on(d.tag()))
                    .map(|(id, _)| EntityId::Dimension(id)),
            )
            .chain(
                self.notes()
                    .filter(move |(_, n)| on(n.tag()))
                    .map(|(id, _)| EntityId::Note(id)),
            )
            .chain(
                self.sections()
                    .filter(move |(_, p)| on(p.tag))
                    .map(|(id, _)| EntityId::Section(id)),
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
    /// first, and every page hiding it stops hiding it, so undo puts it all back.
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
        // Only meshes with something on the tag get an edit step.
        let meshes: Vec<MeshId> = self
            .meshes()
            .filter(|(_, m)| m.tagged(id).next().is_some())
            .map(|(m, _)| m)
            .collect();
        let mut changes = Vec::new();
        for mesh in meshes {
            changes.push(self.edit(mesh, |m| Ok(m.retag(id, move_to)))?);
        }
        changes.push(self.unhide_on_pages(id));
        let step = Step::RemoveTag(id, tag);
        self.apply_step(&step);
        changes.push(step.into());
        Ok(SceneChange::join(changes))
    }

    /// Puts edges, faces, instances, guides and annotations of mesh `mesh` on `tag` (None:
    /// Untagged).
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

    /// How many edges, faces, instances, guides and annotations carry `tag`, in the whole
    /// model (each definition counted once).
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

    /// Whether entities on `tag` (None: Untagged) are shown: Untagged always is, a tag when
    /// it is visible (see [`tag_shown_among`]). An unknown tag shows.
    pub fn tag_shown(&self, tag: Option<TagId>) -> bool {
        tag.and_then(|t| self.tag(t)).is_none_or(|t| t.visible)
    }

    /// The root instances hidden for viewing only (see [`Scene::set_view_hidden`]).
    pub fn view_hidden(&self) -> &BTreeSet<InstanceId> {
        &self.view_hidden
    }

    /// Hides the root instances `ids`, with all they place, from every view of the scene (drawing,
    /// picking, snapping, `shown_bounds`) without changing the model: nothing is saved, undone or
    /// counted as a change. A viewer isolates a part this way. Unknown ids hide nothing.
    pub fn set_view_hidden(&mut self, ids: BTreeSet<InstanceId>) {
        self.view_hidden = ids;
    }

    /// The notes hidden for viewing only (see [`Scene::set_view_hidden_notes`]).
    pub fn view_hidden_notes(&self) -> &BTreeSet<NoteId> {
        &self.view_hidden_notes
    }

    /// Hides the notes `ids` (of any mesh: note ids are unique in a scene) from every view of
    /// the scene, as [`Scene::set_view_hidden`] does instances: a viewer hides the labels of
    /// the parts it hides this way.
    pub fn set_view_hidden_notes(&mut self, ids: BTreeSet<NoteId>) {
        self.view_hidden_notes = ids;
    }

    /// Whether note `id` (`note`) shows: it is not on a hidden tag and not hidden for viewing.
    /// The instances placing it are checked by [`Scene::instance_shown`].
    pub fn note_shown(&self, id: NoteId, note: &Note) -> bool {
        self.tag_shown(note.tag()) && !self.view_hidden_notes.contains(&id)
    }

    /// The dimensions hidden for viewing only (see [`Scene::set_view_hidden_dimensions`]).
    pub fn view_hidden_dimensions(&self) -> &BTreeSet<DimensionId> {
        &self.view_hidden_dimensions
    }

    /// Hides the dimensions `ids` (of any mesh: dimension ids are unique in a scene) from
    /// every view of the scene, as [`Scene::set_view_hidden_notes`] does notes: a viewer hides
    /// what measures the parts it hides this way.
    pub fn set_view_hidden_dimensions(&mut self, ids: BTreeSet<DimensionId>) {
        self.view_hidden_dimensions = ids;
    }

    /// Whether dimension `id` (`dimension`) shows: it is not on a hidden tag and not hidden
    /// for viewing. The instances placing it are checked by [`Scene::instance_shown`].
    pub fn dimension_shown(&self, id: DimensionId, dimension: &Dimension) -> bool {
        self.tag_shown(dimension.tag()) && !self.view_hidden_dimensions.contains(&id)
    }

    /// Whether instance `id` (`instance`) of mesh `mesh` shows: it is not hidden itself, not on
    /// a hidden tag and not hidden for viewing. Every walk down the instances goes by this.
    pub fn instance_shown(&self, mesh: MeshId, id: InstanceId, instance: &Instance) -> bool {
        self.instance_shown_with(mesh, id, instance, false)
    }

    /// `instance_shown`, with an instance hidden by its own flag counting as shown when
    /// `reveal` (Hidden Geometry): hidden tags and the viewing isolation still hide it.
    pub fn instance_shown_with(
        &self,
        mesh: MeshId,
        id: InstanceId,
        instance: &Instance,
        reveal: bool,
    ) -> bool {
        (reveal || !instance.hidden)
            && self.tag_shown(instance.tag)
            && !(mesh == MeshId::Root && self.view_hidden.contains(&id))
    }

    /// Whether `instance` shows by itself: it is not hidden and not on a hidden tag (the
    /// check of [`Scene::instance_shown`] without the viewing isolation).
    pub fn placed_instance_shown(&self, instance: &Instance) -> bool {
        !instance.hidden && self.tag_shown(instance.tag)
    }

    /// Whether `face` shows: it is not hidden itself and not on a hidden tag. Instances
    /// placing it are checked by [`Scene::instance_shown`].
    pub fn face_shown(&self, face: &Face) -> bool {
        !face.hidden && self.tag_shown(face.tag)
    }

    /// Whether `edge` shows: it is not hidden itself and not on a hidden tag. A hidden edge
    /// still bounds its faces.
    pub fn edge_shown(&self, edge: &Edge) -> bool {
        !edge.hidden && self.tag_shown(edge.tag)
    }

    /// Whether `guide` shows: it is not hidden itself and not on a hidden tag.
    pub fn guide_shown(&self, guide: &Guide) -> bool {
        !guide.hidden() && self.tag_shown(guide.tag())
    }

    /// Whether mesh `mesh` or a definition it places holds an edge, face or instance with its
    /// hidden flag set (guides count in no bounds); `memo` holds the meshes answered so far.
    fn any_hidden_flag(&self, mesh: MeshId, memo: &mut HashMap<MeshId, bool>) -> bool {
        if let Some(&known) = memo.get(&mesh) {
            return known;
        }
        memo.insert(mesh, false);
        let Some(m) = self.mesh(mesh) else {
            return false;
        };
        let any = m.has_hidden_edge_or_face()
            || m.instances().any(|(_, i)| {
                i.hidden || self.any_hidden_flag(MeshId::Definition(i.definition), memo)
            });
        memo.insert(mesh, any);
        any
    }

    /// The tags that are hidden.
    pub fn hidden_tags(&self) -> BTreeSet<TagId> {
        self.tags()
            .filter(|(_, t)| !t.visible)
            .map(|(id, _)| id)
            .collect()
    }

    /// The box around what a mesh shows, in the mesh's coordinates: the ends of its shown
    /// edges, the corners of its shown faces and the shown parts of its shown instances. The
    /// instances along `open` (a path of open instances from this mesh down) count as shown
    /// whatever their tags, as an open instance stays drawn. None when it shows nothing. With
    /// no tag hidden and nothing hidden itself, `bounds`.
    pub fn shown_bounds(&self, mesh: MeshId, open: &[InstanceId]) -> Option<[DVec3; 2]> {
        self.shown_bounds_in(mesh, open, &mut HashMap::new(), &mut HashMap::new())
    }

    /// The box around what instance `instance` shows, in the coordinates of the mesh holding
    /// it (`instance_bounds` leaving out what is hidden). None when it shows nothing.
    pub fn shown_instance_bounds(&self, instance: &Instance) -> Option<[DVec3; 2]> {
        let b = self.shown_bounds(MeshId::Definition(instance.definition), &[])?;
        enclose(&placed_box(instance.transform, b))
    }

    /// `shown_bounds`, with the meshes found to hold hidden entities kept in `flagged` and the
    /// boxes of meshes with nothing open in them in `boxes` (a mesh placed many times is
    /// measured once).
    fn shown_bounds_in(
        &self,
        mesh: MeshId,
        open: &[InstanceId],
        flagged: &mut HashMap<MeshId, bool>,
        boxes: &mut HashMap<MeshId, Option<[DVec3; 2]>>,
    ) -> Option<[DVec3; 2]> {
        if open.is_empty()
            && let Some(&b) = boxes.get(&mesh)
        {
            return b;
        }
        let b = self.shown_bounds_measured(mesh, open, flagged, boxes);
        if open.is_empty() {
            boxes.insert(mesh, b);
        }
        b
    }

    /// `shown_bounds_in`, measured.
    fn shown_bounds_measured(
        &self,
        mesh: MeshId,
        open: &[InstanceId],
        flagged: &mut HashMap<MeshId, bool>,
        boxes: &mut HashMap<MeshId, Option<[DVec3; 2]>>,
    ) -> Option<[DVec3; 2]> {
        if self.view_hidden.is_empty()
            && self.tags().all(|(_, t)| t.visible)
            && !self.any_hidden_flag(mesh, flagged)
        {
            return self.bounds(mesh);
        }
        let m = self.mesh(mesh)?;
        let mut points: Vec<DVec3> = Vec::new();
        for (_, e) in m.edges().filter(|(_, e)| self.edge_shown(e)) {
            points.extend(e.v.iter().filter_map(|v| m.position(*v)));
        }
        for (_, f) in m.faces().filter(|(_, f)| self.face_shown(f)) {
            points.extend(f.outer.iter().filter_map(|v| m.position(*v)));
        }
        for (id, i) in m.instances() {
            let below = match open.split_first() {
                Some((first, rest)) if *first == id => rest,
                _ if self.instance_shown(mesh, id, i) => &[],
                _ => continue,
            };
            let def = MeshId::Definition(i.definition);
            if let Some(b) = self.shown_bounds_in(def, below, flagged, boxes) {
                points.extend(placed_box(i.transform, b));
            }
        }
        enclose(&points)
    }

    /// Whether hidden tags or hidden entities hide a face that would otherwise be placed in the
    /// model (hidden or on a hidden tag itself, or placed by an instance that is).
    pub fn hides_faces(&self) -> bool {
        let mut memo = std::collections::HashMap::new();
        self.hides_faces_in(MeshId::Root, &mut memo)
    }

    /// Whether mesh `id`, placed and shown, has a face hidden by a tag; `memo` holds the meshes
    /// answered so far.
    fn hides_faces_in(
        &self,
        id: MeshId,
        memo: &mut std::collections::HashMap<MeshId, bool>,
    ) -> bool {
        if let Some(&known) = memo.get(&id) {
            return known;
        }
        memo.insert(id, false);
        let Some(m) = self.mesh(id) else { return false };
        let hides = m.faces().any(|(_, f)| !self.face_shown(f))
            || m.instances().any(|(_, i): (_, &Instance)| {
                let def = MeshId::Definition(i.definition);
                if self.placed_instance_shown(i) {
                    self.hides_faces_in(def, memo)
                } else {
                    self.places_faces(def)
                }
            });
        memo.insert(id, hides);
        hides
    }

    /// Whether mesh `id` places a face, its own or an instance's at any depth.
    fn places_faces(&self, id: MeshId) -> bool {
        self.mesh(id).is_some_and(|m| {
            m.faces().next().is_some()
                || m.instances()
                    .any(|(_, i)| self.places_faces(MeshId::Definition(i.definition)))
        })
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
