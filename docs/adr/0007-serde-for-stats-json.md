# 0007 — `serde` and `serde_json` for `--stats-json`

- **Status:** accepted
- **Date:** 2026-09-20
- **Phase:** 9

> The implementation plan reserved this number for a progress-bar dependency
> ("no `indicatif` in v1; ADR-0007 if you disagree"). We did not disagree: a
> batch reports one closing line on stderr and nothing is drawn. The number is
> used here instead, for the other dependency Phase 9 adds.

## Context

`--stats-json` has to emit one JSON object per file, and the plan asks for the
schema to be stable enough that a snapshot test can pin it.

The objects are small and flat: two paths, six integers, a nested count of five
integers, and an optional float. Writing that by hand is perhaps forty lines,
and every one of them is a chance to emit invalid JSON: a path containing a
quote or a backslash, a `NaN` fidelity, a non-UTF-8 filename. Those are exactly
the cases a hand-rolled writer gets wrong and a test never covers, because the
test uses `logo.png`.

`serde` and `serde_json` cost two dependencies. Both are already in the
dependency graph transitively, both are the default choice in the ecosystem,
and `serde_json` escapes strings correctly, refuses non-finite floats, and
never emits a newline inside an object, which is what "one object per line"
requires.

Against that, `#[derive(Serialize)]` ties the JSON field names to the Rust field
names. Renaming a field silently changes the output format.

## Decision

Add `serde` (with `derive`) and `serde_json`, and derive `Serialize` on
`stats::Stats`, `stats::Primitives`, and `stats::Totals`.

Treat the JSON as an interface: `stats_json_lines_are_valid_and_schema_stable`
snapshots one line, so renaming a field fails a test rather than a user's
script.

## Consequences

**Gained.** Correct escaping for free, a schema that is visible in one snapshot
file, and no hand-written serializer to review. Adding a field is a one-line
change plus a snapshot review.

**Lost.** Two dependencies, and a coupling between Rust field names and the
public JSON schema. The snapshot makes the coupling loud rather than removing
it.

**Accepted.** The binary grows by what `serde_json` costs. Measured at the end
of the phase in `docs/size.md`; it is small next to the optimizer's CSS parser.
