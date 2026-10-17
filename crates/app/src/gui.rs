//! egui around the 3D view: window events in, panel layout per frame, painting on top.

use std::time::Duration;

use egui::ViewportId;
use egui_wgpu::{RendererOptions, ScreenDescriptor};
use render::Viewport;
use winit::event::WindowEvent;
use winit::window::Window;

use crate::ui::theme::{self, ThemeChoice};
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
    /// Fly mode holds the pointer: egui keeps the cursor hidden rather than showing the icon
    /// of what it last saw under it.
    pub hide_cursor: bool,
    output: egui::FullOutput,
}

impl Gui {
    pub fn new(
        window: &Window,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        theme: ThemeChoice,
    ) -> Self {
        let ctx = egui::Context::default();
        theme::install(&ctx, theme);
        let state = egui_winit::State::new(
            ctx.clone(),
            ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            // So System starts with the window's appearance; egui-winit follows `ThemeChanged`.
            window.theme(),
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let renderer = egui_wgpu::Renderer::new(device, format, RendererOptions::default());
        Self {
            ctx,
            state,
            renderer,
        }
    }

    /// The egui context, for loading images the panels show.
    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }

    /// The theme the panels are drawn in now (System resolved to what the window reports).
    pub fn theme(&self) -> egui::Theme {
        self.ctx.theme()
    }

    /// Feeds a window event to egui. Returns true if egui needs a repaint.
    pub fn on_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        self.state.on_window_event(window, event).repaint
    }

    /// True if egui wants a press or scroll at `pos` (physical window pixels): it is using
    /// the pointer (e.g. dragging a widget), or one of its windows, menus or popups lies
    /// there. Panels around the view are not counted; check the view's rectangle for those.
    /// It tests `pos` itself rather than the pointer position egui saw in its last frame,
    /// which lags when a press or scroll arrives in the same batch as the move before it.
    pub fn wants_pointer_at(&self, pos: glam::DVec2) -> bool {
        wants_pointer_at(&self.ctx, pos)
    }

    /// True while a text field has focus (a focused button must not swallow editor keys).
    pub fn wants_keyboard(&self) -> bool {
        self.ctx.text_edit_focused()
    }

    /// Lays out the panels for this frame.
    pub fn frame(&mut self, window: &Window, view: &View) -> GuiFrame {
        let input = self.state.take_egui_input(window);
        // Every pass's actions (a discarded pass's too), the last pass's view rectangle.
        let mut actions = Vec::new();
        let mut central = egui::Rect::NOTHING;
        let output = self
            .ctx
            .run_ui(input, |ui| central = draw(ui, view, &mut actions));
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
            hide_cursor: false,
            output,
        }
    }

    /// Paints the frame's panels over `target` (whose size is `size`), handles clipboard and
    /// cursor output, and returns how soon egui wants another frame (None: only on input).
    pub fn paint(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        size: (u32, u32),
        frame: GuiFrame,
    ) -> Option<Duration> {
        let mut output = frame.output;
        if frame.hide_cursor {
            output.platform_output.cursor_icon = egui::CursorIcon::None;
        }
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
            .map(|v| v.repaint_delay)
            .filter(|d| *d < Duration::MAX)
    }
}

/// See `Gui::wants_pointer_at`.
fn wants_pointer_at(ctx: &egui::Context, pos: glam::DVec2) -> bool {
    if ctx.egui_is_using_pointer() {
        return true;
    }
    let ppp = ctx.pixels_per_point() as f64;
    let at = egui::pos2((pos.x / ppp) as f32, (pos.y / ppp) as f32);
    ctx.layer_id_at(at)
        .is_some_and(|layer| layer.order != egui::Order::Background)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect, Vec2};
    use glam::DVec2;

    #[test]
    fn the_pointer_test_uses_the_position_given_not_the_last_frame() {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(2.0);
        let frame = |events: Vec<egui::Event>| egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let ui = |ui: &mut egui::Ui| {
            egui::Window::new("popup")
                .fixed_pos(Pos2::new(100.0, 100.0))
                .fixed_size(Vec2::new(150.0, 100.0))
                .show(ui.ctx(), |ui| ui.label("over the view"));
            // A menu or popup: a foreground area.
            egui::Area::new(egui::Id::new("menu"))
                .order(egui::Order::Foreground)
                .fixed_pos(Pos2::new(500.0, 300.0))
                .show(ui.ctx(), |ui| ui.label("a menu item"));
        };
        // egui last saw the pointer over the window.
        let over = Pos2::new(150.0, 150.0);
        for _ in 0..2 {
            ctx.run_ui(frame(vec![egui::Event::PointerMoved(over)]), ui)
                .drop_without_applying_deltas();
        }
        // Physical pixels at 2x: the window spans 200..500 across.
        assert!(wants_pointer_at(&ctx, DVec2::new(300.0, 300.0)));
        assert!(
            !wants_pointer_at(&ctx, DVec2::new(1200.0, 900.0)),
            "the view beside it, though egui still has the pointer over the window"
        );
        assert!(
            wants_pointer_at(&ctx, DVec2::new(1010.0, 610.0)),
            "over the menu"
        );
    }
}
