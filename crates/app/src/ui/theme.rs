//! Design tokens: the dark and light color sets, the spacing grid, corner radii and type sizes,
//! and how they map onto egui's style.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Vec2};
use kernel::EntityId;
use serde::{Deserialize, Serialize};
use tools::ToolKind;

use super::fonts;

/// Which theme the window uses; System follows the operating system's appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    System,
    Dark,
    Light,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light];

    /// The name in View > Theme.
    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System",
            ThemeChoice::Dark => "Dark",
            ThemeChoice::Light => "Light",
        }
    }
}

/// One theme's colors. The whole set the spec names is defined here; a few tokens (`success`)
/// get their first user in a later milestone.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    /// Behind the panels.
    pub bg_app: Color32,
    pub bg_panel: Color32,
    /// Inputs, menus and windows.
    pub bg_raised: Color32,
    pub bg_hover: Color32,
    pub bg_pressed: Color32,
    pub border: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_disabled: Color32,
    pub accent: Color32,
    /// The accent under selected rows.
    pub accent_fill: Color32,
    /// Text and icons on an accent fill.
    pub on_accent: Color32,
    pub warn: Color32,
    pub danger: Color32,
    pub success: Color32,
    /// The 3D view's day canvas, top and bottom.
    pub canvas_top: Color32,
    pub canvas_bottom: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// `rgb` at `alpha` out of 255.
const fn hex_alpha(rgb: u32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied_const((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, alpha)
}

pub const DARK: Tokens = Tokens {
    bg_app: hex(0x141518),
    bg_panel: hex(0x1C1D21),
    bg_raised: hex(0x25272C),
    bg_hover: hex(0x2E3036),
    bg_pressed: hex(0x373A41),
    border: hex(0x33363C),
    text_primary: hex(0xE8E9EC),
    text_secondary: hex(0xA3A7B0),
    text_disabled: hex(0x6B6F78),
    accent: hex(0x4C8DFF),
    // 22%
    accent_fill: hex_alpha(0x4C8DFF, 56),
    on_accent: Color32::WHITE,
    warn: hex(0xF5B544),
    danger: hex(0xF2555A),
    success: hex(0x3DBE74),
    canvas_top: hex(0x4A4D55),
    canvas_bottom: hex(0x383B42),
};

pub const LIGHT: Tokens = Tokens {
    bg_app: hex(0xECEEF1),
    bg_panel: hex(0xFFFFFF),
    bg_raised: hex(0xF6F7F9),
    bg_hover: hex(0xE9ECF0),
    bg_pressed: hex(0xDFE3E8),
    border: hex(0xD8DCE2),
    text_primary: hex(0x17191E),
    text_secondary: hex(0x5B616C),
    text_disabled: hex(0x9EA4AD),
    accent: hex(0x2463EB),
    // 14%
    accent_fill: hex_alpha(0x2463EB, 36),
    on_accent: Color32::WHITE,
    warn: hex(0xB7791F),
    danger: hex(0xD93036),
    success: hex(0x1E9E57),
    canvas_top: hex(0xF4F6F8),
    canvas_bottom: hex(0xE3E7EC),
};

/// The tokens of the theme egui resolved (for System, the one the window reported).
pub fn tokens(theme: egui::Theme) -> &'static Tokens {
    match theme {
        egui::Theme::Dark => &DARK,
        egui::Theme::Light => &LIGHT,
    }
}

/// How far faces are tinted toward the accent when selected.
pub const SELECTION_TINT: f32 = 0.18;
/// How far the grid and axes fade toward the dark theme's day canvas (black edges keep theirs).
pub const DARK_GROUND_FADE: f32 = 0.6;

/// How the window's 3D view looks in `theme`: its day canvas, the selection in the accent and
/// the axes in the shared axis colors, 1.5 pt wide.
pub fn view_style(theme: egui::Theme) -> render::ViewStyle {
    let t = tokens(theme);
    let rgba = |c: Color32| {
        let [r, g, b] = [c.r(), c.g(), c.b()].map(|u| f32::from(u) / 255.0);
        [r, g, b, 1.0]
    };
    let accent = rgba(t.accent);
    render::ViewStyle {
        day_canvas: [rgba(t.canvas_top), rgba(t.canvas_bottom)],
        day_ground_fade: match theme {
            egui::Theme::Dark => DARK_GROUND_FADE,
            egui::Theme::Light => 0.0,
        },
        selected_edge: accent,
        selected_face: [accent[0], accent[1], accent[2]],
        face_tint: SELECTION_TINT,
        wide_selected_edges: true,
        axes: [rgba(AXIS_X), rgba(AXIS_Y), rgba(AXIS_Z)],
        wide_axes: true,
    }
}

