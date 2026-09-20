//! Command-line surface: `clap` definitions and the thin accessors that
//! project them onto the option structs each stage takes.
//!
//! No decisions are made here. The one thing that is not pure is
//! [`Cli::trace_options`], which reads `--palette-file` from disk, because a
//! file path is an argument like any other and resolving it anywhere else
//! would spread the palette across two modules.

use std::path::PathBuf;

use clap::Parser;

use crate::color::{ParseRgbError, Rgb, parse_palette};
use crate::decode::DecodeOptions;
use crate::plan::PlanOptions;
use crate::shapes::{Detect, ShapeFitOptions};
use crate::trace::{Clustering, FitMode, Hierarchical, Preset, TraceOptions};
use crate::writer::{MAX_PRECISION, WriterOptions};

/// Convert raster images into maximally compact SVG.
///
/// Phase 2 exposes the argument skeleton plus the output group. Later phases
/// fill in tracing, shape detection, and reporting per
/// `IMPLEMENTATION_PLAN.md` §5.
///
/// # Examples
///
/// ```
/// use clap::Parser as _;
/// use vectorise::cli::Cli;
///
/// let cli = Cli::try_parse_from(["vectorise", "-n", "logo.png"]).expect("valid");
/// assert_eq!(cli.inputs.len(), 1);
/// assert!(cli.dry_run);
/// ```
// Flags are flags: a CLI struct is exactly the place for a row of booleans.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Parser)]
#[command(name = "vectorise", version, about, long_about = None)]
pub struct Cli {
    /// Raster files.
    ///
    /// Globs are normally expanded by your shell; a literal pattern (e.g.
    /// quoted `'**/*.png'`) is expanded by vectorise itself.
    #[arg(value_name = "INPUT", required = true)]
    pub inputs: Vec<PathBuf>,

    /// Write all outputs into DIR (flattened). Default: next to each input.
    #[arg(short, long, value_name = "DIR", help_heading = "Output")]
    pub output_dir: Option<PathBuf>,

    /// Overwrite existing outputs. Duplicate outputs still fail.
    #[arg(short, long, help_heading = "Output")]
    pub force: bool,

    /// Print the input to output plan and exit.
    #[arg(short = 'n', long, help_heading = "Output")]
    pub dry_run: bool,

    /// Emit width and height on the root element as well as viewBox.
    #[arg(long, help_heading = "Output")]
    pub keep_size: bool,

    /// Color that transparency is resolved against, as `#rrggbb` or `#rgb`.
    ///
    /// Fully transparent pixels are dropped by the tracer, so this mainly
    /// decides what semi-transparent pixels become.
    #[arg(long, value_name = "COLOR", default_value_t = Rgb::WHITE, help_heading = "Tracing")]
    pub background: Rgb,

    /// Starting point tuned for a kind of input.
    #[arg(long, value_enum, default_value_t = Preset::Auto, help_heading = "Tracing")]
    pub preset: Preset,

    /// Region-forming algorithm.
    #[arg(long, value_enum, help_heading = "Tracing")]
    pub clustering: Option<Clustering>,

    /// Stacked layers or a seam-free mosaic.
    #[arg(long, value_enum, help_heading = "Tracing")]
    pub hierarchical: Option<Hierarchical>,

    /// Curve-fitting mode.
    #[arg(short, long, value_enum, help_heading = "Tracing")]
    pub mode: Option<FitMode>,

    /// Discard speckles smaller than this side length, in pixels.
    #[arg(long, value_name = "PX", value_parser = clap::value_parser!(u8).range(0..=128), help_heading = "Tracing")]
    pub filter_speckle: Option<u8>,

    /// Significant bits per RGB channel.
    #[arg(long, value_name = "BITS", value_parser = clap::value_parser!(u8).range(1..=8), help_heading = "Tracing")]
    pub color_precision: Option<u8>,

    /// Color difference between gradient layers.
    #[arg(long, value_name = "N", help_heading = "Tracing")]
    pub gradient_step: Option<u8>,

    /// Quantize to at most this many colors.
    #[arg(long, value_name = "N", help_heading = "Tracing")]
    pub max_colors: Option<u16>,

