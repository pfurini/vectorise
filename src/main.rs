//! The `vectorise` binary: parse arguments, call the library, map the outcome
//! to an exit code. No business logic lives here.

use std::io::IsTerminal as _;
use std::process::ExitCode as ProcessExitCode;

use clap::Parser as _;
use clap::error::ErrorKind;
use vectorise::cli::Cli;
use vectorise::error::ExitCode;
use vectorise::plan::{Plan, PlanError};
use vectorise::{Options, RunReport};

fn main() -> ProcessExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => return report_parse_error(&err),
    };
    install_logging(&cli);

    let options = match cli.options() {
        Ok(options) => options,
        Err(err) => {
            eprintln!("error: {err}");
            return Cli::exit_code_for(&err).into();
        }
    };

    let plan = match vectorise::plan(&cli.inputs, &cli.plan_options()) {
        Ok(plan) => plan,
        Err(err) => return report_plan_error(&err),
    };

    if cli.dry_run {
        print_mapping(&plan);
        return ExitCode::Ok.into();
    }

    convert(&plan, &options, &cli)
}

/// Run the batch and report it.
fn convert(plan: &Plan, options: &Options, cli: &Cli) -> ProcessExitCode {
    let report = vectorise::run(plan, options, cli.job_count());
    for (job, error) in &report.failed {
        eprintln!("error: {}: {error}", job.input.display());
    }
    if !cli.quiet {
        summarize(&report);
    }
    ExitCode::from(&report).into()
}

/// One closing line on stderr, so a batch says what it did without a progress
/// bar and without a dependency to draw one.
fn summarize(report: &RunReport) {
    let converted = report.ok.len();
    let failed = report.failed.len();
    if failed == 0 {
        eprintln!("info: converted {converted} file(s)");
    } else {
        eprintln!("info: converted {converted} file(s), {failed} failed");
    }
}

/// Send `tracing` output to stderr at the level the flags ask for.
fn install_logging(cli: &Cli) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(cli.log_level())
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .without_time()
        .with_target(false)
        .try_init();
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

/// List every reason the batch was rejected, one machine-readable line each.
fn report_plan_error(err: &PlanError) -> ProcessExitCode {
    for problem in &err.problems {
        eprintln!("error: {problem}");
    }
    ExitCode::Preflight.into()
}

/// The input-to-output plan, one line per job, on stdout so it can be piped.
fn print_mapping(plan: &Plan) {
    for job in &plan.jobs {
        println!("{}\t{}", job.input.display(), job.output.display());
    }
}
