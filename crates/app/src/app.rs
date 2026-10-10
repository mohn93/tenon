use std::path::PathBuf;
use std::sync::Arc;

use api::{Hub, listen_unix};
use glam::DVec2;
use render::{Renderer, Snapshotter};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::editor::Editor;
use crate::host::WindowHost;
use crate::navigation::Button;

/// Sent to the event loop when an agent API request arrives.
#[derive(Debug)]
pub struct ApiWake;

/// Trackpad pixel deltas per zoom step.
const PIXELS_PER_SCROLL_STEP: f64 = 50.0;

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    snapshotter: Snapshotter,
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
        let snapshotter = Snapshotter::with_device(&device, &queue);
        let editor = Editor::new(
            DVec2::new(config.width as f64, config.height as f64),
            window.scale_factor(),
        );
        Self {
            window,
            surface,
            device,
            queue,
            config,
            renderer,
            snapshotter,
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
        let revision = self.editor.doc().revision();
        if self.drawn_revision != Some(revision) {
            self.renderer.set_model(self.editor.doc().model());
            self.drawn_revision = Some(revision);
        }
        self.renderer.render(
            &view,
            (self.config.width, self.config.height),
            &self.editor.doc().camera,
            &self.editor.overlay(),
        );
        self.window.pre_present_notify();
        self.queue.present(frame);
    }

    /// Handles waiting API requests; redraws if they changed the model or camera.
    fn process_api(&mut self, hub: &mut Hub) {
        let before = (
            self.editor.doc().revision(),
            self.editor.doc().camera.clone(),
            self.editor.overlay(),
        );
        let mut host = WindowHost {
            editor: &mut self.editor,
            snapshotter: &mut self.snapshotter,
        };
        hub.process(&mut host);
        if (
            self.editor.doc().revision(),
            self.editor.doc().camera.clone(),
            self.editor.overlay(),
        ) != before
        {
            self.window.request_redraw();
        }
    }

    /// Tells API subscribers about edits made in the window.
    fn notify_api(&mut self, hub: &mut Hub) {
        let host = WindowHost {
            editor: &mut self.editor,
            snapshotter: &mut self.snapshotter,
        };
        hub.notify(&host);
    }
}

fn map_button(button: MouseButton) -> Button {
    match button {
        MouseButton::Left => Button::Left,
        MouseButton::Middle => Button::Middle,
        _ => Button::Other,
    }
}

pub struct App {
    state: Option<State>,
    hub: Hub,
    /// Socket the agent API listens on, removed on exit (None if listening failed).
    socket: Option<PathBuf>,
}

impl App {
    /// `socket` is where the agent API listens; None runs without it.
    pub fn new(event_loop: &EventLoop<ApiWake>, socket: Option<PathBuf>) -> Self {
        let proxy = event_loop.create_proxy();
        let hub = Hub::new(move || {
            let _ = proxy.send_event(ApiWake);
        });
        let socket = socket.and_then(|socket| match listen_unix(&socket, hub.inbox()) {
            Ok(()) => {
                eprintln!("agent API listening on {}", socket.display());
                Some(socket)
            }
            Err(e) => {
                eprintln!(
                    "agent API disabled: cannot listen on {}: {e}",
                    socket.display()
                );
                None
            }
        });
        Self {
            state: None,
            hub,
            socket,
        }
    }
}

impl ApplicationHandler<ApiWake> for App {
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(path) = self.socket.take() {
            let _ = std::fs::remove_file(path);
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ApiWake) {
        if let Some(s) = self.state.as_mut() {
            s.process_api(&mut self.hub);
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("sketchup-alt")
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let mut state = State::new(window);
        state.window.request_redraw();
        // Requests that arrived before the window existed.
        state.process_api(&mut self.hub);
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
                s.editor.set_scale_factor(scale_factor);
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
                // Use the physical Z key only when the layout's character is not ASCII (e.g.
                // Arabic or Cyrillic), so Cmd+W on AZERTY does not undo.
                let is_z = match &event.logical_key {
                    Key::Character(c) if c.is_ascii() => c.eq_ignore_ascii_case("z"),
                    _ => event.physical_key == PhysicalKey::Code(KeyCode::KeyZ),
                };
                if event.logical_key == Key::Named(NamedKey::Escape) {
                    s.editor.cancel()
                } else if command_key && is_z {
                    if s.modifiers.shift_key() {
                        s.editor.redo()
                    } else {
                        s.editor.undo()
                    }
                } else {
                    false
                }
            }
            _ => false,
        };
        if changed {
            s.window.request_redraw();
            s.notify_api(&mut self.hub);
        }
    }
}
