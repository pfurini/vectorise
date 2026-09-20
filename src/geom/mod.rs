//! Geometry: converting VTracer's paths to `kurbo`, proposing primitive
//! candidates, and measuring how far a candidate is from the traced outline.
//!
//! The three submodules run in that order and do one thing each:
//!
//! | Module | Question it answers |
//! |---|---|
//! | [`convert`] | what does this traced path look like as a [`kurbo::BezPath`]? |
//! | [`fit`] | which primitives could this outline be? |
//! | [`verify`] | how many square pixels does a candidate get wrong? |
//!
//! Nothing here decides anything. [`crate::shapes`] takes the candidates, asks
//! [`verify`] for a number, and compares it to the tolerance.

pub mod convert;
pub mod fit;
pub mod verify;

/// Are two floats equal to within `epsilon`?
///
/// Every float comparison in the geometry code goes through this, so the
/// `float_cmp` lint has exactly one place to be silenced and the tolerance is
/// always visible at the call site.
///
/// # Examples
///
/// ```
/// use vectorise::geom::approx_eq;
///
/// assert!(approx_eq(0.1 + 0.2, 0.3, 1e-12));
/// assert!(!approx_eq(1.0, 1.1, 1e-3));
/// ```
#[must_use]
pub fn approx_eq(a: f64, b: f64, epsilon: f64) -> bool {
    (a - b).abs() <= epsilon
}

/// Are two floats equal to within `epsilon` relative to their magnitude?
///
/// Use this when the values scale with the image: a 0.5 px absolute tolerance
/// means something different on a 16 px shape than on a 1600 px one.
///
/// # Examples
///
/// ```
/// use vectorise::geom::approx_eq_relative;
///
/// assert!(approx_eq_relative(1000.0, 1000.5, 0.01));
/// assert!(!approx_eq_relative(1.0, 1.5, 0.01));
/// ```
#[must_use]
pub fn approx_eq_relative(a: f64, b: f64, epsilon: f64) -> bool {
    let scale = a.abs().max(b.abs()).max(1.0);
    (a - b).abs() <= epsilon * scale
}

#[cfg(test)]
mod tests {
    use super::{approx_eq, approx_eq_relative};

    #[test]
    fn approx_eq_accepts_within_epsilon_and_rejects_outside() {
        assert!(approx_eq(1.0, 1.0, 0.0), "equal values need no slack");
        assert!(approx_eq(1.0, 1.000_1, 1e-3));
        assert!(!approx_eq(1.0, 1.01, 1e-3));
        assert!(approx_eq(-1.0, -1.000_1, 1e-3), "sign does not matter");
    }

    #[test]
    fn approx_eq_relative_scales_with_magnitude() {
        assert!(approx_eq_relative(1_000.0, 1_005.0, 0.01));
        assert!(!approx_eq_relative(1_000.0, 1_020.0, 0.01));
        // Below 1.0 the tolerance stops shrinking, so tiny values stay usable.
        assert!(approx_eq_relative(0.0, 0.005, 0.01));
    }
}
