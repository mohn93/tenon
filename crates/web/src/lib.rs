//! sketchup-alt in a browser canvas. The page loads the module built from this crate
//! (`scripts/build-web.sh`); `start` runs when it loads.

pub mod names;
pub mod viewer;

#[cfg(target_arch = "wasm32")]
mod files;
#[cfg(target_arch = "wasm32")]
mod load;
#[cfg(target_arch = "wasm32")]
mod platform;
#[cfg(target_arch = "wasm32")]
mod start;
