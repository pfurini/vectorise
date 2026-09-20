# `vectorise` — Implementation Plan

**Status:** authoritative plan for the coding harness. Follow it phase by phase.
**Date:** 2026-09-20
**Targets:** macOS (arm64 + x86_64), Linux (x86_64 + aarch64). Windows is explicitly out of scope by design.
**Language:** Rust, edition 2024, single self-contained static binary, zero runtime dependencies.

---

## 0. Ground rules for the harness

Read this section fully before touching code.

1. **TDD is mandatory, not optional.** For every task in every phase: write the failing test first, run it, watch it fail for the *right* reason, then write the minimum implementation to pass, then refactor. Commit after each green. Never write implementation code that has no test driving it, except for the thin `main.rs` shell and the Phase 0 spike.
2. **One phase = one branch = one PR-sized unit of work.** Branch names: `phase-NN-short-slug`. Squash-merge to `main` only when the phase's Definition of Done checklist is fully green.
3. **Verify before you assume.** Several third-party APIs are pinned to pre-release versions. Where this document says `VERIFY`, you must read the actual docs.rs page or crate source for the *pinned* version and record what you found in `docs/api-notes.md` before writing code against it. Do not write code from memory of an older API.
4. **No shelling out. No network at runtime.** The binary must never invoke another executable and must never open a socket. A test in Phase 8 enforces this.
5. **Licensing is a hard constraint.** Only crates licensed MIT, Apache-2.0, BSD-2/3, MPL-2.0, Zlib, ISC, Unicode-3.0, or CC0 may be linked. `cargo deny check licenses` must pass in CI. GPL/LGPL/AGPL crates are forbidden. This is why Potrace is *not* used.
6. **Conventional Commits** (`feat:`, `fix:`, `test:`, `refactor:`, `chore:`, `docs:`, `ci:`). Every commit compiles and passes the test suite.
7. **Lint gate:** `cargo clippy --all-targets --all-features -- -D warnings` with the pedantic groups enabled in `Cargo.toml` (see §3). `cargo fmt --check`. `cargo doc --no-deps` with `-D warnings` for rustdoc. All three run in CI and must be green before a phase is done.
8. **Quality over speed.** If a task's test proves hard to write, that is a design signal: split the unit until it is testable. Do not skip the test.
9. **When blocked** (an API does not exist as described, a crate does not compile on a target, a design assumption is false): stop, write the blocker into `docs/blockers.md` with what you tried and two alternative paths, and choose the alternative this document marks as fallback. Do not silently improvise a fourth path.

---

## 1. What we are building

A command-line tool that converts one or more raster images (PNG, JPEG, WebP, GIF, BMP, TIFF) into **maximally compact, standard-compliant SVG**:

- fewest paths and colors that faithfully reproduce the input, using VTracer's stacking strategy and curve simplification;
- **native SVG primitives** (`<circle>`, `<ellipse>`, `<rect>` incl. rounded) emitted wherever a traced region is geometrically indistinguishable from one within a tolerance, instead of a Bézier `<path>`;
- final minification of the document.

Output file for `foo/bar.png` is `foo/bar.svg` (same directory, same stem, unless `--output-dir` is given). The tool **refuses to run** if any output file already exists or if two inputs would map to the same output, and it checks this for the *whole batch before writing anything*.

### 1.1 Non-goals (v1)

- Windows support.
- Gradients, strokes, text, or any non-solid-fill output.
- A GUI or a long-running service.
- Perfect photographic reproduction. The goal is compactness with tunable fidelity.
- Reimplementing a tracer. We build on VTracer's core and add value around it.

### 1.2 Why Rust and not a self-contained Python tool

Both approaches would call the same VTracer core (the PyPI package is a `pyo3` binding over the identical Rust crate), so raw tracing quality is equal. Rust wins on the two things that matter for *this* tool:

- The Python binding exposes only "image in → SVG string out". The Rust crate exposes the intermediate `VectorDoc` IR with full-precision float geometry. Primitive-shape detection on the IR is exact; doing it in Python means re-parsing the serialized SVG (quantized coordinates, lost structure) and is strictly worse.
- The SVG optimizer (`oxvg`) and the SVG renderer used for verification (`resvg`) are Rust libraries. In Python they would be a Node subprocess and a C dependency respectively, which breaks "self-contained".

---

## 2. Locked technical decisions

