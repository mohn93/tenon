//! How a renderer draws what the window's theme decides. `ViewStyle::CLASSIC` is the default
//! and what `render_view`, headless renders and the golden images use.

use crate::grid::BACKGROUND;
use crate::scene::SELECTED_EDGE_COLOR;

/// The canvas, the line fading and the selection's look. Colors are sRGB-authored like
/// `grid::BACKGROUND`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewStyle {
    /// The day background: the view's top and bottom, a vertical gradient between them.
    pub day_canvas: [[f32; 4]; 2],
    /// How far the grid and axes fade toward the canvas by day (0 to 1). Night has its own.
    pub day_ground_fade: f32,
    /// Selected edges and guides.
    pub selected_edge: [f32; 4],
    /// The color selected faces are tinted toward, and how far (0 to 1).
    pub selected_face: [f32; 3],
    pub face_tint: f32,
    /// Selected edges drawn about 2 px wide rather than 1.
    pub wide_selected_edges: bool,
}

impl ViewStyle {
    /// The look before themes: a flat light background, a blue selection at half strength.
    pub const CLASSIC: Self = Self {
        day_canvas: [BACKGROUND, BACKGROUND],
        day_ground_fade: 0.0,
        selected_edge: SELECTED_EDGE_COLOR,
        selected_face: [0.25, 0.45, 0.95],
        face_tint: 0.5,
        wide_selected_edges: false,
    };

    /// The day background's middle: the clear color, and what lines fade toward by day.
    pub fn day_background(&self) -> [f32; 4] {
        let [a, b] = self.day_canvas;
        [0, 1, 2, 3].map(|k| (a[k] + b[k]) / 2.0)
    }

    /// Whether the day canvas is drawn as a gradient rather than cleared to one color.
    pub fn has_gradient(&self) -> bool {
        self.day_canvas[0] != self.day_canvas[1]
    }
}

impl Default for ViewStyle {
    fn default() -> Self {
        Self::CLASSIC
    }
}
