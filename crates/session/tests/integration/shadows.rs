//! Location and sun shadow settings: the location is a model property (an undo step), the
//! shadow settings a view setting saved with the camera and scenes (no undo step).

use std::path::{Path, PathBuf};

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

fn ok(d: &mut Document, v: Value) -> Value {
    run(d, v).unwrap()
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn shadows(d: &Document) -> Value {
    ask(d, json!({"type": "camera"}))["shadows"].clone()
}

fn depth(d: &Document) -> u64 {
    ask(d, json!({"type": "history"}))["undo_depth"]
        .as_u64()
        .unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-shadows-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn stored(path: &Path) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

fn save(d: &mut Document, path: &Path) {
    ok(
        d,
        json!({"type": "save", "path": path.to_str().unwrap(), "overwrite": true}),
    );
}

fn open(path: &Path) -> Document {
    let mut d = doc();
    ok(
        &mut d,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    );
    d
}

fn square(d: &mut Document) {
    ok(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    );
}

fn default_shadows() -> Value {
    json!({
        "on": false,
        "date": "2026-11-08",
        "time": "13:30",
        "light": 80,
        "dark": 45,
        "on_ground": true,
    })
}

#[test]
fn the_defaults_are_sketchups() {
    let d = doc();
    assert_eq!(shadows(&d), default_shadows());
    let location = ask(&d, json!({"type": "location"}));
    assert_eq!(location["latitude"], 40.0167);
    assert_eq!(location["longitude"], -105.2833);
    assert_eq!(location["north_angle"], 0.0);
    assert_eq!(location["utc_offset"], -7.0);
    // The sun at 13:30 on 8 November in Boulder: low in the south-southwest.
    let sun = ask(&d, json!({"type": "sun"}));
    assert_eq!(sun["up"], true);
    assert!((sun["elevation"].as_f64().unwrap() - 28.17).abs() < 0.5);
    assert!((sun["azimuth"].as_f64().unwrap() - 208.75).abs() < 0.5);
    let dir: Vec<f64> = sun["direction"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let length = dir.iter().map(|v| v * v).sum::<f64>().sqrt();
    assert!((length - 1.0).abs() < 1e-9);
    // South-southwest: x and y both negative, z up.
    assert!(dir[0] < 0.0 && dir[1] < 0.0 && dir[2] > 0.4);
}

#[test]
fn set_shadows_changes_only_the_fields_given_and_is_not_an_edit() {
    let mut d = doc();
    square(&mut d);
    let (revision, undo) = (d.revision(), depth(&d));
    ok(&mut d, json!({"type": "set_shadows", "on": true}));
    let mut want = default_shadows();
    want["on"] = json!(true);
    assert_eq!(shadows(&d), want);
    ok(
        &mut d,
        json!({"type": "set_shadows", "date": "2028-02-29", "time": "07:05", "light": 100,
               "dark": 0, "on_ground": false}),
    );
    assert_eq!(
        shadows(&d),
        json!({"on": true, "date": "2028-02-29", "time": "07:05", "light": 100, "dark": 0,
               "on_ground": false})
    );
    assert_eq!(d.revision(), revision, "a view setting is not an edit");
    assert_eq!(depth(&d), undo, "and no undo step");
}

#[test]
fn bad_dates_times_and_levels_are_invalid_input_and_change_nothing() {
    let mut d = doc();
    for bad in [
        json!({"date": "2026-02-29"}),
        json!({"date": "2026-13-01"}),
        json!({"date": "2026-00-10"}),
        json!({"date": "2026-04-31"}),
        json!({"date": "2026-4-3"}),
        json!({"date": "08/11/2026"}),
        json!({"date": "1700-01-01"}),
        json!({"date": "2300-01-01"}),
        json!({"time": "24:00"}),
        json!({"time": "12:60"}),
        json!({"time": "9:05"}),
        json!({"time": "noon"}),
        json!({"light": 101}),
        json!({"dark": 101}),
        json!({"light": -1}),
        // One bad field spoils the rest.
        json!({"on": true, "light": 50, "time": "25:00"}),
    ] {
        let mut v = bad.clone();
        v["type"] = json!("set_shadows");
        let err = run(&mut d, v).unwrap_err();
        assert_eq!(err.0, ErrorKind::InvalidInput, "{bad}: {}", err.1);
        assert_eq!(shadows(&d), default_shadows(), "{bad} changed something");
    }
    // 1900 is not a leap year, 2000 is.
    assert!(run(&mut d, json!({"type": "set_shadows", "date": "1900-02-29"})).is_err());
    assert!(run(&mut d, json!({"type": "set_shadows", "date": "2000-02-29"})).is_ok());
}

#[test]
fn the_sun_follows_the_settings_and_the_location() {
    let mut d = doc();
    ok(
        &mut d,
        json!({"type": "set_shadows", "date": "2026-11-08", "time": "23:00"}),
    );
    assert_eq!(ask(&d, json!({"type": "sun"}))["up"], false);
    ok(
        &mut d,
        json!({"type": "set_shadows", "date": "2026-03-20", "time": "12:00"}),
    );
    ok(
        &mut d,
        json!({"type": "set_location", "latitude": 0.0, "longitude": 0.0, "utc_offset": 0.0}),
    );
    let sun = ask(&d, json!({"type": "sun"}));
    assert!((sun["elevation"].as_f64().unwrap() - 88.14).abs() < 0.5);
    // North a quarter turn clockwise from the green axis: the morning sun (east) is at +y...
    ok(
        &mut d,
        json!({"type": "set_location", "latitude": 40.0, "north_angle": 90.0}),
    );
    ok(&mut d, json!({"type": "set_shadows", "time": "12:00"}));
    let sun = ask(&d, json!({"type": "sun"}));
    // At noon in the northern middle latitudes the sun is south; with north at +x it is -x.
    assert!(sun["direction"][0].as_f64().unwrap() < -0.5, "{sun}");
}

#[test]
fn set_location_is_one_named_undo_step_and_changes_only_given_fields() {
    let mut d = doc();
    let before = depth(&d);
    let out = ok(&mut d, json!({"type": "set_location", "latitude": 51.5}));
    let _ = out;
    let location = ask(&d, json!({"type": "location"}));
    assert_eq!(location["latitude"], 51.5);
    assert_eq!(location["longitude"], -105.2833);
    assert_eq!(depth(&d), before + 1);
    let history = ask(&d, json!({"type": "history"}));
    assert_eq!(history["undo"][0], "set_location");
    ok(
        &mut d,
        json!({"type": "set_location", "longitude": -0.12, "north_angle": 12.5, "utc_offset": 5.75}),
    );
    assert_eq!(depth(&d), before + 2);
    ok(&mut d, json!({"type": "undo"}));
    assert_eq!(ask(&d, json!({"type": "location"}))["longitude"], -105.2833);
    assert_eq!(ask(&d, json!({"type": "location"}))["latitude"], 51.5);
    ok(&mut d, json!({"type": "redo"}));
    let location = ask(&d, json!({"type": "location"}));
    assert_eq!(location["utc_offset"], 5.75);
    assert_eq!(location["north_angle"], 12.5);
    // Nothing to change makes no step.
    let before = depth(&d);
    ok(&mut d, json!({"type": "set_location", "latitude": 51.5}));
    assert_eq!(depth(&d), before);
    // It is a model change.
    let revision = d.revision();
    ok(&mut d, json!({"type": "set_location", "latitude": 10.0}));
    assert!(d.revision() > revision);
}

#[test]
fn bad_locations_are_invalid_input_and_change_nothing() {
    let mut d = doc();
    let before = depth(&d);
    for bad in [
        json!({"latitude": 90.5}),
        json!({"latitude": -91}),
        json!({"longitude": 180.5}),
        json!({"longitude": -181}),
        json!({"north_angle": 361}),
        json!({"north_angle": -360.5}),
        json!({"utc_offset": 14.25}),
        json!({"utc_offset": -12.25}),
        json!({"utc_offset": 5.1}),
        json!({"latitude": 10, "utc_offset": 5.1}),
    ] {
        let mut v = bad.clone();
        v["type"] = json!("set_location");
        let err = run(&mut d, v).unwrap_err();
        assert_eq!(err.0, ErrorKind::InvalidInput, "{bad}: {}", err.1);
    }
    assert_eq!(depth(&d), before);
    assert_eq!(ask(&d, json!({"type": "location"}))["latitude"], 40.0167);
    // The edges of the ranges are fine.
    for good in [
        json!({"latitude": 90, "longitude": 180, "north_angle": 360, "utc_offset": 14}),
        json!({"latitude": -90, "longitude": -180, "north_angle": -360, "utc_offset": -12}),
        json!({"utc_offset": 5.5}),
        json!({"utc_offset": -3.75}),
    ] {
        let mut v = good.clone();
        v["type"] = json!("set_location");
        ok(&mut d, v);
    }
}

#[test]
fn a_failed_atomic_batch_puts_the_shadow_settings_back() {
    let mut d = doc();
    ok(
        &mut d,
        json!({"type": "set_shadows", "on": true, "light": 60}),
    );
    let batch: Vec<Command> = vec![
        serde_json::from_value(json!({"type": "set_shadows", "on": false, "light": 10})).unwrap(),
        serde_json::from_value(json!({"type": "set_location", "latitude": 1.0})).unwrap(),
        serde_json::from_value(json!({"type": "delete_tag", "tag": "t999"})).unwrap(),
    ];
    assert!(d.execute_batch(batch, true).is_err());
    let s = shadows(&d);
    assert_eq!(s["on"], true);
    assert_eq!(s["light"], 60);
    assert_eq!(ask(&d, json!({"type": "location"}))["latitude"], 40.0167);
}

#[test]
fn new_starts_with_the_defaults() {
    let mut d = doc();
    ok(
        &mut d,
        json!({"type": "set_shadows", "on": true, "time": "08:00"}),
    );
    ok(&mut d, json!({"type": "set_location", "latitude": 1.0}));
    ok(&mut d, json!({"type": "new", "discard": true}));
    assert_eq!(shadows(&d), default_shadows());
    assert_eq!(ask(&d, json!({"type": "location"}))["latitude"], 40.0167);
}

#[test]
fn files_without_either_are_unchanged() {
    let mut d = doc();
    square(&mut d);
    let path = scratch("plain.skr");
    save(&mut d, &path);
    let document = stored(&path);
    assert_eq!(document["schema_version"], 5);
    let text = document.to_string();
    assert!(
        !text.contains("shadows") && !text.contains("location"),
        "{text}"
    );
    // Settings put back to the defaults write the same.
    ok(&mut d, json!({"type": "set_shadows", "on": true}));
    ok(&mut d, json!({"type": "set_shadows", "on": false}));
    ok(&mut d, json!({"type": "set_location", "latitude": 3.0}));
    ok(&mut d, json!({"type": "set_location", "latitude": 40.0167}));
    save(&mut d, &path);
    assert_eq!(stored(&path), document);
}

#[test]
fn the_axes_schema_stays_15_without_either() {
    let mut d = doc();
    square(&mut d);
    ok(
        &mut d,
        json!({"type": "set_axes", "origin": [1, 0, 0], "x_axis": [1, 0, 0], "y_axis": [0, 1, 0]}),
    );
    let path = scratch("axes.skr");
    save(&mut d, &path);
    assert_eq!(stored(&path)["schema_version"], 15);
    assert_eq!(session::file::SCHEMA_VERSION, 16);
}

#[test]
fn shadow_settings_are_saved_with_the_camera_at_schema_16() {
    let mut d = doc();
    square(&mut d);
    ok(
        &mut d,
        json!({"type": "set_shadows", "on": true, "date": "2027-06-21", "time": "16:45",
               "light": 70, "dark": 30, "on_ground": false}),
    );
    let path = scratch("shadows.skr");
    save(&mut d, &path);
    let document = stored(&path);
    assert_eq!(document["schema_version"], 16);
    assert!(document["camera"].get("shadows").is_some());
    assert!(document["model"].get("location").is_none());
    let opened = open(&path);
    assert_eq!(
        shadows(&opened),
        json!({"on": true, "date": "2027-06-21", "time": "16:45", "light": 70, "dark": 30,
               "on_ground": false})
    );
    // Settings that are off but not the defaults are kept too.
    ok(&mut d, json!({"type": "set_shadows", "on": false}));
    save(&mut d, &path);
    assert_eq!(stored(&path)["schema_version"], 16);
    let opened = open(&path);
    assert_eq!(shadows(&opened)["time"], "16:45");
    assert_eq!(shadows(&opened)["on"], false);
}

#[test]
fn the_location_is_saved_with_the_model_at_schema_16() {
    let mut d = doc();
    square(&mut d);
    ok(
        &mut d,
        json!({"type": "set_location", "latitude": -33.87, "longitude": 151.21,
               "north_angle": -20.0, "utc_offset": 10.5}),
    );
    let path = scratch("location.skr");
    save(&mut d, &path);
    let document = stored(&path);
    assert_eq!(document["schema_version"], 16);
    assert_eq!(document["model"]["location"]["latitude"], -33.87);
    assert!(document["camera"].get("shadows").is_none());
    let opened = open(&path);
    let location = ask(&opened, json!({"type": "location"}));
    assert_eq!(location["latitude"], -33.87);
    assert_eq!(location["longitude"], 151.21);
    assert_eq!(location["north_angle"], -20.0);
    assert_eq!(location["utc_offset"], 10.5);
}

#[test]
fn a_file_with_a_bad_location_or_shadow_date_does_not_open() {
    let mut d = doc();
    square(&mut d);
    ok(&mut d, json!({"type": "set_location", "latitude": 10.0}));
    ok(&mut d, json!({"type": "set_shadows", "on": true}));
    let path = scratch("good.skr");
    save(&mut d, &path);
    let rewrite = |name: &str, edit: &dyn Fn(&mut Value)| {
        let mut document = stored(&path);
        edit(&mut document);
        let out = scratch(name);
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&out).unwrap());
        zip.start_file("document.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, document.to_string().as_bytes()).unwrap();
        zip.finish().unwrap();
        out
    };
    let opens = |p: &Path| {
        let mut e = doc();
        run(&mut e, json!({"type": "open", "path": p.to_str().unwrap()})).is_ok()
    };
    assert!(opens(&path));
    assert!(!opens(&rewrite("lat.skr", &|v| {
        v["model"]["location"]["latitude"] = json!(95.0)
    })));
    assert!(!opens(&rewrite("offset.skr", &|v| {
        v["model"]["location"]["utc_offset"] = json!(5.1)
    })));
    assert!(!opens(&rewrite("date.skr", &|v| {
        v["camera"]["shadows"]["month"] = json!(2);
        v["camera"]["shadows"]["day"] = json!(31);
    })));
    assert!(!opens(&rewrite("light.skr", &|v| {
        v["camera"]["shadows"]["light"] = json!(101)
    })));
}

#[test]
fn a_scene_stores_the_shadow_settings_and_going_to_it_restores_them() {
    let mut d = doc();
    square(&mut d);
    ok(
        &mut d,
        json!({"type": "set_shadows", "on": true, "date": "2026-12-21", "time": "10:15",
               "light": 90, "dark": 20, "on_ground": false}),
    );
    let out = ok(&mut d, json!({"type": "add_scene", "name": "Winter"}));
    let scene = out["changes"]["scenes"][0].clone();
    let view = ask(&d, json!({"type": "scene", "scene": scene}))["view"].clone();
    assert_eq!(
        view["shadows"],
        json!({"on": true, "date": "2026-12-21", "time": "10:15", "light": 90, "dark": 20,
               "on_ground": false})
    );
    ok(
        &mut d,
        json!({"type": "set_shadows", "on": false, "time": "12:00", "light": 80,
                      "dark": 45, "on_ground": true, "date": "2026-11-08"}),
    );
    ok(&mut d, json!({"type": "go_to_scene", "scene": scene}));
    assert_eq!(shadows(&d)["on"], true);
    assert_eq!(shadows(&d)["time"], "10:15");
    assert_eq!(shadows(&d)["on_ground"], false);
    // A scene whose view has shadows on needs schema 16, with no other setting on.
    ok(
        &mut d,
        json!({"type": "set_shadows", "on": false, "time": "13:30", "light": 80,
                      "dark": 45, "on_ground": true, "date": "2026-11-08"}),
    );
    let path = scratch("scene.skr");
    save(&mut d, &path);
    assert_eq!(stored(&path)["schema_version"], 16);
    let opened = open(&path);
    let id = ask(&opened, json!({"type": "scenes"}))["scenes"][0]["id"].clone();
    let view = ask(&opened, json!({"type": "scene", "scene": id}))["view"].clone();
    assert_eq!(view["shadows"]["time"], "10:15");
    assert_eq!(view["shadows"]["on"], true);
}

#[test]
fn a_scene_with_default_shadows_is_not_schema_16() {
    let mut d = doc();
    square(&mut d);
    ok(&mut d, json!({"type": "add_scene", "name": "Plain"}));
    let path = scratch("scene-plain.skr");
    save(&mut d, &path);
    let document = stored(&path);
    assert_eq!(document["schema_version"], 11);
    assert!(!document.to_string().contains("shadows"));
}
