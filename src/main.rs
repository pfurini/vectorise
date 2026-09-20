//! The `vectorise` binary: parse arguments, call the library, map the outcome
//! to an exit code. No business logic lives here.

use std::process::ExitCode as ProcessExitCode;

use clap::Parser as _;
use clap::error::ErrorKind;
use vectorise::cli::Cli;
use vectorise::error::ExitCode;
use vectorise::plan::{Plan, PlanError};

fn main() -> ProcessExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => return report_parse_error(&err),
    };

    let plan = match vectorise::plan(&cli.inputs, &cli.plan_options()) {
        Ok(plan) => plan,
        Err(err) => return report_plan_error(&err),
    };

    print_mapping(&plan);
    if !cli.dry_run {
        // Phase 8 replaces this with `vectorise::run(&plan, ...)`.
        eprintln!("info: conversion is wired up in Phase 8; nothing was written");
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
