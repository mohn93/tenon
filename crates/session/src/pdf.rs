//! A small PDF 1.4 writer: pages of RGB images, lines, filled rectangles and Helvetica text.
//!
//! Coordinates are millimetres with the origin at the page's top left (y down); the writer
//! converts to PDF points with the origin bottom left. The output is deterministic: objects
//! come in a fixed order, there is no date and no file id.

use std::fmt::Write as _;

use miniz_oxide::deflate::compress_to_vec_zlib;

const PT_PER_MM: f64 = 72.0 / 25.4;
const COMPRESSION_LEVEL: u8 = 6;
/// Average Helvetica glyph width in em, for placing text that is centred or right aligned.
const AVERAGE_GLYPH_EM: f64 = 0.55;

/// A document under construction.
#[derive(Default)]
pub struct Pdf {
    pages: Vec<Page>,
}

struct Page {
    size_mm: [f64; 2],
    content: Vec<u8>,
    images: Vec<Image>,
}

struct Image {
    width: u32,
    height: u32,
    /// zlib-compressed RGB.
    data: Vec<u8>,
}

/// Appends to one page of a [`Pdf`].
pub struct PageBuilder<'a> {
    page: &'a mut Page,
}

impl Pdf {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Adds a page of `size_mm` (width, height) and returns its builder.
    pub fn page(&mut self, size_mm: [f64; 2]) -> PageBuilder<'_> {
        self.pages.push(Page {
            size_mm,
            content: Vec::new(),
            images: Vec::new(),
        });
        PageBuilder {
            page: self.pages.last_mut().expect("just pushed"),
        }
    }

    /// The file. Objects: 1 catalog, 2 page tree, 3 font, then per page its page object, its
    /// content stream and its images.
    pub fn finish(&self) -> Vec<u8> {
        let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets: Vec<usize> = Vec::new();
        let mut object = |out: &mut Vec<u8>, body: &[u8]| {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        };
        // Numbers of each page's first object.
        let mut first = Vec::with_capacity(self.pages.len());
        let mut next = 4usize;
        for p in &self.pages {
            first.push(next);
            next += 2 + p.images.len();
        }
        object(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
        let kids: String = first.iter().map(|n| format!("{n} 0 R ")).collect();
        object(
            &mut out,
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                kids.trim_end(),
                self.pages.len()
            )
            .as_bytes(),
        );
        object(
            &mut out,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        );
        for (p, &n) in self.pages.iter().zip(&first) {
            let xobjects: String = (0..p.images.len())
                .map(|i| format!("/Im{i} {} 0 R ", n + 2 + i))
                .collect();
            let body = format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents {} 0 R \
                 /Resources << /Font << /F1 3 0 R >> /XObject << {}>> >> >>",
                num(p.size_mm[0] * PT_PER_MM),
                num(p.size_mm[1] * PT_PER_MM),
                n + 1,
                xobjects,
            );
            object(&mut out, body.as_bytes());
            object(&mut out, &stream("", &p.content));
            for im in &p.images {
                let dict = format!(
                    "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB \
                     /BitsPerComponent 8 /Filter /FlateDecode",
                    im.width, im.height
                );
                object(&mut out, &stream(&dict, &im.data));
            }
        }
        let xref = out.len();
        let count = offsets.len() + 1;
        let mut table = format!("xref\n0 {count}\n0000000000 65535 f \n");
        for o in &offsets {
            let _ = writeln!(table, "{o:010} 00000 n ");
        }
        let _ = write!(
            table,
            "trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
        );
        out.extend_from_slice(table.as_bytes());
        out
    }
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut body = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    body
}

/// A number with at most 3 decimals, no trailing zeros and no "-0".
fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".to_string()
    } else {
        s.to_string()
    }
}

