//! Material density and the `mass_properties` query, driven through JSON.

use std::path::PathBuf;

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, String)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.message))
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-mass-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn stored(path: &PathBuf) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

fn material(d: &mut Document, density: Option<f64>) -> Value {
    let mut c = json!({"type": "add_material", "name": "Steel", "color": [200, 200, 210]});
    if let Some(x) = density {
        c["density"] = json!(x);
    }
    run(d, c).unwrap()["changes"]["materials"][0].clone()
}

fn add_box(d: &mut Document, size: [f64; 3], at: [f64; 3], material: &Value) -> Value {
    run(
        d,
        json!({"type": "add_box", "corner": at, "size": size, "as": "group",
               "material": material}),
    )
    .unwrap()["placed"][0]
        .clone()
}

fn close(a: &Value, b: f64) -> bool {
    (a.as_f64().unwrap() - b).abs() <= 1e-9 * b.abs().max(1.0)
}

#[test]
fn density_is_set_listed_cleared_and_validated() {
    let mut d = doc();
    let steel = material(&mut d, Some(7850.0));
    let listed = ask(&d, json!({"type": "materials"}));
    assert_eq!(listed["materials"][0]["density"], 7850.0);
    run(
        &mut d,
        json!({"type": "change_material", "material": steel, "density": 2700}),
    )
    .unwrap();
    assert_eq!(
        ask(&d, json!({"type": "materials"}))["materials"][0]["density"],
        2700.0
    );
    run(
        &mut d,
        json!({"type": "change_material", "material": steel, "density": null}),
    )
    .unwrap();
    assert!(
        ask(&d, json!({"type": "materials"}))["materials"][0]
            .get("density")
            .is_none()
    );
    for bad in [0.0, 0.5, 30001.0, -5.0] {
        let e = run(
            &mut d,
            json!({"type": "change_material", "material": steel, "density": bad}),
        )
        .unwrap_err();
        assert_eq!(e.0, ErrorKind::InvalidInput, "{bad}");
        assert!(e.1.contains("density"), "{}", e.1);
        let e = run(
            &mut d,
            json!({"type": "add_material", "name": "Bad", "color": [1, 2, 3], "density": bad}),
        )
        .unwrap_err();
        assert_eq!(e.0, ErrorKind::InvalidInput);
    }
    // Undo restores the density.
    run(
        &mut d,
        json!({"type": "change_material", "material": steel, "density": 100}),
    )
    .unwrap();
    d.execute(serde_json::from_value(json!({"type": "undo"})).unwrap())
        .unwrap();
    assert!(
        ask(&d, json!({"type": "materials"}))["materials"][0]
            .get("density")
            .is_none()
    );
}

