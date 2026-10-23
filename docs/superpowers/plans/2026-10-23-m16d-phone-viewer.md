# M16d Phone Viewer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** "Shop Mode": File > Open on Phone shows a QR code; a phone on the same Wi-Fi scans it and opens a read-only, touch-first viewer of the model (orbit, tap a part for its size, a parts list, measure, isolate a part, recent models kept offline). The model travels end-to-end encrypted: the key is only in the QR code's URL fragment, never sent over the network.

**Architecture:** No relay and no hosting (user decision 2026-10-22): the desktop app runs a small local-network HTTP server while the Open on Phone sheet is open. It serves the web build's files (the same `web` crate, which starts in viewer mode when the URL fragment carries a share) and one encrypted blob of the model. The QR code encodes `http://<LAN address>:<port>/#v1.<blob id>.<key>`; the fragment is never sent to the server. The blob is the model's `.skr` bytes (session `save_bytes` of a copy) sealed with ChaCha20-Poly1305 (`chacha20poly1305` crate) under a random 256-bit key and 96-bit nonce. The phone page fetches the blob, opens it with the key, and shows it read-only. Phone browsers block WebGPU and WebCrypto on plain `http` to a LAN address, so the phone viewer uses WebGL2 (shadows off) and the Rust AEAD in wasm.

**Tech Stack:** Rust 2024, egui 0.36.2, wgpu 30 (WebGL2 on the phone). New dependencies approved by the user (2026-10-22): `chacha20poly1305 =0.11.0` (default features off; `alloc`, plus `getrandom` natively for keys and nonces) and `qrcode =0.14.1` (default features off: the QR matrix only, drawn with egui). Their dependencies are new packages in `Cargo.lock`; nothing else may be added.

**Spec:** `docs/superpowers/roadmap.md` row M16d; user decisions 2026-10-22 (LAN only, the two crates above).

## Global Constraints

- Only the two crates above (and their own dependencies) may be added; record the new packages in the report. All crates involved must be MIT/Apache-2.0 compatible; list them in `THIRD_PARTY_NOTICES.md`.
- Never write our own cryptographic primitives; only use the crate's AEAD. Keys and nonces come from the OS random source (`getrandom`) natively; the phone only decrypts.
- The LAN server only ever serves: the viewer's static files (from one directory) and the one current blob by its random id; nothing else (no directory listing, no path traversal, GET/HEAD only, responses capped, at most 16 concurrent connections, idle timeout 10 s). It binds only while the Open on Phone sheet is open (and at most 30 minutes), and stops when the sheet closes or the model changes (the sheet then offers to share again).
- Native behaviour otherwise unchanged; golden images unchanged; the desktop web build unchanged except the viewer mode it gains.
- Gate per task (project rules in the SDD rules files): native gate + `cargo check --target wasm32-unknown-unknown -p kernel -p session -p tools -p render -p api -p app --lib -p web` + `scripts/build-web.sh` from Task 3 on.
- Docs (`docs/web.md`, a new `docs/phone.md`) updated in the task that changes behaviour. No emoji. Short commit messages. Never `git add -A`.

## Spec notes (rulings)

