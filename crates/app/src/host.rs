//! The agent API's view of the window app: commands go through the Editor (so the active tool
//! stays consistent) and `render_view` uses an offscreen renderer on the window's GPU device.

use api::Host;
use render::Snapshotter;
use session::{BatchError, Camera, Command, CommandError, Document, Outcome};

use crate::editor::Editor;

pub struct WindowHost<'a> {
    pub editor: &'a mut Editor,
    pub snapshotter: &'a mut Snapshotter,
}

impl Host for WindowHost<'_> {
    fn document(&self) -> &Document {
        self.editor.doc()
    }

    fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
        self.editor.execute(command)
    }

    fn execute_batch(
        &mut self,
        commands: Vec<Command>,
        atomic: bool,
    ) -> Result<Vec<Outcome>, BatchError> {
        self.editor.execute_batch(commands, atomic)
    }

    fn render_view(&mut self, camera: &Camera, width: u32, height: u32) -> Option<Vec<u8>> {
        let doc = self.editor.doc();
        self.snapshotter.capture(
            doc.scene(),
            doc.context(),
            doc.selection(),
            camera,
            width,
            height,
        )
    }
}
