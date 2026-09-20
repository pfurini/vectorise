//! Turn a decoded image into VTracer's document IR.
//!
//! This module is a thin, typed wrapper: it maps our options onto
//! [`vtracer::Config`] and runs the pipeline. No vtracer type crosses the
//! crate boundary except the [`VectorDoc`] that Phase 5 consumes.
//!
//! # Option naming
//!
//! Our names follow the CLI, which does not always match VTracer's field
//! names. The mapping, verified against the pinned source in
//! `docs/api-notes.md` §4:
//!
//! | Ours | VTracer |
//! |---|---|
//! | `gradient_step` | `layer_difference` |
//! | `segment_length` | `length_threshold` |
//! | `threshold` | `binary_threshold` |
//! | `adaptive` | `binary_adaptive` |
//!
//! # Precision
//!
//! VTracer rounds every coordinate to `path_precision` decimals inside the
//! pipeline. Two decimals, its default, is enough for output but not for
//! fitting a circle to a traced outline, so [`trace`] raises it to
//! [`INTERNAL_PRECISION`]. User-facing rounding happens in the writer instead.
//! [`TraceOptions::to_config`] deliberately does *not* apply that override, so
//! the default mapping stays comparable to `Config::default()` field by field.

use std::fmt;
use std::str::FromStr;

use vtracer::ColorImage;
use vtracer::ir::VectorDoc;

use crate::color::Rgb;

/// Decimals VTracer keeps while we still need the geometry for shape fitting.
///
/// Six decimals is far below the noise floor of a traced pixel outline, so it
/// is effectively "do not quantize" without turning off the cleanup pass that
/// shares the same switch.
pub const INTERNAL_PRECISION: u32 = 6;

/// Which region-forming algorithm segments the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Clustering {
    /// Hierarchical color clustering. VTracer's classic path.
    #[default]
    #[value(name = "color-cluster")]
    ColorCluster,
    /// Threshold to black and white, then cluster the foreground.
    #[value(name = "bw")]
    Binary,
    /// Hierarchical watershed on the pixel graph.
    Watershed,
}

/// How regions are combined into one document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Hierarchical {
    /// Trace each layer and stack them in paint order.
    #[default]
    Stacked,
    /// Seam-free mosaic: each shared boundary is fitted once.
    Cutout,
}

/// How a region outline becomes vector geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum FitMode {
    /// Exact pixel-lattice polyline, no smoothing.
    Pixel,
    /// Douglas-Peucker polygon: straight edges, fewer points.
    Polygon,
    /// Corner detection plus least-squares cubics.
    #[default]
    Spline,
}

/// A starting point tuned for a kind of input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Preset {
    /// Black-and-white line art.
    Bw,
    /// Flat, poster-like color with a full palette.
    Poster,
    /// Photographic input: heavier speckle filtering, coarser layering.
    Photo,
    /// Choose for the user. v1 picks `Poster`; see ADR-0003.
    #[default]
    Auto,
}

impl Preset {
    /// The preset actually handed to VTracer.
    ///
    /// # Examples
    ///
    /// ```
    /// use vectorise::trace::Preset;
    ///
    /// assert_eq!(Preset::Auto.resolve(), Preset::Poster);
    /// assert_eq!(Preset::Bw.resolve(), Preset::Bw);
    /// ```
    #[must_use]
    pub const fn resolve(self) -> Self {
        match self {
            Self::Auto | Self::Poster => Self::Poster,
            Self::Bw => Self::Bw,
            Self::Photo => Self::Photo,
        }
    }
}

