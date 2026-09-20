# API notes — Phase 0 verification spike

Every `VERIFY` item from `IMPLEMENTATION_PLAN.md` §Phase 0, answered against the
vendored source of the pinned versions. Nothing here is from memory.

- **Date:** 2026-09-20
- **Host:** macOS 15 (Darwin 24.6.0), `aarch64-apple-darwin`, rustc 1.98.1
- **Spike:** `examples/spike.rs` (deleted at the end of Phase 1); its output is
  quoted verbatim below under "Observed".

Source roots (`$R` = `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f`):

| Crate | Version | Path |
|---|---|---|
| vtracer | 1.0.0-alpha.4 | `$R/vtracer-1.0.0-alpha.4` |
| visioncortex | 0.9.3 | `$R/visioncortex-0.9.3` |
| kurbo | 0.13.1 | `$R/kurbo-0.13.1` |
| tiny-skia | 0.11.4 | `$R/tiny-skia-0.11.4` |
| image | 0.25.10 | `$R/image-0.25.10` |
| oxvg_optimiser | 0.0.8 | `$R/oxvg_optimiser-0.0.8` |
| oxvg_ast | 0.0.8 | `$R/oxvg_ast-0.0.8` |

---

## 1. vtracer compiles and traces

`cargo build --example spike` succeeds on `aarch64-apple-darwin` with rustc
1.98.1. `Config::default().build()?.to_svg(&img)?` returns an SVG string.
Linux is verified in CI (Phase 1), not on this host.

Observed on a 128×128 fixture (white background, red circle r=32, blue 40×24
rect):

```
== default: 128x128 shapes=3 subpaths=3 multi-subpath=0 colors=3 cmds=21
== to_svg len=665 head="<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- Generator: visioncortex VTracer 1.0.0-alpha.4 -->\n<svg version=\"1.1\" ... width=\"128\" height=\"128\">"
```

---

## 2. `vtracer::ir` — the IR we build on

### 2.1 `VectorDoc` (`src/ir/vector.rs:74-90`)

```rust
pub struct VectorDoc {
    pub width: u32,
    pub height: u32,
    /// Shapes in paint order (first drawn is bottom).
    pub shapes: Vec<Shape>,
}
```

Iterate shapes with `doc.shapes.iter()`. Canvas size is `doc.width` / `doc.height`
as `u32`. Paint order is bottom-to-top: index 0 is drawn first, later shapes
paint over it (`src/ir/vector.rs:78`). The frontend reverses visioncortex's
top-to-bottom cluster output to produce it (`src/frontend/color_cluster.rs:91-93`).

`ShapeDoc` in Phase 5 must preserve this order exactly.

### 2.2 `Shape` (`src/ir/vector.rs:67-71`)

```rust
pub struct Shape {
    pub paint: Paint,
    pub path: MultiPath,
}
```

Both fields are public; there are no accessors. The fill is `shape.paint`.

### 2.3 `MultiPath` / `SubPath` / `PathCmd` (`src/ir/vector.rs:5-64`)

```rust
pub enum PathCmd {
    MoveTo(PointF64),
    LineTo(PointF64),
    CubicTo(PointF64, PointF64, PointF64),   // c1, c2, end
    Close,
}
pub struct SubPath  { pub commands: Vec<PathCmd> }
pub struct MultiPath { pub subpaths: Vec<SubPath> }
```

The four variants are exactly what the plan expected. No quadratics, no arcs.
Coordinate type is `visioncortex::PointF64 { x: f64, y: f64 }` — **absolute, in
full-canvas document space**, with any region offset already baked in
(`src/ir/vector.rs:5-6`). No per-path `transform` is ever needed.

`SubPath::start()` returns the `MoveTo` point; `MultiPath::push` silently drops
empty subpaths (`src/ir/vector.rs:59-63`).

Traced subpaths end with `PathCmd::Close` (observed: `last cmd is Close: true`).

