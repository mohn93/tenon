//! The fixtures read as SketchUp 2023 files, and an optional corpus-wide run.

use crate::{face_count, fixture, FIXTURES};
use skp::container::{Container, Generation};
use skp::geom::{parse_component, parse_component_within, RawGuide};
use skp::scene::{read_model, ReadOptions};
use skp::source::MemorySource;
use skp::tags as t;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[test]
fn cupboard_and_desk_read_as_vff() {
    for name in FIXTURES {
        let src = MemorySource::new(fixture(name));
        let c = Container::open(&src).expect(name);
        assert_eq!(c.generation, Generation::Vff, "{name}");
        let version = c.header.version.trim_start_matches('{');
        assert!(
            version.starts_with("23."),
            "{name}: version {:?}",
            c.header.version
        );

        let m = read_model(&src, &c, &ReadOptions::default()).expect(name);
        assert!(face_count(&m) > 0, "{name}: no faces");
        let index = m.index(&src, &c);
        assert!(!index.materials.is_empty(), "{name}: no materials");

        // The cupboard places its knob and hinge components twice each. The
        // desk places each of its definitions at most once.
        let mut placements: BTreeMap<u64, usize> = BTreeMap::new();
        let all = m
            .root
            .instances
            .iter()
            .chain(m.definitions.iter().flat_map(|d| &d.geom.instances));
        for i in all {
            *placements.entry(i.def_pid).or_default() += 1;
        }
        let total: usize = placements.values().sum();
        let shared = total > placements.len();
        assert_eq!(
            shared,
            name == "cupboard.skp",
            "{name}: placements {placements:?}"
        );
    }
}

