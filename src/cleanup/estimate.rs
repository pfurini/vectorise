//! The two signals that decide how hard cleanup tries.
//!
//! | Signal | Definition | Finds |
//! |---|---|---|
//! | [`edge_width`] | median 10% to 90% rise distance across detected edges, in pixels | blur |
//! | [`flat_noise`] | mean absolute deviation from the 3x3 median, over flat pixels only, in luminance levels | JPEG ringing and noise |
//!
//! The two are orthogonal, and both are needed. JPEG compression leaves
//! `edge_width` where a clean image has it and raises `flat_noise`; blur does
//! the reverse. `CLEANUP_IMPLEMENTATION_PLAN.md` §9.1 has the separation table.
//!
//! # Calibration
//!
//! Every constant here is calibrated against the exact arithmetic below, and
//! the thresholds of the automatic rule are calibrated against these constants.
//! `CLEANUP_IMPLEMENTATION_PLAN.md` §11 lists the vectors that prove a change
//! kept the arithmetic, and §11.3 lists the changes that break it. The one
//! non-obvious consequence: a hard step measures **0.80**, not 1.00, because
//! its 10% and 90% crossings sit 0.8 px apart inside one pixel interval.

use vtracer::ColorImage;

/// The luminance plane of an image, with clamp-to-edge reads.
///
/// Every signal measures on Rec. 601 luminance, computed once per pixel. A
/// read outside the image returns the nearest edge pixel, which is what the
/// calibrated reference does.
pub(crate) struct Luma {
    values: Vec<f32>,
    width: usize,
    height: usize,
}

impl Luma {
    /// The luminance of every pixel of `image`, row-major.
    pub(crate) fn of(image: &ColorImage) -> Self {
        let values = image
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|[r, g, b, _]| luma(*r, *g, *b))
            .collect();
        Self {
            values,
            width: image.width,
            height: image.height,
        }
    }

    /// Luminance at `(x, y)`, clamped to the image.
    pub(crate) fn at(&self, x: i64, y: i64) -> f32 {
        let x = clamp_index(x, self.width);
        let y = clamp_index(y, self.height);
        self.values.get(y * self.width + x).copied().unwrap_or(0.0)
    }

    fn width(&self) -> i64 {
        i64::try_from(self.width).unwrap_or(i64::MAX)
    }

    fn height(&self) -> i64 {
        i64::try_from(self.height).unwrap_or(i64::MAX)
    }
}

/// Rec. 601 luminance of one straight RGB pixel, in 0..=255.
///
/// Every signal and both filters measure on this. Changing the coefficients
/// invalidates every calibrated constant in this module.
#[expect(
    clippy::suboptimal_flops,
    reason = "a fused multiply-add rounds differently from the calibrated reference"
)]
pub(crate) fn luma(r: u8, g: u8, b: u8) -> f32 {
    0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b)
}

/// `index` clamped into `0..len`, as a `usize`. An empty `len` yields 0.
fn clamp_index(index: i64, len: usize) -> usize {
    let last = i64::try_from(len).unwrap_or(i64::MAX).saturating_sub(1);
    usize::try_from(index.clamp(0, last.max(0))).unwrap_or(0)
}

/// A pixel coordinate as a float, for a sub-pixel position.
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel coordinates are far below 2^24"
)]
const fn coordinate(index: i64) -> f32 {
    index as f32
}

/// One row or one column of a luminance plane, read by position along it.
struct Line<'a> {
    luma: &'a Luma,
    horizontal: bool,
    index: i64,
}

impl Line<'_> {
    fn at(&self, t: i64) -> f32 {
        if self.horizontal {
            self.luma.at(t, self.index)
        } else {
            self.luma.at(self.index, t)
        }
    }

    fn len(&self) -> i64 {
        if self.horizontal {
            self.luma.width()
        } else {
            self.luma.height()
        }
    }
}

/// A gradient (`|v(i+1) - v(i-1)|` in luminance levels) at or below this is
/// not an edge. Calibrated with the §9.1 table: anti-aliased fixture edges
/// gradient at 100 levels and more, JPEG ringing in flat regions well under 8.
const MIN_GRADIENT: f32 = 8.0;

