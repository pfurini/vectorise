//! Last-mile byte reduction, without ever undoing shape detection.
//!
//! The writer already emits minimal markup, but it does not rewrite path data.
//! [`oxvg`](https://crates.io/crates/oxvg_optimiser), a Rust port of SVGO, does:
//! `convertPathData` alone is worth most of the saving on a traced document.
//!
//! # What is disabled, and why
//!
//! | Job | Reason |
//! |---|---|
//! | `convertShapeToPath` | It rewrites `<rect>` as `<path>`, which silently undoes Phase 5. Verified in `docs/api-notes.md` §9.2. |
//! | `mergePaths` | It would merge a detected primitive's neighbours across paint order. |
//! | `removeViewBox` | Our root has no `width`/`height` by default, so the `viewBox` is the only size information there is. |
//! | `sortAttrs` | Cosmetic, and it reorders the attributes the snapshots pin. |
//!
//! Everything else in SVGO's default set stays on. In particular
//! `removeUnknownsAndDefaults` is left enabled: it strips attributes that equal
//! the SVG default, such as a `fill="#000"` or a `rect`'s `x="0"`, which is a
//! byte saving with no rendering change.
//!
//! # The output is never larger
//!
//! An optimizer can, in principle, spend bytes to gain structure. Ours has one
//! job, so [`optimize`] returns whichever of the input and the output is
//! shorter. A pass that made things worse is simply not applied.

use oxvg_ast::parse::roxmltree::parse;
use oxvg_ast::serialize::Node as _;
use oxvg_ast::visitor::Info;
use oxvg_optimiser::{ConvertPathData, Jobs};

/// Why a document could not be optimized.
///
/// Every variant means the input is returned unchanged by the caller, so an
/// optimizer failure never loses a conversion.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OptimizeError {
    /// The SVG could not be parsed.
    #[error("cannot parse the SVG to optimize it: {0}")]
    Parse(String),
    /// A job failed.
    #[error("an optimizer job failed: {0}")]
    Job(String),
    /// The optimized document could not be serialized.
    #[error("cannot serialize the optimized SVG: {0}")]
    Serialize(String),
}

/// The job list, configured once so the reasons live next to the switches.
fn jobs() -> Jobs {
    Jobs {
        // Would rewrite `<rect>` as `<path>`, undoing the whole point of
        // Phase 5.
        convert_shape_to_path: None,
        // Would merge shapes across paint order.
        merge_paths: None,
        // The `viewBox` is the only size our root carries.
        remove_view_box: None,
        // Cosmetic, and it churns the snapshots.
        sort_attrs: None,
        convert_path_data: Some(lossless_path_data()),
        ..Jobs::default()
    }
}

/// `convertPathData` with its approximating sub-passes turned off.
///
/// The re-encoding sub-passes are exact: joining collinear nodes, dropping
/// empty and zero-length segments. Three others are not, and each has its own
/// tolerance stacked on top of the one Phase 5 already spent:
///
/// | Sub-pass | What it does | Why it is off |
/// |---|---|---|
/// | `arc_curves` | rewrites runs of cubics as elliptical arcs | shifts pixels on a traced star, measured |
/// | `smart_arc_rounding` | rounds arc radii to whole numbers | only matters with `arc_curves` |
/// | `straight_curves` | replaces a nearly straight cubic with a line | a second approximation of geometry we already verified |
///
/// The measured cost of turning them off is a few percent of the saving; the
/// benefit is that `optimize` is a pure re-encoding and the rendered result is
/// bit-identical.
fn lossless_path_data() -> ConvertPathData {
    ConvertPathData {
        arc_curves: false,
        smart_arc_rounding: false,
        straight_curves: false,
        ..ConvertPathData::default()
    }
}

