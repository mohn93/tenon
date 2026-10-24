//! Pages (saved scenes): a named, ordered list of stored views and tag visibilities, each
//! gone back to in one click. A page may store a view, tag visibility, both or neither.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::edit::KernelError;
use crate::ids::{PageId, StyleId, TagId};
use crate::scene::{Scene, SceneChange, Step};

/// The longest page name, in characters.
pub const MAX_PAGE_NAME: usize = 256;

/// The name new pages take, followed by the lowest number no page has ("Scene 1").
pub const PAGE_NAME_BASE: &str = "Scene";

/// A stored view: the camera, its projection and how the view looks.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageView {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    /// Which way is up the screen.
    pub up: [f64; 3],
    /// The vertical field of view, in radians (1 to 170 degrees). Radians, as the camera holds
    /// it, so storing and going back to a view changes no bit of it.
    pub fov_y: f64,
    /// Parallel (orthographic) projection rather than perspective.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub parallel: bool,
    /// Lit by night rather than by day.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub night: bool,
    /// Lit by the studio lights (key, fill and rim); wins over `night` if both are set.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub studio: bool,
    /// Faces drawn in their tags' colors.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub color_by_tag: bool,
    /// Every shown face drawn as glass.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub x_ray: bool,
    /// Entities hidden by their own flag drawn ghosted and open to picking.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden_geometry: bool,
    /// Sun shadows (see [`Shadows`]); written only when not the default.
    #[serde(default, skip_serializing_if = "Shadows::is_default")]
    pub shadows: Shadows,
    /// The style the view draws with (see [`crate::Style`]); written only when one is chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StyleId>,
}

/// The sun shadow settings of a view: whether the sun lights the model and casts shadows, and
/// when. The date is a calendar date and `minutes` the local clock time (minutes since
/// midnight) at the model's UTC offset. `light` and `dark` are percentages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Shadows {
    pub on: bool,
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub minutes: u16,
    pub light: u8,
    pub dark: u8,
    pub on_ground: bool,
}

impl Default for Shadows {
    fn default() -> Self {
        Shadows {
            on: false,
            year: 2026,
            month: 11,
            day: 8,
            minutes: 13 * 60 + 30,
            light: 80,
            dark: 45,
            on_ground: true,
        }
    }
}

/// The years a shadow date may have.
pub const SHADOW_YEARS: std::ops::RangeInclusive<u16> = 1800..=2200;

impl Shadows {
    pub fn is_default(&self) -> bool {
        *self == Shadows::default()
    }

    /// Whether `year` is a leap year of the Gregorian calendar.
    pub fn is_leap_year(year: u16) -> bool {
        (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
    }

    /// The days in `month` (1 to 12) of `year`; 0 for any other month.
    pub fn days_in_month(year: u16, month: u8) -> u8 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if Shadows::is_leap_year(year) => 29,
            2 => 28,
            _ => 0,
        }
    }

    /// Fails, saying why, when the date is not a real date in [`SHADOW_YEARS`], the time is
    /// not within a day or a level is over 100.
    pub fn check(&self) -> Result<(), String> {
        if !SHADOW_YEARS.contains(&self.year) {
            return Err(format!(
                "the shadow date's year must be from {} to {}",
                SHADOW_YEARS.start(),
                SHADOW_YEARS.end()
            ));
        }
        if self.day == 0 || self.day > Shadows::days_in_month(self.year, self.month) {
            return Err(format!(
                "the shadow date {:04}-{:02}-{:02} is not a date",
                self.year, self.month, self.day
            ));
        }
        if self.minutes >= 24 * 60 {
            return Err("the shadow time must be within a day (00:00 to 23:59)".into());
        }
        if self.light > 100 || self.dark > 100 {
            return Err("light and dark must be from 0 to 100".into());
        }
        Ok(())
    }
}

/// A saved scene. `view: None` leaves the camera and view settings alone when the page is gone
/// to; `tags: None` leaves tag visibility alone. `tags` holds the tags hidden on the page
/// (every other tag is shown).
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub name: String,
    pub view: Option<PageView>,
    pub tags: Option<BTreeSet<TagId>>,
}

/// Fails, saying why, when a page name is blank or longer than [`MAX_PAGE_NAME`] characters.
pub(crate) fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("a scene needs a name".into());
    }
    if name.chars().count() > MAX_PAGE_NAME {
        return Err(format!(
            "a scene name may have at most {MAX_PAGE_NAME} characters"
        ));
    }
    Ok(())
}