    /// Fixed palette, as comma-separated hex colors.
    #[arg(
        long,
        value_name = "COLORS",
        conflicts_with = "palette_file",
        help_heading = "Tracing"
    )]
    pub palette: Option<String>,

    /// Fixed palette, read from a file of hex colors.
    #[arg(long, value_name = "FILE", help_heading = "Tracing")]
    pub palette_file: Option<PathBuf>,

    /// Curve simplification tolerance in pixels. Try 1 to 2.5.
    #[arg(long, value_name = "PX", help_heading = "Tracing")]
    pub simplify: Option<f64>,

    /// Black-and-white cutoff for `--clustering bw`.
    #[arg(
        long,
        value_name = "N",
        conflicts_with = "adaptive",
        help_heading = "Tracing"
    )]
    pub threshold: Option<u8>,

    /// Use adaptive thresholding instead of a fixed cutoff.
    #[arg(long, help_heading = "Tracing")]
    pub adaptive: bool,

    /// Where to cut the watershed hierarchy. Higher keeps more regions.
    #[arg(long, value_name = "N", help_heading = "Tracing")]
    pub watershed_detail: Option<u32>,

    /// Corner threshold in degrees. Higher smooths through sharper turns.
    #[arg(long, value_name = "DEG", help_heading = "Tracing")]
    pub corner_threshold: Option<u16>,

    /// Segment length threshold in pixels.
    ///
    /// Lower it to fit large smooth curves more closely; the default leaves a
    /// 400 by 100 oval several percent too fat to be recognized as an ellipse.
    #[arg(long, value_name = "PX", help_heading = "Tracing")]
    pub segment_length: Option<f64>,

    /// Detect native SVG shapes, or leave every region a path.
    #[arg(long, value_enum, default_value_t = Shapes::Auto, help_heading = "Shapes")]
    pub shapes: Shapes,

    /// Allowed difference between a shape and the outline it replaces, as a
    /// fraction of the region's area.
    #[arg(
        long,
        value_name = "FRACTION",
        default_value_t = 0.02,
        help_heading = "Shapes"
    )]
    pub shape_tolerance: f64,

    /// Regions smaller than this many square pixels stay paths.
    #[arg(
        long,
        value_name = "PX2",
        default_value_t = 16.0,
        help_heading = "Shapes"
    )]
    pub min_shape_area: f64,

    /// Keep rotated ellipses as paths instead of emitting a rotation.
    #[arg(long, help_heading = "Shapes")]
    pub no_rotated_ellipses: bool,

    /// Skip the optimizer pass.
    ///
    /// The writer already emits minimal markup; the optimizer rewrites path
    /// data on top of that. Turn it off to keep the output exactly as the
    /// writer produced it.
    #[arg(long, help_heading = "Output quality")]
    pub no_optimize: bool,

    /// Decimal places kept on every coordinate.
    #[arg(
        short,
        long,
        value_name = "N",
        default_value_t = 2,
        value_parser = clap::value_parser!(u8).range(0..=MAX_PRECISION as i64),
        help_heading = "Output quality"
    )]
    pub precision: u8,
}

/// Whether shape detection runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Shapes {
    /// Detect circles, ellipses, and rectangles.
    #[default]
    Auto,
    /// Emit every region as a path.
    Off,
}

/// Why the tracing options could not be assembled from the command line.
#[derive(Debug, thiserror::Error)]
pub enum TraceOptionsError {
    /// `--palette-file` could not be read.
    #[error("--palette-file {path}: {source}")]
    PaletteFile {
        /// The file we tried to read.
        path: PathBuf,
        /// The underlying I/O failure.
        source: std::io::Error,
    },
    /// A palette entry is not a hex color.
    #[error("{source}")]
    Palette {
        /// What the parser objected to.
        #[from]
        source: ParseRgbError,
    },
}

impl Cli {
    /// The planning-relevant projection of these arguments.
    ///
    /// # Examples
    ///
    /// ```
    /// use clap::Parser as _;
    /// use vectorise::cli::Cli;
    ///
    /// let cli = Cli::try_parse_from(["vectorise", "-f", "logo.png"]).expect("valid");
    /// assert!(cli.plan_options().force);
    /// ```
    pub fn plan_options(&self) -> PlanOptions {
        PlanOptions {
            output_dir: self.output_dir.clone(),
            force: self.force,
        }
    }

