# Blockers

Blockers found during implementation, per `IMPLEMENTATION_PLAN.md` §0.9: what was
tried, two alternative paths, and which one the plan marks as fallback.

---

## B-001 — oxvg costs 7.3 MB of binary for 29% of output bytes

- **Found:** Phase 0, 2026-09-20.
- **Status:** open, does not block work before Phase 7.
- **Severity:** a product trade-off, not a technical failure. Both paths work.

### What was measured

`examples/spike.rs`, release profile from plan §Phase 1
(`lto = "fat"`, `codegen-units = 1`, `strip = true`, `panic = "abort"`),
`aarch64-apple-darwin`:

| Build | Binary |
|---|---|
| spike without the oxvg call graph | 1 544 176 B |
| spike with oxvg | 8 890 880 B |
| delta | **+7 346 704 B (+476%)** |

`oxvg_optimiser` 0.0.8 pulls `lightningcss`, `parcel_selectors`, `selectors`,
`cssparser`, `regex`, and `phf` — a complete CSS parser and selector engine.
Our output has no `<style>`, no classes, and no ids, so none of that machinery
ever does useful work at runtime.

Against it, on a vtracer document trimmed to what the Phase 6 writer will emit:

```
== oxvg on vtracer output: 549 -> 389 bytes (-29.1%)
```

The win is real and comes almost entirely from `convertPathData`, which our own
writer does not replicate.

### Alternative paths

1. **GO (plan's primary).** Link oxvg, disable the destructive jobs
   (`convert_shape_to_path`, `merge_paths`, `remove_view_box`, `sort_attrs`).
   Output is ~29% smaller; the binary is ~8.5 MB. Verified working and
   idempotent in the spike.
2. **FALLBACK (plan §Phase 7).** Drop oxvg. Rely on vtracer's `optimize` level
   plus our writer's encoder. `--no-optimize` becomes a documented no-op. Binary
   stays ~1.5 MB; output stays ~29% larger than it could be.

A third option exists but is out of scope for v1: implement the subset of
`convertPathData` we actually benefit from (collapse repeated commands, drop
useless segments, straighten near-straight curves) directly in `writer.rs`. That
captures most of the win at no dependency cost, and is the right move if this
ever needs revisiting.

### Chosen

**GO**, per plan §1 ("maximally compact, standard-compliant SVG") which names
output compactness as the product goal and nowhere sets a binary-size budget.
Recorded in `docs/api-notes.md` §9. ADR-0006 is written in Phase 7 either way,
so reversing costs one ADR and the deletion of `optimize.rs`'s oxvg path.

Raised with the repository owner on 2026-09-20.

### Update, end of Phase 7

Built and measured with the whole pipeline linked: **8 725 552 bytes**, against
about 1.1 MB for the decode path alone.

The output saving is smaller than the Phase 0 estimate, because
`convertPathData`'s approximating sub-passes are now off: it is **4.4%**, not
29%, and the rendered result is bit-identical. See
`docs/adr/0006-optimizer-is-lossless.md` for the measurement and the reasoning.

That sharpens the question rather than settling it: 7.6 MB of binary for 4.4% of
output bytes. The FALLBACK remains fully specified and is now a smaller change
than it was, since `optimize.rs` is the only file involved.


---

## B-002 — `paste` is unmaintained (RUSTSEC-2024-0436)

- **Found:** Phase 7, 2026-09-20.
- **Status:** accepted, ignored in both tools.

`oxvg_ast` depends on `paste`, whose author has archived it. The advisory is an
unmaintained-status notice, not a vulnerability: `paste` is a proc-macro that
expands at compile time and contributes no runtime code to the binary.

Ignored with the same reason in two places, because the tools do not share a
config: `deny.toml` for `cargo deny check`, and `.cargo/audit.toml` for
`cargo audit`. Keep them in step.

Revisit if oxvg drops the dependency, or if a real advisory is ever filed
against `paste`.