/// An edge whose whole rise is smaller than this, in luminance levels, is
/// ignored. Calibrated with §9.1: it keeps the 10-level ripple of JPEG
/// quality 30 out of the edge set while every fixture edge rises 100 or more.
const MIN_AMPLITUDE: f32 = 20.0;

/// A monotonic run longer than this, in pixels, is a gradient fill, not an
/// edge. Calibrated with §9.1: a sigma-3 blur spans about 18 pixels.
const MAX_SPAN: i64 = 96;

/// Below this many measured edges the estimator refuses to judge and returns
/// `None`. Sixteen is two edges per row over eight rows, the least evidence
/// that gave a stable median on the §11.1 vectors.
const MIN_EDGES: usize = 16;

/// A measured width at or above this, in pixels, is discarded as noise.
const MAX_WIDTH: f32 = 64.0;

/// A 3x3 window whose luminance range exceeds this is an edge and is excluded
/// from `flat_noise`. Calibrated with §9.1: 24 keeps every anti-aliased edge
/// pixel out while the ringing of JPEG quality 30 (about 5 levels) stays in.
const FLAT_RANGE: f32 = 24.0;

/// The 3x3 neighbourhood in row-major order, as `(dy, dx)`. Index 4 is the
/// centre.
const NEIGHBOURHOOD: [(i64, i64); 9] = [
    (-1, -1),
    (-1, 0),
    (-1, 1),
    (0, -1),
    (0, 0),
    (0, 1),
    (1, -1),
    (1, 0),
    (1, 1),
];

/// Median 10% to 90% rise distance across the edges of `image`, in pixels.
///
/// Scans rows and columns of the luminance plane. At every local maximum of
/// the gradient it walks to the nearest extremum on each side, then measures
/// the sub-pixel distance between the 10% and 90% crossings of that rise.
/// The median over every edge found is the answer.
///
/// Returns `None` when fewer than sixteen edges could be measured: too little
/// evidence means the caller should do nothing.
///
/// # Examples
///
/// ```
/// use vectorise::cleanup::estimate::edge_width;
/// use vtracer::ColorImage;
///
/// // A hard vertical step: 0 on the left, 255 on the right.
/// let pixels = (0..64 * 64)
///     .flat_map(|i| {
///         let value = if i % 64 < 32 { 0 } else { 255 };
///         [value, value, value, 255]
///     })
///     .collect();
/// let step = ColorImage { pixels, width: 64, height: 64 };
///
/// let width = edge_width(&step).expect("plenty of edges");
/// assert!((width - 0.8).abs() < 1e-3, "{width}");
/// ```
#[must_use]
pub fn edge_width(image: &ColorImage) -> Option<f32> {
    let luma = Luma::of(image);
    let mut widths = Vec::new();
    scan(&luma, true, &mut widths);
    scan(&luma, false, &mut widths);
    if widths.len() < MIN_EDGES {
        return None;
    }
    widths.sort_unstable_by(f32::total_cmp);
    widths.get(widths.len() / 2).copied()
}

/// Measure every edge along every row (`horizontal`) or column of `luma`.
fn scan(luma: &Luma, horizontal: bool, widths: &mut Vec<f32>) {
    let (outer, inner) = if horizontal {
        (luma.height(), luma.width())
    } else {
        (luma.width(), luma.height())
    };
    for index in 0..outer {
        let line = Line {
            luma,
            horizontal,
            index,
        };
        let mut i = 2_i64;
        while i < inner - 2 {
            let (width, next) = measure_at(&line, i);
            widths.extend(width);
            i = next;
        }
    }
}

