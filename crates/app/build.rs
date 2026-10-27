//! Build-time facts: the git hash About shows, and (Windows packaging) the icon and version
//! resource of `app.exe`.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=TENON_GIT_HASH");
    println!("cargo:rerun-if-env-changed=TENON_WINRES");
    println!("cargo:rerun-if-changed=build.rs");

    // The packaging scripts may set the hash (a source tarball has no git); otherwise ask git.
    let hash = std::env::var("TENON_GIT_HASH").ok().or_else(|| {
        let out = Command::new("git")
            .args(["rev-parse", "--short=7", "HEAD"])
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    });
    if let Some(hash) = hash.filter(|h| !h.is_empty()) {
        println!("cargo:rustc-env=TENON_GIT_HASH={hash}");
    }
    // `rerun-if-changed=build.rs` turns off rerunning on every change, so a new commit would
    // keep the old hash (a cached target in CI too): watch what moves with the commit.
    for file in git_files() {
        println!("cargo:rerun-if-changed={}", file.display());
    }

    windows_resource();
}

/// The files that change with the checked-out commit: HEAD, the branch it names and the packed
/// refs (those that exist; a missing one would rerun the script on every build).
fn git_files() -> Vec<std::path::PathBuf> {
    let git = |args: &[&str]| {
        let out = Command::new("git").args(args).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let mut names = vec!["HEAD".to_string(), "packed-refs".to_string()];
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
        names.push(branch);
    }
    names
        .iter()
        .filter_map(|name| git(&["rev-parse", "--path-format=absolute", "--git-path", name]))
        .map(std::path::PathBuf::from)
        .filter(|path| path.exists())
        .collect()
}

/// Embeds the icon and version resource when `TENON_WINRES` is set (the packaging
/// workflow); other builds skip it, so they never need the resource compiler.
#[cfg(windows)]
fn windows_resource() {
    if std::env::var_os("TENON_WINRES").is_none() {
        return;
    }
    println!("cargo:rerun-if-changed=../../packaging/icons/app.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../packaging/icons/app.ico");
    res.set("ProductName", "tenon");
    res.set("FileDescription", "tenon");
    res.compile().expect("embedding the Windows resource");
}

#[cfg(not(windows))]
fn windows_resource() {}