/// Shrink an SVG document.
///
/// Returns the input unchanged when optimizing it would make it larger.
///
/// # Errors
///
/// [`OptimizeError::Parse`] if the document does not parse,
/// [`OptimizeError::Job`] if a job fails, and [`OptimizeError::Serialize`] if
/// the result cannot be written back out.
///
/// # Examples
///
/// ```
/// use vectorise::optimize::optimize;
///
/// let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><circle cx="5.000" cy="5.0" r="4" fill="#ff0000"/></svg>"##;
/// let smaller = optimize(svg).expect("valid SVG");
/// assert!(smaller.len() <= svg.len());
/// assert!(smaller.contains("<circle"), "a circle stays a circle");
/// ```
pub fn optimize(svg: &str) -> Result<String, OptimizeError> {
    let optimized = parse(svg, |dom, allocator| {
        // The arena dies with this closure, so the errors have to be flattened
        // to owned strings before they escape it.
        jobs()
            .run(dom, &Info::new(allocator))
            .map_err(|error| OptimizeError::Job(error.to_string()))?;
        dom.serialize()
            .map_err(|error| OptimizeError::Serialize(error.to_string()))
    })
    .map_err(|error| OptimizeError::Parse(error.to_string()))??;

    if optimized.len() < svg.len() {
        Ok(optimized)
    } else {
        Ok(svg.to_owned())
    }
}

