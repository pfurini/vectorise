//! Render the SVG we produced and ask how close it is to the image we read.
//!
//! `--verify` answers one question: how much of the picture survived? It
//! renders the output with `resvg` at the input's pixel size and reports
//!
//! ```text
//! fidelity = 1 - mean absolute error over RGB
//! ```
//!
//! as a number in 0..=1, where 1 means every pixel matched exactly.
//!
//! # Why mean absolute error
//!
//! It is the metric nobody can misread. A structural metric like SSIM
//! correlates better with what a person notices, but it has parameters
//! (window, sigma, the stabilizing constants) that change the answer, and a
//! hand-rolled version of it would be a number nobody could compare with
//! anyone else's. See ADR-0008.
//!
//! # Transparency
//!
//! Both images are composited over the same opaque background before they are
//! compared. Without that, a fully transparent pixel in one and an opaque white
//! pixel in the other would differ by nothing in RGB and everything to the eye,
//! or the reverse, depending on what the encoder happened to leave in the color
//! channels.

use tiny_skia::{Pixmap, Transform};
use vtracer::ColorImage;

use crate::color::Rgb;

/// How close a rendered SVG came to the image it was made from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fidelity {
    /// `1 - mean_absolute_error`, in 0..=1. Higher is better.
    pub score: f64,
    /// Mean absolute difference per RGB channel, in 0..=1.
    pub mean_absolute_error: f64,
}

/// Why fidelity could not be measured.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    /// The SVG we produced could not be parsed back.
    #[error("cannot parse the SVG to verify it: {0}")]
    Parse(String),
    /// A pixmap of the required size could not be allocated.
    #[error("cannot render {width}x{height} to verify it")]
    Render {
        /// Requested width.
        width: u32,
        /// Requested height.
        height: u32,
    },
    /// The reference image has no pixels to compare against.
    #[error("the reference image is empty")]
    EmptyReference,
}

/// Compare an SVG against the image it was traced from.
///
/// # Errors
///
/// [`VerifyError::Parse`] if the SVG does not parse, [`VerifyError::Render`] if
/// it cannot be rasterized at the reference size, and
/// [`VerifyError::EmptyReference`] if the reference has no pixels.
pub fn fidelity(
    reference: &ColorImage,
    svg: &str,
    background: Rgb,
) -> Result<Fidelity, VerifyError> {
    if reference.width == 0 || reference.height == 0 {
        return Err(VerifyError::EmptyReference);
    }
    let width = u32::try_from(reference.width).map_err(|_| VerifyError::Render {
        width: u32::MAX,
        height: u32::MAX,
    })?;
    let height = u32::try_from(reference.height).map_err(|_| VerifyError::Render {
        width,
        height: u32::MAX,
    })?;

    let rendered = render(svg, width, height)?;
    let expected = flatten_reference(reference, background);
    let actual = flatten_pixmap(&rendered, background);

    Ok(compare(&expected, &actual))
}

/// The fidelity of two equally sized RGB buffers.
///
/// Exposed so the metric can be tested for the properties a metric should
/// have, independently of rendering.
///
/// # Examples
///
/// ```
/// use vectorise::verify::compare;
///
/// let black = [0_u8; 12];
/// let white = [255_u8; 12];
/// assert!((compare(&black, &black).score - 1.0).abs() < 1e-12);
/// assert!(compare(&black, &white).score.abs() < 1e-12);
/// ```
#[must_use]
pub fn compare(expected: &[u8], actual: &[u8]) -> Fidelity {
    let samples = expected.len().min(actual.len());
    if samples == 0 {
        return Fidelity {
            score: 1.0,
            mean_absolute_error: 0.0,
        };
    }

    let total: u64 = expected
        .iter()
        .zip(actual)
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    let samples = u64::try_from(samples).unwrap_or(u64::MAX);
    let mean_absolute_error = counts_as_f64(total) / (counts_as_f64(samples) * 255.0);

    Fidelity {
        score: 1.0 - mean_absolute_error,
        mean_absolute_error,
    }
}

