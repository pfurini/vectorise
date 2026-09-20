//! Propose the primitives a traced outline could be.
//!
//! Nothing here accepts or rejects anything; every function returns a
//! *candidate* that [`crate::geom::verify`] then measures against the original
//! outline. Being generous here and strict there is what keeps the detector
//! honest: a wrong candidate costs a raster comparison, never a wrong output.
//!
//! # Ellipses come from moments
//!
//! Green's theorem gives the area integrals of a closed path in one pass
//! (`kurbo::ParamCurveMoments`). From them:
//!
//! ```text
//! cx = Mx / A                    cy = My / A
//! uxx = Mxx / A - cx^2           uyy = Myy / A - cy^2
//! uxy = Mxy / A - cx * cy
//! ```
//!
//! For an ellipse the central second moments are `rx^2 / 4` and `ry^2 / 4`
//! about its own axes, so diagonalizing the 2x2 matrix `[[uxx, uxy], [uxy,
//! uyy]]` recovers both radii and the rotation at once. The same numbers exist
//! for any shape, which is exactly why the result has to be verified: a square
//! has moments too, and they describe an ellipse that is nothing like it.
//!
//! # Rectangles come from structure
//!
//! Moments cannot tell a rectangle from a rounded one, so rectangles are found
//! structurally: straight segments only, four corners after collinear cleanup,
//! square angles, axis-aligned edges. Rotated rectangles are not converted in
//! v1 (ADR-0004).

use kurbo::{BezPath, ParamCurveMoments as _, PathEl, Point, Rect, Shape as _};

use crate::geom::approx_eq;

/// Points closer than this are the same point.
const COINCIDENT_EPS: f64 = 1e-6;

/// Perpendicular distance below which three points count as collinear.
const COLLINEAR_EPS: f64 = 1e-3;

/// How far a cubic's control points may stray from the straight line between
/// its endpoints before it stops counting as a straight edge.
///
/// VTracer's spline fitter emits a cubic for every segment, including the flat
/// sides of a rectangle, with both control points sitting exactly on the line.
/// Without this, no traced rectangle would ever be recognized as one.
const STRAIGHT_CUBIC_EPS: f64 = 0.1;

/// How far an edge may lean off an axis and still count as axis-aligned.
const AXIS_ALIGNED_TOLERANCE_DEG: f64 = 0.5;

/// How far a corner may deviate from a right angle.
const RIGHT_ANGLE_TOLERANCE_DEG: f64 = 1.0;

/// Below this rotation an ellipse is treated as axis-aligned.
pub const MIN_ROTATION_DEG: f64 = 0.5;

/// A primitive proposed for an outline, before verification.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Candidate {
    /// A circle.
    Circle {
        /// Center x.
        cx: f64,
        /// Center y.
        cy: f64,
        /// Radius.
        r: f64,
    },
    /// An ellipse, possibly rotated.
    Ellipse {
        /// Center x.
        cx: f64,
        /// Center y.
        cy: f64,
        /// Semi-axis along the rotated x direction.
        rx: f64,
        /// Semi-axis along the rotated y direction.
        ry: f64,
        /// Rotation in degrees, in (-90, 90].
        rotate_deg: f64,
    },
    /// An axis-aligned rectangle, with optional equal corner radii.
    Rect {
        /// Left edge.
        x: f64,
        /// Top edge.
        y: f64,
        /// Width.
        w: f64,
        /// Height.
        h: f64,
        /// Corner radius. Zero for a sharp rectangle.
        r: f64,
    },
}

impl Candidate {
    /// The candidate as a path, so it can be rasterized next to the original.
    #[must_use]
    pub fn to_path(self) -> BezPath {
        match self {
            Self::Circle { cx, cy, r } => kurbo::Circle::new((cx, cy), r).to_path(1e-3),
            Self::Ellipse {
                cx,
                cy,
                rx,
                ry,
                rotate_deg,
            } => kurbo::Ellipse::new((cx, cy), (rx, ry), rotate_deg.to_radians()).to_path(1e-3),
            Self::Rect { x, y, w, h, r } if r > 0.0 => {
                kurbo::RoundedRect::new(x, y, x + w, y + h, r).to_path(1e-3)
            }
            Self::Rect { x, y, w, h, .. } => Rect::new(x, y, x + w, y + h).to_path(1e-3),
        }
    }
}

