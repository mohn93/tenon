//! Styles: the model's named looks, and the one the view draws with.

use kernel::{KernelError, SceneChange, Style, StyleId, StyleSettings};

use super::Document;
use crate::command::{CommandError, ErrorKind, StylePatch};
use crate::query::StyleInfo;

impl Document {
    /// The style the view draws with, while it exists: undoing the add of the chosen style, or
    /// deleting it, leaves the default look (and undoing the delete brings the choice back).
    pub fn style(&self) -> Option<StyleId> {
        self.style.filter(|id| self.scene.style(*id).is_some())
    }

    /// The settings of the chosen style; None draws the default look, with the app theme's
    /// background.
    pub fn chosen_style(&self) -> Option<&StyleSettings> {
        self.style()
            .and_then(|id| self.scene.style(id))
            .map(|s| &s.settings)
    }

    /// Chooses `style` (None: the default look). A view setting: no undo step.
    pub(super) fn choose_style(&mut self, style: Option<StyleId>) -> Result<(), CommandError> {
        if let Some(id) = style {
            self.style_ref(id)?;
        }
        self.style = style;
        Ok(())
    }

    /// Adds a style after the last one: a copy of `from`'s settings, or the defaults.
    pub(super) fn add_style_changes(
        &mut self,
        name: Option<&str>,
        from: Option<StyleId>,
    ) -> Result<SceneChange, CommandError> {
        let settings = match from {
            Some(id) => self.style_ref(id)?.settings,
            None => StyleSettings::default(),
        };
        let style = Style {
            name: name.map_or_else(|| self.scene.free_style_name(), str::to_string),
            settings,
        };
        let (_, change) = self
            .scene
            .add_style(style)
            .map_err(|e| style_error(e, None))?;
        Ok(change)
    }

    /// Renames a style and/or changes the parts of its settings `patch` gives.
    pub(super) fn change_style_changes(
        &mut self,
        id: StyleId,
        name: Option<&str>,
        patch: Option<&StylePatch>,
    ) -> Result<SceneChange, CommandError> {
        if name.is_none() && patch.is_none() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "change_style needs a name or settings",
            ));
        }
        let from = self.style_ref(id)?;
        let mut to = Style {
            name: name.map_or_else(|| from.name.clone(), str::to_string),
            settings: from.settings,
        };
        if let Some(patch) = patch {
            apply_patch(&mut to.settings, patch);
        }
        self.scene
            .change_style(id, to)
            .map_err(|e| style_error(e, Some(id)))
    }

    /// Removes a style; scenes that chose it stop choosing it in the same change. The view's
    /// own choice follows `style`, so it falls back with no step of its own and undo brings it
    /// back.
    pub(super) fn delete_style_changes(
        &mut self,
        id: StyleId,
    ) -> Result<SceneChange, CommandError> {
        self.scene
            .delete_style(id)
            .map_err(|e| style_error(e, Some(id)))
    }

    /// Every style, in the order they were made.
    pub(super) fn style_infos(&self) -> Vec<StyleInfo> {
        self.scene
            .styles()
            .map(|(id, s)| StyleInfo {
                id,
                name: s.name.clone(),
                settings: s.settings,
            })
            .collect()
    }

    fn style_ref(&self, id: StyleId) -> Result<&Style, CommandError> {
        self.scene
            .style(id)
            .ok_or_else(|| style_error(KernelError::StyleNotFound, Some(id)))
    }
}

/// `settings` with the parts `patch` gives replaced.
fn apply_patch(settings: &mut StyleSettings, patch: &StylePatch) {
    if let Some(e) = &patch.edges {
        let edges = &mut settings.edges;
        if let Some(mode) = e.color_mode {
            edges.color_mode = mode;
        }
        if let Some(color) = e.color {
            edges.color = color;
        }
        if let Some(show) = e.show_edges {
            edges.show_edges = show;
        }
        if let Some(profiles) = e.profiles {
            edges.profiles = profiles;
        }
        if let Some(sketchy) = e.sketchy {
            edges.sketchy = sketchy;
        }
        if let Some(endpoints) = e.endpoints {
            edges.endpoints = endpoints;
        }
    }
    if let Some(background) = patch.background {
        settings.background = background;
    }
    if let Some(sky) = patch.sky {
        settings.sky = sky;
    }
    if let Some(ground) = patch.ground {
        settings.ground = ground;
    }
    if let Some(fog) = patch.fog {
        settings.fog = fog;
    }
}

/// A kernel refusal as a command error; a missing style is `not_found`.
fn style_error(e: KernelError, id: Option<StyleId>) -> CommandError {
    match (e, id) {
        (KernelError::StyleNotFound, Some(id)) => {
            CommandError::new(ErrorKind::NotFound, format!("no style {id}"))
        }
        (e @ KernelError::StyleNotFound, None) => {
            CommandError::new(ErrorKind::NotFound, e.to_string())
        }
        (e, _) => CommandError::new(ErrorKind::InvalidInput, e.to_string()),
    }
}