**Multi-subpath shapes are not necessarily holes.** With `max_colors = 2` the
spike produced one shape whose two subpaths had signed areas `+3265.09` and
`+960.00` — two disjoint outer rings of the same color, not a ring plus a hole
(a hole would have the opposite sign). The Phase 5 rule "only a single-subpath
`MultiPath` is a primitive candidate" therefore also rejects disjoint same-color
rings. That is a known false-negative class for `docs/shape-detection.md`, not a
correctness problem.

### 2.4 `Paint` (`src/ir.rs:20-33`)

```rust
pub enum Paint { Solid(Color) }
impl Paint { pub fn color(&self) -> Color }
```

`Solid` is the only variant today. `visioncortex::Color`
(`visioncortex-0.9.3/src/color.rs:9-13`) is `{ pub r: u8, pub g: u8, pub b: u8,
pub a: u8 }`, so RGBA is `shape.paint.color()` and its four public fields.
Constructors: `Color::new(r, g, b)` (alpha 255) and `Color::new_rgba(r, g, b, a)`
(`color.rs:58-63`).

In practice traced paints are opaque: every color observed had `a = 255`.

### 2.5 `Segmentation` (`src/ir/region.rs`)

Re-exported as `vtracer::Segmentation` alongside `Layer` and `RegionMask`
(`src/ir.rs:15`). We do not use it in v1; it is the cache for `Session`-style
interactive tuning.

---

## 3. `vtracer::optimize::OptimizerPass` (`src/optimize.rs:16-19`)

```rust
pub trait OptimizerPass {
    fn run(&self, doc: &mut VectorDoc);
}
```

In-place rewrite, no error channel, no context. Two built-ins:

| Pass | Source | Effect |
|---|---|---|
| `QuantizePass { precision: u32 }` | `src/optimize.rs:22-64` | rounds every coordinate to `precision` decimals |
| `CleanupPass` | `src/optimize.rs:66-153` | drops zero-length (`1e-6`) and collinear (`1e-4`) line segments, then empty subpaths and empty shapes |

**Consequence for Phase 4 (important).** `Config::optimizers`
(`src/config.rs:278-287`) always builds `[QuantizePass::new(path_precision.unwrap_or(2)), CleanupPass]`
whenever `optimize != 0`, so the `VectorDoc` we receive is **already quantized to
2 decimals by default**. Primitive fitting wants full-precision geometry, and we
still want `CleanupPass`. Therefore `trace.rs` sets `config.path_precision =
Some(6)` internally and lets our own writer do the user-facing rounding
(`--precision`, default 2). Setting `optimize = 0` is the wrong lever: it
disables `CleanupPass` too.

---

## 4. `vtracer::Config` (`src/config.rs:86-165`)

Every field is public. Types and defaults as found in the source:

| Field | Type | Default | Our CLI flag |
|---|---|---|---|
| `clustering` | `Clustering` | `ColorCluster` | `--clustering` |
| `hierarchical` | `Hierarchical` | `Stacked` | `--hierarchical` |
| `filter_speckle` | `usize` | `4` | `--filter-speckle` |
| `color_precision` | `i32` | `6` | `--color-precision` |
| `layer_difference` | `i32` | `16` | `--gradient-step` |
| `mode` | `FitMode` | `Spline` | `-m/--mode` |
| `corner_threshold` | `i32` (degrees) | `60` | `--corner-threshold` |
| `length_threshold` | `f64` (px) | `4.0` | `--segment-length` |
| `max_iterations` | `usize` | `10` | not exposed in v1 |
| `splice_threshold` | `i32` (degrees) | `45` | `--splice-threshold` |
| `simplify` | `Option<f64>` (px) | `None` | `--simplify` |
| `path_precision` | `Option<u32>` | `Some(2)` | internal (see §3) |
| `palette` | `Vec<Color>` | `[]` | `--palette` / `--palette-file` |
| `max_colors` | `Option<usize>` | `None` | `--max-colors` |
| `optimize` | `u8` (0/1/2) | `1` | internal |
| `binary_threshold` | `u8` | `128` | `--threshold` |
| `binary_adaptive` | `bool` | `false` | `--adaptive` |
| `binary_adaptive_window` | `u32` (0 = auto) | `0` | not exposed in v1 |
| `binary_adaptive_t` | `f64` (percent) | `15.0` | not exposed in v1 |
| `watershed_detail` | `u32` | `128` | `--watershed-detail` |

