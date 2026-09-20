# 0006 — link oxvg, and run it losslessly

- **Status:** accepted
- **Date:** 2026-09-20
- **Phase:** 7

## Context

Two decisions, taken together because the second is what makes the first
defensible.

### Whether to link oxvg at all

Phase 0 established that `oxvg_optimiser` is usable as a library, that its jobs
can be enabled individually, and that its licence is clean
(`docs/api-notes.md` §9). It also established the cost: linking it takes the
release binary from about 1.5 MB to about 8.5 MB, because it brings
`lightningcss` and `parcel_selectors`, a complete CSS parser and selector
engine that our output never exercises. Measured again at the end of this
phase, with the whole pipeline linked: **8 725 552 bytes**.

Against that, `convertPathData` is the one thing the writer does not do. The
writer chooses the shortest spelling per segment; it never re-encodes path
data across segments.

The implementation plan names output compactness as the product goal (§1) and
sets no binary-size budget. The plan's own FALLBACK has no substitute for
`convertPathData`. So: link it.

### How aggressively to run it

`convertPathData` has three sub-passes that change geometry rather than
re-encode it: `arc_curves` rewrites runs of cubics as elliptical arcs,
`smart_arc_rounding` rounds those arcs' radii, and `straight_curves` replaces a
nearly straight cubic with a line. Each has its own tolerance, stacked on top of
the one the shape pass already spent.

Measured on a traced document (a disc and a five-pointed star, rendered at 2x
and compared pixel by pixel against the writer's own output):

| Configuration | Output | Differing pixels | Largest channel difference |
|---|---:|---:|---:|
| Re-encoding only | −4.4% | 0 of 240 000 | 0 |
| With arcs and straight curves | −13.4% | 38 of 240 000 (0.016%) | 58 of 255 |

The differing pixels are on the star's points, where a long cubic becomes an
arc. Nobody would see them. But `--shape-tolerance` is how a user says how much
fidelity they are willing to trade, and an optimizer that spends more on its own
makes that dial a lie.

## Decision

Link `oxvg_optimiser`, starting from SVGO's default job set, with four jobs
disabled (`convertShapeToPath`, `mergePaths`, `removeViewBox`, `sortAttrs`) and
`convertPathData` configured with `arc_curves`, `smart_arc_rounding`, and
`straight_curves` off.

`optimize` returns whichever of its input and its output is shorter, so the pass
can never make a document larger.

## Consequences

**Gained.** Output is about 4% smaller than the writer alone produces, and the
rendered result is bit-identical, which a test asserts on the real pipeline and
not only on a hand-written sample. `--shape-tolerance` remains the only thing
that trades fidelity for size.

**Lost.** About 9 percentage points of further saving, and about 7 MB of binary.
The binary cost buys one job; the rest of oxvg's dependency tree is dead weight
at runtime.

**Accepted.** `docs/blockers.md` B-001 records the size trade-off for the
repository owner, who may still decide it is not worth it. Reversing is
contained: delete `src/optimize.rs`'s oxvg path, drop two dependencies, and make
`--no-optimize` a documented no-op, which is the plan's FALLBACK exactly.

If the 9 points are wanted later, the right shape is a flag
(`--optimize-geometry`, off by default) rather than a change of default, so the
fidelity contract stays opt-out rather than opt-in. A cheaper alternative also
exists: implementing the re-encoding half of `convertPathData` in `writer.rs`
captures most of the measured 4.4% at no dependency cost.
