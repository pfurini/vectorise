# 0011 — `--verify` measures against the image the tracer saw

- **Status:** accepted
- **Date:** 2026-09-20
- **Phase:** cleanup 4

## Context

ADR-0008 defines `fidelity` as `1 - mean absolute error over RGB` between
the rendered SVG and the decoded input. Until the cleanup stage, the decoded
input was also the image the tracer traced, so the number answered one
question: how well did the tracer reproduce what it was given?

With cleanup active, the tracer traces an image that is not the file on
disk. Comparing the render against the raw decoded input then conflates two
different quantities: how well the tracer did its job, and how much cleanup
changed the raster. Measured on a blurred logo (`CLEANUP_IMPLEMENTATION_PLAN.md`
§9.4), cleanup made the output 14 times smaller and much closer to the
original drawing, while fidelity measured against the blurry input *fell*
from 0.9920 to 0.9771. Faithfully reproducing a blurry raster requires a
busy document; that is the whole problem cleanup exists to fix. Reported the
old way, the feature looks like a regression in the tool's own numbers.

Two alternatives were considered and rejected:

- Keep comparing against the raw input. `--verify` then actively misreports
  the feature's effect.
- Report only against the raw input and document the effect. The same
  misreport, with a footnote.

## Decision

`fidelity` is measured against the image the tracer saw, and a second
number, `cleanup_delta`, reports what cleanup did.

| Field | Compares | Answers |
|---|---|---|
| `fidelity` | rendered SVG against the cleaned image | did the tracer reproduce what it was given? |
| `cleanup_delta` | cleaned image against the raw decoded image | how much did cleanup change the raster? |

`cleanup_delta` is the same quantity ADR-0008 defines, mean absolute error
over RGB in `0..=1`, applied to two rasters instead of a raster and a render.
It is an error, not a score, and is reported as is, never as `1 - error`. It
appears in `--stats-json` only when cleanup changed something; absent means
zero.

With `--cleanup off`, or whenever cleanup changes nothing, the cleaned image
*is* the raw image, so `fidelity` keeps its v0.1.0 meaning exactly and
`cleanup_delta` is absent. ADR-0008 stays valid and carries an amendment
pointing here.

Confirmed by the repository owner on 2026-09-20.

## Consequences

**Gained.** `--verify` keeps answering the question it always answered, and
a second number answers the new one. A script can tell the two situations
apart: a `fidelity` next to a `cleanup_delta` is against the cleaned raster,
and one without is against the file.

**Lost.** A user who reads only `fidelity` no longer sees a measure against
the file on disk when cleanup acted. The README's tuning section says so
next to the first example where it matters.

**Accepted.** The two numbers are the same metric on different pairs of
images, so they add: a rough upper bound on the error against the raw input
is `(1 - fidelity) + cleanup_delta`. That bound is documented rather than
reported, because the exact figure would need a second render.
