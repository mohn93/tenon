# Open on Phone

File > Open on Phone... (desktop only) shows a model on a phone's browser, read only, over the
local network. The web build and the phone viewer itself do not have this item.

## How it works

1. The sheet seals a copy of the open model (the bytes a `.skr` save would write; the model is
   not marked saved) with a random key and id, using ChaCha20-Poly1305 (`session::share`). The
   sealed data is the model's name (the document's file name without its extension, or
   "Untitled"; at most 256 bytes of UTF-8) and the model, so the name is encrypted too.
2. A small HTTP server starts on this computer, on all interfaces, on port 47240 (or, when that
   is taken, a free port the OS picks), and the sheet
   shows a QR code and the link `http://<this computer's address>:<port>/#v1.<id>.<key>`.
3. The phone scans the code, loads the viewer from the server, fetches the encrypted model from
   `/s/<id>` and decrypts it in the browser with the key from the link.

## What is sent

- The viewer's own files (the web build: `index.html`, `web.js`, `web_bg.wasm`) and the one
  encrypted model, nothing else. The server answers only `GET` and `HEAD`, has no directory
  listing, and answers 404 to any other path. To a browser that takes gzip, `web.js` and
  `web_bg.wasm` go as the gzip copies the build makes (`web_bg.wasm` is about 18 MB, 6 MB
  gzipped). Viewer files carry an `ETag` and `Cache-Control: no-cache`, so a phone that opens
  another share revalidates them instead of downloading them again; the model is `no-store`.
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
3. Fits the model to the screen.

Before any of that (in `index.html`, before the module downloads, handing the link to the module through a global it reads once and deletes), as soon as the page's fragment starts `v1.` (valid or not), the fragment and
any query are removed from the address bar and the history entry (`history.replaceState`), so
the key is not left in the browser's address, history or a bookmark made afterwards, whatever
happens next (a wrong key, an unreachable computer). The link is kept in memory only, so after a
failure the link must be opened again. The key is never logged. Each request gives up after 20
seconds without an answer or a next piece of the model ("Could not reach..."). Opening another
share link in the same tab reloads the page, which starts the viewer for it.

Errors are shown as a message instead of the viewer: "This link is damaged or expired" (the
link is malformed, the key is wrong, the blob is damaged or too big, or the share has stopped),
"Could not reach the computer sharing this model" (the request failed: the computer is off the
network or the share stopped and the page was reloaded).

The screen is the 3D view with a small top bar showing the model's name (from the share) and
the buttons Parts, Measure and Recent. There are no menus, tools, panels, status line or keyboard
shortcuts, and the model cannot be changed: the session refuses every command that changes the
model (editing, undo and redo, new, open, import) and saving, whatever asked, and only the
Select tool can be chosen. Export commands are not refused (the viewer has no menu to start
them).

### Parts

A part is a top-level group or component instance of the model (what the root places); what is
nested inside a part belongs to it and is not listed or counted. Tapping a part selects it and
shows a card over the bottom of the view:

- the name: the group's name, or the component's definition name ("Group" and "Component" when
  unnamed);
- the size, "width x depth x height": the axis-aligned box, in the model's axes, around the
  geometry the part places (its vertices taken through the part's own transform and those of
  everything nested in it, so a turned, scaled or mirrored part is measured as it stands, not
  by its own box turned), written in the document's units. Geometry on hidden tags is left
  out, and so are parts on hidden tags;
- for a component, "1 of N": which of the model's top-level instances of that component it is
  (in id order) and how many there are;
- Isolate.

The Parts button opens a list, every part with its size; a component's instances are one row
with their count ("Crate x3"). Rows are at least 44 points tall. Tapping a row selects its parts
and frames them, and closes the list.

Isolate hides every other part, for looking only: nothing is saved or undone and the model
is not marked changed (its own bytes are the same), and what is hidden is hidden from drawing, picking,
snapping and Zoom Extents. While parts are hidden the card shows Show all instead (and a bar
with Show all stays when nothing is selected). Selecting a row of the parts list shows
everything again first. Opening another model starts with everything shown.

### Measure

The Measure button turns Measure on: a strip under the bar says "Tap the first point". A tap
picks the nearest corner, edge middle, point on an edge or point on a face within 24 logical
pixels (24 points times the display scale, where a mouse snaps at 8); a tap on nothing is
ignored. After the second point the strip shows the distance in the document's units, with a
cross at each point and a line between them over the view. Another tap starts a new
measurement (a quick second tap counts as a point, not a zoom). Done (or the Measure button)
leaves; taps select parts again.

### Recent

Recent is opt-in per model. Nothing is kept when a share opens; the part card, and the top of
the Recent list, offer "Keep on this phone" for the open model (with a line saying that a kept
model, key included, can be read by pages served at this address). Only then is it stored. The
Recent button lists the kept models (last 5; name, when, size); a tap opens one, Delete forgets
one. They live in the browser's IndexedDB: database `sketchup-alt-viewer`, object store `recent`
(`id`, `key`, `name`, `time`, `blob`), and a small `recent-info` store (`id`, `name`, `time`,
`size`) so the list does not read every blob. The blob is the encrypted share as the server
sent it. The store opens in the background after the viewer starts (and is given up on after
3 seconds), so a slow browser does not delay the model.

What this is and is not:

- The kept model (blob and key) is readable by any page later served from the same
  `http://<address>:47240` on any network the phone joins, because the browser keeps IndexedDB
  per origin and a plain-http LAN address is not private to this computer. Keep only models you
  would let such a page read. The app tries port 47240 first for every share so the origin is
  the same from one share to the next (if that port is taken another is used and Recent starts
  empty there).
- The viewer page is served by the computer, so Recent works while a computer is sharing, after
  opening that share's link (any share, not only the one that made the entry). It does not work
  with the computer off or sharing nothing: a page served over plain `http` on a local network
  cannot install itself for offline use.
- Reloading the phone page starts the editor, not the viewer: the share link is taken out of
  the address bar as the page loads, so the reload has no link. Open the link again, or open a
  kept model from Recent.
- The page removes the link's `#...` part from the address bar and the tab's history entry
  (`history.replaceState`) before the viewer's code downloads, but the browser's history list
  or a QR scanner app may still have recorded the original link. The link stops working once
  the share stops.
- The Copy button in the Open on Phone sheet puts the full link, key included, on the clipboard.
- Delete removes a kept model's blob and key. A model over 64 MiB is not kept and the list says
  so. Where the browser blocks IndexedDB (some private windows) nothing is kept and the list
  says that; when the storage is full the list says the model could not be kept, and the older
  ones stay (the oldest beyond 5 are removed only after the new one is safely written).
  Escape (or Done) leaves Measure; Escape also closes a list.

### Touch

| Gesture | Does |
| --- | --- |
| One finger drag | Orbit about the point under the finger where the drag began |
| Two finger drag | Pan (by the midpoint) |
| Pinch | Zoom (by the ratio of the distance between the fingers), toward the midpoint |
| Tap | Select the part (top-level group or component) under the finger and show its card; tap empty space to clear. In Measure, pick a point |
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
After Stop in the sheet, opening the link again says "This link is damaged or expired".

There is no agent API for sharing in this version.
