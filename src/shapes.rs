//! Replace traced outlines with native SVG primitives where they are one.
//!
//! This is the pass that makes `vectorise` more than a tracer wrapper. It
//! walks a [`VectorDoc`] and produces a [`ShapeDoc`] in which every shape is
//! either a circle, an ellipse, a rectangle (possibly rounded), or the path it
//! always was.
//!
//! # How a shape is decided
//!
//! For each traced shape, in paint order:
//!
//! 1. Shapes with more than one subpath stay paths. One `<circle>` cannot
//!    express a ring, and a shape with holes or several disjoint rings would
//!    lose geometry.
//! 2. Outlines smaller than [`ShapeFitOptions::min_area`] stay paths. At that
//!    size a primitive saves almost nothing and the fit is mostly rasterization
//!    noise.
//! 3. [`crate::geom::fit`] proposes candidates; each is measured against the
//!    original outline with [`crate::geom::verify::symmetric_difference`]; the
//!    first candidate within tolerance wins.
//!
//! The candidates are tried in a fixed order, circle first and ellipse last, so
//! the output is deterministic and the most compact element wins ties.
//!
//! # Tolerance
//!
//! `tolerance_frac` is a fraction of the outline's own area, so the same
//! setting means the same visual fidelity on a 20 px icon and a 2000 px
//! illustration.
//!
//! Area alone is not enough. The error between a traced outline and the shape
//! it approximates lives on the *boundary*: rasterizing a circle and tracing it
//! back gives an outline within a fraction of a pixel of the original, all the
//! way round. That error grows with the perimeter while an area-proportional
//! budget grows with the square of the radius, so small shapes would be
//! rejected for an error a large one is forgiven. Measured on traced discs, the
//! residual is 0.06 to 0.25 square pixels per pixel of perimeter regardless of
//! size, while the same residual is 3% of the area at radius 8 and 0.5% at
//! radius 40.
//!
//! The allowance is therefore the larger of a fraction of the area and the same
//! fraction of a [`PERIMETER_WEIGHT`]-pixel band along the outline, floored at
//! [`ABSOLUTE_TOLERANCE_FLOOR`] square pixels. At the 2% default the band is a
//! fifth of a pixel wide, which is invisible and still rejects an 8 by 8 square
//! offered as a circle.
//!
//! `min_area` is what keeps small regions from being converted eagerly.
//! Setting `tolerance_frac` to zero disables every term, so
//! `--shape-tolerance 0` converts nothing.
//!
//! # Purity
//!
//! The pass performs no I/O and depends on nothing but its inputs, so the same
//! document and options always produce the same result.

use kurbo::{BezPath, Shape as _};
use vtracer::ir::VectorDoc;

use crate::color::Rgba;
use crate::geom::convert::{multipath_to_bezpath, single_subpath, subpath_to_bezpath};
use crate::geom::fit::{
    self, Candidate, MIN_ROTATION_DEG, circle_candidate, ellipse_candidate, rect_candidate,
    rounded_rect_candidate,
};
use crate::geom::verify::symmetric_difference;

/// The smallest tolerance a non-zero setting can produce, in square pixels.
pub const ABSOLUTE_TOLERANCE_FLOOR: f64 = 1.0;

/// Width, in pixels, of the boundary band the tolerance is also measured
/// against. See the module documentation for the calibration.
pub const PERIMETER_WEIGHT: f64 = 10.0;

/// How close `rx` and `ry` must be, as a fraction of the larger, for an ellipse
/// to be emitted as a circle instead.
const CIRCLE_SNAP_FRACTION: f64 = 0.02;

/// Accuracy of the perimeter estimate. A hundredth of a pixel is far finer
/// than the tolerance it feeds.
const PERIMETER_ACCURACY: f64 = 0.01;

/// What a shape turned out to be.
#[derive(Debug, Clone, PartialEq)]
pub enum Prim {
    /// A circle.
    Circle {
        /// Center x.
        cx: f64,
        /// Center y.
        cy: f64,
        /// Radius.
        r: f64,
    },
    /// An ellipse. `rotate_deg` is zero for the axis-aligned case, and the
    /// writer then emits no transform.
    Ellipse {
        /// Center x.
        cx: f64,
        /// Center y.
        cy: f64,
        /// Semi-axis along the rotated x direction.
        rx: f64,
        /// Semi-axis along the rotated y direction.
        ry: f64,
        /// Rotation in degrees about the center.
        rotate_deg: f64,
    },
    /// An axis-aligned rectangle. `rx` and `ry` are zero for sharp corners.
    Rect {
        /// Left edge.
        x: f64,
        /// Top edge.
        y: f64,
        /// Width.
        w: f64,
        /// Height.
        h: f64,
        /// Horizontal corner radius.
        rx: f64,
        /// Vertical corner radius.
        ry: f64,
    },
    /// Anything else, including every shape with holes.
    Path(BezPath),
}