/// Tracing parameters.
///
/// Every field is optional. `None` means "whatever the preset says", and with
/// no preset that is VTracer's own default. An explicit value always wins over
/// the preset.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TraceOptions {
    /// Start from this preset. `None` uses VTracer's defaults untouched; the
    /// CLI passes `Some(Preset::Auto)`.
    pub preset: Option<Preset>,
    /// Region-forming algorithm.
    pub clustering: Option<Clustering>,
    /// Stacked layers or a seam-free mosaic.
    pub hierarchical: Option<Hierarchical>,
    /// Curve-fitting mode.
    pub mode: Option<FitMode>,
    /// Speckle filter, as a side length in pixels (0..=128).
    pub filter_speckle: Option<u8>,
    /// Significant bits per RGB channel (1..=8).
    pub color_precision: Option<u8>,
    /// Color difference between gradient layers (VTracer's `layer_difference`).
    pub gradient_step: Option<u8>,
    /// Corner threshold in degrees.
    pub corner_threshold: Option<u16>,
    /// Segment length threshold in pixels (VTracer's `length_threshold`).
    pub segment_length: Option<f64>,
    /// Splice threshold in degrees.
    pub splice_threshold: Option<u16>,
    /// Curve simplification tolerance in pixels. Spline mode only.
    pub simplify: Option<f64>,
    /// Auto-quantize to at most this many colors.
    pub max_colors: Option<u16>,
    /// Fixed palette. Takes priority over `max_colors`.
    pub palette: Option<Vec<Rgb>>,
    /// Binary-mode threshold (VTracer's `binary_threshold`).
    pub threshold: Option<u8>,
    /// Use adaptive thresholding in binary mode.
    pub adaptive: Option<bool>,
    /// Where to cut the watershed hierarchy.
    pub watershed_detail: Option<u32>,
}

impl TraceOptions {
    /// The VTracer configuration these options describe.
    ///
    /// This is the pure mapping: preset first, then every explicit override.
    /// It does not apply the [`INTERNAL_PRECISION`] override, which belongs to
    /// [`trace`].
    ///
    /// # Examples
    ///
    /// ```
    /// use vectorise::trace::TraceOptions;
    ///
    /// // No preset and no overrides is VTracer's own default.
    /// let config = TraceOptions::default().to_config();
    /// assert_eq!(config.color_precision, vtracer::Config::default().color_precision);
    /// ```
    pub fn to_config(&self) -> vtracer::Config {
        let mut config = match self.preset.map(Preset::resolve) {
            Some(Preset::Bw) => vtracer::Config::from_preset(vtracer::Preset::Bw),
            Some(Preset::Poster | Preset::Auto) => {
                vtracer::Config::from_preset(vtracer::Preset::Poster)
            }
            Some(Preset::Photo) => vtracer::Config::from_preset(vtracer::Preset::Photo),
            None => vtracer::Config::default(),
        };

        if let Some(clustering) = self.clustering {
            config.clustering = clustering.into();
        }
        if let Some(hierarchical) = self.hierarchical {
            config.hierarchical = hierarchical.into();
        }
        if let Some(mode) = self.mode {
            config.mode = mode.into();
        }
        if let Some(filter_speckle) = self.filter_speckle {
            config.filter_speckle = usize::from(filter_speckle);
        }
        if let Some(color_precision) = self.color_precision {
            config.color_precision = i32::from(color_precision);
        }
        if let Some(gradient_step) = self.gradient_step {
            config.layer_difference = i32::from(gradient_step);
        }
        if let Some(corner_threshold) = self.corner_threshold {
            config.corner_threshold = i32::from(corner_threshold);
        }
        if let Some(segment_length) = self.segment_length {
            config.length_threshold = segment_length;
        }
        if let Some(splice_threshold) = self.splice_threshold {
            config.splice_threshold = i32::from(splice_threshold);
        }
        if let Some(simplify) = self.simplify {
            config.simplify = Some(simplify);
        }
        if let Some(max_colors) = self.max_colors {
            config.max_colors = Some(usize::from(max_colors));
        }
        if let Some(palette) = self.palette.as_ref() {
            config.palette = palette
                .iter()
                .map(|color| vtracer::Color::new(color.r, color.g, color.b))
                .collect();
        }
        if let Some(threshold) = self.threshold {
            config.binary_threshold = threshold;
        }
        if let Some(adaptive) = self.adaptive {
            config.binary_adaptive = adaptive;
        }
        if let Some(watershed_detail) = self.watershed_detail {
            config.watershed_detail = watershed_detail;
        }

        config
    }
}

