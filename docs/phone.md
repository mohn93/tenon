# Open on Phone

File > Open on Phone... (desktop only) shows a model on a phone's browser, read only, over the
local network. The web build and the phone viewer itself do not have this item.

## How it works

1. The sheet seals a copy of the open model (the bytes a `.skr` save would write; the model is
   not marked saved) with a random key and id, using ChaCha20-Poly1305 (`session::share`). The
   sealed data is the model's name (the document's file name without its extension, or
   "Untitled"; at most 256 bytes of UTF-8) and the model, so the name is encrypted too.
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
3. Fits the model to the screen.

Before any of that, as soon as the page's fragment starts `v1.` (valid or not), the fragment and
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

Isolate hides every other part, for looking only: nothing is saved, undone or marked changed
(the model's own bytes are the same), and what is hidden is hidden from drawing, picking,
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

The viewer keeps the last 5 shared models it opened on the phone, so they open again with no
computer: the Recent button lists them (name, when, size), a tap opens one, Delete forgets one.
They live in the browser's IndexedDB: database `sketchup-alt-viewer`, object store `recent`
(`id`, `key`, `name`, `time`, `blob`), and a small `recent-info` store (`id`, `name`, `time`,
`size`) so the list does not read every blob. The blob is the encrypted share as the server sent
it.

The key is kept beside the blob. That is deliberate: a model that opens offline must be able to
decrypt itself. The encryption protects the model on the network and on the computer's server;
on the phone the kept copy is as private as the browser's own storage (anyone with the unlocked
phone and the browser's data can read it). Delete removes both. A share over 64 MiB is not kept
and the Recent list says so. Where the browser blocks IndexedDB (some private windows) nothing
is kept and the list says that.

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