/// The shape statistics every fit starts from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moments {
    /// Unsigned area.
    pub area: f64,
    /// Centroid x.
    pub cx: f64,
    /// Centroid y.
    pub cy: f64,
    /// Central second moment about x, normalized by area.
    pub uxx: f64,
    /// Central product moment, normalized by area.
    pub uxy: f64,
    /// Central second moment about y, normalized by area.
    pub uyy: f64,
}

/// Area, centroid, and central second moments of a closed path.
///
/// The sign of the signed area is normalized away, so the result does not
/// depend on winding direction.
///
/// Returns `None` for a path with no area to speak of, where the centroid and
/// the moments would be divisions by (almost) zero.
///
/// # Examples
///
/// ```
/// use kurbo::{Rect, Shape as _};
/// use vectorise::geom::fit::moments;
///
/// let m = moments(&Rect::new(0.0, 0.0, 4.0, 2.0).to_path(1e-9)).expect("has area");
/// assert!((m.area - 8.0).abs() < 1e-9);
/// assert!((m.cx - 2.0).abs() < 1e-9 && (m.cy - 1.0).abs() < 1e-9);
/// ```
#[must_use]
pub fn moments(path: &BezPath) -> Option<Moments> {
    let signed_area = path.area();
    if signed_area.abs() < COINCIDENT_EPS {
        return None;
    }

    let raw = path.moments();
    // Flip every integral together with the area so a clockwise outline gives
    // the same answer as a counter-clockwise one.
    let sign = signed_area.signum();
    let area = signed_area.abs();
    let (mx, my) = (raw.moment_x * sign, raw.moment_y * sign);
    let (mxx, mxy, myy) = (
        raw.moment_xx * sign,
        raw.moment_xy * sign,
        raw.moment_yy * sign,
    );

    let cx = mx / area;
    let cy = my / area;
    Some(Moments {
        area,
        cx,
        cy,
        uxx: cx.mul_add(-cx, mxx / area),
        uxy: cx.mul_add(-cy, mxy / area),
        uyy: cy.mul_add(-cy, myy / area),
    })
}

/// The ellipse whose moments match this outline's.
///
/// Returns `None` when the outline has no usable area, or when the recovered
/// radii are degenerate.
#[must_use]
pub fn ellipse_candidate(path: &BezPath) -> Option<Candidate> {
    let m = moments(path)?;

    // Eigenvalues of [[uxx, uxy], [uxy, uyy]] are the variances along the
    // principal axes; for an ellipse each is (semi-axis / 2)^2.
    let mean = f64::midpoint(m.uxx, m.uyy);
    let half_difference = (m.uxx - m.uyy) / 2.0;
    let spread = half_difference
        .mul_add(half_difference, m.uxy * m.uxy)
        .sqrt();
    let major_variance = mean + spread;
    let minor_variance = mean - spread;
    if major_variance <= 0.0 || minor_variance <= 0.0 {
        return None;
    }

    let rx = 2.0 * major_variance.sqrt();
    let ry = 2.0 * minor_variance.sqrt();
    if !rx.is_finite() || !ry.is_finite() || rx <= 0.0 || ry <= 0.0 {
        return None;
    }

    // atan2 of the doubled angle: the principal axis direction is only defined
    // modulo 180 degrees, which is exactly what an ellipse needs.
    let rotate_deg = normalize_rotation(0.5 * (2.0 * m.uxy).atan2(m.uxx - m.uyy).to_degrees());

    Some(Candidate::Ellipse {
        cx: m.cx,
        cy: m.cy,
        rx,
        ry,
        rotate_deg,
    })
}

/// The circle whose area and centroid match this outline's.
///
/// A circle is proposed for any outline with area; whether it is *right* is
/// verification's problem.
#[must_use]
pub fn circle_candidate(path: &BezPath) -> Option<Candidate> {
    let m = moments(path)?;
    let r = (m.area / std::f64::consts::PI).sqrt();
    if !r.is_finite() || r <= 0.0 {
        return None;
    }
    Some(Candidate::Circle {
        cx: m.cx,
        cy: m.cy,
        r,
    })
}