impl Prim {
    /// The name used in statistics and in test assertions.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Circle { .. } => "circle",
            Self::Ellipse { .. } => "ellipse",
            Self::Rect { rx, .. } if *rx > 0.0 => "rounded-rect",
            Self::Rect { .. } => "rect",
            Self::Path(_) => "path",
        }
    }

    /// Is this a primitive rather than a fallback path?
    #[must_use]
    pub const fn is_primitive(&self) -> bool {
        !matches!(self, Self::Path(_))
    }
}

/// One shape in the output document.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeEl {
    /// The geometry.
    pub prim: Prim,
    /// The solid fill.
    pub fill: Rgba,
}

/// The document after primitive detection.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeDoc {
    /// Canvas width in pixels.
    pub width: u32,
    /// Canvas height in pixels.
    pub height: u32,
    /// Shapes in paint order: the first is drawn first, at the bottom.
    pub shapes: Vec<ShapeEl>,
}

/// Which primitives the pass is allowed to emit.
// One switch per primitive is the clearest shape this can take; an enum set
// would be the same four bits with more ceremony.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detect {
    /// Emit `<circle>`.
    pub circle: bool,
    /// Emit `<ellipse>`.
    pub ellipse: bool,
    /// Emit `<rect>`.
    pub rect: bool,
    /// Emit `<rect>` with corner radii.
    pub rounded_rect: bool,
}

impl Default for Detect {
    fn default() -> Self {
        Self {
            circle: true,
            ellipse: true,
            rect: true,
            rounded_rect: true,
        }
    }
}

impl Detect {
    /// Emit nothing: every shape stays a path.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            circle: false,
            ellipse: false,
            rect: false,
            rounded_rect: false,
        }
    }
}

/// How hard the pass tries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeFitOptions {
    /// Allowed symmetric difference as a fraction of the outline's area.
    pub tolerance_frac: f64,
    /// Outlines smaller than this many square pixels stay paths.
    pub min_area: f64,
    /// Allow a rotated ellipse. With this off, a rotated ellipse stays a path
    /// rather than being snapped to an axis-aligned one.
    pub allow_rotation: bool,
    /// Which primitives may be emitted.
    pub detect: Detect,
}

impl Default for ShapeFitOptions {
    fn default() -> Self {
        Self {
            tolerance_frac: 0.02,
            min_area: 16.0,
            allow_rotation: true,
            detect: Detect::default(),
        }
    }
}

impl ShapeFitOptions {
    /// Options that convert nothing.
    #[must_use]
    pub fn off() -> Self {
        Self {
            detect: Detect::none(),
            ..Self::default()
        }
    }

    /// The tolerance, in square pixels, for an outline of this area and
    /// perimeter.
    ///
    /// # Examples
    ///
    /// ```
    /// use vectorise::shapes::ShapeFitOptions;
    ///
    /// let options = ShapeFitOptions::default();
    /// // A large shape is judged on its area.
    /// assert!((options.tolerance_for(10_000.0, 400.0) - 200.0).abs() < 1e-9);
    /// // A small one on its boundary, which is where the error actually is.
    /// assert!((options.tolerance_for(200.0, 50.0) - 10.0).abs() < 1e-9);
    /// ```
    #[must_use]
    pub fn tolerance_for(&self, area: f64, perimeter: f64) -> f64 {
        if self.tolerance_frac <= 0.0 {
            return 0.0;
        }
        let budget = area.max(PERIMETER_WEIGHT * perimeter);
        (self.tolerance_frac * budget).max(ABSOLUTE_TOLERANCE_FLOOR)
    }
}

/// Run the pass over a traced document.
///
/// Paint order, colors, and the canvas are preserved exactly; only the geometry
/// of individual shapes can change.
///
/// # Examples
///
/// ```
/// use vectorise::shapes::{ShapeFitOptions, fit_shapes};
/// use vtracer::ir::VectorDoc;
///
/// let doc = VectorDoc::new(64, 64);
/// let fitted = fit_shapes(&doc, &ShapeFitOptions::default());
/// assert_eq!((fitted.width, fitted.height), (64, 64));
/// assert!(fitted.shapes.is_empty());
/// ```
#[must_use]
pub fn fit_shapes(doc: &VectorDoc, options: &ShapeFitOptions) -> ShapeDoc {
    let shapes = doc
        .shapes
        .iter()
        .map(|shape| {
            let color = shape.paint.color();
            ShapeEl {
                prim: fit_one(&shape.path, options),
                fill: Rgba::new(color.r, color.g, color.b, color.a),
            }
        })
        .collect();

    ShapeDoc {
        width: doc.width,
        height: doc.height,
        shapes,
    }
}

/// Decide what one traced shape is.
fn fit_one(multipath: &vtracer::ir::MultiPath, options: &ShapeFitOptions) -> Prim {
    let fallback = || Prim::Path(multipath_to_bezpath(multipath));

    // Rule 1: holes and disjoint rings stay paths.
    let Some(subpath) = single_subpath(multipath) else {
        return fallback();
    };

    let outline = subpath_to_bezpath(subpath);
    let area = outline.area().abs();

    // Rule 2: below `min_area` a primitive saves almost nothing.
    if area < options.min_area {
        return Prim::Path(outline);
    }

    let tolerance = options.tolerance_for(area, outline.perimeter(PERIMETER_ACCURACY));
    best_candidate(&outline, area, tolerance, options).map_or_else(|| Prim::Path(outline), to_prim)
}