/// How far a hovered face is tinted toward the accent (or, for Erase, the danger color).
pub const HOVER_TINT: f32 = 0.25;

/// How the entity a click of `tool` would act on (`target`) is highlighted in `theme`: faces
/// tinted toward the accent with accent edges, in the danger color for Erase. Paint shows a
/// face in the color it would paint (`paint`, sRGB) with accent edges; a group, or anything
/// while `sampling` (a click picks a material up), as Select.
pub fn hover_look(
    theme: egui::Theme,
    tool: ToolKind,
    target: EntityId,
    paint: [u8; 3],
    sampling: bool,
) -> render::HoverLook {
    let t = tokens(theme);
    let unit = |c: Color32| [c.r(), c.g(), c.b()].map(|u| f32::from(u) / 255.0);
    let edge = |[r, g, b]: [f32; 3]| [r, g, b, 1.0];
    let accent = unit(t.accent);
    match (tool, target) {
        (ToolKind::Erase, _) => {
            let danger = unit(t.danger);
            render::HoverLook {
                face: danger,
                face_mix: HOVER_TINT,
                edge: edge(danger),
            }
        }
        (ToolKind::Paint, EntityId::Face(_)) if !sampling => render::HoverLook {
            face: paint.map(|u| f32::from(u) / 255.0),
            face_mix: 1.0,
            edge: edge(accent),
        },
        _ => render::HoverLook {
            face: accent,
            face_mix: HOVER_TINT,
            edge: edge(accent),
        },
    }
}

/// The spacing grid every gap and padding is a multiple of.
pub const GRID: f32 = 4.0;
/// A control's height.
pub const CONTROL_HEIGHT: f32 = 28.0;
pub const CONTROL_RADIUS: u8 = 6;
/// Windows and menus.
pub const WINDOW_RADIUS: u8 = 8;
/// Swatches and key chips.
pub const SWATCH_RADIUS: u8 = 4;
/// The width of the accent ring around a focused or chosen control.
pub const FOCUS_RING: f32 = 2.0;
pub const BODY_SIZE: f32 = 13.0;
pub const SMALL_SIZE: f32 = 11.0;
/// Section titles, shown uppercase.
pub const SECTION_SIZE: f32 = 11.0;
/// The gap above a section title.
pub const SECTION_GAP: f32 = 12.0;
pub const SWATCH_SIZE: f32 = 16.0;
/// A material tile's side in the Materials grid, and the gap between tiles.
pub const TILE: f32 = 56.0;
pub const TILE_GAP: f32 = 8.0;
/// Icon glyphs in buttons.
pub const ICON_SIZE: f32 = 16.0;
/// The tool rail's width, its buttons' side, their glyph size, and the gap between groups.
pub const RAIL_WIDTH: f32 = 44.0;
pub const RAIL_BUTTON: f32 = 32.0;
pub const RAIL_ICON: f32 = 18.0;
pub const RAIL_GROUP_GAP: f32 = 8.0;
/// The context bar's height, under the menus at the window's top.
pub const CONTEXT_BAR_HEIGHT: f32 = 36.0;
/// The status bar's height, and the measurements box's size in it.
pub const STATUS_HEIGHT: f32 = 28.0;
pub const MEASUREMENTS_WIDTH: f32 = 180.0;
pub const MEASUREMENTS_HEIGHT: f32 = 24.0;
/// The snap chip by the cursor: its top-left's offset from the cursor, its padding and its
/// corner radius. The flash of a refused value has the same metrics.
pub const CHIP_OFFSET: Vec2 = Vec2::new(14.0, 18.0);
pub const CHIP_PADDING: Vec2 = Vec2::new(6.0, 4.0);
pub const CHIP_RADIUS: u8 = SWATCH_RADIUS;
/// The gap between the chip and the flash under it, and between the flash and the status
/// bar when the cursor is not in the view.
pub const CHIP_GAP: f32 = GRID;
/// The axis colors of a lock: red x, green y, blue z.
pub const AXIS_X: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
pub const AXIS_Y: Color32 = Color32::from_rgb(0x30, 0xA4, 0x6C);
pub const AXIS_Z: Color32 = Color32::from_rgb(0x3E, 0x63, 0xDD);
/// The orientation gizmo: its size, its inset from the view's top-right corner, the length of
/// an axis seen side-on, the radii of a positive end's cap, a negative end's ring and the
/// centre, and how near the centre an end is hidden (its axis points at or away from the
/// viewer).
pub const GIZMO: f32 = 72.0;
pub const GIZMO_INSET: f32 = 12.0;
pub const GIZMO_AXIS: f32 = 26.0;
pub const GIZMO_CAP: f32 = 8.0;
pub const GIZMO_NEG_CAP: f32 = 5.0;
pub const GIZMO_CENTRE: f32 = 7.0;
pub const GIZMO_HIDE: f32 = 10.0;

