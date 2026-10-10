mod app;
mod cli;
mod editor;
mod headless;
mod host;
mod navigation;

use winit::event_loop::{ControlFlow, EventLoop};

use crate::cli::{Mode, Transport};

fn main() {
    env_logger::init();
    let mode = match cli::parse(std::env::args().skip(1)) {
        Ok(mode) => mode,
        Err(message) => {
            eprintln!("{message}\n{}", cli::USAGE);
            std::process::exit(2);
        }
    };
    match mode {
        Mode::Headless { transport } => {
            if let Transport::Socket(path) = &transport
                && let Some(dir) = cli::private_dir_for(path)
                && let Err(e) = cli::ensure_private_dir(dir)
            {
                eprintln!("cannot use {}: {e}", dir.display());
                std::process::exit(1);
            }
            std::process::exit(headless::run(transport))
        }
        Mode::Window { socket } => {
            // A socket in the fallback directory is only safe if that directory is private.
            let socket = match cli::private_dir_for(&socket).map(cli::ensure_private_dir) {
                Some(Err(e)) => {
                    eprintln!("agent API disabled: cannot use {}: {e}", socket.display());
                    None
                }
                _ => Some(socket),
            };
            let event_loop = EventLoop::<app::ApiWake>::with_user_event()
                .build()
                .expect("create event loop");
            // Redraw on demand only; the loop sleeps until input or an API request arrives.
            event_loop.set_control_flow(ControlFlow::Wait);
            let mut app = app::App::new(&event_loop, socket);
            event_loop.run_app(&mut app).expect("run app");
        }
    }
}
