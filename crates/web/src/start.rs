//! The module's entry point: finds the canvas, makes the GPU objects (WebGPU, else WebGL2),
//! and hands the window app to winit's web event loop.

use app::app::{App, AppConfig, AppEvent, GpuContext};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Event, HtmlCanvasElement, KeyboardEvent};
use winit::event_loop::EventLoop;
use winit::platform::web::{EventLoopExtWebSys, WindowAttributesExtWebSys};
use winit::window::Window;

use crate::files::{describe, document, window};
use crate::names::{self, CANVAS_ID};
use crate::platform::WebPlatform;

#[wasm_bindgen(start)]
pub fn start() {
    // Without a panic hook a panic is an "unreachable" trap with no message.
    std::panic::set_hook(Box::new(|info| {
        let text = format!("sketchup-alt crashed: {info}");
        web_sys::console::error_1(&text.clone().into());
        show_problem(&text);
    }));
    spawn_local(async {
        if let Err(why) = run().await {
            web_sys::console::error_1(&why.clone().into());
            show_problem(&why);
        }
    });
}

/// Tells the person what went wrong in the page's `#problem` element (when it has one).
fn show_problem(text: &str) {
    if let Some(element) = document().get_element_by_id("problem") {
        element.set_text_content(Some(text));
        let _ = element.remove_attribute("hidden");
    }
}

/// The page's canvas, or a new one filling the window.
fn canvas() -> Result<HtmlCanvasElement, String> {
    let document = document();
    if let Some(element) = document.get_element_by_id(CANVAS_ID) {
        return element
            .dyn_into()
            .map_err(|_| format!("#{CANVAS_ID} is not a canvas"));
    }
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .map_err(|e| describe(&e))?
        .dyn_into()
        .map_err(|_| "cannot make a canvas".to_string())?;
    canvas.set_id(CANVAS_ID);
    let style = canvas.style();
    for (property, value) in [
        ("position", "fixed"),
        ("inset", "0"),
        ("width", "100%"),
        ("height", "100%"),
        ("touch-action", "none"),
    ] {
        let _ = style.set_property(property, value);
    }
    document
        .body()
        .ok_or("the page has no body")?
        .append_child(&canvas)
        .map_err(|e| describe(&e))?;
    Ok(canvas)
}

/// WebGPU when the browser has it, else WebGL2; the device is asked for before the app starts
/// because a browser only answers asynchronously.
async fn gpu(canvas: &HtmlCanvasElement) -> Result<GpuContext, String> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL;
    // Drops WebGPU from the backends when the browser defines it but cannot make an adapter.
    let instance = wgpu::util::new_instance_with_webgpu_detection(descriptor).await;
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
        .map_err(|e| format!("cannot draw to the page: {e}"))?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        })
        .await
        .map_err(|e| format!("this browser has neither WebGPU nor WebGL2 ({e})"))?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_limits: render::required_limits(&adapter),
            ..Default::default()
        })
        .await
        .map_err(|e| format!("cannot get a GPU device: {e}"))?;
    Ok(GpuContext {
        instance,
        surface,
        adapter,
        device,
        queue,
    })
}

/// What the page does with input on the canvas, so the browser does not also act on it: keys
/// the app handles (`names::should_prevent_default`), the context menu, wheel and pinch zoom,
/// touch scrolling, and a middle click's autoscroll. winit's own blanket `preventDefault` is
/// off so the browser's shortcuts (reload, developer tools, copy and paste events) still work.
fn guard_input(canvas: &HtmlCanvasElement) {
    let on_key = Closure::<dyn FnMut(KeyboardEvent)>::new(|event: KeyboardEvent| {
        if names::should_prevent_default(
            &event.key(),
            event.ctrl_key(),
            event.meta_key(),
            event.alt_key(),
            event.shift_key(),
        ) {
            event.prevent_default();
        }
    });
    let _ = canvas.add_event_listener_with_callback("keydown", on_key.as_ref().unchecked_ref());
    on_key.forget();

    let prevent = Closure::<dyn FnMut(Event)>::new(|event: Event| event.prevent_default());
    for name in ["contextmenu", "wheel", "touchstart", "touchmove"] {
        let _ = canvas.add_event_listener_with_callback(name, prevent.as_ref().unchecked_ref());
    }
    prevent.forget();

    // A press focuses the canvas (keys go to it) and middle-click does not autoscroll.
    let on_press = {
        let canvas = canvas.clone();
        Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            event.prevent_default();
            let _ = canvas.focus();
        })
    };
    let _ = canvas.add_event_listener_with_callback("mousedown", on_press.as_ref().unchecked_ref());
    on_press.forget();
}

async fn run() -> Result<(), String> {
    let canvas = canvas()?;
    let context = gpu(&canvas).await?;
    guard_input(&canvas);
    let _ = canvas.focus();

    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|e| format!("cannot start the event loop: {e}"))?;
    let mut config = AppConfig::new(Box::new(WebPlatform::new()));
    config.gpu = Some(context);
    config.window = Some(
        Window::default_attributes()
            .with_title("sketchup-alt")
            .with_canvas(Some(canvas))
            .with_prevent_default(false),
    );
    let app = App::new(event_loop.create_proxy(), config);
    if let Some(loading) = document().get_element_by_id("loading") {
        loading.remove();
    }
    let _ = window();
    event_loop.spawn_app(app);
    Ok(())
}
