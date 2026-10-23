# Open on Phone

File > Open on Phone... (desktop only) shows a model on a phone's browser, read only, over the
local network. The web build and the phone viewer itself do not have this item.

## How it works

1. The sheet seals a copy of the open model (the bytes a `.skr` save would write; the model is
   not marked saved) with a random key and id, using ChaCha20-Poly1305 (`session::share`).
2. A small HTTP server starts on this computer, on a free port of all interfaces, and the sheet
   shows a QR code and the link `http://<this computer's address>:<port>/#v1.<id>.<key>`.
3. The phone scans the code, loads the viewer from the server, fetches the encrypted model from
   `/s/<id>` and decrypts it in the browser with the key from the link.

## What is sent

- The viewer's own files (the web build: `index.html`, `web.js`, `web_bg.wasm`) and the one
  encrypted model, nothing else. The server answers only `GET` and `HEAD`, has no directory
  listing, and answers 404 to any other path.
- The key is in the link's fragment (after `#`). Browsers never send the fragment, so the
  server, and anything else on the network, sees only the ciphertext. Anyone who gets the
  whole link (for example by seeing the QR code) can open the model while the share runs.
- Traffic is plain HTTP on the local network; the encryption of the model is what protects it.
- Limits: 16 connections at once, 10 seconds idle (no progress), 2 minutes at most per
  connection, the request head (line and headers) capped at 8 KiB and the request line alone
  at 2 KiB. Stopping the share also ends downloads in progress.

## When it runs

The server runs only while the Open on Phone sheet is open. It stops when the sheet closes
(Stop, Escape), after 30 minutes, when the app quits, and when the share is replaced. If the
model changes while the sheet is open (an agent edit, for example), the sheet says so and
offers Share the new version, which makes a new link and code and stops the old one.

## Requirements

- The computer and the phone must be on the same Wi-Fi (or network) and the network must let
  them talk to each other (guest networks often do not). The address in the link is the one
  the computer would send from by default; with a VPN that routes everything, that may be an
  address the phone cannot reach (turn the VPN off or use split tunnelling). With no network address the sheet
  says "Connect this computer to Wi-Fi to share". A firewall may ask to allow incoming
  connections the first time.
- The viewer must be built: run `scripts/build-web.sh`. The app looks for the viewer in a
  `viewer` folder next to the executable, then in `$SKETCHUP_ALT_VIEWER`, then in the source
  tree's `web/dist`. Otherwise the sheet says "The phone viewer is not built: run
  scripts/build-web.sh".

There is no agent API for sharing in this version.