| Concern | Decision | License | Notes |
|---|---|---|---|
| Tracing engine | `vtracer = "=1.0.0-alpha.4"` (pin exact) | MIT OR Apache-2.0 | The 0.6.x stable line lacks `VectorDoc`, `--simplify`, `--max-colors`, watershed. Not a fallback. |
| Image decoding | `image = "0.25"` with features `png, jpeg, webp, gif, bmp, tiff` only | MIT OR Apache-2.0 | Disable default features to keep binary small. |
| 2D geometry | `kurbo = "0.13"` | Apache-2.0 OR MIT | `BezPath`, `Shape::area`, `ParamCurveMoments`, `Ellipse`, `Circle`, `Rect`, `RoundedRect`, `simplify`. |
| Raster verification | `tiny-skia = "0.11"` | BSD-3-Clause | Pure Rust, no C toolchain. |
| SVG optimizer | `oxvg_optimiser` (latest on crates.io at Phase 0; pin exact) | MIT OR Apache-2.0 | Rust port of SVGO, library crate. Optional: see Phase 7 fallback. |
| SVG renderer (`--verify`) | `resvg = "0.45"`, `usvg = "0.45"` | MPL-2.0 | File-level copyleft; linking is fine. Match the version vtracer uses in its dev-deps. |
| CLI parsing | `clap = "4"` with `derive`, `wrap_help` | MIT OR Apache-2.0 | |
| Glob expansion | `glob = "0.3"` | MIT OR Apache-2.0 | Fallback only; the shell expands first. |
| Parallelism | `rayon = "1"` | MIT OR Apache-2.0 | |
| Errors (lib) | `thiserror = "2"` | MIT OR Apache-2.0 | Typed errors in the library. |
| Errors (bin) | `anyhow = "1"` | MIT OR Apache-2.0 | Only in `main.rs`. |
| Logging | `tracing = "0.1"`, `tracing-subscriber = "0.3"` | MIT | `RUST_LOG`-style filtering; `-v/-vv` map to levels. |
| Temp files / atomic write | `tempfile = "3"` | MIT OR Apache-2.0 | `NamedTempFile::persist_noclobber`. |
| Test: CLI | `assert_cmd = "2"`, `predicates = "3"` | MIT OR Apache-2.0 | |
| Test: snapshots | `insta = "1"` (`cargo insta`) | Apache-2.0 | SVG outputs as `.snap`. |
| Test: property | `proptest = "1"` | MIT OR Apache-2.0 | Shape detection invariants. |
| Test runner | `cargo-nextest` | MIT OR Apache-2.0 | Faster, per-test isolation. |
| Coverage | `cargo-llvm-cov` | MIT OR Apache-2.0 | Target ≥ 85% line coverage on `src/`. |
| License audit | `cargo-deny` | MIT OR Apache-2.0 | `deny.toml` committed. |
| Vulnerability audit | `cargo-audit` | MIT OR Apache-2.0 | CI, weekly cron. |
| Release | `cargo-dist` | MIT OR Apache-2.0 | Phase 10. |
| Edition / MSRV | `edition = "2024"`, MSRV = the stable toolchain installed at Phase 1; record it in `rust-toolchain.toml` and `Cargo.toml` `rust-version`. | | |

Everything else must be justified in `docs/adr/NNNN-*.md` (Architecture Decision Record) before being added.

---

## 3. Repository layout and conventions

```
.
├── Cargo.toml                 # single package: lib + bin
├── Cargo.lock                 # committed (this is a binary)
├── rust-toolchain.toml
├── deny.toml
├── clippy.toml
├── rustfmt.toml
├── .cargo/config.toml         # static-linking flags for musl, per-target settings
├── justfile                   # developer entry points (test, lint, cov, release)
├── .github/workflows/
│   ├── ci.yml                 # fmt, clippy, deny, nextest, doc, coverage — matrix macOS/Linux
│   └── release.yml            # generated by cargo-dist in Phase 10
├── src/
│   ├── main.rs                # thin: parse args, call lib, map errors to exit codes
│   ├── lib.rs                 # pub API surface, re-exports, crate-level docs
│   ├── cli.rs                 # clap definitions only (no logic)
│   ├── error.rs               # thiserror enums; ExitCode mapping
│   ├── plan.rs                # Phase 2: input resolution, output mapping, preflight
│   ├── decode.rs              # Phase 3: file → ColorImage
│   ├── trace.rs               # Phase 4: options → vtracer Config → VectorDoc
│   ├── geom/
│   │   ├── mod.rs
│   │   ├── convert.rs         # Phase 5: vtracer SubPath ↔ kurbo BezPath
│   │   ├── fit.rs             # Phase 5: moment-based ellipse/rect candidates
│   │   └── verify.rs          # Phase 5: tiny-skia symmetric-difference check
│   ├── shapes.rs              # Phase 5: ShapeFitPass over VectorDoc → ShapeDoc
│   ├── writer.rs              # Phase 6: ShapeDoc → SVG string
│   ├── optimize.rs            # Phase 7: oxvg pass (or fallback)
│   ├── output.rs              # Phase 8: atomic file writing
│   ├── verify.rs              # Phase 9: resvg render + similarity metric
│   └── stats.rs               # Phase 9: counters
├── tests/
│   ├── cli.rs                 # assert_cmd integration tests
│   ├── fixtures/              # tiny generated PNGs (committed, each < 5 KB)
│   └── snapshots/             # insta
├── examples/
│   └── spike.rs               # Phase 0 throwaway; deleted at end of Phase 1
├── benches/                   # optional, criterion, Phase 9
└── docs/
    ├── api-notes.md           # Phase 0 findings (VERIFY items)
    ├── blockers.md
    └── adr/
```

### 3.1 `Cargo.toml` lint configuration (copy exactly)

```toml
[lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"
unreachable_pub = "warn"
unused_qualifications = "warn"

[lints.clippy]
all = { level = "deny", priority = -1 }
pedantic = { level = "warn", priority = -1 }
nursery = { level = "warn", priority = -1 }
cargo = { level = "warn", priority = -1 }
# Deliberate allowances, each justified:
module_name_repetitions = "allow"   # writer::Writer is fine
must_use_candidate = "allow"        # too noisy on builders
missing_errors_doc = "warn"         # keep, document every Err
unwrap_used = "deny"                # in src/; tests may use expect()
expect_used = "warn"
panic = "deny"
indexing_slicing = "warn"
float_cmp = "warn"                  # use approx helpers
```

