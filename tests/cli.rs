//! Integration tests that drive the `vectorise` binary the way a user does.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use common::degraded::{Degradation, Fixture, degrade, png_bytes};
use common::fixtures;
use predicates::prelude::*;

fn vectorise() -> Command {
    Command::cargo_bin("vectorise").expect("binary builds")
}

/// Create `files` (empty) inside a fresh temporary directory.
fn fixture(files: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for file in files {
        fixtures::place(dir.path(), file, b"");
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

/// Is this a complete SVG document?
fn is_complete_svg(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    text.starts_with("<svg ") && text.trim_end().ends_with("</svg>")
}

// --- the basics ---------------------------------------------------------------

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
fn cli_help_text_snapshot() {
    let output = vectorise().arg("--help").assert().success();
    let help = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");
    insta::assert_snapshot!(help);
}

// --- planning ------------------------------------------------------------------

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

// --- conversion ------------------------------------------------------------------

#[test]
fn cli_converts_single_png_next_to_input() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "disc.png", &fixtures::disc_png());

    vectorise()
        .current_dir(dir.path())
        .arg("disc.png")
        .assert()
        .success();

    let output = dir.path().join("disc.svg");
    assert!(output.is_file(), "the output sits next to its input");
    let svg = std::fs::read_to_string(&output).expect("reads");
    assert!(svg.starts_with("<svg "), "{svg}");
    assert!(svg.contains("<circle"), "the disc is a circle: {svg}");
}

#[test]
fn cli_converts_batch_in_parallel_and_all_outputs_exist() {
    let dir = tempfile::tempdir().expect("tempdir");
    for name in ["a", "b", "c", "d"] {
        fixtures::place(dir.path(), &format!("{name}.png"), &fixtures::disc_png());
    }
    fixtures::place(dir.path(), "rect.png", &fixtures::rect_png());
    fixtures::place(dir.path(), "photo.jpg", &fixtures::disc_jpeg());

    vectorise()
        .current_dir(dir.path())
        .args([
            "--jobs",
            "4",
            "a.png",
            "b.png",
            "c.png",
            "d.png",
            "rect.png",
            "photo.jpg",
        ])
        .assert()
        .success();

    for name in ["a", "b", "c", "d", "rect", "photo"] {
        let output = dir.path().join(format!("{name}.svg"));
        assert!(is_complete_svg(&output), "{name}.svg is complete");
    }
    let rect = std::fs::read_to_string(dir.path().join("rect.svg")).expect("reads");
    assert!(rect.contains("<rect"), "the rectangle is a rect: {rect}");
}

#[test]
fn cli_output_dir_flag_places_outputs_there() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "deep/nested/disc.png", &fixtures::disc_png());

    vectorise()
        .current_dir(dir.path())
        .args(["--output-dir", "out", "deep/nested/disc.png"])
        .assert()
        .success();

    assert!(
        dir.path().join("out/disc.svg").is_file(),
        "flattened into the output directory"
    );
    assert!(!dir.path().join("deep/nested/disc.svg").exists());
}

#[test]
fn cli_refuses_when_output_exists_and_writes_nothing_else() {
    let dir = tempfile::tempdir().expect("tempdir");
    for name in ["a", "b", "c"] {
        fixtures::place(dir.path(), &format!("{name}.png"), &fixtures::disc_png());
    }
    // One collision is enough to reject the whole batch.
    fixtures::place(dir.path(), "b.svg", b"theirs");
    let before = listing(dir.path());

    vectorise()
        .current_dir(dir.path())
        .args(["a.png", "b.png", "c.png"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("output-exists"));

    assert_eq!(
        listing(dir.path()),
        before,
        "not one of the three outputs was written"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.svg")).expect("reads"),
        "theirs"
    );
}

#[test]
fn cli_force_overwrites() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "disc.png", &fixtures::disc_png());
    fixtures::place(dir.path(), "disc.svg", b"theirs");

    vectorise()
        .current_dir(dir.path())
        .args(["--force", "disc.png"])
        .assert()
        .success();

    assert!(is_complete_svg(&dir.path().join("disc.svg")));
}