/// Is this ellipse candidate round enough to be a circle?
///
/// `tolerance` is a fraction of the larger radius.
#[must_use]
pub fn is_round(candidate: Candidate, tolerance: f64) -> bool {
    match candidate {
        Candidate::Circle { .. } => true,
        Candidate::Ellipse { rx, ry, .. } => {
            let larger = rx.max(ry);
            larger > 0.0 && (rx - ry).abs() / larger <= tolerance
        }
        Candidate::Rect { .. } => false,
    }
}

/// The axis-aligned rectangle this outline is, if it is one.
///
/// Requires straight segments only, exactly four corners once collinear points
/// are dropped, right angles to within one degree, and edges within half a
/// degree of an axis. Rotated rectangles are deliberately not detected
/// (ADR-0004).
#[must_use]
pub fn rect_candidate(path: &BezPath) -> Option<Candidate> {
    let corners = polygon_corners(path)?;
    let [a, b, c, d] = corners.as_slice() else {
        return None;
    };

    for (from, to) in [(a, b), (b, c), (c, d), (d, a)] {
        if !is_axis_aligned(*from, *to) {
            return None;
        }
    }
    for (previous, corner, next) in [(d, a, b), (a, b, c), (b, c, d), (c, d, a)] {
        if !is_right_angle(*previous, *corner, *next) {
            return None;
        }
    }

    let bbox = path.bounding_box();
    if bbox.width() <= 0.0 || bbox.height() <= 0.0 {
        return None;
    }
    Some(Candidate::Rect {
        x: bbox.x0,
        y: bbox.y0,
        w: bbox.width(),
        h: bbox.height(),
        r: 0.0,
    })
}

/// The axis-aligned rounded rectangle this outline is, if it is one.
///
/// Requires exactly four straight edges, each lying on one side of the
/// bounding box and axis-aligned, with at least one curve per corner between
/// them. The corner count is not fixed: `kurbo` emits two cubics per corner and
/// a spline fit of a traced outline may emit more.
///
/// The radius is the average of the four insets the straight edges imply, which
/// is only meaningful when the corners really do share a radius; verification
/// catches the cases where they do not.
#[must_use]
pub fn rounded_rect_candidate(path: &BezPath) -> Option<Candidate> {
    let bbox = path.bounding_box();
    if bbox.width() <= 0.0 || bbox.height() <= 0.0 {
        return None;
    }

    let mut straight = Vec::new();
    let mut curves = 0usize;
    let mut current = Point::ZERO;
    let mut start = Point::ZERO;
    for element in path.elements() {
        match *element {
            PathEl::MoveTo(p) => {
                current = p;
                start = p;
            }
            PathEl::LineTo(p) => {
                if current.distance(p) > COINCIDENT_EPS {
                    straight.push((current, p));
                }
                current = p;
            }
            PathEl::CurveTo(c1, c2, end) => {
                if is_straight_cubic(current, c1, c2, end) {
                    if current.distance(end) > COINCIDENT_EPS {
                        straight.push((current, end));
                    }
                } else {
                    curves += 1;
                }
                current = end;
            }
            PathEl::QuadTo(c, end) => {
                if is_straight_cubic(current, c, c, end) {
                    if current.distance(end) > COINCIDENT_EPS {
                        straight.push((current, end));
                    }
                } else {
                    curves += 1;
                }
                current = end;
            }
            PathEl::ClosePath => {
                if current.distance(start) > COINCIDENT_EPS {
                    straight.push((current, start));
                }
                current = start;
            }
        }
    }

    // One curve per corner at least. A shape with no curve is a sharp
    // rectangle, which `rect_candidate` already handles.
    if curves < 4 || straight.len() != 4 {
        return None;
    }
    for (from, to) in &straight {
        if !is_axis_aligned(*from, *to) || !lies_on_bbox_side(*from, *to, bbox) {
            return None;
        }
    }

    // Each straight edge is the full side minus two corner radii.
    let mut radii = Vec::with_capacity(4);
    for (from, to) in &straight {
        let horizontal = (to.x - from.x).abs() > (to.y - from.y).abs();
        let (length, side) = if horizontal {
            ((to.x - from.x).abs(), bbox.width())
        } else {
            ((to.y - from.y).abs(), bbox.height())
        };
        radii.push((side - length) / 2.0);
    }

    let radius = radii.iter().sum::<f64>() / 4.0;
    if radius <= 0.0 || radius > bbox.width() / 2.0 || radius > bbox.height() / 2.0 {
        return None;
    }

    Some(Candidate::Rect {
        x: bbox.x0,
        y: bbox.y0,
        w: bbox.width(),
        h: bbox.height(),
        r: radius,
    })
}

