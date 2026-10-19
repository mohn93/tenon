//! The fixtures read as SketchUp 2023 files, and an optional corpus-wide run.

use crate::{face_count, fixture, FIXTURES};
use skp::container::{Container, Generation};
use skp::scene::{read_model, ReadOptions};
use skp::source::MemorySource;
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
