//! Measure how wrong a candidate is, in square pixels of the original image.
//!
//! The measure is the **symmetric difference**: the area covered by exactly one
//! of the two shapes. Zero means they coincide; the larger it gets, the worse
//! the substitution. It is the only fidelity criterion the shape pass uses, so
//! a candidate that looks nothing like the outline cannot slip through however
//! confidently it was proposed.
//!
//! # How it is computed
//!
//! Both paths are rasterized into single-channel coverage maps with
//! `tiny-skia`, and the per-pixel absolute difference in coverage is summed.
//! Anti-aliasing is on, so a pixel the two shapes split differently contributes
//! its fractional difference rather than a whole pixel; that is what makes the
//! estimate usable at modest resolutions.
//!
//! The shared bounding box is scaled so its longest side is at most
//! [`MAX_RASTER_SIDE`] pixels, and the result is scaled back, so the cost of
//! verifying a shape does not grow with the size of the image. The returned
//! number is always in the original coordinate system's square pixels.
//!
//! Where both shapes partially cover the same pixel, `|a - b|` understates the
//! true symmetric difference (two half-covered pixels could disagree about
//! *which* half). The error is bounded by the length of the shared boundary in
//! raster pixels, which is exactly where the two shapes agree anyway.

use kurbo::{BezPath, PathEl, Rect, Shape as _};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

/// Longest side, in pixels, the raster used for a comparison may reach.
pub const MAX_RASTER_SIDE: f32 = 512.0;

/// Shortest side the raster is allowed to shrink to.
///
/// A 6 px shape compared at 6 px would be measured to the nearest whole pixel,
/// which is the same order as the tolerances we are testing against. Small
/// shapes are therefore rendered larger than life.
pub const MIN_RASTER_SIDE: f32 = 64.0;

/// Resolution per input pixel, before the two bounds above are applied.
const OVERSAMPLE: f32 = 2.0;

/// Border, in raster pixels, kept around the shapes so anti-aliased edges are
/// never clipped by the edge of the pixmap.
const RASTER_MARGIN: f32 = 2.0;

/// Area covered by exactly one of `a` and `b`, in square pixels of the input
/// coordinate system.
///
/// Returns `0.0` when both paths are empty, and `f64::INFINITY` when the
/// comparison cannot be made at all (a pixmap that cannot be allocated, a
/// bounding box that is not finite). Infinity is the safe answer: it fails
/// every tolerance, so an unverifiable candidate is never accepted.
///
/// # Examples
///
/// ```
/// use kurbo::{Rect, Shape as _};
/// use vectorise::geom::verify::symmetric_difference;
///
/// let square = Rect::new(0.0, 0.0, 10.0, 10.0).to_path(1e-9);
/// assert!(symmetric_difference(&square, &square) < 0.5);
/// ```
#[must_use]
pub fn symmetric_difference(a: &BezPath, b: &BezPath) -> f64 {
    let empty_a = is_empty(a);
    let empty_b = is_empty(b);
    match (empty_a, empty_b) {
        (true, true) => return 0.0,
        (true, false) => return b.area().abs(),
        (false, true) => return a.area().abs(),
        (false, false) => {}
    }

    let Some(bounds) = union_bounds(a, b) else {
        return f64::INFINITY;
    };

    let longest = f32_of(bounds.width().max(bounds.height()));
    if !longest.is_finite() || longest <= 0.0 {
        return f64::INFINITY;
    }

    // Oversample, then clamp: large shapes are scaled down so a comparison
    // costs a bounded amount of work, and small ones are scaled up so there is
    // something to measure.
    let target = (longest * OVERSAMPLE).clamp(MIN_RASTER_SIDE, MAX_RASTER_SIDE);
    let scale = target / longest;
    if !scale.is_finite() || scale <= 0.0 {
        return f64::INFINITY;
    }

    let width = f32_of(bounds.width())
        .mul_add(scale, 2.0 * RASTER_MARGIN)
        .ceil();
    let height = f32_of(bounds.height())
        .mul_add(scale, 2.0 * RASTER_MARGIN)
        .ceil();
    let (Some(width), Some(height)) = (raster_side(width), raster_side(height)) else {
        return f64::INFINITY;
    };

    let transform = Transform::from_translate(
        f32_of(bounds.x0).mul_add(-scale, RASTER_MARGIN),
        f32_of(bounds.y0).mul_add(-scale, RASTER_MARGIN),
    )
    .pre_scale(scale, scale);

    let (Some(coverage_a), Some(coverage_b)) = (
        coverage(a, width, height, transform),
        coverage(b, width, height, transform),
    ) else {
        return f64::INFINITY;
    };

    // Only the alpha channel carries coverage; the paint is opaque white.
    let difference: u64 = coverage_a
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .zip(coverage_b.data().as_chunks::<4>().0)
        .map(|([.., alpha_a], [.., alpha_b])| u64::from(alpha_a.abs_diff(*alpha_b)))
        .sum();

    // Back to input units: alpha 255 is one raster pixel, and one raster pixel
    // is 1 / scale^2 input pixels.
    let raster_pixels = coverage_units(difference) / 255.0;
    raster_pixels / f64::from(scale) / f64::from(scale)
}

