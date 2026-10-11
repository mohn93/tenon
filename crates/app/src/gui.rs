//! egui around the 3D view: window events in, panel layout per frame, painting on top.

use egui::ViewportId;
use egui_wgpu::{RendererOptions, ScreenDescriptor};
use render::Viewport;
use winit::event::WindowEvent;
use winit::window::Window;

use crate::ui::{UiAction, View, draw};

pub struct Gui {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
}

/// One laid-out frame of the panels, ready to paint.
pub struct GuiFrame {
    pub actions: Vec<UiAction>,
    /// Where the 3D view goes, in physical pixels.
    pub viewport: Viewport,
    output: egui::FullOutput,
}

impl Gui {
    pub fn new(window: &Window, device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let renderer = egui_wgpu::Renderer::new(device, format, RendererOptions::default());
        Self {
            ctx,
            state,
            renderer,
        }
    }

    /// Feeds a window event to egui. Returns true if egui needs a repaint.
    pub fn on_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        self.state.on_window_event(window, event).repaint
    }

    /// True while the pointer is over a panel or egui is using it (e.g. a button press).
    pub fn wants_pointer(&self) -> bool {
        self.ctx.is_pointer_over_egui() || self.ctx.egui_is_using_pointer()
    }

    /// True while an egui widget takes typing.
    pub fn wants_keyboard(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }

    /// Lays out the panels for this frame.
    pub fn frame(&mut self, window: &Window, view: &View) -> GuiFrame {
        let input = self.state.take_egui_input(window);
        let mut laid_out = (Vec::new(), egui::Rect::NOTHING);
        let output = self.ctx.run_ui(input, |ui| laid_out = draw(ui, view));
        let (actions, central) = laid_out;
        let ppp = output.pixels_per_point;
        let px = |v: f32| (v * ppp).round().max(0.0) as u32;
        let viewport = Viewport {
            x: px(central.min.x),
            y: px(central.min.y),
            width: px(central.width()),
            height: px(central.height()),
        };
        GuiFrame {
            actions,
            viewport,
            output,
        }
    }

    /// Paints the frame's panels over `target` (whose size is `size`), handles clipboard and
    /// cursor output, and returns true if egui asks to be repainted right away.
    pub fn paint(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        size: (u32, u32),
        frame: GuiFrame,
    ) -> bool {
        let mut output = frame.output;
        self.state
            .handle_platform_output(window, output.platform_output);
        let screen = ScreenDescriptor {
            size_in_pixels: [size.0, size.1],
            pixels_per_point: output.pixels_per_point,
        };
        let jobs = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                self.renderer.update_texture(device, queue, *id, delta);
            }
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("egui"),
        });
        let uploads = self
            .renderer
            .update_buffers(device, queue, &mut encoder, &jobs, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.renderer
                .render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        queue.submit(uploads.into_iter().chain([encoder.finish()]));
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
        output.textures_delta.clear();
        output
            .viewport_output
            .get(&ViewportId::ROOT)
            .is_some_and(|v| v.repaint_delay.is_zero())
    }
}