Spellings differ from the plan's `TraceOptions` sketch. The mapping above is
authoritative for Phase 4: the plan's `gradient_step` is `layer_difference`, its
`segment_length` is `length_threshold`, its `threshold` is `binary_threshold`,
and its `adaptive` is `binary_adaptive`.

Semantics worth knowing:

- `filter_speckle` is a **side length**; the area threshold is its square
  (`speckle_area`, `src/config.rs:229-231`).
- `color_precision` is inverted internally: `color_precision_loss = 8 - color_precision`
  (`src/config.rs:202`).
- `palette` takes priority over `max_colors`; either one appends a
  `MergeAdjacent` color fitter (`src/config.rs:233-244`).
- `simplify` only has an effect in `FitMode::Spline`, and only when
  `> 0.0` (`src/config.rs:268-276`, `src/config.rs:111-115`).
- Enums: `Clustering { ColorCluster, Binary, Watershed }`,
  `Hierarchical { Stacked, Cutout }`, `FitMode { Pixel, Polygon, Spline }`,
  `Preset { Bw, Poster, Photo }` — **there is no `Preset::Auto`**; ours is a
  vectorise-level concept (ADR-0003). All four implement `FromStr`
  (`src/config.rs:369-414`) with the spellings the CLI uses.

`Config::from_preset` (`src/config.rs:170-188`), observed:

```
== preset Bw:     clustering=Binary       color_precision=6 filter_speckle=4  layer_difference=16 corner_threshold=60  mode=Spline
== preset Poster: clustering=ColorCluster color_precision=8 filter_speckle=4  layer_difference=16 corner_threshold=60  mode=Spline
== preset Photo:  clustering=ColorCluster color_precision=8 filter_speckle=10 layer_difference=48 corner_threshold=180 mode=Spline
```

Observed effect of the knobs on the fixture:

```
== simplify=None: shapes=3 cmds=21
== simplify=1.0:  shapes=3 cmds=17
== simplify=2.5:  shapes=3 cmds=17
== max_colors=2:  shapes=2 colors=2 (one shape gained a second subpath)
== palette=[white,black]: shapes=2 colors=2 cmds=12
```

`simplify` does reduce the command count, so the Phase 4 test
`trace_simplify_reduces_command_count_vs_unsimplified` is satisfiable with
`simplify = 1.0` on this fixture.

---

## 5. `vtracer::Pipeline` (`src/pipeline.rs`)

```rust
pub fn run(&self, img: &ColorImage) -> Result<VectorDoc, Error>;                      // :34
pub fn run_with_progress(&self, img: &ColorImage, cancel: &CancelToken,
                         on_progress: &mut dyn FnMut(Progress)) -> Result<VectorDoc, Error>; // :45
pub fn segment(&self, img: &ColorImage) -> Result<Segmentation, Error>;               // :66
pub fn segment_with_progress(&self, img: &ColorImage, cancel: &CancelToken,
                             on_progress: &mut dyn FnMut(Progress)) -> Result<Segmentation, Error>; // :71
pub fn finish(&self, seg: &Segmentation) -> Result<VectorDoc, Error>;                 // :89
pub fn finish_with_progress(&self, seg: &Segmentation, cancel: &CancelToken,
                            on_progress: &mut dyn FnMut(Progress)) -> Result<VectorDoc, Error>; // :95
pub fn to_svg(&self, img: &ColorImage) -> Result<String, Error>;                      // :129
```

Phase 4 uses `Config::build()?` then `Pipeline::run(&img)` and never touches
`to_svg`. `segment`/`finish` exist for interactive tuning; v1 has no use for
them (one image, one config, one pass).

`CancelToken` is `vtracer::CancelToken` (`src/progress.rs`, re-exported at
`src/lib.rs:101`) and is cheap to pass, so the optional Phase 4 test
`trace_large_image_respects_cancel_token` is feasible via `run_with_progress`.

`vtracer::Error` (`src/error.rs:6-18`) is a plain, `Clone + PartialEq` enum:
`EmptyImage`, `NoKeyColor`, `Unsupported(String)`, `Cancelled`, `Other(String)`.
It implements `std::error::Error`, so `thiserror`'s `#[from]` works.