Test code may relax `unwrap_used`/`expect_used` via `#![allow(clippy::unwrap_used)]` at the top of test modules only.

### 3.2 Public API discipline

`lib.rs` exposes a small, documented, stable surface:

```rust
pub struct Options { /* all tunables, Default impl, builder */ }
pub struct Plan { /* resolved inputs → outputs, validated */ }
pub fn plan(inputs: &[OsString], opts: &PlanOptions) -> Result<Plan, PlanError>;
pub fn convert_one(input: &Path, opts: &Options) -> Result<Converted, ConvertError>;
pub fn run(plan: &Plan, opts: &Options, jobs: usize) -> RunReport;
```

`main.rs` is < 80 lines and contains no business logic. Every module has a `//!` doc comment and every `pub` item a `///` doc comment with at least one `# Examples` block that is a doctest where meaningful.

### 3.3 Test conventions

- Unit tests live in `#[cfg(test)] mod tests` at the bottom of each module.
- Test names: `<unit>_<condition>_<expected>` e.g. `preflight_duplicate_stem_rejects_batch`.
- Fixtures are **generated by code** (`tests/common/gen.rs`) with `image` + `tiny-skia`, then optionally committed as PNG if < 5 KB. Never commit a fixture you cannot regenerate.
- Snapshot tests (`insta`) for every SVG the writer produces from a fixture. Review with `cargo insta review`; never accept a snapshot without reading the diff.
- Property tests (`proptest`) for the geometry module with a fixed seed in CI and 256 cases.
- Floating-point comparisons go through a single `approx_eq(a, b, eps)` helper in `geom/mod.rs`.

---

## 4. Phased plan

Each phase lists: **Goal → Tests first → Implement → DoD (Definition of Done)**.

### Phase 0 — API verification spike (time-box: half a day)

**Goal:** eliminate every assumption about pinned pre-release APIs before writing real code.

**Tasks (no TDD, throwaway code in `examples/spike.rs`):**

1. `cargo add vtracer@=1.0.0-alpha.4 image@0.25` and compile a program that loads a PNG and prints the SVG via `Config::default().build()?.to_svg(&img)?`. Confirm the crate compiles on both macOS and Linux hosts.
2. **VERIFY** and record in `docs/api-notes.md`, with exact field/method names and types as found in the *alpha.4* source (`~/.cargo/registry/src/*/vtracer-1.0.0-alpha.4/src/ir.rs`):
   - `vtracer::ir::VectorDoc`: how to iterate shapes, canvas width/height, paint order.
   - `vtracer::ir::Shape`: fill/paint field, its `MultiPath`.
   - `vtracer::ir::MultiPath` / `SubPath` / `PathCmd`: exact variants (expected: `MoveTo`, `LineTo`, `CubicTo`, `Close`) and coordinate type.
   - `vtracer::ir::Paint`: how to get RGBA.
   - `vtracer::optimize::OptimizerPass`: trait signature.
   - `vtracer::Config`: every public field, their types, and how `simplify`, `max_colors`, `palette`, `path_precision`, `optimize` are spelled.
   - `vtracer::Pipeline::segment` / `finish` signatures.
   - Whether `ColorImage` expects premultiplied alpha, and what the tracer does with transparent pixels.
3. `cargo add oxvg_optimiser` (latest) and **VERIFY**: (a) can it be called as a library on an SVG string; (b) can individual jobs be enabled/disabled programmatically; (c) does it compile without pulling in anything license-incompatible (`cargo deny check`); (d) binary size delta. Record the answer as **GO** or **FALLBACK** for Phase 7.
4. `cargo add kurbo@0.13 tiny-skia@0.11` and confirm: `kurbo::ParamCurveMoments` is available on `CubicBez`/`Line`/`PathSeg`; `Shape::area()` sign convention (winding); `tiny_skia::Pixmap` fill of a `PathBuilder` path.
5. Check crate name availability: if `vectorise` is taken on crates.io, choose `vectorise-cli` for the package and keep the binary name `vectorise`. Record in api-notes.

**DoD:** `docs/api-notes.md` exists with every VERIFY item answered by citing a file path and line in the vendored crate source. A GO/FALLBACK decision for oxvg is recorded. No production code written.

---

### Phase 1 — Skeleton, tooling, CI

**Goal:** an empty-but-complete project where `just check` runs fmt, clippy, deny, doc, and one trivial test, on both OSes in CI.

**Tests first:**
- `tests/cli.rs::binary_runs_and_prints_version`: `vectorise --version` exits 0 and prints the crate version.
- `tests/cli.rs::no_args_is_usage_error`: exits 64, prints usage to stderr.

