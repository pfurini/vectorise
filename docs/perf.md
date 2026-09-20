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

## Reproducing

```sh
cargo bench --bench pipeline
# Quicker, for a rough comparison:
cargo bench --bench pipeline -- --warm-up-time 1 --measurement-time 3
```

## Still to do

The plan asks for baseline numbers from both CI runners. These are from the
development machine only; the CI job that records them is part of Phase 10.
