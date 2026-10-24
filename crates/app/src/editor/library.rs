//! The component library in the window: placing an item with the Paste tool, inserting one,
//! and saving to the library. The folder and its listing are the window's (`library`).

use std::path::Path;

use glam::DVec3;
use kernel::MaterialId;
use session::{Command, CommandError, ErrorKind, LibraryKind, LibraryRef, library};
use tools::PasteTool;

use super::{Editor, Message};

fn text(path: &Path) -> Result<String, CommandError> {
    path.to_str().map(str::to_string).ok_or_else(|| {
        CommandError::new(
            ErrorKind::InvalidInput,
            format!("{} is not a valid UTF-8 path", path.display()),
        )
    })
}

impl Editor {
    /// Brings out the Paste tool with the component or model item at `path`: its edges follow
    /// the cursor and a click inserts it there (`insert_library_item`, one `insert` step,
    /// new definitions each time; inside a turned or scaled group the preview is drawn in world
    /// axes and size too, as the insert keeps them); Escape goes back to the tool used before. A material
    /// item, or a file that is not a valid item, is refused with why in the status.
    pub fn place_library_item(&mut self, path: &Path) -> Result<(), CommandError> {
        let outline = match library::outline(path) {
            Ok(outline) => outline,
            Err(e) => return Err(self.refuse_library(e)),
        };
        // The tool adds the cursor in context coordinates and the view maps the preview through
        // the context, while an insert keeps the item's world orientation and size: the outline
        // goes through the inverse of the context's linear part so both agree.
        let linear = self.doc().context_transform().matrix3;
        let outline = if linear.determinant().abs() > 1e-12 {
            let back = linear.inverse();
            session::ClipOutline {
                base: DVec3::ZERO,
                segments: outline
                    .segments
                    .iter()
                    .map(|[a, b]| [back * (*a - outline.base), back * (*b - outline.base)])
                    .collect(),
            }
        } else {
            outline
        };
        let tool = PasteTool::library(text(path)?, &outline);
        self.start_paste_tool(tool);
        Ok(())
    }

    /// Inserts the item at `path` as one `insert` step: a component or model at the open
    /// context's origin (selected afterwards), a material added to the model and chosen to
    /// paint with. Refusals are in the status. Ends the Paste tool, if out.
    pub fn insert_library_item(&mut self, path: &Path) -> Result<Option<MaterialId>, CommandError> {
        let path = text(path)?;
        self.end_paste();
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        let result =
            self.execute_recorded_as(Command::InsertLibraryItem { path, at: None }, "insert");
        self.invalidate_hover();
        let outcome = result.map_err(|e| self.refuse_library(e))?;
        if let Some(material) = outcome.material {
            self.set_paint_material(Some(material))?;
        }
        Ok(outcome.material)
    }

    /// Writes a library item to `path` (see `Command::SaveToLibrary`). The status says what was
    /// saved, or why not.
    pub fn save_to_library(
        &mut self,
        path: &Path,
        kind: LibraryKind,
        item: Option<LibraryRef>,
        name: &str,
        overwrite: bool,
    ) -> Result<(), CommandError> {
        let command = Command::SaveToLibrary {
            path: text(path)?,
            kind,
            item,
            name: Some(name.to_string()),
            overwrite,
        };
        match self.execute_recorded(command) {
            Ok(_) => {
                self.message = Some(Message::info(format!("Saved {name} to the library")));
                Ok(())
            }
            Err(e) => Err(self.refuse_library(e)),
        }
    }