**Implement:**
- `cargo init --lib` then add `[[bin]]`. Fill `Cargo.toml` from §2 and §3.1 (`rust-version`, `description`, `license = "MIT OR Apache-2.0"`, `repository`, `categories`, `keywords`, `[profile.release] lto = "fat", codegen-units = 1, strip = true, panic = "abort"`).
- `rust-toolchain.toml` (`channel = "<stable version found>"`, `components = ["rustfmt", "clippy"]`, `profile = "minimal"`).
- `rustfmt.toml`: `edition = "2024"`, `imports_granularity = "Module"`, `group_imports = "StdExternalCrate"` (note: last two are nightly-only options; if using stable, omit them and document).
- `clippy.toml`: `cognitive-complexity-threshold = 15`, `too-many-arguments-threshold = 6`.
- `deny.toml`: allow-list from §0.5; deny `GPL-*`, `LGPL-*`, `AGPL-*`; `[bans] multiple-versions = "warn"`.
- `.cargo/config.toml`: `[target.x86_64-unknown-linux-musl] rustflags = ["-C", "target-feature=+crt-static"]` and same for aarch64 musl.
- `justfile` recipes: `check` (fmt-check, clippy, deny, doc), `test` (nextest), `cov`, `spike`, `fixtures` (regenerate), `release-dry`.
- `.github/workflows/ci.yml`: matrix `[ubuntu-latest, macos-latest]`, cache with `Swatinem/rust-cache`, steps: fmt, clippy, deny, nextest, doc, llvm-cov upload as artifact. Add a weekly `cargo audit` job.
- `LICENSE-MIT`, `LICENSE-APACHE`, `README.md` (stub with badges), `CHANGELOG.md` (Keep a Changelog format), `.gitignore`.
- `src/main.rs` + `src/cli.rs` with clap derive skeleton exposing only `--version` and required `<INPUT>...`.
- ExitCode enum in `error.rs`: `Ok = 0, Partial = 1, Preflight = 2, Usage = 64` (BSD `EX_USAGE`), `Io = 74` (`EX_IOERR`).

**DoD:** CI green on both OSes. `examples/spike.rs` deleted. `docs/adr/0001-language-and-stack.md` written summarizing §1.2 and §2.

---

### Phase 2 — Input resolution, output mapping, preflight

**Goal:** pure, side-effect-light logic that turns argv into a validated `Plan` or a complete list of reasons the batch is rejected. This is the most important correctness surface in the tool; test it exhaustively.

**Types:**
```rust
pub struct PlanOptions { pub output_dir: Option<PathBuf>, pub force: bool }
pub struct Job { pub input: PathBuf, pub output: PathBuf }
pub struct Plan { pub jobs: Vec<Job> }
pub enum PlanProblem {
    OutputExists { output: PathBuf, input: PathBuf },
    DuplicateOutput { output: PathBuf, inputs: Vec<PathBuf> },
    InputNotFound(PathBuf),
    InputIsDirectory(PathBuf),
    UnsupportedExtension(PathBuf),
    GlobMatchedNothing(String),
    OutputDirNotWritable(PathBuf),
}
pub struct PlanError { pub problems: Vec<PlanProblem> }  // always non-empty
```

**Tests first (all in `plan.rs` tests + `tests/cli.rs`, using `tempfile::tempdir`):**

Resolution:
- `resolve_literal_existing_file_is_kept`
- `resolve_glob_fallback_expands_when_literal_missing` (arg `*.png` passed literally, i.e. shell did not expand)
- `resolve_glob_fallback_not_used_when_literal_exists` (a file literally named `*.png` exists → treat as literal)
- `resolve_recursive_glob_double_star_expands`
- `resolve_glob_no_match_is_problem`
- `resolve_results_are_deduplicated_and_sorted` (same file given twice or via two globs → one job; deterministic order)
- `resolve_unsupported_extension_is_problem` (`.txt`, `.svg`, no extension)
- `resolve_extension_match_is_case_insensitive` (`.PNG`, `.Jpeg`)
- `resolve_directory_argument_is_problem` (v1: do not recurse implicitly; user uses `dir/**/*.png`)

Mapping:
- `map_output_same_dir_same_stem_svg_extension`
- `map_output_dir_flattens_into_given_directory`
- `map_output_dir_preserves_stem_only_not_subpath` (document this choice; ADR-0002)
- `map_multi_dot_stem_keeps_inner_dots` (`a.b.png` → `a.b.svg`)

Preflight:
- `preflight_existing_output_rejects_batch`
- `preflight_reports_all_problems_not_just_first` (two existing outputs + one duplicate → three problems in one error)
- `preflight_duplicate_stem_across_extensions_rejects` (`x.png` + `x.jpg` → same `x.svg`)
- `preflight_duplicate_stem_across_dirs_with_output_dir_rejects` (`a/x.png` + `b/x.png` + `--output-dir out`)
- `preflight_force_allows_existing_output_but_still_rejects_duplicates` (`--force` never makes a duplicate-output batch valid)
- `preflight_writes_nothing` (assert tempdir unchanged after a rejected plan)
- `preflight_unwritable_output_dir_is_problem` (chmod 0o500 on Unix)

CLI:
- `cli_dry_run_prints_mapping_and_exits_zero_without_writing`
- `cli_preflight_failure_exits_2_and_lists_every_problem_on_stderr`
- `cli_problem_output_is_stable_and_machine_readable` (one problem per line, `KIND\tPATH[\tPATH...]`)

**Implement:** `plan.rs`. Keep filesystem access behind a tiny `trait Fs { fn exists, is_dir, is_file, can_write_dir }` with a real impl and an in-memory test impl so most tests need no tempdir.

**DoD:** all tests above green; coverage of `plan.rs` ≥ 95%; ADR-0002 (output-dir flattening) written; `--dry-run` works end to end.

---

### Phase 3 — Decoding

**Goal:** `decode(path) -> Result<ColorImage, DecodeError>` producing exactly what vtracer expects.

