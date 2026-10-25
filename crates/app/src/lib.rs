//! The sketchup-alt app as a library: the window (`app::App`, started with a `platform::Platform`
//! and, on the web, a GPU context made beforehand) and, on desktops, the command line, the
//! headless and MCP modes. `main.rs` is the desktop's thin binary; the web crate calls
//! `app::App` itself.

pub mod app;
pub mod platform;

#[cfg(not(target_arch = "wasm32"))]
mod awake;
#[cfg(not(target_arch = "wasm32"))]
pub mod cli;
pub mod clipboard;
mod editor;
mod feedback;
mod flows;
mod fly;
mod gui;
#[cfg(not(target_arch = "wasm32"))]
pub mod headless;
mod host;
mod image_labels;
mod info;
mod library;
#[cfg(not(target_arch = "wasm32"))]
pub mod mcp;
mod menus;
#[cfg(target_os = "macos")]
mod native_menu;
mod navigation;
pub mod paths;
pub mod prefs;
mod recovered;
#[cfg(not(target_arch = "wasm32"))]
pub mod recovery;
#[cfg(target_arch = "wasm32")]
#[path = "recovery_off.rs"]
pub mod recovery;
#[cfg(not(target_arch = "wasm32"))]
mod share_server;
pub mod shortcuts;
#[cfg(target_os = "macos")]
mod system_quit;
mod touch;
mod ui;
pub mod update;
pub mod viewer;
#[cfg(windows)]
pub mod windows;
