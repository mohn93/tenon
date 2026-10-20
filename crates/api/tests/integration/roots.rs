//! `--api-root`: every path the API reads or writes stays inside the roots.

use std::path::PathBuf;

use api::{ApiRoots, HeadlessHost, codes, handle};
use serde_json::{Value, json};

fn call(host: &mut HeadlessHost, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    serde_json::from_str(&handle(host, &line).expect("a reply")).unwrap()
}

/// A fresh folder for one test, with `root` and `outside` inside it.
fn folders(name: &str) -> (PathBuf, PathBuf) {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("api-roots-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (root, outside) = (base.join("root"), base.join("outside"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    (root, outside)
}

fn host(root: &PathBuf) -> HeadlessHost {
    HeadlessHost::with_roots(ApiRoots::new(std::slice::from_ref(root)).unwrap())
}

fn edge(h: &mut HeadlessHost) {
    call(
        h,
        "execute",
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
    );
}

#[test]
fn relative_paths_land_in_the_first_root() {
    let (root, _) = folders("relative");
    let mut h = host(&root);
    edge(&mut h);
    let r = call(&mut h, "execute", json!({"type": "save", "path": "a.skr"}));
    assert!(r["result"].is_object(), "{r}");
    assert!(root.join("a.skr").exists());
    let r = call(&mut h, "execute", json!({"type": "open", "path": "a.skr"}));
    assert!(r["result"].is_object(), "{r}");
    let file = call(&mut h, "query", json!({"type": "file"}));
    let real = session::fspath::canonical(&root).unwrap();
    assert_eq!(file["result"]["api_roots"], json!([real]));
    assert_eq!(file["result"]["path"], json!(real.join("a.skr")));
}

#[test]
fn paths_outside_the_roots_are_refused() {
    let (root, outside) = folders("outside");
    let mut h = host(&root);
    edge(&mut h);
    std::fs::write(
        outside.join("model.obj"),
        "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
    )
    .unwrap();
    std::fs::write(outside.join("t.png"), "x").unwrap();
    let cases = [
        json!({"type": "save", "path": outside.join("a.skr")}),
        json!({"type": "save", "path": "../outside/a.skr"}),
        json!({"type": "export", "path": outside.join("a.stl")}),
        json!({"type": "import", "path": outside.join("model.obj")}),
        json!({"type": "open", "path": "../outside/model.obj"}),
        json!({"type": "add_material", "texture": {"path": outside.join("t.png")}}),
    ];
    for c in cases {
        let r = call(&mut h, "execute", c.clone());
        assert_eq!(r["error"]["code"], codes::COMMAND_FAILED, "{c}: {r}");
        assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{c}: {r}");
        assert!(
            r["error"]["message"]
                .as_str()
                .unwrap()
                .contains("--api-root"),
            "{r}"
        );
    }
    assert!(!outside.join("a.skr").exists() && !outside.join("a.stl").exists());
}

#[cfg(unix)]
#[test]
fn links_out_of_the_roots_are_followed_and_refused() {
    let (root, outside) = folders("links");
    let mut h = host(&root);
    edge(&mut h);
    std::os::unix::fs::symlink(&outside, root.join("door")).unwrap();
    std::fs::write(outside.join("secret.obj"), "v 0 0 0\n").unwrap();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "save", "path": "door/a.skr"}),
    );
    assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
    let r = call(
        &mut h,
        "execute",
        json!({"type": "import", "path": "door/secret.obj"}),
    );
    assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
    // A link as the file to write is refused even when it points inside.
    std::fs::write(root.join("real.skr"), "").unwrap();
    std::os::unix::fs::symlink(root.join("real.skr"), root.join("link.skr")).unwrap();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "save", "path": "link.skr"}),
    );
    assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
    // A missing file is a file error naming the path given.
    let r = call(
        &mut h,
        "execute",
        json!({"type": "open", "path": "nope.skr"}),
    );
    assert_eq!(r["error"]["data"]["kind"], "file", "{r}");
    assert!(r["error"]["message"].as_str().unwrap().contains("nope.skr"));
}

