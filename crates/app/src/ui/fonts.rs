//! The fonts built into the binary: Inter for text, JetBrains Mono for values and Phosphor for
//! icons (see `assets/fonts/README.md`; Inter Regular comes from the session crate, which draws
//! 3D text with it).
//!
//! The icons have a family of their own, with Phosphor first: Inter 4 has glyphs of its own at
//! some of the Private Use Area codepoints Phosphor uses (the cursor, the eraser, the undo
//! arrows...), so in the proportional family, where Inter comes first, those icons would be
//! drawn as Inter's glyphs. Phosphor cannot come first there instead: it maps the space, '-'
//! and 'a'-'z' to the blank glyphs its ligatures are built from, which would blank out text.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

pub const INTER: &str = "inter";
pub const INTER_SEMIBOLD: &str = "inter-semibold";
pub const MONO: &str = "jetbrains-mono";
/// The name egui_phosphor gives its font.
pub const PHOSPHOR: &str = "phosphor";

static INTER_REGULAR_TTF: &[u8] = session::text3d::INTER_REGULAR_TTF;
static INTER_SEMIBOLD_TTF: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
static MONO_TTF: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");

/// egui's fonts with Inter first for text (then the Phosphor icons, then egui's own fonts as
/// fallbacks), JetBrains Mono first for monospace, an Inter SemiBold family for titles and a
/// Phosphor family for the icons.
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
    // Phosphor first, then the text fonts as fallbacks.
    let mut icon_fonts = vec![PHOSPHOR.to_string()];
    icon_fonts.extend(
        fonts.families[&FontFamily::Proportional]
            .iter()
            .filter(|name| *name != PHOSPHOR)
            .cloned(),
    );
    fonts.families.insert(icons(), icon_fonts);
    fonts
}

/// Inter SemiBold, for section and window titles.
pub fn semibold() -> FontFamily {
    FontFamily::Name(INTER_SEMIBOLD.into())
}

/// Phosphor, for the icons: every glyph in `icons` is drawn in this family.
pub fn icons() -> FontFamily {
    FontFamily::Name(PHOSPHOR.into())
}

#[cfg(test)]
mod tests {
    use egui::{FontFamily, FontId};

    use super::super::icons;

    #[test]
    fn every_icon_is_drawn_from_phosphor() {
        let mut fonts = super::definitions();
        // Phosphor alone, to compare against.
        let only = FontFamily::Name("only-phosphor".into());
        fonts
            .families
            .insert(only.clone(), vec![super::PHOSPHOR.into()]);
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts);
        ctx.run_ui(egui::RawInput::default(), |_| {})
            .drop_without_applying_deltas();
        // Where egui put the glyph it laid out for `icon` in `family` in the font atlas: one
        // place per font face, glyph and size.
        let atlas_rect = |icon: &str, family: FontFamily| {
            let galley = ctx.fonts_mut(|f| {
                f.layout_no_wrap(icon.into(), FontId::new(16.0, family), egui::Color32::WHITE)
            });
            galley.rows[0].glyphs[0].uv_rect
        };
        let wrong: Vec<String> = icons::all()
            .into_iter()
            .filter(|icon| atlas_rect(icon, super::icons()) != atlas_rect(icon, only.clone()))
            .map(|icon| format!("U+{:04X}", u32::from(icon.chars().next().unwrap())))
            .collect();
        assert_eq!(
            wrong,
            Vec::<String>::new(),
            "icons drawn from another font than Phosphor"
        );
    }
}
