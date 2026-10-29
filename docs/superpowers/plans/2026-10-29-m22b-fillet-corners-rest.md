# M22b: fillet and chamfer corners, the rest

Builds on M21d (`crates/kernel/src/fillet/corner.rs`): a corner cutter per corner, subtracted
after the edge cutters, falling back to the crossed result when it fails.

## Rulings

1. **Chamfer corners.** At a convex corner of exactly three chosen chamfers and three faces the
   run-on cutters leave the three chamfer planes meeting in a point. The corner cutter is a
   tetrahedron: the plane through P0, P1, P2 (P_k is where the chamfer strips of face k meet,
   `V + s*e_a + t*e_b` with `s = size_b / (d_b . e_a)`, `t = size_a / (d_a . e_b)`) and three
   caps just outside the faces. The solid's section by that plane is exactly the triangle
   P0P1P2 (each pair of points lies in one chamfer plane), so the cutter removes only the tip and
   the triangle is a new face. Works for oblique corners and unequal distances.
2. **Concave corners.** Three concave fillets of one radius at a vertex of three faces: the same
   hexahedron-minus-sphere solid built with the faces' normals flipped (the ball lies in the
   empty corner), added with a union. Three concave chamfers keep their planes meeting in a
   point (a cone of empty space, not a defect). Mixed convex and concave corners keep the crossed
   cuts (closed solid, no patch): the ideal surface there is a toroidal blend, not worth building.
3. **Mixed radii and round plus chamfer.** One call may carry per-edge sizes (`radii`,
   `distances`, optional lists parallel to `edges`; `radius` and `distance` stay the required
   defaults). Convex corners whose three radii differ get a star-shaped patch (`patch_mixed`):
   each arc ends at the furthest of the two crossings of its tangent line with the neighbours'
   (as `flat`'s `meet`, with fillet setbacks), a straight segment on each face joins one arc's
   end to the next arc's start, rings are slerped from the apex (about the mean of the arc
   centres) to the boundary and clamped inside the faceted rounds and below the faces. The
   cutter is the patch plus skirts (in the arc planes, square to the face over a segment) and a
   cone cap, so it touches the solid only along new lines. `took`, `on_sphere` and `soften` use
   the patch's own vertex list. Concave corners of different radii keep the crossed cuts. A second call whose edge ends at faces that are not flat and coplanar is still
   refused. Round plus chamfer stays two calls.
4. **Crash.** `vector_area` summed cross products of absolute positions: for a 0.02 m face at
   offset 1e5 the cancellation left a zero normal, `triangle_vertices` then fed NaN coordinates
   to earcutr, which unwraps. `vector_area` now sums about the first point; `triangle_vertices`
   works about the face's first vertex and returns None for a bad normal or non-finite
   coordinates. The boolean itself loses precision far from the origin for tiny radii: such
   calls end in a clean error ("the result would not be a solid"), not a panic.

## Tests

Kernel (`fillet.rs`): flat triangle on a box corner and on boxes, prisms and shears; unequal
distances; three concave fillets (sphere vertices, patch sag, soft edges, volume); mixed
convex/concave corners closed; per-edge radii; tiny box far from the origin. Session: `radii`
and `distances` (length and range errors, undo).
