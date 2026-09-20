//! Command-line surface: `clap` definitions only, no logic.
//!
//! Every field here is parsed data. Turning it into a plan, a tracing config,
//! or a writer setting happens in the modules that own those concerns.

use std::path::PathBuf;

use clap::Parser;

use crate::plan::PlanOptions;

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
    fn cli_defaults_leave_plan_options_empty() {
        let cli = Cli::try_parse_from(["vectorise", "a.png"]).expect("parses");
        assert_eq!(cli.plan_options(), crate::plan::PlanOptions::default());
    }
}
