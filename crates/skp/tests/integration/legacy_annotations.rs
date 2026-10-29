//! Dimensions and text of 2017-2020 files, on the corpus (`SKP_CORPUS`, the corpus folder).

use crate::robustness::Rng;
use crate::{largest_allocation, read, ALLOCATION_LIMIT};
use skp::annotations::RawAnnotation;
use skp::container::{Container, Generation};
use skp::legacy;
use skp::scene::{Model, ReadOptions};
use skp::source::MemorySource;
use std::path::PathBuf;

fn corpus_file(rel: &str) -> Option<Vec<u8>> {
    let dir = std::env::var_os("SKP_CORPUS")?;
    Some(std::fs::read(PathBuf::from(dir).join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}")))
}

fn all(m: &Model) -> Vec<&RawAnnotation> {
    std::iter::once(&m.root)
        .chain(m.definitions.iter().map(|d| &d.geom))
        .flat_map(|g| &g.annotations)
        .collect()
}

fn read_legacy(bytes: Vec<u8>) -> skp::Result<Model> {
    let src = MemorySource::new(bytes);
    let c = Container::open(&src)?;
    if c.generation != Generation::Legacy {
        return Err(skp::Error::Malformed("not a legacy file"));
    }
    Ok(legacy::read_model_and_index(&src, &c, &ReadOptions::default())?.0)
}

/// The synthetic fixture: one leader text and one aligned dimension.
#[test]
#[ignore = "needs a local corpus in SKP_CORPUS"]
fn corpus_fixture_with_both_kinds() {
    let Some(bytes) = corpus_file("openskp_fixtures/legacy_annotations.skp") else {
        println!("SKP_CORPUS is not set; skipping");
        return;
    };
    let m = read_legacy(bytes).unwrap();
    assert_eq!(m.annotation_skips().total(), 0);
    let notes = all(&m);
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes.iter().any(|a| matches!(a,
        RawAnnotation::Leader { anchor, text, .. } if *anchor == [50.0, 50.0, 0.0] && text == "Hello")));
    assert!(notes.iter().any(|a| matches!(a,
        RawAnnotation::Linear { start, end, offset, custom_text: false, .. }
            if *start == [0.0; 3] && *end == [100.0, 0.0, 0.0] && *offset == [0.0, 10.0, 0.0])));
}