/// Reads every `*.skp` under `$SKP_CORPUS`. Legacy files are counted, not read.
#[test]
#[ignore = "needs a local corpus in SKP_CORPUS"]
fn corpus_reads() {
    let Some(dir) = std::env::var_os("SKP_CORPUS") else {
        println!("SKP_CORPUS is not set; skipping");
        return;
    };
    let mut files = Vec::new();
    collect(Path::new(&dir), &mut files);
    files.sort();
    let (mut vff, mut legacy, mut failed) = (0, 0, Vec::new());
    for path in &files {
        let bytes = std::fs::read(path).expect("read corpus file");
        let src = MemorySource::new(bytes);
        let c = match Container::open(&src) {
            Ok(c) => c,
            Err(e) => {
                failed.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        if c.generation == Generation::Legacy {
            legacy += 1;
            continue;
        }
        vff += 1;
        match read_model(&src, &c, &ReadOptions::default()) {
            Ok(m) => {
                let _ = m.index(&src, &c);
                println!(
                    "ok {} ({}, {} faces)",
                    path.display(),
                    c.header.version,
                    face_count(&m)
                );
            }
            Err(e) => failed.push(format!("{}: {e}", path.display())),
        }
    }
    println!(
        "{} files: {vff} VFF read, {legacy} legacy skipped, {} failed",
        files.len(),
        failed.len()
    );
    assert!(failed.is_empty(), "failed:\n{}", failed.join("\n"));
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read corpus dir") {
        let path = entry.expect("corpus entry").path();
        if path.is_dir() {
            collect(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("skp"))
        {
            out.push(path);
        }
    }
}

/// Neither fixture has guides; a 2021+ read gives none and skips none.
#[test]
fn vff_fixtures_have_no_guides() {
    for name in FIXTURES {
        let src = MemorySource::new(fixture(name));
        let c = Container::open(&src).expect(name);
        let m = read_model(&src, &c, &ReadOptions::default()).expect(name);
        for g in std::iter::once(&m.root).chain(m.definitions.iter().map(|d| &d.geom)) {
            assert!(g.guides.is_empty(), "{name}: {:?}", g.guides);
        }
        assert_eq!((m.hidden_guides(), m.unread_guides()), (0, 0), "{name}");
    }
}

/// One TLV record: a `u16` tag, a `u32` size, the payload.
fn rec(tag: u16, payload: &[u8]) -> Vec<u8> {
    let mut v = tag.to_le_bytes().to_vec();
    v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

fn f64s(v: &[f64]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// `ConstructionGeometry > DrawingElement { Entity { EntityPID }, DrawingElementFlags }`,
/// as SketchUp 2024 writes it before a guide's own records.
fn construction_geometry(pid: u16, flags: u8) -> Vec<u8> {
    let entity = rec(t::ENTITY, &rec(t::ENTITY_PID, &pid.to_le_bytes()));
    let mut de = entity;
    de.extend(rec(t::DRAWING_ELEMENT_LAYER_PID_REF, &[0x53, 0xce]));
    de.extend(rec(t::DRAWING_ELEMENT_FLAGS, &[flags]));
    rec(t::CONSTRUCTION_GEOMETRY, &rec(t::DRAWING_ELEMENT, &de))
}

fn guide_point(pid: u16, flags: u8, position: &[u8]) -> Vec<u8> {
    let mut p = construction_geometry(pid, flags);
    p.extend(rec(t::CONSTRUCTION_POINT_POSITION, position));
    p.extend(rec(t::CONSTRUCTION_POINT_FROM_POSITION, &f64s(&[0.0; 3])));
    p.extend(rec(t::CONSTRUCTION_POINT_DISPLAY_LINE, &[1]));
    rec(t::CONSTRUCTION_POINT, &p)
}

fn guide_line(pid: u16, flags: u8, body: &[u8]) -> Vec<u8> {
    let mut l = construction_geometry(pid, flags);
    l.extend(rec(t::CONSTRUCTION_LINE_GEOM_LINE, body));
    l.extend(rec(t::CONSTRUCTION_LINE_STIPPLE, &[0]));
    rec(t::CONSTRUCTION_LINE, &l)
}

/// A component whose guide lists hold one good guide of each kind, a hidden one of each,
/// and records that are not a guide this reader knows: they are skipped and counted.
fn component_with_guides() -> Vec<u8> {
    let mut points = guide_point(1, 0x06, &f64s(&[1.0, 2.0, 3.0]));
    points.extend(guide_point(2, 0x07, &f64s(&[9.0, 9.0, 9.0])));
    // A point without its position, one whose position is short, and a record that is
    // not a point at all.
    points.extend(rec(t::CONSTRUCTION_POINT, &construction_geometry(3, 0x06)));
    points.extend(guide_point(4, 0x06, &f64s(&[1.0, 2.0])));
    points.extend(guide_line(5, 0x06, &f64s(&[0.0; 8])));
    let line = [4.0, 5.0, 6.0, 0.0, 0.0, 1.0, -1e30, 1e30];
    let mut lines = guide_line(6, 0x06, &f64s(&line));
    lines.extend(guide_line(7, 0x07, &f64s(&line)));
    // A line whose body is shorter than its eight doubles, and a point in the line list.
    lines.extend(guide_line(8, 0x06, &f64s(&line[..6])));
    lines.extend(guide_point(9, 0x06, &f64s(&[7.0, 7.0, 7.0])));
    let mut c = rec(t::COMPONENT_CONSTRUCTION_POINTS, &points);
    c.extend(rec(t::COMPONENT_CONSTRUCTION_LINES, &lines));
    c
}

#[test]
fn vff_guide_records_are_read_and_unknown_ones_counted() {
    let g = parse_component(&component_with_guides()).expect("component");
    assert_eq!(
        g.guides,
        vec![
            RawGuide::Point {
                position: [1.0, 2.0, 3.0]
            },
            RawGuide::Line {
                point: [4.0, 5.0, 6.0],
                direction: [0.0, 0.0, 1.0]
            },
        ]
    );
    assert_eq!(g.hidden_guides, 2);
    assert_eq!(g.unread_guides, 5);
}

#[test]
fn vff_guides_count_against_the_element_budget() {
    let bytes = component_with_guides();
    let mut enough = 5; // the tables, then the two guides kept and the two hidden ones
    let g = parse_component_within(&bytes, &mut enough).expect("within budget");
    assert_eq!(
        (g.guides.len(), g.hidden_guide_items.len(), enough),
        (2, 2, 0)
    );
    let mut short = 4;
    assert!(parse_component_within(&bytes, &mut short).is_err());
}

/// The tutorial's 7 guide points, read from its SketchUp 2024 file, are where the 2017-2020
/// copies of it put them.
#[test]
#[ignore = "needs a local corpus in SKP_CORPUS"]
fn vff_guides_are_read() {
    let Some(dir) = std::env::var_os("SKP_CORPUS") else {
        println!("SKP_CORPUS is not set; skipping");
        return;
    };
    let dir = Path::new(&dir);
    let src = MemorySource::new(
        std::fs::read(dir.join("su2024/su007_Tutorial01_.skp")).expect("tutorial"),
    );
    let c = Container::open(&src).expect("open");
    let m = read_model(&src, &c, &ReadOptions::default()).expect("read");
    let vff = all_guides(&m);
    assert_eq!(vff.len(), 7, "{vff:?}");
    assert!(vff.iter().all(|g| matches!(g, RawGuide::Point { .. })));
    assert_eq!((m.hidden_guides(), m.unread_guides()), (0, 0));
    for year in 2017..=2020 {
        let path = dir.join(format!("../legacy_out/tutorial01_v{year}.skp"));
        let src = MemorySource::new(std::fs::read(&path).expect("legacy copy"));
        let c = Container::open(&src).expect("open legacy");
        let (lm, stats) =
            skp::legacy::read_model(&src, &c, &ReadOptions::default()).expect("legacy");
        let legacy = all_guides(&lm);
        assert_eq!(stats.hidden_guides, 0, "{year}");
        assert_eq!(legacy.len(), vff.len(), "{year}: {legacy:?}");
        for (a, b) in vff.iter().zip(&legacy) {
            let (RawGuide::Point { position: p }, RawGuide::Point { position: q }) = (a, b) else {
                panic!("{year}: {a:?} against {b:?}");
            };
            let d = (0..3).map(|k| (p[k] - q[k]).abs()).fold(0.0, f64::max);
            assert!(d <= 1e-9, "{year}: {p:?} against {q:?}");
            println!("{year}: {p:?} = {q:?} (off by {d:e})");
        }
    }
}

fn all_guides(m: &skp::scene::Model) -> Vec<RawGuide> {
    std::iter::once(&m.root)
        .chain(m.definitions.iter().map(|d| &d.geom))
        .flat_map(|g| g.guides.iter().copied())
        .collect()
}
