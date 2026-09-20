# Shape detection

How `vectorise` decides that a traced region *is* a circle, an ellipse, or a
rectangle, what tolerance that decision uses, and which shapes it knowingly
misses. Everything here is measured, not assumed; the measurements are
reproducible from the tests in `src/shapes.rs` and `src/geom/`.

## The algorithm

For each traced shape, in paint order:

| Step | Module | What happens |
|---|---|---|
| 1 | `geom/convert.rs` | The shape becomes a `kurbo::BezPath`. A shape with more than one subpath stops here and stays a path. |
| 2 | `shapes.rs` | Outlines under `--min-shape-area` (default 16 px²) stay paths. |
| 3 | `geom/fit.rs` | Candidates are proposed: circle, rectangle, rounded rectangle, ellipse, in that order. |
| 4 | `shapes.rs` | Each candidate's area is compared with the outline's. A candidate that already differs by more than the tolerance is dropped without rasterizing, because the symmetric difference of two shapes is never smaller than the difference of their areas. |
| 5 | `geom/verify.rs` | The survivors are rasterized next to the outline and their symmetric difference is measured. |
| 6 | `shapes.rs` | The first candidate within tolerance wins. If none is, the shape stays a path. |

Proposing generously and verifying strictly is deliberate. A wrong candidate
costs one raster comparison; it can never cost fidelity.

### Where each candidate comes from

- **Ellipse and circle: moments.** Green's theorem gives the area integrals of
  a closed path in one pass. Diagonalizing the central second-moment matrix
  recovers the centre, both radii, and the rotation at once. The circle
  candidate is area-preserving instead: `r = sqrt(A / pi)`.
- **Rectangle: structure.** Straight segments only, four corners after collinear
  cleanup, interior angles within 1 degree of square, edges within half a degree
  of an axis.
- **Rounded rectangle: structure.** Exactly four straight edges, each on one side
  of the bounding box, with at least one curve per corner between them. The
  radius is the average of the four insets the straight edges imply.

A cubic whose control points sit on the line between its endpoints counts as a
straight edge. VTracer's spline fitter writes every segment as a cubic,
including the flat sides of a rectangle, so without this no traced rectangle
would ever be recognized.

## Tolerance

```text
tolerance = max(frac * area, frac * 10 * perimeter, 1.0)   square pixels
```

`--shape-tolerance` is `frac`, default 0.02. `--shape-tolerance 0` switches
every term off and converts nothing.

The perimeter term is not decoration. The error between a traced outline and the
shape it approximates lives on the **boundary**, so it grows with the perimeter,
while an area-proportional budget grows with the square of the radius. Measured
on traced discs:

| Drawn radius | Traced area (px²) | Perimeter (px) | Difference from the fitted circle (px²) | As a fraction of area | Per pixel of perimeter |
|---:|---:|---:|---:|---:|---:|
| 8 | 217 | 53 | 6.8 | 3.1% | 0.13 |
| 10 | 335 | 65 | 6.8 | 2.0% | 0.10 |
| 12 | 468 | 77 | 11.2 | 2.4% | 0.15 |
| 16 | 830 | 102 | 6.3 | 0.8% | 0.06 |
| 24 | 1832 | 152 | 17.5 | 1.0% | 0.12 |
| 40 | 5092 | 253 | 24.6 | 0.5% | 0.10 |
| 80 | 20239 | 504 | 117.2 | 0.6% | 0.23 |

The last column is flat; the second-to-last is not. Without the perimeter term
a radius-8 disc would be rejected for an error a radius-40 disc is forgiven.

The weight of 10 pixels is calibrated against the opposite failure. At the 2%
default it allows a band a fifth of a pixel wide along the outline, which is
invisible, and it still rejects the tightest false positive there is: an
axis-aligned square offered as the circle of equal area, which differs by 18.1%
of its area. The rejection holds for every square above about 4.4 px a side,
and `--min-shape-area` (16 px², a 4 px square) keeps everything smaller out of
the pass entirely. Raising the weight to 15 would let a 6x6 square through as a
circle.