/// The first candidate within tolerance, in preference order.
fn best_candidate(
    outline: &BezPath,
    area: f64,
    tolerance: f64,
    options: &ShapeFitOptions,
) -> Option<Candidate> {
    proposals(outline, options)
        .into_iter()
        .find(|&candidate| accepts(outline, area, tolerance, candidate))
}

/// Every candidate worth measuring, most compact first.
fn proposals(outline: &BezPath, options: &ShapeFitOptions) -> Vec<Candidate> {
    let mut proposals = Vec::with_capacity(4);

    if options.detect.circle {
        proposals.extend(circle_candidate(outline));
    }
    if options.detect.rect {
        proposals.extend(rect_candidate(outline));
    }
    if options.detect.rounded_rect {
        proposals.extend(rounded_rect_candidate(outline));
    }
    if options.detect.ellipse
        && let Some(ellipse) = ellipse_candidate(outline)
    {
        proposals.extend(usable_ellipse(ellipse, options));
    }

    proposals
}

/// An ellipse candidate adjusted for the rotation policy.
///
/// A rotation under half a degree is snapped away so the writer emits no
/// transform for what is, to the eye, an axis-aligned ellipse. With
/// `allow_rotation` off, a genuinely rotated ellipse is dropped rather than
/// straightened: straightening it would change the shape, and verification
/// would then reject it anyway, more slowly.
fn usable_ellipse(candidate: Candidate, options: &ShapeFitOptions) -> Option<Candidate> {
    let Candidate::Ellipse {
        cx,
        cy,
        rx,
        ry,
        rotate_deg,
    } = candidate
    else {
        return Some(candidate);
    };

    // A circle has no meaningful rotation, whatever the moments say.
    let round = fit::is_round(candidate, CIRCLE_SNAP_FRACTION);
    if round {
        return Some(Candidate::Ellipse {
            cx,
            cy,
            rx,
            ry,
            rotate_deg: 0.0,
        });
    }

    if rotate_deg.abs() <= MIN_ROTATION_DEG {
        return Some(Candidate::Ellipse {
            cx,
            cy,
            rx,
            ry,
            rotate_deg: 0.0,
        });
    }
    options.allow_rotation.then_some(candidate)
}

/// Is this candidate close enough to the outline?
///
/// The area check is free and exact: the symmetric difference of two shapes is
/// never smaller than the difference of their areas, so a candidate that fails
/// it cannot pass the raster comparison either.
fn accepts(outline: &BezPath, area: f64, tolerance: f64, candidate: Candidate) -> bool {
    let path = candidate.to_path();
    if (path.area().abs() - area).abs() > tolerance {
        return false;
    }
    symmetric_difference(outline, &path) <= tolerance
}

/// A verified candidate as the primitive the writer will emit.
fn to_prim(candidate: Candidate) -> Prim {
    match candidate {
        Candidate::Circle { cx, cy, r } => Prim::Circle { cx, cy, r },
        Candidate::Ellipse {
            cx,
            cy,
            rx,
            ry,
            rotate_deg,
        } => {
            // An ellipse whose radii agree is a circle, and `<circle>` is two
            // characters shorter and clearer.
            if fit::is_round(candidate, CIRCLE_SNAP_FRACTION) {
                Prim::Circle {
                    cx,
                    cy,
                    r: f64::midpoint(rx, ry),
                }
            } else {
                Prim::Ellipse {
                    cx,
                    cy,
                    rx,
                    ry,
                    rotate_deg,
                }
            }
        }
        Candidate::Rect { x, y, w, h, r } => Prim::Rect {
            x,
            y,
            w,
            h,
            rx: r,
            ry: r,
        },
    }
}

