//! The `vectorise` binary: parse arguments, call the library, map the outcome
//! to an exit code. No business logic lives here.

use std::process::ExitCode as ProcessExitCode;

use clap::Parser as _;
use clap::error::ErrorKind;
use vectorise::cli::Cli;
use vectorise::error::ExitCode;

fn main() -> ProcessExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => return report_parse_error(&err),
    };

    // Phase 2 replaces this with `vectorise::plan(...)` and `vectorise::run(...)`.
    for input in &cli.inputs {
        eprintln!("info: {} (not converted yet: Phase 2)", input.display());
    }
    ExitCode::Ok.into()
}

/// Print a clap error the way the user expects and choose its exit code.
///
/// `--help` and `--version` are not failures: clap reports them as errors, they
/// go to stdout, and the process exits 0. Everything else is a usage error on
/// stderr, exit 64 (BSD `EX_USAGE`), not clap's default 2.
fn report_parse_error(err: &clap::Error) -> ProcessExitCode {
    let _ = err.print();
    match err.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => ExitCode::Ok.into(),
        _ => ExitCode::Usage.into(),
    }
}