impl From<Clustering> for vtracer::Clustering {
    fn from(value: Clustering) -> Self {
        match value {
            Clustering::ColorCluster => Self::ColorCluster,
            Clustering::Binary => Self::Binary,
            Clustering::Watershed => Self::Watershed,
        }
    }
}

impl From<Hierarchical> for vtracer::Hierarchical {
    fn from(value: Hierarchical) -> Self {
        match value {
            Hierarchical::Stacked => Self::Stacked,
            Hierarchical::Cutout => Self::Cutout,
        }
    }
}

impl From<FitMode> for vtracer::FitMode {
    fn from(value: FitMode) -> Self {
        match value {
            FitMode::Pixel => Self::Pixel,
            FitMode::Polygon => Self::Polygon,
            FitMode::Spline => Self::Spline,
        }
    }
}

/// Why tracing failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TraceError {
    /// The image had no pixels.
    #[error("image is empty; there is nothing to trace")]
    EmptyImage,
    /// The run was cancelled.
    #[error("tracing was cancelled")]
    Cancelled,
    /// Anything else the engine reported.
    #[error("tracing failed: {0}")]
    Engine(String),
}

impl From<vtracer::Error> for TraceError {
    fn from(error: vtracer::Error) -> Self {
        match error {
            vtracer::Error::EmptyImage => Self::EmptyImage,
            vtracer::Error::Cancelled => Self::Cancelled,
            other => Self::Engine(other.to_string()),
        }
    }
}

/// Trace an image into the document IR.
///
/// # Errors
///
/// [`TraceError::EmptyImage`] for a zero-size image, [`TraceError::Cancelled`]
/// if the engine was stopped, and [`TraceError::Engine`] for anything else it
/// reports.
pub fn trace(image: &ColorImage, options: &TraceOptions) -> Result<VectorDoc, TraceError> {
    let mut config = options.to_config();
    // Keep the geometry precise enough for Phase 5 to fit primitives to it.
    config.path_precision = Some(INTERNAL_PRECISION);
    Ok(config.build()?.run(image)?)
}

impl fmt::Display for Preset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Bw => "bw",
            Self::Poster => "poster",
            Self::Photo => "photo",
            Self::Auto => "auto",
        };
        f.write_str(name)
    }
}

