//! A structural walk of the model stream, used to prove the streaming works
//! and to report what a file contains before any geometry is built.

use crate::container::{Container, Generation};
use crate::error::Result;
use crate::source::ByteSource;
use crate::stream::{InflateRead, RangeRead, RecordStream, SeqRead};
use crate::tags;
use crate::tlv;
use crate::zip::entry_data_offset;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Default)]
pub struct WalkReport {
    /// (depth, tag, size) for the sections we entered or skipped.
    pub sections: Vec<(u32, u16, u64)>,
    pub definitions: u64,
    /// The largest payload we had to materialise. This is the number that
    /// decides whether a file of any size fits in a browser tab.
    pub peak_record: u64,
    pub stream_bytes: u64,
}

/// Enter `SketchUpModel`, then its sections, materialising one definition at a
/// time and never the whole stream.
pub fn walk_model(src: &dyn ByteSource, c: &Container) -> Result<WalkReport> {
    match c.generation {
        Generation::Vff => {
            let e = c
                .model_entry()
                .ok_or(crate::error::Error::Malformed("VFF file with no model.dat"))?;
            let at = entry_data_offset(src, e)?;
            let r = InflateRead::new(src, at, e.compressed_size, e.uncompressed_size)?;
            walk(RecordStream::new(r), e.uncompressed_size)
        }
        Generation::Legacy => {
            let (start, end) = c.legacy_stream.ok_or(crate::error::Error::Malformed(
                "legacy file with no model stream",
            ))?;
            let r = RangeRead::new(src, start, end);
            walk(RecordStream::new(r), end - start)
        }
    }
}

fn walk<R: SeqRead>(mut rs: RecordStream<R>, total: u64) -> Result<WalkReport> {
    let mut rep = WalkReport::default();
    let mut buf: Vec<u8> = Vec::new();

    // The outer record is the whole model. Enter it rather than reading it.
    let Some(root) = rs.header(total)? else {
        return Ok(rep);
    };
    rep.sections.push((0, root.tag, root.size));
    if root.tag != tags::SKETCH_UP_MODEL {
        // Not the wrapper we expect: report and stop rather than guess.
        return Ok(rep);
    }
    let root_end = rs.enter(&root)?;

    while let Some(sec) = rs.header(root_end)? {
        rep.sections.push((1, sec.tag, sec.size));
        // Enter only what we are going to iterate. Everything else is read
        // as a payload, because asking a leaf for its first child reads past
        // the end of it.
        let sec_end = if sec.tag == tags::SKETCH_UP_MODEL_COMPONENTS {
            rs.enter(&sec)?
        } else {
            rs.position() + sec.size
        };

        if sec.tag == tags::SKETCH_UP_MODEL_COMPONENTS {
            // Two more containers sit between the section and the definitions:
            //   SketchUpModelComponents > DefinitionList > DefinitionListDefs
            // Entering both is what keeps the high-water mark at one
            // definition instead of the whole 23 MB section.
            let mut inner_end = sec_end;
            for expect in [tags::DEFINITION_LIST, tags::DEFINITION_LIST_DEFS] {
                match rs.header(inner_end)? {
                    Some(h) if h.tag == expect => {
                        rep.sections.push((2, h.tag, h.size));
                        inner_end = rs.enter(&h)?;
                    }
                    Some(h) => {
                        // Not the shape we expect. Record what was actually
                        // there - a wrong guess about nesting is the single
                        // easiest way to silently read nothing - then skip it.
                        rep.sections.push((3, h.tag, h.size));
                        rs.skip(h.size)?;
                        inner_end = rs.position();
                        break;
                    }
                    None => {
                        inner_end = rs.position();
                        break;
                    }
                }
            }
            while let Some(def) = rs.header(inner_end)? {
                if def.tag != tags::COMPONENT_DEFINITION {
                    rs.skip(def.size)?;
                    continue;
                }
                rep.definitions += 1;
                rep.peak_record = rep.peak_record.max(def.size);
                rs.payload(def.size, &mut buf)?;
                // Touch the payload so the walk is not optimised away and so a
                // malformed definition is noticed here rather than later.
                let kids = tlv::Tlv::new(&buf).count();
                debug_assert!(kids < usize::MAX);
            }
        } else {
            rep.peak_record = rep.peak_record.max(sec.size);
            rs.payload(sec.size, &mut buf)?;
        }

        // Re-sync in case a nested walk left us short of the section end.
        let here = rs.position();
        if here < sec_end {
            rs.skip(sec_end - here)?;
        }
    }

    rep.stream_bytes = rs.position();
    Ok(rep)
}

pub fn tag_label(tag: u16) -> String {
    match tags::tag_name(tag) {
        Some(n) => alloc::format!("{n} ({tag:#06x})"),
        None => alloc::format!("unknown ({tag:#06x})"),
    }
}