**Tests first:**
- `decode_png_rgba_dimensions_and_pixels_roundtrip` (generate 3×2 image with known pixels, encode PNG in-memory, decode, compare bytes)
- `decode_jpeg_yields_opaque_alpha_255`
- `decode_grayscale_png_expands_to_rgba`
- `decode_palette_png_expands_to_rgba`
- `decode_16bit_png_downconverts_to_8bit`
- `decode_webp_lossless_roundtrip`, `decode_gif_first_frame`, `decode_bmp`, `decode_tiff`
- `decode_corrupt_file_is_typed_error_not_panic`
- `decode_zero_size_image_is_error`
- `decode_alpha_policy_matches_api_notes` — encode what Phase 0 found: if vtracer keys transparent pixels itself, pass through; if not, composite onto `--background` (default white) here. Test both branches only for the branch that applies; document the other.
- `decode_exif_orientation_is_applied` (JPEG with orientation tag 6 → rotated dimensions). If `image` 0.25 does not auto-apply EXIF, implement via `image::metadata::Orientation`; record in api-notes.

**Implement:** `decode.rs`. Restrict `image` features to the six formats. Reject anything else at plan time (Phase 2 already does by extension) and again here by content.

**DoD:** tests green; `image` default features disabled; binary size recorded in `docs/size.md` as a baseline.

---

### Phase 4 — Tracing wrapper

**Goal:** `trace(&ColorImage, &TraceOptions) -> Result<VectorDoc, TraceError>` mapping our CLI options onto `vtracer::Config`.

**`TraceOptions`** (all `Option<_>` so presets fill defaults): `preset: Preset {Bw, Poster, Photo, Auto}`, `clustering`, `hierarchical`, `mode`, `filter_speckle`, `color_precision`, `gradient_step`, `simplify: Option<f64>`, `max_colors: Option<u16>`, `palette: Option<Vec<Rgb>>`, `path_precision: u8`, `corner_threshold`, `segment_length`, `splice_threshold`, `threshold`, `adaptive`.

**Tests first:**
- `config_from_default_options_equals_vtracer_default` (field-by-field; guards against silent drift when bumping vtracer)
- `config_preset_poster_sets_expected_fields`
- `config_explicit_field_overrides_preset`
- `config_palette_parses_hex_with_and_without_hash_and_rejects_bad`
- `trace_solid_color_image_yields_one_shape_one_color`
- `trace_two_rects_side_by_side_yields_two_shapes_two_colors`
- `trace_max_colors_2_on_four_color_image_yields_at_most_two_paints`
- `trace_simplify_reduces_command_count_vs_unsimplified` (same fixture, count `PathCmd`s; assert strictly fewer)
- `trace_transparent_background_does_not_produce_background_shape` (depends on Phase 0/3 alpha policy)
- `trace_1x1_image_does_not_panic`
- `trace_large_image_respects_cancel_token` (optional if `CancelToken` is easy; otherwise defer)

**Implement:** `trace.rs`. Do not re-export vtracer types from `lib.rs`; wrap them.

**DoD:** tests green; `Preset::Auto` heuristic documented (v1: `Auto` = `Poster`; ADR-0003 explains and lists the signal you would use later, e.g. unique-color count).

---

### Phase 5 — Geometry and primitive detection (the core value-add)

**Goal:** `ShapeFitPass` that walks a `VectorDoc` and produces a `ShapeDoc`:

```rust
pub enum Prim {
    Circle { cx: f64, cy: f64, r: f64 },
    Ellipse { cx: f64, cy: f64, rx: f64, ry: f64, rotate_deg: f64 },   // rotate 0 → no transform attr
    Rect { x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64 },        // rx=ry=0 → sharp
    Path(kurbo::BezPath),                                             // fallback, may have holes
}
pub struct ShapeEl { pub prim: Prim, pub fill: Rgba }
pub struct ShapeDoc { pub width: u32, pub height: u32, pub shapes: Vec<ShapeEl> }
```

**Rules:**
- Only a `MultiPath` with exactly one subpath (no holes) is a primitive candidate. Anything with holes stays `Path`.
- Tolerance is expressed in **pixels²** of symmetric difference relative to the region area: `--shape-tolerance` is a fraction (default `0.02` = 2% of area). Also an absolute floor of 1.0 px² so tiny regions are not over-eagerly converted.
- A subpath smaller than `--min-shape-area` (default 16 px²) is never converted.
- Rotated ellipses: emit `transform="rotate(θ cx cy)"` only if `|θ| > 0.5°` and the ellipse is not a circle; otherwise snap to axis-aligned.
- Rect detection requires: all segments `LineTo`, 4 distinct corners after collinear-cleanup, each interior angle within 1° of 90°, and edges axis-aligned within 0.5°. Rotated rects are **not** converted in v1 (ADR-0004).
- Rounded rect: a subpath whose straight edges are axis-aligned and whose 4 corners are cubic arcs of equal radius (within tolerance). Verify via raster check like everything else.

**Tests first — `geom/convert.rs`:**
- `subpath_to_bezpath_preserves_command_sequence`
- `bezpath_to_subpath_roundtrip_is_identity_within_1e_9`
- `multipath_with_hole_is_not_candidate`

