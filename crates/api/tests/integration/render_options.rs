//! `render_view`'s optional overrides: each changes the image and none changes the document.

use api::{HeadlessHost, codes, handle};
use base64::Engine;
use serde_json::{Value, json};

fn call(host: &mut HeadlessHost, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    serde_json::from_str(&handle(host, &line).expect("a reply")).unwrap()
}

/// A decoded render: width, height and RGBA pixels.
struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Image {
    fn px(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        self.rgba[i..i + 4].try_into().unwrap()
    }

    /// How many pixels differ from `other`.
    fn diff(&self, other: &Image) -> usize {
        assert_eq!((self.width, self.height), (other.width, other.height));
        self.rgba
            .chunks(4)
            .zip(other.rgba.chunks(4))
            .filter(|(a, b)| a != b)
            .count()
    }

    fn distinct_colors(&self) -> usize {
        let mut seen: Vec<&[u8]> = self.rgba.chunks(4).collect();
        seen.sort();
        seen.dedup();
        seen.len()
    }

    /// The columns spanned by drawn (not clear) pixels, in a transparent render.
    fn drawn_width(&self) -> u32 {
        let columns: Vec<u32> = (0..self.width)
            .filter(|&x| (0..self.height).any(|y| self.px(x, y)[3] > 0))
            .collect();
        columns.last().unwrap() - columns.first().unwrap() + 1
    }
}

fn decode(reply: &Value) -> Image {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(reply["result"]["png_base64"].as_str().expect("a png"))
        .unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    Image {
        width: info.width,
        height: info.height,
        rgba: buf,
    }
}

/// A 2 m cube on the ground around the origin.
fn cube(h: &mut HeadlessHost) {
    let r = call(
        h,
        "execute",
        json!({"type": "add_box", "corner": [-1, -1, 0], "size": [2, 2, 2]}),
    );
    assert!(r.get("error").is_none(), "{r}");
}

fn view() -> Value {
    json!({"eye": [4, -6, 3], "target": [0, 0, 1]})
}

/// Renders 64 x 48 from `view()` with `extra` fields added; None without a GPU.
fn render(h: &mut HeadlessHost, extra: Value) -> Option<Image> {
    let mut params = json!({"width": 64, "height": 48, "camera": view()});
    for (k, v) in extra.as_object().unwrap() {
        params[k] = v.clone();
    }
    let r = call(h, "render_view", params);
    if r["error"]["code"] == codes::RENDER_UNAVAILABLE {
        eprintln!("no GPU adapter, skipping");
        return None;
    }
    assert!(r.get("error").is_none(), "{r}");
    Some(decode(&r))
}

/// A tiny render with `extra` fields, for the replies that refuse it.
fn refused(h: &mut HeadlessHost, extra: Value) -> Value {
    let mut params = json!({"width": 16, "height": 16});
    for (k, v) in extra.as_object().unwrap() {
        params[k] = v.clone();
    }
    call(h, "render_view", params)
}

/// The document's revision and view settings, to compare after a render.
fn settings(h: &HeadlessHost) -> String {
    let d = &h.doc;
    format!(
        "{} {:?} {:?} {:?} {:?} {:?}",
        d.revision(),
        d.style(),
        d.lighting,
        d.section_display,
        d.shadows,
        d.camera
    )
}

#[test]
fn style_override_renders_with_that_style_and_leaves_the_document() {
    let mut h = HeadlessHost::new();
    cube(&mut h);
    let added = call(&mut h, "execute", json!({"type": "add_style"}));
    let id = added["result"]["changes"]["styles"][0].clone();
    call(
        &mut h,
        "execute",
        json!({"type": "change_style", "style": id, "settings": {"background": [10, 200, 30]}}),
    );
    let before = settings(&h);
    let Some(plain) = render(&mut h, json!({})) else {
        return;
    };
    let styled = render(&mut h, json!({"style": id})).unwrap();
    assert_eq!(styled.px(0, 0), [10, 200, 30, 255]);
    assert_ne!(plain.px(0, 0), styled.px(0, 0));
    assert_eq!(settings(&h), before);
    assert_eq!(h.doc.style(), None);
    // Rendering without the override afterwards is as before.
    assert_eq!(render(&mut h, json!({})).unwrap().diff(&plain), 0);
    let unknown = refused(&mut h, json!({"style": "y99999"}));
    assert_eq!(
        unknown["error"]["data"]["kind"], "invalid_input",
        "{unknown}"
    );
}

