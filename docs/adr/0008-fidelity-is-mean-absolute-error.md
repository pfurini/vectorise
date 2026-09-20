# 0008 — `--verify` reports mean absolute error, not SSIM

- **Status:** accepted
- **Date:** 2026-09-20
- **Phase:** 9

## Context

`--verify` renders the SVG we produced and reports how close it came to the
input. Two families of metric are usual.

**Mean absolute error** over RGB is the average per-channel difference. It is
one line of arithmetic, it has no parameters, and it is exactly reproducible:
two people who compute it on the same pair of images get the same number. It is
also a poor model of perception. A one-pixel shift of a sharp edge scores badly;
a subtle global colour cast scores well. Neither matches what a person notices.

**SSIM** models perception better by comparing local means, variances, and
covariance over a sliding window. It is the right metric for "does this look the
same". It also has parameters: the window size and shape, the Gaussian sigma,
and the two stabilizing constants `C1` and `C2`. Different implementations
choose differently, and their numbers are not comparable. Hand-rolling it, as
the plan explicitly forbids, would produce a number nobody could check against
anything.

A permissively licensed SSIM crate would solve the second problem but not the
first: the parameters would still have to be chosen and documented, and the
score would still need a threshold nobody has calibrated for traced vector art.

The number also has to mean something to a user in one glance. `fidelity
0.9876` is legible as "almost all of it survived". An SSIM of 0.9876 means
something different and less obvious.

## Decision

`--verify` reports `fidelity = 1 - mean absolute error over RGB`, a number in
0..=1, computed after compositing both images over the same opaque background
so transparency cannot flatter or punish either side.

The metric is exposed as `verify::compare` on two byte slices, so its
properties can be tested without rendering anything.

## Consequences

**Gained.** A number with no parameters, reproducible anywhere, and cheap: one
pass over the pixels. The tests can state exact expectations (identical scores
1, inverted scores 0, mid-grey against white scores 128/255) rather than
approximate ones.

**Lost.** The number does not track perception. A traced logo whose edges moved
half a pixel scores lower than a photograph whose colours drifted, even though
the second looks worse. Users should read fidelity as "how much of the picture
survived", not "how good does this look".

**Accepted.** Adding SSIM later is additive: a second field on `Fidelity`, a
second column in `--stats`, and a `--metric` flag if both are wanted. It should
arrive with a permissively licensed implementation and a documented parameter
set, not with a hand-rolled one.
