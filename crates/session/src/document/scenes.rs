//! Scenes: saved views and tag visibility, gone back to in one step.

use std::collections::BTreeMap;

use kernel::{
    EntityId, InstanceId, JointId, JointKind, KernelError, MeshId, Page, PageId, PageSections,
    PageView, SceneChange, SceneTransition, TagEdit,
};

use glam::DAffine3;

/// What a scene stores of joints: each joint's (value, slide), radians or meters.
type JointValues = BTreeMap<JointId, (f64, f64)>;

use glam::DVec3;

use super::{Document, MAX_COORDINATE, unit_vector};
use crate::camera::{Camera, Lighting, Projection};
use crate::command::{CommandError, ErrorKind, SceneJoints, ScenePositions};
use crate::motion::{JointPose, Motion};
use crate::query::{
    QueryResult, SceneInfo, SceneJointValue, SceneSectionPlane, SceneSections, SceneView,
};

use super::sections::SectionFillColor;

impl Document {
    /// The scene last added or gone to, while it exists.
    pub fn selected_scene(&self) -> Option<PageId> {
        self.selected_scene
            .filter(|id| self.scene.page(*id).is_some())
    }

    /// The view a scene stores now: the camera, lighting and color by tag.
    fn current_view(&self) -> PageView {
        self.camera.page_view(
            self.lighting,
            self.color_by_tag,
            self.x_ray,
            self.hidden_geometry,
            self.shadows,
            self.style(),
        )
    }

