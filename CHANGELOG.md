# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Project skeleton: library plus binary, pinned toolchain, lint configuration,
  `cargo-deny` license gate, `justfile`, and CI on macOS and Linux.
- `vectorise --version` and the exit-code contract (0, 1, 2, 64, 74).
- `docs/api-notes.md`: Phase 0 verification of every pinned pre-release API
  against its vendored source.
- Input resolution, output mapping, and batch preflight: `vectorise::plan`
  turns arguments into a validated `Plan` or the complete list of reasons the
  batch is rejected. Supports literal paths, self-expanded globs when the shell
  did not expand them, `--output-dir`, `--force`, and `--dry-run`.
- Decoding: `vectorise::decode` reads PNG, JPEG, WebP, GIF, BMP, and TIFF into
  the tracer's image type, deciding the format by content rather than by
  extension, applying EXIF orientation (`image` 0.25 does not), and resolving
  transparency against `--background` (default white).
- `vectorise::color::Rgb` with `#rrggbb` and `#rgb` parsing.
- `docs/adr/0001-language-and-stack.md`, `docs/adr/0002-output-dir-flattens.md`,
  `docs/size.md`.

[Unreleased]: https://github.com/pfurini/vectorise/compare/HEAD
