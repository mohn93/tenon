//! Make Unique and Flip Along.

use std::collections::BTreeSet;

use glam::{DAffine3, DVec3};
use kernel::{EntityId, Instance, MeshId, Parts, SceneChange};

use super::Document;
use crate::command::{CommandError, ErrorKind, FlipAxis, FlipFrame};

impl Document {
    /// `ids`, or the selection when there are none.
    fn ids_or_selection(&self, ids: Option<&[EntityId]>) -> Vec<EntityId> {
        match ids {
            Some(ids) => ids.to_vec(),
            None => self.selection.iter().copied().collect(),
        }
    }

    /// The current drawing axes as a frame in the open context's coordinates: its origin and
    /// its x, y and z axes as columns. These are the context's own axes until model axes exist.
    fn drawing_frame(&self) -> DAffine3 {
        DAffine3::IDENTITY
    }

    /// Gives the shared component instances among `ids` definitions of their own.
    pub(super) fn make_unique_changes(
        &mut self,
        ids: Option<&[EntityId]>,
    ) -> Result<SceneChange, CommandError> {
        let ids = self.ids_or_selection(ids);
        let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
        if !missing.is_empty() {
            return Err(
                CommandError::new(ErrorKind::NotFound, "no such entities").with_ids(missing)
            );
        }
        let parts = Parts::of(&ids);
        let mesh = self.active();
        let (made, change) = self
            .scene
            .make_unique(mesh, &parts.instances)
            .map_err(|e| self.kernel_error(e, &ids))?;
        self.made_definitions = Some(made);
        Ok(change)
    }

    /// Mirrors `ids` along `axis` of the current drawing axes about the center of their box, or
    /// (`FlipFrame::Own`) one instance along its own axis about its own center.
    pub(super) fn flip_changes(
        &mut self,
        ids: Option<&[EntityId]>,
        axis: FlipAxis,
        frame: FlipFrame,
    ) -> Result<SceneChange, CommandError> {
        if ids.is_some_and(<[EntityId]>::is_empty) {
            self.changed_entities = Some(Vec::new());
            return Ok(SceneChange::default());
        }
        let ids = self.ids_or_selection(ids);
        let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
        if ids.is_empty() {
            return Err(invalid("nothing is selected: select something or give ids"));
        }
        let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
        if !missing.is_empty() {
            return Err(
                CommandError::new(ErrorKind::NotFound, "no such entities").with_ids(missing)
            );
        }
        let ids: Vec<EntityId> = ids
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mirror = match frame {
            FlipFrame::Axes => self
                .axes_mirror(&ids, axis.index(), self.drawing_frame())
                .ok_or_else(|| invalid("nothing to flip"))?,
            FlipFrame::Own => self.own_mirror(&ids, axis.index())?,
        };
        let vertices = self.vertex_ids(&ids);
        let parts = Parts::of(&ids);
        self.check_placed(&vertices, &parts, &[mirror], false)?;
        let change = self.transform_changes(&ids, &vertices, &parts, mirror)?;
        self.changed_entities = Some(ids);
        Ok(change)
    }

    /// The mirror across the plane normal to axis `k` of `frame` (a frame in the open context's
    /// coordinates) through the center of the box of `ids` measured in that frame. None when
    /// `ids` hold nothing with a position.
    fn axes_mirror(&self, ids: &[EntityId], k: usize, frame: DAffine3) -> Option<DAffine3> {
        let inverse = frame.inverse();
        let m = self.model();
        let vertices = self.vertex_ids(ids);
        let parts = Parts::of(ids);
        let mut points: Vec<DVec3> = vertices
            .iter()
            .filter_map(|v| m.position(*v))
            .map(|p| inverse.transform_point3(p))
            .collect();
        points.extend(
            parts
                .guides
                .iter()
                .filter_map(|g| m.guide(*g))
                .map(|g| inverse.transform_point3(g.anchor())),
        );
        points.extend(
            parts
                .sections
                .iter()
                .filter_map(|p| m.section(*p))
                .map(|p| inverse.transform_point3(p.point)),
        );
        points.extend(self.annotation_points_placed(
            &parts.dimensions,
            &parts.notes,
            inverse,
            &|_| true,
        ));
        for id in &parts.instances {
            let Some(i) = m.instance(*id) else { continue };
            let placed = Instance::new(i.definition, inverse * i.transform);
            match self.scene.instance_bounds(&placed) {
                Some(b) => points.extend(b),
                None => points.push(placed.transform.translation),
            }
            points.extend(self.scene.instance_guide_anchors(&placed));
        }
        let first = *points.first()?;
        let (lo, hi) = points
            .iter()
            .fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        let center = (lo + hi) / 2.0;
        let mut flip = DVec3::ONE;
        flip[k] = -1.0;
        let about = DAffine3::from_translation(center)
            * DAffine3::from_scale(flip)
            * DAffine3::from_translation(-center);
        Some(frame * about * inverse)
    }

    /// The mirror of the one instance in `ids` about its own center, across the plane normal to
    /// its own axis `k`, in the open context's coordinates.
    fn own_mirror(&self, ids: &[EntityId], k: usize) -> Result<DAffine3, CommandError> {
        let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
        let [EntityId::Instance(id)] = ids else {
            return Err(
                invalid("flipping along the component's axes takes exactly one instance")
                    .with_ids(ids.to_vec()),
            );
        };
        let t = self.model().instance(*id).expect("checked to exist");
        let local = self
            .scene
            .bounds(MeshId::Definition(t.definition))
            .map_or(DVec3::ZERO, |[lo, hi]| (lo + hi) / 2.0);
        let mut flip = DVec3::ONE;
        flip[k] = -1.0;
        // The mirror in the definition's own frame, about its center, brought into the context.
        let own = DAffine3::from_translation(local)
            * DAffine3::from_scale(flip)
            * DAffine3::from_translation(-local);
        Ok(t.transform * own * t.transform.inverse())
    }
}