/// The corners of a straight-edged closed outline, with collinear points and
/// duplicates removed.
///
/// A cubic whose control points lie on the line between its endpoints counts
/// as a straight edge: that is how the spline fitter writes one.
///
/// Returns `None` if the path has a real curve, more than one subpath, or
/// fewer than three corners.
#[must_use]
pub fn polygon_corners(path: &BezPath) -> Option<Vec<Point>> {
    let mut points: Vec<Point> = Vec::new();
    let mut moves = 0usize;

    for element in path.elements() {
        match *element {
            PathEl::MoveTo(p) => {
                moves += 1;
                if moves > 1 {
                    return None;
                }
                points.push(p);
            }
            PathEl::LineTo(p) => points.push(p),
            PathEl::CurveTo(c1, c2, end) => {
                let from = *points.last()?;
                if !is_straight_cubic(from, c1, c2, end) {
                    return None;
                }
                points.push(end);
            }
            PathEl::QuadTo(c, end) => {
                let from = *points.last()?;
                // A quadratic is straight on the same condition, with one
                // control point instead of two.
                if !is_straight_cubic(from, c, c, end) {
                    return None;
                }
                points.push(end);
            }
            PathEl::ClosePath => {}
        }
    }

    // A closed outline often repeats its first point as its last.
    if let (Some(first), Some(last)) = (points.first().copied(), points.last().copied())
        && points.len() > 1
        && first.distance(last) <= COINCIDENT_EPS
    {
        points.pop();
    }
    points.dedup_by(|a, b| a.distance(*b) <= COINCIDENT_EPS);

    if points.len() < 3 {
        return None;
    }

    // Drop any point that sits on the line between its neighbours. Repeat
    // until nothing changes: removing one point can expose another.
    loop {
        let before = points.len();
        let mut kept: Vec<Point> = Vec::with_capacity(before);
        for index in 0..before {
            let (Some(&previous), Some(&corner), Some(&next)) = (
                points.get((index + before - 1) % before),
                points.get(index),
                points.get((index + 1) % before),
            ) else {
                return None;
            };
            if !is_collinear(previous, corner, next) {
                kept.push(corner);
            }
        }
        if kept.len() < 3 {
            return None;
        }
        points = kept;
        if points.len() == before {
            break;
        }
    }

    Some(points)
}

/// Is this cubic a straight line written as a curve?
///
/// Both control points must sit on the segment between the endpoints, and
/// between them rather than beyond either end.
fn is_straight_cubic(from: Point, c1: Point, c2: Point, to: Point) -> bool {
    let span = to - from;
    let length = span.hypot();
    if length < COINCIDENT_EPS {
        return false;
    }

    for control in [c1, c2] {
        let offset = control - from;
        let along = offset.dot(span) / length;
        if !(-STRAIGHT_CUBIC_EPS..=length + STRAIGHT_CUBIC_EPS).contains(&along) {
            return false;
        }
        let across = offset.y.mul_add(-span.x, offset.x * span.y).abs() / length;
        if across > STRAIGHT_CUBIC_EPS {
            return false;
        }
    }
    true
}

/// Does this edge sit on one of the bounding box's four sides?
///
/// Without this a shape with four incidental axis-aligned segments would be
/// proposed as a rounded rectangle, and verification would have to do the work
/// the detector should have done.
fn lies_on_bbox_side(from: Point, to: Point, bbox: Rect) -> bool {
    let tolerance = (bbox.width().max(bbox.height()) * 1e-3).max(COINCIDENT_EPS);
    let horizontal = (to.x - from.x).abs() > (to.y - from.y).abs();
    if horizontal {
        let y = f64::midpoint(from.y, to.y);
        approx_eq(y, bbox.y0, tolerance) || approx_eq(y, bbox.y1, tolerance)
    } else {
        let x = f64::midpoint(from.x, to.x);
        approx_eq(x, bbox.x0, tolerance) || approx_eq(x, bbox.x1, tolerance)
    }
}