/// The edge at position `i` along `line`, if there is one: its measured
/// width, and the position the scan continues from.
///
/// Only local maxima of the gradient count, so one edge is measured at its
/// steepest point rather than at every pixel of its rise.
#[expect(
    clippy::suboptimal_flops,
    reason = "a fused multiply-add rounds differently from the calibrated reference"
)]
fn measure_at(line: &Line<'_>, i: i64) -> (Option<f32>, i64) {
    let gradient = (line.at(i + 1) - line.at(i - 1)).abs();
    if gradient <= MIN_GRADIENT {
        return (None, i + 1);
    }
    if gradient < (line.at(i) - line.at(i - 2)).abs()
        || gradient < (line.at(i + 2) - line.at(i)).abs()
    {
        return (None, i + 1);
    }
    let rising = line.at(i + 1) > line.at(i - 1);

    let (a, b) = extremes(line, i, rising);
    if b <= a || (b - a) > MAX_SPAN {
        return (None, i + 1);
    }
    let next = b.max(i + 1);

    let (lo, hi) = (line.at(a).min(line.at(b)), line.at(a).max(line.at(b)));
    let amplitude = hi - lo;
    if amplitude < MIN_AMPLITUDE {
        return (None, next);
    }

    let c10 = crossing(line, a, b, lo + 0.1 * amplitude);
    let c90 = crossing(line, a, b, lo + 0.9 * amplitude);
    let width = match (c10, c90) {
        (Some(c10), Some(c90)) => (c90 - c10).abs(),
        _ => return (None, next),
    };
    ((width > 0.0 && width < MAX_WIDTH).then_some(width), next)
}

/// Walk from `i` to the local extremum on each side of a `rising` (or
/// falling) edge: the last positions the line keeps going the same way.
fn extremes(line: &Line<'_>, i: i64, rising: bool) -> (i64, i64) {
    let goes_on = |step: f32| if rising { step > 0.0 } else { step < 0.0 };
    let mut a = i;
    while a > 0 && goes_on(line.at(a) - line.at(a - 1)) {
        a -= 1;
    }
    let mut b = i;
    while b < line.len() - 1 && goes_on(line.at(b + 1) - line.at(b)) {
        b += 1;
    }
    (a, b)
}

/// The sub-pixel position in `a..b` where `line` first crosses `threshold`,
/// by linear interpolation between the two pixels around it.
fn crossing(line: &Line<'_>, a: i64, b: i64, threshold: f32) -> Option<f32> {
    (a..b).find_map(|t| {
        let (v0, v1) = (line.at(t), line.at(t + 1));
        let crosses = (v0 - threshold) * (v1 - threshold) <= 0.0 && (v1 - v0).abs() > 1e-6;
        crosses.then(|| coordinate(t) + (threshold - v0) / (v1 - v0))
    })
}

/// Mean absolute deviation from the 3x3 median, in luminance levels, over the
/// pixels whose 3x3 window is flat.
///
/// A window whose luminance range exceeds 24 levels contains an edge and is
/// excluded. That exclusion is what makes this a noise measure rather than a
/// detail measure: a clean hard edge scores zero.
///
/// # Examples
///
/// ```
/// use vectorise::cleanup::estimate::flat_noise;
/// use vtracer::ColorImage;
///
/// let flat = ColorImage { pixels: [128, 128, 128, 255].repeat(16 * 16), width: 16, height: 16 };
/// assert_eq!(flat_noise(&flat), 0.0);
/// ```
#[must_use]
pub fn flat_noise(image: &ColorImage) -> f32 {
    let luma = Luma::of(image);
    let (mut total, mut count) = (0.0_f32, 0.0_f32);
    for y in 1..luma.height() - 1 {
        for x in 1..luma.width() - 1 {
            if let Some(deviation) = flat_deviation(&luma, x, y) {
                total += deviation;
                count += 1.0;
            }
        }
    }
    if count == 0.0 { 0.0 } else { total / count }
}

/// How far the pixel at `(x, y)` sits from its 3x3 median, or `None` when the
/// window spans more than [`FLAT_RANGE`] and so contains an edge.
fn flat_deviation(luma: &Luma, x: i64, y: i64) -> Option<f32> {
    let mut window = [0.0_f32; 9];
    for (slot, (dy, dx)) in window.iter_mut().zip(NEIGHBOURHOOD) {
        *slot = luma.at(x + dx, y + dy);
    }
    let (lo, hi) = window
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &l| (lo.min(l), hi.max(l)));
    if hi - lo > FLAT_RANGE {
        return None;
    }
    let [_, _, _, _, centre, ..] = window;
    let mut sorted = window;
    sorted.sort_unstable_by(f32::total_cmp);
    let [_, _, _, _, median, ..] = sorted;
    Some((centre - median).abs())
}

