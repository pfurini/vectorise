# Binary size

Release profile from `Cargo.toml`: `lto = "fat"`, `codegen-units = 1`,
`strip = true`, `panic = "abort"`. Numbers are bytes, measured on
`aarch64-apple-darwin` with rustc 1.98.1 unless stated otherwise.

Re-measure at the end of every phase that adds a dependency, and explain any
jump of more than 500 KB.

## End of Phase 8

| Build | Size | Notes |
|---|---|---|
| `target/release/vectorise` | 9 438 752 | The shipping binary, now that `main` calls the whole pipeline. |
| pipeline without the CLI (Phase 7 probe) | 8 725 552 | A throwaway example calling decode, trace, fit, write, optimize. |

The jump from 1.1 MB (decode only) to 8.7 MB is `oxvg_optimiser` and its CSS
parser, exactly as Phase 0 predicted. It buys `convertPathData`, worth about 4%
of output bytes at bit-identical fidelity; see
`docs/adr/0006-optimizer-is-lossless.md` and `docs/blockers.md` B-001.

The further 0.7 MB from the probe to the shipping binary is `clap` with
`wrap_help`, `tracing-subscriber`, and `rayon`.

## Baseline — end of Phase 3

| Build | Size | Notes |
|---|---|---|
| `target/release/vectorise` | 584 464 | The CLI as it stands. `main` does not call `decode` yet, so fat LTO drops nearly all of `image` and `vtracer`. |
| decode path linked | 1 146 848 | A throwaway example calling `decode::decode`, measured the same way. This is the honest floor for Phase 8, when the CLI links the pipeline. |
| Phase 0 spike, decode + trace + kurbo + tiny-skia | 1 544 176 | From `docs/api-notes.md` §9.4. Closest estimate of the finished binary before the optimizer. |
| Phase 0 spike, same plus oxvg | 8 890 880 | The +7.3 MB recorded in `docs/blockers.md` B-001. |

## What is linked today

- `image` 0.25.10 with `default-features = false` and exactly six formats
  (`png`, `jpeg`, `webp`, `gif`, `bmp`, `tiff`). Leaving the default features on
  would add AVIF, DDS, EXR, farbfeld, HDR, ICO, PNM, QOI, and TGA.
- `vtracer` 1.0.0-alpha.4 and `visioncortex` 0.9.3.
- `clap` 4.6.7 with `derive` and `wrap_help`.
- `glob`, `tempfile`, `thiserror`.

## Expected additions

| Phase | Crate | Expected effect |
|---|---|---|
| 5 | `kurbo`, `tiny-skia` | small; both are compact pure-Rust libraries (the Phase 0 spike linked them for ~400 KB on top of decode) |
| 7 | `oxvg_optimiser`, `oxvg_ast` | measured +7.6 MB, dominated by `lightningcss` and `parcel_selectors` (see `docs/blockers.md` B-001) |
| 8 | `rayon`, `tracing`, `tracing-subscriber`, `anyhow` | moderate; `tracing-subscriber` is the larger half |
| 9 | `resvg`, `usvg`, `serde_json` | moderate; `usvg` brings a full SVG parser |

## How to reproduce

```sh
cargo build --release
ls -l target/release/vectorise
```

To measure a path the CLI does not yet call, write a throwaway
`examples/<name>.rs` that calls it, build with
`cargo build --release --example <name>`, measure, and delete the file.
