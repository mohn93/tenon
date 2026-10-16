mod app;
mod awake;
mod cli;
mod editor;
mod fly;
mod gui;
mod headless;
mod host;
mod info;
mod navigation;
mod ui;

use std::path::PathBuf;

use winit::event_loop::{ControlFlow, EventLoop};

use crate::cli::{Mode, Transport};

/// The folders given with `--api-root`, resolved; a bad one stops the app.
fn roots(dirs: &[PathBuf]) -> api::ApiRoots {
    let roots = api::ApiRoots::new(dirs).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    if !roots.is_empty() {
        let dirs: Vec<_> = roots
            .dirs()
            .iter()
            .map(|d| d.display().to_string())
            .collect();
        eprintln!("agent API files limited to: {}", dirs.join(", "));
    }
    roots
}

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
        Mode::Help => println!("{}", cli::USAGE),
        Mode::Headless {
            transport,
            file,
            api_roots,
        } => {
            let roots = roots(&api_roots);
            if let Transport::Socket(path) = &transport
                && let Some(dir) = cli::private_dir_for(path)
                && let Err(e) = cli::ensure_private_dir(dir)
            {
                eprintln!("cannot use {}: {e}", dir.display());
                std::process::exit(1);
            }
            std::process::exit(headless::run(transport, file, roots))
        }
        Mode::Window {
            socket,
            file,
            api_roots,
        } => {
            let roots = roots(&api_roots);
            // A socket in the fallback directory is only safe if that directory is private.
            let socket = match cli::private_dir_for(&socket).map(cli::ensure_private_dir) {
                Some(Err(e)) => {
                    eprintln!("agent API disabled: cannot use {}: {e}", socket.display());
                    None
                }
                _ => Some(socket),
            };
            let mut builder = EventLoop::<app::ApiWake>::with_user_event();
            // The default macOS menu's Quit would exit without the unsaved-changes prompt;
            // Cmd+Q is handled by the app instead.
            #[cfg(target_os = "macos")]
            winit::platform::macos::EventLoopBuilderExtMacOS::with_default_menu(
                &mut builder,
                false,
            );
            let event_loop = builder.build().expect("create event loop");
            // Redraw on demand only; the loop sleeps until input or an API request arrives.
            event_loop.set_control_flow(ControlFlow::Wait);
            let recovery = std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(cli::FALLBACK_DIR).join("recovery"));
            if recovery.is_none() {
                eprintln!(
                    "warning: HOME is not set: work an agent discards in the window is not kept"
                );
            }
            let mut app = app::App::new(&event_loop, socket, file, roots, recovery);
            event_loop.run_app(&mut app).expect("run app");
        }
    }
}