1. **Share envelope** (`session::share`): `seal(model_bytes) -> Share { id: [u8;16], key: [u8;32], blob: Vec<u8> }` where `blob = b"SKAS1" || nonce(12) || ciphertext+tag`, AAD = `b"SKAS1" || id`; `open(blob, id, key) -> Result<Vec<u8>, CommandError>`; the link fragment is `v1.<base64url(id)>.<base64url(key)>` (no padding); parse refuses anything else. Blob size cap 256 MiB (as `.skr`).
2. **QR:** `qrcode` with error correction M, the matrix drawn as egui rectangles with a 4-module quiet zone, at least 4 px per module; the URL is also shown as text with a Copy button.
3. **LAN server** (app, native-only `share_server` module): binds `0.0.0.0:<free port>` (OS-chosen) only while sharing; the advertised address is the machine's primary LAN IPv4 (found with a UDP "connect" to a non-routable address, no packets sent); if none is found the sheet says "Connect this computer to Wi-Fi to share". Routes: `GET /` -> `index.html`; `GET /<file>` for files in the viewer directory (names `[A-Za-z0-9._-]+` only); `GET /s/<blob id hex>` -> the blob (`application/octet-stream`, `Cache-Control: no-store`); everything else 404. MIME: `.wasm` `application/wasm`, `.js` `text/javascript`, `.html` `text/html`. Viewer directory: the `viewer` folder next to the executable, else `$SKETCHUP_ALT_VIEWER`, else `web/dist` of the source tree in development; if none has `index.html`, the sheet says "The phone viewer is not built: run scripts/build-web.sh".
4. **Open on Phone (window):** File > Open on Phone... (no key) opens a sheet: QR code, URL, "Waiting for the phone" / "Sent to 1 phone" (counts blob downloads), the time left, Stop. It seals a copy of the current model when opened; editing the model while it is open marks the share stale and the sheet offers "Share the new version". Agents: none in this milestone (documented).
5. **Viewer mode (web crate):** when the URL fragment starts with `v1.`, the page starts the viewer instead of the editor: fetches `/s/<id>`, opens it with the key (a wrong key or damaged blob shows "This link is damaged or expired"), opens the model read-only (`open_bytes`), removes the fragment from the address bar (history.replaceState) so the key is not left in browser history. Read-only: no drawing or editing tools, no menus except a small top bar (model name, Parts, Measure, Recent); Undo/edit commands unreachable.
6. **Touch:** one finger drag = orbit; two fingers = pan (midpoint) and pinch zoom; double tap = zoom to the tapped part (or extents on empty space); tap = select a part. Mouse works too (desktop testing).
7. **Parts:** a part is a top-level group or component instance of the model (root). Tapping one highlights it and shows a card: its name (definition or group name), its size (width x depth x height of its axis-aligned box in model axes, in the model's display units), and for components "1 of N". Parts list: every part with its size, components grouped with a count; tapping a row selects and frames it. Isolate: from the card, "Isolate" hides every other part (view-only, not saved); "Show all" restores.
8. **Measure:** Measure mode: tap a first point and a second point on the model (snapping to vertices and edges as the inference engine does, with a 24 px touch radius); the distance shows in display units; tap again to start a new measurement; Esc/Done leaves.
9. **Recent:** the viewer keeps the last 5 opened shares (encrypted blob + id + key + name + time) in IndexedDB (database `sketchup-alt-viewer`); the Recent list reopens them offline; Delete removes one. A share larger than 64 MiB is not kept (the list says why).

## Review Focus

1. The key never leaves the phone: not in any request (only the fragment holds it), removed from the address bar after load, never logged; the server never sees it.
2. The LAN server cannot be made to serve anything outside the viewer directory or any blob but the current one (path traversal, encoded slashes, long paths, HEAD/POST, slowloris idle connections), and it stops when the sheet closes, after 30 minutes, and when the app quits.
3. A tampered or truncated blob, or a wrong key, is refused (AEAD tag) with the damaged-link message; nothing panics.
4. Touch gestures on a phone-sized canvas (375 x 812, devicePixelRatio 3): orbit, pan, pinch, tap and double tap do what ruling 6 says, and no edit is reachable.
5. Part sizes are right for rotated and scaled instances (box in the model's axes of the placed geometry) and in the display units.

---

### Task 1: Share envelope, link and QR in the session

**Files:** workspace `Cargo.toml` (the two crates), `crates/session/{Cargo.toml, src/share.rs (new), src/lib.rs}`, `THIRD_PARTY_NOTICES.md`; tests `crates/session/tests/integration/share.rs`.

- [ ] **Step 1: Write the failing tests** for ruling 1 (seal/open round trip; tampered byte, truncated blob, wrong key, wrong id, bad prefix refused; link encode/parse round trip and refusals) and a QR matrix helper (`share::qr_modules(url) -> (width, Vec<bool>)`) round-trip sanity (size for a typical URL, quiet zone not included).
- [ ] **Step 2-5:** run, implement, gate (+ wasm check: session must still build for wasm32 with chacha20poly1305; getrandom only natively), commit `git commit -m "Share envelope, link and QR"`.

### Task 2: The LAN server and Open on Phone in the window

**Files:** `crates/app/src/{share_server.rs (new, native-only), ui/sheets.rs (Open on Phone sheet), menus.rs, app.rs, platform.rs (Features: share = native only)}`, `docs/phone.md` (new); tests: server unit/integration tests against a local client (routes, MIME, 404s, traversal, HEAD, size cap, concurrency cap, idle timeout, stop), LAN address picking (pure function over candidate addresses), sheet UI tests.

- [ ] **Step 1: Write the failing tests** for rulings 2-4 and Review Focus 2.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Open on Phone: local server and QR sheet"`.

### Task 3: Viewer mode: load, decrypt, read-only, touch

**Files:** `crates/web/src/{viewer.rs (new), lib.rs, start.rs, platform.rs}`, `crates/app/src/{platform.rs (Features::VIEWER), lib.rs entry for the viewer, editor touch handling or a viewer input module}`, `docs/web.md`, `docs/phone.md`; native unit tests for the fragment parsing, gesture recognition (pure state machine over touch events) and read-only feature flags.

- [ ] **Step 1: Write the failing tests** for rulings 5-6 and Review Focus 1, 3, 4 (pure parts natively).
- [ ] **Step 2-5:** run, implement, gate (+ wasm check + build), commit `git commit -m "Phone viewer: load, decrypt and touch"`. The controller then smoke-tests in Chrome with a phone-sized viewport over `http://<LAN IP>` from the running desktop app.

### Task 4: Parts, measure, isolate and recent

**Files:** `crates/app/src/{viewer/ (new: parts.rs, measure.rs, isolate.rs) or editor modules}`, `crates/web/src/{recent.rs (IndexedDB), viewer.rs}`, `crates/session` only if a part-size helper belongs there, docs; tests natively for part listing and sizes (Review Focus 5), measure snapping, isolate, recent list logic (storage behind a trait).

- [ ] **Step 1: Write the failing tests** for rulings 7-9 and Review Focus 5.
- [ ] **Step 2-5:** run, implement, gate (+ wasm check + build), commit `git commit -m "Phone viewer: parts, measure, isolate and recent"`.

### After the tasks (controller)

- Browser smoke test (phone viewport, LAN URL from the running app); push `dev`; CI green; roadmap row M16d done; spec bullet; memory; merge and push; next milestone (M14a).