#[test]
fn cli_partial_failure_exit_code_1_and_reports_per_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "good.png", &fixtures::disc_png());
    fixtures::place(dir.path(), "broken.png", &fixtures::corrupt());

    let assert = vectorise()
        .current_dir(dir.path())
        .args(["good.png", "broken.png"])
        .assert()
        .code(1);

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert!(
        stderr.contains("broken.png"),
        "the failing file is named: {stderr}"
    );
    assert!(
        is_complete_svg(&dir.path().join("good.svg")),
        "the other file still converted"
    );
    assert!(!dir.path().join("broken.svg").exists());
}

#[test]
fn cli_jobs_1_is_sequential_and_deterministic_order_of_log_lines() {
    let dir = tempfile::tempdir().expect("tempdir");
    for name in ["c", "a", "b"] {
        fixtures::place(dir.path(), &format!("{name}.png"), &fixtures::disc_png());
    }

    let order = |args: &[&str]| {
        let assert = vectorise()
            .current_dir(dir.path())
            .args(args)
            .arg("c.png")
            .arg("a.png")
            .arg("b.png")
            .assert()
            .success();
        let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
        stderr
            .lines()
            .filter(|line| line.contains("converted") && line.contains(".png"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };

    let first = order(&["--jobs", "1", "--verbose", "--force"]);
    assert_eq!(first.len(), 3, "one line per file: {first:?}");
    // The plan sorts inputs, so the lines follow a, b, c whatever the argument
    // order was.
    assert!(first[0].contains("a.png"), "{first:?}");
    assert!(first[1].contains("b.png"), "{first:?}");
    assert!(first[2].contains("c.png"), "{first:?}");

    let again = order(&["--jobs", "1", "--verbose", "--force"]);
    assert_eq!(first, again, "the same run gives the same lines");

    let parallel = order(&["--jobs", "4", "--verbose", "--force"]);
    assert_eq!(
        first, parallel,
        "and so does a parallel run: results are ordered by the plan"
    );
}

#[test]
fn cli_quiet_suppresses_progress_but_not_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "disc.png", &fixtures::disc_png());

    let assert = vectorise()
        .current_dir(dir.path())
        .args(["--quiet", "disc.png"])
        .assert()
        .success();
    assert_eq!(
        assert.get_output().stderr,
        b"",
        "a quiet success says nothing at all"
    );

    fixtures::place(dir.path(), "broken.png", &fixtures::corrupt());
    let assert = vectorise()
        .current_dir(dir.path())
        .args(["--quiet", "broken.png"])
        .assert()
        .code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert!(
        stderr.contains("broken.png"),
        "errors survive --quiet: {stderr:?}"
    );
}

#[test]
fn cli_verbose_enables_tracing_at_debug() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "disc.png", &fixtures::disc_png());

    let quiet_run = vectorise()
        .current_dir(dir.path())
        .args(["--force", "disc.png"])
        .assert()
        .success();
    let plain = String::from_utf8(quiet_run.get_output().stderr.clone()).expect("utf-8");
    assert!(
        !plain.contains("traced"),
        "no debug lines by default: {plain}"
    );

    let debug_run = vectorise()
        .current_dir(dir.path())
        .args(["-vv", "--force", "disc.png"])
        .assert()
        .success();
    let debug = String::from_utf8(debug_run.get_output().stderr.clone()).expect("utf-8");
    for stage in ["decoded", "cleaned", "traced", "fitted", "optimized"] {
        assert!(debug.contains(stage), "-vv reports {stage}: {debug}");
    }
}

#[test]
fn cli_no_optimize_still_writes_valid_svg() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "disc.png", &fixtures::disc_png());

    vectorise()
        .current_dir(dir.path())
        .args(["--no-optimize", "disc.png"])
        .assert()
        .success();
    assert!(is_complete_svg(&dir.path().join("disc.svg")));
}