#[test]
fn projection_override_changes_the_silhouette() {
    let mut h = HeadlessHost::new();
    cube(&mut h);
    let before = settings(&h);
    let clear = |projection: &str| json!({"projection": projection, "background": "transparent"});
    let Some(perspective) = render(&mut h, clear("perspective")) else {
        return;
    };
    let parallel = render(&mut h, clear("parallel")).unwrap();
    assert!(perspective.diff(&parallel) > 50);
    assert_ne!(perspective.drawn_width(), parallel.drawn_width());
    // The override wins over the camera's own projection.
    let camera = json!({"eye": [4, -6, 3], "target": [0, 0, 1], "projection": "parallel"});
    let forced = render(
        &mut h,
        json!({"camera": camera, "projection": "perspective", "background": "transparent"}),
    )
    .unwrap();
    assert_eq!(forced.diff(&perspective), 0);
    assert_eq!(settings(&h), before);
    let bad = refused(&mut h, json!({"projection": "ortho"}));
    assert_eq!(bad["error"]["code"], codes::INVALID_PARAMS, "{bad}");
}

#[test]
fn section_override_changes_cuts_and_fill_and_leaves_the_display() {
    let mut h = HeadlessHost::new();
    cube(&mut h);
    call(
        &mut h,
        "execute",
        json!({"type": "add_section_plane", "point": [0, 0, 0], "normal": [1, 0, 0]}),
    );
    let before = settings(&h);
    // Look along the plane's normal from both sides: one of them sees the cut face.
    let (mut fill_changes, mut color_changes) = (false, false);
    for x in [8, -8] {
        let camera = json!({"eye": [x, 0.3, 1.5], "target": [0, 0, 1]});
        let Some(cut) = render(&mut h, json!({"camera": camera})) else {
            return;
        };
        let with = |h: &mut HeadlessHost, section: Value| {
            render(h, json!({"camera": camera, "section": section})).unwrap()
        };
        let whole = with(&mut h, json!({"cuts": false}));
        assert!(cut.diff(&whole) > 20, "cuts false shows the whole cube");
        fill_changes |= cut.diff(&with(&mut h, json!({"fill": false}))) > 20;
        color_changes |= cut.diff(&with(&mut h, json!({"fill_color": "material"}))) > 0;
    }
    assert!(fill_changes, "the fill shows on one side of the cut");
    assert!(color_changes, "the material fill differs from the dark one");
    assert_eq!(settings(&h), before);
    assert!(h.doc.section_display.cuts && h.doc.section_display.fill);
    let bad = refused(&mut h, json!({"section": {"cuts": "yes"}}));
    assert_eq!(bad["error"]["code"], codes::INVALID_PARAMS, "{bad}");
    let bad = refused(&mut h, json!({"section": {"fill_color": "red"}}));
    assert_eq!(bad["error"]["code"], codes::INVALID_PARAMS, "{bad}");
}

#[test]
fn shadows_override_toggles_the_sun_shadows() {
    let mut h = HeadlessHost::new();
    cube(&mut h);
    call(
        &mut h,
        "execute",
        json!({"type": "set_shadows", "on": false}),
    );
    let before = settings(&h);
    let Some(off) = render(&mut h, json!({})) else {
        return;
    };
    let on = render(&mut h, json!({"shadows": true})).unwrap();
    assert!(off.diff(&on) > 20, "shadows change the shading");
    assert!(!h.doc.shadows.on);
    assert_eq!(settings(&h), before);
    // With the document's shadows on, `shadows: false` turns them off for the render.
    call(
        &mut h,
        "execute",
        json!({"type": "set_shadows", "on": true}),
    );
    let doc_on = render(&mut h, json!({})).unwrap();
    assert_eq!(doc_on.diff(&on), 0);
    let forced_off = render(&mut h, json!({"shadows": false})).unwrap();
    assert_eq!(forced_off.diff(&off), 0);
    assert!(h.doc.shadows.on);
}

