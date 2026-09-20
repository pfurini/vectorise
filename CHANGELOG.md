# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Project skeleton: library plus binary, pinned toolchain, lint configuration,
  `cargo-deny` license gate, `justfile`, and CI on macOS and Linux.
- `vectorise --version` and the exit-code contract (0, 1, 2, 64, 74).
- `docs/api-notes.md`: Phase 0 verification of every pinned pre-release API
  against its vendored source.
- Input resolution, output mapping, and batch preflight: `vectorise::plan`
  turns arguments into a validated `Plan` or the complete list of reasons the
  batch is rejected. Supports literal paths, self-expanded globs when the shell
  did not expand them, `--output-dir`, `--force`, and `--dry-run`.
- Decoding: `vectorise::decode` reads PNG, JPEG, WebP, GIF, BMP, and TIFF into
  the tracer's image type, deciding the format by content rather than by
  extension, applying EXIF orientation (`image` 0.25 does not), and resolving
  transparency against `--background` (default white).
- `vectorise::color::Rgb` with `#rrggbb` and `#rgb` parsing.
- Tracing: `vectorise::trace` maps typed options onto `vtracer::Config` and
  runs the pipeline, with `--preset`, `--clustering`, `--hierarchical`,
  `--mode`, `--filter-speckle`, `--color-precision`, `--gradient-step`,
  `--max-colors`, `--palette`, `--palette-file`, `--simplify`, `--threshold`,
  `--adaptive`, and `--watershed-detail`.
- Shape detection: `vectorise::shapes` replaces a traced outline with a
  `<circle>`, `<ellipse>`, or `<rect>` (sharp or rounded) whenever the
  substitution is within `--shape-tolerance` of the original, measured as
  symmetric difference on a raster. `--shapes`, `--shape-tolerance`,
  `--min-shape-area`, `--no-rotated-ellipses`.
- `--corner-threshold` and `--segment-length`, which control how closely the
  tracer fits a curve and therefore whether large smooth shapes are detected.
- SVG writer: `vectorise::writer` serializes a `ShapeDoc` to minimal SVG with
  no prolog, no comments, and no groups; shortest-form hex fills, shortest-form
  numbers, and relative path commands with `h`, `v`, and `s` shorthands.
  `--precision` and `--keep-size`.
- Optimizer pass: `vectorise::optimize` runs oxvg over the written SVG with
  every job that would undo shape detection disabled, and with
  `convertPathData`'s approximating sub-passes off, so the pass is a pure
  re-encoding and the rendered result is bit-identical. `--no-optimize`.
- End-to-end conversion: `vectorise logo.png` now reads, traces, fits shapes,
  writes, optimizes, and saves `logo.svg`. Outputs are written atomically, so
  an interrupted run never leaves a partial file, and the rename refuses to
  clobber a file that appeared after preflight.
- Batches run in parallel (`--jobs`), report results in plan order whatever the
  thread count, and keep going after one file fails (exit 1, with the failing
  file named).
- `--quiet` and `-v`/`-vv`/`-vvv`.
- `--verify` renders the result at the input's pixel size and reports
  `1 - mean absolute error` (ADR-0008).
- `--stats` reports what each conversion did, one line per file plus a total,
  on stderr; `--stats-json` reports the same numbers as one JSON object per
  line on stdout (ADR-0007).
- Benchmarks (`cargo bench`) and `docs/perf.md`. A 1024x1024 illustration
  converts in 34.6 ms, against a 1.5 s target; the shape pass is 5.1% of the
  pipeline, after two optimizations that made it 2.3x faster.
- `docs/adr/0001-language-and-stack.md`, `docs/adr/0002-output-dir-flattens.md`,
  `docs/adr/0003-preset-auto.md`, `docs/adr/0004-no-rotated-rectangles.md`,
  `docs/adr/0005-no-group-merging.md`, `docs/adr/0006-optimizer-is-lossless.md`,
  `docs/adr/0007-serde-for-stats-json.md`,
  `docs/adr/0008-fidelity-is-mean-absolute-error.md`,
  `docs/shape-detection.md`, `docs/perf.md`, `docs/size.md`.

[Unreleased]: https://github.com/pfurini/vectorise/compare/HEAD
