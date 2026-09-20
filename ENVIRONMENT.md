# Environment

Recorded during repository bootstrap.

- **Date:** 2026-09-20
- **OS:** macOS (Darwin), host `aarch64-apple-darwin`
- **Arch:** arm64

## Rust toolchain

- **rustc:** 1.98.1 (48a229cea 2026-09-01)
- **cargo:** 1.98.1 (797e8a9bc 2026-08-05)
- **Installed via:** rustup (already present on host)
- **Components:** rustfmt, clippy, llvm-tools-preview
- **Targets added:** aarch64-apple-darwin (default), x86_64-apple-darwin

## Cargo tools

| Tool | Version |
|---|---|
| cargo-binstall | 1.23.0 |
| cargo-audit | 0.22.2 |
| cargo-deny | 0.20.2 |
| cargo-nextest | 0.9.145 |
| cargo-insta | 1.48.0 |
| cargo-llvm-cov | 0.9.1 |
| cargo-dist | 0.32.0 (binary installed as `dist`; symlinked to `cargo-dist` in `~/.cargo/bin` so `cargo dist` also resolves) |
| just | 1.58.0 (installed via Homebrew) |

## Vendored vtracer source

Pre-warmed in a throwaway `cargo init` project (`/tmp/vectorise-warm`, deleted after use) by adding the exact dependency set from the implementation plan plus `oxvg_optimiser` (latest, resolved to v0.0.8), then running `cargo build`. All 234+141 packages resolved and compiled without error.

`vtracer` 1.0.0-alpha.4 vendored source path:

```
/Users/paolof/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/vtracer-1.0.0-alpha.4
```

Confirmed present under `src/`: `ir.rs`, `config.rs`, `optimize.rs`, `pipeline.rs`.
