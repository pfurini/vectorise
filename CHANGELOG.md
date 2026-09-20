# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-09-20

### Added

- A raster cleanup stage between decoding and tracing, on by default. It
  measures how blurred and how noisy the input is, flattens noise with a
  Kuwahara filter, and steepens soft edges with morphological toggle
  contrast, choosing each radius from its own measurement. On a clean, sharp
  input it changes nothing, byte for byte. On the plan's 24-case matrix of
  blurred, downscaled, and JPEG-damaged fixtures it makes the output 6.7x
  smaller in total, never larger, and closer to the original drawing in every
  case (ADR-0010).
- `--cleanup <auto|off>`, `--denoise <PX>`, and `--sharpen <PX>`. `--cleanup
  off` restores the v0.1.0 pipeline exactly; a radius overrides the automatic
  choice for that operator. `--preset photo` makes `off` the default, and an
  explicit flag still wins over it. `--cleanup off` together with a radius is a
  usage error.
- `--stats` reports what cleanup measured and did (`cleanup d3 s2 (blur 4.1px,
  noise 0.166)`), and `--stats-json` gains `edge_width`, `flat_noise`,
  `cleanup_denoise`, `cleanup_sharpen`, and `cleanup_delta`, present only when
  cleanup changed something.
- `docs/adr/0010-raster-cleanup-operators.md` and
  `docs/adr/0011-verify-measures-against-the-cleaned-image.md`.

### Changed

- `--verify` measures `fidelity` against the image the tracer saw, which is
  the cleaned raster when cleanup acted and the decoded input otherwise;
  `cleanup_delta` reports how far the two are apart (ADR-0011). With
  `--cleanup off` the number means exactly what it meant in 0.1.0.
- `vectorise::Options` gains `cleanup`, and `Cli::options` returns
  `cli::OptionsError`, which wraps the tracing and cleanup option errors.

## [0.1.0] - 2026-09-20

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
- Release engineering: `cargo-dist` builds four targets (macOS arm64 and
  x86_64, Linux x86_64 and aarch64 on musl) plus a universal macOS binary,
  with a shell installer and a Homebrew formula. Every artifact is downloaded
  on a runner of its own architecture and run before the release is done, and
  the musl ones are asserted to have no dynamic dependencies.
- `SIGNING.md`, the runbook for signing and notarizing the macOS binaries, and
  a README note on clearing the quarantine attribute meanwhile.
- `docs/adr/0001-language-and-stack.md`, `docs/adr/0002-output-dir-flattens.md`,
  `docs/adr/0003-preset-auto.md`, `docs/adr/0004-no-rotated-rectangles.md`,
  `docs/adr/0005-no-group-merging.md`, `docs/adr/0006-optimizer-is-lossless.md`,
  `docs/adr/0007-serde-for-stats-json.md`,
  `docs/adr/0008-fidelity-is-mean-absolute-error.md`,
  `docs/adr/0009-release-targets-and-tap.md`,
  `docs/shape-detection.md`, `docs/perf.md`, `docs/size.md`.

[Unreleased]: https://github.com/pfurini/vectorise/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/pfurini/vectorise/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/pfurini/vectorise/releases/tag/v0.1.0
