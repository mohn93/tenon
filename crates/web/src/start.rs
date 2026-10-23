//! The module's entry point: finds the canvas, makes the GPU objects (WebGPU, else WebGL2),
//! and hands the window app to winit's web event loop.

use std::cell::RefCell;
use std::rc::Rc;

use app::app::{App, AppConfig, AppEvent, GpuContext, SharedModel};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Event, HtmlCanvasElement, KeyboardEvent};
use winit::event_loop::EventLoop;
use winit::platform::web::{EventLoopExtWebSys, WindowAttributesExtWebSys};
use winit::window::Window;

use crate::files::{describe, document, window};
use crate::idb;
use crate::load;
use crate::names::{self, CANVAS_ID};
use crate::platform::WebPlatform;
use crate::recent::{self, Info, Recent, Record};
use crate::viewer::{self, Start};

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

/// The share the page's fragment names, fetched and opened with its key: the model and name
/// and what the Recent list keeps of it (None: no share, the editor starts). The key is only
/// ever in the fragment: the request is for the id alone, and it is taken out of the address
/// bar before the request.
async fn shared_model() -> Result<Option<(SharedModel, Record)>, String> {
    let start = viewer::classify(&load::hash());
    // The key leaves the address bar and the history before any request, whatever follows.
    if start.forgets_fragment() {
        load::forget_fragment();
    }
    let link = match start {
        Start::Editor => return Ok(None),
        Start::Damaged => return Err(viewer::DAMAGED.to_string()),
        Start::Viewer(link) => link,
    };
    let blob = load::fetch_blob(&viewer::blob_path(&link.id))
        .await
        .map_err(|failure| failure.message().to_string())?;
    let (name, bytes) = session::share::open(&blob, &link.id, &link.key)
        .map_err(|_| viewer::DAMAGED.to_string())?;
    let record = Record {
        info: Info {
            id: recent::id_hex(&link.id),
            name: session::share::clip_name(&name).to_string(),
            time: js_sys::Date::now(),
            size: blob.len(),
        },
        key: link.key,
        blob,
    };
    Ok(Some((SharedModel { name, bytes }, record)))
}

/// A Recent list whose storage opens in the background, so a browser that is slow with
/// IndexedDB (or never answers: 3 seconds, then nothing is kept) cannot hold the viewer back.
fn recent_list() -> Rc<RefCell<Recent>> {
    let recent = Rc::new(RefCell::new(Recent::waiting()));
    let waiting = recent.clone();
    spawn_local(async move {
        let storage: Box<dyn recent::Storage> = match idb::open_within(3000).await {
            Some(db) => Box::new(db),
            None => Box::new(idb::Unavailable),
        };
        waiting.borrow_mut().attach(storage);
    });
    recent
}

/// Opening another share link in this tab only changes its fragment: the page reloads, which
/// then starts the viewer for it as for any new page.
fn reload_on_new_share() {
    let on_change = Closure::<dyn FnMut(Event)>::new(|_: Event| {
        if matches!(viewer::classify(&load::hash()), Start::Editor) {
            return;
        }
        let _ = window().location().reload();
    });
    let _ =
        window().add_event_listener_with_callback("hashchange", on_change.as_ref().unchecked_ref());
    on_change.forget();
}

async fn run() -> Result<(), String> {
    reload_on_new_share();
    let shared = shared_model().await?;
    // Only the viewer keeps models; the model just opened is the newest of them.
    let (model, recent) = match shared {
        Some((model, record)) => {
            let recent = recent_list();
            recent.borrow_mut().keep(record);
            (Some(model), Some(recent))
        }
        None => (None, None),
    };
    let canvas = canvas()?;
    let context = gpu(&canvas).await?;
    guard_input(&canvas);
    let _ = canvas.focus();

    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|e| format!("cannot start the event loop: {e}"))?;
    let mut config = AppConfig::new(Box::new(WebPlatform::new(
        event_loop.create_proxy(),
        recent,
    )));
    config.model = model;
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