/// The symmetric difference as a fraction of `reference`'s own area.
///
/// Convenient for reporting; the pass itself compares against an absolute
/// tolerance so tiny shapes are not judged by a percentage of almost nothing.
#[must_use]
pub fn relative_difference(reference: &BezPath, candidate: &BezPath) -> f64 {
    let area = reference.area().abs();
    if area <= 0.0 {
        return f64::INFINITY;
    }
    symmetric_difference(reference, candidate) / area
}

/// Rasterize one path's coverage into an alpha channel.
fn coverage(path: &BezPath, width: u32, height: u32, transform: Transform) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width, height)?;
    let skia_path = to_skia(path)?;

    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    paint.anti_alias = true;

    // Non-zero winding matches how the tracer and SVG fill shapes by default.
    pixmap.fill_path(&skia_path, &paint, FillRule::Winding, transform, None);
    Some(pixmap)
}

/// `kurbo` path to `tiny-skia` path.
fn to_skia(path: &BezPath) -> Option<tiny_skia::Path> {
    let mut builder = PathBuilder::new();
    for element in path.elements() {
        match *element {
            PathEl::MoveTo(p) => builder.move_to(f32_of(p.x), f32_of(p.y)),
            PathEl::LineTo(p) => builder.line_to(f32_of(p.x), f32_of(p.y)),
            PathEl::QuadTo(c, end) => {
                builder.quad_to(f32_of(c.x), f32_of(c.y), f32_of(end.x), f32_of(end.y));
            }
            PathEl::CurveTo(c1, c2, end) => builder.cubic_to(
                f32_of(c1.x),
                f32_of(c1.y),
                f32_of(c2.x),
                f32_of(c2.y),
                f32_of(end.x),
                f32_of(end.y),
            ),
            PathEl::ClosePath => builder.close(),
        }
    }
    builder.finish()
}

/// The bounding box containing both paths, if it is finite.
fn union_bounds(a: &BezPath, b: &BezPath) -> Option<Rect> {
    let bounds = a.bounding_box().union(b.bounding_box());
    let finite = bounds.x0.is_finite()
        && bounds.y0.is_finite()
        && bounds.x1.is_finite()
        && bounds.y1.is_finite();
    finite.then_some(bounds)
}

/// Does this path contain any drawing command?
fn is_empty(path: &BezPath) -> bool {
    !path.elements().iter().any(|element| {
        matches!(
            element,
            PathEl::LineTo(_) | PathEl::QuadTo(..) | PathEl::CurveTo(..)
        )
    })
}

/// A pixmap side length, rejecting anything that cannot be one.
///
/// The ceiling is generous: with the raster budget capped at
/// [`MAX_RASTER_SIDE`] a side can never approach it, so hitting this bound
/// means the geometry was nonsense.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the range check above leaves only values a u32 holds exactly"
)]
fn raster_side(value: f32) -> Option<u32> {
    (value.is_finite() && (1.0..=16_384.0).contains(&value)).then_some(value as u32)
}

/// The rasterizer works in `f32`; every coordinate crossing into it goes
/// through here so the narrowing has one documented home.
#[expect(
    clippy::cast_possible_truncation,
    reason = "tiny-skia is f32 throughout; the loss is below a raster pixel"
)]
const fn f32_of(value: f64) -> f32 {
    value as f32
}