/// Fails, saying why, when a view has a number that is not finite, a field of view outside 1
/// to 170 degrees (checked in degrees), its eye on its target or no up direction.
pub(crate) fn check_view(v: &PageView) -> Result<(), String> {
    let numbers = v.eye.iter().chain(&v.target).chain(&v.up).chain([&v.fov_y]);
    if !numbers.into_iter().all(|x| x.is_finite()) {
        return Err("a scene's view has a number that is not finite".into());
    }
    if !(1.0..=170.0).contains(&v.fov_y.to_degrees()) {
        return Err("a scene's field of view must be from 1 to 170 degrees".into());
    }
    let apart = v
        .eye
        .iter()
        .zip(&v.target)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt();
    if apart < 1e-6 {
        return Err("a scene's eye and target must be distinct points".into());
    }
    if v.up.iter().all(|x| *x == 0.0) {
        return Err("a scene's up direction must not be zero".into());
    }
    v.shadows.check()?;
    Ok(())
}

impl Scene {
    /// Adds `page` after the last one, after checking its name (valid, see [`check_name`], and
    /// not taken), its view and its tags.
    pub fn add_page(&mut self, page: Page) -> Result<(PageId, SceneChange), KernelError> {
        self.check_page(&page, None)?;
        let id = self.alloc_page();
        let step = Step::AddPage(id, self.pages().count(), page);
        self.apply_step(&step);
        Ok((id, step.into()))
    }

    /// Replaces a page's name, view and tags with `page`'s, checked as `add_page` does. One
    /// step; nothing to change changes nothing.
    pub fn change_page(&mut self, id: PageId, page: Page) -> Result<SceneChange, KernelError> {
        let from = self.page(id).ok_or(KernelError::PageNotFound)?.clone();
        self.check_page(&page, Some(id))?;
        if page == from {
            return Ok(SceneChange::default());
        }
        let step = Step::ChangePage(id, from, page);
        self.apply_step(&step);
        Ok(step.into())
    }

    /// Moves a page to `index` in the order (0: first). An index past the last page is
    /// refused; the page's own index changes nothing.
    pub fn move_page(&mut self, id: PageId, index: usize) -> Result<SceneChange, KernelError> {
        let from = self.page_index(id).ok_or(KernelError::PageNotFound)?;
        let count = self.pages().count();
        if index >= count {
            return Err(KernelError::Page(format!(
                "index {index} is out of range: there are {count} scenes"
            )));
        }
        if index == from {
            return Ok(SceneChange::default());
        }
        let step = Step::MovePage(id, from, index);
        self.apply_step(&step);
        Ok(step.into())
    }

    /// Removes a page; undo puts it back where it was.
    pub fn delete_page(&mut self, id: PageId) -> Result<SceneChange, KernelError> {
        let index = self.page_index(id).ok_or(KernelError::PageNotFound)?;
        let page = self.pages[index].1.clone();
        let step = Step::RemovePage(id, index, page);
        self.apply_step(&step);
        Ok(step.into())
    }

    /// "Scene" followed by the lowest number from 1 no page's name has ("Scene 1").
    pub fn free_page_name(&self) -> String {
        let taken: BTreeSet<&str> = self.pages().map(|(_, p)| p.name.as_str()).collect();
        (1..)
            .map(|n| format!("{PAGE_NAME_BASE} {n}"))
            .find(|n| !taken.contains(n.as_str()))
            .expect("a free name")
    }

    /// The steps that drop `tag` from every page hiding it, applied (for deleting the tag).
    pub(crate) fn unhide_on_pages(&mut self, tag: TagId) -> SceneChange {
        let hiding: Vec<(PageId, Page)> = self
            .pages()
            .filter(|(_, p)| p.tags.as_ref().is_some_and(|t| t.contains(&tag)))
            .map(|(id, p)| (id, p.clone()))
            .collect();
        let mut steps = Vec::new();
        for (id, from) in hiding {
            let mut to = from.clone();
            if let Some(t) = &mut to.tags {
                t.remove(&tag);
            }
            let step = Step::ChangePage(id, from, to);
            self.apply_step(&step);
            steps.push(step);
        }
        SceneChange { steps }
    }

    fn check_page(&self, page: &Page, except: Option<PageId>) -> Result<(), KernelError> {
        check_name(&page.name).map_err(KernelError::Page)?;
        let taken = self
            .pages()
            .any(|(id, p)| Some(id) != except && p.name == page.name);
        if taken {
            return Err(KernelError::Page(format!(
                "a scene named {:?} already exists",
                page.name
            )));
        }
        if let Some(v) = &page.view {
            check_view(v).map_err(KernelError::Page)?;
        }
        if page.tags.iter().flatten().any(|t| self.tag(*t).is_none()) {
            return Err(KernelError::TagNotFound);
        }
        if page
            .view
            .and_then(|v| v.style)
            .is_some_and(|s| self.style(s).is_none())
        {
            return Err(KernelError::StyleNotFound);
        }
        Ok(())
    }
}
