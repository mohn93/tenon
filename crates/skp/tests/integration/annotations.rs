//! Dimensions and text of 2021+ files, on the corpus (`SKP_CORPUS`, the corpus folder).

use crate::read;
use skp::annotations::RawAnnotation;
use skp::scene::Model;
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

#[test]
fn fixtures_without_annotations_read_none() {
    for name in crate::FIXTURES {
        let (_, m, _) = read(crate::fixture(name)).unwrap();
        assert!(all(&m).is_empty(), "{name}");
        assert_eq!(m.annotation_skips().total(), 0, "{name}");
    }
}

/// `Untitled.skp` holds 13 linear dimensions (the same two lists of them twice over), all
/// between free points, drawn at 2.6 to 6.6 inches from what they measure.
#[test]
#[ignore = "needs a local corpus in SKP_CORPUS"]
fn corpus_linear_dimensions() {
    let Some(bytes) = corpus_file("openskp_fixtures/Untitled.skp") else {
        println!("SKP_CORPUS is not set; skipping");
        return;
    };
    let (_, m, _) = read(bytes).unwrap();
    let dims = all(&m);
    assert_eq!(dims.len(), 13);
    assert_eq!(m.annotation_skips().total(), 0);
    let RawAnnotation::Linear {
        start,
        end,
        offset,
        custom_text,
        ..
    } = dims[0]
    else {
        panic!("{:?}", dims[0]);
    };
    assert_eq!(*start, [0.0, 0.0, 0.0]);
    assert!((end[1] - 89.7396).abs() < 1e-3 && end[0] == 0.0 && end[2] == 0.0);
    // Drawn below the floor edge it measures.
    assert!(
        offset[2] < -4.7 && offset[0] == 0.0 && offset[1] == 0.0,
        "{offset:?}"
    );
    assert!(!custom_text);
    for d in &dims {
        let RawAnnotation::Linear { offset, .. } = d else {
            panic!()
        };
        let len = offset.iter().map(|x| x * x).sum::<f64>().sqrt();
        assert!((2.5..7.0).contains(&len), "{offset:?}");
    }
}

/// The tutorial's three labels: two free pushpin leaders and one attached to a guide point.
#[test]
#[ignore = "needs a local corpus in SKP_CORPUS"]
fn corpus_text_labels() {
    let Some(bytes) = corpus_file("su2024/su007_Tutorial01_.skp") else {
        println!("SKP_CORPUS is not set; skipping");
        return;
    };
    let (_, m, _) = read(bytes).unwrap();
    let notes = all(&m);
    assert_eq!(notes.len(), 3, "{:?}", m.annotation_skips());
    let texts: Vec<&str> = notes
        .iter()
        .map(|n| match n {
            RawAnnotation::Leader { text, .. } => text.as_str(),
            _ => panic!(),
        })
        .collect();
    assert_eq!(
        texts,
        ["Click here first", "Click here first", "Then click here"]
    );
    // The first is on a guide point, resolved to its position (not the zero a point ref
    // attached to an entity states).
    let RawAnnotation::Leader { anchor, vector, .. } = notes[0] else {
        panic!()
    };
    assert!(anchor.iter().any(|x| *x != 0.0), "{anchor:?}");
    assert!((vector[2] - 114.621259).abs() < 1e-5);
    let RawAnnotation::Leader { anchor, .. } = notes[2] else {
        panic!()
    };
    for (a, b) in anchor.iter().zip([60.0, -60.0, 240.0]) {
        assert!((a - b).abs() < 1e-6, "{anchor:?}");
    }
}