#[test]
fn a_density_is_saved_as_the_new_schema_and_read_back_only_when_used() {
    let mut d = doc();
    let steel = material(&mut d, Some(7850.0));
    let path = scratch("steel.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let document = stored(&path);
    assert_eq!(
        document["schema_version"],
        session::file::DENSITY_SCHEMA_VERSION
    );
    assert_eq!(document["model"]["materials"][0]["density"], 7850.0);
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(
        ask(&e, json!({"type": "materials"})),
        ask(&d, json!({"type": "materials"}))
    );
    run(
        &mut d,
        json!({"type": "change_material", "material": steel, "density": null}),
    )
    .unwrap();
    run(&mut d, json!({"type": "save"})).unwrap();
    let document = stored(&path);
    assert!(
        document["schema_version"].as_u64().unwrap() < session::file::DENSITY_SCHEMA_VERSION as u64
    );
    assert!(document["model"]["materials"][0].get("density").is_none());
}

#[test]
fn mass_properties_of_a_box_and_an_assembly() {
    let mut d = doc();
    let steel = material(&mut d, Some(7850.0));
    let a = add_box(&mut d, [2.0, 3.0, 4.0], [0.0, 0.0, 0.0], &steel);
    let r = ask(&d, json!({"type": "mass_properties", "ids": [a]}));
    let t = &r["total"];
    let mass = 24.0 * 7850.0;
    assert!(close(&t["volume"], 24.0) && close(&t["mass"], mass), "{r}");
    assert!(close(&t["centroid"][0], 1.0) && close(&t["centroid"][2], 2.0));
    assert!(close(&t["inertia"][0][0], mass * 25.0 / 12.0));
    assert!(close(&t["inertia"][2][2], mass * 13.0 / 12.0));
    assert!(close(&t["principal"]["moments"][0], mass * 13.0 / 12.0));
    assert_eq!(r["instances"][0]["id"], a);
    assert!(close(&r["instances"][0]["mass"], mass));
    assert_eq!(r["skipped"], json!([]));
    // An empty list is every instance of the open context.
    let b = add_box(&mut d, [2.0, 3.0, 4.0], [10.0, 0.0, 0.0], &steel);
    let all = ask(&d, json!({"type": "mass_properties"}));
    assert_eq!(all["instances"].as_array().unwrap().len(), 2);
    assert!(close(&all["total"]["mass"], 2.0 * mass));
    assert!(close(&all["total"]["centroid"][0], 6.0));
    let one = ask(&d, json!({"type": "mass_properties", "ids": [b]}));
    assert!(close(&one["total"]["centroid"][0], 11.0));
}

#[test]
fn without_density_there_is_no_mass_and_bad_ids_are_refused() {
    let mut d = doc();
    let plain = material(&mut d, None);
    let a = add_box(&mut d, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0], &plain);
    let r = ask(&d, json!({"type": "mass_properties", "ids": [a]}));
    let t = &r["total"];
    assert!(close(&t["volume"], 1.0));
    assert!(t.get("mass").is_none() && t.get("inertia").is_none() && t.get("principal").is_none());
    assert!(close(&t["centroid"][0], 0.5));
    let q: Query =
        serde_json::from_value(json!({"type": "mass_properties", "ids": ["f1"]})).unwrap();
    assert_eq!(d.query(&q).unwrap_err().kind, ErrorKind::InvalidInput);
    let q: Query =
        serde_json::from_value(json!({"type": "mass_properties", "ids": ["i999"]})).unwrap();
    assert!(d.query(&q).is_err());
}

#[test]
fn painted_faces_win_and_the_open_context_does_not_change_the_mass() {
    let mut d = doc();
    let steel = material(&mut d, Some(7850.0));
    let red = run(
        &mut d,
        json!({"type": "add_material", "name": "Red", "color": [200, 0, 0]}),
    )
    .unwrap()["changes"]["materials"][0]
        .clone();
    // Steel faces inside a group painted Red: the faces' Steel counts.
    let a = add_box(&mut d, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0], &red);
    run(&mut d, json!({"type": "open_instance", "id": a})).unwrap();
    let faces: Vec<Value> = ask(&d, json!({"type": "entities"}))["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].clone())
        .collect();
    run(
        &mut d,
        json!({"type": "paint", "ids": faces, "material": steel}),
    )
    .unwrap();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    let r = ask(&d, json!({"type": "mass_properties", "ids": [a]}));
    assert!(close(&r["total"]["mass"], 7850.0), "{r}");
    // An unpainted box in an outer group painted Steel: the same from the root and inside.
    let none = Value::Null;
    let inner = add_box(&mut d, [2.0, 1.0, 1.0], [5.0, 0.0, 0.0], &none);
    let out = run(&mut d, json!({"type": "make_group", "ids": [inner]})).unwrap();
    let outer = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i.as_str().is_some_and(|s| s.starts_with('i')))
        .cloned()
        .unwrap_or_else(|| panic!("{out}"));
    run(
        &mut d,
        json!({"type": "paint", "ids": [outer], "material": steel}),
    )
    .unwrap();
    let root = ask(&d, json!({"type": "mass_properties", "ids": [outer]}));
    assert!(close(&root["total"]["mass"], 2.0 * 7850.0), "{root}");
    run(&mut d, json!({"type": "open_instance", "id": outer})).unwrap();
    let inside = ask(&d, json!({"type": "mass_properties"}));
    assert!(close(&inside["total"]["mass"], 2.0 * 7850.0), "{inside}");
}
