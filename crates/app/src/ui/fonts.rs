//! The fonts built into the binary: Inter for text, JetBrains Mono for values and Phosphor for
//! icons (see `assets/fonts/README.md`).

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

pub const INTER: &str = "inter";
pub const INTER_SEMIBOLD: &str = "inter-semibold";
pub const MONO: &str = "jetbrains-mono";

static INTER_REGULAR_TTF: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
static INTER_SEMIBOLD_TTF: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
static MONO_TTF: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");

/// egui's fonts with Inter first for text (then the Phosphor icons, then egui's own fonts as
/// fallbacks), JetBrains Mono first for monospace, and an Inter SemiBold family for titles.
pub fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        (INTER, INTER_REGULAR_TTF),
        (INTER_SEMIBOLD, INTER_SEMIBOLD_TTF),
        (MONO, MONO_TTF),
    ] {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, INTER.into());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, MONO.into());
    // Second in the proportional list, after Inter.
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    let mut titles = vec![INTER_SEMIBOLD.to_string()];
    titles.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts.families.insert(semibold(), titles);
    fonts
}

/// Inter SemiBold, for section and window titles.
pub fn semibold() -> FontFamily {
    FontFamily::Name(INTER_SEMIBOLD.into())
}