    /// The current view to store, refused (`invalid_input`) when the eye or target is beyond
    /// `MAX_COORDINATE`, the limit a file's scenes keep to (window navigation can go past it).
    fn view_to_store(&self) -> Result<PageView, CommandError> {
        let view = self.current_view();
        if view
            .eye
            .iter()
            .chain(&view.target)
            .any(|x| x.abs() > MAX_COORDINATE)
        {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!(
                    "the camera is beyond +/-{MAX_COORDINATE} m: a scene cannot store it (move \
                     the view closer to the model)"
                ),
            ));
        }
        Ok(view)
    }

    /// The positions a scene stores: the current transforms of the instances `ids` names (each
    /// an instance of the root), or of every instance of the root. `None` when there are none.
    fn current_positions(
        &self,
        ids: Option<&[EntityId]>,
    ) -> Result<Option<BTreeMap<InstanceId, DAffine3>>, CommandError> {
        let root = self.scene.root();
        let map: BTreeMap<InstanceId, DAffine3> = match ids {
            None => root.instances().map(|(id, i)| (id, i.transform)).collect(),
            Some([]) => {
                return Err(CommandError::new(
                    ErrorKind::InvalidInput,
                    "positions must list at least one instance (or be true or false)",
                ));
            }
            Some(ids) => {
                let mut map = BTreeMap::new();
                let mut bad = Vec::new();
                for e in ids {
                    match e {
                        EntityId::Instance(i) if root.instance(*i).is_some() => {
                            map.insert(*i, root.instance(*i).expect("checked").transform);
                        }
                        _ => bad.push(*e),
                    }
                }
                if !bad.is_empty() {
                    return Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        "positions takes only instances of the root (close any open group first)",
                    )
                    .with_ids(bad));
                }
                map
            }
        };
        Ok((!map.is_empty()).then_some(map))
    }

    /// The positions a scene stores after `arg`, given what it stored (`stored`): absent keeps
    /// them, false drops them, true stores the stored set (every instance of the root when
    /// none) again from the current transforms, ids store those instances.
    fn positions_after(
        &self,
        arg: Option<&ScenePositions>,
        stored: Option<&BTreeMap<InstanceId, DAffine3>>,
        keep_absent: bool,
    ) -> Result<Option<BTreeMap<InstanceId, DAffine3>>, CommandError> {
        match arg {
            None if keep_absent => Ok(stored.cloned()),
            Some(ScenePositions::Flag(false)) => Ok(None),
            None | Some(ScenePositions::Flag(true)) => {
                let same: Option<Vec<EntityId>> =
                    stored.map(|m| m.keys().copied().map(EntityId::Instance).collect());
                self.current_positions(same.as_deref())
            }
            Some(ScenePositions::Ids(ids)) => self.current_positions(Some(ids)),
        }
    }

    /// The joint values a scene stores: the current values of the joints `ids` names, or of
    /// every joint that has a value and is not a cam follower. `None` when there are none.
    fn current_joints(&self, ids: Option<&[JointId]>) -> Result<Option<JointValues>, CommandError> {
        let invalid = |why: String| CommandError::new(ErrorKind::InvalidInput, why);
        let map: JointValues = match ids {
            None => self
                .scene
                .joints()
                .filter(|(id, j)| j.kind.has_value() && self.scene.follower_of(*id).is_none())
                .map(|(id, j)| (id, (j.value, j.slide)))
                .collect(),
            Some([]) => {
                return Err(invalid(
                    "joints must list at least one joint (or be true or false)".into(),
                ));
            }
            Some(ids) => {
                let mut map = BTreeMap::new();
                for id in ids {
                    let joint = self.scene.joint(*id).ok_or_else(|| {
                        CommandError::new(ErrorKind::NotFound, format!("no joint {id}"))
                    })?;
                    self.scene
                        .page_joint_problem(*id, joint.value, joint.slide)
                        .map_err(invalid)?;
                    map.insert(*id, (joint.value, joint.slide));
                }
                map
            }
        };
        Ok((!map.is_empty()).then_some(map))
    }

    /// The joint values a scene stores after `arg`, given what it stored (`stored`), as
    /// `positions_after` does for positions.
    fn joints_after(
        &self,
        arg: Option<&SceneJoints>,
        stored: Option<&JointValues>,
        keep_absent: bool,
    ) -> Result<Option<JointValues>, CommandError> {
        match arg {
            None if keep_absent => Ok(stored.cloned()),
            Some(SceneJoints::Flag(false)) => Ok(None),
            None | Some(SceneJoints::Flag(true)) => {
                let same: Option<Vec<JointId>> = stored.map(|m| m.keys().copied().collect());
                self.current_joints(same.as_deref())
            }
            Some(SceneJoints::Ids(ids)) => self.current_joints(Some(ids)),
        }
    }

    /// The section planes a scene stores now: the active plane of each mesh that has one, and
    /// the section display.
    fn current_sections(&self) -> PageSections {
        let d = self.section_display;
        PageSections {
            active: self
                .scene
                .meshes()
                .filter_map(|(id, m)| Some((id, m.active_section()?)))
                .collect(),
            planes: d.planes,
            cuts: d.cuts,
            fill: d.fill,
            material_fill: d.fill_color == SectionFillColor::Material,
        }
    }

    /// Adds a scene after the last one, storing the current view (`camera`) and hidden tags
    /// (`tags`); without `name` it is "Scene 1" (or the lowest free number). It becomes the
    /// selected scene.
    pub(super) fn add_scene_changes(
        &mut self,
        name: Option<&str>,
        camera: bool,
        tags: bool,
        positions: Option<&ScenePositions>,
        joints: Option<&SceneJoints>,
        sections: Option<bool>,
    ) -> Result<SceneChange, CommandError> {
        let page = Page {
            name: name.map_or_else(|| self.scene.free_page_name(), str::to_string),
            sections: (sections == Some(true)).then(|| self.current_sections()),
            view: camera.then(|| self.view_to_store()).transpose()?,
            tags: tags.then(|| self.scene.hidden_tags()),
            positions: match positions {
                None => None,
                p => self.positions_after(p, None, false)?,
            },
            joints: match joints {
                None => None,
                j => self.joints_after(j, None, false)?,
            },
        };
        let (id, change) = self
            .scene
            .add_page(page)
            .map_err(|e| scene_error(e, None))?;
        self.selected_scene = Some(id);
        Ok(change)
    }

    /// Stores the current view and hidden tags in a scene again; a part absent is stored again
    /// when the scene stores it, false stops storing it.
    pub(super) fn update_scene_changes(
        &mut self,
        id: PageId,
        camera: Option<bool>,
        tags: Option<bool>,
        positions: Option<&ScenePositions>,
        joints: Option<&SceneJoints>,
        sections: Option<bool>,
    ) -> Result<SceneChange, CommandError> {
        let page = self.page(id)?;
        let sections = sections
            .unwrap_or(page.sections.is_some())
            .then(|| self.current_sections());
        let camera = camera.unwrap_or(page.view.is_some());
        let tags = tags.unwrap_or(page.tags.is_some());
        let stored = page.positions.as_ref();
        // Absent: stored again only when the scene stores positions.
        let positions = match positions {
            None if stored.is_none() => None,
            p => self.positions_after(p, stored, false)?,
        };
        let stored_joints = page.joints.as_ref();
        let joints = match joints {
            None if stored_joints.is_none() => None,
            j => self.joints_after(j, stored_joints, false)?,
        };
        let page = Page {
            name: page.name.clone(),
            view: camera.then(|| self.view_to_store()).transpose()?,
            tags: tags.then(|| self.scene.hidden_tags()),
            positions,
            joints,
            sections,
        };
        self.scene
            .change_page(id, page)
            .map_err(|e| scene_error(e, Some(id)))
    }

    /// Renames a scene and chooses what it stores: a part turned off is dropped, a part
    /// turned on is stored from the current state only when the scene lacks it, and a part that
    /// stays keeps what was stored.
    pub(super) fn change_scene_changes(
        &mut self,
        id: PageId,
        name: Option<&str>,
        camera: Option<bool>,
        tags: Option<bool>,
        positions: Option<&ScenePositions>,
        joints: Option<&SceneJoints>,
        sections: Option<bool>,
    ) -> Result<SceneChange, CommandError> {
        if name.is_none()
            && camera.is_none()
            && tags.is_none()
            && positions.is_none()
            && joints.is_none()
            && sections.is_none()
        {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "change_scene needs a name, camera, tags, positions, joints or sections",
            ));
        }
        let page = self.page(id)?;
        let view = match camera {
            Some(false) => None,
            Some(true) => Some(match page.view {
                Some(v) => v,
                None => self.view_to_store()?,
            }),
            None => page.view,
        };
        let hidden = match tags {
            Some(false) => None,
            Some(true) => Some(
                page.tags
                    .clone()
                    .unwrap_or_else(|| self.scene.hidden_tags()),
            ),
            None => page.tags.clone(),
        };
        let positions = self.positions_after(positions, page.positions.as_ref(), true)?;
        let joints = self.joints_after(joints, page.joints.as_ref(), true)?;
        let sections = match sections {
            Some(false) => None,
            Some(true) => Some(
                page.sections
                    .clone()
                    .unwrap_or_else(|| self.current_sections()),
            ),
            None => page.sections.clone(),
        };
        let page = Page {
            name: name.map_or_else(|| page.name.clone(), str::to_string),
            view,
            tags: hidden,
            positions,
            joints,
            sections,
        };
        self.scene
            .change_page(id, page)
            .map_err(|e| scene_error(e, Some(id)))
    }

    pub(super) fn move_scene_changes(
        &mut self,
        id: PageId,
        index: usize,
    ) -> Result<SceneChange, CommandError> {
        self.scene
            .move_page(id, index)
            .map_err(|e| scene_error(e, Some(id)))
    }

    pub(super) fn delete_scene_changes(&mut self, id: PageId) -> Result<SceneChange, CommandError> {
        self.scene
            .delete_page(id)
            .map_err(|e| scene_error(e, Some(id)))
    }

    /// Goes to a scene: sets the camera, projection, lighting and color by tag it stores (not
    /// part of the returned change), and returns the tag visibility changes it makes. It
    /// becomes the selected scene. Nothing changes on failure.
    /// The camera a scene's stored view makes, with the view (the view's own numbers kept bit
    /// for bit); `None` when the scene stores no view.
    pub(super) fn scene_camera(
        &self,
        id: PageId,
        page: &Page,
    ) -> Result<Option<(Camera, PageView)>, CommandError> {
        Ok(match &page.view {
            Some(v) => {
                let projection = if v.parallel {
                    Projection::Parallel
                } else {
                    Projection::Perspective
                };
                let degrees = v.fov_y.to_degrees();
                let mut camera = self
                    .camera_for(v.eye, v.target, Some(degrees), Some(v.up), Some(projection))
                    .map_err(|e| {
                        CommandError::new(
                            ErrorKind::InvalidInput,
                            format!("scene {id}'s view cannot be used: {}", e.message),
                        )
                    })?;
                // The stored values as they are: through degrees the field of view could drift
                // by a bit, and an update would then store a change. A vertical view's up is
                // the stored one (horizontal, checked by `camera_for`); any other view's is
                // straight up.
                camera.fov_y = v.fov_y;
                let up = DVec3::from(v.up);
                if camera.is_vertical() && up.z == 0.0 {
                    // A unit up (as stored from a camera) is kept bit for bit.
                    camera.up = if (up.length() - 1.0).abs() < 1e-12 {
                        up
                    } else {
                        unit_vector(up).unwrap_or(camera.up)
                    };
                }
                Some((camera, *v))
            }
            None => None,
        })
    }

    pub(super) fn go_to_scene_changes(&mut self, id: PageId) -> Result<SceneChange, CommandError> {
        let page = self.page(id)?.clone();
        let view = self.scene_camera(id, &page)?;
        let mut changes = Vec::new();
        if let Some(hidden) = &page.tags {
            let flips: Vec<_> = self
                .scene
                .tags()
                .filter(|(t, tag)| tag.visible == hidden.contains(t))
                .map(|(t, tag)| (t, !tag.visible))
                .collect();
            for (t, visible) in flips {
                let edit = TagEdit {
                    visible: Some(visible),
                    ..TagEdit::default()
                };
                changes.push(
                    self.scene
                        .change_tag(t, edit)
                        .expect("showing or hiding a tag cannot fail"),
                );
            }
        }
        for (i, t) in page.positions.iter().flatten() {
            if self.scene.root().instance(*i).is_some() {
                changes.push(
                    self.scene
                        .set_instance_transform(MeshId::Root, *i, *t)
                        .expect("moving an instance of the root to a finite transform"),
                );
            }
        }
        if page.positions.is_some() {
            // The joints' values follow the stored pose (measured, not solved).
            changes.push(self.scene.update_joint_values());
        }
        if let Some(stored) = &page.joints {
            // The stored joint values are then solved for, on the parts as the positions left
            // them. A pose that cannot be met refuses the whole go-to: nothing changes.
            let values: Vec<(JointId, f64, Option<f64>)> = stored
                .iter()
                .map(|(j, (v, s))| (*j, *v, Some(*s)))
                .collect();
            match self.scene.drive_joints(&values) {
                Ok(change) => changes.push(change),
                Err(e) => {
                    self.scene.apply(&SceneChange::join(changes).inverse());
                    return Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        format!("scene {id} cannot be gone to: {e}"),
                    ));
                }
            }
        }
        if let Some(stored) = &page.sections {
            // Exactly the stored planes are active; a mesh with another one active drops it.
            let contexts: Vec<(MeshId, Option<kernel::SectionId>)> = self
                .scene
                .meshes()
                .map(|(m, model)| (m, model.active_section()))
                .collect();
            for (mesh, now) in contexts {
                let want = stored.active.get(&mesh).copied();
                if want != now {
                    changes.push(
                        self.scene
                            .set_active_section(mesh, want)
                            .expect("activating a plane the scene checked exists"),
                    );
                }
            }
            let d = &mut self.section_display;
            d.planes = stored.planes;
            d.cuts = stored.cuts;
            d.fill = stored.fill;
            d.fill_color = if stored.material_fill {
                SectionFillColor::Material
            } else {
                SectionFillColor::Dark
            };
        }
        if let Some((camera, v)) = view {
            let v = &v;
            self.camera = camera;
            self.lighting = lighting_of(*v);
            self.color_by_tag = v.color_by_tag;
            self.x_ray = v.x_ray;
            self.hidden_geometry = v.hidden_geometry;
            self.shadows = v.shadows;
            // A scene's style exists (the model checks), so the choice is exact.
            self.style = v.style;
            // A scene with it off drops hidden entities from the selection, as turning it off.
            self.prune_hidden_selection();
        }
        self.selected_scene = Some(id);
        Ok(SceneChange::join(changes))
    }

    /// `frame` (a copy of the model to draw) with the section planes scene `id` stores active,
    /// and the section display it stores; `None` when the scene stores none (or does not
    /// exist). The document is untouched.
    pub fn frame_with_scene_sections(
        &self,
        mut frame: kernel::Scene,
        id: PageId,
    ) -> Option<(kernel::Scene, super::SectionDisplay)> {
        let stored = self.scene.page(id)?.sections.as_ref()?;
        let contexts: Vec<MeshId> = frame.meshes().map(|(m, _)| m).collect();
        for mesh in contexts {
            let want = stored.active.get(&mesh).copied();
            if frame.mesh(mesh).and_then(|m| m.active_section()) != want {
                let _ = frame.set_active_section(mesh, want);
            }
        }
        let display = super::SectionDisplay {
            planes: stored.planes,
            cuts: stored.cuts,
            fill: stored.fill,
            fill_color: if stored.material_fill {
                SectionFillColor::Material
            } else {
                SectionFillColor::Dark
            },
        };
        Some((frame, display))
    }

    /// Every scene in order, with what it stores and whether it is selected.
    pub(super) fn scene_infos(&self) -> Vec<SceneInfo> {
        let selected = self.selected_scene();
        self.scene
            .pages()
            .map(|(id, p)| SceneInfo {
                id,
                name: p.name.clone(),
                camera: p.view.is_some(),
                tags: p.tags.is_some(),
                positions: p
                    .positions
                    .iter()
                    .flat_map(|m| m.keys().copied().map(EntityId::Instance))
                    .collect(),
                joints: p.joints.iter().flat_map(|m| m.keys().copied()).collect(),
                sections: p.sections.is_some(),
                selected: selected == Some(id),
            })
            .collect()
    }

    /// The `scene` query: the view and hidden tags a scene stores.
    pub(super) fn scene_detail(&self, id: PageId) -> Result<QueryResult, CommandError> {
        let page = self.page(id)?;
        Ok(QueryResult::Scene {
            id,
            name: page.name.clone(),
            view: page.view.map(|v| SceneView {
                eye: v.eye,
                target: v.target,
                up: v.up,
                fov_y_deg: v.fov_y.to_degrees(),
                projection: if v.parallel {
                    Projection::Parallel
                } else {
                    Projection::Perspective
                },
                lighting: lighting_of(v),
                color_by_tag: v.color_by_tag,
                x_ray: v.x_ray,
                hidden_geometry: v.hidden_geometry,
                shadows: v.shadows.into(),
                style: v.style,
            }),
            hidden_tags: page.tags.as_ref().map(|t| t.iter().copied().collect()),
            positions: page
                .positions
                .as_ref()
                .map(|m| m.keys().copied().map(EntityId::Instance).collect()),
            joints: page.joints.as_ref().map(|m| {
                m.iter()
                    .map(|(joint, (value, slide))| {
                        let kind = self.scene.joint(*joint).map(|j| j.kind);
                        let angular = kind.is_some_and(JointKind::is_angular);
                        SceneJointValue {
                            joint: *joint,
                            value: if angular { value.to_degrees() } else { *value },
                            slide: (kind == Some(JointKind::Cylindrical)).then_some(*slide),
                        }
                    })
                    .collect()
            }),
            sections: page.sections.as_ref().map(|s| SceneSections {
                active: s
                    .active
                    .iter()
                    .map(|(m, plane)| SceneSectionPlane {
                        context: match m {
                            MeshId::Root => None,
                            MeshId::Definition(d) => Some(*d),
                        },
                        plane: EntityId::Section(*plane),
                    })
                    .collect(),
                planes: s.planes,
                cuts: s.cuts,
                fill: s.fill,
                fill_color: if s.material_fill {
                    SectionFillColor::Material
                } else {
                    SectionFillColor::Dark
                },
            }),
            revision: self.revision,
        })
    }

    fn page(&self, id: PageId) -> Result<&Page, CommandError> {
        self.scene
            .page(id)
            .ok_or_else(|| scene_error(KernelError::PageNotFound, Some(id)))
    }
}