#[test]
fn supersample_keeps_the_size_and_smooths_edges() {
    let mut h = HeadlessHost::new();
    cube(&mut h);
    let before = settings(&h);
    let Some(one) = render(&mut h, json!({})) else {
        return;
    };
    let reply = call(
        &mut h,
        "render_view",
        json!({"width": 64, "height": 48, "camera": view(), "supersample": 3}),
    );
    assert_eq!(reply["result"]["width"], 64);
    let three = decode(&reply);
    assert_eq!((three.width, three.height), (64, 48));
    assert!(one.diff(&three) > 20);
    assert!(
        three.distinct_colors() > one.distinct_colors(),
        "more edge shades: {} vs {}",
        three.distinct_colors(),
        one.distinct_colors()
    );
    // 1 is what leaving it out gives.
    let explicit = render(&mut h, json!({"supersample": 1})).unwrap();
    assert_eq!(explicit.diff(&one), 0);
    assert_eq!(settings(&h), before);
}

#[test]
fn supersample_is_limited() {
    let mut h = HeadlessHost::new();
    for bad in [json!(0), json!(5)] {
        let r = refused(&mut h, json!({"supersample": bad}));
        assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
    }
    // 4096 * 3 is over twice the largest side.
    for (w, h_) in [(4096, 8), (8, 4096)] {
        let r = call(
            &mut h,
            "render_view",
            json!({"width": w, "height": h_, "supersample": 3}),
        );
        assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
    }
}

#[test]
fn transparent_background_leaves_undrawn_pixels_clear() {
    let mut h = HeadlessHost::new();
    cube(&mut h);
    let before = settings(&h);
    let Some(solid) = render(&mut h, json!({})) else {
        return;
    };
    assert_eq!(solid.px(0, 0)[3], 255);
    let clear = render(&mut h, json!({"background": "transparent"})).unwrap();
    assert_eq!(clear.px(0, 0)[3], 0);
    assert_eq!(clear.px(63, 0)[3], 0);
    assert_eq!(clear.px(32, 24)[3], 255, "the cube is opaque");
    // The cube's color is the same with the background or without.
    assert_eq!(clear.px(32, 24)[..3], solid.px(32, 24)[..3]);
    let styled = render(&mut h, json!({"background": "style"})).unwrap();
    assert_eq!(styled.diff(&solid), 0);
    // Antialiased edges keep partial alpha.
    let partial = clear.rgba.chunks(4).filter(|p| p[3] > 0 && p[3] < 255);
    assert!(partial.count() > 0);
    // Supersampling keeps it.
    let big = render(
        &mut h,
        json!({"background": "transparent", "supersample": 2}),
    )
    .unwrap();
    assert_eq!(big.px(0, 0)[3], 0);
    assert_eq!(big.px(32, 24)[3], 255);
    assert_eq!(settings(&h), before);
    let bad = refused(&mut h, json!({"background": "red"}));
    assert_eq!(bad["error"]["code"], codes::INVALID_PARAMS, "{bad}");
}

#[test]
fn see_through_faces_keep_their_alpha_on_a_transparent_background() {
    let mut h = HeadlessHost::new();
    cube(&mut h);
    let glass = call(
        &mut h,
        "execute",
        json!({"type": "add_material", "name": "Glass", "color": [40, 40, 240], "opacity": 0.5}),
    );
    let material = glass["result"]["changes"]["materials"][0].clone();
    assert!(material.is_string(), "{glass}");
    // A panel over half the cube and half the empty ground beside it, seen from above.
    let panel = call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [0, -1, 3], "opposite": [3, 1, 3]}),
    );
    let face = panel["result"]["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone();
    let painted = call(
        &mut h,
        "execute",
        json!({"type": "paint", "ids": [face], "material": material, "side": "both"}),
    );
    assert!(painted.get("error").is_none(), "{painted}");
    let top = json!({"eye": [0, 0, 10], "target": [0, 0, 0], "up": [0, 1, 0],
                     "projection": "parallel"});
    let Some(image) = render(
        &mut h,
        json!({"camera": top, "background": "transparent", "width": 128, "height": 96}),
    ) else {
        return;
    };
    let count = |range: std::ops::RangeInclusive<u8>| {
        image
            .rgba
            .chunks(4)
            .filter(|p| range.contains(&p[3]))
            .count()
    };
    assert!(
        count(255..=255) > 200,
        "the box stays opaque under the panel"
    );
    assert!(count(124..=132) > 400, "the panel alone shows half alpha");
    // The square of the opacity (alpha 64) is what the old blend left there.
    assert!(count(60..=68) < 100, "squared alpha: {}", count(60..=68));
}

