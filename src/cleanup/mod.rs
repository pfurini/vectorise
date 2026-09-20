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
//! | [`filter`] | the two operators: Kuwahara and toggle contrast |
//! | this one | choose the radii from the measurements and run the operators |
//!
//! # The no-op guarantee
//!
//! On a clean, sharp input the pass returns its input byte for byte. A clean
//! image measures as clean, both radii come out zero, and [`cleanup`] returns
//! before touching a filter. That is a branch at the top of the function, not
//! a property that happens to emerge from two filters being the identity at
//! radius zero. Cleanup can therefore be on by default.
//!
//! # The automatic rule
//!
//! Each radius is driven by its own signal, because the two are orthogonal:
//! a JPEG icon is very noisy and perfectly sharp; a blurred PNG is the
//! reverse. Sharpening a noisy image without denoising it first makes it
//! worse, and denoising a blurred one barely helps.
//! `CLEANUP_IMPLEMENTATION_PLAN.md` §9.3 validates the rule on every cell of
//! a 24-case matrix, and `tests/cleanup.rs` keeps that matrix under test.
//!
//! # Examples
//!
//! ```
//! use vectorise::cleanup::{CleanupOptions, cleanup};
//! use vtracer::ColorImage;
//!
//! // A flat image is as clean as an image gets: nothing to do.
//! let flat = ColorImage { pixels: [10, 20, 30, 255].repeat(32 * 32), width: 32, height: 32 };
//! let (cleaned, report) = cleanup(&flat, CleanupOptions::default());
//! assert_eq!(cleaned.pixels, flat.pixels);
//! assert_eq!((report.denoise, report.sharpen), (0, 0));
//! assert_eq!(report.changed_fraction, 0.0);
//! ```

pub mod estimate;
pub mod filter;

use vtracer::ColorImage;

/// The calibration fixtures of `CLEANUP_IMPLEMENTATION_PLAN.md` §10.7, shared
/// with the integration tests so each drawing exists once.
#[cfg(test)]
#[path = "../../tests/common/degraded.rs"]
pub(crate) mod degraded;

/// Whether the pass runs at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum CleanupMode {
    /// Measure the raster and choose both radii from what it finds.
    #[default]
    Auto,
    /// Leave the raster exactly as decoded.
    Off,
}

/// How hard the cleanup pass tries, and whether it runs at all.
///
/// An explicit radius replaces the automatic choice for that operator only.
/// Zero disables it. [`CleanupMode::Off`] wins over any radius: the command
/// line refuses that combination as contradictory, and the library treats it
/// as off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupOptions {
    /// Automatic, or not at all.
    pub mode: CleanupMode,
    /// Kuwahara radius in pixels, or `None` to choose from `flat_noise`.
    pub denoise: Option<u8>,
    /// Toggle contrast radius in pixels, or `None` to choose from `edge_width`.
    pub sharpen: Option<u8>,
    /// Split each filter over the `rayon` pool. Right when the conversion
    /// runs alone; wrong in a batch that already fills the cores, where inner
    /// parallelism would only contend. The output is the same either way.
    pub parallel: bool,
}

impl Default for CleanupOptions {
    fn default() -> Self {
        Self {
            mode: CleanupMode::Auto,
            denoise: None,
            sharpen: None,
            parallel: true,
        }
    }
}

impl CleanupOptions {
    /// Options that leave the raster untouched.
    #[must_use]
    pub fn off() -> Self {
        Self {
            mode: CleanupMode::Off,
            ..Self::default()
        }
    }
}

/// What the pass measured and what it did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cleanup {
    /// Median 10% to 90% edge rise, in pixels; `None` with too few edges to
    /// judge, or when the pass was off.
    pub edge_width: Option<f32>,
    /// Mean deviation from the 3x3 median over flat pixels, in luminance
    /// levels. Zero when the pass was off.
    pub flat_noise: f32,
    /// The Kuwahara radius applied. Zero means the operator did not run.
    pub denoise: u8,
    /// The toggle contrast radius applied. Zero means the operator did not run.
    pub sharpen: u8,
    /// Mean absolute error between the cleaned and the decoded image over
    /// RGB, in `0..=1`. The quantity ADR-0008 defines for `--verify`, applied
    /// to two rasters. Zero means cleanup changed nothing. It is an error, not
    /// a score: report it as is, never as `1 - error`.
    pub changed_fraction: f64,
}