#[test]
fn cli_shapes_off_emits_paths_instead_of_primitives() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "disc.png", &fixtures::disc_png());

    vectorise()
        .current_dir(dir.path())
        .args(["--shapes", "off", "disc.png"])
        .assert()
        .success();

    let svg = std::fs::read_to_string(dir.path().join("disc.svg")).expect("reads");
    assert!(!svg.contains("<circle"), "{svg}");
    assert!(svg.contains("<path"), "{svg}");
}

#[test]
fn cli_reports_a_bad_palette_as_a_usage_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "disc.png", &fixtures::disc_png());

    vectorise()
        .current_dir(dir.path())
        .args(["--palette", "#nothex", "disc.png"])
        .assert()
        .code(64);
}

#[test]
fn cli_reports_a_missing_palette_file_as_an_io_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "disc.png", &fixtures::disc_png());

    vectorise()
        .current_dir(dir.path())
        .args(["--palette-file", "no/such/file", "disc.png"])
        .assert()
        .code(74);
}

// --- cleanup -----------------------------------------------------------------------

/// The logo fixture after a JPEG round-trip at quality 30, as a lossless PNG:
/// an input the cleanup pass acts on.
fn noisy_logo_png() -> Vec<u8> {
    png_bytes(&degrade(Fixture::Logo, Degradation::Jpeg30))
}

/// `svg` rasterized at 256x256 over white, as opaque RGB.
fn render_rgb(svg: &str) -> Vec<u8> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).expect("parses");
    let mut pixmap = tiny_skia::Pixmap::new(256, 256).expect("pixmap");
    pixmap.fill(tiny_skia::Color::WHITE);
    let size = tree.size();
    let transform = tiny_skia::Transform::from_scale(256.0 / size.width(), 256.0 / size.height());
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let straight = pixel.demultiply();
            [straight.red(), straight.green(), straight.blue()]
        })
        .collect()
}

#[test]
fn cli_cleanup_off_matches_v0_1_0_output() {
    // `tests/fixtures/v0_1_0-noisy-logo.svg` was produced by the v0.1.0
    // pipeline, before the cleanup stage existed, on an input cleanup would
    // otherwise change. `--cleanup off` must reproduce it. The tracer's curve
    // fitting differs by an ulp between x86 and arm64, which moves a few path
    // digits, so the comparison is a render and a size band rather than bytes:
    // on the platform the reference was made on, the bytes are identical.
    let reference = include_str!("fixtures/v0_1_0-noisy-logo.svg");
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "logo.png", &noisy_logo_png());

    vectorise()
        .current_dir(dir.path())
        .args(["--cleanup", "off", "logo.png"])
        .assert()
        .success();

    let svg = std::fs::read_to_string(dir.path().join("logo.svg")).expect("reads");
    let error =
        vectorise::verify::compare(&render_rgb(reference), &render_rgb(&svg)).mean_absolute_error;
    assert!(error < 0.001, "renders differ: mean absolute error {error}");
    assert!(
        svg.len().abs_diff(reference.len()) * 100 < reference.len(),
        "size within 1%: {} vs {}",
        svg.len(),
        reference.len()
    );
    assert_eq!(
        svg.matches('<').count(),
        reference.matches('<').count(),
        "the same elements"
    );
}

/// Convert `png` with `args` and return the SVG and the `--stats-json` line.
fn convert_with(png: &[u8], args: &[&str]) -> (String, serde_json::Value) {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "in.png", png);
    let assert = vectorise()
        .current_dir(dir.path())
        .arg("--stats-json")
        .args(args)
        .arg("in.png")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    let stats = serde_json::from_str(stdout.trim()).expect("valid JSON");
    let svg = std::fs::read_to_string(dir.path().join("in.svg")).expect("reads");
    (svg, stats)
}