/// Is `b` on the straight line from `a` to `c`?
fn is_collinear(a: Point, b: Point, c: Point) -> bool {
    let cross = (b.x - a.x).mul_add(c.y - a.y, -((b.y - a.y) * (c.x - a.x)));
    let base = a.distance(c);
    if base < COINCIDENT_EPS {
        return true;
    }
    cross.abs() / base < COLLINEAR_EPS
}

/// Does this edge run along an axis, to within half a degree?
fn is_axis_aligned(from: Point, to: Point) -> bool {
    let dx = (to.x - from.x).abs();
    let dy = (to.y - from.y).abs();
    if dx < COINCIDENT_EPS && dy < COINCIDENT_EPS {
        return false;
    }
    let lean_deg = dy.min(dx).atan2(dy.max(dx)).to_degrees();
    lean_deg <= AXIS_ALIGNED_TOLERANCE_DEG
}

/// Is the interior angle at `corner` a right angle, to within one degree?
fn is_right_angle(previous: Point, corner: Point, next: Point) -> bool {
    let incoming = corner - previous;
    let outgoing = next - corner;
    if incoming.hypot() < COINCIDENT_EPS || outgoing.hypot() < COINCIDENT_EPS {
        return false;
    }
    let cosine = incoming.dot(outgoing) / (incoming.hypot() * outgoing.hypot());
    let turn_deg = cosine.clamp(-1.0, 1.0).acos().to_degrees();
    approx_eq(turn_deg, 90.0, RIGHT_ANGLE_TOLERANCE_DEG)
}

