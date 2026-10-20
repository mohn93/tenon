//! What egui draws over the 3D view: the labels of the model's dimensions and notes; for the
//! tools, the preview lines, the snap marker with its halo, the snap chip by the cursor and the
//! flash of a refused value; the view's cursor; and the orientation gizmo in the view's top
//! right.

use std::f32::consts::SQRT_2;
use std::sync::Arc;

use egui::{
    Color32, CornerRadius, CursorIcon, FontId, Galley, Painter, Pos2, Rect, Stroke, StrokeKind, Ui,
};
use egui::{Vec2, vec2};
use glam::{DVec2, DVec3};
use session::{Camera, Projection, StandardView};
use tools::overlay::{HALO_PT, MARKER_PT, Rgba, Shape};
use tools::{Label, ToolKind};

use super::{UiAction, fonts, theme};
use crate::feedback::{Hud, HudLabel, LabelAnchor, LabelLook};

/// Space around a label's text, in points.
const LABEL_PADDING: Vec2 = Vec2::new(4.0, 2.0);

/// Where `hud` keeps the labels it laid out, in egui's frame data (see `laid_out_labels`).
fn labels_id() -> egui::Id {
    egui::Id::new("annotation labels")
}

/// The labels the last `hud` laid out: each with its rectangle in view pixels, in drawing
/// order (the last on top). Empty before the first.
pub fn laid_out_labels(ctx: &egui::Context) -> Vec<Label> {
    ctx.data(|d| d.get_temp(labels_id())).unwrap_or_default()
}

/// The stroke of an X or + marker, and of a square's outline, in points.
const CROSS_STROKE: f32 = 2.5;
const SQUARE_STROKE: f32 = 2.0;

/// Paints `hud` over the 3D view, whose rectangle is `view_rect` (points); the HUD's positions
/// are view pixels. The labels go first (keeping their rectangles for `laid_out_labels`), then
/// the lines, then the marker (its halo, then its shape), then the chip and the flash. The
/// chip is left out while the pointer is over a window, a menu or the gizmo; there, and with
/// no cursor in the view, the flash shows centred above the status bar.
pub fn hud(ui: &Ui, view_rect: Rect, hud: &Hud) {
    let ctx = ui.ctx();
    let ppp = ctx.pixels_per_point();
    let at = |p: DVec2| view_rect.min + vec2(p.x as f32 / ppp, p.y as f32 / ppp);
    let painter = ui.painter_at(view_rect);
    let to_view = |p: Pos2| {
        let v = (p - view_rect.min) * ppp;
        DVec2::new(f64::from(v.x), f64::from(v.y))
    };
    let laid_out: Vec<Label> = hud
        .labels
        .iter()
        .map(|l| {
            let rect = label(&painter, at(l.at), l);
            Label {
                owner: l.owner,
                id: l.id,
                min: to_view(rect.min),
                max: to_view(rect.max),
            }
        })
        .collect();
    ctx.data_mut(|d| d.insert_temp(labels_id(), laid_out));
    for line in &hud.lines {
        painter.line_segment(
            [at(line.a), at(line.b)],
            Stroke::new(line.width, color(line.color)),
        );
    }
    if let Some((p, shape, c)) = hud.marker {
        let halo = if hud.dark_canvas {
            Color32::BLACK
        } else {
            Color32::WHITE
        };
        marker(&painter, at(p), shape, halo, HALO_PT);
        marker(&painter, at(p), shape, color(c), 0.0);
    }

    let t = theme::tokens(ctx.theme());
    // Under a window the chip would be hidden and the flash lost: as with no cursor in the
    // view, the flash goes above the status bar.
    let cursor = hud.cursor.map(at).filter(|_| !ctx.is_pointer_over_egui());
    let chip = hud.chip.as_ref().filter(|_| cursor.is_some()).map(|chip| {
        let label = chip.label.as_ref().map(|s| {
            painter.layout_no_wrap(
                s.clone(),
                FontId::proportional(theme::SMALL_SIZE),
                t.text_primary,
            )
        });
        let value_color = if chip.typing {
            t.accent
        } else {
            t.text_primary
        };
        let value = chip.value.as_ref().map(|s| {
            painter.layout_no_wrap(s.clone(), FontId::monospace(theme::BODY_SIZE), value_color)
        });
        Card::new([label, value].into_iter().flatten().collect())
    });
    let flash = hud.flash.as_ref().map(|message| {
        Card::new(vec![painter.layout_no_wrap(
            message.clone(),
            FontId::proportional(theme::SMALL_SIZE),
            t.on_accent,
        )])
    });

    let chip_fill = (t.bg_raised, Stroke::new(1.0, t.border));
    let flash_fill = (t.danger, Stroke::NONE);
    match cursor {
        Some(cursor) => {
            // The chip, and the flash under it, kept inside the view together.
            let gap = if chip.is_some() && flash.is_some() {
                theme::CHIP_GAP
            } else {
                0.0
            };
            let chip_size = chip.as_ref().map_or(Vec2::ZERO, |c| c.size);
            let flash_size = flash.as_ref().map_or(Vec2::ZERO, |c| c.size);
            let size = vec2(
                chip_size.x.max(flash_size.x),
                chip_size.y + gap + flash_size.y,
            );
            if size == Vec2::ZERO {
                return;
            }
            let stack = chip_rect(cursor, size, view_rect);
            if let Some(chip) = chip {
                chip.paint(&painter, stack.min, chip_fill);
            }
            if let Some(flash) = flash {
                flash.paint(
                    &painter,
                    stack.min + vec2(0.0, chip_size.y + gap),
                    flash_fill,
                );
            }
        }
        None => {
            if let Some(flash) = flash {
                let min = Pos2::new(
                    view_rect.center().x - flash.size.x / 2.0,
                    view_rect.max.y - theme::CHIP_GAP - flash.size.y,
                );
                flash.paint(&painter, min, flash_fill);
            }
        }
    }
}

