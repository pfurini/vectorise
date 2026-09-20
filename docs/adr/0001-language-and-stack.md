# 0001 — Rust, VTracer 1.0.0-alpha.4, and a single static binary

- **Status:** accepted
- **Date:** 2026-09-20
- **Phase:** 1

## Context

The tool converts raster images into maximally compact SVG. Its distinguishing
feature is not tracing — VTracer already does that well — but what happens to
the traced geometry afterwards: detecting that a traced region *is* a circle, an
ellipse, or a rectangle, and emitting the native SVG element instead of a Bézier
path.

Two implementations were available.

A Python tool would call the same tracer: the PyPI `vtracer` package is a `pyo3`
binding over the identical Rust crate, so raw tracing quality is equal. But the
binding exposes only "image in, SVG string out". Primitive detection would have
to re-parse the serialized SVG, whose coordinates are already quantized to two
decimals and whose region structure is gone. The two other libraries the tool
needs are Rust as well: the SVG optimizer (`oxvg`) and the renderer used to
verify fidelity (`resvg`). In Python they become a Node subprocess and a C
dependency.

Rust exposes VTracer's intermediate `VectorDoc`: ordered shapes with
full-precision `f64` geometry, before serialization. Fitting a primitive to that
is exact.

Two VTracer lines exist. The stable 0.6.x line has no `VectorDoc`, no
`--simplify`, no `--max-colors`, and no watershed frontend. The 1.0.0-alpha.4
line has all four. Phase 0 verified every API we depend on against the vendored
alpha source and recorded the findings in `docs/api-notes.md`.

Licensing is a hard constraint: only permissive licenses (MIT, Apache-2.0,
BSD-2/3, MPL-2.0, Zlib, ISC, Unicode-3.0, CC0) may be linked. This is why
Potrace, the obvious alternative tracer, is not used: it is GPL.

## Decision

Build `vectorise` in Rust (edition 2024) as one self-contained static binary
with no runtime dependencies, on VTracer pinned to exactly `1.0.0-alpha.4`,
operating on the `VectorDoc` IR rather than on serialized SVG.

The rest of the stack is fixed in `IMPLEMENTATION_PLAN.md` §2: `image` for
decoding, `kurbo` for 2D geometry, `tiny-skia` for raster verification, `oxvg`
for final minification, `resvg`/`usvg` for the `--verify` oracle, `clap`,
`rayon`, `thiserror`/`anyhow`, `tracing`, `tempfile`. Every crate is pure Rust,
so the musl targets link statically without a C toolchain. `cargo deny check`
enforces the license allow-list in CI from this phase on.

Targets: macOS (arm64, x86_64) and Linux (x86_64, aarch64). Windows is out of
scope.

## Consequences

**Gained.** Primitive detection runs on exact geometry, which is the whole point
of the tool. One binary, no interpreter, no subprocess, no network — the
distribution story is `brew install` or `curl | sh`. The license audit is
mechanical. A cross-compiled musl build from macOS already links static-pie
(verified in Phase 1), so the release targets carry no surprise.

**Lost.** Pinning a pre-release means a later alpha can break the IR without a
semver signal. `Cargo.lock` is committed and the pin is exact (`=1.0.0-alpha.4`)
so no update happens silently; if a future alpha breaks us, the fallback is a
vendored fork via `[patch.crates-io]` (plan §8). Contributors must write Rust
rather than Python, which narrows the contributor pool for a tool of this size.

**Accepted.** The MSRV is pinned to the toolchain installed at this phase,
1.98.1, in both `rust-toolchain.toml` and `Cargo.toml`. Raising one without the
other is a defect.