**Tests first — `geom/fit.rs` (analytic):**
- `moments_of_axis_aligned_ellipse_bezpath_recover_center_and_radii` (build with `kurbo::Ellipse::to_path(1e-3)`, compute area + second moments, recover `rx, ry` within 0.5%)
- `moments_of_rotated_ellipse_recover_angle_mod_180`
- `area_sign_is_normalized_regardless_of_winding`
- `rect_detector_accepts_axis_aligned_square`
- `rect_detector_rejects_rotated_square`
- `rect_detector_rejects_trapezoid`
- `rect_detector_accepts_after_collinear_midpoint_cleanup` (5-point rect with a midpoint on one edge)
- `rounded_rect_detector_recovers_radius`

**Tests first — `geom/verify.rs` (raster):**
- `symmetric_difference_of_identical_paths_is_zero`
- `symmetric_difference_of_circle_vs_inscribed_square_matches_analytic_area_within_2pct`
- `verify_scales_bbox_to_at_most_512px_and_scales_result_back` (result is in original px², not raster px²)
- `verify_handles_subpixel_shapes_without_panic`

**Tests first — `shapes.rs` (the pass), driven by end-to-end fixtures generated with `tiny-skia`:**
- `pass_converts_traced_circle_r40_to_circle_within_1px`
- `pass_converts_traced_ellipse_60x30_to_ellipse`
- `pass_converts_traced_rect_to_rect_exact`
- `pass_converts_traced_rounded_rect_to_rect_with_rx`
- `pass_keeps_star_as_path`
- `pass_keeps_donut_as_path` (hole)
- `pass_keeps_circle_when_tolerance_is_zero`
- `pass_keeps_tiny_blob_below_min_area_as_path`
- `pass_preserves_paint_order_and_colors`
- `pass_circle_snaps_when_rx_ry_differ_less_than_tolerance`
- `pass_rotated_ellipse_emits_rotation_when_above_half_degree`

**Property tests (`proptest`, 256 cases, seeded):**
- `prop_random_axis_aligned_ellipse_rasterized_then_traced_is_detected` (rx, ry ∈ [8, 200], canvas margin 4px)
- `prop_random_rect_is_detected`
- `prop_random_polygon_with_5_to_9_irregular_vertices_is_not_detected_as_rect_or_ellipse` (reject rate must be 100%)
- `prop_detection_is_translation_invariant` (shift by integer offset → same prim ± offset)

**Implement:** in this order: `convert.rs` → `fit.rs` → `verify.rs` → `shapes.rs`. The pass must be **pure** (no I/O) and deterministic. Expose `ShapeFitOptions { tolerance_frac, min_area, allow_rotation, detect: {circle, ellipse, rect, rounded_rect} }`.

**DoD:** all tests and property tests green; a small table in `docs/shape-detection.md` documenting the algorithm, the tolerance semantics, and known false-negative classes (e.g., ellipses whose traced outline has a corner artifact from `corner_threshold`).

---

### Phase 6 — SVG writer

**Goal:** `write_svg(&ShapeDoc, &WriterOptions) -> String` producing minimal, valid SVG 1.1/2.0.

**Output contract:**
- `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 W H">` — no `width`/`height` attributes by default (responsive); `--keep-size` adds them.
- Fill as `#rrggbb`; shortest form `#rgb` when exact; `fill-opacity` only when alpha < 255.
- Coordinates rounded to `precision` decimals with trailing zeros stripped (`10.50` → `10.5`, `10.0` → `10`).
- Paths use relative commands and shorthands (`h`, `v`, `s`) where they shorten output; implement a small encoder with tests, or reuse vtracer's if exposed (VERIFY). No whitespace after flags or before negatives (`M10-5`).
- No XML prolog, no comments, no metadata, no `<g>` unless it saves bytes (v1: never emit `<g>`).
- Consecutive shapes with identical fill are **not** merged in v1 (paint order must be preserved; ADR-0005).

**Tests first:**
- `writer_empty_doc_emits_only_svg_root_with_viewbox`
- `writer_circle_element_attributes_are_minimal_and_ordered` (`<circle cx="50" cy="50" r="40" fill="#f00"/>`)
- `writer_ellipse_with_rotation_emits_transform`
- `writer_rect_rounded_emits_rx_only_when_rx_eq_ry`
- `writer_path_uses_relative_commands_and_shorthands`
- `writer_number_formatting_strips_trailing_zeros_and_leading_zero` (`0.5` → `.5`)
- `writer_number_precision_respected`
- `writer_fill_shortens_to_3_digit_hex_when_possible`
- `writer_fill_opacity_emitted_only_for_alpha_lt_255`
- `writer_output_parses_with_usvg` (validity oracle: `usvg::Tree::from_str` succeeds and node count matches)
- Snapshot tests: one `.snap` per fixture from Phase 5 (`cargo insta`).

**DoD:** green; every snapshot reviewed; the writer has zero dependencies on vtracer types (it only knows `ShapeDoc`).

---

### Phase 7 — Optimizer pass (conditional on Phase 0 GO/FALLBACK)

**Goal:** last-mile byte reduction without ever undoing Phase 5.

**If GO (oxvg_optimiser usable as library):**
- Configure an explicit job list. **Disabled, always:** `convertShapeToPath`, `mergePaths` (would merge primitives with paths), `removeViewBox`, `removeUnknownsAndDefaults` if it strips `fill`, `sortAttrs` (cosmetic, skip). **Enabled:** `convertPathData`, `cleanupNumericValues`, `convertColors`, `collapseGroups`, `convertEllipseToCircle`, `removeEmptyContainers`, `removeUselessStrokeAndFill`.
- Tests first:
  - `optimize_never_converts_circle_to_path` (input has `<circle>`, output still has `<circle>`)
  - `optimize_output_is_not_larger_than_input` (byte length ≤; if a case appears where it is larger, keep the smaller — test that too: `optimize_keeps_smaller_of_input_and_output`)
  - `optimize_output_renders_identically` (resvg render before/after, per-pixel exact match at 2× scale)
  - `optimize_idempotent` (running twice equals running once)