/// Fold a rotation into (-90, 90], the range in which an ellipse's axes are
/// unambiguous.
fn normalize_rotation(degrees: f64) -> f64 {
    let mut degrees = degrees % 180.0;
    if degrees > 90.0 {
        degrees -= 180.0;
    } else if degrees <= -90.0 {
        degrees += 180.0;
    }
    // Keep -0.0 from reaching the writer.
    if degrees == 0.0 { 0.0 } else { degrees }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use kurbo::{Affine, BezPath, Circle, Ellipse, Point, Rect, Shape as _, Vec2};

    use super::{
        Candidate, circle_candidate, ellipse_candidate, is_round, moments, polygon_corners,
        rect_candidate, rounded_rect_candidate,
    };
    use crate::geom::{approx_eq, approx_eq_relative};

    fn polygon(points: &[(f64, f64)]) -> BezPath {
        let mut path = BezPath::new();
        let mut points = points.iter();
        if let Some(&(x, y)) = points.next() {
            path.move_to((x, y));
        }
        for &(x, y) in points {
            path.line_to((x, y));
        }
        path.close_path();
        path
    }

    // --- moments -------------------------------------------------------------

    #[test]
    fn moments_of_axis_aligned_ellipse_bezpath_recover_center_and_radii() {
        let path = Ellipse::new(Point::new(30.0, 40.0), Vec2::new(20.0, 8.0), 0.0).to_path(1e-3);
        let Some(Candidate::Ellipse {
            cx,
            cy,
            rx,
            ry,
            rotate_deg,
        }) = ellipse_candidate(&path)
        else {
            unreachable!("an ellipse must yield an ellipse candidate");
        };

        assert!(approx_eq_relative(cx, 30.0, 0.005), "cx {cx}");
        assert!(approx_eq_relative(cy, 40.0, 0.005), "cy {cy}");
        assert!(approx_eq_relative(rx, 20.0, 0.005), "rx {rx}");
        assert!(approx_eq_relative(ry, 8.0, 0.005), "ry {ry}");
        assert!(
            rotate_deg.abs() < 0.5,
            "an axis-aligned ellipse has no rotation"
        );
    }

    #[test]
    fn moments_of_rotated_ellipse_recover_angle_mod_180() {
        for angle_deg in [10.0_f64, 34.377, 60.0, 120.0, 170.0] {
            let path = Ellipse::new(Point::ZERO, Vec2::new(20.0, 8.0), angle_deg.to_radians())
                .to_path(1e-3);
            let Some(Candidate::Ellipse { rotate_deg, .. }) = ellipse_candidate(&path) else {
                unreachable!("an ellipse must yield an ellipse candidate");
            };

            let difference = (rotate_deg - angle_deg).rem_euclid(180.0);
            let distance = difference.min(180.0 - difference);
            assert!(
                distance < 0.5,
                "expected {angle_deg} mod 180, recovered {rotate_deg}"
            );
        }
    }

    #[test]
    fn area_sign_is_normalized_regardless_of_winding() {
        let clockwise = polygon(&[(0.0, 0.0), (4.0, 0.0), (4.0, 3.0), (0.0, 3.0)]);
        let counter_clockwise = polygon(&[(0.0, 0.0), (0.0, 3.0), (4.0, 3.0), (4.0, 0.0)]);

        let a = moments(&clockwise).expect("has area");
        let b = moments(&counter_clockwise).expect("has area");

        assert!(a.area > 0.0 && b.area > 0.0, "area is never negative");
        assert!(approx_eq(a.area, b.area, 1e-9));
        assert!(approx_eq(a.cx, b.cx, 1e-9) && approx_eq(a.cy, b.cy, 1e-9));
        assert!(approx_eq(a.uxx, b.uxx, 1e-9) && approx_eq(a.uyy, b.uyy, 1e-9));
    }

    #[test]
    fn moments_of_a_degenerate_path_are_absent() {
        let line = polygon(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)]);
        assert!(moments(&line).is_none(), "a line has no area");
        assert!(ellipse_candidate(&line).is_none());
        assert!(circle_candidate(&line).is_none());
    }

    #[test]
    fn circle_candidate_preserves_area() {
        let path = Circle::new(Point::new(5.0, 6.0), 4.0).to_path(1e-4);
        let Some(Candidate::Circle { cx, cy, r }) = circle_candidate(&path) else {
            unreachable!("a circle must yield a circle candidate");
        };
        assert!(approx_eq_relative(cx, 5.0, 1e-3));
        assert!(approx_eq_relative(cy, 6.0, 1e-3));
        assert!(approx_eq_relative(r, 4.0, 1e-3), "r {r}");
    }

    #[test]
    fn is_round_separates_circles_from_stretched_ellipses() {
        let circle =
            ellipse_candidate(&Circle::new(Point::ZERO, 10.0).to_path(1e-4)).expect("candidate");
        assert!(is_round(circle, 0.02));

        let ellipse =
            ellipse_candidate(&Ellipse::new(Point::ZERO, Vec2::new(20.0, 8.0), 0.0).to_path(1e-4))
                .expect("candidate");
        assert!(!is_round(ellipse, 0.02));

        assert!(!is_round(
            Candidate::Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
                r: 0.0
            },
            1.0
        ));
    }

    // --- rectangles ----------------------------------------------------------

    #[test]
    fn rect_detector_accepts_axis_aligned_square() {
        let path = polygon(&[(2.0, 3.0), (12.0, 3.0), (12.0, 13.0), (2.0, 13.0)]);
        assert_eq!(
            rect_candidate(&path),
            Some(Candidate::Rect {
                x: 2.0,
                y: 3.0,
                w: 10.0,
                h: 10.0,
                r: 0.0
            })
        );
    }

    #[test]
    fn rect_detector_rejects_rotated_square() {
        let square = Rect::new(0.0, 0.0, 10.0, 10.0).to_path(1e-9);
        let rotated = Affine::rotate(20.0_f64.to_radians()) * square;
        assert_eq!(
            rect_candidate(&rotated),
            None,
            "ADR-0004: v1 keeps these as paths"
        );
    }

    #[test]
    fn rect_detector_rejects_trapezoid() {
        let path = polygon(&[(0.0, 0.0), (10.0, 0.0), (8.0, 6.0), (2.0, 6.0)]);
        assert_eq!(rect_candidate(&path), None);
    }

    #[test]
    fn rect_detector_accepts_after_collinear_midpoint_cleanup() {
        // A rectangle with an extra vertex in the middle of its top edge.
        let path = polygon(&[(0.0, 0.0), (5.0, 0.0), (10.0, 0.0), (10.0, 4.0), (0.0, 4.0)]);
        assert_eq!(
            rect_candidate(&path),
            Some(Candidate::Rect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 4.0,
                r: 0.0
            })
        );
    }

    #[test]
    fn rect_detector_rejects_a_curve() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((10.0, 0.0));
        path.curve_to((11.0, 1.0), (11.0, 3.0), (10.0, 4.0));
        path.line_to((0.0, 4.0));
        path.close_path();
        assert_eq!(rect_candidate(&path), None);
    }

    #[test]
    fn rect_detector_rejects_a_pentagon() {
        let path = polygon(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (12.0, 5.0),
            (5.0, 9.0),
            (-2.0, 5.0),
        ]);
        assert_eq!(rect_candidate(&path), None);
    }

    #[test]
    fn polygon_corners_rejects_multiple_subpaths() {
        let mut path = polygon(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
        path.move_to((10.0, 10.0));
        path.line_to((12.0, 10.0));
        path.line_to((12.0, 12.0));
        path.close_path();
        assert!(polygon_corners(&path).is_none());
    }

    #[test]
    fn polygon_corners_drops_the_repeated_closing_point() {
        let path = polygon(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0), (0.0, 0.0)]);
        let corners = polygon_corners(&path).expect("a rectangle");
        assert_eq!(corners.len(), 4);
    }

    // --- rounded rectangles ---------------------------------------------------

    #[test]
    fn rounded_rect_detector_recovers_radius() {
        let path = kurbo::RoundedRect::new(1.0, 2.0, 21.0, 12.0, 3.0).to_path(1e-4);
        let Some(Candidate::Rect { x, y, w, h, r }) = rounded_rect_candidate(&path) else {
            unreachable!("a rounded rectangle must be detected");
        };

        assert!(approx_eq(x, 1.0, 1e-6) && approx_eq(y, 2.0, 1e-6));
        assert!(approx_eq(w, 20.0, 1e-6) && approx_eq(h, 10.0, 1e-6));
        assert!(approx_eq_relative(r, 3.0, 0.01), "radius {r}");
    }

    #[test]
    fn rounded_rect_detector_rejects_a_sharp_rectangle() {
        let path = Rect::new(0.0, 0.0, 10.0, 4.0).to_path(1e-9);
        assert_eq!(rounded_rect_candidate(&path), None, "no corners to round");
    }

    #[test]
    fn rounded_rect_detector_rejects_a_circle() {
        let path = Circle::new(Point::ZERO, 5.0).to_path(1e-4);
        assert_eq!(rounded_rect_candidate(&path), None, "no straight edges");
    }

    #[test]
    fn rounded_rect_detector_rejects_a_rotated_rounded_rectangle() {
        let path = kurbo::RoundedRect::new(0.0, 0.0, 20.0, 10.0, 3.0).to_path(1e-4);
        let rotated = Affine::rotate(15.0_f64.to_radians()) * path;
        assert_eq!(rounded_rect_candidate(&rotated), None);
    }

    // --- guard paths -----------------------------------------------------------

    #[test]
    fn a_circle_candidate_is_always_round() {
        assert!(is_round(
            Candidate::Circle {
                cx: 0.0,
                cy: 0.0,
                r: 3.0
            },
            0.0
        ));
    }

    #[test]
    fn degenerate_candidates_are_refused() {
        // An hourglass: the two lobes cancel, leaving no usable area.
        let bowtie = polygon(&[(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)]);
        assert!(moments(&bowtie).is_none(), "the signed areas cancel");
        assert!(ellipse_candidate(&bowtie).is_none());
        assert!(circle_candidate(&bowtie).is_none());
        assert!(rounded_rect_candidate(&bowtie).is_none());

        // Zero extent in one direction: nothing to bound.
        let flat = polygon(&[(0.0, 0.0), (10.0, 0.0), (5.0, 0.0)]);
        assert!(rect_candidate(&flat).is_none());
        assert!(rounded_rect_candidate(&flat).is_none());
    }

    #[test]
    fn quadratic_segments_are_classified_like_cubics() {
        // A rectangle whose straight edges are written as quadratics.
        let mut straight = BezPath::new();
        straight.move_to((0.0, 0.0));
        straight.quad_to((5.0, 0.0), (10.0, 0.0));
        straight.quad_to((10.0, 2.0), (10.0, 4.0));
        straight.quad_to((5.0, 4.0), (0.0, 4.0));
        straight.quad_to((0.0, 2.0), (0.0, 0.0));
        straight.close_path();
        assert_eq!(
            rect_candidate(&straight),
            Some(Candidate::Rect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 4.0,
                r: 0.0
            }),
            "quadratics with control points on the line are straight edges"
        );

        // A genuinely curved quadratic is a corner, not an edge.
        let mut curved = BezPath::new();
        curved.move_to((2.0, 0.0));
        curved.line_to((8.0, 0.0));
        curved.quad_to((10.0, 0.0), (10.0, 2.0));
        curved.line_to((10.0, 8.0));
        curved.quad_to((10.0, 10.0), (8.0, 10.0));
        curved.line_to((2.0, 10.0));
        curved.quad_to((0.0, 10.0), (0.0, 8.0));
        curved.line_to((0.0, 2.0));
        curved.quad_to((0.0, 0.0), (2.0, 0.0));
        curved.close_path();
        assert!(
            rect_candidate(&curved).is_none(),
            "curved corners are not a sharp rect"
        );
        let Some(Candidate::Rect { r, .. }) = rounded_rect_candidate(&curved) else {
            unreachable!("four straight sides and four curved corners is a rounded rect");
        };
        assert!(approx_eq_relative(r, 2.0, 0.05), "radius {r}");
    }

    #[test]
    fn collinear_cleanup_tolerates_a_doubled_back_edge() {
        // Three points where the first and last coincide: the perpendicular
        // distance is undefined, and such a point is treated as collinear.
        let spike = polygon(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 4.0),
            (5.0, 4.0),
            (5.0, 4.0),
            (0.0, 4.0),
        ]);
        assert_eq!(
            rect_candidate(&spike),
            Some(Candidate::Rect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 4.0,
                r: 0.0
            })
        );
    }

    #[test]
    fn rotations_fold_into_the_open_interval_around_zero() {
        // An ellipse rotated past a right angle comes back as the equivalent
        // negative rotation, never as a value outside (-90, 90].
        for angle_deg in [95.0_f64, 135.0, 179.0, -95.0, -170.0] {
            let path = Ellipse::new(Point::ZERO, Vec2::new(20.0, 8.0), angle_deg.to_radians())
                .to_path(1e-3);
            let Some(Candidate::Ellipse { rotate_deg, .. }) = ellipse_candidate(&path) else {
                unreachable!("an ellipse must yield an ellipse candidate");
            };
            assert!(
                rotate_deg > -90.0 && rotate_deg <= 90.0,
                "{angle_deg} folded to {rotate_deg}"
            );
        }
    }

    #[test]
    fn a_rectangle_with_a_zero_length_edge_is_refused() {
        // Two coincident corners leave a triangle, not a rectangle.
        let degenerate = polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 0.0), (10.0, 5.0)]);
        assert_eq!(rect_candidate(&degenerate), None);
    }

    // --- candidate geometry ---------------------------------------------------

    #[test]
    fn candidates_render_back_to_the_shape_they_describe() {
        let circle = Candidate::Circle {
            cx: 0.0,
            cy: 0.0,
            r: 5.0,
        };
        assert!(approx_eq_relative(
            circle.to_path().area().abs(),
            std::f64::consts::PI * 25.0,
            0.01
        ));

        let rect = Candidate::Rect {
            x: 0.0,
            y: 0.0,
            w: 4.0,
            h: 3.0,
            r: 0.0,
        };
        assert!(approx_eq(rect.to_path().area().abs(), 12.0, 1e-6));

        let rounded = Candidate::Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 4.0,
            r: 1.5,
        };
        let expected = ((4.0 - std::f64::consts::PI) * 1.5).mul_add(-1.5, 40.0);
        assert!(approx_eq_relative(
            rounded.to_path().area().abs(),
            expected,
            0.01
        ));

        let ellipse = Candidate::Ellipse {
            cx: 0.0,
            cy: 0.0,
            rx: 20.0,
            ry: 8.0,
            rotate_deg: 30.0,
        };
        assert!(approx_eq_relative(
            ellipse.to_path().area().abs(),
            std::f64::consts::PI * 160.0,
            0.01
        ));
    }
}
