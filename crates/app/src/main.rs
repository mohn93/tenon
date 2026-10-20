mod app;
mod awake;
mod cli;
mod editor;
mod feedback;
mod fly;
mod gui;
mod headless;
mod host;
mod info;
mod mcp;
mod menus;
#[cfg(target_os = "macos")]
mod native_menu;
mod navigation;
mod paths;
mod prefs;
mod shortcuts;
mod ui;
#[cfg(windows)]
mod windows;

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
    let args = cli::unicode_args(std::env::args_os().skip(1)).unwrap_or_else(|message| {
        eprintln!("{message}");
        std::process::exit(2);
    });
    let mode = match cli::parse(args) {
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
            if let Transport::Socket(endpoint) = &transport
                && let Some(dir) = cli::private_dir_for_endpoint(endpoint)
                && let Err(e) = cli::ensure_private_dir(dir)
            {
                eprintln!("cannot use {}: {e}", dir.display());
                std::process::exit(1);
            }
            std::process::exit(headless::run(transport, file, roots))
        }
        Mode::Mcp {
            live,
            file,
            api_roots,
        } => {
            let roots = roots(&api_roots);
            std::process::exit(mcp::run(live, file, roots))
        }
        Mode::Window {
            socket,
            file,
            api_roots,
        } => {
            let roots = roots(&api_roots);
            // Started from Explorer: no console window beside the app's. Only once the
            // arguments are known good, so a message about a bad one stays readable.
            #[cfg(windows)]
            windows::hide_own_console();
            // A socket in the fallback directory is only safe if that directory is private.
            let socket = match cli::private_dir_for_endpoint(&socket).map(cli::ensure_private_dir) {
                Some(Err(e)) => {
                    eprintln!("agent API disabled: cannot use {socket}: {e}");
                    None
                }
                _ => Some(socket),
            };
            let mut builder = EventLoop::<app::AppEvent>::with_user_event();
            // The app builds its own macOS menu bar (`native_menu`), whose Quit asks about
            // unsaved changes; winit's default one would exit without asking.
            #[cfg(target_os = "macos")]
            winit::platform::macos::EventLoopBuilderExtMacOS::with_default_menu(
                &mut builder,
                false,
            );
            let event_loop = builder.build().expect("create event loop");
            // Redraw on demand only; the loop sleeps until input or an API request arrives.
            event_loop.set_control_flow(ControlFlow::Wait);
            // Without an absolute HOME (on Windows, application data folders) the files would
            // land under the working folder.
            let found = paths::resolve(
                paths::Os::current(),
                &|k| std::env::var_os(k),
                &std::env::temp_dir(),
                std::process::id(),
            );
            if found.is_none() {
                let what = if cfg!(windows) {
                    "APPDATA or LOCALAPPDATA is not set, nor USERPROFILE"
                } else {
                    "HOME is not set"
                };
                eprintln!(
                    "warning: {what}: work an agent discards in the window is not kept, and preferences last only for the session"
                );
            }
            let recovery = found.as_ref().map(|p| p.recovery.clone());
            let prefs = found.map(|p| prefs::PrefsFile {
                path: p.prefs,
                legacy: p.legacy_prefs,
            });
            let mut app = app::App::new(&event_loop, socket, file, roots, recovery, prefs);
            event_loop.run_app(&mut app).expect("run app");
        }
    }
}