impl Cleanup {
    /// Did either operator run?
    #[must_use]
    pub const fn acted(&self) -> bool {
        self.denoise > 0 || self.sharpen > 0
    }
}

/// `flat_noise` above this, in luminance levels, selects the strong
/// denoise radius. Every JPEG fixture of §9.1 measures 0.077 or more; every
/// blurred or clean one measures 0.016 or less.
const STRONG_NOISE: f32 = 0.05;

/// `flat_noise` above this selects the mild denoise radius. Every clean
/// fixture of §9.1 measures 0.000; the `soft` and `blur3` cells reach 0.016.
const MILD_NOISE: f32 = 0.02;

/// The Kuwahara radius for strong noise. §9.3: `d3` wins or ties `best` on
/// every JPEG cell.
const STRONG_DENOISE: u8 = 3;

/// The Kuwahara radius for mild noise.
const MILD_DENOISE: u8 = 2;

/// An `edge_width` at or below this, in pixels, is sharp: the anti-aliased
/// fixtures of §9.1 measure 1.47 to 1.54, hard edges 0.80.
const SHARP_EDGE_WIDTH: f32 = 1.5;

/// Every further 1.5 px of edge width adds one to the toggle radius. §9.3:
/// `s2` for the sigma-1.5 cells (widths 3.9 to 4.5), `s3` for sigma 3 (7.7
/// to 8.8).
const PIXELS_PER_SHARPEN_STEP: f32 = 1.5;

/// The largest toggle radius the rule selects. A wider window rounds off
/// genuinely sharp small detail; the star fixture is in the matrix to catch
/// exactly that.
const MAX_SHARPEN: u8 = 3;

/// The Kuwahara radius the automatic rule selects for a measured
/// `flat_noise`.
///
/// # Examples
///
/// ```
/// use vectorise::cleanup::denoise_for;
///
/// assert_eq!(denoise_for(0.0), 0);
/// assert_eq!(denoise_for(0.03), 2);
/// assert_eq!(denoise_for(0.13), 3);
/// ```
#[must_use]
pub fn denoise_for(flat_noise: f32) -> u8 {
    if flat_noise > STRONG_NOISE {
        STRONG_DENOISE
    } else if flat_noise > MILD_NOISE {
        MILD_DENOISE
    } else {
        0
    }
}

/// The toggle contrast radius the automatic rule selects for a measured
/// `edge_width`. Too few edges to measure (`None`) means do nothing.
///
/// # Examples
///
/// ```
/// use vectorise::cleanup::sharpen_for;
///
/// assert_eq!(sharpen_for(None), 0);
/// assert_eq!(sharpen_for(Some(1.5)), 0);
/// assert_eq!(sharpen_for(Some(4.5)), 2);
/// assert_eq!(sharpen_for(Some(8.8)), 3);
/// ```
#[must_use]
pub fn sharpen_for(edge_width: Option<f32>) -> u8 {
    let Some(width) = edge_width else {
        return 0;
    };
    let steps = ((width - SHARP_EDGE_WIDTH) / PIXELS_PER_SHARPEN_STEP).round();
    // A rounded value clamped into `0..=3` is one of four integers.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped into 0..=MAX_SHARPEN before the cast"
    )]
    let radius = steps.clamp(0.0, f32::from(MAX_SHARPEN)) as u8;
    radius
}

