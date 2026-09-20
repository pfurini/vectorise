# 0010 — Raster cleanup: Kuwahara plus toggle contrast, hand-written, on by default

- **Status:** accepted
- **Date:** 2026-09-20
- **Phase:** cleanup 1 to 3

## Context

VTracer decides region boundaries by colour clustering. When an input has
soft edges, the transition band between two flat colours spans several
pixels, and those intermediate pixels cluster into regions of their own: a
halo of thin slivers along every edge, and a boundary that wanders pixel to
pixel and so fits into many curve segments. No existing option fixes this.
`--simplify`, `--filter-speckle`, `--color-precision`, and `--max-colors` all
act after the damage, on geometry or on the palette. Measured: a 256x256
four-colour logo traces to 875 bytes clean and to 54 950 bytes after a
Gaussian blur of sigma 3, from the same drawing.

The precedent is Potrace, whose `mkbitmap` is a separate raster pass
(high-pass, blur, scale, threshold) run before tracing. We do the same thing
in process, with the strength chosen automatically.

Three questions had to be settled: which operators, whether to depend on a
crate for them, and whether the pass may be on by default.

### Which operators

The obvious operator is the unsharp mask, and it is the wrong one. It is a
high-pass boost, so it amplifies noise and adds overshoot and undershoot
halos on both sides of every edge, and a colour clusterer reads those halos
as new colour bands. On the plan's 24-case harness it was the worst
candidate tested: on clean input it inflated a 173-byte disc to 1 025 bytes
at sigma 1.0, and on JPEG input it made output larger than doing nothing in
every case (`CLEANUP_IMPLEMENTATION_PLAN.md` §9.2).

The pair that won:

| Operator | What it does | Why it cannot halo |
|---|---|---|
| Kuwahara filter | replaces each pixel with the mean of the lowest-variance quadrant around it | a quadrant never straddles an edge, because the one on one side always has lower variance |
| morphological toggle contrast | replaces each pixel with the darkest or brightest neighbour, whichever is closer | it copies a whole pixel that already exists; no value is synthesised |

Each is driven by its own signal, because the two failure modes are
orthogonal. JPEG compression leaves edges sharp and raises noise in flat
regions; blur does the reverse. `edge_width` (median 10% to 90% rise across
detected edges) drives the toggle radius; `flat_noise` (deviation from the
3x3 median over flat windows only) drives the Kuwahara radius. §9.1 of the
plan has the separation table, and `src/cleanup/estimate.rs` reproduces it
to three decimals.

On the plan's 24-case matrix the rule takes 447 582 bytes of output to
43 762, a 10.2x reduction, with no cell larger and fidelity to the pristine
drawing better in every degraded cell. `tests/cleanup.rs` keeps that matrix
under test and snapshots its own byte counts.

Those counts differ from the plan's, and the reason was found: the plan's
spike traced with `TraceOptions::default()`, VTracer's own configuration at
six bits per colour channel, while the command line traces with the `auto`
preset, which is `poster` at eight bits. Traced the spike's way, the port
reproduces §9.3 to the byte in 23 of 24 cells. Traced the command line's
way, the same rule takes 550 749 bytes to 81 627, a 6.7x reduction, with
both properties still holding on every cell. The heavily blurred cells keep
more of their gradient bands at eight bits; how the tracer's colour
precision should interact with cleanup is a question for a later change,
not for this one.

### Whether to depend on a crate

Every crate surveyed either lacks the two winning operators or costs more
than it gives:

| Crate | Version | License | Verdict |
|---|---|---|---|
| `imageproc` | 0.27.0 | MIT | has bilateral, box, median, sharpen; no Kuwahara, no toggle contrast. Pulls `nalgebra`, `rand`, `rand_distr`, `itertools`, `num`, `approx`, `getrandom` unconditionally. Its bilateral filter also drops alpha and measured lower fidelity than a 40-line hand-written one |
| `zenfilters` | 0.1.0 | AGPL-3.0 | disqualified by `IMPLEMENTATION_PLAN.md` §0.5 |
| `oxideav-image-filter` | 0.1.2 | MIT | created 2026-04-24, 1 180 downloads. Too new to depend on |
| `libblur` | 0.24.0 | Apache-2.0 OR BSD-3 | blur-focused, large SIMD surface, neither operator |
| `scirs2-ndimage` | 0.6.5 | Apache-2.0 | has both, inside a scientific-computing stack |
| `fast_morphology` | 0.3.2 | BSD-3 OR Apache-2.0 | dilate and erode only; toggle contrast is fifteen lines on top |

