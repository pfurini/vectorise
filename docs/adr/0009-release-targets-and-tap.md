# 0009 — five artifacts, one tap, no signing

- **Status:** accepted
- **Date:** 2026-09-20
- **Phase:** 10

## Context

Three decisions the release needs, none of which is reversible once people have
downloaded something.

### Which targets

The plan names four: `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, plus a universal
macOS binary. `cargo-dist` 0.32 builds the four but has no `universal2` target,
so the fifth has to be assembled from the first two.

musl rather than glibc for Linux, because a glibc binary carries the glibc it
was built against as a floor, and "works on the distribution I built on" is not
a release. Every dependency is pure Rust, so nothing is lost: CI has built the
musl target since Phase 1, and a macOS host cross-compiles it with `rust-lld`.

### Where the Homebrew formula goes

A formula has to live in a tap repository. `homebrew-core` requires notability
that a new tool does not have. So: a personal tap, `pfurini/homebrew-tap`,
giving `brew install pfurini/tap/vectorise`.

### Whether to sign the macOS binaries

Signing needs an Apple Developer Program membership and puts a private signing
key in a GitHub secret, where anyone who can run a workflow can sign anything as
the key's owner. Against that, the cost of not signing is narrow: Gatekeeper
only quarantines binaries downloaded through a browser, and neither
`brew install` nor the installer script does that. The people affected are those
who click a link on the releases page, and one `xattr -d` fixes it for them.

## Decision

Ship five artifacts: the four targets plus
`vectorise-universal-apple-darwin.tar.xz`, assembled with `lipo` in a
`post-announce-jobs` workflow of our own.

Publish a shell installer and a Homebrew formula to `pfurini/homebrew-tap`.

Do not sign. Document the quarantine workaround in the README and the full
signing procedure in `SIGNING.md`, for whoever owns an Apple Developer account.

Before a release is considered done, every artifact is downloaded on a runner of
its own architecture and executed: `--version`, `--help`, and a no-argument run
that must exit 64. The musl ones must additionally report no dynamic
dependencies. An artifact nobody has run is a guess.

## Consequences

**Gained.** One download per platform, with a universal option for anyone who
does not want to know which Mac they have. No private key in CI. A release that
cannot complete while an artifact is broken, because the smoke job runs before
the workflow finishes.

**Lost.** A browser download on macOS shows a scary dialog until the user runs
one command. The universal binary is built after the release exists rather than
alongside the others, so it appears a minute later than the rest.

**Accepted.** `cargo-dist` regenerates `.github/workflows/release.yml`, so that
file is never edited by hand: changes go in `dist-workspace.toml` followed by
`just dist-generate`. The two custom jobs live in their own files precisely so
they survive regeneration.

If signing is wanted later, `cargo-dist` supports it with `macos-sign = true`
and the secrets `SIGNING.md` lists. That is a decision for the account owner.