/// Summed alpha as a float. The sum is bounded by 255 * 516^2, far inside an
/// f64 mantissa.
#[expect(
    clippy::cast_precision_loss,
    reason = "bounded by 255 * MAX_RASTER_SIDE^2, which f64 represents exactly"
)]
const fn coverage_units(total: u64) -> f64 {
    total as f64
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use kurbo::{Affine, BezPath, Circle, Point, Rect, Shape as _};

    use super::{relative_difference, symmetric_difference};

    fn circle(r: f64) -> BezPath {
        Circle::new(Point::new(r, r), r).to_path(1e-3)
    }

    #[test]
    fn symmetric_difference_of_identical_paths_is_zero() {
        let square = Rect::new(0.0, 0.0, 40.0, 40.0).to_path(1e-9);
        assert!(
            symmetric_difference(&square, &square) < 0.5,
            "identical paths differ only by rasterization noise"
        );

        let disc = circle(30.0);
        assert!(symmetric_difference(&disc, &disc) < 1.0);
    }

    #[test]
    fn symmetric_difference_of_circle_vs_inscribed_square_matches_analytic_area_within_2pct() {
        // A square inscribed in a circle of radius r has side r * sqrt(2), so
        // the circle covers pi*r^2 - 2*r^2 more than the square does, and the
        // square covers nothing the circle does not.
        let r = 100.0;
        let disc = circle(r);
        let half_side = r / std::f64::consts::SQRT_2;
        let square =
            Rect::new(r - half_side, r - half_side, r + half_side, r + half_side).to_path(1e-9);

        let expected = (std::f64::consts::PI - 2.0) * r * r;
        let measured = symmetric_difference(&disc, &square);
        let error = (measured - expected).abs() / expected;

        assert!(error < 0.02, "expected {expected}, measured {measured}");
    }

    #[test]
    fn symmetric_difference_is_symmetric() {
        let disc = circle(50.0);
        let square = Rect::new(0.0, 0.0, 100.0, 100.0).to_path(1e-9);
        let forward = symmetric_difference(&disc, &square);
        let backward = symmetric_difference(&square, &disc);
        assert!(
            (forward - backward).abs() < 1.0,
            "{forward} then {backward}"
        );
    }

    #[test]
    fn verify_scales_bbox_to_at_most_512px_and_scales_result_back() {
        // Far larger than the raster budget: the answer must still be in input
        // square pixels, not raster ones.
        let side = 4_000.0;
        let big = Rect::new(0.0, 0.0, side, side).to_path(1e-9);
        let inner = Rect::new(0.0, 0.0, side, side / 2.0).to_path(1e-9);

        let expected = side * side / 2.0;
        let measured = symmetric_difference(&big, &inner);
        let error = (measured - expected).abs() / expected;

        assert!(
            error < 0.01,
            "expected about {expected} input px^2, measured {measured}"
        );
    }

    #[test]
    fn verify_handles_subpixel_shapes_without_panic() {
        let tiny = Rect::new(0.0, 0.0, 0.3, 0.2).to_path(1e-9);
        let other = Rect::new(0.1, 0.1, 0.4, 0.3).to_path(1e-9);

        let measured = symmetric_difference(&tiny, &other);
        assert!(measured.is_finite(), "{measured}");
        assert!(measured >= 0.0);
    }

    #[test]
    fn verify_of_disjoint_shapes_is_the_sum_of_their_areas() {
        let left = Rect::new(0.0, 0.0, 10.0, 10.0).to_path(1e-9);
        let right = Rect::new(50.0, 0.0, 60.0, 10.0).to_path(1e-9);

        let measured = symmetric_difference(&left, &right);
        assert!((measured - 200.0).abs() / 200.0 < 0.02, "{measured}");
    }

    #[test]
    fn verify_against_an_empty_path_is_the_other_path_area() {
        let square = Rect::new(0.0, 0.0, 10.0, 10.0).to_path(1e-9);
        let empty = BezPath::new();

        assert!((symmetric_difference(&square, &empty) - 100.0).abs() < 1e-6);
        assert!((symmetric_difference(&empty, &square) - 100.0).abs() < 1e-6);
        assert!(symmetric_difference(&empty, &empty).abs() < f64::EPSILON);
    }

    #[test]
    fn relative_difference_normalizes_by_the_reference_area() {
        let square = Rect::new(0.0, 0.0, 100.0, 100.0).to_path(1e-9);
        let half = Rect::new(0.0, 0.0, 100.0, 50.0).to_path(1e-9);

        let fraction = relative_difference(&square, &half);
        assert!((fraction - 0.5).abs() < 0.02, "{fraction}");

        assert!(relative_difference(&BezPath::new(), &square).is_infinite());
    }

    #[test]
    fn verify_is_translation_invariant() {
        let disc = circle(40.0);
        let square = Rect::new(0.0, 0.0, 80.0, 80.0).to_path(1e-9);
        let at_origin = symmetric_difference(&disc, &square);

        let shift = Affine::translate((1_000.0, -500.0));
        let moved = symmetric_difference(&(shift * disc), &(shift * square));

        assert!(
            (at_origin - moved).abs() / at_origin < 0.02,
            "{at_origin} then {moved}"
        );
    }
}