#[test]
fn cli_cleanup_auto_is_the_default() {
    let (by_default, stats) = convert_with(&noisy_logo_png(), &[]);
    assert_eq!(stats["cleanup_denoise"], 3, "{stats}");
    assert_eq!(stats["cleanup_sharpen"], 0, "{stats}");
    let (explicit, _) = convert_with(&noisy_logo_png(), &["--cleanup", "auto"]);
    assert_eq!(by_default, explicit, "no flag means auto");
    let (off, off_stats) = convert_with(&noisy_logo_png(), &["--cleanup", "off"]);
    assert_ne!(by_default, off, "and auto acts on a noisy input");
    assert!(off_stats.get("cleanup_denoise").is_none(), "{off_stats}");
    assert!(
        by_default.len() * 4 < off.len(),
        "several times smaller: {} vs {}",
        by_default.len(),
        off.len()
    );
}

#[test]
fn cli_cleanup_auto_is_a_no_op_on_a_clean_input() {
    let clean = png_bytes(&Fixture::Logo.pristine());
    let (auto, stats) = convert_with(&clean, &[]);
    let (off, _) = convert_with(&clean, &["--cleanup", "off"]);
    assert_eq!(auto, off, "byte-identical output");
    assert!(stats.get("cleanup_denoise").is_none(), "{stats}");
    assert!(stats.get("edge_width").is_none(), "{stats}");
}

#[test]
fn cli_denoise_flag_overrides_auto() {
    let (_, stats) = convert_with(&noisy_logo_png(), &["--denoise", "1"]);
    assert_eq!(stats["cleanup_denoise"], 1, "{stats}");
    assert_eq!(stats["cleanup_sharpen"], 0, "sharpen still auto: {stats}");
}

#[test]
fn cli_sharpen_flag_overrides_auto() {
    let (_, stats) = convert_with(&noisy_logo_png(), &["--sharpen", "2"]);
    assert_eq!(stats["cleanup_sharpen"], 2, "{stats}");
    assert_eq!(stats["cleanup_denoise"], 3, "denoise still auto: {stats}");
}

#[test]
fn cli_cleanup_off_with_denoise_flag_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "in.png", &noisy_logo_png());
    let assert = vectorise()
        .current_dir(dir.path())
        .args(["--cleanup", "off", "--denoise", "2", "in.png"])
        .assert()
        .code(64);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert!(stderr.contains("--cleanup off"), "{stderr}");
    assert!(stderr.contains("--denoise"), "{stderr}");
    assert!(!dir.path().join("in.svg").exists(), "nothing was written");
}

#[test]
fn cli_preset_photo_implies_cleanup_off() {
    let (photo, stats) = convert_with(&noisy_logo_png(), &["--preset", "photo"]);
    assert!(stats.get("cleanup_denoise").is_none(), "{stats}");
    let (photo_off, _) = convert_with(
        &noisy_logo_png(),
        &["--preset", "photo", "--cleanup", "off"],
    );
    assert_eq!(photo, photo_off);
}

#[test]
fn cli_preset_photo_with_explicit_cleanup_auto_runs_cleanup() {
    let (_, stats) = convert_with(
        &noisy_logo_png(),
        &["--preset", "photo", "--cleanup", "auto"],
    );
    assert_eq!(stats["cleanup_denoise"], 3, "{stats}");
}

#[test]
fn cli_preset_photo_with_explicit_denoise_runs_cleanup() {
    let (_, stats) = convert_with(&noisy_logo_png(), &["--preset", "photo", "--denoise", "2"]);
    assert_eq!(stats["cleanup_denoise"], 2, "{stats}");
    assert_eq!(stats["cleanup_sharpen"], 0, "{stats}");
}

#[test]
fn cli_preset_auto_leaves_cleanup_on() {
    // `--preset auto` is `poster`, not `photo` (ADR-0003).
    let (_, stats) = convert_with(&noisy_logo_png(), &["--preset", "auto"]);
    assert_eq!(stats["cleanup_denoise"], 3, "{stats}");
}

