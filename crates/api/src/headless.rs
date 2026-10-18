//! A host with no window: a document plus an offscreen renderer created on first use.

use glam::DVec2;
use render::Snapshotter;
use session::{BatchError, Camera, Command, CommandError, Document, Lighting, Origin, Outcome};

use crate::dispatch::Host;
use crate::roots::ApiRoots;

pub struct HeadlessHost {
    pub doc: Document,
    snapshotter: Option<Snapshotter>,
    gpu_checked: bool,
    roots: ApiRoots,
}

impl HeadlessHost {
    pub fn new() -> Self {
        // Everything in a headless document comes through the API.
        let mut doc = Document::new(DVec2::new(1280.0, 800.0));
        doc.set_origin(Origin::Agent);
        Self {
            doc,
            snapshotter: None,
            gpu_checked: false,
            roots: ApiRoots::default(),
        }
    }

    /// A host whose API files must lie within `roots` (empty: no limit).
    pub fn with_roots(roots: ApiRoots) -> Self {
        Self {
            roots,
            ..Self::new()
        }
    }
}

impl Default for HeadlessHost {
    fn default() -> Self {
        Self::new()
    }
}

impl Host for HeadlessHost {
    fn roots(&self) -> Option<&ApiRoots> {
        (!self.roots.is_empty()).then_some(&self.roots)
    }

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

    fn render_view(
        &mut self,
        camera: &Camera,
        lighting: Lighting,
        width: u32,
        height: u32,
    ) -> Option<Vec<u8>> {
        if !self.gpu_checked {
            self.gpu_checked = true;
            self.snapshotter = Snapshotter::new_headless();
        }
        let doc = &self.doc;
        self.snapshotter.as_mut().and_then(|s| {
            s.capture(
                doc.scene(),
                doc.context(),
                doc.selection(),
                camera,
                lighting,
                width,
                height,
            )
        })
    }
}
