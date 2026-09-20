# Performance

Where the time goes, and whether the targets in `IMPLEMENTATION_PLAN.md`
§Phase 9 are met.

Benchmarks live in `benches/pipeline.rs` and run with `cargo bench`. They are
not a gate: a regression in them is a reason to look, not a reason to fail a
build.

## Targets

| Target | Result | Verdict |
|---|---|---|
| A 1024x1024 flat-colour illustration end to end in under 1.5 s, single-threaded | **34.6 ms** | met, with a 43x margin |
| The shape pass under 5% of total time | **5.1%** | met, within measurement noise |
| Both cleanup filters together under 20 ms at 1024x1024, single-threaded (`CLEANUP_IMPLEMENTATION_PLAN.md` §4 Phase 2) | **19.4 ms** | met; see "Cleanup stage" below |

## Baseline

`aarch64-apple-darwin`, Apple M-series, rustc 1.98.1, release profile
(`lto = "fat"`, `codegen-units = 1`), criterion medians, 2026-09-20.

The fixture is a flat-colour poster: a white background, a disc, a rectangle,
and a five-pointed star, drawn with hard edges and scaled to the benchmark size.

### Whole pipeline

Decode is excluded (it is `image`'s cost, not ours); trace, shape pass, write,
and optimize are included.

| Canvas | Median |
|---|---:|
| 256 x 256 | 2.57 ms |
| 512 x 512 | 8.92 ms |
| 1024 x 1024 | 34.6 ms |

Time grows with pixel count, as expected: the tracer dominates and it is a
per-pixel algorithm.

### By stage, at 512 x 512

| Stage | Median | Share |
|---|---:|---:|
| trace | 8.43 ms | 94.5% |
| shape pass | 0.46 ms | 5.1% |
| write | 0.078 ms | 0.9% |
| optimize | 0.093 ms | 1.0% |

Tracing is the whole cost. Everything this tool adds on top is about 7% of it.

## What made the shape pass fast

It started at 1.07 ms, 11.4% of the pipeline, and two changes brought it to
0.46 ms without changing a single test outcome.

| Change | Shape pass | Why it works |
|---|---:|---|
| starting point | 1.07 ms | every shape's circle candidate was rasterized |
| structural detectors first | 0.70 ms | a rectangle is recognized by its corners and costs nothing to reject; the circle and ellipse are proposed for *any* outline and need a raster comparison to rule out. Trying `rect` first means a rectangle never rasterizes a circle. |
| bounding-box pre-filter | 0.46 ms | a candidate whose box is more than 10% off is rejected before rasterizing. A star's equal-area circle is 30% out; a real fit is within 1%. |

An exact area pre-filter was there from the start: the symmetric difference of
two shapes is never smaller than the difference of their areas, so a candidate
that fails on area cannot pass the raster comparison. That one is free and
rigorous; the bounding-box filter is a heuristic whose only failure mode is a
missed detection.

## Cleanup stage

`CLEANUP_IMPLEMENTATION_PLAN.md` adds a raster cleanup pass before tracing:
two signals that measure blur and noise, and two filters driven by them.
Same machine and profile as the baseline above; the fixture is the poster at
1024x1024 after a Gaussian blur of sigma 3, which is the input the pass exists
for. Radii are the ones the automatic rule picks for it: Kuwahara 3, toggle
contrast 2. Single-threaded unless stated.

| Step | Naive | Optimised | Change |
|---|---:|---:|---:|
| `edge_width` | 3.0 ms | 3.0 ms | signal, not optimised |
| `flat_noise` | 4.6 ms | 4.6 ms | signal, not optimised |
| Kuwahara, radius 3 | 67.1 ms | 7.6 ms | 8.8x |
| toggle contrast, radius 2 | 17.3 ms | 11.8 ms | 1.5x |
| **both filters** | **84.4 ms** | **19.4 ms** | **4.4x** |
| Kuwahara, radius 3, parallel path | | 1.3 ms | |
| toggle contrast, radius 2, parallel path | | 1.8 ms | |

The naive forms are the plan's §10.4 and §10.5 reference, ported as written;
the plan's own spike measured them at 103.5 ms and 34.3 ms on the same size.
Both optimised forms reproduce the naive output **bit for bit**, which the
tests `optimised_kuwahara_matches_the_naive_oracle_exactly` and
`optimised_toggle_matches_the_naive_oracle_exactly` assert on the fixtures and
on random images built to be full of ties. Nothing about the answer changed;
only how it is computed.

| Change | Why it works |
|---|---|
| Kuwahara: sliding row sums, then a sliding stack of `radius + 1` rows | each quadrant sum is O(1) instead of O(radius²); a ring of the last few rows keeps the working set at a handful of rows |
| Kuwahara: exact integer luminance and `u32` division for the mean | the four divisions per pixel were the hot spot once the sums were cheap; integers also make the two forms agree exactly, which floats cannot promise |
| toggle: van Herk and Gil-Werman running extreme along rows and down columns | three comparisons per pixel per axis whatever the radius |
| toggle: pixels packed as `(luminance, row, column)` keys | every key is unique, so the running extreme needs no tie logic at all, and a plain `min` or `max` picks the same pixel the naive scan order picks |
| both: bands of 32 rows, processed in sequence or on the `rayon` pool | the parallel path is the same code over the same bands, so the two paths give the same bytes; a test asserts it |

The toggle's remaining cost is the vertical pass: NEON has no 64-bit integer
minimum, so the elementwise row comparisons stay scalar. It is within budget
and the parallel path takes it to under 2 ms, so it was left there.

## Reproducing

```sh
cargo bench --bench pipeline
# Quicker, for a rough comparison:
cargo bench --bench pipeline -- --warm-up-time 1 --measurement-time 3
# The cleanup stage only:
cargo bench --bench pipeline -- 'cleanup/' --warm-up-time 1 --measurement-time 3
```

## Acceptance run

The whole-project acceptance from `IMPLEMENTATION_PLAN.md` §7, on twenty
generated logos (mixed PNG and JPEG, 120 to 240 px, discs, rectangles, ovals,
and stars, some with a second colour on top):

| Measure | Result |
|---|---|
| Batch | 49 882 -> 31 416 bytes (63%), 77 ms for twenty files |
| Elements | 46 native shapes, 117 paths |
| Fidelity (`--verify`) | min 0.9919, median 0.9968, max 1.0000; none below 0.99 |
| Size ratio per file | 5% to 158% |
| Second run | exit 2, all twenty collisions listed, nothing written |
| `--force` | exit 0 |

Two of the twenty came out larger than their input. Both are JPEG photographs
of a star: JPEG is very good at a small lossy raster, and a faithful vector of
a many-cornered shape is not small. The tool's job is the smallest *faithful
vector*, not the smallest file of any kind, and `--stats` is there so this is
visible rather than surprising.

## Cleanup acceptance run

The manual acceptance from `CLEANUP_IMPLEMENTATION_PLAN.md` §5, on a real
downloaded logo: the 120 px thumbnail of the Rust logo from Wikimedia Commons
(a grey-plus-alpha PNG), then the same file saved as a quality-20 JPEG with
macOS `sips`, then scaled up three times and saved at quality 20 again. Debug
build, `--stats --verify`, `--cleanup off` against the default.

| Input | Cleanup | Output | Shapes | Commands | Fidelity | Time |
|---|---|---:|---:|---:|---:|---:|
| clean PNG, 120 px | off | 3 885 B | 1 | 190 | 0.9193 | 23 ms |
| clean PNG, 120 px | auto: did nothing | 3 885 B, byte-identical | 1 | 190 | 0.9193 | 36 ms |
| JPEG quality 20, 120 px | off | 38 775 B | 145 | 1 893 | 0.9446 | 135 ms |
| JPEG quality 20, 120 px | auto: `d3 s0 (blur 1.6px, noise 1.713)` | 16 597 B | 58 | 805 | 0.9675 | 101 ms |
| upscaled 3x, JPEG quality 20, 360 px | off | 291 687 B | 828 | 14 313 | 0.9799 | 1 073 ms |
| upscaled 3x, JPEG quality 20, 360 px | auto: `d3 s2 (blur 4.8px, noise 0.390)` | 36 666 B | 136 | 1 645 | 0.9845 | 512 ms |

The clean file is the no-op guarantee on a real input. The two damaged ones
convert 2.3x and 8x smaller, closer to the drawing by the tool's own
measure, and the upscaled one in half the time, because there is far less
geometry to fit once the halo of slivers is gone. Fidelity here is against
the cleaned raster (ADR-0011); the clean file's low absolute score is the
grey-plus-alpha thumbnail's anti-aliased edge against a one-path trace, the
same with cleanup on or off.

## Still to do

The plan asks for baseline numbers from both CI runners. These are from the
development machine only; the CI job that records them is part of Phase 10.
