//! Translate between VTracer's path IR and `kurbo`'s.
//!
//! The two are structurally the same (move, line, cubic, close, absolute
//! coordinates), so the conversion is lossless in both directions. Everything
//! downstream works in `kurbo`, which has the geometry we need.

use kurbo::{BezPath, PathEl, Point};
use vtracer::PointF64;
use vtracer::ir::{MultiPath, PathCmd, SubPath};

/// One traced outline as a `kurbo` path.
///
/// # Examples
///
/// ```
/// use vectorise::geom::convert::subpath_to_bezpath;
/// use vtracer::PointF64;
/// use vtracer::ir::{PathCmd, SubPath};
///
/// let subpath = SubPath {
///     commands: vec![
///         PathCmd::MoveTo(PointF64 { x: 0.0, y: 0.0 }),
///         PathCmd::LineTo(PointF64 { x: 1.0, y: 0.0 }),
///         PathCmd::Close,
///     ],
/// };
/// assert_eq!(subpath_to_bezpath(&subpath).elements().len(), 3);
/// ```
#[must_use]
pub fn subpath_to_bezpath(subpath: &SubPath) -> BezPath {
    let mut path = BezPath::new();
    append_subpath(&mut path, subpath);
    path
}

/// A whole shape, outer ring and holes alike, as one `kurbo` path.
#[must_use]
pub fn multipath_to_bezpath(multipath: &MultiPath) -> BezPath {
    let mut path = BezPath::new();
    for subpath in &multipath.subpaths {
        append_subpath(&mut path, subpath);
    }
    path
}

/// The reverse conversion, used by the round-trip test and by anything that
/// needs to hand geometry back to VTracer.
#[must_use]
pub fn bezpath_to_subpath(path: &BezPath) -> SubPath {
    let commands = path
        .elements()
        .iter()
        .map(|element| match *element {
            PathEl::MoveTo(p) => PathCmd::MoveTo(point(p)),
            PathEl::LineTo(p) => PathCmd::LineTo(point(p)),
            PathEl::CurveTo(c1, c2, end) => PathCmd::CubicTo(point(c1), point(c2), point(end)),
            // `kurbo` has quadratics; VTracer does not. Elevate to a cubic,
            // which is exact: a quadratic is a cubic with coincident controls
            // at two thirds of the way to the control point.
            PathEl::QuadTo(c, end) => PathCmd::CubicTo(point(c), point(c), point(end)),
            PathEl::ClosePath => PathCmd::Close,
        })
        .collect();
    SubPath { commands }
}

/// The single subpath a primitive could be fitted to, if there is exactly one.
///
/// A shape with holes, or with several disjoint rings, stays a path in v1: one
/// `<circle>` cannot express a ring, and the extra rings would be lost. See
/// `docs/shape-detection.md` for the false negatives this costs.
///
/// # Examples
///
/// ```
/// use vectorise::geom::convert::single_subpath;
/// use vtracer::ir::{MultiPath, SubPath};
/// use vtracer::PointF64;
/// use vtracer::ir::PathCmd;
///
/// let one = SubPath { commands: vec![PathCmd::MoveTo(PointF64 { x: 0.0, y: 0.0 })] };
/// let multi = MultiPath { subpaths: vec![one.clone(), one] };
/// assert!(single_subpath(&multi).is_none());
/// ```
#[must_use]
pub const fn single_subpath(multipath: &MultiPath) -> Option<&SubPath> {
    match multipath.subpaths.as_slice() {
        [only] => Some(only),
        _ => None,
    }
}

fn append_subpath(path: &mut BezPath, subpath: &SubPath) {
    for command in &subpath.commands {
        match *command {
            PathCmd::MoveTo(p) => path.move_to(pt(p)),
            PathCmd::LineTo(p) => path.line_to(pt(p)),
            PathCmd::CubicTo(c1, c2, end) => path.curve_to(pt(c1), pt(c2), pt(end)),
            PathCmd::Close => path.close_path(),
        }
    }
}

const fn pt(p: PointF64) -> Point {
    Point { x: p.x, y: p.y }
}

