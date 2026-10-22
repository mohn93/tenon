//! Printing: renders a document's sheets (see `session::print`) with the GPU into a PDF.

use session::pdf::Pdf;
use session::print::{PrintSpec, decorate, layout};
use session::{CommandError, Document, ErrorKind};

use crate::Snapshotter;

/// What printing made.
pub struct Printed {
    /// The PDF file's bytes.
    pub pdf: Vec<u8>,
    pub sheets: usize,
    pub warnings: Vec<String>,
}

/// Renders the current view of `doc` as `spec` says: one page per sheet, each tile drawn with
/// the document's lighting, tag colors and section display on white paper (no ground grid or
/// axes), nothing selected and no open group (the model is printed as it looks closed, without
/// the fading around an open context). Pixels are composited over white and stored as RGB.
pub fn render_pdf(
    snapshotter: &mut Snapshotter,
    doc: &Document,
    spec: &PrintSpec,
) -> Result<Printed, CommandError> {
    snapshotter.set_print_mode(true);
    let printed = render_sheets(snapshotter, doc, spec);
    snapshotter.set_print_mode(false);
    printed
}

fn render_sheets(
    snapshotter: &mut Snapshotter,
    doc: &Document,
    spec: &PrintSpec,
) -> Result<Printed, CommandError> {
    let layout = layout(doc, &doc.camera, spec)?;
    snapshotter.set_color_by_tag(doc.color_by_tag);
    snapshotter.set_section_display(doc.section_display);
    snapshotter.prepare(doc.scene(), &[], &Default::default());
    let mut pdf = Pdf::new();
    for (index, sheet) in layout.sheets.iter().enumerate() {
        let mut images = Vec::new();
        for chunk in sheet.chunks() {
            let rgba = snapshotter
                .capture_prepared(&chunk.camera, doc.lighting, chunk.px[0], chunk.px[1])
                .ok_or_else(|| {
                    CommandError::new(ErrorKind::File, "the GPU could not render the print")
                })?;
            images.push((chunk, white_rgb(&rgba)));
        }
        let mut page = pdf.page(sheet.page_mm);
        for (chunk, rgb) in &images {
            page.image(chunk.rect_mm, chunk.px[0], chunk.px[1], rgb);
        }
        decorate(&mut page, sheet, &layout.labels[index]);
    }
    Ok(Printed {
        pdf: pdf.finish(),
        sheets: layout.sheets.len(),
        warnings: layout.warnings,
    })
}

/// RGBA over white as RGB.
fn white_rgb(rgba: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for p in rgba.as_chunks::<4>().0 {
        let a = u32::from(p[3]);
        for &c in &p[..3] {
            rgb.push(((u32::from(c) * a + 255 * (255 - a) + 127) / 255) as u8);
        }
    }
    rgb
}

#[cfg(test)]
mod tests {
    use super::white_rgb;

    #[test]
    fn transparent_pixels_become_white() {
        assert_eq!(
            white_rgb(&[10, 20, 30, 255, 0, 0, 0, 0]),
            [10, 20, 30, 255, 255, 255]
        );
    }
}