impl PageBuilder<'_> {
    fn height_mm(&self) -> f64 {
        self.page.size_mm[1]
    }

    /// PDF x and y in points for a point in page millimetres.
    fn pt(&self, p: [f64; 2]) -> (String, String) {
        (
            num(p[0] * PT_PER_MM),
            num((self.height_mm() - p[1]) * PT_PER_MM),
        )
    }

    fn op(&mut self, s: &str) {
        self.page.content.extend_from_slice(s.as_bytes());
        self.page.content.push(b'\n');
    }

    /// An RGB image (`width` x `height`, 3 bytes per pixel, top row first) drawn in
    /// `rect_mm` = [x, y, width, height].
    pub fn image(&mut self, rect_mm: [f64; 4], width: u32, height: u32, rgb: &[u8]) {
        assert_eq!(
            rgb.len(),
            width as usize * height as usize * 3,
            "image data does not match its size"
        );
        let index = self.page.images.len();
        self.page.images.push(Image {
            width,
            height,
            data: compress_to_vec_zlib(rgb, COMPRESSION_LEVEL),
        });
        let (x, y) = self.pt([rect_mm[0], rect_mm[1] + rect_mm[3]]);
        let (w, h) = (num(rect_mm[2] * PT_PER_MM), num(rect_mm[3] * PT_PER_MM));
        self.op(&format!("q {w} 0 0 {h} {x} {y} cm /Im{index} Do Q"));
    }

    /// A black line `width_mm` wide.
    pub fn line(&mut self, a_mm: [f64; 2], b_mm: [f64; 2], width_mm: f64) {
        let (ax, ay) = self.pt(a_mm);
        let (bx, by) = self.pt(b_mm);
        let w = num(width_mm * PT_PER_MM);
        self.op(&format!("q 0 G {w} w {ax} {ay} m {bx} {by} l S Q"));
    }

    /// A rectangle [x, y, width, height] filled with grey `gray` (0 black, 1 white).
    pub fn rect_fill(&mut self, rect_mm: [f64; 4], gray: f64) {
        let (x, y) = self.pt([rect_mm[0], rect_mm[1] + rect_mm[3]]);
        let (w, h) = (num(rect_mm[2] * PT_PER_MM), num(rect_mm[3] * PT_PER_MM));
        let g = num(gray.clamp(0.0, 1.0));
        self.op(&format!("q {g} g {x} {y} {w} {h} re f Q"));
    }

    /// Black Helvetica text of `size_pt` with its baseline starting at `at_mm`. Characters
    /// outside WinAnsi, and line breaks, are written as "?".
    pub fn text(&mut self, at_mm: [f64; 2], size_pt: f64, text: &str) {
        let (x, y) = self.pt(at_mm);
        let s = pdf_string(text);
        self.op(&format!(
            "q 0 g BT /F1 {} Tf {x} {y} Td ({s}) Tj ET Q",
            num(size_pt)
        ));
    }
}

/// The width of `text` in millimetres at `size_pt`, estimated from an average glyph width.
pub fn text_width_mm(text: &str, size_pt: f64) -> f64 {
    text.chars().count() as f64 * AVERAGE_GLYPH_EM * size_pt / PT_PER_MM
}

/// The WinAnsi byte for `c`, if it has one.
fn win_ansi(c: char) -> Option<u8> {
    let cp = c as u32;
    match cp {
        0x20..=0x7E | 0xA0..=0xFF => Some(cp as u8),
        _ => Some(match c {
            '\u{20AC}' => 0x80,
            '\u{201A}' => 0x82,
            '\u{0192}' => 0x83,
            '\u{201E}' => 0x84,
            '\u{2026}' => 0x85,
            '\u{2020}' => 0x86,
            '\u{2021}' => 0x87,
            '\u{02C6}' => 0x88,
            '\u{2030}' => 0x89,
            '\u{0160}' => 0x8A,
            '\u{2039}' => 0x8B,
            '\u{0152}' => 0x8C,
            '\u{017D}' => 0x8E,
            '\u{2018}' => 0x91,
            '\u{2019}' => 0x92,
            '\u{201C}' => 0x93,
            '\u{201D}' => 0x94,
            '\u{2022}' => 0x95,
            '\u{2013}' => 0x96,
            '\u{2014}' => 0x97,
            '\u{02DC}' => 0x98,
            '\u{2122}' => 0x99,
            '\u{0161}' => 0x9A,
            '\u{203A}' => 0x9B,
            '\u{0153}' => 0x9C,
            '\u{017E}' => 0x9E,
            '\u{0178}' => 0x9F,
            _ => return None,
        }),
    }
}