impl FromStr for Preset {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "bw" => Ok(Self::Bw),
            "poster" => Ok(Self::Poster),
            "photo" => Ok(Self::Photo),
            "auto" => Ok(Self::Auto),
            other => Err(format!("unknown preset {other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use std::collections::BTreeSet;

    use vtracer::ColorImage;
    use vtracer::ir::PathCmd;

    use super::{
        Clustering, FitMode, Hierarchical, INTERNAL_PRECISION, Preset, TraceError, TraceOptions,
        trace,
    };
    use crate::color::Rgb;

    fn image_from(
        width: usize,
        height: usize,
        paint: impl Fn(usize, usize) -> [u8; 4],
    ) -> ColorImage {
        let mut pixels = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&paint(x, y));
            }
        }
        ColorImage {
            pixels,
            width,
            height,
        }
    }

    fn paints(doc: &vtracer::ir::VectorDoc) -> BTreeSet<(u8, u8, u8)> {
        doc.shapes
            .iter()
            .map(|shape| {
                let color = shape.paint.color();
                (color.r, color.g, color.b)
            })
            .collect()
    }

    fn command_count(doc: &vtracer::ir::VectorDoc) -> usize {
        doc.shapes
            .iter()
            .flat_map(|shape| shape.path.subpaths.iter())
            .map(|subpath| subpath.commands.len())
            .sum()
    }

    // --- configuration mapping ----------------------------------------------

    #[test]
    fn config_from_default_options_equals_vtracer_default() {
        // Field by field, so bumping vtracer cannot silently move a default
        // out from under us.
        let ours = TraceOptions::default().to_config();
        let theirs = vtracer::Config::default();

        assert_eq!(ours.clustering, theirs.clustering);
        assert_eq!(ours.hierarchical, theirs.hierarchical);
        assert_eq!(ours.filter_speckle, theirs.filter_speckle);
        assert_eq!(ours.color_precision, theirs.color_precision);
        assert_eq!(ours.layer_difference, theirs.layer_difference);
        assert_eq!(ours.mode, theirs.mode);
        assert_eq!(ours.corner_threshold, theirs.corner_threshold);
        assert!((ours.length_threshold - theirs.length_threshold).abs() < f64::EPSILON);
        assert_eq!(ours.max_iterations, theirs.max_iterations);
        assert_eq!(ours.splice_threshold, theirs.splice_threshold);
        assert_eq!(ours.simplify, theirs.simplify);
        assert_eq!(ours.path_precision, theirs.path_precision);
        assert_eq!(ours.palette, theirs.palette);
        assert_eq!(ours.max_colors, theirs.max_colors);
        assert_eq!(ours.optimize, theirs.optimize);
        assert_eq!(ours.binary_threshold, theirs.binary_threshold);
        assert_eq!(ours.binary_adaptive, theirs.binary_adaptive);
        assert_eq!(ours.binary_adaptive_window, theirs.binary_adaptive_window);
        assert!((ours.binary_adaptive_t - theirs.binary_adaptive_t).abs() < f64::EPSILON);
        assert_eq!(ours.watershed_detail, theirs.watershed_detail);
    }

    #[test]
    fn config_preset_poster_sets_expected_fields() {
        let config = TraceOptions {
            preset: Some(Preset::Poster),
            ..TraceOptions::default()
        }
        .to_config();

        assert_eq!(config.color_precision, 8, "poster widens the palette");
        assert_eq!(config.clustering, vtracer::Clustering::ColorCluster);
        assert_eq!(config.filter_speckle, 4, "everything else stays default");
    }

    #[test]
    fn config_preset_bw_and_photo_set_expected_fields() {
        let bw = TraceOptions {
            preset: Some(Preset::Bw),
            ..TraceOptions::default()
        }
        .to_config();
        assert_eq!(bw.clustering, vtracer::Clustering::Binary);

        let photo = TraceOptions {
            preset: Some(Preset::Photo),
            ..TraceOptions::default()
        }
        .to_config();
        assert_eq!(photo.filter_speckle, 10);
        assert_eq!(photo.layer_difference, 48);
        assert_eq!(photo.corner_threshold, 180);
    }

    #[test]
    fn config_preset_auto_resolves_to_poster() {
        let auto = TraceOptions {
            preset: Some(Preset::Auto),
            ..TraceOptions::default()
        }
        .to_config();
        let poster = TraceOptions {
            preset: Some(Preset::Poster),
            ..TraceOptions::default()
        }
        .to_config();
        assert_eq!(auto.color_precision, poster.color_precision);
        assert_eq!(auto.clustering, poster.clustering);
    }

    #[test]
    fn config_explicit_field_overrides_preset() {
        let config = TraceOptions {
            preset: Some(Preset::Photo),
            filter_speckle: Some(2),
            corner_threshold: Some(30),
            ..TraceOptions::default()
        }
        .to_config();

        assert_eq!(config.filter_speckle, 2, "explicit beats the preset");
        assert_eq!(config.corner_threshold, 30);
        assert_eq!(
            config.layer_difference, 48,
            "untouched preset fields remain"
        );
    }

    #[test]
    fn config_maps_every_renamed_field() {
        let config = TraceOptions {
            clustering: Some(Clustering::Watershed),
            hierarchical: Some(Hierarchical::Cutout),
            mode: Some(FitMode::Polygon),
            gradient_step: Some(33),
            segment_length: Some(7.5),
            splice_threshold: Some(21),
            simplify: Some(1.5),
            max_colors: Some(12),
            threshold: Some(77),
            adaptive: Some(true),
            watershed_detail: Some(200),
            ..TraceOptions::default()
        }
        .to_config();

        assert_eq!(config.clustering, vtracer::Clustering::Watershed);
        assert_eq!(config.hierarchical, vtracer::Hierarchical::Cutout);
        assert_eq!(config.mode, vtracer::FitMode::Polygon);
        assert_eq!(
            config.layer_difference, 33,
            "gradient_step is layer_difference"
        );
        assert!((config.length_threshold - 7.5).abs() < f64::EPSILON);
        assert_eq!(config.splice_threshold, 21);
        assert_eq!(config.simplify, Some(1.5));
        assert_eq!(config.max_colors, Some(12));
        assert_eq!(config.binary_threshold, 77, "threshold is binary_threshold");
        assert!(config.binary_adaptive, "adaptive is binary_adaptive");
        assert_eq!(config.watershed_detail, 200);
    }

    #[test]
    fn config_palette_parses_hex_with_and_without_hash_and_rejects_bad() {
        let palette = crate::color::parse_palette("#ff0000, 00ff00\n#00f").expect("valid");
        assert_eq!(
            palette,
            [
                Rgb::new(255, 0, 0),
                Rgb::new(0, 255, 0),
                Rgb::new(0, 0, 255)
            ]
        );

        let config = TraceOptions {
            palette: Some(palette),
            ..TraceOptions::default()
        }
        .to_config();
        assert_eq!(
            config.palette,
            vec![
                vtracer::Color::new(255, 0, 0),
                vtracer::Color::new(0, 255, 0),
                vtracer::Color::new(0, 0, 255),
            ]
        );

        assert!(crate::color::parse_palette("#ff0000, nope").is_err());
        assert!(crate::color::parse_palette("#12345").is_err());
    }

    #[test]
    fn config_empty_palette_is_not_forwarded_as_a_palette() {
        // An empty `Vec` means "no fixed palette" to vtracer, which is exactly
        // what an empty `--palette` should mean.
        let config = TraceOptions {
            palette: Some(Vec::new()),
            max_colors: Some(4),
            ..TraceOptions::default()
        }
        .to_config();
        assert!(config.palette.is_empty());
        assert_eq!(config.max_colors, Some(4));
    }

    // --- tracing -------------------------------------------------------------

    #[test]
    fn trace_raises_path_precision_above_the_vtracer_default() {
        let image = image_from(8, 8, |_, _| [200, 30, 40, 255]);
        // The override is invisible in the IR, so assert it on the config the
        // way `trace` builds it.
        assert_eq!(TraceOptions::default().to_config().path_precision, Some(2));
        let doc = trace(&image, &TraceOptions::default()).expect("traces");
        assert!(!doc.shapes.is_empty());
        assert_eq!(INTERNAL_PRECISION, 6);
    }

    #[test]
    fn trace_solid_color_image_yields_one_shape_one_color() {
        let image = image_from(16, 16, |_, _| [200, 30, 40, 255]);
        let doc = trace(&image, &TraceOptions::default()).expect("traces");

        assert_eq!(doc.shapes.len(), 1);
        assert_eq!(paints(&doc), BTreeSet::from([(200, 30, 40)]));
        assert_eq!((doc.width, doc.height), (16, 16));
    }

    #[test]
    fn trace_two_rects_side_by_side_yields_two_shapes_two_colors() {
        let image = image_from(32, 16, |x, _| {
            if x < 16 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        });
        let doc = trace(&image, &TraceOptions::default()).expect("traces");

        assert_eq!(doc.shapes.len(), 2);
        assert_eq!(paints(&doc), BTreeSet::from([(255, 0, 0), (0, 0, 255)]));
    }

    #[test]
    fn trace_max_colors_2_on_four_color_image_yields_at_most_two_paints() {
        let image = image_from(32, 32, |x, y| match (x < 16, y < 16) {
            (true, true) => [255, 0, 0, 255],
            (false, true) => [0, 255, 0, 255],
            (true, false) => [0, 0, 255, 255],
            (false, false) => [255, 255, 0, 255],
        });

        let unlimited = trace(&image, &TraceOptions::default()).expect("traces");
        assert_eq!(paints(&unlimited).len(), 4);

        let limited = trace(
            &image,
            &TraceOptions {
                max_colors: Some(2),
                ..TraceOptions::default()
            },
        )
        .expect("traces");
        assert!(
            paints(&limited).len() <= 2,
            "expected at most two paints, got {:?}",
            paints(&limited)
        );
    }

    #[test]
    fn trace_simplify_reduces_command_count_vs_unsimplified() {
        // A disc: a spline fit of a curved outline has room to be simplified.
        let image = image_from(64, 64, |x, y| {
            let dx = f64::from(u32::try_from(x).unwrap_or(u32::MAX)) - 31.5;
            let dy = f64::from(u32::try_from(y).unwrap_or(u32::MAX)) - 31.5;
            if dx.mul_add(dx, dy * dy) < 26.0 * 26.0 {
                [255, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        });

        let plain = trace(&image, &TraceOptions::default()).expect("traces");
        let simplified = trace(
            &image,
            &TraceOptions {
                simplify: Some(2.0),
                ..TraceOptions::default()
            },
        )
        .expect("traces");

        assert!(
            command_count(&simplified) < command_count(&plain),
            "simplify must shrink the path: {} then {}",
            command_count(&plain),
            command_count(&simplified)
        );
    }

    #[test]
    fn trace_transparent_background_does_not_produce_background_shape() {
        // The decode stage leaves `a = 0` in place precisely so the tracer can
        // key it out. Confirm it still does.
        let image = image_from(64, 64, |x, y| {
            if (16..48).contains(&x) && (16..48).contains(&y) {
                [255, 0, 0, 255]
            } else {
                [255, 255, 255, 0]
            }
        });
        let doc = trace(&image, &TraceOptions::default()).expect("traces");

        assert_eq!(doc.shapes.len(), 1, "only the square survives");
        assert_eq!(paints(&doc), BTreeSet::from([(255, 0, 0)]));
    }

    #[test]
    fn trace_1x1_image_does_not_panic() {
        let image = image_from(1, 1, |_, _| [1, 2, 3, 255]);
        let doc = trace(&image, &TraceOptions::default()).expect("traces");
        assert!(
            doc.shapes.is_empty(),
            "a single pixel is below the speckle filter"
        );
    }

    #[test]
    fn trace_empty_image_is_a_typed_error() {
        let image = ColorImage {
            pixels: Vec::new(),
            width: 0,
            height: 0,
        };
        assert_eq!(
            trace(&image, &TraceOptions::default()).err(),
            Some(TraceError::EmptyImage)
        );
    }

    #[test]
    fn trace_binary_preset_produces_a_monochrome_document() {
        let image = image_from(32, 32, |x, _| {
            if x < 16 {
                [10, 10, 10, 255]
            } else {
                [250, 250, 250, 255]
            }
        });
        let doc = trace(
            &image,
            &TraceOptions {
                preset: Some(Preset::Bw),
                ..TraceOptions::default()
            },
        )
        .expect("traces");

        assert!(!doc.shapes.is_empty());
        for color in paints(&doc) {
            assert!(
                color == (0, 0, 0) || color == (255, 255, 255),
                "binary mode paints black or white, got {color:?}"
            );
        }
    }

    #[test]
    fn trace_pixel_mode_emits_only_straight_segments() {
        let image = image_from(32, 32, |x, y| {
            if (8..24).contains(&x) && (8..24).contains(&y) {
                [255, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        });
        let doc = trace(
            &image,
            &TraceOptions {
                mode: Some(FitMode::Pixel),
                ..TraceOptions::default()
            },
        )
        .expect("traces");

        for shape in &doc.shapes {
            for subpath in &shape.path.subpaths {
                for command in &subpath.commands {
                    assert!(
                        !matches!(command, PathCmd::CubicTo(..)),
                        "pixel mode never fits a curve"
                    );
                }
            }
        }
    }

    #[test]
    fn preset_round_trips_through_its_string_form() {
        for preset in [Preset::Bw, Preset::Poster, Preset::Photo, Preset::Auto] {
            assert_eq!(
                preset.to_string().parse::<Preset>().expect("round trip"),
                preset
            );
        }
        assert!("nope".parse::<Preset>().is_err());
    }
}
