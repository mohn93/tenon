# M21e: renderer level of detail, memory and cracks

## Compact vertex

The simplified copies hold flat-coloured, untextured, non-glowing, opaque triangles only
(glowing and textured triangles stay whole at full detail; glass is never simplified), and
a chunk with anything selected or hovered is already drawn at full detail (`chunk_marked`),
so a copy needs no flags, uv, layer, glow or alpha. `LodVertex` (28 bytes, was 104 plus a
4-byte flag buffer as long as the largest level):

- position f32x3 (12)
- normal snorm8x4 (4)
- front colour unorm8x4 (4), back colour unorm8x4 (4)
- finish unorm8x4 (4): front metalness, roughness, back metalness, roughness

A vertex entry `vs_face_lod` fills the same `FaceOut` with constants (opaque, no texture, no
glow, not selected, not hovered), so `fs_face` and `fs_ao_normal` are reused. Two pipelines
(`face_lod_pipeline`, `ao_normal_lod_pipeline`) differ from the full ones in the vertex
layout only. Simplified chunks are drawn with one vertex buffer.

`Renderer::lod_gpu_bytes` reports what the copies take; measured on Dubai before and after.

## Full detail near section planes

Faces are clipped on the CPU against the active planes, but the cut fill and cut edges are
built from the full geometry, so a simplified chunk at a cut would not meet them. `SceneBase`
lists the planes that cut (`cut_planes`, root coordinates); a chunk whose box lies within the
largest cell of a plane (the plane's guard) is drawn at level 0.

## No cracks

Clustering is on one global grid, but a cell's position is the average of the vertices of the
chunk being simplified, and each chunk is simplified on its own, so the two sides of a border
between chunks never agree, even at the same level. Chosen fix: pin the vertices that more than
one chunk touches.

- A first pass over all chunks (solid and not) finds the places where vertices of two or more
  chunks lie (`Borders`, exact positions). Vertices of an edge shared by two chunks lie at such
  places in both. (A first try pinned whole cells of the finest level holding two chunks; with
  chunks a few cells wide that pinned nearly everything, so exact places it is.)
- Clustering keeps a vertex at such a place, or equal to a corner of a triangle kept whole in
  the chunk (long, glowing, textured), at its exact position: it is its own cluster, and
  triangles on it are dropped only when two corners are the same vertex position.
- The nested coarser levels reuse the same pins (the positions do not move).

Every edge shared between two chunks (or between a kept triangle and a simplified one) then has
pinned ends, so it is the same segment in the full chunk, and in a chunk at any level. A gap
cannot open between chunks at different levels. Cost: triangles around chunk borders and around
kept triangles stay finer. Skirts were rejected: chunks are not grid blocks, so there is no
simple border to hang them on.

Test: a bumpy surface split in two chunks, simplified at different levels; every edge lying on
the shared line in one is also in the other (and not without the pins).

## Fountain

Dubai's fountain becomes thin tapered jets (frustum columns with a spray head), arched jets of
short segments, and a spray halo, in pale translucent water material with emission at night,
along the curving lines of the real fountain. Rendered full size and checked by eye.
