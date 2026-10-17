//! What the window draws over the 3D view for the tools (the snap marker, the preview lines,
//! the snap chip and the flash of a refused value), and when it changes by itself: the chip
//! waits for the cursor to rest, the flash lasts a while. Those two times are repaint deadlines,
//! so a still window draws nothing more once they pass.

use std::time::{Duration, Instant};

use glam::DVec2;
use tools::overlay::{Rgba, ScreenLine, Shape};

/// How long the cursor rests before the chip shows.
pub const LABEL_REST: Duration = Duration::from_millis(150);
/// How long a refused value's message flashes by the cursor.
pub const FLASH_TIME: Duration = Duration::from_millis(2500);

/// What the window draws over the 3D view for the tools; positions in view pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hud {
    pub marker: Option<(DVec2, Shape, Rgba)>,
    pub lines: Vec<ScreenLine>,
    pub chip: Option<Chip>,
    pub flash: Option<String>,
    /// The cursor, while it is in the view.
    pub cursor: Option<DVec2>,
    /// The canvas is dark (night lighting, or the dark theme by day): halos are black.
    pub dark_canvas: bool,
}

impl Hud {
    /// The HUD with the pointer over a window, menu or the gizmo, where a press never reaches
    /// the tools: no marker and no chip. The preview lines and the flash stay.
    pub fn covered(self) -> Hud {
        Hud {
            marker: None,
            chip: None,
            ..self
        }
    }
}

/// The chip by the cursor: the snap's name over the measurement or the typed text.
#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    pub label: Option<String>,
    pub value: Option<String>,
    /// `value` is typed text.
    pub typing: bool,
}

/// When the cursor last moved in the view, and the message flashing since when.
#[derive(Clone, Debug, Default)]
pub struct Timing {
    moved: Option<Instant>,
    flash: Option<(String, Instant)>,
}

impl Timing {
    /// The cursor moved in the view: the rest starts again.
    pub fn moved(&mut self, now: Instant) {
        self.moved = Some(now);
    }

    /// `message` starts flashing.
    pub fn flash(&mut self, message: String, now: Instant) {
        self.flash = Some((message, now));
    }

    /// `hud` as shown at `now`: the chip only once rested and not `navigating`, the flash
    /// while it lasts.
    pub fn dress(&self, hud: Hud, now: Instant, navigating: bool) -> Hud {
        let rested = self.moved.is_none_or(|t| now >= t + LABEL_REST);
        Hud {
            chip: hud.chip.filter(|_| rested && !navigating),
            flash: self
                .flash
                .as_ref()
                .filter(|(_, t)| now < *t + FLASH_TIME)
                .map(|(m, _)| m.clone()),
            ..hud
        }
    }

    /// When the HUD next changes by itself (the rest's end, the flash's end); None when
    /// nothing is pending. The rest counts only when `chip`: a chip waits for it, so the frame
    /// at its end shows something new.
    pub fn deadline(&self, now: Instant, chip: bool) -> Option<Instant> {
        let rest = self.moved.filter(|_| chip).map(|t| t + LABEL_REST);
        let flash = self.flash.as_ref().map(|(_, t)| *t + FLASH_TIME);
        [rest, flash]
            .into_iter()
            .flatten()
            .filter(|t| now < *t)
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_asks_for_one_frame_per_change_then_none() {
        let t0 = Instant::now();
        let mut t = Timing::default();
        assert_eq!(t.deadline(t0, true), None, "idle from the start");
        t.moved(t0);
        assert_eq!(t.deadline(t0, true), Some(t0 + LABEL_REST));
        let rested = t0 + LABEL_REST;
        assert_eq!(
            t.deadline(rested, true),
            None,
            "the frame at the rest is the last"
        );
        t.flash("Not a length: 2,x".into(), rested);
        assert_eq!(t.deadline(rested, true), Some(rested + FLASH_TIME));
        assert_eq!(t.deadline(rested + FLASH_TIME, true), None);
    }

    #[test]
    fn the_rest_wakes_the_window_only_for_a_chip() {
        // Select over empty space, or an orbit drag: the cursor moves, but no chip waits for
        // the rest, so the rest's end draws nothing new.
        let t0 = Instant::now();
        let mut t = Timing::default();
        t.moved(t0);
        assert_eq!(t.deadline(t0, false), None);
        // A flash still ends on time.
        t.flash("Not a length: 2,x".into(), t0);
        assert_eq!(t.deadline(t0, false), Some(t0 + FLASH_TIME));
    }

    #[test]
    fn the_chip_waits_for_the_rest_and_hides_while_navigating() {
        let t0 = Instant::now();
        let mut t = Timing::default();
        t.moved(t0);
        let hud = Hud {
            chip: Some(Chip {
                label: Some("Endpoint".into()),
                value: None,
                typing: false,
            }),
            ..Hud::default()
        };
        assert_eq!(t.dress(hud.clone(), t0, false).chip, None);
        assert!(t.dress(hud.clone(), t0 + LABEL_REST, false).chip.is_some());
        assert_eq!(t.dress(hud.clone(), t0 + LABEL_REST, true).chip, None);
        t.flash("Not a length: 2,x".into(), t0);
        assert_eq!(
            t.dress(hud.clone(), t0, false).flash.as_deref(),
            Some("Not a length: 2,x")
        );
        assert_eq!(t.dress(hud, t0 + FLASH_TIME, false).flash, None);
    }

    #[test]
    fn a_covered_hud_keeps_only_its_lines_and_flash() {
        let line = ScreenLine {
            a: DVec2::ZERO,
            b: DVec2::ONE,
            color: tools::overlay::LINE_DAY,
            width: tools::overlay::LINE_PT,
        };
        let hud = Hud {
            marker: Some((DVec2::ONE, Shape::Dot, tools::overlay::ENDPOINT)),
            lines: vec![line],
            chip: Some(Chip {
                label: Some("Endpoint".into()),
                value: None,
                typing: false,
            }),
            flash: Some("Not a length: 2,x".into()),
            cursor: Some(DVec2::ONE),
            dark_canvas: true,
        };
        let covered = hud.clone().covered();
        assert_eq!((covered.marker, covered.chip), (None, None));
        assert_eq!(covered.lines, hud.lines);
        assert_eq!(covered.flash, hud.flash);
        assert_eq!(covered.cursor, hud.cursor);
    }
}
