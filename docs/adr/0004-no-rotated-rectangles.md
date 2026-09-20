# 0004 — rotated rectangles stay paths in v1

- **Status:** accepted
- **Date:** 2026-09-20
- **Phase:** 5

## Context

SVG's `<rect>` is axis-aligned. A rotated rectangle can only be expressed as a
`<rect>` plus a `transform="rotate(a cx cy)"`, the same way a rotated ellipse
is. So the capability exists; the question is whether detecting it is worth it.

Detecting an axis-aligned rectangle is cheap and unambiguous: four corners after
collinear cleanup, right angles, edges along the axes. Detecting a rotated one
is a different problem. The corner test still works, but the shape's angle has
to be recovered from the geometry, and every later decision depends on that
angle being right:

- The angle has to come from the edges, and a traced outline's edges are not
  exact. A rectangle 200 px wide whose recovered angle is half a degree off is
  nearly 2 px out of place at its corners.
- Raster verification catches a bad angle, so a wrong answer is not a fidelity
  risk. It is a cost risk: every rotated-ish quadrilateral becomes another
  candidate to rasterize, and most of them are not rectangles.
- The output is no longer obviously smaller. `<rect x y width height
  transform="rotate(a cx cy)"/>` is close in bytes to the four-command path it
  replaces, and the path is the thing a later SVG optimizer already handles
  well.
- Rotated rectangles are rare in the inputs this tool targets. Logos, icons, and
  flat illustrations are overwhelmingly axis-aligned; the shapes that are not
  tend to be rotated at angles a human chose, drawn once.

Rotated *ellipses* are different on every count: the angle falls out of the
moment computation for free and exactly, a `<ellipse>` with a transform is much
shorter than the eight-cubic path it replaces, and tilted ellipses are common.
They are detected.

## Decision

`rect_candidate` requires every edge to lie within half a degree of an axis. A
rotated rectangle produces no rectangle candidate and stays a path.

Rotated ellipses are detected, with the rotation emitted only when it exceeds
half a degree, and `--no-rotated-ellipses` turns even that off.

## Consequences

**Gained.** The rectangle detector is a handful of predicates with no angle
estimation and no failure mode beyond "not a rectangle". No candidate is
rasterized for the large class of quadrilaterals that are not rectangles. The
`<rect>` elements in the output never carry a transform, which keeps them
readable and keeps the optimizer's job simple.

**Lost.** A rotated rectangle is emitted as a four-command path instead of a
`<rect>` with a transform. The byte difference is small; the visual result is
identical. Recorded as a known false negative in `docs/shape-detection.md`.

**Accepted.** Adding rotated rectangles later is additive: a new candidate
function, the existing verification, and one more `Prim` field. No output
already produced would change meaning, so it needs no migration, only a
changelog entry noting that some paths become rects.
