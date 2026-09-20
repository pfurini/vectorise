# vectorise

[![CI](https://github.com/pfurini/vectorise/actions/workflows/ci.yml/badge.svg)](https://github.com/pfurini/vectorise/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/vectorise.svg)](https://crates.io/crates/vectorise)
[![License](https://img.shields.io/crates/l/vectorise.svg)](#license)

Convert raster images into maximally compact, standard-compliant SVG.

`vectorise` traces PNG, JPEG, WebP, GIF, BMP, and TIFF input with
[VTracer](https://crates.io/crates/vtracer), then does what a tracer alone
cannot: every traced region that is geometrically indistinguishable from a
circle, ellipse, or rectangle is emitted as that **native SVG element** instead
of a Bézier path. The document is minified last. The output is smaller, and it
is still markup a human can read and edit.

One self-contained binary. No runtime dependencies, no subprocesses, no network.

> **Status: under construction.** The repository is being built phase by phase
> against `IMPLEMENTATION_PLAN.md`. Everything below works; the release
> artifacts are what remain.

## Install

Not yet published. Once the first release is tagged:

```sh
# Homebrew
brew install pfurini/tap/vectorise

# Installer script
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/pfurini/vectorise/releases/latest/download/vectorise-installer.sh | sh

# From source
cargo install vectorise
```

Supported targets: macOS (arm64, x86_64) and Linux (x86_64, aarch64). Windows is
out of scope by design.

## Usage

```sh
# One file. Writes logo.svg next to logo.png.
vectorise logo.png

# A batch, into one directory, four at a time.
vectorise icons/*.png --output-dir build/icons --jobs 4

# See what would happen, without writing anything.
vectorise --dry-run 'assets/**/*.png'
```

Output for `foo/bar.png` is `foo/bar.svg` — same directory, same stem, unless
`--output-dir` says otherwise. The whole batch is validated before anything is
written: if any output already exists, or two inputs would produce the same
output, nothing is written at all and every problem is listed at once.

### Options

```
Output:
  -o, --output-dir <DIR>   Write all outputs into DIR (flattened)
  -f, --force              Overwrite existing outputs; duplicates still fail
  -n, --dry-run            Print the input to output plan and exit
      --keep-size          Emit width and height as well as viewBox

Tracing:
      --background <COLOR>          Color transparency resolves against [default: #ffffff]
      --preset <bw|poster|photo|auto>                             [default: auto]
      --clustering <color-cluster|bw|watershed>
      --hierarchical <stacked|cutout>
  -m, --mode <pixel|polygon|spline>
      --filter-speckle <PX>         Discard speckles below this side length
      --color-precision <BITS>      Significant bits per RGB channel
      --gradient-step <N>           Color difference between gradient layers
      --max-colors <N>              Quantize to at most this many colors
      --palette <COLORS>            Fixed palette, comma-separated hex
      --palette-file <FILE>         Fixed palette, one color per line
      --simplify <PX>               Curve simplification tolerance; try 1 to 2.5
      --threshold <N>               Black-and-white cutoff for --clustering bw
      --adaptive                    Adaptive thresholding instead of a cutoff
      --watershed-detail <N>        Where to cut the watershed hierarchy
      --corner-threshold <DEG>      Higher smooths through sharper turns
      --segment-length <PX>         Lower fits large smooth curves more closely

Shapes:
      --shapes <auto|off>           Detect native SVG shapes  [default: auto]
      --shape-tolerance <FRACTION>  Allowed difference, as a fraction of area
                                                              [default: 0.02]
      --min-shape-area <PX2>        Smaller regions stay paths [default: 16]
      --no-rotated-ellipses         Keep rotated ellipses as paths

Output quality:
      --no-optimize                 Skip the optimizer pass
      --verify                      Render the result and report fidelity
      --stats                       One line per file plus a total, on stderr
      --stats-json                  One JSON object per file, on stdout
  -p, --precision <N>               Coordinate decimals        [default: 2]

Runtime:
  -j, --jobs <N>                    Files at once  [default: available cores]
  -q, --quiet                       Report nothing but errors
  -v, --verbose...                  -v info, -vv debug, -vvv trace
```

`vectorise --help` prints the full text, with a sentence on each option.

### Reporting

```sh
$ vectorise --stats --verify logo.png
logo.png -> logo.svg  280 -> 164 bytes (59%)  2 colors  2 shapes (1 circle, 0 ellipse, 1 rect, 0 rounded, 0 path, 0 cmds)  1 ms fidelity 0.9949
total: 1 file(s)  280 -> 164 bytes (59%)  2 shapes, 0 paths  7 ms
```

`--verify` renders the SVG back at the input's pixel size and reports
`1 - mean absolute error`, so 1.0 means every pixel matched. `--stats-json`
prints the same numbers as one JSON object per line on stdout, leaving stderr
for messages:

```sh
vectorise --stats-json icons/*.png | jq -s 'map(.output_bytes) | add'
```

### Tuning

| Symptom | Try |
|---|---|
| A large circle or oval came out as a `<path>` | `--segment-length 1`. The tracer's default fit of a long curve is several percent too fat for any ellipse to match; see `docs/shape-detection.md`. |
| Too many tiny shapes | Raise `--filter-speckle`, or `--max-colors` to merge similar colors. |
| Output is still large | `--simplify 1.5` for fewer curve segments, `--precision 1` for shorter numbers. |
| Shapes were detected that should not have been | Lower `--shape-tolerance`, or `--shapes off`. |
| A photograph looks blotchy | `--preset photo`. |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | all inputs converted |
| 1 | one or more inputs failed; the others were written |
| 2 | preflight rejected the batch; nothing was written |
| 64 | usage error |
| 74 | I/O error outside per-file conversion |

## Development

Requires the toolchain pinned in `rust-toolchain.toml`, plus
[`just`](https://github.com/casey/just), `cargo-nextest`, `cargo-deny`, and
`cargo-llvm-cov`.

```sh
just check   # fmt, clippy, cargo-deny, rustdoc
just test    # nextest
just cov     # coverage summary
```

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual-licensed as above, without any additional terms or conditions.