/// The pointer over the 3D view with `tool` active: `Grabbing` while `navigating` (an orbit or
/// pan drag), a crosshair for the drawing tools, Tape and the Paste tool (while `pasting`,
/// whatever `tool` it goes back to), move arrows for Move, a vertical resize for Push/Pull, and
/// for Paint and Erase a pointing hand while `hovering` a target.
pub fn cursor_icon(tool: ToolKind, pasting: bool, hovering: bool, navigating: bool) -> CursorIcon {
    if navigating {
        return CursorIcon::Grabbing;
    }
    if pasting {
        return CursorIcon::Crosshair;
    }
    match tool {
        ToolKind::Select => CursorIcon::Default,
        ToolKind::Line
        | ToolKind::Rectangle
        | ToolKind::Circle
        | ToolKind::Polygon
        | ToolKind::Arc
        | ToolKind::Rotate
        | ToolKind::Scale
        | ToolKind::Offset
        | ToolKind::FollowMe
        | ToolKind::TapeMeasure
        | ToolKind::Dimension
        | ToolKind::Text => CursorIcon::Crosshair,
        ToolKind::Move => CursorIcon::Move,
        ToolKind::PushPull => CursorIcon::ResizeVertical,
        ToolKind::Paint | ToolKind::Erase if hovering => CursorIcon::PointingHand,
        ToolKind::Paint | ToolKind::Erase => CursorIcon::Default,
    }
}

/// Shows `cursor` while the pointer is over the 3D view (`view_rect`), on no window, menu or
/// gizmo, and no widget chose a cursor this frame (the dock's resize edge keeps its own).
/// Called after everything else is laid out.
pub fn set_cursor(ctx: &egui::Context, view_rect: Rect, cursor: CursorIcon) {
    let Some(pointer) = ctx.pointer_hover_pos() else {
        return;
    };
    let in_view = view_rect.contains(pointer)
        && ctx
            .layer_id_at(pointer)
            .is_none_or(|layer| layer.order == egui::Order::Background);
    if in_view && ctx.output(|o| o.cursor_icon) == CursorIcon::Default {
        ctx.set_cursor_icon(cursor);
    }
}

/// The gizmo's axis ends, each as the standard view that looks from it, its offset from the
/// gizmo's centre in points (y down), and how far it points toward the viewer
/// (`-dir.dot(forward)`): +X, -X, +Y, -Y, +Z, -Z.
pub fn gizmo_ends(camera: &Camera) -> [(StandardView, Vec2, f32); 6] {
    let (right, up, forward) = (camera.right(), camera.up(), camera.forward());
    [
        StandardView::Right,
        StandardView::Left,
        StandardView::Back,
        StandardView::Front,
        StandardView::Top,
        StandardView::Bottom,
    ]
    .map(|view| {
        let dir: DVec3 = view.direction().0;
        let offset = vec2(dir.dot(right) as f32, -dir.dot(up) as f32) * theme::GIZMO_AXIS;
        (view, offset, -dir.dot(forward) as f32)
    })
}