/// The inside of a PDF literal string for `text`: WinAnsi bytes, non-ASCII as octal escapes.
fn pdf_string(text: &str) -> String {
    let mut s = String::new();
    for c in text.chars() {
        match win_ansi(c).unwrap_or(b'?') {
            b @ (b'(' | b')' | b'\\') => {
                s.push('\\');
                s.push(b as char);
            }
            b @ 0x20..=0x7E => s.push(b as char),
            b => {
                let _ = write!(s, "\\{b:03o}");
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
        hay[from..]
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|p| p + from)
    }

    fn sample() -> Vec<u8> {
        let mut pdf = Pdf::new();
        {
            let mut p = pdf.page([210.0, 297.0]);
            p.image(
                [10.0, 10.0, 50.0, 50.0],
                2,
                2,
                &[255, 0, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9],
            );
            p.line([10.0, 10.0], [100.0, 10.0], 0.25);
            p.rect_fill([10.0, 280.0, 20.0, 4.0], 0.0);
            p.text(
                [10.0, 290.0],
                8.0,
                "Hello (world) \\ caf\u{e9} \u{20AC} \u{4e2d}",
            );
        }
        pdf.page([100.0, 100.0]).text([1.0, 5.0], 8.0, "second");
        pdf.finish()
    }

    #[test]
    fn xref_offsets_and_lengths_are_exact() {
        let b = sample();
        let start = find(&b, b"startxref\n", 0).unwrap() + 10;
        let end = find(&b, b"\n", start).unwrap();
        let xref: usize = std::str::from_utf8(&b[start..end])
            .unwrap()
            .parse()
            .unwrap();
        assert!(b[xref..].starts_with(b"xref\n0 "));
        let line_end = find(&b, b"\n", xref + 5).unwrap();
        let count: usize = std::str::from_utf8(&b[xref + 7..line_end])
            .unwrap()
            .parse()
            .unwrap();
        // 3 fixed objects + 2 x (page + content) + 1 image.
        assert_eq!(count, 1 + 3 + 4 + 1);
        let t = line_end + 1;
        assert_eq!(&b[t..t + 20], b"0000000000 65535 f \n");
        for n in 1..count {
            let row = &b[t + 20 * n..t + 20 * (n + 1)];
            assert_eq!(&row[10..], b" 00000 n \n");
            let off: usize = std::str::from_utf8(&row[..10]).unwrap().parse().unwrap();
            assert!(
                b[off..].starts_with(format!("{n} 0 obj\n").as_bytes()),
                "object {n}"
            );
            // Stream length matches.
            let body_end = find(&b, b"endobj", off).unwrap();
            let body = &b[off..body_end];
            if let Some(s) = find(body, b"stream\n", 0) {
                let l = find(body, b"/Length ", 0).unwrap() + 8;
                let le = find(body, b" ", l).unwrap();
                let len: usize = std::str::from_utf8(&body[l..le]).unwrap().parse().unwrap();
                let data = &body[s + 7..];
                assert_eq!(&data[len..], b"\nendstream\n", "object {n}");
            }
        }
        assert!(b.ends_with(format!("startxref\n{xref}\n%%EOF\n").as_bytes()));
        let s = String::from_utf8_lossy(&b);
        assert!(s.contains("/Count 2"));
        assert!(s.contains(&format!("/Size {count} /Root 1 0 R")));
        assert!(s.contains("/MediaBox [0 0 595.276 841.89]"));
    }

    #[test]
    fn output_is_deterministic() {
        assert_eq!(sample(), sample());
        let s = String::from_utf8_lossy(&sample()).into_owned();
        assert!(!s.contains("/ID") && !s.contains("CreationDate"));
    }

    #[test]
    fn image_stream_inflates_to_the_pixels() {
        let b = sample();
        let at = find(&b, b"/FlateDecode /Length ", 0).unwrap() + 21;
        let e = find(&b, b" ", at).unwrap();
        let len: usize = std::str::from_utf8(&b[at..e]).unwrap().parse().unwrap();
        let ds = find(&b, b"stream\n", e).unwrap() + 7;
        let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&b[ds..ds + len]).unwrap();
        assert_eq!(raw, [255, 0, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9]);
    }

    #[test]
    fn text_is_win_ansi_and_escaped() {
        assert_eq!(pdf_string("a(b)c\\d"), "a\\(b\\)c\\\\d");
        assert_eq!(pdf_string("caf\u{e9}"), "caf\\351");
        assert_eq!(pdf_string("\u{20AC}"), "\\200");
        assert_eq!(pdf_string("\u{4e2d}x\n"), "?x?");
    }

    #[test]
    fn coordinates_flip_to_the_bottom_left() {
        let mut pdf = Pdf::new();
        pdf.page([100.0, 200.0]).line([0.0, 0.0], [100.0, 0.0], 1.0);
        let s = String::from_utf8_lossy(&pdf.finish()).into_owned();
        // y = 200 mm = 566.929 pt.
        assert!(s.contains("0 566.929 m 283.465 566.929 l S"), "{s}");
    }

    #[test]
    fn numbers_are_short() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(1.23456), "1.235");
    }
}
