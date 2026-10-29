# M21d: fillet corners

Goal: where three convex fillets of one radius meet at a trihedral vertex, close the corner with a
sphere patch instead of crossed rounds.

## Approach

Edge cutters still run on past the vertex (as in M17d), so the booleans give the crossed-round
corner, which is always valid. Then, per qualifying corner, one extra cutter is subtracted from the
result: the hexahedron between the three planes through the sphere centre perpendicular to the edges
(`P_k`) and three caps a quarter radius outside the faces, minus the sphere patch. The sphere is
tangent to the three faces (centre = intersection of the faces offset inward by r); it lies inside
all three fillet cylinders, so only the crossed leftovers are removed. The patch triangles are built
from the fillets' own profile points on the planes `P_k`, so the boundary arcs coincide with the
cylinder facets exactly. Equal arc step counts give a barycentric grid (octant: n^2 triangles),
unequal counts (or a grid point that would poke through a faceted fillet) concentric rings from the
patch middle: rings about one arc step apart, every inner point checked against the three faceted
fillets and pulled toward the middle until inside, inner rings with fewer points, rings joined by
advancing along the shorter edge. A middle point a coarse fillet would cut sinks into the ball.

Chosen over stopping the edge cutters at the planes: that leaves a pinched face (the corner square
touches the rest of the face at one point) between boolean steps. A corner cutter on top of the
known-good crossed rounds can simply be skipped when it fails.

## Rulings

- Qualifies: vertex with exactly three faces and three edges, all chosen, convex, all running on,
  fillet profile (one radius per call), edge lengths leave room for both patches (sum of the two
  depths under the length). Anything else keeps the M17d result.
- Chamfers keep the M17d result: three chamfers meet in a point (clean, tested); no flat triangle.
- Mixed fillet/chamfer and unequal radii need separate calls (one profile per call); the second call
  sees the first's rounds as soft-edged faces and is tested to stay a solid. Corners with one or
  two chosen edges are unchanged.
- A failed corner boolean (error, open, no sphere vertices) leaves that corner crossed.
- Patch faces take the first edge's materials; edges inside the patch and between patch and
  fillet facets are soft.

## Tasks

1. Tests: box 12 edges (volume, sphere distances, counts), box 3 edges, prism corner, chamfer
   corner, mixed stays solid, smooth normals, session command.
2. `fillet/corner.rs`: patch geometry and cutter; wire into `bevelled`.
3. Docs: agent-api.md, roadmap row.