`docs/blockers.md` B-001 is already open about binary size. Both operators
are hand-written against `image` and `rayon`, which the crate already links.
The fallback the plan names, should a reviewer reject hand-writing, is
`fast_morphology` for the dilate and erode under toggle contrast; it was not
needed.

### Whether the pass may be on by default

Only if a clean input comes back byte for byte. That is the no-op
guarantee: a clean image measures as clean, both radii come out zero, and
`cleanup()` returns before touching a filter, in one branch at the top of
the function. The four clean fixtures assert it pixel for pixel, and the
matrix test asserts the resulting SVG is byte-identical.

## Decision

The cleanup pass is Kuwahara (radius from `flat_noise`) followed by
morphological toggle contrast (radius from `edge_width`), both hand-written
in `src/cleanup/filter.rs` with no new dependency, on by default
(`--cleanup auto`), with `--cleanup off` restoring the v0.1.0 pipeline
exactly and `--denoise` and `--sharpen` overriding either radius.

`--preset photo` changes the default of `--cleanup` to `off`. Both
operators assume the source was piecewise constant, and a photograph is
not. The implication sets a default, not a lock: an explicit
`--cleanup auto`, `--denoise`, or `--sharpen` on the same command line wins,
the same way an explicit tracing flag overrides a preset's value. Note that
`--preset auto` resolves to `poster` (ADR-0003), so the default preset leaves
cleanup on; only an explicit `--preset photo` turns it off. Decided by the
repository owner on 2026-09-20.

## Consequences

**Gained.** A blurred or JPEG-damaged logo converts to a document an order
of magnitude smaller and closer to the drawing, with no flag. Both radii and
both signals are visible in `--stats`, so the pass explains itself. No
dependency, and a binary size change near zero.

**Lost.** Every conversion pays for the two measurements, about 8 ms at
1024x1024, even when the answer is "do nothing". Users who know their inputs
are clean can pass `--cleanup off`. `--verify` had to change meaning to stay
honest, which is ADR-0011.

**Accepted.**

- The optimised filters are held bit for bit to the naive reference
  implementations, which stay in the test module as oracles. Variance and
  luminance comparisons use exact integers for that reason; a float form
  cannot make the promise.
- The automatic rule's constants are calibrated against the exact arithmetic
  of `src/cleanup/estimate.rs`. Changing a luminance coefficient, a
  threshold, or the edge walk invalidates them; the plan's §11.3 lists what
  breaks the calibration and the harness that must be re-run.
- Decoding keeps fully transparent pixels at alpha zero (`docs/api-notes.md`
  §6), so the pass does not always see opaque pixels. Kuwahara averages alpha
  like any other channel, so a transparent region bordering an opaque region
  of the same luminance can acquire intermediate alpha, which the tracer
  then treats as opaque. That only happens on a degraded input with a hard
  alpha mask, which is rare; `--cleanup off` is the escape hatch, and the
  plan's risk register covers adding any misfiring input to the fixture set.
- The pass is not idempotent, and not a strict contraction either. Kuwahara
  smooths residual noise a little further on a second run, toggle contrast
  sharpens a very wide blur in steps, and on a smooth gradient with too few
  edges to measure a second Kuwahara pass can move slightly more than the
  first before a third settles. The plan asked for an idempotency property
  test; the property is false, and no unbounded replacement is claimed.
  `a_second_pass_changes_less_than_the_first_on_every_degraded_fixture`
  checks the settling behaviour on the 24-case matrix instead. The pass
  never has to run twice.