#[test]
fn a_batch_with_a_path_outside_runs_nothing() {
    let (root, outside) = folders("batch");
    let mut h = host(&root);
    let r = call(
        &mut h,
        "batch",
        json!({"commands": [
            {"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]},
            {"type": "save", "path": outside.join("a.skr")}
        ]}),
    );
    assert_eq!(r["error"]["code"], codes::BATCH_FAILED, "{r}");
    assert_eq!(r["error"]["data"]["index"], 1);
    assert_eq!(r["error"]["data"]["completed"], json!([]));
    assert_eq!(
        call(&mut h, "query", json!({"type": "stats"}))["result"]["edges"],
        0
    );
    // Save without a path in a batch is refused under roots.
    let r = call(&mut h, "batch", json!({"commands": [{"type": "save"}]}));
    assert_eq!(r["error"]["data"]["error"]["kind"], "invalid_input", "{r}");
}

#[test]
fn material_libraries_outside_the_roots_are_not_read() {
    let (root, outside) = folders("mtllib");
    std::fs::write(outside.join("paint.mtl"), "newmtl red\nKd 1 0 0\n").unwrap();
    let obj = "mtllib ../outside/paint.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    std::fs::write(root.join("tri.obj"), obj).unwrap();
    let mut limited = host(&root);
    call(
        &mut limited,
        "execute",
        json!({"type": "import", "path": "tri.obj"}),
    );
    let m = call(&mut limited, "query", json!({"type": "materials"}));
    assert_eq!(m["result"]["materials"], json!([]), "{m}");
    // Without roots the library is read, as before.
    let mut free = HeadlessHost::new();
    call(
        &mut free,
        "execute",
        json!({"type": "import", "path": root.join("tri.obj")}),
    );
    let m = call(&mut free, "query", json!({"type": "materials"}));
    assert_eq!(m["result"]["materials"].as_array().unwrap().len(), 1, "{m}");
}

#[test]
fn without_roots_the_file_query_says_null() {
    let mut h = HeadlessHost::new();
    let file = call(&mut h, "query", json!({"type": "file"}));
    assert_eq!(file["result"]["api_roots"], Value::Null);
    assert!(ApiRoots::new(&[PathBuf::from("/no/such/folder")]).is_err());
}

#[test]
fn a_root_folder_itself_is_not_a_file_to_write_or_read() {
    let (root, outside) = folders("rootself");
    let base = root.parent().unwrap().to_path_buf();
    let mut h = host(&root);
    call(
        &mut h,
        "execute",
        json!({"type": "add_material", "name": "red", "color": [255, 0, 0]}),
    );
    edge(&mut h);
    std::fs::write(base.join("root.mtl"), "keep me").unwrap();
    let real = session::fspath::canonical(&root).unwrap();
    for path in [real.display().to_string(), "../root".to_string()] {
        let r = call(
            &mut h,
            "execute",
            json!({"type": "export", "path": path, "format": "obj", "overwrite": true}),
        );
        assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
        let r = call(
            &mut h,
            "execute",
            json!({"type": "save", "path": path, "overwrite": true}),
        );
        assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
        let r = call(&mut h, "execute", json!({"type": "open", "path": path}));
        assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
    }
    assert_eq!(
        std::fs::read_to_string(base.join("root.mtl")).unwrap(),
        "keep me"
    );
    assert!(!base.join("root.partial").exists() && outside.exists());
}

/// A 1x1 PNG at `path`.
fn png_file(path: &std::path::Path) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, 1, 1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&[200, 0, 0, 255]).unwrap();
}

/// A rectangle painted with a textured material named "T".
#[cfg(unix)]
fn textured_face(h: &mut HeadlessHost) {
    call(
        h,
        "execute",
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    );
    let r = call(
        h,
        "execute",
        json!({"type": "add_material", "name": "T", "texture": {"path": "tex.png"}}),
    );
    let material = r["result"]["changes"]["materials"][0].clone();
    let faces = call(h, "query", json!({"type": "entities", "kinds": ["face"]}));
    let face = faces["result"]["faces"][0]["id"].clone();
    let r = call(
        h,
        "execute",
        json!({"type": "paint", "ids": [face], "material": material}),
    );
    assert!(r["result"].is_object(), "{r}");
}

#[cfg(unix)]
#[test]
fn obj_side_files_that_are_links_are_refused_and_their_targets_not_read() {
    let (root, outside) = folders("side-links");
    let mut h = host(&root);
    png_file(&root.join("tex.png"));
    textured_face(&mut h);
    // Targets of the right format: a library and an image outside.
    std::fs::write(outside.join("victim.mtl"), "newmtl a\nKd 1 1 1\n").unwrap();
    png_file(&outside.join("victim.png"));
    let before = std::fs::read(outside.join("victim.png")).unwrap();
    for (link, target) in [
        ("exp.mtl", outside.join("victim.mtl")),
        ("exp_T.png", outside.join("victim.png")),
        ("exp.mtl", outside.join("missing.mtl")),
    ] {
        let _ = std::fs::remove_file(root.join("exp.mtl"));
        let _ = std::fs::remove_file(root.join("exp_T.png"));
        std::os::unix::fs::symlink(&target, root.join(link)).unwrap();
        for overwrite in [false, true] {
            let r = call(
                &mut h,
                "execute",
                json!({"type": "export", "path": "exp.obj", "overwrite": overwrite}),
            );
            assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{link}: {r}");
            let message = r["error"]["message"].as_str().unwrap();
            assert!(
                message.contains(link) && message.contains("symbolic link"),
                "{message}"
            );
            let meta = std::fs::symlink_metadata(root.join(link)).unwrap();
            assert!(meta.file_type().is_symlink(), "the link is left as it is");
        }
    }
    assert!(!root.join("exp.obj").exists());
    assert!(!outside.join("missing.mtl").exists());
    assert_eq!(std::fs::read(outside.join("victim.png")).unwrap(), before);
    // With the links gone the export writes all three files.
    std::fs::remove_file(root.join("exp.mtl")).unwrap();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "export", "path": "exp.obj"}),
    );
    assert!(r["result"].is_object(), "{r}");
    assert!(root.join("exp.mtl").is_file() && root.join("exp_T.png").is_file());
}

