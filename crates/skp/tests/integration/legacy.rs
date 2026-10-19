//! The pre-2021 container: the cupboard saved by SketchUp 2024 as a 2017, 2018, 2019 and
//! 2020 file reads as the same model as the original, damaged copies give errors rather
//! than panics, and an optional corpus-wide comparison.

use crate::robustness::Rng;
use crate::{fixture, largest_allocation, ALLOCATION_LIMIT};
use skp::container::{Container, Generation};
use skp::geom::{DefGeometry, NO_UV};
use skp::legacy::{self, ReadStats};
use skp::model::{SceneIndex, TextureLocation};
use skp::scene::{read_model, Model, ReadOptions};
use skp::source::MemorySource;
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const VERSIONS: [u32; 4] = [2017, 2018, 2019, 2020];

fn legacy_fixture(version: u32) -> Vec<u8> {
    fixture(&format!("legacy/cupboard_v{version}.skp"))
}

/// A legacy file's topology and index.
fn read_legacy(bytes: Vec<u8>) -> skp::Result<(Model, SceneIndex, ReadStats)> {
    let src = MemorySource::new(bytes);
    let c = Container::open(&src)?;
    if c.generation != Generation::Legacy {
        return Err(skp::Error::Malformed("not a legacy file"));
    }
    legacy::read_model_and_index(&src, &c, &ReadOptions::default())
}

/// A VFF file's topology and index.
fn read_vff(bytes: Vec<u8>) -> (Model, SceneIndex) {
    let src = MemorySource::new(bytes);
    let c = Container::open(&src).expect("open");
    assert_eq!(c.generation, Generation::Vff);
    let m = read_model(&src, &c, &ReadOptions::default()).expect("read");
    let index = m.index_without_placements(&src, &c);
    (m, index)
}

/// Faces, loops, edges and vertices.
fn shape(g: &DefGeometry) -> [usize; 4] {
    [g.faces(), g.loops(), g.edges(), g.vertices()]
}

/// Each definition's shapes, by name.
fn shapes(m: &Model) -> BTreeMap<String, Vec<[usize; 4]>> {
    let mut out: BTreeMap<String, Vec<[usize; 4]>> = BTreeMap::new();
    for d in &m.definitions {
        out.entry(d.name.clone()).or_default().push(shape(&d.geom));
    }
    for v in out.values_mut() {
        v.sort();
    }
    out
}

/// Each geometry's placements (root under ""), as the placed definition's name and the 13
/// transform terms, sorted.
fn placements(m: &Model) -> BTreeMap<String, Vec<(String, [f64; 13])>> {
    let names: BTreeMap<u64, &str> = m
        .definitions
        .iter()
        .map(|d| (d.pid, d.name.as_str()))
        .collect();
    let list = |g: &DefGeometry| {
        let mut v: Vec<(String, [f64; 13])> = g
            .instances
            .iter()
            .map(|i| (names.get(&i.def_pid).unwrap_or(&"?").to_string(), i.m))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.partial_cmp(&b.1).expect("finite")));
        v
    };
    let mut out = BTreeMap::new();
    out.insert(String::new(), list(&m.root));
    for d in &m.definitions {
        out.insert(d.name.clone(), list(&d.geom));
    }
    out
}

