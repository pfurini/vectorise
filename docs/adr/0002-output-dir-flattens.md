# 0002 — `--output-dir` flattens onto the file stem

- **Status:** accepted
- **Date:** 2026-09-20
- **Phase:** 2

## Context

`--output-dir DIR` redirects every output into one directory. Given
`a/x.png` and `b/x.png`, two behaviours are possible:

1. **Flatten.** Both map to `DIR/x.svg`. The two inputs now collide, and
   preflight rejects the batch.
2. **Mirror.** They map to `DIR/a/x.svg` and `DIR/b/x.svg`, recreating the input
   tree under `DIR`.

Mirroring sounds more helpful, but it has no well-defined answer for the common
cases. Inputs are arbitrary paths, not necessarily under one root: `../logo.png`
and `/tmp/logo.png` have no shared prefix to mirror from, and absolute paths
would either be pasted under `DIR` (producing `DIR/tmp/logo.svg`, silently
rewriting the user's absolute path as relative) or rejected. Mirroring also
means the tool creates directories the user never named, at depths it infers.
Shells make the ambiguity worse: `vectorise **/*.png -o out` and
`cd a && vectorise *.png -o ../out` would produce different layouts from the
same files.

Flattening has one visible cost: name collisions become common, because
`a/x.png` and `b/x.png` are an ordinary thing to have.

## Decision

`--output-dir DIR` maps each input to `DIR/<file_stem>.svg`, dropping the
input's directory entirely. Inner dots in the stem are preserved, so
`a/report.v2.png` becomes `DIR/report.v2.svg`.

Collisions are not resolved and are never silently overwritten: two inputs
mapping to one output is a `DuplicateOutput` problem, preflight rejects the
whole batch before writing anything, and `--force` does not override it
(`--force` permits overwriting a *pre-existing* file, which is a different
question).

## Consequences

**Gained.** The mapping is one line of code and one sentence of documentation,
and it is identical for relative paths, absolute paths, and `..`. A user can
always predict the output name from the input name alone. Collisions surface as
a complete, machine-readable list of exactly which inputs clash, instead of as a
half-written output directory.

**Lost.** Converting a nested tree in one command requires either accepting the
flat result or running the tool per directory. Users with many same-named files
across directories will hit `DuplicateOutput` and have to rename, batch, or drop
`--output-dir`.

**Accepted.** If mirroring is wanted later it arrives as a separate, explicit
flag (for example `--output-tree ROOT`, which names the root to mirror from and
so has no ambiguity). Changing `--output-dir` itself would silently relocate
users' output and is ruled out.
