//! Integration tests that drive the `vectorise` binary the way a user does.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;

fn vectorise() -> Command {
    Command::cargo_bin("vectorise").expect("binary builds")
}

/// Create `files` (empty) inside a fresh temporary directory.
fn fixture(files: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for file in files {
        let path = dir.path().join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(&path, b"").expect("write");
    }
    dir
}

/// Every path under `dir`, relative to it, so two listings compare cleanly.
fn listing(dir: &Path) -> BTreeSet<PathBuf> {
    fn walk(dir: &Path, root: &Path, into: &mut BTreeSet<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            into.insert(path.strip_prefix(root).expect("under root").to_path_buf());
            if path.is_dir() {
                walk(&path, root, into);
            }
        }
    }
    let mut paths = BTreeSet::new();
    walk(dir, dir, &mut paths);
    paths
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

#[test]
fn cli_dry_run_prints_mapping_and_exits_zero_without_writing() {
    let dir = fixture(&["one.png", "two.jpg"]);
    let before = listing(dir.path());

    let assert = vectorise()
        .current_dir(dir.path())
        .args(["--dry-run", "one.png", "two.jpg"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    assert_eq!(
        stdout, "one.png\tone.svg\ntwo.jpg\ttwo.svg\n",
        "one job per line, input then output, tab separated"
    );
    assert_eq!(listing(dir.path()), before, "--dry-run writes nothing");
}

#[test]
fn cli_dry_run_honours_output_dir() {
    let dir = fixture(&["nested/one.png"]);
    vectorise()
        .current_dir(dir.path())
        .args(["--dry-run", "--output-dir", "out", "nested/one.png"])
        .assert()
        .success()
        .stdout("nested/one.png\tout/one.svg\n");
}

#[test]
fn cli_preflight_failure_exits_2_and_lists_every_problem_on_stderr() {
    // Three problems at once: an existing output, a duplicate output, and a
    // missing input.
    let dir = fixture(&["one.png", "one.svg", "x.png", "x.jpg"]);
    let before = listing(dir.path());

    let assert = vectorise()
        .current_dir(dir.path())
        .args(["one.png", "x.png", "x.jpg", "absent.png"])
        .assert()
        .code(2);

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    let kinds: Vec<&str> = stderr
        .lines()
        .map(|line| {
            line.strip_prefix("error: ")
                .expect("every problem line is prefixed")
                .split('\t')
                .next()
                .expect("a kind")
        })
        .collect();
    assert_eq!(
        kinds,
        ["input-not-found", "output-exists", "duplicate-output"],
        "all three problems are reported, not just the first: {stderr}"
    );
    assert_eq!(
        listing(dir.path()),
        before,
        "a rejected batch writes nothing"
    );
}

#[test]
fn cli_problem_output_is_stable_and_machine_readable() {
    let dir = fixture(&["x.png", "x.jpg"]);

    let assert = vectorise()
        .current_dir(dir.path())
        .args(["x.jpg", "x.png"])
        .assert()
        .code(2);

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert_eq!(stderr, "error: duplicate-output\tx.svg\tx.jpg\tx.png\n");
}

#[test]
fn cli_force_accepts_an_existing_output() {
    let dir = fixture(&["one.png", "one.svg"]);
    vectorise()
        .current_dir(dir.path())
        .args(["--force", "--dry-run", "one.png"])
        .assert()
        .success()
        .stdout("one.png\tone.svg\n");
}

#[test]
fn cli_rejects_a_directory_argument() {
    let dir = fixture(&["sub/one.png"]);
    vectorise()
        .current_dir(dir.path())
        .args(["--dry-run", "sub"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("input-is-directory\tsub"));
}

#[test]
fn cli_expands_an_unexpanded_glob_itself() {
    let dir = fixture(&["a/one.png", "a/two.png", "a/note.txt"]);
    vectorise()
        .current_dir(dir.path())
        .args(["--dry-run", "a/*.png"])
        .assert()
        .success()
        .stdout("a/one.png\ta/one.svg\na/two.png\ta/two.svg\n");
}
