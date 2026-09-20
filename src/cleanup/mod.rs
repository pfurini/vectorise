//! Raster cleanup before tracing.
//!
//! VTracer decides region boundaries by colour clustering. When an input has
//! soft edges, the transition band between two flat colours spans several
//! pixels, and those intermediate pixels cluster into regions of their own:
//! a halo of thin slivers along every edge, and a boundary that wanders
//! pixel to pixel and so fits into many curve segments. No tracing option
//! fixes that after the fact. This module restores the piecewise-constant
//! structure the input had before it was degraded.
//!
//! | Module | Job |
//! |---|---|
//! | [`estimate`] | measure how blurred and how noisy the raster is |
//!
//! Phase 1 ships the signals only. The filters and the pass that ties them
//! together follow in the next phases.

pub mod estimate;

/// The calibration fixtures of `CLEANUP_IMPLEMENTATION_PLAN.md` §10.7, shared
/// with the integration tests so each drawing exists once.
#[cfg(test)]
#[path = "../../tests/common/degraded.rs"]
pub(crate) mod degraded;