---

## 6. `ColorImage` and the alpha policy

### 6.1 Layout

`visioncortex::ColorImage` (`visioncortex-0.9.3/src/image/format.rs:27-33`):

```rust
pub struct ColorImage { pub pixels: Vec<u8>, pub width: usize, pub height: usize }
```

Four bytes per pixel, row-major, **straight (non-premultiplied) RGBA**. Nothing
in visioncortex or vtracer multiplies or divides by alpha; `set_pixel` writes the
four `Color` bytes as given (`format.rs:316-327`). So `decode.rs` can hand
`image::RgbaImage::into_raw()` straight through:

```rust
ColorImage { width: w as usize, height: h as usize, pixels: rgba.into_raw() }
```

`ColorImage::new_w_h` zero-fills (`format.rs:273-279`), which means fully
transparent *and* black.

### 6.2 What the tracer does with transparency

Only `ColorClusterFrontend` handles alpha, and it does so automatically
(`src/frontend/color_cluster.rs:60-68`):

1. `should_key_image` samples 5 rows (top, ¼, ½, ¾, bottom) and returns true when
   at least `0.2 × 2 × width` of the sampled pixels have `a == 0`
   (`src/frontend/keying.rs:14-42`).
2. If keyed, `find_unused_color` picks a color absent from the image and
   `apply_key` recolors every `a == 0` pixel to it (`keying.rs:59-105`).
3. The runner gets `keying_action: KeyingAction::Discard`
   (`color_cluster.rs:81`), so that region never becomes a shape.

Consequences, all confirmed by the spike:

```
== alpha: transparent border around red square: shapes=1 colors={(255,0,0,255)}
== alpha: opaque white border around red square: shapes=2 colors={red, white}
```

- Fully transparent pixels **are** keyed out — we must not composite them away.
- Keying is all-or-nothing and threshold-gated. Below ~20% sampled transparency
  nothing is keyed and the `a == 0` pixels are clustered **by their RGB**, which
  for a typical PNG is `(0,0,0)` — a black halo.
- Partial alpha (`0 < a < 255`) is ignored entirely: clustering reads RGB only.
  A 10%-opaque red pixel clusters as full red.
- `BinaryFrontend` and `WatershedFrontend` never look at alpha at all
  (`src/frontend/binary.rs:89` thresholds on intensity; no `.a` reference in
  either file).

**Phase 3 policy (decided here, tested by `decode_alpha_policy_matches_api_notes`):**

| Input pixel | What `decode.rs` emits |
|---|---|
| `a == 255` | unchanged |
| `0 < a < 255` | `src` composited over `--background` (default white), `a = 255` |
| `a == 0` | RGB replaced by `--background`, `a = 0` preserved |

Keeping `a = 0` preserves vtracer's keying; normalizing its RGB makes the
sub-threshold case degrade to the background color instead of a black halo. This
is the "pass through" branch of the plan's either/or, with the partial-alpha gap
closed.

### 6.3 Degenerate sizes

```
== 1x1 ok: shapes=0
== 0x0: Some(EmptyImage)
```

A 1×1 image yields an empty document rather than a panic; 0×0 returns
`Error::EmptyImage` (raised in `ColorClusterFrontend::prepare`,
`src/frontend/color_cluster.rs:52-54`). Phase 3 rejects zero-size images before
tracing anyway.

---

## 7. kurbo 0.13.1

### 7.1 `ParamCurveMoments` is a **path** trait, not a segment trait

The plan expected it on `CubicBez` / `Line` / `PathSeg`. It is not there.
`src/moments.rs:523-528`:

```rust
pub trait ParamCurveMoments<'a> { fn moments(&'a self) -> Moments; }
impl<'a, T: 'a> ParamCurveMoments<'a> for T where &'a T: IntoIterator<Item = PathEl> { ... }
```

So it applies to anything iterable as `PathEl` — `BezPath`, `Circle`, `Ellipse`,
`Rect`, `RoundedRect`. This is better for us: one `bezpath.moments()` call covers
a whole subpath, including its implicit closing line (`moments.rs:559-564`).