/// The orientation gizmo in the top right of the view (`view_rect`), in its own `Area` so that
/// no press on it reaches the tools. Clicking an axis end asks for the standard view that looks
/// from it; clicking the centre switches `projection`.
pub fn gizmo(
    ui: &Ui,
    view_rect: Rect,
    camera: &Camera,
    projection: Projection,
    actions: &mut Vec<UiAction>,
) {
    let size = Vec2::splat(theme::GIZMO);
    let min = view_rect.right_top() + vec2(-theme::GIZMO_INSET - theme::GIZMO, theme::GIZMO_INSET);
    let t = theme::tokens(ui.ctx().theme());
    egui::Area::new(egui::Id::new("gizmo"))
        .order(egui::Order::Middle)
        .movable(false)
        .constrain(false)
        .fixed_pos(min)
        .show(ui.ctx(), |ui| {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let centre = rect.center();
            let painter = ui.painter();
            let [r, g, b, _] = t.bg_panel.to_srgba_unmultiplied();
            painter.circle_filled(
                centre,
                theme::GIZMO / 2.0,
                Color32::from_rgba_unmultiplied(r, g, b, 160),
            );

            let mut ends = gizmo_ends(camera);
            ends.sort_by(|a, b| a.2.total_cmp(&b.2));
            for (view, offset, _) in ends {
                if offset.length() < theme::GIZMO_HIDE {
                    continue;
                }
                let (color, letter) = match view {
                    StandardView::Right | StandardView::Left => (theme::AXIS_X, "X"),
                    StandardView::Back | StandardView::Front => (theme::AXIS_Y, "Y"),
                    _ => (theme::AXIS_Z, "Z"),
                };
                let positive = matches!(
                    view,
                    StandardView::Right | StandardView::Back | StandardView::Top
                );
                let end = centre + offset;
                let radius = if positive {
                    painter.line_segment([centre, end], Stroke::new(3.0, color));
                    painter.circle_filled(end, theme::GIZMO_CAP, color);
                    painter.text(
                        end,
                        egui::Align2::CENTER_CENTER,
                        letter,
                        FontId::new(theme::SMALL_SIZE, fonts::semibold()),
                        t.on_accent,
                    );
                    theme::GIZMO_CAP
                } else {
                    painter.circle_stroke(end, theme::GIZMO_NEG_CAP, Stroke::new(1.5, color));
                    theme::GIZMO_NEG_CAP
                };
                let name = view_name(view);
                let hit = Rect::from_center_size(end, Vec2::splat(2.0 * radius));
                if gizmo_button(ui, hit, name, false).clicked() {
                    actions.push(UiAction::StandardView(view));
                }
            }

            painter.circle(
                centre,
                theme::GIZMO_CENTRE,
                t.bg_raised,
                Stroke::new(1.0, t.border),
            );
            let parallel = projection == Projection::Parallel;
            let hit = Rect::from_center_size(centre, Vec2::splat(2.0 * theme::GIZMO_CENTRE));
            if gizmo_button(ui, hit, "Switch Projection", parallel).clicked() {
                actions.push(UiAction::SetProjection(if parallel {
                    Projection::Perspective
                } else {
                    Projection::Parallel
                }));
            }
        });
}

/// A clickable part of the gizmo over `rect`, named `name` for AccessKit (`selected`: toggled
/// on), with the pointing hand while hovered.
fn gizmo_button(ui: &Ui, rect: Rect, name: &str, selected: bool) -> egui::Response {
    let response = ui
        .interact(rect, ui.id().with(name), egui::Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, name));
    response
}

/// The name of the gizmo end that looks from `view`.
fn view_name(view: StandardView) -> &'static str {
    match view {
        StandardView::Right => "Right View",
        StandardView::Left => "Left View",
        StandardView::Back => "Back View",
        StandardView::Front => "Front View",
        StandardView::Top => "Top View",
        StandardView::Bottom => "Bottom View",
        StandardView::Iso => unreachable!("no axis end looks from Iso"),
    }
}

/// Where the chip of `size` goes for a cursor at `cursor` (points): (14, 18) past it, flipped
/// to stay inside `view_rect`.
pub fn chip_rect(cursor: Pos2, size: Vec2, view_rect: Rect) -> Rect {
    let offset = theme::CHIP_OFFSET;
    let mut min = cursor + offset;
    if min.x + size.x > view_rect.max.x {
        min.x = cursor.x - offset.x - size.x;
    }
    if min.y + size.y > view_rect.max.y {
        min.y = cursor.y - offset.y - size.y;
    }
    // Neither side has room (a view smaller than the offset and the chip): pushed in.
    min.x = min.x.min(view_rect.max.x - size.x).max(view_rect.min.x);
    min.y = min.y.min(view_rect.max.y - size.y).max(view_rect.min.y);
    Rect::from_min_size(min, size)
}

