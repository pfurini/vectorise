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
use crate::trace::{Clustering, FitMode, Hierarchical, Preset, TraceOptions};

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
            // Not on the command line in v1; see IMPLEMENTATION_PLAN.md §5.
            corner_threshold: None,
            segment_length: None,
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
    fn cli_defaults_leave_plan_options_empty() {
        let cli = Cli::try_parse_from(["vectorise", "a.png"]).expect("parses");
        assert_eq!(cli.plan_options(), crate::plan::PlanOptions::default());
    }
}
