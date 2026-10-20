//! Copy (clip text of a selection) and paste.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use glam::{DAffine3, DVec3};
use kernel::{DefinitionId, EntityId, MeshId, SceneChange};

use super::{Document, check_coordinates};
use crate::clip;
use crate::command::{CommandError, ErrorKind};

impl Document {
    /// The token naming this document since it was created, opened or imported as a new
    /// model (not saved). A clip records it, so pasting into the same document reuses its
    /// component definitions, and pasting a clip again reuses those its last paste made.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The clip text of `ids` (edges, faces, instances, guides and annotations of the open
    /// context): them,
    /// in world coordinates, with every definition, material and tag they use. An empty list
    /// is `invalid_input`; vertex ids are `invalid_input` and unknown ids `not_found`, as for
    /// `erase`.
    pub fn clip(&self, ids: &[EntityId]) -> Result<String, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one edge, face, instance or guide",
            ));
        }
        let parts = self.parts(ids)?;
        let invalid = |e: String| CommandError::new(ErrorKind::InvalidInput, e);
        let scene = self
            .scene
            .extract_with(self.active(), &parts, self.context_transform())
            .map_err(|e| invalid(e.to_string()))?;
        clip::encode(&scene, &self.token, &self.camera, self.view_look()).map_err(invalid)
    }

    /// Pastes clip text into the open context: its base point at `at` (moved only), or, with
    /// no `at`, where it was copied from in world coordinates. Selects what it pasted.
    pub(super) fn paste_changes(
        &mut self,
        text: &str,
        at: Option<[f64; 3]>,
    ) -> Result<SceneChange, CommandError> {
        if let Some(at) = at {
            check_coordinates(&[at])?;
        }
        let clip = clip::decode(text)?;
        let placement = match at {
            Some(at) => DAffine3::from_translation(DVec3::from(at) - clip.base),
            None => self.context_transform().inverse(),
        };
        // What lands in the open context must keep to the limit commands keep to. Each
        // definition's box is worked out once, so a nested clip costs no more than its size.
        let root = clip.scene.mesh(MeshId::Root).expect("a scene has a root");
        let boxes = clip::definition_boxes(&clip.scene);
        let at = |p: DVec3| placement.transform_point3(p).to_array();
        let mut points: Vec<[f64; 3]> = root.vertices().map(|(_, v)| at(v.pos)).collect();
        points.extend(root.guides().map(|(_, g)| at(g.anchor())));
        points.extend(root.annotation_points().into_iter().map(at));
        for (_, i) in root.instances() {
            let placed = placement * i.transform;
            match boxes.get(&i.definition).copied().flatten() {
                Some(b) => points.extend(clip::corners(placed, b).map(|p| p.to_array())),
                None => points.push(placed.translation.to_array()),
            }
        }
        check_coordinates(&points)?;

        // A clip definition is placed as the definition it was copied from (when the clip is
        // from this document) or the one an earlier paste of it made; the kernel copies it when
        // that is gone or no longer a component.
        let same_document = clip.source == self.token;
        let offered: BTreeMap<DefinitionId, DefinitionId> = clip
            .scene
            .definitions()
            .filter_map(|(id, _)| {
                let original = (same_document && self.scene.definition(id).is_some()).then_some(id);
                let earlier = || self.pasted.get(&(clip.source.clone(), id)).copied();
                Some((id, original.or_else(earlier)?))
            })
            .collect();
        let reuse = |id| offered.get(&id).copied();
        let parent = self.active();
        let (inserted, change) = self
            .scene
            .insert(parent, &clip.scene, placement, &reuse)
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))?;
        for (from, to) in &inserted.definitions {
            if !(same_document && from == to) {
                self.pasted.insert((clip.source.clone(), *from), *to);
            }
        }
        self.selection = inserted
            .edges
            .iter()
            .map(|e| EntityId::Edge(*e))
            .chain(inserted.faces.iter().map(|f| EntityId::Face(*f)))
            .chain(inserted.instances.iter().map(|i| EntityId::Instance(*i)))
            .chain(inserted.guides.iter().map(|g| EntityId::Guide(*g)))
            .chain(inserted.dimensions.iter().map(|r| EntityId::Dimension(*r)))
            .chain(inserted.notes.iter().map(|n| EntityId::Note(*n)))
            .collect::<BTreeSet<_>>();
        Ok(change)
    }
}

/// A token no other document of this process, or of another process at another time, has:
/// the process id, the time and a count.
pub(super) fn new_token() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "{:x}-{nanos:x}-{:x}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