**If FALLBACK:** skip oxvg entirely. Rely on vtracer's own `optimize` level plus our writer's encoder; add a single `--no-optimize` flag that is a no-op in this configuration and document that. Write ADR-0006 recording why.

**DoD:** green; ADR-0006 written either way (GO records the exact job list and versions).

---

### Phase 8 — Atomic output, concurrency, CLI wiring

**Goal:** end-to-end `vectorise a.png b.jpg` works, is safe under interruption, and is parallel.

**Tests first (`tests/cli.rs` unless noted):**
- `cli_converts_single_png_next_to_input`
- `cli_converts_batch_in_parallel_and_all_outputs_exist`
- `cli_output_dir_flag_places_outputs_there`
- `cli_refuses_when_output_exists_and_writes_nothing_else` (batch of 3, one collision → zero files written, exit 2)
- `cli_force_overwrites`
- `output_write_is_atomic_no_partial_file_on_failure` (unit test in `output.rs`: inject a writer error mid-way; assert no `*.svg` and no leftover temp file in dir)
- `output_uses_create_new_semantics_to_beat_toctou` (create the target between plan and write → typed `AlreadyExists` error, exit 1 for that file, others succeed, final exit 1)
- `cli_partial_failure_exit_code_1_and_reports_per_file` (one corrupt input in a batch)
- `cli_jobs_1_is_sequential_and_deterministic_order_of_log_lines`
- `cli_quiet_suppresses_progress_but_not_errors`
- `cli_verbose_enables_tracing_at_debug`
- `cli_sigint_leaves_no_temp_files` (spawn, send SIGINT after first output appears, assert no `.tmp` files; Unix only)
- `cli_no_network_and_no_subprocess` (Linux only: run under `strace -f -e trace=execve,connect,socket` if available in CI; skip with a clear reason otherwise)
- `cli_help_text_snapshot` (insta snapshot of `--help`; catches accidental UX changes)

**Implement:**
- `output.rs`: write to `NamedTempFile` in the target directory → `persist_noclobber` (or `persist` when `--force`) → fsync file, then fsync directory on Linux.
- `run()` in `lib.rs`: `rayon` thread pool sized by `--jobs`, collects a `RunReport { ok: Vec<Job>, failed: Vec<(Job, ConvertError)> }`. Progress to stderr via a minimal counter (no `indicatif` in v1 to keep deps small; ADR-0007 if you disagree).
- Exit-code mapping in `main.rs`.

**DoD:** green on both OSes; `cli_help_text_snapshot` reviewed; README "Usage" section written from the real `--help`.

---

### Phase 9 — Verification, stats, performance

**Goal:** give users a fidelity number and an accounting of what the tool did.

**`--verify`:** render the produced SVG with `resvg` at the input's pixel size and compute a similarity score against the decoded input. v1 metric: mean absolute error over RGB (0–1, reported as `fidelity = 1 - MAE`). Add SSIM later (ADR-0008) if a permissive-license crate meets the bar; do not hand-roll SSIM in v1.

**`--stats`:** per file and totals: input bytes, output bytes, ratio, colors, shapes, primitives by kind, path commands, elapsed ms. `--stats-json` emits one JSON object per line (use `serde` + `serde_json`, add to §2 table with ADR).

**Tests first:**
- `verify_identical_render_scores_1_0`
- `verify_inverted_render_scores_near_0`
- `verify_metric_is_symmetric`
- `stats_counts_match_writer_output` (parse the emitted SVG with `usvg` and count elements)
- `stats_json_lines_are_valid_and_schema_stable` (insta snapshot of one line with elapsed zeroed)
- Benchmarks (criterion, not gating): trace 512×512 poster, shape-pass on 200 subpaths, full pipeline on fixture set. Record numbers in `docs/perf.md`.

**Performance targets (release build, Apple M-series or comparable x86):** a 1024×1024 flat-color illustration end-to-end in < 1.5 s single-threaded; shape pass < 5% of total time.

**DoD:** green; `docs/perf.md` has baseline numbers from both CI runners.

---

### Phase 10 — Release engineering

**Goal:** reproducible, downloadable, self-contained binaries.

**Tasks:**
- `cargo dist init` with targets `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`. Enable the shell installer and Homebrew formula generation (tap repo name in ADR-0009).
- Add a post-build step producing a macOS **universal** binary via `lipo -create` and ship it as an additional artifact.
- Verify the musl binaries are static: `file` shows "statically linked"; `ldd` reports "not a dynamic executable". Add a CI assertion.
- Document macOS Gatekeeper: unsigned binaries downloaded via browser are quarantined. Provide `xattr -d com.apple.quarantine` guidance in README, and a `SIGNING.md` describing Developer ID signing + notarization (`codesign`, `notarytool`) as an operator step; do not commit credentials or attempt it in CI unless secrets are configured.
- `cargo publish --dry-run` passes.
- Tag `v0.1.0`; `CHANGELOG.md` updated; GitHub release created by `release.yml`.

