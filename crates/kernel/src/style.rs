//! Styles: named sets of edge, background, sky, ground and fog settings a view may choose. A
//! style is part of the model (undoable like tags and pages); which one a view draws with is
//! part of the view (`PageView::style`, the camera and every scene).

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::edit::KernelError;
use crate::ids::StyleId;
use crate::page::PageView;
use crate::scene::{Scene, SceneChange, Step};

/// The longest style name, in characters (after trimming).
pub const MAX_STYLE_NAME: usize = 100;

/// The name new styles take, followed by the lowest number no style has ("Style 1").
pub const STYLE_NAME_BASE: &str = "Style";

/// Profile edge widths, in logical pixels.
pub const PROFILE_WIDTH: std::ops::RangeInclusive<f32> = 2.0..=10.0;
/// Sketchy edge jitter, in logical pixels.
pub const SKETCHY_JITTER: std::ops::RangeInclusive<f32> = 0.0..=10.0;
/// Sketchy edge extension, in logical pixels.
pub const SKETCHY_EXTENSION: std::ops::RangeInclusive<f32> = 0.0..=30.0;
/// Endpoint mark lengths, in logical pixels.
pub const ENDPOINT_LENGTH: std::ops::RangeInclusive<f32> = 2.0..=30.0;

/// A named style.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub name: String,
    pub settings: StyleSettings,
}

/// How ordinary edges take their color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EdgeColorMode {
    /// All in [`EdgeStyle::color`].
    #[default]
    Fixed,
    /// The front material color of the first shown face on the edge (the fixed color without
    /// one).
    ByMaterial,
    /// The axis colors for edges along a drawing axis (the fixed color for the others).
    ByAxis,
}

/// Sketchy edges: a per-edge wobble and overshoot, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sketchy {
    /// The most an edge is moved sideways, 0 to 10.
    pub jitter: f32,
    /// The most each end of an edge is lengthened, 0 to 30.
    pub extension: f32,
}

/// How edges are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct EdgeStyle {
    pub color_mode: EdgeColorMode,
    /// sRGB red, green and blue of ordinary edges (and the fallback of the other modes).
    pub color: [u8; 3],
    /// Draw ordinary edges; false leaves only the profiles (when on).
    pub show_edges: bool,
    /// Profile (silhouette) edges drawn this wide, 2 to 10 pixels; null: off.
    pub profiles: Option<f32>,
    /// Sketchy edges; null: off.
    pub sketchy: Option<Sketchy>,
    /// Marks this long at every edge end, 2 to 30 pixels; null: off.
    pub endpoints: Option<f32>,
}

impl Default for EdgeStyle {
    fn default() -> Self {
        EdgeStyle {
            color_mode: EdgeColorMode::Fixed,
            color: [20, 20, 20],
            show_edges: true,
            profiles: None,
            sketchy: None,
            endpoints: None,
        }
    }
}

/// The ground plane's tint.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ground {
    pub color: [u8; 3],
    /// 0 to 1.
    pub opacity: f32,
    /// Also drawn when the eye is below the ground.
    pub from_below: bool,
}

/// What fog fades toward.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FogColor {
    /// The style's background.
    Background,
    Rgb([u8; 3]),
}

/// Fog: things fade toward its color between `start` and `end` meters from the eye.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fog {
    /// Meters, from 0.
    pub start: f32,
    /// Meters, over `start`.
    pub end: f32,
    pub color: FogColor,
}

/// What a style changes about how a view looks. The default is today's look.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct StyleSettings {
    pub edges: EdgeStyle,
    pub background: [u8; 3],
    /// The sky's zenith color (the horizon takes the background); null: no sky.
    pub sky: Option<[u8; 3]>,
    pub ground: Option<Ground>,
    pub fog: Option<Fog>,
}

impl Default for StyleSettings {
    fn default() -> Self {
        StyleSettings {
            edges: EdgeStyle::default(),
            background: [245, 245, 245],
            sky: None,
            ground: None,
            fog: None,
        }
    }
}

fn check_range(what: &str, x: f32, range: &std::ops::RangeInclusive<f32>) -> Result<(), String> {
    if x.is_finite() && range.contains(&x) {
        Ok(())
    } else {
        Err(format!(
            "{what} must be from {} to {}",
            range.start(),
            range.end()
        ))
    }
}

