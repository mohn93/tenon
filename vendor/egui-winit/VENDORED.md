# Vendored egui-winit

Source: crates.io `egui-winit` 0.36.2 (licence: MIT OR Apache-2.0; the texts are in
LICENSE-MIT and LICENSE-APACHE, copied from the upstream egui repository because the crates.io
package lacks them). Used through `[patch.crates-io]` in the root `Cargo.toml`; the workspace
excludes this folder.

## Change

Only `src/dropped_file.rs`: the `egui::DroppedFile` impl for `NativeFile` now has
`#[cfg(not(target_arch = "wasm32"))]` on the existing `bytes` method and a
`#[cfg(target_arch = "wasm32")]` `bytes_async` method that returns an error. egui 0.36 swaps
those two trait methods on wasm32, so the unpatched crate does not compile for the web.
Dropped files are not used on the web (the app opens files through its `Platform`).

## When to drop it

When a released egui-winit compiles for wasm32 and the workspace may move to it, delete this
folder, the `[patch.crates-io]` entry and the `exclude` line in the root `Cargo.toml`.
