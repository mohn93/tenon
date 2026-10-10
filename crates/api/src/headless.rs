//! A host with no window: a document plus an offscreen renderer created on first use.

use glam::DVec2;
use render::Snapshotter;
use session::{BatchError, Camera, Command, CommandError, Document, Outcome};

use crate::dispatch::Host;

pub struct HeadlessHost {
    pub doc: Document,
    snapshotter: Option<Snapshotter>,
    gpu_checked: bool,
}

impl HeadlessHost {
    pub fn new() -> Self {
        Self {
            doc: Document::new(DVec2::new(1280.0, 800.0)),
            snapshotter: None,
            gpu_checked: false,
        }
    }
}

impl Default for HeadlessHost {
    fn default() -> Self {
        Self::new()
    }
}

impl Host for HeadlessHost {
    fn document(&self) -> &Document {
        &self.doc
    }

    fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
        self.doc.execute(command)
    }

    fn execute_batch(
        &mut self,
        commands: Vec<Command>,
        atomic: bool,
    ) -> Result<Vec<Outcome>, BatchError> {
        self.doc.execute_batch(commands, atomic)
    }

    fn render_view(&mut self, camera: &Camera, width: u32, height: u32) -> Option<Vec<u8>> {
        if !self.gpu_checked {
            self.gpu_checked = true;
            self.snapshotter = Snapshotter::new_headless();
        }
        let model = self.doc.model();
        self.snapshotter
            .as_mut()
            .and_then(|s| s.capture(model, camera, width, height))
    }
}
