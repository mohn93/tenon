//! Drag fields shared by the panels.

use egui::Ui;

/// What a `drag_row` frame gave.
pub(super) struct DragRow<const N: usize> {
    /// When a drag ended or a typed value was entered: the values shown when the edit began,
    /// and the values now.
    pub(super) done: Option<([f64; N], [f64; N])>,
    /// Whether a field is being dragged or typed in.
    pub(super) editing: bool,
}

/// A row of `label` and one field per value, changing by `speed` per point dragged; with more
/// than one field the label goes on a line of its own, so long values still fit the panel. While a
/// field is dragged or typed in, the values shown when the edit began, and those being dragged,
/// live in egui's memory under `key`, so a change of what is shown meanwhile does not leak in.
pub(super) fn drag_row<const N: usize>(
    ui: &mut Ui,
    key: egui::Id,
    label: &str,
    shown: [f64; N],
    range: std::ops::RangeInclusive<f64>,
    speed: f64,
) -> DragRow<N> {
    let stored = ui.data(|d| d.get_temp::<([f64; N], [f64; N])>(key));
    let (start, mut values) = stored.unwrap_or((shown, shown));
    if N > 1 {
        ui.label(label);
    }
    let (editing, ended) = ui
        .horizontal(|ui| {
            if N == 1 {
                ui.label(label);
            }
            let (mut editing, mut ended) = (false, false);
            for value in &mut values {
                let field = ui.add(
                    egui::DragValue::new(value)
                        .range(range.clone())
                        .speed(speed)
                        .max_decimals(3)
                        .update_while_editing(false),
                );
                // A field being typed in reports a change per keystroke; its value comes
                // when it loses focus.
                editing |= field.dragged() || field.has_focus();
                ended |= field.drag_stopped() || field.changed();
            }
            (editing, ended)
        })
        .inner;
    if editing {
        ui.data_mut(|d| d.insert_temp(key, (start, values)));
        return DragRow {
            done: None,
            editing: true,
        };
    }
    ui.data_mut(|d| d.remove::<([f64; N], [f64; N])>(key));
    DragRow {
        done: (ended || stored.is_some()).then_some((start, values)),
        editing: false,
    }
}

/// Takes what `drag_row` keeps under `key` while its edit is in progress: the values shown
/// when it began and the values now.
pub(super) fn take_row<const N: usize>(ui: &mut Ui, key: egui::Id) -> Option<([f64; N], [f64; N])> {
    let row = ui.data(|d| d.get_temp::<([f64; N], [f64; N])>(key));
    ui.data_mut(|d| d.remove::<([f64; N], [f64; N])>(key));
    row
}