#[cfg(test)]
mod tests {
    // Exact float comparisons are the point of several of these tests: a
    // flat image must score exactly zero, not approximately.
    #![allow(clippy::expect_used, clippy::indexing_slicing, clippy::float_cmp)]

    use vtracer::ColorImage;

    use super::{edge_width, flat_noise};
    use crate::cleanup::degraded::{Degradation, Fixture, degrade, from_rgba, to_rgba};

    /// A 64x64 opaque greyscale image whose value at `(x, y)` is `f(x, y)`.
    fn grey(f: impl Fn(usize, usize) -> u8) -> ColorImage {
        let mut pixels = Vec::with_capacity(64 * 64 * 4);
        for y in 0..64 {
            for x in 0..64 {
                let value = f(x, y);
                pixels.extend_from_slice(&[value, value, value, 255]);
            }
        }
        ColorImage {
            pixels,
            width: 64,
            height: 64,
        }
    }

    fn hard_step() -> ColorImage {
        grey(|x, _| if x < 32 { 0 } else { 255 })
    }

    fn blurred_step(sigma: f32) -> ColorImage {
        from_rgba(&image::imageops::blur(&to_rgba(&hard_step()), sigma))
    }

    /// A deterministic pseudo-random sequence, so the noisy fixture is the
    /// same on every run without a dependency.
    fn lcg(seed: &mut u64) -> u64 {
        *seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *seed >> 33
    }

    // --- §11.1 calibration vectors -------------------------------------------

    #[test]
    fn edge_width_of_hard_step_is_zero_point_eight() {
        let width = edge_width(&hard_step()).expect("64 rows of edges");
        assert!((width - 0.8).abs() < 1e-3, "{width}");
    }

    #[test]
    fn edge_width_of_three_pixel_ramp_is_two_point_four() {
        // 0 below x=30, 255 from x=33, linear between: 85 at 31, 170 at 32.
        let ramp = grey(|x, _| match x {
            0..=30 => 0,
            31 => 85,
            32 => 170,
            _ => 255,
        });
        let width = edge_width(&ramp).expect("64 rows of edges");
        assert!((width - 2.4).abs() < 1e-3, "{width}");
    }

    #[test]
    fn edge_width_of_gaussian_blurred_step_grows_with_sigma() {
        let widths: Vec<f32> = [1.0, 2.0, 3.0]
            .into_iter()
            .map(|sigma| edge_width(&blurred_step(sigma)).expect("edges"))
            .collect();
        assert!(
            widths.windows(2).all(|pair| pair[0] < pair[1]),
            "strictly increasing: {widths:?}"
        );
        assert!(widths[0] > 0.8, "blur widens a hard step: {widths:?}");
    }

    #[test]
    fn edge_width_is_orientation_invariant() {
        let horizontal = edge_width(&hard_step()).expect("edges");
        let vertical = edge_width(&grey(|_, y| if y < 32 { 0 } else { 255 })).expect("edges");
        assert!(
            (horizontal - vertical).abs() < 1e-6,
            "{horizontal} vs {vertical}"
        );
    }

    #[test]
    fn edge_width_of_flat_image_is_none() {
        assert_eq!(edge_width(&grey(|_, _| 128)), None);
    }

    #[test]
    fn edge_width_ignores_low_amplitude_edges() {
        // A 10-level step clears the gradient floor (8) but not the amplitude
        // floor (20), so it is not an edge.
        let faint = grey(|x, _| if x < 32 { 120 } else { 130 });
        assert_eq!(edge_width(&faint), None);
    }

    #[test]
    fn edge_width_of_tiny_images_is_none_without_panic() {
        for (width, height) in [(1, 1), (1, 40), (40, 1), (3, 3)] {
            let image = ColorImage {
                pixels: (0..width * height)
                    .flat_map(|i| {
                        let value = if i % 2 == 0 { 0 } else { 255 };
                        [value, value, value, 255]
                    })
                    .collect(),
                width,
                height,
            };
            assert_eq!(edge_width(&image), None, "{width}x{height}");
        }
    }

