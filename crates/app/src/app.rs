use std::sync::Arc;

use glam::DVec2;
use render::Renderer;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::editor::Editor;
use crate::navigation::Button;

/// Trackpad pixel deltas per zoom step.
const PIXELS_PER_SCROLL_STEP: f64 = 50.0;
/// Snap radius in logical pixels (scaled to physical pixels by the window's scale factor).
const PICK_RADIUS_LOGICAL_PX: f64 = 8.0;

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    editor: Editor,
    /// Document revision the renderer's model buffers were built from.
    drawn_revision: Option<u64>,
    modifiers: ModifiersState,
}

impl State {
    fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no compatible GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("request device");

        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface not supported by adapter");
        // Colors are authored as sRGB values, so prefer a non-sRGB target that writes them as-is.
        // Keep the default format's channel layout when its non-sRGB twin is available.
        let caps = surface.get_capabilities(&adapter);
        let linear = config.format.remove_srgb_suffix();
        config.format = if caps.formats.contains(&linear) {
            linear
        } else {
            caps.formats
                .iter()
                .copied()
                .find(|f| !f.is_srgb())
                .unwrap_or(config.format)
        };
        surface.configure(&device, &config);

        let renderer = Renderer::new(&device, &queue, config.format);
        let editor = Editor::new(
            DVec2::new(config.width as f64, config.height as f64),
            PICK_RADIUS_LOGICAL_PX * window.scale_factor(),
        );
        Self {
            window,
            surface,
            device,
            queue,
            config,
            renderer,
            editor,
            drawn_revision: None,
            modifiers: ModifiersState::empty(),
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.editor.resize(DVec2::new(width as f64, height as f64));
    }

    fn render(&mut self) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return;
            }
            _ => return, // Timeout or occluded: skip this frame.
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let revision = self.editor.doc.revision();
        if self.drawn_revision != Some(revision) {
            self.renderer.set_model(self.editor.doc.model());
            self.drawn_revision = Some(revision);
        }
        self.renderer.render(
            &view,
            (self.config.width, self.config.height),
            &self.editor.doc.camera,
            &self.editor.overlay(),
        );
        self.window.pre_present_notify();
        self.queue.present(frame);
    }
}

fn map_button(button: MouseButton) -> Button {
    match button {
        MouseButton::Left => Button::Left,
        MouseButton::Middle => Button::Middle,
        _ => Button::Other,
    }
}

#[derive(Default)]
pub struct App {
    state: Option<State>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("sketchup-alt")
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let state = State::new(window);
        state.window.request_redraw();
        self.state = Some(state);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(s) = self.state.as_mut() else { return };
        let changed = match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                false
            }
            WindowEvent::Resized(size) => {
                s.resize(size.width, size.height);
                true
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                s.editor
                    .set_pick_radius(PICK_RADIUS_LOGICAL_PX * scale_factor);
                false
            }
            WindowEvent::RedrawRequested => {
                s.render();
                false
            }
            WindowEvent::ModifiersChanged(m) => {
                s.modifiers = m.state();
                s.editor
                    .set_modifiers(m.state().shift_key(), m.state().alt_key());
                false
            }
            WindowEvent::Focused(false) => {
                s.editor.focus_lost();
                false
            }
            WindowEvent::CursorMoved { position, .. } => {
                s.editor.cursor_moved(DVec2::new(position.x, position.y))
            }
            WindowEvent::MouseInput { state, button, .. } => s
                .editor
                .button(map_button(button), state == ElementState::Pressed),
            WindowEvent::MouseWheel { delta, .. } => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / PIXELS_PER_SCROLL_STEP,
                };
                s.editor.scroll(steps)
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let command_key = s.modifiers.super_key() || s.modifiers.control_key();
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => s.editor.cancel(),
                    Key::Character(c) if command_key && c.eq_ignore_ascii_case("z") => {
                        if s.modifiers.shift_key() {
                            s.editor.redo()
                        } else {
                            s.editor.undo()
                        }
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        if changed {
            s.window.request_redraw();
        }
    }
}