const fn point(p: Point) -> PointF64 {
    PointF64 { x: p.x, y: p.y }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use kurbo::{PathEl, Point, Shape as _};

    use super::{bezpath_to_subpath, multipath_to_bezpath, single_subpath, subpath_to_bezpath};
    use crate::geom::approx_eq;
    use vtracer::PointF64;
    use vtracer::ir::{MultiPath, PathCmd, SubPath};

    fn p(x: f64, y: f64) -> PointF64 {
        PointF64 { x, y }
    }

    fn square() -> SubPath {
        SubPath {
            commands: vec![
                PathCmd::MoveTo(p(0.0, 0.0)),
                PathCmd::LineTo(p(10.0, 0.0)),
                PathCmd::CubicTo(p(11.0, 3.0), p(11.0, 7.0), p(10.0, 10.0)),
                PathCmd::LineTo(p(0.0, 10.0)),
                PathCmd::Close,
            ],
        }
    }

    #[test]
    fn subpath_to_bezpath_preserves_command_sequence() {
        let path = subpath_to_bezpath(&square());
        let elements = path.elements();

        assert_eq!(elements.len(), 5);
        assert_eq!(elements[0], PathEl::MoveTo(Point::new(0.0, 0.0)));
        assert_eq!(elements[1], PathEl::LineTo(Point::new(10.0, 0.0)));
        assert_eq!(
            elements[2],
            PathEl::CurveTo(
                Point::new(11.0, 3.0),
                Point::new(11.0, 7.0),
                Point::new(10.0, 10.0)
            )
        );
        assert_eq!(elements[3], PathEl::LineTo(Point::new(0.0, 10.0)));
        assert_eq!(elements[4], PathEl::ClosePath);
    }

    #[test]
    fn bezpath_to_subpath_roundtrip_is_identity_within_1e_9() {
        let original = square();
        let round_tripped = bezpath_to_subpath(&subpath_to_bezpath(&original));

        assert_eq!(round_tripped.commands.len(), original.commands.len());
        for (after, before) in round_tripped.commands.iter().zip(&original.commands) {
            match (after, before) {
                (PathCmd::MoveTo(a), PathCmd::MoveTo(b))
                | (PathCmd::LineTo(a), PathCmd::LineTo(b)) => {
                    assert!(approx_eq(a.x, b.x, 1e-9) && approx_eq(a.y, b.y, 1e-9));
                }
                (PathCmd::CubicTo(a1, a2, a3), PathCmd::CubicTo(b1, b2, b3)) => {
                    for (a, b) in [(a1, b1), (a2, b2), (a3, b3)] {
                        assert!(approx_eq(a.x, b.x, 1e-9) && approx_eq(a.y, b.y, 1e-9));
                    }
                }
                (PathCmd::Close, PathCmd::Close) => {}
                mismatch => unreachable!("command kind changed: {mismatch:?}"),
            }
        }
    }

    #[test]
    fn quadratic_curves_are_elevated_to_cubics() {
        // `kurbo` can produce quadratics; VTracer's IR cannot represent them.
        let mut path = kurbo::BezPath::new();
        path.move_to((0.0, 0.0));
        path.quad_to((1.0, 2.0), (2.0, 0.0));
        let subpath = bezpath_to_subpath(&path);

        assert!(matches!(subpath.commands[1], PathCmd::CubicTo(c1, c2, _) if c1 == c2));
    }

    #[test]
    fn multipath_with_hole_is_not_candidate() {
        let outer = square();
        let inner = SubPath {
            commands: vec![
                PathCmd::MoveTo(p(3.0, 3.0)),
                PathCmd::LineTo(p(7.0, 3.0)),
                PathCmd::LineTo(p(7.0, 7.0)),
                PathCmd::Close,
            ],
        };
        let multipath = MultiPath {
            subpaths: vec![outer, inner],
        };

        assert!(single_subpath(&multipath).is_none());
        // It still converts, as one path with two subpaths.
        let path = multipath_to_bezpath(&multipath);
        assert_eq!(
            path.elements()
                .iter()
                .filter(|element| matches!(element, PathEl::MoveTo(_)))
                .count(),
            2
        );
    }

    #[test]
    fn single_subpath_returns_the_only_ring() {
        let multipath = MultiPath {
            subpaths: vec![square()],
        };
        assert!(single_subpath(&multipath).is_some());

        let empty = MultiPath { subpaths: vec![] };
        assert!(single_subpath(&empty).is_none());
    }

    #[test]
    fn converted_geometry_keeps_its_area() {
        let path = subpath_to_bezpath(&SubPath {
            commands: vec![
                PathCmd::MoveTo(p(0.0, 0.0)),
                PathCmd::LineTo(p(4.0, 0.0)),
                PathCmd::LineTo(p(4.0, 3.0)),
                PathCmd::LineTo(p(0.0, 3.0)),
                PathCmd::Close,
            ],
        });
        assert!(approx_eq(path.area().abs(), 12.0, 1e-9));
    }
}
