# Licence texts for crates that publish none

A few crates.io packages ship without a LICENSE file. `scripts/licenses.py` reads these copies
instead, taken from the crate's upstream repository (the `repository` field of its Cargo.toml),
for the licences its Cargo.toml declares. `dispatch-MIT.txt` is the standard MIT text with the
copyright holder from the crate's `authors`, as its repository holds no licence file.

| File | Crates |
| --- | --- |
| accesskit-* | accesskit, accesskit_atspi_common, accesskit_consumer, accesskit_macos, accesskit_unix, accesskit_windows, accesskit_winit (Apache-2.0 only) |
| objc2-* | block2, dispatch2, objc-sys, objc2 and the objc2-* binding crates |
| egui-* | ecolor, egui, egui-wgpu, emath, epaint, epaint_default_fonts |
| defmt-* | defmt-parser |
| profiling-* | profiling |
| clipboard-win-BSL-1.0.txt | clipboard-win |
| rspirv-Apache-2.0.txt | spirv |
| draco-core-Apache-2.0.txt | draco-core (the LICENSE of its repository, Filyus/draco-rust, which is Google Draco's) |
| dispatch-MIT.txt | dispatch |
| phosphor-icons-MIT.txt | egui-phosphor (its icon fonts; from phosphor-icons/web, which the crate's README names) |