/// A SketchUp 2017 elevation with 28 dimensions (all three types) and a label. Dimensions
/// attached to a vertex resolve to its position; three attached to something else are unread.
/// A dimension line is drawn outside what it measures: where a segment lies on the model's
/// bounding box, its line must lie beyond it. This is the evidence for the offset directions
/// of `annotations::linear_offset`; the opposite sign of any type breaks it.
#[test]
#[ignore = "needs a local corpus in SKP_CORPUS"]
fn corpus_dimension_lines_lie_outside_the_model() {
    let Some(bytes) = corpus_file("openskp_fixtures/capilla_quiroz_v17.skp") else {
        println!("SKP_CORPUS is not set; skipping");
        return;
    };
    let m = read_legacy(bytes).unwrap();
    let dims = all(&m);
    assert_eq!(dims.len(), 26);
    assert_eq!(m.annotation_skips().unread, 3);
    // The outline of each drawing plane (the dimensions are on two elevations, y = 19.7 and
    // y = 0), taken from the dimensions' own points.
    let points = |d: &RawAnnotation| match d {
        RawAnnotation::Linear { start, end, .. } => [*start, *end],
        other => panic!("{other:?}"),
    };
    let outline = |plane: f64| {
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for p in dims
            .iter()
            .flat_map(|d| points(d))
            .filter(|p| (p[1] - plane).abs() < 1e-6)
        {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        (lo, hi)
    };
    let mut checked = 0;
    for d in dims.iter().copied() {
        let RawAnnotation::Linear {
            start,
            end,
            offset,
            custom_text,
            ..
        } = d
        else {
            panic!("{d:?}")
        };
        assert!(!custom_text);
        // Horizontal and vertical offsets are axis-aligned in this elevation.
        let ax = (0..3)
            .max_by(|a, b| offset[*a].abs().total_cmp(&offset[*b].abs()))
            .unwrap();
        let (lo, hi) = outline(start[1]);
        let on = |v: f64, edge: f64| (v - edge).abs() < 1e-6;
        if on(start[ax], hi[ax]) && on(end[ax], hi[ax]) {
            assert!(offset[ax] > 0.0, "{start:?} {end:?} {offset:?}");
            checked += 1;
        } else if on(start[ax], lo[ax]) && on(end[ax], lo[ax]) {
            assert!(offset[ax] < 0.0, "{start:?} {end:?} {offset:?}");
            checked += 1;
        }
    }
    // The base, eave and end-wall chains of the front elevation and the roof's base chain.
    assert!(checked >= 12, "{checked}");
}

/// The tutorial saved by SketchUp 2024 and again as a 2017, 2018, 2019 and 2020 file: the
/// three labels read the same in all five (the container is the only difference). No file
/// with dimensions exists in both containers, so the dimension offsets are not compared.
#[test]
#[ignore = "needs a local corpus in SKP_CORPUS"]
fn corpus_labels_match_the_2021_file() {
    let Some(modern) = corpus_file("su2024/su007_Tutorial01_.skp") else {
        println!("SKP_CORPUS is not set; skipping");
        return;
    };
    let (_, m, _) = read(modern).unwrap();
    type Key = (String, [f64; 3], [f64; 3]);
    let key = |m: &Model| -> Vec<Key> {
        let mut v: Vec<Key> = all(m)
            .iter()
            .map(|a| match a {
                RawAnnotation::Leader {
                    anchor,
                    vector,
                    text,
                    ..
                } => (text.clone(), *anchor, *vector),
                other => panic!("{other:?}"),
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1[0].total_cmp(&b.1[0])));
        v
    };
    let want = key(&m);
    assert_eq!(want.len(), 3);
    for year in 2017..=2020 {
        let bytes = corpus_file(&format!("../legacy_out/tutorial01_v{year}.skp")).unwrap();
        let old = read_legacy(bytes).unwrap();
        assert_eq!(old.annotation_skips().total(), 0, "{year}");
        let got = key(&old);
        assert_eq!(got.len(), 3, "{year}");
        for (g, w) in got.iter().zip(&want) {
            assert_eq!(g.0, w.0, "{year}");
            for i in 0..3 {
                assert!((g.1[i] - w.1[i]).abs() < 1e-6, "{year}: {g:?} {w:?}");
                assert!((g.2[i] - w.2[i]).abs() < 1e-6, "{year}: {g:?} {w:?}");
            }
        }
    }
}

/// Damage in the annotation records (the last bytes of the fixture's model stream) never
/// panics and never allocates wildly.
#[test]
#[ignore = "needs a local corpus in SKP_CORPUS"]
fn corpus_mutated_annotation_records_never_panic() {
    let Some(bytes) = corpus_file("openskp_fixtures/legacy_annotations.skp") else {
        println!("SKP_CORPUS is not set; skipping");
        return;
    };
    mutate(&bytes, 0xf80, 0x1120, 3000);
}

/// The same on a real file's dimensions (`capilla_quiroz_v17.skp`, where they start).
#[test]
#[ignore = "needs a local corpus in SKP_CORPUS"]
fn corpus_mutated_real_dimensions_never_panic() {
    let Some(bytes) = corpus_file("openskp_fixtures/capilla_quiroz_v17.skp") else {
        println!("SKP_CORPUS is not set; skipping");
        return;
    };
    // Where the file's first dimension record begins, found by its class name.
    let at = bytes
        .windows(16)
        .position(|w| w == b"CDimensionLinear")
        .expect("class name");
    mutate(&bytes, at, at + 6000, 1500);
}

fn mutate(bytes: &[u8], from: usize, to: usize, rounds: usize) {
    let to = to.min(bytes.len());
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut panicked = Vec::new();
    for round in 0..rounds {
        let mut b = bytes.to_vec();
        for _ in 0..1 + rng.below(4) {
            let at = from + rng.below(to - from);
            match rng.below(3) {
                0 => b[at] = rng.next() as u8,
                1 => b[at] ^= 1 << rng.below(8),
                _ => b[at] = 0xff,
            }
        }
        if round % 7 == 0 {
            b.truncate(from + rng.below(to - from));
        }
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            largest_allocation(|| {
                let _ = read_legacy(b);
            })
        }));
        match r {
            Ok(largest) => assert!(largest < ALLOCATION_LIMIT, "round {round}: {largest}"),
            Err(_) => panicked.push(round),
        }
    }
    assert!(panicked.is_empty(), "panicked in rounds {panicked:?}");
}