/// The empty-model card: its size (border included), its inset from the view's bottom left
/// corner, its padding and the height of its recent file rows.
pub const START_CARD: Vec2 = Vec2::new(360.0, 200.0);
pub const START_CARD_INSET: f32 = 12.0;
pub const START_CARD_PADDING: f32 = 12.0;
pub const START_CARD_ROW: f32 = 18.0;

/// egui's style for one theme's tokens.
pub fn style(t: &Tokens, dark: bool) -> egui::Style {
    let mut style = egui::Style {
        visuals: if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        },
        ..Default::default()
    };
    let v = &mut style.visuals;
    v.panel_fill = t.bg_panel;
    v.window_fill = t.bg_raised;
    v.extreme_bg_color = t.bg_app;
    v.faint_bg_color = t.bg_raised;
    v.code_bg_color = t.bg_raised;
    v.text_edit_bg_color = Some(t.bg_raised);
    v.window_stroke = Stroke::new(1.0, t.border);
    v.window_corner_radius = CornerRadius::same(WINDOW_RADIUS);
    v.menu_corner_radius = CornerRadius::same(WINDOW_RADIUS);
    v.hyperlink_color = t.accent;
    v.warn_fg_color = t.warn;
    v.error_fg_color = t.danger;
    v.weak_text_color = Some(t.text_secondary);
    v.selection.bg_fill = t.accent_fill;
    // Also a focused text field's frame.
    v.selection.stroke = Stroke::new(FOCUS_RING, t.accent);
    let w = &mut v.widgets;
    for (state, fill) in [
        (&mut w.noninteractive, t.bg_panel),
        (&mut w.inactive, t.bg_raised),
        (&mut w.hovered, t.bg_hover),
        (&mut w.active, t.bg_pressed),
        (&mut w.open, t.bg_hover),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, t.border);
        state.fg_stroke = Stroke::new(1.0, t.text_primary);
        state.corner_radius = CornerRadius::same(CONTROL_RADIUS);
        state.expansion = 0.0;
    }
    // egui draws a focused widget as active, so this is the focus ring.
    w.active.bg_stroke = Stroke::new(FOCUS_RING, t.accent);

    let s = &mut style.spacing;
    s.item_spacing = Vec2::new(2.0 * GRID, GRID);
    s.button_padding = Vec2::new(2.0 * GRID, GRID);
    s.interact_size = Vec2::new(10.0 * GRID, CONTROL_HEIGHT);
    s.window_margin = Margin::same(12);
    s.menu_margin = Margin::same(8);
    s.indent = 4.0 * GRID;
    // Values in the value font: DragValue and Slider read this, not `override_text_style`.
    style.drag_value_text_style = TextStyle::Monospace;

    style.text_styles = [
        (
            TextStyle::Small,
            FontId::new(SMALL_SIZE, FontFamily::Proportional),
        ),
        (
            TextStyle::Body,
            FontId::new(BODY_SIZE, FontFamily::Proportional),
        ),
        (
            TextStyle::Button,
            FontId::new(BODY_SIZE, FontFamily::Proportional),
        ),
        (
            TextStyle::Heading,
            FontId::new(BODY_SIZE, fonts::semibold()),
        ),
        (
            TextStyle::Monospace,
            FontId::new(BODY_SIZE, FontFamily::Monospace),
        ),
    ]
    .into();
    style
}

