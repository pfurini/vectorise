//! Convert raster images into maximally compact, standard-compliant SVG.
//!
//! `vectorise` traces a raster image with [VTracer], then replaces every traced
//! region that is geometrically indistinguishable from a circle, ellipse, or
//! rectangle with the corresponding native SVG element, and finally minifies
//! the document. The result is fewer paths, fewer bytes, and markup a human can
//! still edit.
//!
//! [VTracer]: https://crates.io/crates/vtracer
//!
//! # Status
//!
//! Phase 1 of `IMPLEMENTATION_PLAN.md`: the skeleton is in place and the public
//! API described in §3.2 is still being built up phase by phase. Today the
//! crate exposes the command-line definitions and the exit-code contract.
//!
//! # Examples
//!
//! ```
//! use vectorise::error::ExitCode;
//!
//! // The shell can tell a rejected batch from a partial failure.
//! assert_ne!(u8::from(ExitCode::Preflight), u8::from(ExitCode::Partial));
//! ```

pub mod cli;
pub mod error;