/// Rasterize an SVG at an exact pixel size.
fn render(svg: &str, width: u32, height: u32) -> Result<Pixmap, VerifyError> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default())
        .map_err(|error| VerifyError::Parse(error.to_string()))?;

    let mut pixmap = Pixmap::new(width, height).ok_or(VerifyError::Render { width, height })?;
    let size = tree.size();
    // The document's own size comes from its `viewBox`; scale it onto the
    // reference's pixel grid so the comparison is pixel for pixel.
    let scale_x = f32_of(f64::from(width)) / size.width();
    let scale_y = f32_of(f64::from(height)) / size.height();
    resvg::render(
        &tree,
        Transform::from_scale(scale_x, scale_y),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

/// The reference image as opaque RGB, composited over `background`.
fn flatten_reference(reference: &ColorImage, background: Rgb) -> Vec<u8> {
    let mut out = Vec::with_capacity(reference.width * reference.height * 3);
    for chunk in reference.pixels.as_chunks::<4>().0 {
        let [r, g, b, a] = *chunk;
        out.extend_from_slice(&over([r, g, b], a, background));
    }
    out
}

/// A rendered pixmap as opaque RGB, composited over `background`.
fn flatten_pixmap(pixmap: &Pixmap, background: Rgb) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixmap.pixels().len() * 3);
    for pixel in pixmap.pixels() {
        let straight = pixel.demultiply();
        out.extend_from_slice(&over(
            [straight.red(), straight.green(), straight.blue()],
            straight.alpha(),
            background,
        ));
    }
    out
}

/// One pixel composited over an opaque background.
fn over(source: [u8; 3], alpha: u8, background: Rgb) -> [u8; 3] {
    if alpha == 255 {
        return source;
    }
    let alpha = u32::from(alpha);
    let blend = |source: u8, background: u8| {
        let value = (u32::from(source) * alpha + u32::from(background) * (255 - alpha) + 127) / 255;
        u8::try_from(value).unwrap_or(u8::MAX)
    };
    [
        blend(source[0], background.r),
        blend(source[1], background.g),
        blend(source[2], background.b),
    ]
}

/// The renderer works in `f32`; the narrowing has one documented home.
#[expect(
    clippy::cast_possible_truncation,
    reason = "resvg is f32 throughout; image sizes are far inside its range"
)]
const fn f32_of(value: f64) -> f32 {
    value as f32
}