#[cfg(test)]
mod tests {
    // The renderer works in f32; every size here is a two-digit pixel count.
    #![allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::expect_used
    )]

    use super::optimize;

    /// A document with everything the pass must respect: each primitive, a
    /// path, a `viewBox`, and numbers with room to shrink.
    fn sample() -> String {
        [
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 90">"#,
            r##"<rect x="0.00" y="0.0" width="120" height="90" fill="#ffffff"/>"##,
            r##"<circle cx="60.000" cy="45.00" r="20.50" fill="#ff0000"/>"##,
            r##"<ellipse cx="95" cy="30" rx="15.000" ry="7.50" fill="#008000"/>"##,
            r##"<rect x="4" y="60" width="40" height="24" rx="6.00" fill="#0000ff"/>"##,
            r##"<path d="M 5 5 L 25 5 C 30 10 30 20 25 25 L 5 25 Z" fill="#3366ff"/>"##,
            "</svg>",
        ]
        .concat()
    }

    fn render(svg: &str, scale: f32) -> tiny_skia::Pixmap {
        let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).expect("parses");
        let size = tree.size();
        let width = (size.width() * scale).ceil() as u32;
        let height = (size.height() * scale).ceil() as u32;

        let mut pixmap = tiny_skia::Pixmap::new(width.max(1), height.max(1)).expect("pixmap");
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        pixmap
    }

    #[test]
    fn optimize_never_converts_circle_to_path() {
        let optimized = optimize(&sample()).expect("optimizes");
        assert!(optimized.contains("<circle"), "{optimized}");
        assert!(optimized.contains("<ellipse"), "{optimized}");
        assert_eq!(
            optimized.matches("<rect").count(),
            2,
            "both rects survive: {optimized}"
        );
    }

    #[test]
    fn optimize_keeps_the_viewbox() {
        let optimized = optimize(&sample()).expect("optimizes");
        assert!(optimized.contains("viewBox"), "{optimized}");
    }

    #[test]
    fn optimize_output_is_not_larger_than_input() {
        let input = sample();
        let optimized = optimize(&input).expect("optimizes");
        assert!(
            optimized.len() <= input.len(),
            "{} then {}",
            input.len(),
            optimized.len()
        );
    }

    #[test]
    fn optimize_keeps_smaller_of_input_and_output() {
        // Already minimal: there is nothing left to take out, so the input
        // comes back unchanged rather than being replaced by something equal
        // or longer.
        let minimal = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><circle cx="5" cy="5" r="4" fill="red"/></svg>"#;
        let optimized = optimize(minimal).expect("optimizes");
        assert!(optimized.len() <= minimal.len());
    }

    #[test]
    fn optimize_output_renders_identically() {
        let input = sample();
        let optimized = optimize(&input).expect("optimizes");

        let before = render(&input, 2.0);
        let after = render(&optimized, 2.0);

        assert_eq!(
            (before.width(), before.height()),
            (after.width(), after.height())
        );
        assert_eq!(
            before.data(),
            after.data(),
            "optimizing must not change a single pixel"
        );
    }

    #[test]
    fn optimize_idempotent() {
        let once = optimize(&sample()).expect("optimizes");
        let twice = optimize(&once).expect("optimizes");
        assert_eq!(once, twice);
    }

    #[test]
    fn optimize_actually_shrinks_a_verbose_document() {
        let input = sample();
        let optimized = optimize(&input).expect("optimizes");
        assert!(
            optimized.len() < input.len(),
            "nothing was saved: {} bytes both ways",
            input.len()
        );
    }

    #[test]
    fn optimize_output_of_the_real_pipeline_renders_identically() {
        // The hand-written sample has no curve long enough for the optimizer's
        // arc conversion to fire. A traced star does, and rewriting cubics as
        // arcs is exactly the kind of change that could shift a pixel.
        use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

        let mut pixmap = Pixmap::new(300, 200).expect("pixmap");
        pixmap.fill(Color::WHITE);
        let mut paint = Paint {
            anti_alias: false,
            ..Paint::default()
        };

        let mut builder = PathBuilder::new();
        builder.push_circle(70.0, 100.0, 50.0);
        paint.set_color_rgba8(220, 30, 40, 255);
        pixmap.fill_path(
            &builder.finish().expect("circle"),
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );

        let mut builder = PathBuilder::new();
        let (cx, cy) = (200.0_f32, 130.0_f32);
        for index in 0..10_u8 {
            let radius = if index % 2 == 0 { 45.0_f32 } else { 18.0 };
            let angle = std::f32::consts::PI * f32::from(index) / 5.0 - std::f32::consts::FRAC_PI_2;
            let (x, y) = (
                radius.mul_add(angle.cos(), cx),
                radius.mul_add(angle.sin(), cy),
            );
            if index == 0 {
                builder.move_to(x, y);
            } else {
                builder.line_to(x, y);
            }
        }
        builder.close();
        paint.set_color_rgba8(240, 180, 20, 255);
        pixmap.fill_path(
            &builder.finish().expect("star"),
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );

        let image = vtracer::ColorImage {
            pixels: pixmap
                .pixels()
                .iter()
                .flat_map(|pixel| {
                    let c = pixel.demultiply();
                    [c.red(), c.green(), c.blue(), c.alpha()]
                })
                .collect(),
            width: 300,
            height: 200,
        };
        let traced =
            crate::trace::trace(&image, &crate::trace::TraceOptions::default()).expect("traces");
        let fitted = crate::shapes::fit_shapes(&traced, &crate::shapes::ShapeFitOptions::default());
        let written = crate::writer::write_svg(&fitted, &crate::writer::WriterOptions::default());
        let optimized = optimize(&written).expect("optimizes");

        assert!(
            optimized.len() < written.len(),
            "the pass should save bytes"
        );
        assert!(optimized.contains("<circle"), "{optimized}");

        let before = render(&written, 2.0);
        let after = render(&optimized, 2.0);
        assert_eq!(before.data(), after.data(), "optimizing changed a pixel");
    }

    #[test]
    fn optimize_rejects_a_document_that_is_not_svg() {
        let error = optimize("this is not markup at all <<<").expect_err("rejected");
        assert!(
            matches!(error, super::OptimizeError::Parse(_)),
            "got {error:?}"
        );
    }

    #[test]
    fn optimize_leaves_a_document_with_no_elements_alone() {
        let empty = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"></svg>"#;
        let optimized = optimize(empty).expect("optimizes");
        assert!(optimized.contains("<svg"), "{optimized}");
        assert!(optimized.len() <= empty.len());
    }
}
