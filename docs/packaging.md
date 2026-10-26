# Packaging

The product is named **sketchup-alt**; the program is `app` (`app.exe` on Windows) in every package.
The macOS bundle id is `io.github.mohn93.sketchup-alt`. The version is the workspace version in
`Cargo.toml`; About and Help > Check for Updates show it, with the short git hash of the commit
the program was built from (`SKETCHUP_ALT_GIT_HASH` overrides it, for a build without git).

CI builds every package in `.github/workflows/package.yml`, on a push to `main` or by hand
(Actions, Package, Run workflow). It uploads workflow artifacts (kept 30 days) and never creates
a release or a tag. `ci.yml` is separate; its own `package-windows` job still makes the plain
Windows zip.

| Artifact | Contents |
|---|---|
| `sketchup-alt-macos-dmg` | `sketchup-alt-<version>-macos-universal.dmg`: `sketchup-alt.app` (aarch64 and x86_64, macOS 11 or later, ad-hoc signed) and a link to Applications |
| `sketchup-alt-windows` | `sketchup-alt-<version>-windows-x86_64.msi` (per-machine, Start menu shortcut, `.skr` association, icon) and the `.zip` with `app.exe`; both unsigned |
| `sketchup-alt-linux-x86_64`, `sketchup-alt-linux-aarch64` | `sketchup-alt-<version>-linux-<arch>.AppImage` and `.tar.gz` (program, `.desktop` file, icon, notices) |

## Build locally

**macOS** (on a Mac; opens no window, installs nothing outside `target/`):

```bash
scripts/package-macos.sh                # this Mac's architecture
UNIVERSAL=1 scripts/package-macos.sh    # both; needs rustup targets aarch64-apple-darwin and x86_64-apple-darwin
```

The result is `target/package/macos/sketchup-alt-<version>-macos-<arch>.dmg`. The first start of
an ad-hoc signed app is refused by Gatekeeper: right-click the app, Open, Open.

**Linux**:

```bash
scripts/package-linux.sh                                  # the tarball
APPIMAGETOOL=/path/to/appimagetool-x86_64.AppImage scripts/package-linux.sh   # and the AppImage
```

CI uses appimagetool 1.9.0 and the AppImage runtime of type2-runtime 20251108, checks both
SHA-256 digests (set in `package.yml`) before use, and passes the runtime as
`APPIMAGE_RUNTIME` (appimagetool would otherwise download the unpinned latest one). The
AppImage holds only the program, as the tarball does: the graphics driver and window libraries
come from the system (see [linux.md](linux.md)).

**Windows** (PowerShell, with the Rust toolchain and the .NET SDK):

```powershell
$env:RUSTFLAGS = "-C target-feature=+crt-static"; $env:SKETCHUP_ALT_WINRES = "1"
cargo build --release --locked -p app
dotnet tool install --global wix --version 4.0.5
wix build -arch x64 -o sketchup-alt.msi -d Version=0.1.0 -d ExePath=target/release/app.exe `
  -d IcoPath=packaging/icons/app.ico -d NoticesPath=THIRD_PARTY_NOTICES.md packaging/windows/sketchup-alt.wxs
```

`SKETCHUP_ALT_WINRES` embeds the icon and version resource; without it `app.exe` has none and
the build needs no resource compiler.

**Icons**: `scripts/make-icons.py` draws the icon from vector shapes (original artwork) and
writes `packaging/icons/` (`app.icns`, `app.ico`, PNGs). The outputs are committed.

## Help > Check for Updates

On desktops only (the browser build hides it). Choosing it asks
`https://api.github.com/repos/mohn93/sketchup/releases/latest` once and shows "Version X is
available" with a button that opens the release page in the browser, "You're up to date", "No
releases published yet" (no release exists), or the error. The app makes no request at start or
in the background, and never downloads or installs anything. The new HTTP dependency is `ureq`
(MIT or Apache-2.0) with rustls, used only for this.

## Opening a .skr from the desktop

Windows (the MSI's association) and Linux (`Exec=app %f`) pass the file as the program's
argument. macOS never does: Finder (double-click, Open With) and the Dock icon (a file dropped
on it) send it to the app delegate's `application:openURLs:`. `crates/app/src/open_files.rs`
adds that method to winit's delegate class before the event loop runs (as `system_quit.rs`
adds `applicationShouldTerminate:`) and sends the paths as `AppEvent::OpenFiles`. The first
file is opened as File > Open opens one, asking about unsaved changes first; one that arrives
while the app is still starting is opened once the window exists, in place of a file given as
an argument. It also registers `NSTreatUnknownArgumentsAsOpen` as NO, so AppKit does not take
the command line's own arguments (a model, the folder after `--api-root`) for files to open.

## What the user must provide later

Nothing below is wired up; the workflow runs without any secret.

**macOS Developer ID signing and notarization** (Apple Developer Program account):

1. A "Developer ID Application" certificate, exported with its private key as a `.p12` file with
   a password.
2. An App Store Connect API key (an `.p8` file, its key id and issuer id) for `notarytool`.
3. Repository secrets, for example `MACOS_CERTIFICATE` (the `.p12`, base64), `MACOS_CERTIFICATE_PASSWORD`,
   `APPLE_API_KEY` (the `.p8`, base64), `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID`, and the signing
   identity's name.
4. Then, in the macOS job: import the certificate into a temporary keychain; replace
   `codesign --force --deep -s -` in `scripts/package-macos.sh` by `codesign --force --options runtime
   --timestamp -s "<identity>"` on the app (hardened runtime; the app needs no entitlements as far
   as is known, to be confirmed on a signed build); sign the `.dmg` too; run
   `xcrun notarytool submit <dmg> --key ... --key-id ... --issuer ... --wait`, then
   `xcrun stapler staple <dmg>`.

**Windows code signing**: a code signing certificate (an OV or EV certificate as a `.pfx` with a
password, or a cloud signing service such as Azure Trusted Signing with its account and
certificate profile). Store them as secrets, then sign `app.exe` before the MSI is built and the
`.msi` after, with `signtool sign /fd SHA256 /tr <timestamp url> /td SHA256 ...`. Without it,
SmartScreen warns on the first start.

**Linux**: AppImages can be signed with a GPG key (`appimagetool --sign`); not set up.

## Licence notices

`THIRD_PARTY_NOTICES.md` ships in every package (the dmg's app bundle, the zip, the MSI, the Linux
tarball and the AppImage) and in `web/dist`. Its last section is generated by
`scripts/licenses.py` from `Cargo.lock`: every crate reachable through normal dependencies from
`app` on the five packaged targets or from `web` on wasm32, with its licence expression and the
licence texts its package carries (identical texts once). Run the script after changing
`Cargo.lock` and commit the result; `scripts/licenses.py --check` (a CI step) fails when the file
is stale, a crate has no licence expression or text, or a licence is not on the script's
`ALLOWED` list (permissive licences, the Unicode and CDLA-Permissive-2.0 data licences, the
font licences, and MPL-2.0 for unmodified crates). A few crates publish no licence file; their
texts are copied from upstream into `packaging/licenses/`.

## Known limits

- The Windows and Linux packages build only in CI; the macOS dmg has been built and verified
  locally for this Mac's architecture only (the universal build runs in CI).