/// A pixel or channel count as a float.
///
/// Exact: the sum is bounded by 255 times the number of samples, and an image
/// with 2^45 samples does not fit in memory.
#[expect(
    clippy::cast_precision_loss,
    reason = "sample counts never approach 2^53"
)]
const fn counts_as_f64(count: u64) -> f64 {
    count as f64
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::{Fidelity, VerifyError, compare, fidelity};
    use crate::color::Rgb;
    use vtracer::ColorImage;

    fn solid(width: usize, height: usize, color: [u8; 4]) -> ColorImage {
        ColorImage {
            pixels: color.repeat(width * height),
            width,
            height,
        }
    }

    fn svg_of(width: u32, height: u32, fill: &str) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}"><rect width="{width}" height="{height}" fill="{fill}"/></svg>"#
        )
    }

    #[test]
    fn verify_identical_render_scores_1_0() {
        let reference = solid(16, 12, [255, 0, 0, 255]);
        let svg = svg_of(16, 12, "#f00");

        let Fidelity {
            score,
            mean_absolute_error,
        } = fidelity(&reference, &svg, Rgb::WHITE).expect("verifies");

        assert!((score - 1.0).abs() < 1e-9, "score {score}");
        assert!(mean_absolute_error < 1e-9);
    }

    #[test]
    fn verify_inverted_render_scores_near_0() {
        let reference = solid(16, 12, [0, 0, 0, 255]);
        let svg = svg_of(16, 12, "#fff");

        let score = fidelity(&reference, &svg, Rgb::WHITE)
            .expect("verifies")
            .score;
        assert!(score.abs() < 1e-9, "black against white scores 0: {score}");
    }

    #[test]
    fn verify_a_half_wrong_image_scores_about_half() {
        // Mid grey against white: every channel is off by 128 of 255.
        let reference = solid(8, 8, [128, 128, 128, 255]);
        let svg = svg_of(8, 8, "#fff");

        let score = fidelity(&reference, &svg, Rgb::WHITE)
            .expect("verifies")
            .score;
        assert!((score - 128.0 / 255.0).abs() < 0.01, "{score}");
    }

    #[test]
    fn verify_metric_is_symmetric() {
        let a: Vec<u8> = (0..=255_u8).collect();
        let b: Vec<u8> = (0..=255_u8).rev().collect();

        let forward = compare(&a, &b);
        let backward = compare(&b, &a);
        assert!((forward.score - backward.score).abs() < f64::EPSILON);
        assert!((forward.mean_absolute_error - backward.mean_absolute_error).abs() < f64::EPSILON);
    }

    #[test]
    fn verify_metric_of_nothing_is_perfect() {
        let empty = compare(&[], &[]);
        assert!((empty.score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn verify_composites_transparency_over_the_background() {
        // The reference is fully transparent, which the tracer keys out, so
        // the SVG is empty. Both flatten to the background and match.
        let reference = solid(8, 8, [0, 0, 0, 0]);
        let empty_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"></svg>"#;

        let score = fidelity(&reference, empty_svg, Rgb::WHITE)
            .expect("verifies")
            .score;
        assert!((score - 1.0).abs() < 1e-9, "{score}");

        // Against a black background they still match each other.
        let score = fidelity(&reference, empty_svg, Rgb::BLACK)
            .expect("verifies")
            .score;
        assert!((score - 1.0).abs() < 1e-9, "{score}");
    }

    #[test]
    fn verify_scales_the_document_onto_the_reference_grid() {
        // The SVG's viewBox is half the reference's size: rendering must
        // stretch it rather than compare a corner.
        let reference = solid(32, 32, [0, 0, 255, 255]);
        let svg = svg_of(16, 16, "#00f");

        let score = fidelity(&reference, &svg, Rgb::WHITE)
            .expect("verifies")
            .score;
        assert!((score - 1.0).abs() < 1e-9, "{score}");
    }

    #[test]
    fn verify_rejects_an_empty_reference() {
        let empty = ColorImage {
            pixels: Vec::new(),
            width: 0,
            height: 0,
        };
        assert_eq!(
            fidelity(&empty, "<svg/>", Rgb::WHITE),
            Err(VerifyError::EmptyReference)
        );
    }

    #[test]
    fn verify_rejects_an_unparseable_document() {
        let reference = solid(4, 4, [0, 0, 0, 255]);
        let error = fidelity(&reference, "not markup <<<", Rgb::WHITE).expect_err("rejected");
        assert!(matches!(error, VerifyError::Parse(_)), "{error:?}");
    }

    #[test]
    fn verify_of_the_real_pipeline_is_high() {
        use crate::{Options, convert_one};

        let dir = tempfile::tempdir().expect("tempdir");
        let input = dir.path().join("disc.png");
        std::fs::write(&input, disc_png()).expect("write");

        let converted =
            convert_one(&input, &dir.path().join("disc.svg"), &Options::new()).expect("converts");
        let image = crate::decode::decode(&input, crate::decode::DecodeOptions::default())
            .expect("decodes");

        let score = fidelity(&image, &converted.svg, Rgb::WHITE)
            .expect("verifies")
            .score;
        assert!(
            score > 0.99,
            "a traced disc should come back almost exactly: {score}"
        );
    }

    fn disc_png() -> Vec<u8> {
        use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

        let mut pixmap = Pixmap::new(80, 80).expect("pixmap");
        pixmap.fill(Color::WHITE);
        let mut builder = PathBuilder::new();
        builder.push_circle(40.0, 40.0, 25.0);
        let mut paint = Paint {
            anti_alias: false,
            ..Paint::default()
        };
        paint.set_color_rgba8(220, 30, 40, 255);
        pixmap.fill_path(
            &builder.finish().expect("circle"),
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
        pixmap.encode_png().expect("png")
    }
}
