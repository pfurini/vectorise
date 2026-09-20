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
> against `IMPLEMENTATION_PLAN.md`. Phase 1 (skeleton, tooling, CI) is done;
> conversion itself is not wired up yet. The usage section below is filled in
> from the real `--help` at Phase 8.

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

```
vectorise [OPTIONS] <INPUT>...
```

Output for `foo/bar.png` is `foo/bar.svg` — same directory, same stem, unless
`--output-dir` says otherwise. The whole batch is validated before anything is
written: if any output already exists, or two inputs would produce the same
output, nothing is written at all.

The full option list arrives with Phase 8.

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