    /// The decoding-relevant projection of these arguments.
    ///
    /// # Examples
    ///
    /// ```
    /// use clap::Parser as _;
    /// use vectorise::cli::Cli;
    /// use vectorise::color::Rgb;
    ///
    /// let cli = Cli::try_parse_from(["vectorise", "--background", "#000", "logo.png"])
    ///     .expect("valid");
    /// assert_eq!(cli.decode_options().background, Rgb::BLACK);
    /// ```
    pub const fn decode_options(&self) -> DecodeOptions {
        DecodeOptions {
            background: self.background,
        }
    }

    /// The shape-detection projection of these arguments.
    ///
    /// # Examples
    ///
    /// ```
    /// use clap::Parser as _;
    /// use vectorise::cli::Cli;
    ///
    /// let cli = Cli::try_parse_from(["vectorise", "--shapes", "off", "a.png"])
    ///     .expect("valid");
    /// let options = cli.shape_options();
    /// assert!(!options.detect.circle);
    /// ```
    #[must_use]
    pub fn shape_options(&self) -> ShapeFitOptions {
        ShapeFitOptions {
            tolerance_frac: self.shape_tolerance,
            min_area: self.min_shape_area,
            allow_rotation: !self.no_rotated_ellipses,
            detect: match self.shapes {
                Shapes::Auto => Detect::default(),
                Shapes::Off => Detect::none(),
            },
        }
    }

    /// The serialization projection of these arguments.
    ///
    /// # Examples
    ///
    /// ```
    /// use clap::Parser as _;
    /// use vectorise::cli::Cli;
    ///
    /// let cli = Cli::try_parse_from(["vectorise", "-p", "4", "a.png"]).expect("valid");
    /// assert_eq!(cli.writer_options().precision, 4);
    /// ```
    #[must_use]
    pub const fn writer_options(&self) -> WriterOptions {
        WriterOptions {
            precision: self.precision,
            keep_size: self.keep_size,
        }
    }

