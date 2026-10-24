//! What agents read and set of the editor beyond the model (see `api::EditorState` and
//! `api::EditorCommand`).

use api::Mods;
use kernel::MaterialId;
use session::{CommandError, ErrorKind};
use tools::Phase;

use super::Editor;
use crate::fly;

impl Editor {
    /// Whether the tool is in an operation, and the points it was given so far.
    pub fn phase(&self) -> Phase {
        self.tool.phase()
    }

    /// The text typed into the measurements box, not yet submitted.
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// Fly mode's speed, meters a second.
    pub fn flying_speed(&self) -> f64 {
        self.fly.speed()
    }

    /// The Soften Edges window's angle in degrees, and whether the window is open.
    pub fn soften_state(&self) -> (u8, bool) {
        (self.soften.angle, self.soften.open)
    }

    /// Makes `material` (None: the default material) the one the Paint tool paints with,
    /// keeping the tool. `not_found` for a material the model does not have.
    pub fn set_paint_material(&mut self, material: Option<MaterialId>) -> Result<(), CommandError> {
        if let Some(m) = material
            && self.doc.scene().material(m).is_none()
        {
            return Err(CommandError::new(
                ErrorKind::NotFound,
                format!("no material {m}"),
            ));
        }
        self.material = material;
        Ok(())
    }

    /// Turns fly mode on or off and, if given, sets its speed (meters a second, within
    /// `fly::MIN_SPEED` and `fly::MAX_SPEED`; `invalid_input` otherwise, changing nothing).
    pub fn set_fly_mode(&mut self, on: bool, speed: Option<f64>) -> Result<(), CommandError> {
        if let Some(speed) = speed
            && !self.fly.set_speed(speed)
        {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!(
                    "fly speed must be between {} and {} m/s",
                    fly::MIN_SPEED,
                    fly::MAX_SPEED
                ),
            ));
        }
        self.set_flying(on);
        Ok(())
    }

    /// Sets the Soften Edges window's angle; `invalid_input` above 180 degrees.
    pub fn set_soften_angle_checked(&mut self, angle: u32) -> Result<(), CommandError> {
        match u8::try_from(angle) {
            Ok(a) if a <= 180 => {
                self.set_soften_angle(a);
                Ok(())
            }
            _ => Err(CommandError::new(
                ErrorKind::InvalidInput,
                "the soften angle must be between 0 and 180 degrees",
            )),
        }
    }

    /// Aborts the tool's operation, as Escape pressed until the tool is idle would, without
    /// closing the open group: leaves fly mode, drops typed text, the direction lock, a press
    /// the tool waits to see released, and the operation with anything it waits for (a copy
    /// array, a retyped angle), and lets go of the modifiers `tool_input` holds.
    pub fn cancel(&mut self) {
        // Keys `tool_input` holds are let go.
        self.agent_mods = Mods::default();
        self.apply_modifiers();
        self.set_flying(false);
        self.typed.clear();
        self.lock = None;
        self.shift_lock = false;
        self.tool_pressed = false;
        self.message = None;
        self.reset_tool();
        // The Paste tool goes, back to the tool used before it, as Escape does.
        self.end_paste();
        // Rotate's plane lock goes too (`reset_tool` keeps it: it is not about geometry).
        self.tool.lock_plane(None);
        self.invalidate_hover();
    }
}

#[cfg(test)]
mod tests {
    use glam::{DVec2, DVec3};
    use session::Command;
    use tools::ToolKind;

    use super::*;
    use crate::editor::EditorKey;
    use crate::navigation::Button;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    fn click_at(e: &mut Editor, p: DVec3) {
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        e.button(Button::Left, true);
        e.button(Button::Left, false);
    }

    #[test]
    fn the_phase_follows_the_tools_clicks() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        assert_eq!(e.phase(), Phase::Idle);
        let a = DVec3::new(1.0, 1.0, 0.0);
        click_at(&mut e, a);
        let Phase::Active { points } = e.phase() else {
            panic!("a line is under way");
        };
        assert_eq!(points.len(), 1);
        assert!(points[0].distance(a) < 1e-9);
        e.key(EditorKey::Char('2'));
        assert_eq!(e.typed(), "2");
    }

    #[test]
    fn cancel_ends_the_operation_and_drops_typing_but_keeps_the_open_group() {
        let mut e = editor();
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        })
        .unwrap();
        let all: Vec<_> = e
            .doc()
            .model()
            .edges()
            .map(|(id, _)| kernel::EntityId::Edge(id))
            .chain(
                e.doc()
                    .model()
                    .faces()
                    .map(|(id, _)| kernel::EntityId::Face(id)),
            )
            .collect();
        let out = e
            .execute(Command::MakeGroup {
                ids: all,
                name: None,
            })
            .unwrap();
        let group = out
            .changes
            .created
            .into_iter()
            .find(|id| matches!(id, kernel::EntityId::Instance(_)))
            .unwrap();
        e.execute(Command::OpenInstance { id: group }).unwrap();
        e.select_tool(ToolKind::Line);
        click_at(&mut e, DVec3::new(0.5, 0.5, 0.0));
        e.key(EditorKey::Char('3'));
        e.key(EditorKey::LockAxis(tools::Axis::X));
        assert!(matches!(e.phase(), Phase::Active { .. }));

        e.cancel();
        assert_eq!(e.phase(), Phase::Idle);
        assert_eq!(e.typed(), "");
        assert_eq!(e.status_parts().lock, None);
        assert_eq!(e.doc().context().len(), 1, "the group stays open");
        // Cancelling an idle tool changes nothing.
        e.cancel();
        assert_eq!(e.phase(), Phase::Idle);
        // Rotate's plane lock goes.
        e.select_tool(ToolKind::Rotate);
        e.key(EditorKey::LockAxis(tools::Axis::Z));
        assert_eq!(e.status_parts().lock, Some("Locked: blue axis"));
        e.cancel();
        assert_eq!(e.status_parts().lock, None);
    }

    #[test]
    fn the_paint_material_is_set_without_switching_tools() {
        let mut e = editor();
        e.select_tool(ToolKind::Line);
        let out = e
            .execute(Command::AddMaterial {
                name: Some("Brick".into()),
                color: Some([200, 80, 60]),
                opacity: None,
                emission: None,
                metalness: None,
                roughness: None,
                density: None,
                existing: Default::default(),
                texture: None,
            })
            .unwrap();
        let brick = out.changes.materials[0];
        e.set_paint_material(Some(brick)).unwrap();
        assert_eq!(e.current_material(), Some(brick));
        assert_eq!(e.tool_kind(), ToolKind::Line);
        let missing = e.set_paint_material(Some(MaterialId(999))).unwrap_err();
        assert_eq!(missing.kind, ErrorKind::NotFound);
        assert_eq!(e.current_material(), Some(brick));
        e.set_paint_material(None).unwrap();
        assert_eq!(e.current_material(), None);
    }
}