`Moments` (`src/moments.rs:15-30`) carries `moment_x`, `moment_y`, `moment_xx`,
`moment_xy`, `moment_yy` — the raw Green's-theorem integrals `∫x dA`, `∫y dA`,
`∫x² dA`, `∫xy dA`, `∫y² dA`. **It does not carry area**; area comes from
`Shape::area()`. Central moments must be derived:

```text
cx = moment_x / A                 cy = moment_y / A
μxx = moment_xx / A - cx²         μyy = moment_yy / A - cy²
μxy = moment_xy / A - cx·cy
```

For an ellipse, `μxx = rx²/4` and `μyy = ry²/4` when axis-aligned, and the
rotation is `θ = ½ · atan2(2μxy, μxx - μyy)`. Verified:

```
== kurbo ellipse rx=20 ry=8: area=502.6570 (pi*rx*ry=502.6548)
   centroid=(30.0000,40.0000) rx'=20.0000 ry'=8.0000 mxy=0.000000
== kurbo rotated ellipse (x_rotation=0.6): recovered theta=0.600000 rad (34.377 deg)
```

Both radii and the angle come back exact to four decimals from
`Ellipse::to_path(1e-4)`, so the Phase 5 test tolerance of 0.5% is comfortable.

### 7.2 `Shape::area()` sign convention

Signed, and it follows the path's winding:

```
== kurbo Circle r=5 area=78.539828 (pi*r^2=78.539816) sign=1
   y-flipped (opposite winding) area=-78.539828
```

`Circle::to_path` is positive. vtracer's traced outlines have the **same** sign:

```
== default: shape[0] signed subpath areas: ["16384.00"]   # 128x128 background, exact
            shape[1] ["3265.09"]                          # circle r=32 (pi*r^2 = 3216.99)
            shape[2] ["960.00"]                           # 40x24 rect, exact
```

`geom/fit.rs` must therefore normalize: take `A = area.abs()` and, when `area <
0`, negate the moment integrals before deriving centroid and central moments.
The plan's test `area_sign_is_normalized_regardless_of_winding` covers this.

Note the circle's traced area exceeds `πr²` by 1.5%: pixel tracing follows the
outside of the boundary pixels, so a rasterized radius-32 disc traces as roughly
radius 32.24. Phase 5 assertions must compare against the *traced* geometry
(±1 px), never against the radius used to draw the fixture.

### 7.3 Everything else the plan needs

| Item | Source | Status |
|---|---|---|
| `Circle::new(center, r)` | `src/circle.rs` | present |
| `Ellipse::new(center, radii: Vec2, x_rotation)` | `src/ellipse.rs:40` | present |
| `Ellipse::radii_and_rotation()` | `src/ellipse.rs:160` | present, useful for round-tripping |
| `Rect::new(x0, y0, x1, y1)` | `src/rect.rs` | present, `area()` = 40 for 10×4 |
| `RoundedRect::new(x0, y0, x1, y1, radii)` | `src/rounded_rect.rs` | present; 10×4 r=1.5 → area 38.0686 = 40 − (4−π)·1.5² ✓ |
| `Shape::to_path(tolerance)` | `src/shape.rs` | present on all of the above |
| `simplify_bezpath(path, tol, &SimplifyOptions)` | `src/simplify.rs:298` | present — signature takes `&SimplifyOptions`, **not** `Option<_>`; 9 elements → 3 at tol 0.5 |

All exported from the crate root (`kurbo-0.13.1/src/lib.rs:154-189`).

---

## 8. tiny-skia 0.11.4

`Pixmap::new(w, h) -> Option<Pixmap>`, `Pixmap::fill(Color)`,
`PathBuilder::{push_circle, push_rect, finish}`, and
`Pixmap::fill_path(&Path, &Paint, FillRule, Transform, Option<&Mask>)` all exist
and behave as the plan assumes. `Paint::anti_alias` is a public field; the
fixture generator sets it to `false` for crisp pixel edges.

Storage is **premultiplied**: `Pixmap::pixels()` yields `PremultipliedColorU8`,
and `.demultiply()` gives straight RGBA. Observed round-trip through `image`:

```
== tiny-skia: center px demul=(255,0,0,255) data_len=65536
== decoded 128x128 pixels=65536
```

`tiny-skia` is pinned to `0.11` rather than the newer `0.12` so it stays shared
with `resvg`/`usvg` 0.45.1, which depend on `tiny-skia 0.11.4`
(`resvg-0.45.1/Cargo.toml:97`, `usvg-0.45.1/Cargo.toml:113`). Using 0.12 would
link two copies.

---

## 9. oxvg — decision: **GO**, with a caveat the user must confirm

### 9.1 (a) Usable as a library

Yes. `oxvg_optimiser::Jobs` is driven through `oxvg_ast`'s arena parser:

```rust
use oxvg_ast::{parse::roxmltree::parse, serialize::Node as _, visitor::Info};
use oxvg_optimiser::Jobs;

let out: String = parse(input, |dom, allocator| {
    let mut jobs = Jobs::default();
    jobs.convert_shape_to_path = None;
    jobs.run(dom, &Info::new(allocator))?;
    dom.serialize()
})?;
```

`parse` is `oxvg_ast::parse::roxmltree::parse<T, F>(&str, F) -> Result<T, ParseError>`
(`oxvg_ast-0.0.8/src/parse/roxmltree.rs:144`), where `F` is a higher-ranked
`FnOnce(Ref, Allocator) -> T`. The arena dies with the closure, so the closure
must return the serialized `String`. `oxvg_ast` needs an explicit
`features = ["roxmltree"]`; `oxvg_optimiser` pulls it in with only
`selectors, serialize, visitor`.

`Jobs::run` is `fn run(&self, root: Ref, info: &Info) -> Result<(), JobsError>`
(`oxvg_optimiser-0.0.8/src/jobs/mod.rs:356`).

### 9.2 (b) Individual jobs enabled/disabled programmatically

Yes, two ways (`src/jobs/mod.rs:34-40`, `:270-285`):

- every job is a public `Option<T>` field on `Jobs`, so `jobs.merge_paths = None`
  disables one and `Some(T::default())` enables it;
- `Jobs::omit("merge_paths")` does the same by snake_case name;
- `Jobs::none()`, `Jobs::default()` (the 36 SVGO-default plugins),
  `Jobs::safe()` are the starting points.

Field names for the plan's Phase 7 job list, all present:
`convert_shape_to_path`, `merge_paths`, `remove_view_box`,
`remove_unknowns_and_defaults`, `sort_attrs`, `convert_path_data`,
`cleanup_numeric_values`, `convert_colors`, `collapse_groups`,
`convert_ellipse_to_circle`, `remove_empty_containers`,
`remove_useless_stroke_and_fill`.

**`convert_shape_to_path` must be disabled — it rewrites `<rect>`:**

```
== oxvg default on <rect> -> <svg ...><path fill="red" d="M1 2h30v40H1Z"/></svg>
```

It leaves `<circle>` alone by default (SVGO only converts circles/ellipses with
`convertArcs`), but a `<rect>` is destroyed, which would undo Phase 5 silently.

With the plan's job list applied, primitives and `viewBox` survive and the pass
is idempotent:

```
== oxvg tuned jobs -> <svg xmlns="..." viewBox="0 0 100 100"><circle cx="50" cy="50" r="40" fill="red"/><path d="M10 10h10v10H10Z" fill="#00f"/></svg>
   circle preserved: true | viewBox preserved: true
   idempotent: true
```

### 9.3 (c) License

Clean. All six `oxvg_*` crates are `MIT`. The transitive additions include eight
`MPL-2.0` crates — `cssparser`, `cssparser-color`, `cssparser-macros`,
`dtoa-short`, `lightningcss`, `lightningcss-derive`, `parcel_selectors`,
`selectors` — and MPL-2.0 is on the allow list in plan §0.5. No GPL/LGPL/AGPL
anywhere in the 190-package graph; every remaining license is MIT, Apache-2.0,
BSD-2/3, ISC, Zlib, Unicode-3.0, or an `OR` expression containing one of those.
`cargo deny check licenses` runs in CI from Phase 1.