#[test]
fn legacy_cupboards_match_the_original() {
    let (original, original_index) = read_vff(fixture("cupboard.skp"));
    for version in VERSIONS {
        let src = MemorySource::new(legacy_fixture(version));
        let c = Container::open(&src).expect("open");
        assert_eq!(c.generation, Generation::Legacy, "{version}");
        let (m, stats) =
            legacy::read_model(&src, &c, &ReadOptions::default()).expect("legacy read");
        assert_eq!(stats.lost_entities, 0, "{version}: {:?}", stats.notes);
        assert!(!m.is_vff);
        assert_eq!(shape(&m.root), shape(&original.root), "{version}: root");
        assert_eq!(shapes(&m), shapes(&original), "{version}: definitions");

        let differences = geometry_differences(&m, &original, 1e-9);
        assert!(differences.is_empty(), "{version}: {differences:#?}");
        let index = legacy::read_index(&src, &c).expect("index");
        assert_eq!(
            material_use(&m, &index),
            material_use(&original, &original_index),
            "{version}: materials"
        );

        let (got, want) = (placements(&m), placements(&original));
        assert_eq!(
            got.keys().collect::<Vec<_>>(),
            want.keys().collect::<Vec<_>>(),
            "{version}"
        );
        for (owner, want) in &want {
            let got = &got[owner];
            assert_eq!(got.len(), want.len(), "{version}: placements in {owner:?}");
            for ((gn, gm), (wn, wm)) in got.iter().zip(want) {
                assert_eq!(gn, wn, "{version}: {owner:?}");
                for k in 0..13 {
                    assert!(
                        (gm[k] - wm[k]).abs() <= 1e-9,
                        "{version}: {owner:?} places {gn} at {gm:?}, the original at {wm:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_2020_gate_is_used_only_for_2020() {
    for version in VERSIONS {
        let src = MemorySource::new(legacy_fixture(version));
        let c = Container::open(&src).expect("open");
        let schema = legacy::layer_manager_schema(&src, &c)
            .expect("legacy")
            .expect("a version map");
        assert_eq!(
            schema >= 7,
            version == 2020,
            "{version}: CLayerManager {schema}"
        );
    }
}

/// An index's materials: name, colour and opacity (in thousandths), sorted.
fn materials(i: &SceneIndex) -> Vec<(String, [u8; 4], u32)> {
    let mut v: Vec<(String, [u8; 4], u32)> = i
        .materials
        .iter()
        .map(|m| (m.name.clone(), m.rgba, (m.opacity * 1000.0).round() as u32))
        .collect();
    v.sort();
    v
}

#[test]
fn legacy_index_has_materials() {
    let (_, original) = read_vff(fixture("cupboard.skp"));
    let key = materials;
    for version in VERSIONS {
        let src = MemorySource::new(legacy_fixture(version));
        let c = Container::open(&src).expect("open");
        let index = legacy::read_index(&src, &c).expect("index");
        assert_eq!(key(&index), key(&original), "{version}");
        assert!(index.bbox.is_none(), "{version}: no placements walked");
        // One walk gives the same index.
        let (_, again, _) = read_legacy(legacy_fixture(version)).expect("read");
        assert_eq!(key(&again), key(&index), "{version}");
    }
}

/// 200 damaged copies of a legacy file, each with a label.
fn legacy_mutants(bytes: &[u8], seed: u64) -> Vec<(String, Vec<u8>)> {
    let mut rng = Rng(seed);
    let mut out = Vec::new();
    let len = bytes.len();
    for k in 0..20 {
        let at = len * k / 20 + rng.below(len / 20);
        out.push((format!("truncated at {at}"), bytes[..at].to_vec()));
    }
    for (flips, count) in [(1, 60), (10, 60), (100, 50)] {
        for k in 0..count {
            let mut m = bytes.to_vec();
            for _ in 0..flips {
                let at = rng.below(len);
                m[at] ^= 1 << rng.below(8);
            }
            out.push((format!("{flips} bits flipped, mutant {k}"), m));
        }
    }
    for k in 0..9 {
        let mut m = bytes.to_vec();
        let at = rng.below(len - 4096);
        m[at..at + 4096].fill(0);
        out.push((format!("4 KiB zeroed at {at}, mutant {k}"), m));
    }
    let map = bytes
        .windows(11)
        .position(|w| w == b"CVersionMap")
        .expect("version map");
    let mut m = bytes.to_vec();
    m[map..map + 1024].fill(0xFF);
    out.push(("version map overwritten".to_string(), m));
    assert_eq!(out.len(), 200);
    out
}

/// What an importer reads from a legacy file (`read_index` is the same walk), and the batch
/// reader too.
fn read_everything_legacy(bytes: Vec<u8>) {
    let src = MemorySource::new(bytes);
    let Ok(c) = Container::open(&src) else {
        return;
    };
    if c.generation != Generation::Legacy {
        return;
    }
    let _ = legacy::read_model_and_index(&src, &c, &ReadOptions::default());
    let _ = legacy::read(&src, &c);
}

/// 200 mutants of one fixture never panic, nor allocate past the limit at once.
fn mutants_never_panic(version: u32) {
    let mut failed = Vec::new();
    let seed = 0x5851_F42D_4C95_7F2D + u64::from(version);
    for (label, m) in legacy_mutants(&legacy_fixture(version), seed) {
        let mut outcome = Ok(());
        let largest = largest_allocation(|| {
            outcome = catch_unwind(AssertUnwindSafe(|| read_everything_legacy(m)));
        });
        if outcome.is_err() {
            failed.push(format!("{version}: {label}: panicked"));
        } else if largest > ALLOCATION_LIMIT {
            failed.push(format!(
                "{version}: {label}: allocated {largest} bytes at once"
            ));
        }
    }
    assert!(failed.is_empty(), "failed:\n{}", failed.join("\n"));
}

// One test per fixture, so that the four run in parallel.
#[test]
fn legacy_mutants_never_panic_2017() {
    mutants_never_panic(2017);
}

#[test]
fn legacy_mutants_never_panic_2018() {
    mutants_never_panic(2018);
}

#[test]
fn legacy_mutants_never_panic_2019() {
    mutants_never_panic(2019);
}

#[test]
fn legacy_mutants_never_panic_2020() {
    mutants_never_panic(2020);
}

/// Where the 2017 cupboard's definition count and its second definition are.
const COUNT_AT: usize = 0x94ab;
const SECOND_DEFINITION_AT: usize = 0x9fcd;

/// A definition list that claims a million definitions, followed by 20,000 copies of the
/// start of a real definition, each of which fails to read: every failure starts another
/// `resync_definition` scan. The read ends within the time limit and reports what it lost.
#[test]
fn repeated_resyncs_are_bounded() {
    let bytes = legacy_fixture(2017);
    assert_eq!(
        &bytes[COUNT_AT..COUNT_AT + 6],
        &[6, 0, 0, 0, 0xff, 0xff],
        "the definition count, then the definition class"
    );
    assert_eq!(
        &bytes[SECOND_DEFINITION_AT..SECOND_DEFINITION_AT + 2],
        &[0x1c, 0x80]
    );
    let chunk = &bytes[SECOND_DEFINITION_AT..SECOND_DEFINITION_AT + 96];
    let mut crafted = bytes[..SECOND_DEFINITION_AT].to_vec();
    crafted[COUNT_AT..COUNT_AT + 4].copy_from_slice(&1_000_000u32.to_le_bytes());
    for _ in 0..20_000 {
        crafted.extend_from_slice(chunk);
    }
    crafted.extend_from_slice(&bytes[SECOND_DEFINITION_AT..]);

    let start = Instant::now();
    let result = read_legacy(crafted.clone());
    let index = {
        let src = MemorySource::new(crafted);
        let c = Container::open(&src).expect("open");
        legacy::read_index(&src, &c)
    };
    let took = start.elapsed();
    assert!(took < Duration::from_secs(2), "took {took:?}");
    if let Ok((_, _, stats)) = result {
        assert!(
            stats.lost_entities > 0,
            "a damaged list reads as lost entities"
        );
    }
    let _ = index;
}

#[test]
fn element_caps_apply_to_legacy_files() {
    let src = MemorySource::new(legacy_fixture(2017));
    let c = Container::open(&src).expect("open");
    let options = |cap| ReadOptions {
        max_elements: Some(cap),
        ..ReadOptions::default()
    };
    let r = legacy::read_model(&src, &c, &options(1000));
    assert!(
        matches!(r, Err(skp::Error::Malformed("too many elements"))),
        "a small cap is refused"
    );
    legacy::read_model(&src, &c, &options(100_000)).expect("a large cap reads");
}

/// Where two readings of the same model differ beyond counts: each definition's (and the
/// root's) vertex bounds and face planes, and every placement's transform. Empty when they
/// agree within `tol` inches.
fn geometry_differences(m: &Model, o: &Model, tol: f64) -> Vec<String> {
    fn bounds(g: &DefGeometry) -> [f64; 6] {
        let mut b = [f64::MAX, f64::MAX, f64::MAX, f64::MIN, f64::MIN, f64::MIN];
        for p in g.vertex_pos.as_chunks::<3>().0 {
            for a in 0..3 {
                b[a] = b[a].min(p[a]);
                b[3 + a] = b[3 + a].max(p[a]);
            }
        }
        b
    }
    /// Per face: its plane, then the centroid and the vertex count of its loops' rings,
    /// which is what the loops, their edge uses and the edges' vertices make together.
    fn planes(g: &DefGeometry) -> Vec<[f64; 8]> {
        let mut ring = Vec::new();
        (0..g.faces())
            .map(|f| {
                let n = g.normal(f);
                let (mut c, mut k) = ([0.0f64; 3], 0usize);
                for l in g.face_loop_ofs[f] as usize..g.face_loop_ofs[f + 1] as usize {
                    if !g.loop_ring(l, &mut ring) {
                        return [f64::NAN; 8];
                    }
                    for v in &ring {
                        let p = g.vertex(*v);
                        for a in 0..3 {
                            c[a] += p[a];
                        }
                    }
                    k += ring.len();
                }
                let m = k.max(1) as f64;
                [
                    n[0],
                    n[1],
                    n[2],
                    g.face_plane_d[f],
                    c[0] / m,
                    c[1] / m,
                    c[2] / m,
                    k as f64,
                ]
            })
            .collect()
    }
    let close = |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol);
    let mut out = Vec::new();
    fn by_name(m: &Model) -> BTreeMap<String, Vec<&DefGeometry>> {
        let mut map: BTreeMap<String, Vec<&DefGeometry>> = BTreeMap::new();
        map.insert(String::new(), vec![&m.root]);
        for d in &m.definitions {
            map.entry(d.name.clone()).or_default().push(&d.geom);
        }
        map
    }
    let (gm, om) = (by_name(m), by_name(o));
    for (name, gs) in &gm {
        let Some(os) = om.get(name) else {
            out.push(format!("{name:?} is not in the original"));
            continue;
        };
        if gs.len() != os.len() {
            out.push(format!(
                "{name:?}: {} definitions, {} in the original",
                gs.len(),
                os.len()
            ));
            continue;
        }
        for (g, og) in gs.iter().zip(os) {
            let positioned = |g: &DefGeometry| g.face_uv.iter().filter(|u| **u != NO_UV).count();
            if positioned(g) != positioned(og) {
                out.push(format!(
                    "{name:?}: {} faces with a positioned texture, {} in the original",
                    positioned(g),
                    positioned(og)
                ));
            }
            if g.vertices() > 0 && !close(&bounds(g), &bounds(og)) {
                out.push(format!(
                    "{name:?}: bounds {:?} vs {:?}",
                    bounds(g),
                    bounds(og)
                ));
            }
            // Each plane matched to one of the original's, in any order.
            let (p, op) = (planes(g), planes(og));
            let mut used = vec![false; op.len()];
            let unmatched = p.iter().find(|a| {
                match (0..op.len()).find(|&k| !used[k] && close(&a[..], &op[k])) {
                    Some(k) => {
                        used[k] = true;
                        false
                    }
                    None => true,
                }
            });
            if p.len() != op.len() || unmatched.is_some() {
                out.push(format!(
                    "{name:?}: face plane {unmatched:?} is not in the original"
                ));
            }
        }
    }
    let (gp, op) = (placements(m), placements(o));
    for (owner, want) in &op {
        let Some(got) = gp.get(owner) else {
            out.push(format!("no placements in {owner:?}"));
            continue;
        };
        if got.len() != want.len()
            || !got
                .iter()
                .zip(want)
                .all(|((gn, g), (wn, w))| gn == wn && close(g, w))
        {
            out.push(format!("placements in {owner:?} differ"));
        }
    }
    out
}

/// Each definition's faces' front and back material names, and its placements' material
/// names, sorted; "" for none.
fn material_use(m: &Model, index: &SceneIndex) -> BTreeMap<String, Vec<String>> {
    let names: BTreeMap<u32, &str> = index
        .materials
        .iter()
        .map(|x| (x.id, x.name.as_str()))
        .collect();
    let name = |id: u32| names.get(&id).copied().unwrap_or("").to_string();
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let all = std::iter::once((String::new(), &m.root))
        .chain(m.definitions.iter().map(|d| (d.name.clone(), &d.geom)));
    for (owner, g) in all {
        let v = out.entry(owner).or_default();
        v.extend(g.face_material.iter().map(|id| name(*id)));
        v.extend(
            g.face_back_material
                .iter()
                .map(|id| format!("back {}", name(*id))),
        );
        v.extend(g.instances.iter().filter(|i| i.has_material).map(|i| {
            format!(
                "placed {}",
                name(u32::try_from(i.material).unwrap_or(u32::MAX))
            )
        }));
        v.sort();
    }
    out
}

/// Every geometry of a model, summed: definitions, instances, faces, hidden faces, edges,
/// vertices, guides.
fn totals(m: &Model) -> [usize; 7] {
    let mut t = [m.definitions.len(), 0, 0, 0, 0, 0, 0];
    for g in std::iter::once(&m.root).chain(m.definitions.iter().map(|d| &d.geom)) {
        t[1] += g.instances.len();
        t[2] += g.faces();
        t[3] += g.face_flags.iter().filter(|f| **f & 1 != 0).count();
        t[4] += g.edges();
        t[5] += g.vertices();
        t[6] += g.guides.len();
    }
    t
}

/// Reads every `legacy_out/<model>_v<year>.skp` next to `$SKP_CORPUS` (or under
/// `$SKP_LEGACY`) and compares it with `legacy_src/<model>.skp`: definitions, instances,
/// faces, edges, vertices and materials. Prints a table; fails only on a panic.
#[test]
#[ignore = "needs a local corpus in SKP_CORPUS"]
fn corpus_legacy_matches_originals() {
    let dir: PathBuf = match (
        std::env::var_os("SKP_LEGACY"),
        std::env::var_os("SKP_CORPUS"),
    ) {
        (Some(d), _) => PathBuf::from(d),
        (None, Some(c)) => Path::new(&c).join("../legacy_out"),
        (None, None) => {
            println!("SKP_CORPUS is not set; skipping");
            return;
        }
    };
    let src_dir = dir.join("../legacy_src");
    let mut models: Vec<String> = std::fs::read_dir(&src_dir)
        .expect("legacy_src")
        .filter_map(|e| {
            let p = e.ok()?.path();
            if p.extension()? != "skp" {
                return None;
            }
            Some(p.file_stem()?.to_string_lossy().into_owned())
        })
        .collect();
    models.sort();
    let mut panicked = Vec::new();
    let mut matched = [0usize; 4];
    println!("model                  ver   result   defs inst faces(hidden) edges verts guides mats lost");
    for model in &models {
        let (original, oindex) =
            read_vff(std::fs::read(src_dir.join(format!("{model}.skp"))).expect("original"));
        let want = totals(&original);
        for (k, version) in VERSIONS.into_iter().enumerate() {
            let path = dir.join(format!("{model}_v{version}.skp"));
            let Ok(bytes) = std::fs::read(&path) else {
                println!("{model:22} {version}  missing");
                continue;
            };
            let r = catch_unwind(AssertUnwindSafe(|| read_legacy(bytes.clone())));
            let line = match r {
                Err(_) => {
                    panicked.push(format!("{model} {version}"));
                    "PANIC".to_string()
                }
                Ok(Err(e)) => format!("ERROR    {e}"),
                Ok(Ok((m, index, stats))) => {
                    let got = totals(&m);
                    // Guides are not compared: the 2021+ reader does not read them yet.
                    let mut differences = geometry_differences(&m, &original, 1e-6);
                    if material_use(&m, &index) != material_use(&original, &oindex) {
                        differences.push("faces or placements use other materials".to_string());
                    }
                    // The average colour of a textured material can differ by one in a
                    // channel between the two files.
                    let close = |a: &[(String, [u8; 4], u32)], b: &[(String, [u8; 4], u32)]| {
                        a.len() == b.len()
                            && a.iter().zip(b).all(|(x, y)| {
                                x.0 == y.0
                                    && x.2 == y.2
                                    && x.1.iter().zip(&y.1).all(|(p, q)| p.abs_diff(*q) <= 1)
                            })
                    };
                    // Each texture is an image where the index says, of the original's tile size.
                    for mat in &index.materials {
                        let Some(t) = &mat.texture else { continue };
                        let TextureLocation::Inline { offset, len } = t.location else {
                            differences.push(format!("{}: texture not inline", mat.name));
                            continue;
                        };
                        let image = &bytes[offset as usize..(offset + len) as usize];
                        let tile = oindex
                            .materials
                            .iter()
                            .find(|o| o.name == mat.name)
                            .and_then(|o| o.texture.as_ref())
                            .map(|o| o.tile_mm);
                        if !(image.starts_with(&[0xff, 0xd8]) || image.starts_with(b"\x89PNG"))
                            || tile.is_none_or(|w| {
                                (w[0] - t.tile_mm[0]).abs() > 1e-6
                                    || (w[1] - t.tile_mm[1]).abs() > 1e-6
                            })
                        {
                            differences.push(format!("{}: texture differs", mat.name));
                        }
                    }
                    if !close(&materials(&index), &materials(&oindex)) {
                        differences.push("material names, colours or opacities differ".to_string());
                    }
                    for d in &differences {
                        println!("    {d}");
                    }
                    let same = got[..6] == want[..6]
                        && index.materials.len() == oindex.materials.len()
                        && stats.lost_entities == 0
                        && differences.is_empty();
                    if same {
                        matched[k] += 1;
                    }
                    format!(
                        "{}    {} {} {}({}) {} {} {} {} {}{}",
                        if same { "match" } else { "DIFF " },
                        got[0],
                        got[1],
                        got[2],
                        got[3],
                        got[4],
                        got[5],
                        got[6],
                        index.materials.len(),
                        stats.lost_entities,
                        if same {
                            String::new()
                        } else {
                            format!(
                                "   original {} {} {}({}) {} {} - {}",
                                want[0],
                                want[1],
                                want[2],
                                want[3],
                                want[4],
                                want[5],
                                oindex.materials.len()
                            )
                        }
                    )
                }
            };
            println!("{model:22} {version}  {line}");
        }
    }
    for (k, version) in VERSIONS.into_iter().enumerate() {
        println!("{version}: {} of {} match", matched[k], models.len());
    }
    assert!(panicked.is_empty(), "panicked: {panicked:?}");
}