    /// The tracing-relevant projection of these arguments.
    ///
    /// Reads `--palette-file` when one was given; `--palette` and
    /// `--palette-file` are mutually exclusive, so at most one is consulted.
    ///
    /// # Errors
    ///
    /// [`TraceOptionsError::PaletteFile`] if the file cannot be read, and
    /// [`TraceOptionsError::Palette`] if any entry is not a hex color.
    ///
    /// # Examples
    ///
    /// ```
    /// use clap::Parser as _;
    /// use vectorise::cli::Cli;
    /// use vectorise::trace::Preset;
    ///
    /// let cli = Cli::try_parse_from(["vectorise", "--preset", "photo", "a.png"])
    ///     .expect("valid");
    /// let options = cli.trace_options().expect("valid");
    /// assert_eq!(options.preset, Some(Preset::Photo));
    /// ```
    pub fn trace_options(&self) -> Result<TraceOptions, TraceOptionsError> {
        let palette = match (self.palette.as_deref(), self.palette_file.as_deref()) {
            (Some(text), _) => Some(parse_palette(text)?),
            (None, Some(path)) => {
                let text = std::fs::read_to_string(path).map_err(|source| {
                    TraceOptionsError::PaletteFile {
                        path: path.to_path_buf(),
                        source,
                    }
                })?;
                Some(parse_palette(&text)?)
            }
            (None, None) => None,
        };

        Ok(TraceOptions {
            preset: Some(self.preset),
            clustering: self.clustering,
            hierarchical: self.hierarchical,
            mode: self.mode,
            filter_speckle: self.filter_speckle,
            color_precision: self.color_precision,
            gradient_step: self.gradient_step,
            corner_threshold: self.corner_threshold,
            segment_length: self.segment_length,
            // Not on the command line in v1; see IMPLEMENTATION_PLAN.md §5.
            splice_threshold: None,
            simplify: self.simplify,
            max_colors: self.max_colors,
            palette,
            threshold: self.threshold,
            adaptive: self.adaptive.then_some(true),
            watershed_detail: self.watershed_detail,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use clap::Parser as _;

    use super::Cli;

    #[test]
    fn cli_accepts_one_or_more_inputs() {
        let cli = Cli::try_parse_from(["vectorise", "a.png", "b.jpg"]).expect("parses");
        assert_eq!(
            cli.inputs,
            [std::path::Path::new("a.png"), std::path::Path::new("b.jpg")]
        );
    }

    #[test]
    fn cli_without_inputs_is_an_error() {
        let err = Cli::try_parse_from(["vectorise"]).expect_err("inputs are required");
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn cli_definition_is_internally_consistent() {
        use clap::CommandFactory as _;
        Cli::command().debug_assert();
    }

    #[test]
    fn cli_output_flags_map_onto_plan_options() {
        let cli = Cli::try_parse_from(["vectorise", "--output-dir", "out", "--force", "a.png"])
            .expect("parses");
        let options = cli.plan_options();
        assert_eq!(
            options.output_dir.as_deref(),
            Some(std::path::Path::new("out"))
        );
        assert!(options.force);
        assert!(!cli.dry_run);
    }

    #[test]
    fn cli_background_defaults_to_white_and_parses_hex() {
        use crate::color::Rgb;

        let cli = Cli::try_parse_from(["vectorise", "a.png"]).expect("parses");
        assert_eq!(cli.decode_options().background, Rgb::WHITE);

        let cli =
            Cli::try_parse_from(["vectorise", "--background", "#3366ff", "a.png"]).expect("parses");
        assert_eq!(cli.decode_options().background, Rgb::new(0x33, 0x66, 0xff));
    }

    #[test]
    fn cli_rejects_a_malformed_background() {
        let err = Cli::try_parse_from(["vectorise", "--background", "nope", "a.png"])
            .expect_err("rejected");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_trace_options_default_to_the_auto_preset() {
        use crate::trace::Preset;

        let cli = Cli::try_parse_from(["vectorise", "a.png"]).expect("parses");
        let options = cli.trace_options().expect("valid");
        assert_eq!(options.preset, Some(Preset::Auto));
        assert_eq!(options.palette, None);
        assert_eq!(options.adaptive, None, "the flag is absent, not false");
    }

    #[test]
    fn cli_trace_options_carry_every_tracing_flag() {
        use crate::trace::{Clustering, FitMode, Hierarchical};

        let cli = Cli::try_parse_from([
            "vectorise",
            "--clustering",
            "watershed",
            "--hierarchical",
            "cutout",
            "--mode",
            "polygon",
            "--filter-speckle",
            "7",
            "--color-precision",
            "5",
            "--gradient-step",
            "24",
            "--max-colors",
            "9",
            "--simplify",
            "1.5",
            "--adaptive",
            "--watershed-detail",
            "200",
            "a.png",
        ])
        .expect("parses");
        let options = cli.trace_options().expect("valid");

        assert_eq!(options.clustering, Some(Clustering::Watershed));
        assert_eq!(options.hierarchical, Some(Hierarchical::Cutout));
        assert_eq!(options.mode, Some(FitMode::Polygon));
        assert_eq!(options.filter_speckle, Some(7));
        assert_eq!(options.color_precision, Some(5));
        assert_eq!(options.gradient_step, Some(24));
        assert_eq!(options.max_colors, Some(9));
        assert_eq!(options.simplify, Some(1.5));
        assert_eq!(options.adaptive, Some(true));
        assert_eq!(options.watershed_detail, Some(200));
    }

    #[test]
    fn cli_palette_is_parsed_from_the_command_line() {
        use crate::color::Rgb;

        let cli =
            Cli::try_parse_from(["vectorise", "--palette", "#f00,0f0", "a.png"]).expect("parses");
        assert_eq!(
            cli.trace_options().expect("valid").palette,
            Some(vec![Rgb::new(255, 0, 0), Rgb::new(0, 255, 0)])
        );
    }

    #[test]
    fn cli_palette_is_read_from_a_file() {
        use crate::color::Rgb;

        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("palette.txt");
        std::fs::write(&file, "#ff0000\n#00ff00\n").expect("write");

        let cli = Cli::try_parse_from([
            "vectorise".as_ref(),
            "--palette-file".as_ref(),
            file.as_os_str(),
            "a.png".as_ref(),
        ])
        .expect("parses");
        assert_eq!(
            cli.trace_options().expect("valid").palette,
            Some(vec![Rgb::new(255, 0, 0), Rgb::new(0, 255, 0)])
        );
    }

    #[test]
    fn cli_missing_palette_file_is_a_typed_error() {
        let cli = Cli::try_parse_from(["vectorise", "--palette-file", "no/such/file", "a.png"])
            .expect("parses");
        assert!(matches!(
            cli.trace_options(),
            Err(super::TraceOptionsError::PaletteFile { .. })
        ));
    }

    #[test]
    fn cli_bad_palette_entry_is_a_typed_error() {
        let cli =
            Cli::try_parse_from(["vectorise", "--palette", "#f00,nope", "a.png"]).expect("parses");
        assert!(matches!(
            cli.trace_options(),
            Err(super::TraceOptionsError::Palette { .. })
        ));
    }

    #[test]
    fn cli_rejects_conflicting_palette_and_threshold_flags() {
        for conflicting in [
            vec![
                "vectorise",
                "--palette",
                "#fff",
                "--palette-file",
                "p.txt",
                "a.png",
            ],
            vec!["vectorise", "--threshold", "10", "--adaptive", "a.png"],
        ] {
            let err = Cli::try_parse_from(conflicting.clone()).expect_err("rejected");
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "{conflicting:?}"
            );
        }
    }

    #[test]
    fn cli_rejects_out_of_range_tracing_values() {
        for bad in [
            vec!["vectorise", "--color-precision", "9", "a.png"],
            vec!["vectorise", "--color-precision", "0", "a.png"],
            vec!["vectorise", "--filter-speckle", "129", "a.png"],
        ] {
            let err = Cli::try_parse_from(bad.clone()).expect_err("rejected");
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ValueValidation,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn cli_shape_options_default_to_detecting_everything() {
        let cli = Cli::try_parse_from(["vectorise", "a.png"]).expect("parses");
        let options = cli.shape_options();
        assert_eq!(options, crate::shapes::ShapeFitOptions::default());
    }

    #[test]
    fn cli_shape_flags_reach_the_pass() {
        let cli = Cli::try_parse_from([
            "vectorise",
            "--shape-tolerance",
            "0.05",
            "--min-shape-area",
            "64",
            "--no-rotated-ellipses",
            "a.png",
        ])
        .expect("parses");
        let options = cli.shape_options();
        assert!((options.tolerance_frac - 0.05).abs() < f64::EPSILON);
        assert!((options.min_area - 64.0).abs() < f64::EPSILON);
        assert!(!options.allow_rotation);
        assert!(options.detect.circle, "detection is still on");
    }

    #[test]
    fn cli_shapes_off_disables_every_detector() {
        let cli = Cli::try_parse_from(["vectorise", "--shapes", "off", "a.png"]).expect("parses");
        assert_eq!(cli.shape_options().detect, crate::shapes::Detect::none());
    }

    #[test]
    fn cli_curve_fitting_flags_reach_the_tracer() {
        let cli = Cli::try_parse_from([
            "vectorise",
            "--corner-threshold",
            "120",
            "--segment-length",
            "1",
            "a.png",
        ])
        .expect("parses");
        let options = cli.trace_options().expect("valid");
        assert_eq!(options.corner_threshold, Some(120));
        assert_eq!(options.segment_length, Some(1.0));
    }

    #[test]
    fn cli_optimizer_runs_unless_told_not_to() {
        let cli = Cli::try_parse_from(["vectorise", "a.png"]).expect("parses");
        assert!(!cli.no_optimize);

        let cli = Cli::try_parse_from(["vectorise", "--no-optimize", "a.png"]).expect("parses");
        assert!(cli.no_optimize);
    }

    #[test]
    fn cli_writer_options_default_to_two_decimals_and_no_size() {
        let cli = Cli::try_parse_from(["vectorise", "a.png"]).expect("parses");
        assert_eq!(
            cli.writer_options(),
            crate::writer::WriterOptions::default()
        );
    }

    #[test]
    fn cli_writer_flags_reach_the_writer() {
        let cli = Cli::try_parse_from(["vectorise", "--precision", "0", "--keep-size", "a.png"])
            .expect("parses");
        let options = cli.writer_options();
        assert_eq!(options.precision, 0);
        assert!(options.keep_size);
    }

    #[test]
    fn cli_rejects_a_precision_above_the_cap() {
        let err =
            Cli::try_parse_from(["vectorise", "--precision", "7", "a.png"]).expect_err("rejected");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_defaults_leave_plan_options_empty() {
        let cli = Cli::try_parse_from(["vectorise", "a.png"]).expect("parses");
        assert_eq!(cli.plan_options(), crate::plan::PlanOptions::default());
    }
}
