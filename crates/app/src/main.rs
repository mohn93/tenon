mod app;
mod editor;
mod navigation;

use winit::event_loop::{ControlFlow, EventLoop};

fn main() {
    env_logger::init();
    let event_loop = EventLoop::new().expect("create event loop");
    // Redraw on demand only; the loop sleeps until input arrives.
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop
        .run_app(&mut app::App::default())
        .expect("run app");
}