/// Paints a dimension's or note's label at `at` (points; its center, or a screen note's top
/// left corner): its text on a small card in the theme's colors, the accent's when selected or
/// hovered, the danger color's when Erase hovers it. Returns the card's rectangle.
fn label(painter: &Painter, at: Pos2, l: &HudLabel) -> Rect {
    let t = theme::tokens(painter.ctx().theme());
    let (text, stroke) = match l.look {
        LabelLook::Normal => (t.text_primary, Stroke::new(1.0, t.border)),
        LabelLook::Faded => (t.text_disabled, Stroke::new(1.0, t.border)),
        LabelLook::Selected => (t.accent, Stroke::new(1.5, t.accent)),
        LabelLook::Hovered => (t.text_primary, Stroke::new(1.5, t.accent)),
        LabelLook::Erasing => (t.danger, Stroke::new(1.5, t.danger)),
    };
    let galley = painter.layout_no_wrap(
        l.text.clone(),
        FontId::proportional(theme::SMALL_SIZE),
        text,
    );
    let size = galley.size() + 2.0 * LABEL_PADDING;
    let rect = match l.anchor {
        LabelAnchor::Center => Rect::from_center_size(at, size),
        LabelAnchor::TopLeft => Rect::from_min_size(at, size),
    };
    painter.rect(
        rect,
        CornerRadius::same(theme::CHIP_RADIUS),
        t.bg_raised,
        stroke,
        StrokeKind::Inside,
    );
    painter.galley(rect.min + LABEL_PADDING, galley, Color32::PLACEHOLDER);
    rect
}

/// Lines of text in a padded, rounded box.
struct Card {
    lines: Vec<Arc<Galley>>,
    size: Vec2,
}

impl Card {
    fn new(lines: Vec<Arc<Galley>>) -> Self {
        let text = lines.iter().fold(Vec2::ZERO, |s, g| {
            vec2(s.x.max(g.size().x), s.y + g.size().y)
        });
        Self {
            lines,
            size: text + 2.0 * theme::CHIP_PADDING,
        }
    }

    fn paint(self, painter: &Painter, min: Pos2, (fill, stroke): (Color32, Stroke)) {
        let rect = Rect::from_min_size(min, self.size);
        painter.rect(
            rect,
            CornerRadius::same(theme::CHIP_RADIUS),
            fill,
            stroke,
            StrokeKind::Inside,
        );
        let mut pos = min + theme::CHIP_PADDING;
        for galley in self.lines {
            let height = galley.size().y;
            painter.galley(pos, galley, Color32::PLACEHOLDER);
            pos.y += height;
        }
    }
}

/// A marker of `shape` centred at `center`, grown by `grow` on every side (a halo).
fn marker(painter: &Painter, center: Pos2, shape: Shape, color: Color32, grow: f32) {
    let half = MARKER_PT / 2.0;
    // Two strokes through the centre, from -`end` to `end` and turned a quarter, each `grow`
    // longer at both ends.
    let cross = |end: Vec2| {
        let stroke = Stroke::new(CROSS_STROKE + 2.0 * grow, color);
        let end = end + end.normalized() * grow;
        for d in [end, end.rot90()] {
            painter.line_segment([center - d, center + d], stroke);
        }
    };
    match shape {
        Shape::Dot => {
            painter.circle_filled(center, half + grow, color);
        }
        Shape::Square => {
            painter.rect_stroke(
                Rect::from_center_size(center, Vec2::splat(MARKER_PT)),
                CornerRadius::ZERO,
                Stroke::new(SQUARE_STROKE + 2.0 * grow, color),
                StrokeKind::Middle,
            );
        }
        Shape::Diamond => {
            // Each side moves out by `grow`, so each corner by `grow` times the square root of 2.
            let r = half + grow * SQRT_2;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    center + vec2(0.0, -r),
                    center + vec2(r, 0.0),
                    center + vec2(0.0, r),
                    center + vec2(-r, 0.0),
                ],
                color,
                Stroke::NONE,
            ));
        }
        // An X whose arms reach the corners of a square `MARKER_PT` across.
        Shape::Cross => cross(Vec2::splat(half)),
        Shape::Plus => cross(vec2(half, 0.0)),
    }
}

/// An `Rgba` as egui's color.
fn color([r, g, b, a]: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, a)
}
