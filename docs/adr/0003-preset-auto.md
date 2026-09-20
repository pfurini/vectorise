# 0003 — `--preset auto` is `poster` in v1

- **Status:** accepted
- **Date:** 2026-09-20
- **Phase:** 4

## Context

`--preset` defaults to `auto`, which promises that the tool picks a sensible
starting point for the image in front of it. VTracer itself has no such preset:
`vtracer::Preset` is `Bw`, `Poster`, or `Photo` (`config.rs:59-67` of
1.0.0-alpha.4). `Auto` is ours to define.

The three presets differ in how much detail they preserve:

| Preset | Clustering | Color precision | Speckle | Corner threshold |
|---|---|---|---|---|
| `bw` | binary | 6 | 4 | 60 |
| `poster` | color cluster | 8 | 4 | 60 |
| `photo` | color cluster | 8 | 10 | 180 |

A real heuristic would have to classify the input before tracing it. The usable
signals are cheap to compute but not cheap to calibrate:

- **Unique color count.** A logo has tens; a photograph has tens of thousands.
  The threshold between them is not obvious, and anti-aliased line art sits in
  the middle.
- **Color histogram concentration.** What fraction of pixels fall in the top
  16 colors. Flat art concentrates; photographs spread.
- **Edge density and gradient magnitude.** Flat art has few, sharp edges.
- **Whether the image is effectively two-tone**, which would select `bw`.

Each threshold needs a corpus to calibrate against, and a wrong guess is worse
than no guess: the user gets output tuned for the wrong kind of image and has no
indication why.

## Decision

`Preset::Auto` resolves to `Preset::Poster`, unconditionally, in v1.
`Preset::resolve` is the single place this happens, and it is public so the
behaviour is visible rather than buried in a mapping function.

`poster` is the right constant choice: it is the widest-palette flat-art preset,
it is what the tool's stated purpose (logos, icons, illustrations) calls for,
and it degrades gracefully on photographs, where the user is expected to reach
for `--preset photo` anyway.

## Consequences

**Gained.** No hidden classifier to explain when output surprises someone, and
no corpus to calibrate before shipping. Users who know their input override the
preset in one flag. `Preset::resolve` gives a later heuristic exactly one place
to live.

**Lost.** `auto` does not earn its name yet. A user who passes a photograph and
never reads the help gets `poster`, with more layers and less speckle filtering
than that image wants.

**Accepted.** When a heuristic does arrive, it changes the output of every run
that did not name a preset. That is a behaviour change, so it belongs in a minor
release with a changelog entry, and `resolve` will need the image (or a summary
of it) as an argument rather than being a method on the preset alone.
