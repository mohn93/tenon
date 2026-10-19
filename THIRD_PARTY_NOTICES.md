# Third-party notices

## skp (vendored from openskp-rs)

`crates/skp` is the `core/skpcore` reader of openskp-rs
(https://github.com/hakkokimkr/openskp-rs, commit
`dee12a0b43a90ad67e6407e4f9a73634e581677e`), copied into this repository and changed as
listed in `crates/skp/VENDORED.md`. It is distributed under the MIT licence:

```
MIT License

Copyright (c) 2026 Hakko Kim

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Test fixtures (not part of the app)

`crates/skp/tests/fixtures/cupboard.skp` and `desk.skp` are SketchUp models by Corynth,
licensed under Creative Commons Attribution 4.0 International (CC BY 4.0,
https://creativecommons.org/licenses/by/4.0/), from
https://corynth.itch.io/free-assets-sketchup-3d-assets, unchanged. They are used only by tests;
see `crates/skp/tests/fixtures/ATTRIBUTION.md`.
