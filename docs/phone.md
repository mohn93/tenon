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

## The viewer (what the phone shows)

The web build starts the viewer instead of the editor when the page's address has a share in its
fragment (`#v1.<id>.<key>`). It:

1. Fetches `/s/<id>` from the server that served the page. Only the id is requested; the key
   never leaves the browser. A response that says it is over 256 MiB is refused before any of
   it is read, and no more than 256 MiB is ever collected. The request is not cached and sends
   no cookies.
2. Decrypts it with the key (a wrong key, a changed or cut-off blob is refused by the
   authentication tag) and opens the model read only.
3. Removes the fragment (and any query) from the address bar and the history entry
   (`history.replaceState`), so the key is not left in the browser's address, history or a
   bookmark made afterwards. The key is never logged.
4. Fits the model to the screen.

Errors are shown as a message instead of the viewer: "This link is damaged or expired" (the
link is malformed, the key is wrong, the blob is damaged or too big, or the share has stopped),
"Could not reach the computer sharing this model" (the request failed: the computer is off the
network or the share stopped and the page was reloaded).

The screen is the 3D view with a small top bar showing the model's name (the link carries no
name, so it reads "Shared model"). There are no menus, tools, panels, status line or keyboard
shortcuts, and the model cannot be changed: the session refuses every command that changes the
model (editing, undo and redo, new, open, import, save), whatever asked, and only the Select
tool can be chosen.

### Touch

| Gesture | Does |
| --- | --- |
| One finger drag | Orbit about the point under the finger where the drag began |
| Two finger drag | Pan (by the midpoint) |
| Pinch | Zoom (by the ratio of the distance between the fingers), toward the midpoint |
| Tap | Select the part (top-level group or component) under the finger; tap empty space to clear |
| Double tap | Zoom to the tapped part, or to the whole model on empty space |

A tap is a touch shorter than 300 ms that moves at most 10 points; a double tap is two taps
within 300 ms and 30 points. Lifting one finger of a pair does not start an orbit with the
other. Touches that begin over the top bar or a window belong to it, not to the view. With a
mouse (desktop testing) the left button does the same as one finger (click selects, drag
orbits, double click zooms), the wheel zooms, and the middle button orbits (Shift pans).

Phones without WebGPU (plain `http://` on a local address has none) use WebGL2: light shadows
are off there, as in the web build.

### Trying it by hand

Run the desktop app, File > Open on Phone..., and open the shown URL in a phone's browser (or
in a browser tab with a phone-sized window, device toolbar on, 375 x 812 at 3x). Check: the model
appears fitted; the address bar loses the `#...` part; reloading the page then starts the
editor, not the viewer; orbit, pan, pinch, tap and double tap work; no editing is reachable.
After Stop in the sheet, a new request for the link says "This link is damaged or expired".

There is no agent API for sharing in this version.