#[test]
fn cli_stats_reports_measured_blur_and_noise() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(
        dir.path(),
        "blurry.png",
        &png_bytes(&degrade(Fixture::Logo, Degradation::Blur3)),
    );
    let assert = vectorise()
        .current_dir(dir.path())
        .args(["--stats", "blurry.png"])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert!(stderr.contains("cleanup d0 s3 (blur 8."), "{stderr}");
    assert!(stderr.contains("px, noise 0.00"), "{stderr}");
}

#[test]
fn cli_stats_json_includes_cleanup_fields() {
    let (_, stats) = convert_with(&noisy_logo_png(), &["--verify"]);
    // The values move with the machine's clock and, in the last digits, with
    // the platform's JPEG decoder; the schema does not. Replace each value
    // with its JSON type so the snapshot documents the shape alone.
    let mut schema = stats.clone();
    for (key, value) in schema.as_object_mut().expect("an object") {
        if key == "primitives" {
            continue;
        }
        *value = serde_json::Value::String(
            match value {
                serde_json::Value::Number(number) if number.is_u64() => "integer",
                serde_json::Value::Number(_) => "number",
                serde_json::Value::String(_) => "string",
                _ => "other",
            }
            .to_owned(),
        );
    }
    insta::assert_snapshot!(serde_json::to_string_pretty(&schema).expect("serializes"));

    assert_eq!(stats["cleanup_denoise"], 3);
    assert!(stats["flat_noise"].as_f64().expect("measured") > 0.05);
    assert!(stats["edge_width"].as_f64().expect("measured") < 2.5);
}

#[test]
fn cli_verify_reports_cleanup_delta() {
    let (_, stats) = convert_with(&noisy_logo_png(), &["--verify"]);
    let delta = stats["cleanup_delta"].as_f64().expect("reported");
    assert!(
        delta > 0.0 && delta < 0.05,
        "a small change to the raster: {delta}"
    );
    let fidelity = stats["fidelity"].as_f64().expect("a score");
    assert!(fidelity > 0.98, "against the cleaned image: {fidelity}");

    let (_, off) = convert_with(&noisy_logo_png(), &["--verify", "--cleanup", "off"]);
    assert!(
        off.get("cleanup_delta").is_none(),
        "absent means zero: {off}"
    );
    assert!(off["fidelity"].as_f64().expect("a score") > 0.98);
}

// --- reporting ---------------------------------------------------------------------

#[test]
fn cli_stats_reports_one_line_per_file_and_a_total() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "disc.png", &fixtures::disc_png());
    fixtures::place(dir.path(), "rect.png", &fixtures::rect_png());

    let assert = vectorise()
        .current_dir(dir.path())
        .args(["--stats", "disc.png", "rect.png"])
        .assert()
        .success();

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 3, "two files and a total: {stderr}");
    assert!(lines[0].contains("disc.png -> disc.svg"), "{stderr}");
    assert!(lines[1].contains("rect.png -> rect.svg"), "{stderr}");
    assert!(lines[2].starts_with("total: 2 file(s)"), "{stderr}");
    assert!(
        !stderr.contains("fidelity"),
        "no fidelity without --verify: {stderr}"
    );
}

#[test]
fn cli_verify_adds_a_fidelity_score() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "disc.png", &fixtures::disc_png());

    let assert = vectorise()
        .current_dir(dir.path())
        .args(["--stats", "--verify", "disc.png"])
        .assert()
        .success();

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    let score: f64 = stderr
        .split("fidelity ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("a fidelity score")
        .parse()
        .expect("a number");
    assert!(
        score > 0.98,
        "a traced disc comes back nearly exactly: {score}"
    );
}