**Tests first:**
- `release_binary_has_no_dynamic_deps_on_linux_musl` (CI-only script test)
- `release_binary_runs_help_on_each_target` (smoke, in `release.yml`)

**DoD:** four artifacts + universal macOS binary attached to a GitHub release; `brew install <tap>/vectorise` works on a clean macOS runner; `curl | sh` installer works on a clean Ubuntu runner.

---

## 5. CLI specification (authoritative)

```
vectorise [OPTIONS] <INPUT>...

Arguments:
  <INPUT>...  Raster files. Globs are normally expanded by your shell; a literal
              pattern (e.g. quoted '**/*.png') is expanded by vectorise itself.

Output:
  -o, --output-dir <DIR>      Write all outputs into DIR (flattened). Default: next to input
  -f, --force                 Overwrite existing outputs (duplicate outputs still fail)
  -n, --dry-run               Print the input → output plan and exit
      --keep-size             Emit width/height attributes on <svg>

Tracing (see vtracer docs):
      --preset <bw|poster|photo|auto>   Default: auto
      --clustering <color-cluster|bw|watershed>
      --hierarchical <stacked|cutout>
  -m, --mode <pixel|polygon|spline>
      --filter-speckle <0..=128>
      --color-precision <1..=8>
      --gradient-step <0..=255>
      --max-colors <N>
      --palette <#rrggbb,...> | --palette-file <FILE>
      --simplify <PX>          Curve simplification tolerance (try 1–2.5)
      --threshold <0..=255> | --adaptive
      --watershed-detail <N>

Shapes:
      --shapes <auto|off>              Default: auto
      --shape-tolerance <FRACTION>     Default: 0.02
      --min-shape-area <PX2>           Default: 16
      --no-rotated-ellipses

Output quality:
  -p, --precision <0..=6>     Coordinate decimals. Default: 2
      --no-optimize           Skip the optimizer pass
      --verify                Render result and report fidelity
      --stats | --stats-json

Runtime:
  -j, --jobs <N>              Default: available parallelism
  -q, --quiet
  -v, --verbose...            -v info, -vv debug, -vvv trace
  -h, --help
  -V, --version

Exit codes:
  0  all inputs converted
  1  one or more inputs failed (others were written)
  2  preflight rejected the batch; nothing was written
  64 usage error
  74 I/O error outside per-file conversion (e.g. cannot create --output-dir)
```

---

## 6. Error model

- Library errors are typed enums (`PlanError`, `DecodeError`, `TraceError`, `ShapeError` (rare; internal invariants), `WriteError`, `OptimizeError`, `VerifyError`) composed into `ConvertError` with `#[from]`. Every variant carries the input path where relevant.
- `main.rs` converts to exit codes via a single `impl From<&RunOutcome> for ExitCode`.
- Never `panic!` in `src/` (clippy `panic = "deny"`). Internal invariant violations return `ShapeError::Invariant(&'static str)` and are logged at `error`; the file fails, the batch continues.
- Messages to stderr are one line per event, prefixed `error:`, `warn:`, or `info:`; paths are printed as given by the user (not canonicalized) for readability.

---

## 7. Definition of Done — whole project

- [ ] All phases merged; CI green on macOS and Linux.
- [ ] `cargo llvm-cov` ≥ 85% lines on `src/`; `plan.rs`, `geom/`, `output.rs` ≥ 95%.
- [ ] `cargo deny check` and `cargo audit` clean.
- [ ] `cargo doc --no-deps` warning-free; `lib.rs` docs include a 10-line usage example that is a doctest.
- [ ] README: install (brew, installer script, cargo), usage, tuning guide with 3 before/after examples, exit codes, license.
- [ ] `docs/` contains: api-notes, shape-detection, perf, size, ADR 0001–0009, blockers (may be empty).
- [ ] Release `v0.1.0` published with 5 artifacts.
- [ ] Manual acceptance on a real machine: a directory of 20 mixed PNG/JPEG logos converts with `vectorise ~/logos/*.{png,jpg} --stats`, second run exits 2 with a full collision list, `--force` succeeds, at least one output contains a `<circle>` or `<rect>`.

---

## 8. Risk register

| Risk | Likelihood | Mitigation |
|---|---|---|
| vtracer alpha API differs from docs | Medium | Phase 0 spike against vendored source; pin `=1.0.0-alpha.4`; commit `Cargo.lock`. If a later alpha breaks the IR, vendor a fork via `[patch.crates-io]`. |
| oxvg not usable as a library / license issue | Medium | Phase 7 FALLBACK path is fully specified. |
| Traced outlines of true circles have corner artifacts → false negatives | Medium | Tune `corner_threshold` default upward when `--shapes auto`; property tests quantify detection rate; document. |
| musl static build fails for a dependency | Low | All chosen crates are pure Rust. Assert in CI early (Phase 1 builds `--target x86_64-unknown-linux-musl` as a smoke job). |
| Gatekeeper blocks unsigned macOS binary | Certain for unsigned | Documented in Phase 10; signing is an operator step. |
| Crate name collision on crates.io | Low | Decided in Phase 0. |

---

## 9. Glossary

- **VectorDoc** — vtracer's post-fit IR: ordered shapes with fitted paths, absolute coordinates.
- **ShapeDoc** — our IR after primitive detection: shapes are either a primitive or a path.
- **Symmetric difference** — area covered by exactly one of two shapes; our fidelity measure for primitive substitution.
- **Preflight** — validation of the entire batch before any output is written.