    #[test]
    fn edge_width_of_clean_antialiased_disc_is_below_two() {
        let width = edge_width(&Fixture::Disc.pristine()).expect("a disc has edges");
        assert!((1.2..2.0).contains(&width), "§11.2 says 1.54: {width}");
    }

    #[test]
    fn edge_width_of_hard_edged_fixtures_is_zero_point_eight() {
        for fixture in [Fixture::Badge, Fixture::Logo] {
            let width = edge_width(&fixture.pristine()).expect("edges");
            assert!(
                (width - 0.8).abs() < 1e-3,
                "{}: §11.2 says 0.80: {width}",
                fixture.name()
            );
        }
    }

    #[test]
    fn flat_noise_of_flat_image_is_zero() {
        assert_eq!(flat_noise(&grey(|_, _| 128)), 0.0);
    }

    #[test]
    fn flat_noise_of_salt_and_pepper_is_high() {
        // Sparse specks 20 levels above the base. The amplitude sits under
        // `FLAT_RANGE` on purpose: a speck the size of an edge would be
        // excluded as one, which is the point of the measure.
        let mut seed = 7;
        let specks: Vec<bool> = (0..64 * 64)
            .map(|_| lcg(&mut seed).is_multiple_of(20))
            .collect();
        let speckled = grey(|x, y| if specks[y * 64 + x] { 148 } else { 128 });
        let noise = flat_noise(&speckled);
        assert!(noise > 0.05, "{noise}");
    }

    #[test]
    fn flat_noise_ignores_edges() {
        assert_eq!(
            flat_noise(&hard_step()),
            0.0,
            "the edge windows are excluded"
        );
    }

    #[test]
    fn flat_noise_of_tiny_images_is_zero_without_panic() {
        for (width, height) in [(1, 1), (1, 40), (40, 1), (2, 2)] {
            let image = ColorImage {
                pixels: [200, 10, 90, 255].repeat(width * height),
                width,
                height,
            };
            assert_eq!(flat_noise(&image), 0.0, "{width}x{height}");
        }
    }

    #[test]
    fn flat_noise_of_jpeg_artifacts_exceeds_threshold() {
        let noise = flat_noise(&degrade(Fixture::Disc, Degradation::Jpeg30));
        assert!(noise > 0.05, "§9.1 says 0.129: {noise}");
    }

    // --- separation: the property the auto rule depends on --------------------

    #[test]
    fn signals_separate_clean_from_blurred() {
        for fixture in Fixture::ALL {
            let clean = edge_width(&fixture.pristine()).expect("edges");
            assert!(clean < 2.0, "{} clean: {clean}", fixture.name());
            for degradation in Degradation::BLURRED {
                let blurred = edge_width(&degrade(fixture, degradation)).expect("edges");
                assert!(
                    blurred > 3.5,
                    "{} {}: {blurred}",
                    fixture.name(),
                    degradation.name()
                );
            }
        }
    }

    #[test]
    fn signals_separate_clean_from_noisy() {
        for fixture in Fixture::ALL {
            let clean = flat_noise(&fixture.pristine());
            assert!(clean < 0.02, "{} clean: {clean}", fixture.name());
            for degradation in Degradation::NOISY {
                let noisy = flat_noise(&degrade(fixture, degradation));
                assert!(
                    noisy > 0.05,
                    "{} {}: {noisy}",
                    fixture.name(),
                    degradation.name()
                );
            }
        }
    }

    #[test]
    fn signals_do_not_mistake_one_regime_for_the_other() {
        // JPEG leaves edges sharp; blur leaves flat regions quiet. Each signal
        // stays on its own side of the rule's thresholds in the other regime.
        for fixture in Fixture::ALL {
            let jpeg = edge_width(&degrade(fixture, Degradation::Jpeg30)).expect("edges");
            assert!(jpeg < 2.25, "{} jpeg30 edge width: {jpeg}", fixture.name());
            let blur = flat_noise(&degrade(fixture, Degradation::Blur3));
            assert!(blur < 0.02, "{} blur3 flat noise: {blur}", fixture.name());
        }
    }
}