impl StyleSettings {
    /// Fails, saying why, when a width, length, opacity or distance is out of range or not
    /// finite, or the fog does not start before it ends.
    pub fn check(&self) -> Result<(), String> {
        let e = &self.edges;
        if let Some(w) = e.profiles {
            check_range("the profile width", w, &PROFILE_WIDTH)?;
        }
        if let Some(s) = e.sketchy {
            check_range("the sketchy jitter", s.jitter, &SKETCHY_JITTER)?;
            check_range("the sketchy extension", s.extension, &SKETCHY_EXTENSION)?;
        }
        if let Some(l) = e.endpoints {
            check_range("the endpoint length", l, &ENDPOINT_LENGTH)?;
        }
        if let Some(g) = &self.ground {
            check_range("the ground opacity", g.opacity, &(0.0..=1.0))?;
        }
        if let Some(f) = &self.fog
            && (!(f.start.is_finite() && f.end.is_finite()) || f.start < 0.0 || f.start >= f.end)
        {
            return Err("the fog needs 0 <= start < end (finite meters)".into());
        }
        Ok(())
    }
}

/// Fails, saying why, when a style name is blank (after trimming) or longer than
/// [`MAX_STYLE_NAME`] characters.
pub(crate) fn check_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a style needs a name".into());
    }
    if name.chars().count() > MAX_STYLE_NAME {
        return Err(format!(
            "a style name may have at most {MAX_STYLE_NAME} characters"
        ));
    }
    Ok(())
}

impl Scene {
    pub fn style(&self, id: StyleId) -> Option<&Style> {
        self.styles.get(&id)
    }

    /// Every style, in id (creation) order.
    pub fn styles(&self) -> impl Iterator<Item = (StyleId, &Style)> {
        self.styles.iter().map(|(k, v)| (*k, v))
    }

    /// Adds `style` after checking its name (valid, see [`check_name`], trimmed, and not
    /// taken) and settings.
    pub fn add_style(&mut self, style: Style) -> Result<(StyleId, SceneChange), KernelError> {
        let style = self.checked_style(style, None)?;
        let id = self.alloc_style();
        let step = Step::AddStyle(id, style);
        self.apply_step(&step);
        Ok((id, step.into()))
    }

    /// Replaces a style's name and settings, checked as `add_style` does. One step; nothing
    /// to change changes nothing.
    pub fn change_style(&mut self, id: StyleId, to: Style) -> Result<SceneChange, KernelError> {
        let from = self.style(id).ok_or(KernelError::StyleNotFound)?.clone();
        let to = self.checked_style(to, Some(id))?;
        if to == from {
            return Ok(SceneChange::default());
        }
        let step = Step::ChangeStyle(id, from, to);
        self.apply_step(&step);
        Ok(step.into())
    }

    /// Removes a style. Every scene whose view chose it goes back to no style in the same
    /// change, so undo puts it all back.
    pub fn delete_style(&mut self, id: StyleId) -> Result<SceneChange, KernelError> {
        let style = self.style(id).ok_or(KernelError::StyleNotFound)?.clone();
        let mut changes = vec![self.unchoose_on_pages(id)];
        let step = Step::RemoveStyle(id, style);
        self.apply_step(&step);
        changes.push(step.into());
        Ok(SceneChange::join(changes))
    }

    /// "Style" followed by the lowest number from 1 no style's name has ("Style 1").
    pub fn free_style_name(&self) -> String {
        let taken: BTreeSet<&str> = self.styles().map(|(_, s)| s.name.as_str()).collect();
        (1..)
            .map(|n| format!("{STYLE_NAME_BASE} {n}"))
            .find(|n| !taken.contains(n.as_str()))
            .expect("a free name")
    }

    /// The steps that drop `style` from every scene view choosing it, applied (for deleting
    /// the style).
    fn unchoose_on_pages(&mut self, style: StyleId) -> SceneChange {
        let choosing: Vec<_> = self
            .pages()
            .filter(|(_, p)| p.view.is_some_and(|v| v.style == Some(style)))
            .map(|(id, p)| (id, p.clone()))
            .collect();
        let mut steps = Vec::new();
        for (id, from) in choosing {
            let mut to = from.clone();
            if let Some(v) = &mut to.view {
                *v = PageView { style: None, ..*v };
            }
            let step = Step::ChangePage(id, from, to);
            self.apply_step(&step);
            steps.push(step);
        }
        SceneChange { steps }
    }

    /// `style` with its name trimmed, after checking its name, that no other style has it,
    /// and its settings.
    fn checked_style(
        &self,
        mut style: Style,
        except: Option<StyleId>,
    ) -> Result<Style, KernelError> {
        check_name(&style.name).map_err(KernelError::Style)?;
        style.name = style.name.trim().to_string();
        let taken = self
            .styles()
            .any(|(id, s)| Some(id) != except && s.name == style.name);
        if taken {
            return Err(KernelError::Style(format!(
                "a style named {:?} already exists",
                style.name
            )));
        }
        style.settings.check().map_err(KernelError::Style)?;
        Ok(style)
    }
}