/// Installs the fonts and both themes' styles on `ctx`, and picks `choice`.
pub fn install(ctx: &egui::Context, choice: ThemeChoice) {
    ctx.set_fonts(fonts::definitions());
    ctx.set_style_of(egui::Theme::Dark, style(&DARK, true));
    ctx.set_style_of(egui::Theme::Light, style(&LIGHT, false));
    set_choice(ctx, choice);
}

/// Switches to `choice`; the styles stay installed.
pub fn set_choice(ctx: &egui::Context, choice: ThemeChoice) {
    ctx.set_theme(match choice {
        ThemeChoice::System => egui::ThemePreference::System,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
        ThemeChoice::Light => egui::ThemePreference::Light,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hover_look_follows_the_tool() {
        let face = EntityId::Face(kernel::FaceId(1));
        let group = EntityId::Instance(kernel::InstanceId(2));
        let t = tokens(egui::Theme::Light);
        let rgb = |c: Color32| [c.r(), c.g(), c.b()].map(|u| f32::from(u) / 255.0);
        let select = hover_look(egui::Theme::Light, ToolKind::Select, face, [0; 3], false);
        assert_eq!((select.face, select.face_mix), (rgb(t.accent), HOVER_TINT));
        assert_eq!(&select.edge[..3], &rgb(t.accent)[..]);
        let push = hover_look(egui::Theme::Light, ToolKind::PushPull, face, [0; 3], false);
        assert_eq!(push, select);
        let erase = hover_look(egui::Theme::Light, ToolKind::Erase, face, [0; 3], false);
        assert_eq!(erase.face, rgb(t.danger));
        assert_eq!(&erase.edge[..3], &rgb(t.danger)[..]);
        let paint = hover_look(
            egui::Theme::Light,
            ToolKind::Paint,
            face,
            [255, 0, 0],
            false,
        );
        assert_eq!((paint.face, paint.face_mix), ([1.0, 0.0, 0.0], 1.0));
        assert_eq!(&paint.edge[..3], &rgb(t.accent)[..]);
        assert_eq!(
            hover_look(
                egui::Theme::Light,
                ToolKind::Paint,
                group,
                [255, 0, 0],
                false
            ),
            select
        );
    }

    #[test]
    fn paint_looks_like_select_while_sampling() {
        let face = EntityId::Face(kernel::FaceId(1));
        let select = hover_look(egui::Theme::Dark, ToolKind::Select, face, [0; 3], false);
        assert_eq!(
            hover_look(egui::Theme::Dark, ToolKind::Paint, face, [255, 0, 0], true),
            select,
            "a sampling click picks the material up, it paints nothing"
        );
    }

    #[test]
    fn the_view_follows_the_theme() {
        let unit = |c: Color32| [c.r(), c.g(), c.b()].map(|u| f32::from(u) / 255.0);
        for (theme, t) in [(egui::Theme::Dark, &DARK), (egui::Theme::Light, &LIGHT)] {
            let s = view_style(theme);
            let [r, g, b] = unit(t.accent);
            assert_eq!(s.selected_edge, [r, g, b, 1.0]);
            assert_eq!(s.selected_face, [r, g, b]);
            assert_eq!(s.face_tint, 0.18);
            assert!(s.wide_selected_edges);
            assert!(s.has_gradient());
            let [r, g, b] = unit(t.canvas_top);
            assert_eq!(s.day_canvas[0], [r, g, b, 1.0]);
            let [r, g, b] = unit(t.canvas_bottom);
            assert_eq!(s.day_canvas[1], [r, g, b, 1.0]);
        }
        assert_eq!(view_style(egui::Theme::Light).day_ground_fade, 0.0);
        assert!(
            view_style(egui::Theme::Dark).day_ground_fade > 0.0,
            "a dark canvas quiets the grid"
        );
    }

    #[test]
    fn the_views_axes_are_the_shared_axis_colors() {
        let unit = |c: Color32| [c.r(), c.g(), c.b()].map(|u| f32::from(u) / 255.0);
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let s = view_style(theme);
            for (axis, color) in s.axes.iter().zip([AXIS_X, AXIS_Y, AXIS_Z]) {
                let [r, g, b] = unit(color);
                assert_eq!(*axis, [r, g, b, 1.0]);
            }
            assert!(s.wide_axes);
        }
        let x = view_style(egui::Theme::Dark).axes[0];
        assert_eq!(x, [229.0 / 255.0, 72.0 / 255.0, 77.0 / 255.0, 1.0]);
    }

    /// WCAG contrast ratio of two opaque colors.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let channel = |u: u8| {
            let s = f32::from(u) / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        let lum = |c: Color32| {
            0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn the_accents_are_the_spec_values() {
        assert_eq!(DARK.accent, Color32::from_rgb(0x4C, 0x8D, 0xFF));
        assert_eq!(LIGHT.accent, Color32::from_rgb(0x24, 0x63, 0xEB));
    }

    #[test]
    fn text_and_accent_read_against_the_panels() {
        for t in [&DARK, &LIGHT] {
            assert!(contrast(t.text_primary, t.bg_panel) >= 7.0);
            assert!(contrast(t.text_secondary, t.bg_panel) >= 4.5);
            assert!(contrast(t.accent, t.bg_panel) >= 3.0);
            assert!(contrast(t.on_accent, t.accent) >= 3.0);
        }
    }

    #[test]
    fn the_style_follows_the_grid_radii_and_type_scale() {
        for (t, dark) in [(&DARK, true), (&LIGHT, false)] {
            let s = style(t, dark);
            assert_eq!(s.visuals.dark_mode, dark);
            assert_eq!(s.spacing.interact_size.y, CONTROL_HEIGHT);
            for v in [s.spacing.item_spacing, s.spacing.button_padding] {
                assert_eq!(
                    (v.x % GRID, v.y % GRID),
                    (0.0, 0.0),
                    "{v:?} on the 4 pt grid"
                );
            }
            assert_eq!(
                s.visuals.widgets.inactive.corner_radius,
                CornerRadius::same(6)
            );
            assert_eq!(s.visuals.window_corner_radius, CornerRadius::same(8));
            assert_eq!(
                s.visuals.widgets.active.bg_stroke,
                Stroke::new(2.0, t.accent)
            );
            assert_eq!(s.visuals.selection.stroke, Stroke::new(2.0, t.accent));
            assert_eq!(s.visuals.selection.bg_fill, t.accent_fill);
            assert_eq!(s.visuals.panel_fill, t.bg_panel);
            let size = |ts: TextStyle| s.text_styles[&ts].size;
            assert_eq!(
                (size(TextStyle::Body), size(TextStyle::Small)),
                (13.0, 11.0)
            );
            assert_eq!(
                s.text_styles[&TextStyle::Monospace].family,
                FontFamily::Monospace
            );
        }
    }

    #[test]
    fn install_sets_both_styles_and_the_choice() {
        let ctx = egui::Context::default();
        let frame = |system: Option<egui::Theme>| egui::RawInput {
            system_theme: system,
            ..Default::default()
        };
        install(&ctx, ThemeChoice::Dark);
        ctx.run_ui(frame(None), |_| {})
            .drop_without_applying_deltas();
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        assert_eq!(ctx.global_style().visuals.panel_fill, DARK.bg_panel);
        set_choice(&ctx, ThemeChoice::Light);
        ctx.run_ui(frame(None), |_| {})
            .drop_without_applying_deltas();
        assert_eq!(ctx.global_style().visuals.panel_fill, LIGHT.bg_panel);
        // System follows what the window reports.
        set_choice(&ctx, ThemeChoice::System);
        ctx.run_ui(frame(Some(egui::Theme::Dark)), |_| {})
            .drop_without_applying_deltas();
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        ctx.run_ui(frame(Some(egui::Theme::Light)), |_| {})
            .drop_without_applying_deltas();
        assert_eq!(ctx.theme(), egui::Theme::Light);
    }

    #[test]
    fn text_icons_and_values_have_their_fonts() {
        let fonts = super::super::fonts::definitions();
        assert_eq!(
            fonts.families[&FontFamily::Proportional][..2],
            ["inter", "phosphor"]
        );
        assert_eq!(fonts.families[&FontFamily::Monospace][0], "jetbrains-mono");
        let ctx = egui::Context::default();
        install(&ctx, ThemeChoice::Dark);
        ctx.run_ui(egui::RawInput::default(), |_| {})
            .drop_without_applying_deltas();
        let body = FontId::proportional(13.0);
        ctx.fonts_mut(|f| {
            assert!(f.has_glyph(
                &body,
                egui_phosphor::regular::CURSOR.chars().next().unwrap()
            ));
            assert!(
                f.has_glyph(&body, '⌘'),
                "the macOS key glyphs come with Inter"
            );
        });
    }
}
