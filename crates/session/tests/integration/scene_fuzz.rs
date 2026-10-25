//! Garbled glTF, glb and COLLADA files: whatever the bytes, an import fails with an error or
//! succeeds with a valid scene, and never panics.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use glam::DVec2;
use serde_json::Value;
use session::{Document, ImportOptions};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// Imports `bytes` as `name` from memory; panics (failing the test) only when the import does.
fn survive(name: &str, bytes: Vec<u8>, what: &str) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut d = Document::new(DVec2::new(800.0, 600.0));
        if d.import_bytes(name, bytes, &ImportOptions::default())
            .is_ok()
        {
            d.scene().validate().unwrap();
        }
    }));
    assert!(result.is_ok(), "{name}: {what} panicked");
}

/// A small deterministic generator, so a failure repeats.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Cut short at many lengths, and with bytes overwritten here and there.
fn garble(name: &str, bytes: &[u8]) {
    let step = (bytes.len() / 64).max(1);
    for len in (0..bytes.len()).step_by(step) {
        survive(name, bytes[..len].to_vec(), &format!("cut at {len}"));
    }
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for round in 0..200 {
        let mut b = bytes.to_vec();
        for _ in 0..1 + rng.below(4) {
            let at = rng.below(b.len());
            b[at] = rng.next() as u8;
        }
        survive(name, b, &format!("round {round}"));
    }
}

const EXTREMES: [f64; 9] = [
    0.0,
    -1.0,
    1.0,
    3.0,
    255.0,
    65535.0,
    4_294_967_295.0,
    1.8e19,
    1e308,
];

/// Every number of the JSON replaced, one at a time, by each extreme.
fn renumber(value: &Value, visit: &mut dyn FnMut(Value)) {
    // Pointers to every number, each then changed in a copy.
    fn pointers(v: &Value, at: String, out: &mut Vec<String>) {
        match v {
            Value::Number(_) => out.push(at),
            Value::Array(a) => {
                for (i, x) in a.iter().enumerate() {
                    pointers(x, format!("{at}/{i}"), out);
                }
            }
            Value::Object(o) => {
                for (k, x) in o {
                    pointers(x, format!("{at}/{k}"), out);
                }
            }
            _ => {}
        }
    }
    let mut all = Vec::new();
    pointers(value, String::new(), &mut all);
    for p in all {
        for x in EXTREMES {
            let mut v = value.clone();
            *v.pointer_mut(&p).unwrap() = if x.fract() == 0.0 && x.abs() < 9e15 {
                Value::from(x as i64)
            } else {
                Value::from(x)
            };
            visit(v);
        }
    }
}

#[test]
fn garbled_gltf_files_never_panic() {
    let text = fixture("gltf/box.gltf");
    garble("box.gltf", &text);
    let json: Value = serde_json::from_slice(&text).unwrap();
    renumber(&json, &mut |v| {
        survive("box.gltf", v.to_string().into_bytes(), &v.to_string());
    });
    // With strides between elements, so they are garbled too.
    let mut strided = json.clone();
    for view in strided["bufferViews"].as_array_mut().unwrap() {
        view["byteStride"] = Value::from(16);
    }
    renumber(&strided, &mut |v| {
        survive("box.gltf", v.to_string().into_bytes(), &v.to_string());
    });
    let json: Value = serde_json::from_slice(&fixture("gltf/table.gltf")).unwrap();
    renumber(&json, &mut |v| {
        survive("table.gltf", v.to_string().into_bytes(), &v.to_string());
    });
}

#[test]
fn garbled_glb_files_never_panic() {
    garble("textured.glb", &fixture("gltf/textured.glb"));
}

#[test]
fn garbled_collada_files_never_panic() {
    let text = fixture("dae/chairs.dae");
    garble("chairs.dae", &text);
    // Each attribute holding a whole number, and each number in element text, set to
    // extremes one at a time.
    let s = String::from_utf8(text).unwrap();
    let mut at = 0;
    let mut spots = Vec::new();
    while let Some(i) = s[at..].find(|c: char| c.is_ascii_digit()) {
        let start = at + i;
        let len = s[start..]
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(s.len() - start);
        spots.push((start, start + len));
        at = start + len;
    }
    let mut rng = Rng(7);
    for (start, end) in spots {
        // Not every number of every float array: a sample, plus all attribute values.
        let attribute = s[..start].ends_with('"');
        if !attribute && rng.below(8) != 0 {
            continue;
        }
        for x in [
            "0",
            "1",
            "3",
            "99999",
            "4294967295",
            "18446744073709551615",
            "-1",
            "1e308",
        ] {
            let changed = format!("{}{x}{}", &s[..start], &s[end..]);
            survive("chairs.dae", changed.into_bytes(), &format!("{start}: {x}"));
        }
    }
}
