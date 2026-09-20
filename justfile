# Developer entry points. `just check` is the gate every phase must pass.

default: check test

# Everything CI checks except the tests: formatting, lints, licenses, rustdoc.
check: fmt-check clippy deny doc

# Rewrite source to the configured style.
fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

clippy:
    cargo clippy --all-targets --all-features -- -D warnings

deny:
    cargo deny check

doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features

test:
    cargo nextest run --all-features

# Doctests, which nextest does not run.
test-doc:
    cargo test --doc --all-features

cov:
    cargo llvm-cov nextest --all-features --summary-only

cov-html:
    cargo llvm-cov nextest --all-features --html --open

# Regenerate the committed test fixtures (Phase 3 onwards).
fixtures:
    cargo run --quiet --bin gen-fixtures

audit:
    cargo audit

# Regenerate THIRD-PARTY.md after the dependency graph changes.
third-party:
    python3 scripts/gen-third-party.py

# Build the release binary the way the release workflow does.
release-dry:
    cargo build --release
    cargo publish --dry-run --allow-dirty
    dist plan

# Regenerate .github/workflows/release.yml after editing dist-workspace.toml.
dist-generate:
    dist generate
