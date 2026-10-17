use super::*;

fn with_card(recent: &[&str]) -> Fixture {
    Fixture {
        start: Some(recent.iter().map(PathBuf::from).collect()),
        ..Default::default()
    }
}

#[test]
fn the_card_sits_bottom_left_at_its_size() {
    let h = harness(with_card(&["/m/house.skr"]));
    let (view, card) = (h.state().central, rect_of(&h, "Start card"));
    assert!(
        (card.width() - 360.0).abs() < 0.5 && (card.height() - 200.0).abs() < 0.5,
        "{card:?}"
    );
    assert!((card.left() - (view.left() + 12.0)).abs() < 0.5, "{card:?}");
    assert!(
        (card.bottom() - (view.bottom() - 12.0)).abs() < 0.5,
        "{card:?}"
    );
    // The frame drawn is the card's size, border included: its contents fit.
    let t = theme::tokens(egui::Theme::Dark);
    let frame = drawn_shapes(&h)
        .into_iter()
        .filter_map(|s| match s {
            egui::Shape::Rect(r) if r.fill == t.bg_raised && r.stroke.color == t.border => Some(r),
            _ => None,
        })
        .max_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
        .expect("the card's frame");
    // egui strokes a frame inside its rectangle.
    assert_eq!(frame.stroke_kind, egui::StrokeKind::Inside);
    let outer = frame.rect;
    assert!((outer.size() - card.size()).length() < 0.5, "{outer:?}");
    assert!((outer.min - card.min).length() < 0.5, "{outer:?}");
}

#[test]
fn the_cards_buttons_report_their_actions() {
    let mut h = harness(with_card(&["/m/house.skr", "/m/barn.skr"]));
    // Dismiss sits on the New and Open row, at the right; all inside the card.
    let card = rect_of(&h, "Start card");
    let (new, dismiss) = (rect_of(&h, "New"), rect_of(&h, "Dismiss"));
    assert!(
        (new.center().y - dismiss.center().y).abs() < 1.0,
        "{new:?} {dismiss:?}"
    );
    assert!(
        (dismiss.right() - (card.right() - 13.0)).abs() < 1.0,
        "{dismiss:?}"
    );
    for label in ["New", "Open...", "Dismiss"] {
        assert!(card.contains_rect(rect_of(&h, label)), "{label}");
    }
    assert_eq!(click(&mut h, "New"), [UiAction::NewModel]);
    assert_eq!(click(&mut h, "Open..."), [UiAction::OpenFile]);
    assert_eq!(
        click(&mut h, "barn.skr"),
        [UiAction::OpenRecent("/m/barn.skr".into())]
    );
    assert_eq!(click(&mut h, "Dismiss"), [UiAction::DismissStart]);
}

#[test]
fn the_card_shows_five_recent_files_and_a_one_line_hint() {
    let names: Vec<String> = (1..=7).map(|i| format!("/m/{i}.skr")).collect();
    // The fixture passes what the app would: at most five.
    let five: Vec<&str> = names.iter().take(5).map(String::as_str).collect();
    let h = harness(with_card(&five));
    let card = rect_of(&h, "Start card");
    for i in 1..=5 {
        let row = rect_of(&h, &format!("{i}.skr"));
        assert!(card.contains_rect(row), "{i}.skr inside the card: {row:?}");
        assert!(row.height() <= 18.5, "{row:?}");
    }
    let texts = drawn_text_rects(&h);
    let (_, hint) = texts
        .iter()
        .find(|(t, _)| t == "R: drag a rectangle.  P: pull it up into a box.")
        .expect("hint");
    assert!(
        hint.width() <= 360.0 - 2.0 * 12.0,
        "one line inside the card"
    );
    assert!(hint.height() < 20.0, "one line: {hint:?}");
    assert!(card.contains_rect(*hint), "{hint:?}");
    let empty = harness(with_card(&[]));
    assert!(drawn_texts(&empty).iter().any(|t| t == "No recent files"));
}

#[test]
fn no_card_when_hidden_and_presses_on_it_stay_off_the_view() {
    let h = harness(Fixture::default());
    assert!(h.query_by_label("Start card").is_none());
    let h = harness(with_card(&[]));
    let at = rect_of(&h, "Start card").center();
    let layer = h.ctx.layer_id_at(at).expect("the card's area");
    assert_ne!(
        layer.order,
        egui::Order::Background,
        "Gui::wants_pointer_at is true there"
    );
}

#[test]
fn the_card_settles_in_one_frame() {
    // As `the_panels_settle_in_one_frame`: after the first frame, no repaint is requested.
    for recent in [&[][..], &["/m/house.skr", "/m/barn.skr"][..]] {
        let mut h = harness(with_card(recent));
        assert_eq!(h.try_run().unwrap(), 1);
    }
}
