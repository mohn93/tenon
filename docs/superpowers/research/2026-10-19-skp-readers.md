# .skp reader evaluation (2026-10-19)

Input for M13e. Two MIT-licensed readers were compared without Trimble's SDK and without
running SketchUp on them: openskp-rs (Rust, reader only) and openskp (Python/C++/TS/.NET/Dart,
reader and legacy-format writer). Nothing from Hew (AGPL) or hew3d/openskp (GPL) was used.
The test corpus and harnesses live outside the repo in `../sketchup-skp-corpus/` (Trimble
sample files are not redistributable); its README says how to rebuild the legacy set.

## Decisions (user, 2026-10-19)

- Import: fork openskp-rs into this workspace and own it.
- Export: port openskp's C++ writer (`create.cpp`, about 2,200 lines) to Rust.
- openskp stays a test-only cross-check.
- Versions: SketchUp 2017 and newer (2017-2020 legacy MFC container, 2021+ VFF zip container).
- Provenance accepted: openskp-rs took record names from SketchUp's framework and cites
  disassembly; openskp's writer builds on a blank file made with the SketchUp SDK.

## Results

Corpus: 329 files (325 VFF from 2021-2024, 4 SketchUp 7), plus 24 models saved by SketchUp
2024 as 2017, 2018, 2019 and 2020 (96 files).

| | openskp-rs | openskp |
|---|---|---|
| VFF 2021-2024 (325) | all read, agree with openskp | all read |
| Legacy 2017 / 2018 / 2019 / 2020, exact match of 24 | 21 / 21 / 21 / 0 | 20 / 20 / 3 / 17 |
| Legacy faces with loops and holes | no (triangles only; records parsed internally) | yes |
| Guides | no | yes |
| Legacy scenes | yes | no (C++ drops them; Python returns none) |
| Silent empty results on valid files | no | yes (several 2017-2020 files) |
| 96 MB file | 0.69 s, 512 MB | 4.3 s, 3.6 GB |
| Corrupt input (1,191 mutants) | no crash, hang or blowup | no crash (UB in bundled miniz) |

Both readers often report success on corrupt files with partial or garbage geometry.

## Gaps M13e must fill (openskp-rs route)

1. 2020 legacy files: every one fails ("implausible definition count"); openskp reads them.
2. Silent face loss on one legacy model (couch: soft edges and hidden faces); add an
   area-or-triangle check, the fork's own shortfall counter misses it.
3. Root entity list stops at an unknown back-reference (truck: 19 entities lost, reported).
4. Deep nesting (barstools, depth 4) fails in every reader and version: reverse engineer or
   give a clear error.
5. Legacy topology: expose faces with outer and inner loops, materials, tags and hidden flags.
6. Guides for both containers.
7. `InflateRead::pump` stops before draining buffered output (valid files re-zipped at zlib
   level 6 or 9 fail); `pid as u32` truncation.
8. Texture bytes (the fork returns their location only), front and back UVs.
9. Instances: hidden flag, group or component on legacy, paint inheritance.
10. Importer rules: validate everything (finite coordinates, loops of 3+, index ranges);
    success with lost or zero geometry is shown as a warning, not as success.
11. Regression: the 96-file legacy set must read the same as its originals.

Export (openskp writer port) writes the 2017 format; it writes no scenes or tag folders.
Files it wrote reopened correctly in all three readers but were not checked in SketchUp.