#[test]
fn the_ground_shadow_has_partial_alpha_on_a_transparent_background() {
    let mut h = HeadlessHost::new();
    cube(&mut h);
    let top = json!({"eye": [0, 0, 12], "target": [0, 0, 0], "up": [0, 1, 0],
                     "projection": "parallel"});
    let mut shown = |shadows: bool| {
        render(
            &mut h,
            json!({"camera": top, "background": "transparent", "shadows": shadows}),
        )
    };
    let Some(off) = shown(false) else {
        return;
    };
    let on = shown(true).unwrap();
    let partial = |i: &Image| i.rgba.chunks(4).filter(|p| p[3] > 0 && p[3] < 255).count();
    assert!(
        partial(&on) > partial(&off) + 100,
        "{} with shadows, {} without",
        partial(&on),
        partial(&off)
    );
}

/// A joint turning the cube about a vertical axis through (3, 0): returns its id.
fn hinge(h: &mut HeadlessHost) -> Value {
    let (id, _) = h.doc.scene().root().instances().next().unwrap();
    let cube = json!(kernel::EntityId::Instance(id).to_string());
    let r = call(
        h,
        "execute",
        json!({"type": "add_joint", "kind": "revolute", "b": cube, "point": [3, 0, 0], "axis": [0, 0, 1]}),
    );
    assert!(r.get("error").is_none(), "{r}");
    r["result"]["changes"]["joints"][0].clone()
}

#[test]
fn joints_render_the_solved_pose_without_changing_the_document() {
    let mut h = HeadlessHost::new();
    cube(&mut h);
    let joint = hinge(&mut h);
    let before = settings(&h);
    let transforms = format!("{:?}", h.doc.scene().root().instances().collect::<Vec<_>>());
    let Some(plain) = render(&mut h, json!({})) else {
        return;
    };
    let turned = render(&mut h, json!({"joints": {joint.as_str().unwrap(): 90}})).unwrap();
    assert!(
        plain.diff(&turned) > 100,
        "the cube moved: {}",
        plain.diff(&turned)
    );
    assert_eq!(settings(&h), before, "no revision, no view change");
    assert_eq!(
        format!("{:?}", h.doc.scene().root().instances().collect::<Vec<_>>()),
        transforms
    );
    // The value the joint already has renders the document; an empty map does too.
    let same = render(&mut h, json!({"joints": {joint.as_str().unwrap(): 0}})).unwrap();
    assert_eq!(same.diff(&plain), 0);
    let empty = render(&mut h, json!({"joints": {}})).unwrap();
    assert_eq!(empty.diff(&plain), 0);
}

#[test]
fn joints_that_cannot_be_met_or_are_not_joints_are_refused() {
    let mut h = HeadlessHost::new();
    cube(&mut h);
    let joint = hinge(&mut h);
    let r = call(
        &mut h,
        "execute",
        json!({"type": "change_joint", "joint": joint, "limits": [-10, 50]}),
    );
    assert!(r.get("error").is_none(), "{r}");
    let before = settings(&h);
    // Refused before any rendering, so with or without a GPU.
    let r = refused(&mut h, json!({"joints": {joint.as_str().unwrap(): 80}}));
    assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
    assert!(
        r["error"]["message"].as_str().unwrap().contains("limits"),
        "{r}"
    );
    let r = refused(&mut h, json!({"joints": {"j999": 1}}));
    assert_eq!(r["error"]["data"]["kind"], "not_found", "{r}");
    let r = refused(&mut h, json!({"joints": {"x": 1}}));
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
    assert_eq!(settings(&h), before);
}

#[test]
fn play_needs_a_window() {
    let mut h = HeadlessHost::new();
    cube(&mut h);
    let joint = hinge(&mut h);
    let r = call(
        &mut h,
        "execute",
        json!({"type": "play", "joint": joint, "from": 0, "to": 90, "seconds": 2, "loop": true}),
    );
    assert_eq!(r["error"]["message"], "this program has no editor", "{r}");
}