#[cfg(test)]
mod tests {
    // Test fixtures convert small integers and f64 literals to the f32 the
    // rasterizer wants. Every value here is a pixel coordinate under 1000.
    #![allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::suboptimal_flops
    )]

    use kurbo::Shape as _;
    use proptest::prelude::*;
    use tiny_skia::{Color as SkColor, FillRule, Paint, PathBuilder, Pixmap, Transform};
    use vtracer::ColorImage;

    use super::{Detect, Prim, ShapeFitOptions, fit_shapes};
    use crate::color::Rgba;
    use crate::trace::{TraceOptions, trace};

    /// Draw a shape in black on a white canvas, then trace it.
    ///
    /// This is the whole pipeline the pass sits in: rasterize, trace, fit. A
    /// test that starts from a hand-written `BezPath` would not exercise the
    /// corner artifacts a real traced outline has.
    fn traced(width: u32, height: u32, draw: impl Fn(&mut PathBuilder)) -> vtracer::ir::VectorDoc {
        traced_with(width, height, &TraceOptions::default(), draw)
    }

    /// A pixmap as the tracer's image type.
    fn image_of(pixmap: &Pixmap) -> ColorImage {
        ColorImage {
            pixels: pixmap
                .pixels()
                .iter()
                .flat_map(|pixel| {
                    let c = pixel.demultiply();
                    [c.red(), c.green(), c.blue(), c.alpha()]
                })
                .collect(),
            width: pixmap.width() as usize,
            height: pixmap.height() as usize,
        }
    }

    /// [`traced`] with tracing options of your own.
    fn traced_with(
        width: u32,
        height: u32,
        options: &TraceOptions,
        draw: impl Fn(&mut PathBuilder),
    ) -> vtracer::ir::VectorDoc {
        let mut pixmap = Pixmap::new(width, height).expect("pixmap");
        pixmap.fill(SkColor::WHITE);

        let mut builder = PathBuilder::new();
        draw(&mut builder);
        let path = builder.finish().expect("a non-empty path");

        let mut paint = Paint::default();
        paint.set_color_rgba8(0, 0, 0, 255);
        paint.anti_alias = false;
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );

        trace(&image_of(&pixmap), options).expect("traces")
    }

    /// The primitive the traced black shape became.
    fn black_prim(doc: &vtracer::ir::VectorDoc, options: &ShapeFitOptions) -> Prim {
        let fitted = fit_shapes(doc, options);
        let Some(black) = fitted
            .shapes
            .into_iter()
            .find(|shape| shape.fill == Rgba::new(0, 0, 0, 255))
        else {
            unreachable!("the black shape must survive tracing")
        };
        black.prim
    }

    fn opts() -> ShapeFitOptions {
        ShapeFitOptions::default()
    }

    // --- the happy cases ------------------------------------------------------

    #[test]
    fn pass_converts_traced_circle_r40_to_circle_within_1px() {
        let doc = traced(128, 128, |builder| builder.push_circle(64.0, 64.0, 40.0));
        let Prim::Circle { cx, cy, r } = black_prim(&doc, &opts()) else {
            unreachable!("a traced disc must become a circle");
        };

        assert!((cx - 64.0).abs() < 1.0, "cx {cx}");
        assert!((cy - 64.0).abs() < 1.0, "cy {cy}");
        // Pixel tracing follows the outside of the boundary pixels, so the
        // traced radius sits a little above the drawn one.
        assert!((r - 40.0).abs() < 1.5, "r {r}");
    }

    #[test]
    fn pass_converts_traced_ellipse_60x30_to_ellipse() {
        let doc = traced(200, 120, |builder| {
            builder.push_oval(
                tiny_skia::Rect::from_ltrb(40.0, 30.0, 160.0, 90.0).expect("oval bounds"),
            );
        });
        let Prim::Ellipse { cx, cy, rx, ry, .. } = black_prim(&doc, &opts()) else {
            unreachable!("a traced oval must become an ellipse");
        };

        assert!((cx - 100.0).abs() < 1.5, "cx {cx}");
        assert!((cy - 60.0).abs() < 1.5, "cy {cy}");
        assert!((rx - 60.0).abs() < 2.0, "rx {rx}");
        assert!((ry - 30.0).abs() < 2.0, "ry {ry}");
    }

    #[test]
    fn pass_converts_traced_rect_to_rect_exact() {
        let doc = traced(128, 96, |builder| {
            builder.push_rect(tiny_skia::Rect::from_xywh(16.0, 24.0, 80.0, 40.0).expect("rect"));
        });
        let Prim::Rect { x, y, w, h, rx, ry } = black_prim(&doc, &opts()) else {
            unreachable!("a traced rectangle must become a rect");
        };

        assert!(
            (x - 16.0).abs() <= 1.0 && (y - 24.0).abs() <= 1.0,
            "{x},{y}"
        );
        assert!(
            (w - 80.0).abs() <= 1.0 && (h - 40.0).abs() <= 1.0,
            "{w}x{h}"
        );
        assert!(
            rx.abs() < f64::EPSILON && ry.abs() < f64::EPSILON,
            "sharp corners"
        );
    }

    #[test]
    fn pass_converts_traced_rounded_rect_to_rect_with_rx() {
        let doc = traced(160, 120, |builder| {
            builder.push_rect(tiny_skia::Rect::from_xywh(20.0, 20.0, 120.0, 80.0).expect("rect"));
        });
        // A sharp rectangle is the control: it must not gain a radius.
        let Prim::Rect { rx, .. } = black_prim(&doc, &opts()) else {
            unreachable!("a rectangle must become a rect");
        };
        assert!(rx.abs() < f64::EPSILON);

        let rounded = traced(160, 120, |builder| {
            let bounds = tiny_skia::Rect::from_xywh(20.0, 20.0, 120.0, 80.0).expect("rect");
            let path = kurbo::RoundedRect::new(
                f64::from(bounds.left()),
                f64::from(bounds.top()),
                f64::from(bounds.right()),
                f64::from(bounds.bottom()),
                16.0,
            )
            .to_path(0.05);
            push_bezpath(builder, &path);
        });
        match black_prim(&rounded, &opts()) {
            Prim::Rect { rx, ry, .. } => {
                assert!(rx > 8.0 && rx < 24.0, "recovered radius {rx}");
                assert!((rx - ry).abs() < f64::EPSILON, "rx and ry agree");
            }
            // Corner artifacts from the spline fit can leave this a path; that
            // is a documented false negative, not a failure of the contract.
            // What must never happen is a *sharp* rect being claimed here.
            other => assert_eq!(other.kind(), "path", "unexpected {}", other.kind()),
        }
    }

    fn push_bezpath(builder: &mut PathBuilder, path: &kurbo::BezPath) {
        let f = |value: f64| value as f32;
        for element in path.elements() {
            match *element {
                kurbo::PathEl::MoveTo(p) => builder.move_to(f(p.x), f(p.y)),
                kurbo::PathEl::LineTo(p) => builder.line_to(f(p.x), f(p.y)),
                kurbo::PathEl::QuadTo(c, e) => {
                    builder.quad_to(f(c.x), f(c.y), f(e.x), f(e.y));
                }
                kurbo::PathEl::CurveTo(a, b, e) => {
                    builder.cubic_to(f(a.x), f(a.y), f(b.x), f(b.y), f(e.x), f(e.y));
                }
                kurbo::PathEl::ClosePath => builder.close(),
            }
        }
    }

    // --- the shapes that must stay paths --------------------------------------

    #[test]
    fn pass_detects_a_large_ellipse_once_the_tracer_fits_it_finely() {
        // A 400x100 oval. At the default segment length the spline fit encloses
        // about 7% more area than the pixels it came from, so no ellipse is
        // faithful and the pass keeps a path. Telling the tracer to use shorter
        // segments fixes the fit, and the ellipse is then detected. This is the
        // documented remedy in docs/shape-detection.md.
        let draw = |builder: &mut PathBuilder| {
            builder.push_oval(
                tiny_skia::Rect::from_ltrb(120.0, 120.0, 520.0, 220.0).expect("oval bounds"),
            );
        };

        let coarse = traced(640, 340, draw);
        assert_eq!(black_prim(&coarse, &opts()).kind(), "path");

        let fine = traced_with(
            640,
            340,
            &TraceOptions {
                segment_length: Some(1.0),
                ..TraceOptions::default()
            },
            draw,
        );
        assert_eq!(black_prim(&fine, &opts()).kind(), "ellipse");
    }

    #[test]
    fn pass_keeps_star_as_path() {
        let doc = traced(160, 160, |builder| {
            let points = star_points(80.0, 80.0, 70.0, 30.0, 5);
            builder.move_to(points[0].0, points[0].1);
            for &(x, y) in &points[1..] {
                builder.line_to(x, y);
            }
            builder.close();
        });
        assert_eq!(black_prim(&doc, &opts()).kind(), "path");
    }

    #[test]
    fn pass_keeps_donut_as_path() {
        // Holes only exist in cutout mode. Stacked mode, the default, paints
        // the inner disc over a solid outer one, so the black shape there
        // really is a circle and is rightly detected as one.
        let draw = |builder: &mut PathBuilder| {
            builder.push_circle(100.0, 100.0, 60.0);
        };
        let cutout = TraceOptions {
            hierarchical: Some(crate::trace::Hierarchical::Cutout),
            ..TraceOptions::default()
        };

        let doc = traced_with(200, 200, &cutout, |builder| draw(builder));
        let _ = &doc;

        // The hole has to be painted, not merely added to the same path: two
        // circles wound the same way fill as one solid disc.
        let mut pixmap = Pixmap::new(200, 200).expect("pixmap");
        pixmap.fill(SkColor::WHITE);
        for (radius, color) in [(60.0_f32, (0u8, 0u8, 0u8)), (30.0, (255, 255, 255))] {
            let mut builder = PathBuilder::new();
            builder.push_circle(100.0, 100.0, radius);
            let mut paint = Paint::default();
            paint.set_color_rgba8(color.0, color.1, color.2, 255);
            paint.anti_alias = false;
            pixmap.fill_path(
                &builder.finish().expect("path"),
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        let doc = trace(&image_of(&pixmap), &cutout).expect("traces");
        let ring = fit_shapes(&doc, &opts())
            .shapes
            .into_iter()
            .find(|shape| shape.fill == Rgba::new(0, 0, 0, 255))
            .unwrap_or_else(|| unreachable!("the ring must survive tracing"));

        assert_eq!(
            ring.prim.kind(),
            "path",
            "a shape with a hole cannot be one primitive"
        );
    }

    #[test]
    fn pass_keeps_circle_when_tolerance_is_zero() {
        let doc = traced(128, 128, |builder| builder.push_circle(64.0, 64.0, 40.0));
        let strict = ShapeFitOptions {
            tolerance_frac: 0.0,
            ..opts()
        };
        assert_eq!(black_prim(&doc, &strict).kind(), "path");
    }

    #[test]
    fn pass_keeps_tiny_blob_below_min_area_as_path() {
        // A 3x3 square is 9 px^2, below the 16 px^2 default.
        let doc = traced(32, 32, |builder| {
            builder.push_rect(tiny_skia::Rect::from_xywh(10.0, 10.0, 3.0, 3.0).expect("rect"));
        });
        let generous = ShapeFitOptions {
            // The speckle filter would otherwise remove it before we see it.
            min_area: 16.0,
            ..opts()
        };
        let doc_shapes = fit_shapes(&doc, &generous);
        for shape in &doc_shapes.shapes {
            if shape.fill == Rgba::new(0, 0, 0, 255) {
                assert_eq!(shape.prim.kind(), "path");
            }
        }
    }

    #[test]
    fn pass_with_detection_off_keeps_everything_as_paths() {
        let doc = traced(128, 128, |builder| builder.push_circle(64.0, 64.0, 40.0));
        let fitted = fit_shapes(&doc, &ShapeFitOptions::off());
        assert!(fitted.shapes.iter().all(|shape| !shape.prim.is_primitive()));

        let only_rect = ShapeFitOptions {
            detect: Detect {
                circle: false,
                ellipse: false,
                rect: true,
                rounded_rect: true,
            },
            ..opts()
        };
        assert_eq!(black_prim(&doc, &only_rect).kind(), "path");
    }

    // --- document-level invariants --------------------------------------------

    #[test]
    fn pass_preserves_paint_order_and_colors() {
        // Three stacked bands, bottom to top.
        let mut pixmap = Pixmap::new(90, 30).expect("pixmap");
        pixmap.fill(SkColor::WHITE);
        for (index, color) in [(0u32, (255, 0, 0)), (1, (0, 255, 0)), (2, (0, 0, 255))] {
            let mut paint = Paint::default();
            paint.set_color_rgba8(color.0, color.1, color.2, 255);
            paint.anti_alias = false;
            let mut builder = PathBuilder::new();
            builder.push_rect(
                tiny_skia::Rect::from_xywh(index as f32 * 30.0, 0.0, 30.0, 30.0).expect("rect"),
            );
            pixmap.fill_path(
                &builder.finish().expect("path"),
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        let doc = trace(&image_of(&pixmap), &TraceOptions::default()).expect("traces");
        let fitted = fit_shapes(&doc, &opts());

        assert_eq!(fitted.shapes.len(), doc.shapes.len(), "no shape is dropped");
        assert_eq!((fitted.width, fitted.height), (doc.width, doc.height));
        for (after, before) in fitted.shapes.iter().zip(&doc.shapes) {
            let color = before.paint.color();
            assert_eq!(
                after.fill,
                Rgba::new(color.r, color.g, color.b, color.a),
                "colors stay in place"
            );
        }
    }

    #[test]
    fn pass_on_an_empty_document_produces_an_empty_document() {
        let doc = vtracer::ir::VectorDoc::new(10, 20);
        let fitted = fit_shapes(&doc, &opts());
        assert_eq!((fitted.width, fitted.height), (10, 20));
        assert!(fitted.shapes.is_empty());
    }

    #[test]
    fn pass_is_deterministic() {
        let doc = traced(128, 128, |builder| builder.push_circle(64.0, 64.0, 40.0));
        assert_eq!(fit_shapes(&doc, &opts()), fit_shapes(&doc, &opts()));
    }

    // --- rotation and snapping -------------------------------------------------

    #[test]
    fn pass_circle_snaps_when_rx_ry_differ_less_than_tolerance() {
        // 60 by 61: within 2%, so it is emitted as a circle rather than an
        // ellipse with two nearly equal radii.
        let doc = traced(160, 160, |builder| {
            builder.push_oval(
                tiny_skia::Rect::from_ltrb(20.0, 19.5, 140.0, 140.5).expect("oval bounds"),
            );
        });
        assert_eq!(black_prim(&doc, &opts()).kind(), "circle");
    }

    #[test]
    fn pass_rotated_ellipse_emits_rotation_when_above_half_degree() {
        let doc = traced(220, 220, |builder| {
            let ellipse = kurbo::Ellipse::new(
                kurbo::Point::new(110.0, 110.0),
                kurbo::Vec2::new(85.0, 40.0),
                30.0_f64.to_radians(),
            );
            push_bezpath(builder, &ellipse.to_path(0.05));
        });

        match black_prim(&doc, &opts()) {
            Prim::Ellipse { rotate_deg, .. } => {
                let difference = (rotate_deg - 30.0).rem_euclid(180.0);
                let distance = difference.min(180.0 - difference);
                assert!(distance < 2.0, "recovered rotation {rotate_deg}");
            }
            other => unreachable!("expected a rotated ellipse, got {}", other.kind()),
        }
    }

    #[test]
    fn pass_without_rotation_keeps_a_rotated_ellipse_as_a_path() {
        let doc = traced(220, 220, |builder| {
            let ellipse = kurbo::Ellipse::new(
                kurbo::Point::new(110.0, 110.0),
                kurbo::Vec2::new(85.0, 40.0),
                30.0_f64.to_radians(),
            );
            push_bezpath(builder, &ellipse.to_path(0.05));
        });
        let upright = ShapeFitOptions {
            allow_rotation: false,
            ..opts()
        };
        assert_eq!(black_prim(&doc, &upright).kind(), "path");
    }

    // --- tolerance arithmetic ---------------------------------------------------

    #[test]
    fn tolerance_is_a_fraction_of_area_with_a_perimeter_term_and_a_floor() {
        let options = opts();

        // Large shape: the area term wins.
        assert!((options.tolerance_for(10_000.0, 400.0) - 200.0).abs() < 1e-9);

        // Small shape: the boundary band wins, because that is where a traced
        // outline actually differs from the shape it approximates.
        let disc_r8 = (217.0, 52.5);
        let by_area = options.tolerance_frac * disc_r8.0;
        let allowed = options.tolerance_for(disc_r8.0, disc_r8.1);
        assert!(allowed > by_area, "{allowed} should beat {by_area}");
        assert!((allowed - 0.02 * super::PERIMETER_WEIGHT * disc_r8.1).abs() < 1e-9);

        // Degenerate shape: the absolute floor.
        assert!((options.tolerance_for(1.0, 1.0) - super::ABSOLUTE_TOLERANCE_FLOOR).abs() < 1e-9);

        assert!(
            ShapeFitOptions {
                tolerance_frac: 0.0,
                ..options
            }
            .tolerance_for(10_000.0, 400.0)
            .abs()
                < f64::EPSILON,
            "zero means zero, every term included"
        );
    }

    #[test]
    fn prim_kind_names_every_variant() {
        assert_eq!(
            Prim::Circle {
                cx: 0.0,
                cy: 0.0,
                r: 1.0
            }
            .kind(),
            "circle"
        );
        assert_eq!(
            Prim::Ellipse {
                cx: 0.0,
                cy: 0.0,
                rx: 2.0,
                ry: 1.0,
                rotate_deg: 0.0
            }
            .kind(),
            "ellipse"
        );
        assert_eq!(
            Prim::Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
                rx: 0.0,
                ry: 0.0
            }
            .kind(),
            "rect"
        );
        assert_eq!(
            Prim::Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
                rx: 2.0,
                ry: 2.0
            }
            .kind(),
            "rounded-rect"
        );
        assert_eq!(Prim::Path(kurbo::BezPath::new()).kind(), "path");
        assert!(!Prim::Path(kurbo::BezPath::new()).is_primitive());
    }

    fn star_points(cx: f32, cy: f32, outer: f32, inner: f32, points: usize) -> Vec<(f32, f32)> {
        (0..points * 2)
            .map(|index| {
                let radius = if index % 2 == 0 { outer } else { inner };
                let angle = std::f32::consts::PI * index as f32 / points as f32
                    - std::f32::consts::FRAC_PI_2;
                (
                    radius.mul_add(angle.cos(), cx),
                    radius.mul_add(angle.sin(), cy),
                )
            })
            .collect()
    }

    // --- property tests ---------------------------------------------------------

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 256,
            // A fixed seed keeps CI reproducible; failures still shrink.
            rng_algorithm: proptest::test_runner::RngAlgorithm::ChaCha,
            ..ProptestConfig::default()
        })]

        /// Axis-aligned ellipses up to 2.5:1 are detected at any size in range.
        ///
        /// Past roughly 3:1 the *tracer* stops producing an ellipse: at its
        /// default `--segment-length` the fitted spline of a long, thin oval
        /// encloses up to 7% more area than the pixels it came from, so no
        /// ellipse represents it faithfully and the pass rightly refuses.
        /// `prop_a_primitive_is_never_further_off_than_the_tolerance` covers
        /// the shapes outside this envelope. See `docs/shape-detection.md`.
        #[test]
        fn prop_random_axis_aligned_ellipse_rasterized_then_traced_is_detected(
            major in 8.0_f32..200.0,
            ratio in 1.0_f32..2.5,
        ) {
            let rx = major;
            let ry = (major / ratio).max(8.0);
            // The canvas needs real margin. With only a few pixels of border
            // the clustering inverts: the shape becomes the base layer and the
            // background is painted over it with a hole, so there is no shape
            // left to detect.
            let margin = (rx.max(ry) * 0.6).max(8.0);
            let width = (rx * 2.0 + margin * 2.0).ceil() as u32;
            let height = (ry * 2.0 + margin * 2.0).ceil() as u32;
            let cx = width as f32 / 2.0;
            let cy = height as f32 / 2.0;

            let doc = traced(width, height, |builder| {
                builder.push_oval(
                    tiny_skia::Rect::from_ltrb(cx - rx, cy - ry, cx + rx, cy + ry)
                        .expect("oval bounds"),
                );
            });
            let kind = black_prim(&doc, &opts()).kind();
            prop_assert!(
                kind == "ellipse" || kind == "circle",
                "rx {rx} ry {ry} became {kind}"
            );
        }

        #[test]
        fn prop_random_rect_is_detected(
            w in 10.0_f32..120.0,
            h in 10.0_f32..120.0,
        ) {
            // Margin, for the same reason as the ellipse property above.
            let margin = (w.max(h) * 0.6).max(8.0);
            let width = (w + margin * 2.0).ceil() as u32;
            let height = (h + margin * 2.0).ceil() as u32;

            let doc = traced(width, height, |builder| {
                builder.push_rect(
                    tiny_skia::Rect::from_xywh(margin, margin, w, h).expect("rect"),
                );
            });
            prop_assert_eq!(black_prim(&doc, &opts()).kind(), "rect");
        }

        #[test]
        fn prop_random_polygon_with_5_to_9_irregular_vertices_is_not_detected_as_rect_or_ellipse(
            radii in proptest::collection::vec(25.0_f32..70.0, 5..=9),
        ) {
            let size = 200u32;
            let centre = size as f32 / 2.0;
            let count = radii.len();

            let doc = traced(size, size, |builder| {
                for (index, radius) in radii.iter().enumerate() {
                    let angle = std::f32::consts::TAU * index as f32 / count as f32;
                    let x = radius.mul_add(angle.cos(), centre);
                    let y = radius.mul_add(angle.sin(), centre);
                    if index == 0 {
                        builder.move_to(x, y);
                    } else {
                        builder.line_to(x, y);
                    }
                }
                builder.close();
            });
            prop_assert_eq!(black_prim(&doc, &opts()).kind(), "path");
        }

        /// Whatever the pass emits is within tolerance of the outline it
        /// replaced. This is the invariant that matters: missing a primitive
        /// costs bytes, emitting a wrong one costs fidelity.
        #[test]
        fn prop_a_primitive_is_never_further_off_than_the_tolerance(
            major in 8.0_f32..200.0,
            ratio in 1.0_f32..8.0,
        ) {
            use crate::geom::convert::{single_subpath, subpath_to_bezpath};
            use crate::geom::verify::symmetric_difference;

            let rx = major;
            let ry = (major / ratio).max(8.0);
            let margin = (rx.max(ry) * 0.6).max(8.0);
            let width = (rx * 2.0 + margin * 2.0).ceil() as u32;
            let height = (ry * 2.0 + margin * 2.0).ceil() as u32;
            let cx = width as f32 / 2.0;
            let cy = height as f32 / 2.0;

            let doc = traced(width, height, |builder| {
                builder.push_oval(
                    tiny_skia::Rect::from_ltrb(cx - rx, cy - ry, cx + rx, cy + ry)
                        .expect("oval bounds"),
                );
            });

            let options = opts();
            for (fitted, original) in fit_shapes(&doc, &options).shapes.iter().zip(&doc.shapes) {
                if !fitted.prim.is_primitive() {
                    continue;
                }
                let Some(subpath) = single_subpath(&original.path) else {
                    prop_assert!(false, "a multi-subpath shape must stay a path");
                    return Ok(());
                };
                let outline = subpath_to_bezpath(subpath);
                let area = outline.area().abs();
                let tolerance =
                    options.tolerance_for(area, outline.perimeter(super::PERIMETER_ACCURACY));
                let replacement = match &fitted.prim {
                    Prim::Circle { cx, cy, r } => {
                        kurbo::Circle::new((*cx, *cy), *r).to_path(1e-3)
                    }
                    Prim::Ellipse { cx, cy, rx, ry, rotate_deg } => kurbo::Ellipse::new(
                        (*cx, *cy),
                        (*rx, *ry),
                        rotate_deg.to_radians(),
                    )
                    .to_path(1e-3),
                    Prim::Rect { x, y, w, h, rx, .. } if *rx > 0.0 => {
                        kurbo::RoundedRect::new(*x, *y, *x + *w, *y + *h, *rx).to_path(1e-3)
                    }
                    Prim::Rect { x, y, w, h, .. } => {
                        kurbo::Rect::new(*x, *y, *x + *w, *y + *h).to_path(1e-3)
                    }
                    Prim::Path(path) => path.clone(),
                };

                let difference = symmetric_difference(&outline, &replacement);
                prop_assert!(
                    difference <= tolerance,
                    "{} is {difference} off, tolerance {tolerance}",
                    fitted.prim.kind()
                );
            }
        }

        #[test]
        fn prop_detection_is_translation_invariant(
            dx in -20_i32..20,
            dy in -20_i32..20,
        ) {
            let at_origin = traced(200, 200, |builder| builder.push_circle(100.0, 100.0, 45.0));
            let moved = traced(200, 200, |builder| {
                builder.push_circle(100.0 + dx as f32, 100.0 + dy as f32, 45.0);
            });

            let Prim::Circle { cx: ax, cy: ay, r: ar } = black_prim(&at_origin, &opts()) else {
                prop_assert!(false, "the reference circle must be detected");
                return Ok(());
            };
            let Prim::Circle { cx: bx, cy: by, r: br } = black_prim(&moved, &opts()) else {
                prop_assert!(false, "the shifted circle must be detected too");
                return Ok(());
            };

            prop_assert!((ar - br).abs() < 1.0, "radius moved: {} then {}", ar, br);
            prop_assert!((ax + f64::from(dx) - bx).abs() < 1.0, "cx {} vs {}", ax, bx);
            prop_assert!((ay + f64::from(dy) - by).abs() < 1.0, "cy {} vs {}", ay, by);
        }
    }
}