### 9.4 (d) Binary size delta — the caveat

Measured with the plan's release profile (`lto = "fat"`, `codegen-units = 1`,
`strip = true`, `panic = "abort"`), same spike binary with and without the oxvg
call graph:

| Build | Size |
|---|---|
| without oxvg | 1 544 176 B (1.47 MiB) |
| with oxvg | 8 890 880 B (8.48 MiB) |
| **delta** | **+7 346 704 B (+476%)** |

The cost is `lightningcss` + `parcel_selectors` + `regex` + `phf`: a full CSS
parser and selector engine, none of which our output uses (we emit no `<style>`,
no classes, no ids).

Benefit, measured on a real vtracer document reduced to what our Phase 6 writer
will emit (no XML prolog, no generator comment, `viewBox` instead of
`width`/`height`):

```
== oxvg on vtracer output: 549 -> 389 bytes (-29.1%)
```

So: **−29% output bytes for +7.3 MB of binary.** Both sides of the plan's
constraint set are real (plan §1 "maximally compact SVG" vs. §2 "keep binary
small"). Recorded decision: **GO**, because the plan's primary goal is output
compactness and the FALLBACK has no substitute for `convertPathData`. The size
cost is flagged for the owner in `docs/blockers.md`; if it is judged
unacceptable, Phase 7 switches to FALLBACK and ADR-0006 records that instead —
the two paths differ only inside `optimize.rs`.

---

## 10. vtracer's own SVG writer is not reusable for Phase 6

`vtracer::svg::SvgWriter` is public (`src/svg.rs:25-32`) but writes a
`VectorDoc`, not our `ShapeDoc`, and its path encoder (`struct Emitter`,
`src/svg.rs:128`) plus number formatter (`fn fmt_num`, `src/svg.rs:333`) are
private. Phase 6 therefore implements its own encoder, as the plan's primary
option allows.

Its behaviour is still a useful reference, and matches the plan's output
contract: shortest of absolute/relative per segment, `H`/`V`/`S` shorthands,
trailing zeros trimmed, leading `0.` → `.`, and no separator before a negative
(`src/svg.rs:318-367`). Divergences we deliberately adopt: no XML prolog, no
generator comment, `viewBox` instead of `width`/`height`, and no `<g>` grouping
(plan §Phase 6, ADR-0005).

---

## 11. Crate name

`vectorise` is **available** on crates.io (`GET /api/v1/crates/vectorise` →
`"crate 'vectorise' does not exist"`, checked 2026-09-20). No rename needed:
package name `vectorise`, binary name `vectorise`. `vectorise-cli` is also free
if a split is ever wanted.

---

## 12. Version pins resolved

`cargo add` with the plan's constraints resolved to these, all latest stable at
the time of writing:

| Crate | Requested | Resolved |
|---|---|---|
| vtracer | `=1.0.0-alpha.4` | 1.0.0-alpha.4 |
| image | `0.25`, default features off, `png jpeg webp gif bmp tiff` | 0.25.10 |
| kurbo | `0.13` | 0.13.1 |
| tiny-skia | `0.11` | 0.11.4 |
| oxvg_optimiser | latest, default features off | 0.0.8 |
| oxvg_ast | latest, features `roxmltree serialize visitor selectors` | 0.0.8 |

Deferred to the phase that needs them: `clap` 4.6.7, `glob` 0.3.4, `rayon`
1.12.0, `thiserror` 2.0.20, `anyhow` 1.0.104, `tracing` 0.1.44,
`tracing-subscriber` 0.3.23, `tempfile` 3.27.0, `assert_cmd` 2.2.2,
`predicates` 3.1.4, `insta` 1.48.0, `proptest` 1.11.0, `resvg`/`usvg` 0.45.1.

One correction to plan §2: **`resvg` and `usvg` 0.45.1 are licensed
`Apache-2.0 OR MIT`, not MPL-2.0** (`resvg-0.45.1/Cargo.toml:31`,
`usvg-0.45.1/Cargo.toml:29`). resvg relicensed at 0.45. No copyleft
consideration remains for Phase 9.