#[cfg(unix)]
#[test]
fn outside_paths_get_one_answer_whether_they_exist_or_not() {
    let (root, outside) = folders("oracle");
    let mut h = host(&root);
    edge(&mut h);
    std::fs::write(outside.join("secret.skr"), "x").unwrap();
    std::fs::create_dir(outside.join("locked")).unwrap();
    std::fs::write(outside.join("locked/in.obj"), "v 0 0 0\n").unwrap();
    std::fs::create_dir(outside.join("exists")).unwrap();
    std::fs::create_dir(outside.join("made")).unwrap();
    std::os::unix::fs::symlink(outside.join("nodir"), root.join("dangling")).unwrap();
    std::os::unix::fs::symlink(outside.join("made"), root.join("dangling-made")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        outside.join("locked"),
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    let paths = [
        outside.join("secret.skr"),
        outside.join("nope.skr"),
        outside.join("nodir/nope.skr"),
        outside.join("locked/in.obj"),
        outside.join("locked/nope.obj"),
        // Out and back into the root through an outside folder that exists or not.
        outside.join("exists/../../root/a.skr"),
        outside.join("missing/../../root/a.skr"),
        outside.join("locked/x/../../../root/a.skr"),
        // A dangling link in the root, its target outside missing or not.
        root.join("dangling/a.skr"),
        root.join("dangling-made/a.skr"),
    ];
    let mut messages = std::collections::BTreeSet::new();
    for path in &paths {
        let p = path.display().to_string();
        for c in [
            json!({"type": "open", "path": p}),
            json!({"type": "import", "path": p}),
            json!({"type": "save", "path": p}),
            json!({"type": "export", "path": format!("{p}.stl")}),
            json!({"type": "add_material", "texture": {"path": p}}),
        ] {
            let r = call(&mut h, "execute", c.clone());
            assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{c}: {r}");
            let message = r["error"]["message"].as_str().unwrap();
            assert!(message.contains("outside the folders"), "{c}: {message}");
            let given = c["path"]
                .as_str()
                .or(c["texture"]["path"].as_str())
                .unwrap();
            messages.insert(message.replace(given, "PATH"));
        }
    }
    std::fs::set_permissions(
        outside.join("locked"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    assert_eq!(messages.len(), 1, "{messages:?}");
    // A part missing within a root is a file error, wherever the rest would lead: it says
    // nothing of what lies outside.
    for path in [
        "missing/nope.skr",
        "missing/../../outside/secret.skr",
        "missing/../../outside/nope.skr",
    ] {
        let r = call(&mut h, "execute", json!({"type": "open", "path": path}));
        assert_eq!(r["error"]["data"]["kind"], "file", "{r}");
    }
}

#[test]
fn a_root_as_the_target_is_a_folder_and_a_nul_is_invalid() {
    let (root, _) = folders("root-target");
    let mut h = host(&root);
    edge(&mut h);
    let real = root.canonicalize().unwrap().display().to_string();
    for c in [
        json!({"type": "open", "path": real}),
        json!({"type": "import", "path": real}),
        json!({"type": "save", "path": real}),
        json!({"type": "export", "path": real, "format": "stl"}),
    ] {
        let r = call(&mut h, "execute", c.clone());
        assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{c}: {r}");
        let message = r["error"]["message"].as_str().unwrap();
        assert!(message.contains("is a folder"), "{c}: {message}");
    }
    for c in [
        json!({"type": "save", "path": "a\u{0}b.skr"}),
        json!({"type": "open", "path": "a\u{0}b.skr"}),
        json!({"type": "export", "path": "a\u{0}b.stl"}),
    ] {
        let r = call(&mut h, "execute", c.clone());
        assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{c}: {r}");
        let message = r["error"]["message"].as_str().unwrap();
        assert!(message.contains("NUL"), "{c}: {message}");
    }
}

#[test]
fn a_referenced_material_keeps_the_texture_path_rewritten_under_roots() {
    let (root, _) = folders("ref-texture");
    png_file(&root.join("tex.png"));
    let mut h = host(&root);
    let r = call(
        &mut h,
        "batch",
        json!({"commands": [
            {"type": "add_material", "color": [1, 2, 3]},
            {"type": "change_material", "material": "$0:m0", "texture": {"path": "tex.png"}}
        ]}),
    );
    assert!(r["result"].is_object(), "{r}");
    let material = r["result"]["outcomes"][0]["changes"]["materials"][0].clone();
    let m = call(&mut h, "query", json!({"type": "materials"}));
    let made = m["result"]["materials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == material)
        .unwrap_or_else(|| panic!("{m}"));
    assert!(made["texture"].is_object(), "{m}");
}
