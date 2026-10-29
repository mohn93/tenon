# M22a: dimensions and text of 2017-2020 files

M21b imports linear dimensions and pushpin text from 2021+ files (`skp::annotations`,
`DefGeometry::annotations`, mapped in `session/src/skp.rs`). The legacy reader walked
`CDimensionLinear` and `CText` records and dropped them. This reads them into the same
`RawAnnotation` list so the session maps both containers identically.

## Record layouts (measured on `legacy_annotations.skp`, `capilla_quiroz_v17.skp`,
`tutorial01_v2017..2020.skp`)

`CDimensionLinear`, after the entity preamble, drawing element, text string (empty: the measured
length) and font:

```text
5 bytes, u32 kind (1 free, 2+ attached), u32 4, point A (3 x f64)     37 bytes
ref (entity ref), second ref, u32 n1, u32 n2 (+ n1 + n2 path items)   connection of A
u32 kind, u32 4, point B (3 x f64)                                    32 bytes
connection of B
normal 3 x f64, x axis 3 x f64, u32 type, y pos f64, x pos f64, u32 text pos    72 bytes
```

`CText`, after the font, the middle that ends at the string marker (123 bytes in every sample):

```text
16 bytes, u32 kind, u32 4, anchor A (3 x f64), connection (as above), vector (3 x f64),
text direction (3 x f64), u32 leader type (2 pushpin), 11-byte block, string marker
```

A point with a connection holds zero in its position; the ref is the store-map index of the
vertex or guide point, resolved when the definition is built (a dimension is written before the
geometry it measures, so the ref can point forward). Refs with instance paths, and refs to
anything but a vertex or guide point of the same definition, are counted as unread.

## Work

1. Tests first: unit test on a synthetic record; corpus tests (ignored, `SKP_CORPUS`) for the
   two samples, tutorial legacy-vs-2024 twin equality, mutation test (no panic).
2. `legacy.rs`: keep the fields in a per-definition pending list (released with the other
   arenas), resolve in `build_topology` into `DefGeometry::annotations` and
   `annotation_skips`, element budget spent per annotation.
3. Session: the legacy "not imported" warning goes; skips are reported as for 2021+.
4. Docs: agent-api import report, roadmap row.

## Limits

Radial dimensions and screen text have no legacy sample: the class of a radial dimension is not
recognised by the walker (unchanged); screen and view-based text (leader type other than 2) is
counted as screen text.
