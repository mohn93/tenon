//! How the window moves between scenes: a model setting.

use crate::edit::KernelError;
use crate::scene::{Scene, SceneChange, Step};

/// Whether going to a scene animates, and for how long.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneTransition {
    pub on: bool,
    /// 0 to [`SceneTransition::MAX_SECONDS`].
    pub seconds: f64,
}

impl SceneTransition {
    pub const MAX_SECONDS: f64 = 10.0;

    /// On, for one second.
    pub const DEFAULT: SceneTransition = SceneTransition {
        on: true,
        seconds: 1.0,
    };

    /// Fails when `seconds` is not a finite number from 0 to 10.
    pub fn check(&self) -> Result<(), KernelError> {
        if !(self.seconds.is_finite() && (0.0..=Self::MAX_SECONDS).contains(&self.seconds)) {
            return Err(KernelError::BadTransition(
                "seconds must be from 0 to 10".into(),
            ));
        }
        Ok(())
    }

    pub fn is_default(&self) -> bool {
        *self == Self::DEFAULT
    }
}

impl Default for SceneTransition {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Scene {
    /// How going to a scene animates in the window.
    pub fn scene_transition(&self) -> SceneTransition {
        self.transition
    }

    /// Sets the scene transition, after checking it. Nothing to change changes nothing.
    pub fn set_scene_transition(
        &mut self,
        transition: SceneTransition,
    ) -> Result<SceneChange, KernelError> {
        transition.check()?;
        if transition == self.transition {
            return Ok(SceneChange::default());
        }
        let step = Step::SetTransition(self.transition, transition);
        self.apply_step(&step);
        Ok(step.into())
    }
}
