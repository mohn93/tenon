//! The labels of dimensions and notes in rendered images (`render_view`, the headless render):
//! the window paints them with egui over the 3D view, and this paints the same labels with the
//! same code and fonts in an egui context of its own, composited by egui-wgpu onto the image.
//! Text is sized in points, one pixel each up to 2000 pixels wide and scaled by width / 1000
//! (at most 4) beyond.

use egui::{Pos2, Rect, vec2};
use egui_wgpu::{RendererOptions, ScreenDescriptor};
use session::{Camera, Lighting};

use crate::feedback::HudLabel;
use crate::ui::{theme, viewport};

/// The pixels per point of an image `width` pixels wide.
pub fn pixels_per_point(width: u32) -> f32 {
    if width > 2000 {
        (width as f32 / 1000.0).min(4.0)
    } else {
        1.0
    }
}

/// The egui context and renderer that paint labels onto images.
#[derive(Default)]
pub struct ImageLabels {
    ctx: Option<egui::Context>,
    renderer: Option<egui_wgpu::Renderer>,
}

impl ImageLabels {
    /// Paints `labels` onto `target` (an image of `camera`'s viewport size, in `format`) on
    /// `device`, over what is there. Day images get the light theme's colors, night ones the
    /// dark theme's.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        camera: &Camera,
        lighting: Lighting,
        labels: &[HudLabel],
    ) {
        if labels.is_empty() {
            return;
        }
        let size = [
            camera.viewport.x.max(1.0) as u32,
            camera.viewport.y.max(1.0) as u32,
        ];
        let ppp = pixels_per_point(size[0]);
        let renderer = self.renderer.get_or_insert_with(|| {
            egui_wgpu::Renderer::new(device, format, RendererOptions::default())
        });
        let ctx = self.ctx.get_or_insert_with(|| {
            let ctx = egui::Context::default();
            theme::install(&ctx, theme::ThemeChoice::Light);
            // Fonts take effect with the next pass: this one only starts them (and the
            // renderer must still hear of its textures).
            let mut warm_up = ctx.run_ui(egui::RawInput::default(), |_| {});
            upload(renderer, device, queue, &mut warm_up.textures_delta);
            ctx
        });
        theme::set_choice(
            ctx,
            match lighting {
                Lighting::Day => theme::ThemeChoice::Light,
                Lighting::Night => theme::ThemeChoice::Dark,
            },
        );
        let rect =
            Rect::from_min_size(Pos2::ZERO, vec2(size[0] as f32 / ppp, size[1] as f32 / ppp));
        let mut input = egui::RawInput {
            screen_rect: Some(rect),
            ..Default::default()
        };
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(ppp);
        let mut output = ctx.run_ui(input, |ui| {
            let painter = ui.painter_at(rect);
            for l in labels {
                let at = Pos2::new(l.at.x as f32 / ppp, l.at.y as f32 / ppp);
                viewport::label(&painter, at, l);
            }
        });
        let screen = ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point: output.pixels_per_point,
        };
        let jobs = ctx.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
        let freed = upload(renderer, device, queue, &mut output.textures_delta);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("image labels"),
        });
        let uploads = renderer.update_buffers(device, queue, &mut encoder, &jobs, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("image labels"),
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
            renderer.render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        queue.submit(uploads.into_iter().chain([encoder.finish()]));
        for id in &freed {
            renderer.free_texture(id);
        }
    }
}

/// Gives `renderer` the textures `delta` sets, leaving `delta` empty; returns those it frees,
/// which go once the frame is drawn.
fn upload(
    renderer: &mut egui_wgpu::Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    delta: &mut egui::TexturesDelta,
) -> Vec<egui::TextureId> {
    for (id, deltas) in &delta.set {
        for d in deltas {
            renderer.update_texture(device, queue, *id, d);
        }
    }
    let freed = std::mem::take(&mut delta.free).into_iter().collect();
    delta.clear();
    freed
}

#[cfg(test)]
mod tests {
    use super::pixels_per_point;

    #[test]
    fn text_is_one_pixel_per_point_up_to_2000_wide_then_scales_to_at_most_4() {
        assert_eq!(pixels_per_point(1), 1.0);
        assert_eq!(pixels_per_point(2000), 1.0);
        assert_eq!(pixels_per_point(3000), 3.0);
        assert_eq!(pixels_per_point(9000), 4.0);
    }
}