/// Clean an image before tracing. Returns the image and what was done to it.
///
/// Pure, deterministic, and free of I/O. With [`CleanupMode::Off`], or when
/// both radii resolve to zero, the input comes back byte for byte.
///
/// # Examples
///
/// ```
/// use vectorise::cleanup::{CleanupOptions, cleanup};
/// use vtracer::ColorImage;
///
/// let image = ColorImage { pixels: [200, 30, 30, 255].repeat(16 * 16), width: 16, height: 16 };
/// let explicit = CleanupOptions { denoise: Some(2), sharpen: Some(0), ..CleanupOptions::default() };
/// let (cleaned, report) = cleanup(&image, explicit);
/// assert_eq!(report.denoise, 2);
/// assert_eq!(cleaned.pixels, image.pixels, "a flat image is a fixed point of every filter");
/// ```
#[must_use]
pub fn cleanup(image: &ColorImage, options: CleanupOptions) -> (ColorImage, Cleanup) {
    if options.mode == CleanupMode::Off {
        return (image.clone(), Cleanup::untouched());
    }

    let edge_width = estimate::edge_width(image);
    let flat_noise = estimate::flat_noise(image);
    let denoise = options.denoise.unwrap_or_else(|| denoise_for(flat_noise));
    let sharpen = options.sharpen.unwrap_or_else(|| sharpen_for(edge_width));
    let report = Cleanup {
        edge_width,
        flat_noise,
        denoise,
        sharpen,
        changed_fraction: 0.0,
    };

    // The no-op guarantee, as one branch: nothing to do means the input
    // comes back untouched, whatever the filters would make of radius zero.
    if !report.acted() {
        return (image.clone(), report);
    }

    let cleaned = match (denoise, sharpen) {
        (0, radius) => filter::toggle_contrast(image, radius, options.parallel),
        (radius, 0) => filter::kuwahara(image, radius, options.parallel),
        (smooth, snap) => filter::toggle_contrast(
            &filter::kuwahara(image, smooth, options.parallel),
            snap,
            options.parallel,
        ),
    };
    let changed_fraction = rgb_error(image, &cleaned);
    (
        cleaned,
        Cleanup {
            changed_fraction,
            ..report
        },
    )
}

impl Cleanup {
    /// The report for a pass that did not run.
    const fn untouched() -> Self {
        Self {
            edge_width: None,
            flat_noise: 0.0,
            denoise: 0,
            sharpen: 0,
            changed_fraction: 0.0,
        }
    }
}

