//! A line drawing (see `drawing`) as SVG text. Numbers are written with three decimals, so the
//! same drawing gives the same bytes.

use std::fmt::Write as _;

use glam::DVec2;

use crate::drawing::{Drawing, LabelAnchor};

/// The fill of section cuts, as in the window.
const FILL: &str = "#3a3a3a";
/// Line width, cut line width and text size in pixels (at scale they are 0.25, 0.75 and 3 mm).
const LINE_PX: f64 = 1.0;
const TEXT_PX: f64 = 11.0;
const LINE_MM: f64 = 0.25;
const TEXT_MM: f64 = 3.0;

/// A number with three decimals, never "-0.000".
fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    if s == "-0.000" {
        "0.000".to_string()
    } else {
        s
    }
}

/// `text` with the characters XML reserves written as entities, and control characters other
/// than the line break (which XML 1.0 cannot hold) as spaces.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() && c != '\n' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// The SVG of `drawing`: in pixels at the view's size, or with `mm_per_px` in millimetres
/// (a pixel of the view being that many millimetres on paper).
pub(crate) fn to_svg(drawing: &Drawing, mm_per_px: Option<f64>) -> String {
    let k = mm_per_px.unwrap_or(1.0);
    let (line, text_size) = match mm_per_px {
        Some(_) => (LINE_MM, TEXT_MM),
        None => (LINE_PX, TEXT_PX),
    };
    let size = drawing.size * k;
    let at = |p: DVec2| format!("{},{}", num(p.x * k), num(p.y * k));
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let (w, h) = match mm_per_px {
        Some(_) => (format!("{}mm", num(size.x)), format!("{}mm", num(size.y))),
        None => (num(size.x), num(size.y)),
    };
    let (vw, vh) = match mm_per_px {
        Some(_) => (num(size.x), num(size.y)),
        None => (num(size.x), num(size.y)),
    };
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {vw} {vh}\">"
    );
    let _ = writeln!(out, "<g id=\"fills\" fill=\"{FILL}\" stroke=\"none\">");
    for tri in &drawing.fills {
        let _ = writeln!(
            out,
            "<polygon points=\"{} {} {}\"/>",
            at(tri[0]),
            at(tri[1]),
            at(tri[2])
        );
    }
    out.push_str("</g>\n");
    let strokes = |out: &mut String, id: &str, width: f64| {
        let _ = writeln!(
            out,
            "<g id=\"{id}\" fill=\"none\" stroke=\"#000000\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\">",
            num(width)
        );
    };
    let polyline = |out: &mut String, points: &[DVec2]| {
        let points: Vec<String> = points.iter().map(|p| at(*p)).collect();
        let _ = writeln!(out, "<polyline points=\"{}\"/>", points.join(" "));
    };
    strokes(&mut out, "lines", line);
    for l in &drawing.lines {
        polyline(&mut out, l);
    }
    out.push_str("</g>\n");
    strokes(&mut out, "cuts", line * 3.0);
    for l in &drawing.cut_lines {
        polyline(&mut out, l);
    }
    out.push_str("</g>\n");
    strokes(&mut out, "annotations", line);
    for l in &drawing.annotation_lines {
        polyline(&mut out, l);
    }
    out.push_str("</g>\n");
    let _ = writeln!(
        out,
        "<g id=\"labels\" fill=\"#000000\" stroke=\"none\" font-family=\"Helvetica, Arial, sans-serif\" font-size=\"{}\">",
        num(text_size)
    );
    for label in &drawing.labels {
        let (anchor, down) = match label.anchor {
            LabelAnchor::Center => ("middle", 0.35),
            LabelAnchor::TopLeft => ("start", 0.8),
        };
        let (x, y) = (label.at.x * k, label.at.y * k + text_size * down);
        let _ = write!(
            out,
            "<text x=\"{}\" y=\"{}\" text-anchor=\"{anchor}\">",
            num(x),
            num(y)
        );
        let lines: Vec<&str> = label.text.split('\n').collect();
        if let [only] = lines[..] {
            out.push_str(&escape(only));
        } else {
            for (i, l) in lines.iter().enumerate() {
                let dy = if i == 0 { 0.0 } else { text_size * 1.2 };
                let _ = write!(
                    out,
                    "<tspan x=\"{}\" dy=\"{}\">{}</tspan>",
                    num(x),
                    num(dy),
                    escape(l)
                );
            }
        }
        out.push_str("</text>\n");
    }
    out.push_str("</g>\n</svg>\n");
    out
}
