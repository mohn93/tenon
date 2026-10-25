//! Build-time facts: the git hash About shows, and (Windows packaging) the icon and version
//! resource of `app.exe`.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=SKETCHUP_ALT_GIT_HASH");
    println!("cargo:rerun-if-env-changed=SKETCHUP_ALT_WINRES");
    println!("cargo:rerun-if-changed=build.rs");

    // The packaging scripts may set the hash (a source tarball has no git); otherwise ask git.
    let hash = std::env::var("SKETCHUP_ALT_GIT_HASH").ok().or_else(|| {
        let out = Command::new("git")
            .args(["rev-parse", "--short=7", "HEAD"])
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    });
    if let Some(hash) = hash.filter(|h| !h.is_empty()) {
        println!("cargo:rustc-env=SKETCHUP_ALT_GIT_HASH={hash}");
    }

    windows_resource();
}

/// Embeds the icon and version resource when `SKETCHUP_ALT_WINRES` is set (the packaging
/// workflow); other builds skip it, so they never need the resource compiler.
#[cfg(windows)]
fn windows_resource() {
    if std::env::var_os("SKETCHUP_ALT_WINRES").is_none() {
        return;
    }
    println!("cargo:rerun-if-changed=../../packaging/icons/app.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../packaging/icons/app.ico");
    res.set("ProductName", "sketchup-alt");
    res.set("FileDescription", "sketchup-alt");
    res.compile().expect("embedding the Windows resource");
}

#[cfg(not(windows))]
fn windows_resource() {}