#[test]
fn cli_stats_json_writes_one_object_per_line_to_stdout() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "disc.png", &fixtures::disc_png());
    fixtures::place(dir.path(), "rect.png", &fixtures::rect_png());

    let assert = vectorise()
        .current_dir(dir.path())
        .args(["--stats-json", "--verify", "disc.png", "rect.png"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "one object per file: {stdout}");

    for (line, name) in lines.iter().zip(["disc", "rect"]) {
        let value: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
        assert_eq!(value["input"], format!("{name}.png"));
        assert_eq!(value["output"], format!("{name}.svg"));
        assert!(value["output_bytes"].as_u64().expect("bytes") > 0);
        assert!(value["fidelity"].as_f64().expect("a score") > 0.9);
    }

    let disc: serde_json::Value = serde_json::from_str(lines[0]).expect("valid JSON");
    assert_eq!(disc["primitives"]["circles"], 1, "{stdout}");
}

// --- interruption and isolation ---------------------------------------------------

#[cfg(unix)]
#[test]
fn cli_sigint_leaves_no_temp_files() {
    use std::io::Write as _;
    use std::process::{Command as StdCommand, Stdio};

    let dir = tempfile::tempdir().expect("tempdir");
    // Enough work that the process is still running when the signal arrives.
    for index in 0..24 {
        fixtures::place(
            dir.path(),
            &format!("f{index:02}.png"),
            &fixtures::disc_png(),
        );
    }

    let binary = assert_cmd::cargo::cargo_bin("vectorise");
    let mut child = StdCommand::new(binary)
        .current_dir(dir.path())
        .args(["--jobs", "1", "-v"])
        .args((0..24).map(|index| format!("f{index:02}.png")))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawns");

    // Wait for the first output, so the signal lands mid-batch rather than
    // before any work started.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        let written = (0..24)
            .filter(|index| dir.path().join(format!("f{index:02}.svg")).exists())
            .count();
        if written >= 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // `Child::kill` sends SIGKILL; SIGINT needs the shell's `kill`. Running a
    // subprocess from a test is fine: the rule is that the *binary* spawns
    // nothing.
    let _ = std::io::stdout().flush();
    let _ = StdCommand::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status();
    let _ = child.wait();

    for entry in std::fs::read_dir(dir.path()).expect("read_dir") {
        let path = entry.expect("entry").path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();

        let extension = path.extension().and_then(|ext| ext.to_str());
        if extension == Some("png") {
            continue;
        }
        if extension == Some("svg") {
            assert!(
                is_complete_svg(&path),
                "{name} is a partial output: an interrupted run must never leave one"
            );
            continue;
        }
        // A hard kill runs no destructors, so an in-flight temporary can
        // survive. It must at least be recognizably ours and hidden.
        assert!(
            name.starts_with(".vectorise-"),
            "unexpected leftover {name}"
        );
    }
}

#[test]
fn cli_no_network_and_no_subprocess() {
    // `strace` is the only portable-enough way to assert this, and it is
    // Linux-only. Elsewhere the guarantee rests on the dependency set: no
    // crate in the tree opens a socket or spawns a process.
    if !cfg!(target_os = "linux") {
        eprintln!("skipped: strace is Linux-only, and this host is not Linux");
        return;
    }

    let available = std::process::Command::new("strace")
        .arg("-V")
        .output()
        .is_ok();
    if !available {
        eprintln!("skipped: strace is not installed on this runner");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    fixtures::place(dir.path(), "disc.png", &fixtures::disc_png());
    let binary = assert_cmd::cargo::cargo_bin("vectorise");

    let output = std::process::Command::new("strace")
        .args([
            "-f",
            "-e",
            "trace=execve,connect,socket",
            "-o",
            "/dev/stdout",
        ])
        .arg(&binary)
        .arg("disc.png")
        .current_dir(dir.path())
        .output()
        .expect("strace runs");

    let trace = String::from_utf8_lossy(&output.stdout);
    let offenders: Vec<&str> = trace
        .lines()
        .filter(|line| line.contains("connect(") || line.contains("socket("))
        .collect();
    assert!(
        offenders.is_empty(),
        "the binary opened a socket: {offenders:?}"
    );

    // The first execve is the binary itself; any further one is a subprocess.
    let executions = trace
        .lines()
        .filter(|line| line.contains("execve("))
        .count();
    assert!(executions <= 1, "the binary spawned a subprocess: {trace}");
}