/// Mean absolute error between two images over RGB, in `0..=1`: the metric
/// of [`crate::verify::compare`], with alpha left out so it measures what
/// ADR-0008 says it measures.
fn rgb_error(before: &ColorImage, after: &ColorImage) -> f64 {
    let rgb = |image: &ColorImage| -> Vec<u8> {
        image
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|[r, g, b, _]| [*r, *g, *b])
            .collect()
    };
    crate::verify::compare(&rgb(before), &rgb(after)).mean_absolute_error
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::float_cmp, clippy::indexing_slicing)]

    use std::collections::BTreeSet;

    use proptest::prelude::*;
    use vtracer::ColorImage;

    use super::degraded::{Degradation, Fixture, degrade, from_rgba, to_rgba};
    use super::{Cleanup, CleanupMode, CleanupOptions, cleanup, denoise_for, sharpen_for};

    fn auto() -> CleanupOptions {
        CleanupOptions::default()
    }

    fn assert_identity(name: &str, image: &ColorImage, options: CleanupOptions) -> Cleanup {
        let (cleaned, report) = cleanup(image, options);
        assert_eq!(
            (cleaned.width, cleaned.height),
            (image.width, image.height),
            "{name}"
        );
        assert!(
            cleaned.pixels == image.pixels,
            "{name}: not byte-identical (report {report:?})"
        );
        assert_eq!(report.changed_fraction, 0.0, "{name}");
        report
    }

    /// A 96x96 icon with hard edges: a rect, a disc, and a bar, no
    /// anti-aliasing.
    fn synthetic_icon() -> ColorImage {
        use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

        let mut pixmap = Pixmap::new(96, 96).expect("pixmap");
        pixmap.fill(Color::WHITE);
        let mut paint = Paint {
            anti_alias: false,
            ..Paint::default()
        };
        let mut draw = |path: tiny_skia::Path, (r, g, b): (u8, u8, u8)| {
            paint.set_color_rgba8(r, g, b, 255);
            pixmap.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        };
        draw(
            PathBuilder::from_rect(Rect::from_xywh(8.0, 8.0, 40.0, 30.0).expect("rect")),
            (200, 40, 40),
        );
        let mut circle = PathBuilder::new();
        circle.push_circle(68.0, 30.0, 18.0);
        draw(circle.finish().expect("circle"), (40, 90, 200));
        draw(
            PathBuilder::from_rect(Rect::from_xywh(8.0, 60.0, 80.0, 12.0).expect("rect")),
            (30, 160, 80),
        );
        super::degraded::pristine(&pixmap)
    }

    fn hard_edged_rect() -> ColorImage {
        let mut pixels = Vec::with_capacity(64 * 64 * 4);
        for y in 0..64 {
            for x in 0..64 {
                let inside = (12..52).contains(&x) && (20..44).contains(&y);
                pixels.extend_from_slice(if inside {
                    &[30, 80, 220, 255]
                } else {
                    &[255, 255, 255, 255]
                });
            }
        }
        ColorImage {
            pixels,
            width: 64,
            height: 64,
        }
    }

    // --- the guarantee, highest priority ------------------------------------------

    #[test]
    fn auto_on_clean_synthetic_icon_is_identity() {
        let report = assert_identity("icon", &synthetic_icon(), auto());
        assert_eq!((report.denoise, report.sharpen), (0, 0));
    }

    #[test]
    fn auto_on_clean_antialiased_disc_is_identity() {
        assert_identity("disc", &Fixture::Disc.pristine(), auto());
    }

    #[test]
    fn auto_on_clean_four_colour_logo_is_identity() {
        assert_identity("logo", &Fixture::Logo.pristine(), auto());
    }

    #[test]
    fn auto_on_hard_edged_rect_is_identity() {
        assert_identity("rect", &hard_edged_rect(), auto());
    }

    #[test]
    fn auto_on_every_clean_fixture_is_identity() {
        for fixture in Fixture::ALL {
            let report = assert_identity(fixture.name(), &fixture.pristine(), auto());
            assert_eq!(
                (report.denoise, report.sharpen),
                (0, 0),
                "{}",
                fixture.name()
            );
        }
    }

    #[test]
    fn off_is_always_identity() {
        let off = CleanupOptions::off();
        for fixture in Fixture::ALL {
            for degradation in Degradation::ALL {
                let name = format!("{} {}", fixture.name(), degradation.name());
                let report = assert_identity(&name, &degrade(fixture, degradation), off);
                assert_eq!(report, Cleanup::untouched(), "{name}: nothing measured");
            }
        }
        // Off wins over an explicit radius, too.
        let contradictory = CleanupOptions {
            denoise: Some(3),
            sharpen: Some(3),
            ..CleanupOptions::off()
        };
        assert_identity(
            "off with radii",
            &degrade(Fixture::Logo, Degradation::Blur3),
            contradictory,
        );
    }

    // --- the rule ---------------------------------------------------------------------

    #[test]
    fn auto_selects_no_denoise_when_flat_noise_is_zero() {
        assert_eq!(denoise_for(0.0), 0);
        assert_eq!(denoise_for(0.02), 0, "the threshold itself is not above it");
        let (_, report) = cleanup(&degrade(Fixture::Disc, Degradation::Blur3), auto());
        assert_eq!(report.denoise, 0, "{report:?}");
    }

    #[test]
    fn auto_selects_denoise_three_on_jpeg_quality_thirty() {
        for fixture in Fixture::ALL {
            let (_, report) = cleanup(&degrade(fixture, Degradation::Jpeg30), auto());
            assert_eq!(report.denoise, 3, "{}: {report:?}", fixture.name());
            assert_eq!(report.sharpen, 0, "{}: a JPEG is sharp", fixture.name());
        }
    }

    #[test]
    fn auto_selects_mild_denoise_between_the_thresholds() {
        assert_eq!(denoise_for(0.03), 2);
        assert_eq!(
            denoise_for(0.05),
            2,
            "the strong threshold itself is not above it"
        );
        assert_eq!(denoise_for(0.0501), 3);
    }

    #[test]
    fn auto_selects_sharpen_from_edge_width() {
        for (width, expected) in [
            (0.8, 0),
            (1.5, 0),
            (2.0, 0),
            (3.0, 1),
            (4.5, 2),
            (5.4, 3),
            (8.8, 3),
        ] {
            assert_eq!(sharpen_for(Some(width)), expected, "{width}");
        }
        assert_eq!(sharpen_for(None), 0, "too few edges means do nothing");
    }

    #[test]
    fn auto_clamps_sharpen_at_three() {
        assert_eq!(sharpen_for(Some(100.0)), 3);
        assert_eq!(sharpen_for(Some(f32::MAX)), 3);
        assert_eq!(sharpen_for(Some(-5.0)), 0, "and at zero below");
    }

    #[test]
    fn auto_picks_match_the_plan_matrix() {
        // §9.3, the "auto picks" column.
        let expected = |fixture: Fixture, degradation: Degradation| match degradation {
            Degradation::Clean => (0, 0),
            // The disc's `soft` cell measures 5.42 px and picks s3 (§9.3).
            Degradation::Soft if fixture == Fixture::Disc => (0, 3),
            Degradation::Blur15 | Degradation::Soft => (0, 2),
            Degradation::Blur3 => (0, 3),
            Degradation::Jpeg30 => (3, 0),
            Degradation::Blur15Jpeg50 => (3, 2),
        };
        for fixture in Fixture::ALL {
            for degradation in Degradation::ALL {
                let (_, report) = cleanup(&degrade(fixture, degradation), auto());
                let want = expected(fixture, degradation);
                assert_eq!(
                    (report.denoise, report.sharpen),
                    want,
                    "{} {}: {report:?}",
                    fixture.name(),
                    degradation.name()
                );
            }
        }
    }

    #[test]
    fn explicit_denoise_overrides_auto() {
        let image = degrade(Fixture::Badge, Degradation::Jpeg30);
        let options = CleanupOptions {
            denoise: Some(1),
            ..auto()
        };
        let (cleaned, report) = cleanup(&image, options);
        assert_eq!(report.denoise, 1);
        assert_eq!(report.sharpen, 0, "sharpen still follows the measurement");
        assert_eq!(
            cleaned.pixels,
            super::filter::kuwahara(&image, 1, true).pixels,
            "exactly one Kuwahara pass at the given radius"
        );
    }

    #[test]
    fn explicit_sharpen_overrides_auto() {
        let image = degrade(Fixture::Badge, Degradation::Blur3);
        let options = CleanupOptions {
            sharpen: Some(1),
            ..auto()
        };
        let (cleaned, report) = cleanup(&image, options);
        assert_eq!(report.sharpen, 1);
        assert_eq!(report.denoise, 0, "denoise still follows the measurement");
        assert_eq!(
            cleaned.pixels,
            super::filter::toggle_contrast(&image, 1, true).pixels
        );
    }

    #[test]
    fn explicit_zero_disables_one_operator_but_not_the_other() {
        let image = degrade(Fixture::Logo, Degradation::Blur15Jpeg50);
        let (_, both) = cleanup(&image, auto());
        assert_eq!((both.denoise, both.sharpen), (3, 2));

        let no_denoise = CleanupOptions {
            denoise: Some(0),
            ..auto()
        };
        let (cleaned, report) = cleanup(&image, no_denoise);
        assert_eq!((report.denoise, report.sharpen), (0, 2));
        assert_eq!(
            cleaned.pixels,
            super::filter::toggle_contrast(&image, 2, true).pixels
        );

        let no_sharpen = CleanupOptions {
            sharpen: Some(0),
            ..auto()
        };
        let (cleaned, report) = cleanup(&image, no_sharpen);
        assert_eq!((report.denoise, report.sharpen), (3, 0));
        assert_eq!(
            cleaned.pixels,
            super::filter::kuwahara(&image, 3, true).pixels
        );

        let neither = CleanupOptions {
            denoise: Some(0),
            sharpen: Some(0),
            ..auto()
        };
        assert_identity("both zero", &image, neither);
    }

    #[test]
    fn both_operators_run_denoise_first() {
        let image = degrade(Fixture::Logo, Degradation::Blur15Jpeg50);
        let (cleaned, report) = cleanup(&image, auto());
        assert_eq!((report.denoise, report.sharpen), (3, 2));
        let expected =
            super::filter::toggle_contrast(&super::filter::kuwahara(&image, 3, true), 2, true);
        assert_eq!(cleaned.pixels, expected.pixels);
    }

    #[test]
    fn cleanup_reports_what_it_measured_and_what_it_did() {
        let image = degrade(Fixture::Logo, Degradation::Blur3);
        let (_, report) = cleanup(&image, auto());
        let width = report.edge_width.expect("edges");
        assert!((7.5..8.5).contains(&width), "§9.1 says 8.09: {width}");
        assert!(
            report.flat_noise < 0.01,
            "§9.1 says 0.002: {}",
            report.flat_noise
        );
        assert_eq!((report.denoise, report.sharpen), (0, 3));
        assert!(report.acted());
        assert!(
            report.changed_fraction > 0.0 && report.changed_fraction < 0.1,
            "sharpening a blurred logo moves a few percent of the raster: {}",
            report.changed_fraction
        );
    }

    #[test]
    fn changed_fraction_is_zero_for_an_identity_pass() {
        let (_, report) = cleanup(&Fixture::Star.pristine(), auto());
        assert_eq!(report.changed_fraction, 0.0);
        assert!(!report.acted());
        let (_, report) = cleanup(
            &degrade(Fixture::Star, Degradation::Blur3),
            CleanupOptions::off(),
        );
        assert_eq!(report.changed_fraction, 0.0);
    }

    #[test]
    fn changed_fraction_is_the_rgb_error_of_adr_0008() {
        let image = degrade(Fixture::Badge, Degradation::Jpeg30);
        let (cleaned, report) = cleanup(&image, auto());
        let rgb = |image: &ColorImage| -> Vec<u8> {
            image
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect()
        };
        let expected = crate::verify::compare(&rgb(&image), &rgb(&cleaned)).mean_absolute_error;
        assert_eq!(report.changed_fraction, expected);
        assert!(expected > 0.0);
    }

    #[test]
    fn parallel_flag_does_not_change_the_result() {
        let image = degrade(Fixture::Star, Degradation::Blur15Jpeg50);
        let sequential = CleanupOptions {
            parallel: false,
            ..auto()
        };
        let (parallel, report) = cleanup(&image, auto());
        let (serial, serial_report) = cleanup(&image, sequential);
        assert_eq!(parallel.pixels, serial.pixels);
        assert_eq!(report, serial_report);
    }

    #[test]
    fn mode_default_is_auto() {
        assert_eq!(CleanupMode::default(), CleanupMode::Auto);
        assert_eq!(CleanupOptions::default().mode, CleanupMode::Auto);
        assert_eq!(CleanupOptions::off().mode, CleanupMode::Off);
    }

    // --- property tests ---------------------------------------------------------------

    /// A scene the operators are meant for: flat rectangles on a flat
    /// background, then optionally blurred and optionally speckled with
    /// low-amplitude noise.
    #[derive(Debug, Clone)]
    struct Scene {
        background: [u8; 3],
        rects: Vec<(u8, u8, u8, u8, [u8; 3])>,
        sigma: f32,
        noise: u8,
        seed: u64,
    }

    fn scene() -> impl Strategy<Value = Scene> {
        let rgb = || any::<[u8; 3]>();
        let rect = (0..40_u8, 0..40_u8, 4..30_u8, 4..30_u8, rgb());
        (
            rgb(),
            proptest::collection::vec(rect, 1..=4),
            0.0_f32..2.5,
            0..6_u8,
            any::<u64>(),
        )
            .prop_map(|(background, rects, sigma, noise, seed)| Scene {
                background,
                rects,
                sigma,
                noise,
                seed,
            })
    }

    fn render(scene: &Scene) -> ColorImage {
        const SIZE: usize = 48;
        let mut pixels = Vec::with_capacity(SIZE * SIZE * 4);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let mut colour = scene.background;
                for &(x0, y0, w, h, fill) in &scene.rects {
                    let (x0, y0, w, h) = (
                        usize::from(x0),
                        usize::from(y0),
                        usize::from(w),
                        usize::from(h),
                    );
                    if (x0..x0 + w).contains(&x) && (y0..y0 + h).contains(&y) {
                        colour = fill;
                    }
                }
                pixels.extend_from_slice(&colour);
                pixels.push(255);
            }
        }
        let mut image = ColorImage {
            pixels,
            width: SIZE,
            height: SIZE,
        };
        if scene.sigma > 0.3 {
            image = from_rgba(&image::imageops::blur(&to_rgba(&image), scene.sigma));
        }
        if scene.noise > 0 {
            let mut seed = scene.seed;
            for pixel in image.pixels.as_chunks_mut::<4>().0 {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let amplitude = i16::from(scene.noise);
                let delta =
                    i16::try_from((seed >> 33) % u64::try_from(2 * amplitude + 1).expect("fits"))
                        .expect("fits")
                        - amplitude;
                for channel in &mut pixel[..3] {
                    *channel =
                        u8::try_from((i16::from(*channel) + delta).clamp(0, 255)).expect("clamped");
                }
            }
        }
        image
    }

    fn colours(image: &ColorImage) -> BTreeSet<[u8; 4]> {
        image.pixels.as_chunks::<4>().0.iter().copied().collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 256,
            // A fixed seed keeps CI reproducible; failures still shrink.
            rng_algorithm: proptest::test_runner::RngAlgorithm::ChaCha,
            ..ProptestConfig::default()
        })]

        /// The pass is not idempotent: Kuwahara smooths residual noise a
        /// little further on a second run, and toggle contrast sharpens a
        /// very wide blur in steps. What holds, and what matters, is that it
        /// converges: a second pass never changes more than the first, and
        /// changes nothing at all once the first did nothing.
        #[test]
        fn prop_cleanup_converges_on_its_own_output(scene in scene()) {
            let image = render(&scene);
            let (once, first) = cleanup(&image, auto());
            let (twice, second) = cleanup(&once, auto());
            prop_assert!(
                second.changed_fraction <= first.changed_fraction,
                "first {:?}, second {:?}",
                first,
                second
            );
            if !first.acted() {
                prop_assert_eq!(&twice.pixels, &once.pixels);
            }
        }

        /// Only the sharpening operator is colour-preserving: Kuwahara emits
        /// quadrant means, which are new colours by design. The property
        /// therefore holds for the pass with denoising disabled, which is the
        /// pass a blurred but noise-free input gets.
        #[test]
        fn prop_cleanup_never_introduces_a_colour_absent_from_the_input(scene in scene()) {
            let image = render(&scene);
            let sharpen_only = CleanupOptions { denoise: Some(0), ..auto() };
            let (cleaned, _) = cleanup(&image, sharpen_only);
            prop_assert!(colours(&cleaned).is_subset(&colours(&image)));
        }

        #[test]
        fn prop_cleanup_preserves_dimensions(scene in scene()) {
            let image = render(&scene);
            let (cleaned, _) = cleanup(&image, auto());
            prop_assert_eq!((cleaned.width, cleaned.height), (image.width, image.height));
            prop_assert_eq!(cleaned.pixels.len(), image.pixels.len());
        }

        #[test]
        fn prop_off_equals_identity_for_any_input(
            width in 1..24_usize,
            height in 1..24_usize,
            seed in any::<u64>(),
        ) {
            let mut state = seed;
            let pixels = (0..width * height * 4).map(|_| {
                state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                u8::try_from(state >> 56).expect("a byte")
            }).collect();
            let image = ColorImage { pixels, width, height };
            let (cleaned, report) = cleanup(&image, CleanupOptions::off());
            prop_assert_eq!(&cleaned.pixels, &image.pixels);
            prop_assert_eq!(report, Cleanup::untouched());
        }
    }
}