The absolute floor of 1 px² exists so a degenerate outline cannot produce a
tolerance of zero and divide by it.

## Known false negatives

These shapes stay paths. Output is larger than it could be; fidelity is never
affected.

| Class | Why | Remedy |
|---|---|---|
| Any shape with holes or several disjoint rings | One `<circle>` cannot express a ring, and extra rings would be lost. Note that holes only arise with `--hierarchical cutout`; the default stacked mode paints the inner region over a solid outer one, and the outer shape is then correctly detected as a circle. | none in v1 |
| Ellipses thinner than about 3:1 | The *tracer* stops producing an ellipse. At the default `--segment-length` the fitted spline of a 400x100 oval encloses 7.3% more area than the pixels it came from, so no ellipse is faithful. | `--segment-length 1` cuts the error to 1.5% and the ellipse is detected. Measured in `pass_detects_a_large_ellipse_once_the_tracer_fits_it_finely`. |
| Large smooth shapes generally | Same cause. The default segment length of 4 px is coarse for a curve hundreds of pixels long. | `--segment-length 1` |
| Rotated rectangles | Not detected in v1 (ADR-0004). | none in v1 |
| Rotated ellipses under `--no-rotated-ellipses` | Deliberately refused rather than straightened, because straightening changes the shape. | drop the flag |
| Outlines under `--min-shape-area` | A primitive saves almost nothing at that size, and the fit would be mostly rasterization noise. | lower `--min-shape-area` |
| Shapes whose traced outline has a corner artifact | The spline fitter inserts a corner where `--corner-threshold` says the turn is sharp, and the outline then is not smooth. | raise `--corner-threshold` |

### The detection envelope, measured

Axis-aligned ellipses, rasterized with hard edges, traced with defaults, then
fitted. `Y` means a circle or ellipse was emitted.

```text
 rx \ ry    8   10   14   20   30   45   60   80  100  140  200
      8     Y
     10     Y    Y
     14     Y    Y    Y
     20     Y    Y    Y    Y
     30     Y    Y    Y    Y    Y
     45     n    n    Y    Y    Y    Y
     60     n    n    n    Y    Y    Y    Y
     80     n    n    n    n    Y    Y    Y    Y
    100     n    n    Y    n    n    Y    Y    Y    Y
    140     n    Y    n    n    n    Y    Y    Y    Y    Y
    200     Y    Y    n    n    n    n    Y    Y    Y    Y    Y
```

Every failure is at an aspect ratio above 3:1, and the boundary is ragged
because it depends on where the spline fitter happens to place its segments.
The property test therefore asserts detection up to 2.5:1, where 120 random
samples found no failure, and a second property asserts the invariant that holds
everywhere: whatever the pass emits is within tolerance of the outline it
replaced.

## Verification

`geom/verify.rs` rasterizes both paths with `tiny-skia` and sums the per-pixel
absolute difference in anti-aliased coverage. The shared bounding box is
oversampled two times, then clamped to between 64 and 512 pixels on its longest
side, and the result is scaled back, so a comparison costs a bounded amount of
work whatever the size of the shape and the answer is always in the input's
square pixels.

Where both shapes partially cover the same pixel, the absolute difference
understates the true symmetric difference: two half-covered pixels could
disagree about *which* half. The error is bounded by the length of the shared
boundary in raster pixels, which is where the two shapes agree anyway.

## Deviations from the implementation plan

| Plan | What was built | Why |
|---|---|---|
| "an absolute floor of 1.0 px²" | The floor is 1.0 px², and a perimeter term was added beside it | The constant floor alone rejects small shapes for an error large ones are forgiven. See the table above. |
| Property test over `rx, ry in [8, 200]` | Same range, aspect ratio capped at 2.5:1, plus a second property covering everything up to 8:1 | Above 3:1 the tracer's own fit is several percent off, so detection would be asserting something the tracer does not deliver. |
| Property test with a 4 px canvas margin | Margin is 60% of the shape, at least 8 px | With only a few pixels of border the clustering inverts: the shape becomes the base layer and the background is painted over it with a hole, leaving no shape to detect. |