/// A kernel refusal as a command error; a missing scene or tag is `not_found`.
/// The lighting a stored view asks for.
fn lighting_of(v: PageView) -> Lighting {
    if v.studio {
        Lighting::Studio
    } else if v.night {
        Lighting::Night
    } else {
        Lighting::Day
    }
}

fn scene_error(e: KernelError, id: Option<PageId>) -> CommandError {
    match (e, id) {
        (KernelError::PageNotFound, Some(id)) => {
            CommandError::new(ErrorKind::NotFound, format!("no scene {id}"))
        }
        (e @ (KernelError::PageNotFound | KernelError::TagNotFound), _) => {
            CommandError::new(ErrorKind::NotFound, e.to_string())
        }
        (e, _) => CommandError::new(ErrorKind::InvalidInput, e.to_string()),
    }
}

impl Document {
    /// Sets how going to a scene animates: the given fields change, the others stay.
    pub(super) fn set_scene_transition_changes(
        &mut self,
        on: Option<bool>,
        seconds: Option<f64>,
    ) -> Result<SceneChange, CommandError> {
        if on.is_none() && seconds.is_none() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "set_scene_transition needs on or seconds",
            ));
        }
        let from = self.scene.scene_transition();
        let to = SceneTransition {
            on: on.unwrap_or(from.on),
            seconds: seconds.unwrap_or(from.seconds),
        };
        self.scene
            .set_scene_transition(to)
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
    }

    /// The motion from the current state to a scene's, or, with `from`, from that scene's:
    /// the camera of the stored view (the current camera for a scene that stores none) and
    /// the transforms of the root instances either stores (the current transform for one
    /// the other lacks). The document is not changed.
    pub fn motion_between(&self, from: Option<PageId>, to: PageId) -> Result<Motion, CommandError> {
        let end = self.page(to)?;
        let start = from.map(|f| self.page(f)).transpose()?;
        let camera_of = |id: PageId, page: &Page| {
            self.scene_camera(id, page)
                .map(|v| v.map(|(camera, _)| camera))
        };
        let current = &self.camera;
        let from_camera = match (from, start) {
            (Some(id), Some(page)) => camera_of(id, page)?,
            _ => None,
        }
        .unwrap_or_else(|| current.clone());
        let to_camera = camera_of(to, end)?.unwrap_or_else(|| current.clone());
        let mut ids: Vec<InstanceId> = Vec::new();
        for page in start.into_iter().chain([end]) {
            for i in page.positions.iter().flat_map(|p| p.keys()) {
                if !ids.contains(i) {
                    ids.push(*i);
                }
            }
        }
        // The joints either scene stores, from where each starts to where it ends (a joint a
        // scene does not store stays at its current value there), and the parts they move.
        let mut joint_ids: Vec<JointId> = Vec::new();
        for page in start.into_iter().chain([end]) {
            for j in page.joints.iter().flat_map(|p| p.keys()) {
                if !joint_ids.contains(j) {
                    joint_ids.push(*j);
                }
            }
        }
        let joints: Vec<(JointId, JointPose, JointPose)> = joint_ids
            .into_iter()
            .filter_map(|j| {
                let now = self.scene.joint(j)?;
                let now = (now.value, now.slide);
                let at = |page: Option<&Page>| {
                    page.and_then(|p| p.joints.as_ref())
                        .and_then(|p| p.get(&j))
                        .copied()
                        .unwrap_or(now)
                };
                Some((j, at(start), at(Some(end))))
            })
            .collect();
        let driven = self.scene.joint_parts(joints.iter().map(|(j, _, _)| *j));
        ids.retain(|i| !driven.contains(i));
        let moves = ids
            .into_iter()
            .filter_map(|i| {
                let now = self.scene.root().instance(i)?.transform;
                let at = |page: Option<&Page>| {
                    page.and_then(|p| p.positions.as_ref())
                        .and_then(|p| p.get(&i))
                        .copied()
                        .unwrap_or(now)
                };
                Some((i, at(start), at(Some(end))))
            })
            .collect();
        Ok(Motion {
            from_camera,
            to_camera,
            moves,
            joints,
        })
    }
}
