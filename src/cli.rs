//! Command-line surface: `clap` definitions only, no logic.
//!
//! Every field here is parsed data. Turning it into a plan, a tracing config,
//! or a writer setting happens in the modules that own those concerns.

use std::ffi::OsString;

use clap::Parser;

/// Convert raster images into maximally compact SVG.
///
/// Phase 1 exposes only the argument skeleton. Later phases fill in the option
/// groups described in `IMPLEMENTATION_PLAN.md` §5.
///
/// # Examples
///
/// ```
/// use clap::Parser as _;
/// use vectorise::cli::Cli;
///
/// let cli = Cli::try_parse_from(["vectorise", "logo.png"]).expect("valid");
/// assert_eq!(cli.inputs.len(), 1);
/// ```
#[derive(Debug, Parser)]
#[command(name = "vectorise", version, about, long_about = None)]
pub struct Cli {
    /// Raster files.
    ///
    /// Globs are normally expanded by your shell; a literal pattern (e.g.
    /// quoted `'**/*.png'`) is expanded by vectorise itself.
    #[arg(value_name = "INPUT", required = true)]
    pub inputs: Vec<OsString>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use clap::Parser as _;

    use super::Cli;

    #[test]
    fn cli_accepts_one_or_more_inputs() {
        let cli = Cli::try_parse_from(["vectorise", "a.png", "b.jpg"]).expect("parses");
        assert_eq!(cli.inputs, ["a.png", "b.jpg"]);
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
}
