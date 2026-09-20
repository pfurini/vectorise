//! Shared helpers for the integration tests.
//!
//! The implementation plan calls this module `gen`, but `gen` is a reserved
//! keyword in edition 2024, so it is `fixtures` here.

// `unreachable_pub` and clippy's `redundant_pub_crate` disagree about a helper
// module inside a test binary: one wants `pub(crate)`, the other wants `pub`.
// `pub` reads better and nothing outside the test binary can see it either way.
#![allow(unreachable_pub)]

pub mod fixtures;