    fn refuse_library(&mut self, e: CommandError) -> CommandError {
        self.message = Some(Message::error(e.message.clone()));
        e
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use glam::{DVec2, DVec3};
    use kernel::EntityId;
    use session::SelectMode;
    use tools::ToolKind;

    use super::*;
    use crate::navigation::Button;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sketchup-alt-editor-library-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An editor holding one 2 m square component named Tile, selected.
    fn with_component() -> Editor {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        e.execute(Command::AddRectangle {
            corner: [0.0; 3],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        })
        .unwrap();
        e.execute(Command::Select {
            ids: Vec::new(),
            mode: SelectMode::Replace,
            all: true,
        })
        .unwrap();
        assert!(e.make_component_named("Tile"));
        e
    }

    fn instance(e: &Editor) -> kernel::InstanceId {
        match e.doc().selection().first() {
            Some(EntityId::Instance(id)) => *id,
            other => panic!("{other:?}"),
        }
    }

    fn save_tile(e: &mut Editor, path: &Path) {
        let item = Some(LibraryRef::Entity(EntityId::Instance(instance(e))));
        e.save_to_library(path, LibraryKind::Component, item, "Tile", false)
            .unwrap();
    }

    fn definitions(e: &Editor) -> usize {
        e.doc().scene().definitions().count()
    }

    fn point_at(e: &mut Editor, p: DVec3) {
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        e.refresh_hover();
    }

    #[test]
    fn a_placed_item_follows_the_cursor_and_a_click_inserts_it_as_one_step() {
        let dir = folder("place");
        let path = dir.join("tile.skr");
        let mut e = with_component();
        save_tile(&mut e, &path);
        e.select_tool(ToolKind::Rectangle);
        point_at(&mut e, DVec3::new(6.0, 0.0, 0.0));
        e.place_library_item(&path).unwrap();
        assert!(e.pasting());
        assert_eq!(e.status_parts().tool, "Paste");
        assert_eq!(e.tool_kind(), ToolKind::Rectangle, "the tool to go back to");
        e.refresh_hover();
        assert!(!e.hud(false).lines.is_empty(), "the item's outline shows");
        let before = definitions(&e);
        e.button(Button::Left, true);
        e.button(Button::Left, false);
        assert!(!e.pasting(), "the click ends the tool");
        assert_eq!(e.doc().undo_label(), Some("insert"));
        assert_eq!(definitions(&e), before + 1, "a new definition");
        assert_eq!(
            e.doc().selection().len(),
            1,
            "what was inserted is selected"
        );
        // Placing again makes another definition (a fresh token each time).
        e.place_library_item(&path).unwrap();
        point_at(&mut e, DVec3::new(-6.0, 0.0, 0.0));
        e.button(Button::Left, true);
        e.button(Button::Left, false);
        assert_eq!(definitions(&e), before + 2);
        // One undo takes the last insert back.
        assert!(e.undo());
        assert_eq!(definitions(&e), before + 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The corners of the box around `points`.
    fn world_box(points: &[DVec3]) -> (DVec3, DVec3) {
        let lo = points.iter().fold(DVec3::splat(f64::MAX), |a, p| a.min(*p));
        let hi = points.iter().fold(DVec3::splat(f64::MIN), |a, p| a.max(*p));
        (lo, hi)
    }

    #[test]
    fn inside_a_turned_scaled_group_the_preview_is_where_and_as_large_as_the_insert() {
        let dir = folder("turned");
        let path = dir.join("tile.skr");
        let mut e = with_component();
        save_tile(&mut e, &path);
        e.execute(Command::Select {
            ids: Vec::new(),
            mode: SelectMode::Replace,
            all: true,
        })
        .unwrap();
        let all = e.doc().selection().iter().copied().collect();
        e.execute(Command::MakeGroup {
            ids: all,
            name: None,
        })
        .unwrap();
        let group = e.doc().selection().first().copied().unwrap();
        e.execute(Command::Rotate {
            ids: vec![group],
            center: [0.0; 3],
            axis: None,
            angle: 30.0,
            copies: None,
        })
        .unwrap();
        e.execute(Command::Scale {
            ids: vec![group],
            origin: [0.0; 3],
            factors: [2.0, 1.0, 1.0],
        })
        .unwrap();
        e.execute(Command::OpenInstance { id: group }).unwrap();
        let context = e.doc().context_transform();
        assert!(!context.matrix3.abs_diff_eq(glam::DMat3::IDENTITY, 1e-6));
        e.place_library_item(&path).unwrap();
        point_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        e.refresh_hover();
        let shown: Vec<DVec3> = e
            .preview
            .iter()
            .flat_map(|[a, b]| [context.transform_point3(*a), context.transform_point3(*b)])
            .collect();
        assert!(!shown.is_empty());
        e.button(Button::Left, true);
        e.button(Button::Left, false);
        let EntityId::Instance(id) = e.doc().selection().first().copied().unwrap() else {
            panic!("an instance was inserted")
        };
        let inserted = e.doc().model().instance(id).unwrap().clone();
        let definition = e.doc().scene().definition(inserted.definition).unwrap();
        let placed: Vec<DVec3> = definition
            .mesh
            .vertices()
            .map(|(_, v)| context.transform_point3(inserted.transform.transform_point3(v.pos)))
            .collect();
        let (plo, phi) = world_box(&shown);
        let (ilo, ihi) = world_box(&placed);
        assert!(plo.abs_diff_eq(ilo, 1e-6), "{plo:?} vs {ilo:?}");
        assert!(phi.abs_diff_eq(ihi, 1e-6), "{phi:?} vs {ihi:?}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn escape_cancels_a_placed_item() {
        let dir = folder("escape");
        let path = dir.join("tile.skr");
        let mut e = with_component();
        save_tile(&mut e, &path);
        let before = definitions(&e);
        e.place_library_item(&path).unwrap();
        assert!(e.pasting());
        e.cancel();
        assert!(!e.pasting());
        assert_eq!(definitions(&e), before);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_material_item_inserts_and_becomes_the_paint_material() {
        let dir = folder("material");
        let path = dir.join("red.skr");
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        assert!(e.add_material());
        let source = e.current_material().unwrap();
        e.save_to_library(
            &path,
            LibraryKind::Material,
            Some(LibraryRef::Material(source)),
            "Red",
            false,
        )
        .unwrap();
        // A material is not placed with the cursor.
        assert!(e.place_library_item(&path).is_err());
        assert!(!e.pasting());
        // Into a model without it: added and chosen.
        let mut other = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        let chosen = other
            .insert_library_item(&path)
            .unwrap()
            .expect("a material");
        assert_eq!(other.current_material(), Some(chosen));
        assert_eq!(other.doc().scene().materials().count(), 1);
        // Into the model that has it already: the identical material is reused.
        assert_eq!(e.insert_library_item(&path).unwrap(), Some(source));
        assert_eq!(e.doc().scene().materials().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_refused_insert_or_save_says_why_in_the_status() {
        let dir = folder("refused");
        let mut e = with_component();
        let missing = dir.join("none.skr");
        assert!(e.place_library_item(&missing).is_err());
        assert!(e.status_parts().message.is_some(), "the status says why");
        assert!(!e.pasting());
        assert!(e.insert_library_item(&missing).is_err());
        let path = dir.join("tile.skr");
        save_tile(&mut e, &path);
        assert!(e.status_parts().message.unwrap().contains("Saved Tile"));
        // An existing file is replaced only with overwrite.
        let item = Some(LibraryRef::Entity(EntityId::Instance(instance(&e))));
        let err = e
            .save_to_library(&path, LibraryKind::Component, item, "Tile", false)
            .unwrap_err();
        assert!(err.message.contains("overwrite"), "{}", err.message);
        e.save_to_library(&path, LibraryKind::Component, item, "Tile", true)
            .unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
