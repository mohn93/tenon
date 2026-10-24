//! A host with no window: a document plus an offscreen renderer created on first use.

use glam::DVec2;
use render::Snapshotter;
use render::print::{Printed, render_pdf};
use session::print::PrintSpec;
use session::{
    BatchCommand, BatchError, Camera, Command, CommandError, Document, Lighting, Origin, Outcome,
};

use crate::dispatch::{Host, no_gpu_for_printing};
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

impl HeadlessHost {
    /// Looks for a GPU on first use.
    fn check_gpu(&mut self) {
        if !self.gpu_checked {
            self.gpu_checked = true;
            self.snapshotter = Snapshotter::new_headless();
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
        commands: Vec<BatchCommand>,
        atomic: bool,
    ) -> Result<Vec<Outcome>, BatchError> {
        self.doc.execute_batch(commands, atomic)
    }

    fn render_pdf(&mut self, spec: &PrintSpec) -> Result<Printed, CommandError> {
        self.check_gpu();
        let snapshotter = self.snapshotter.as_mut().ok_or_else(no_gpu_for_printing)?;
        render_pdf(snapshotter, &self.doc, spec)
    }

    fn render_view(
        &mut self,
        camera: &Camera,
        lighting: Lighting,
        width: u32,
        height: u32,
        frame: Option<&kernel::Scene>,
    ) -> Option<Vec<u8>> {
        self.check_gpu();
        let doc = &self.doc;
        self.snapshotter.as_mut().and_then(|s| {
            s.set_color_by_tag(doc.color_by_tag);
            s.set_glass_view(render::scene::GlassView {
                x_ray: doc.x_ray,
                hidden_geometry: doc.hidden_geometry,
            });
            s.set_section_display(doc.section_display);
            s.set_sun(render::SunLight::of(doc));
            s.set_model_style(doc.chosen_style());
            s.capture(
                frame.unwrap_or_else(|| doc.scene()),
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
