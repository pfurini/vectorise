//! Integration tests that drive the `vectorise` binary the way a user does.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use assert_cmd::Command;
use predicates::prelude::*;

fn vectorise() -> Command {
    Command::cargo_bin("vectorise").expect("binary builds")
}

#[test]
fn binary_runs_and_prints_version() {
    vectorise()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn no_args_is_usage_error() {
    vectorise()
        .assert()
        .code(64)
        .stderr(predicate::str::contains("Usage:"));
}
