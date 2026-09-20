//! Convert raster images into maximally compact, standard-compliant SVG.
//!
//! `vectorise` traces a raster image with [VTracer], then replaces every traced
//! region that is geometrically indistinguishable from a circle, ellipse, or
//! rectangle with the corresponding native SVG element, and finally minifies
//! the document. The result is fewer paths, fewer bytes, and markup a human can
//! still edit.
//!
//! [VTracer]: https://crates.io/crates/vtracer
//!
//! # The pipeline
//!
//! ```text
//! file ──▶ decode ──▶ trace ──▶ shapes ──▶ writer ──▶ optimize ──▶ output
//! ```
//!
//! Each stage is a module with one job and its own typed error. [`convert_one`]
//! runs them in order for a single file; [`run`] runs [`convert_one`] over a
//! [`Plan`] in parallel and reports what happened to each input.
//!
//! # Safety of a batch
//!
//! Nothing is written until the whole batch has been checked. [`plan()`] refuses
//! a batch in which any output already exists or two inputs would produce the
//! same output, and reports every such problem at once. Writing itself is
//! atomic: an interrupted run leaves no partial file behind.
//!
//! # Examples
//!
//! ```no_run
//! use std::path::PathBuf;
//!
//! use vectorise::{Options, PlanOptions, plan, run};
//!
//! let inputs = vec![PathBuf::from("logo.png"), PathBuf::from("icon.png")];
//! let batch = plan(&inputs, &PlanOptions::default())?;
//!
//! let report = run(&batch, &Options::default(), 4);
//! println!("{} converted, {} failed", report.ok.len(), report.failed.len());
//! # Ok::<(), vectorise::PlanError>(())
//! ```

pub mod cleanup;
pub mod cli;
pub mod color;
pub mod decode;
pub mod error;
pub mod geom;
pub mod optimize;
pub mod output;
pub mod plan;
pub mod shapes;
pub mod stats;
pub mod trace;
pub mod verify;
pub mod writer;

use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::decode::{DecodeError, DecodeOptions};
use crate::error::ExitCode;
use crate::optimize::OptimizeError;
use crate::output::WriteError;
use crate::shapes::ShapeFitOptions;
use crate::stats::{Stats, Totals};
use crate::trace::{TraceError, TraceOptions};
use crate::verify::VerifyError;
use crate::writer::WriterOptions;

pub use plan::{Job, Plan, PlanError, PlanOptions, PlanProblem, plan};

/// Everything a conversion needs, beyond which file to read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Options {
    /// How transparency is resolved when reading.
    pub decode: DecodeOptions,
    /// How the image is traced.
    pub trace: TraceOptions,
    /// How hard shape detection tries.
    pub shapes: ShapeFitOptions,
    /// How the document is serialized.
    pub writer: WriterOptions,
    /// Run the optimizer over the written SVG.
    pub optimize: bool,
    /// Overwrite an output that already exists.
    pub force: bool,
    /// Render the result and measure how close it came to the input.
    pub verify: bool,
}

impl Options {
    /// Options that do everything: detect shapes, optimize, refuse to
    /// overwrite.
    ///
    /// # Examples
    ///
    /// ```
    /// use vectorise::Options;
    ///
    /// assert!(Options::new().optimize);
    /// assert!(!Options::new().force);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            optimize: true,
            ..Self::default()
        }
    }
}

/// What one conversion produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Converted {
    /// The SVG document, ready to write.
    pub svg: String,
    /// What the conversion did, in numbers.
    pub stats: Stats,
}

/// Why a conversion failed. Every variant names the file it is about.
#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    /// The input could not be read.
    #[error(transparent)]
    Decode(#[from] DecodeError),
    /// The tracer refused the image.
    #[error("{path}: {source}")]
    Trace {
        /// The input being converted.
        path: PathBuf,
        /// What the tracer said.
        source: TraceError,
    },
    /// The optimizer refused the document we wrote.
    #[error("{path}: {source}")]
    Optimize {
        /// The input being converted.
        path: PathBuf,
        /// What the optimizer said.
        source: OptimizeError,
    },
    /// The output could not be written.
    #[error(transparent)]
    Write(#[from] WriteError),
    /// `--verify` could not measure the result.
    #[error("{path}: {source}")]
    Verify {
        /// The input being converted.
        path: PathBuf,
        /// What the verifier said.
        source: VerifyError,
    },
}

impl ConvertError {
    /// The file this error is about.
    ///
    /// For a write failure that is the *output*; for everything else it is the
    /// input. Both are the file a user would go and look at.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Decode(error) => error.path(),
            Self::Trace { path, .. } | Self::Optimize { path, .. } | Self::Verify { path, .. } => {
                path
            }
            Self::Write(error) => error.path(),
        }
    }
}

/// Convert one file and return the SVG, without writing anything.
///
/// # Errors
///
/// [`ConvertError`], carrying the file the failure is about.
pub fn convert_one(
    input: &Path,
    output: &Path,
    options: &Options,
) -> Result<Converted, ConvertError> {
    let started = std::time::Instant::now();

    let image = decode::decode(input, options.decode)?;
    tracing::debug!(
        input = %input.display(),
        width = image.width,
        height = image.height,
        "decoded"
    );

    let traced = trace::trace(&image, &options.trace).map_err(|source| ConvertError::Trace {
        path: input.to_path_buf(),
        source,
    })?;
    tracing::debug!(input = %input.display(), shapes = traced.shapes.len(), "traced");

    let fitted = shapes::fit_shapes(&traced, &options.shapes);
    tracing::debug!(
        input = %input.display(),
        shapes = fitted.shapes.len(),
        "fitted"
    );

    let written = writer::write_svg(&fitted, &options.writer);
    let svg = if options.optimize {
        let optimized = optimize::optimize(&written).map_err(|source| ConvertError::Optimize {
            path: input.to_path_buf(),
            source,
        })?;
        tracing::debug!(
            input = %input.display(),
            before = written.len(),
            after = optimized.len(),
            "optimized"
        );
        optimized
    } else {
        written
    };

    let mut stats = Stats::of(input, output, &fitted, &svg);
    if options.verify {
        let measured =
            verify::fidelity(&image, &svg, options.decode.background).map_err(|source| {
                ConvertError::Verify {
                    path: input.to_path_buf(),
                    source,
                }
            })?;
        tracing::debug!(input = %input.display(), score = measured.score, "verified");
        stats.fidelity = Some(measured.score);
    }
    stats.elapsed_ms = elapsed_ms(started);

    Ok(Converted { svg, stats })
}

/// Milliseconds since `started`, saturating rather than wrapping.
fn elapsed_ms(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Convert one job and write its output.
///
/// # Errors
///
/// [`ConvertError`], carrying the file the failure is about.
pub fn convert_job(job: &Job, options: &Options) -> Result<Converted, ConvertError> {
    let converted = convert_one(&job.input, &job.output, options)?;
    output::write_atomically(&job.output, &converted.svg, options.force)?;
    Ok(converted)
}

/// What a batch did.
#[derive(Debug, Default)]
pub struct RunReport {
    /// Jobs whose output was written.
    pub ok: Vec<Job>,
    /// What each successful job did, in the same order as `ok`.
    pub stats: Vec<Stats>,
    /// Jobs that failed, with the reason.
    pub failed: Vec<(Job, ConvertError)>,
    /// Wall-clock time for the batch.
    pub elapsed_ms: u64,
}

impl RunReport {
    /// Did every job succeed?
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.failed.is_empty()
    }

    /// The batch's numbers, added up.
    #[must_use]
    pub fn totals(&self) -> Totals {
        let mut totals = Totals::of(&self.stats);
        totals.elapsed_ms = self.elapsed_ms;
        totals
    }
}

impl From<&RunReport> for ExitCode {
    /// A batch that wrote nothing because *every* job failed still exits 1,
    /// not 2: exit 2 is reserved for preflight, which promises nothing was
    /// attempted.
    fn from(report: &RunReport) -> Self {
        if report.failed.is_empty() {
            Self::Ok
        } else {
            Self::Partial
        }
    }
}

/// Convert a whole batch.
///
/// Jobs run on a `rayon` pool of `jobs` threads, or on the current thread when
/// `jobs` is 1. Results are collected in plan order whatever the thread count,
/// so a report and its log lines do not depend on scheduling.
///
/// A failure fails one job. The rest of the batch still runs, which is why the
/// report has two lists rather than being a `Result`.
#[must_use]
pub fn run(plan: &Plan, options: &Options, jobs: usize) -> RunReport {
    let started = std::time::Instant::now();
    let results: Vec<(Job, Result<Converted, ConvertError>)> = if jobs <= 1 {
        plan.jobs
            .iter()
            .map(|job| (job.clone(), convert_job(job, options)))
            .collect()
    } else {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build()
            .ok();
        let convert = || {
            plan.jobs
                .par_iter()
                .map(|job| (job.clone(), convert_job(job, options)))
                .collect()
        };
        // A pool that cannot be built is not a reason to refuse the work; fall
        // back to rayon's global pool.
        pool.map_or_else(convert, |pool| pool.install(convert))
    };

    let mut report = RunReport::default();
    for (job, result) in results {
        match result {
            Ok(converted) => {
                tracing::info!(
                    input = %job.input.display(),
                    output = %job.output.display(),
                    "converted"
                );
                report.ok.push(job);
                report.stats.push(converted.stats);
            }
            Err(error) => report.failed.push((job, error)),
        }
    }
    report.elapsed_ms = elapsed_ms(started);
    report
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use std::path::PathBuf;

    use super::{ConvertError, Job, Options, RunReport, convert_one, run};
    use crate::error::ExitCode;
    use crate::plan::Plan;

    /// A 40x40 PNG: a red disc on white, big enough to be detected as a circle.
    fn disc_png() -> Vec<u8> {
        use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

        let mut pixmap = Pixmap::new(80, 80).expect("pixmap");
        pixmap.fill(Color::WHITE);
        let mut builder = PathBuilder::new();
        builder.push_circle(40.0, 40.0, 25.0);
        let mut paint = Paint {
            anti_alias: false,
            ..Paint::default()
        };
        paint.set_color_rgba8(220, 30, 40, 255);
        pixmap.fill_path(
            &builder.finish().expect("circle"),
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
        pixmap.encode_png().expect("png")
    }

    #[test]
    fn convert_one_runs_the_whole_pipeline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let input = dir.path().join("disc.png");
        std::fs::write(&input, disc_png()).expect("write");

        let output = dir.path().join("disc.svg");
        let converted = convert_one(&input, &output, &Options::new()).expect("converts");
        assert!(converted.svg.starts_with("<svg "), "{}", converted.svg);
        assert_eq!(converted.stats.primitives.circles, 1);
        assert_eq!(converted.stats.output, output);
        assert!(
            converted.svg.contains("<circle"),
            "the disc is detected: {}",
            converted.svg
        );
    }

    #[test]
    fn convert_one_without_the_optimizer_still_produces_svg() {
        let dir = tempfile::tempdir().expect("tempdir");
        let input = dir.path().join("disc.png");
        std::fs::write(&input, disc_png()).expect("write");

        let options = Options {
            optimize: false,
            ..Options::new()
        };
        let converted =
            convert_one(&input, &dir.path().join("disc.svg"), &options).expect("converts");
        assert!(converted.svg.contains("<circle"));
    }

    #[test]
    fn convert_one_reports_the_input_path_on_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let input = dir.path().join("broken.png");
        std::fs::write(&input, b"not a png").expect("write");

        let error = convert_one(&input, &dir.path().join("broken.svg"), &Options::new())
            .expect_err("a corrupt file fails");
        assert_eq!(error.path(), input);
        assert!(matches!(error, ConvertError::Decode(_)), "{error:?}");
    }

    #[test]
    fn run_writes_every_output_and_reports_them_in_plan_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut jobs = Vec::new();
        for name in ["a", "b", "c"] {
            let input = dir.path().join(format!("{name}.png"));
            std::fs::write(&input, disc_png()).expect("write");
            jobs.push(Job {
                output: dir.path().join(format!("{name}.svg")),
                input,
            });
        }
        let plan = Plan { jobs: jobs.clone() };

        let report = run(&plan, &Options::new(), 4);
        assert!(report.is_complete(), "{:?}", report.failed);
        assert_eq!(report.ok, jobs, "order follows the plan, not the threads");
        for job in &jobs {
            assert!(job.output.is_file(), "{} exists", job.output.display());
        }
        assert_eq!(ExitCode::from(&report), ExitCode::Ok);
    }

    #[test]
    fn run_keeps_going_after_one_job_fails() {
        let dir = tempfile::tempdir().expect("tempdir");
        let good = dir.path().join("good.png");
        std::fs::write(&good, disc_png()).expect("write");
        let bad = dir.path().join("bad.png");
        std::fs::write(&bad, b"not a png at all").expect("write");

        let plan = Plan {
            jobs: vec![
                Job {
                    input: bad.clone(),
                    output: dir.path().join("bad.svg"),
                },
                Job {
                    input: good.clone(),
                    output: dir.path().join("good.svg"),
                },
            ],
        };

        let report = run(&plan, &Options::new(), 2);
        assert_eq!(report.ok.len(), 1);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.ok[0].input, good);
        assert_eq!(report.failed[0].0.input, bad);
        assert!(dir.path().join("good.svg").is_file());
        assert!(!dir.path().join("bad.svg").exists());
        assert_eq!(ExitCode::from(&report), ExitCode::Partial);
    }

    #[test]
    fn run_single_threaded_produces_the_same_report() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut jobs = Vec::new();
        for name in ["a", "b"] {
            let input = dir.path().join(format!("{name}.png"));
            std::fs::write(&input, disc_png()).expect("write");
            jobs.push(Job {
                output: dir.path().join(format!("{name}.svg")),
                input,
            });
        }

        let report = run(&Plan { jobs: jobs.clone() }, &Options::new(), 1);
        assert_eq!(report.ok, jobs);
    }

    #[test]
    fn run_on_an_empty_plan_does_nothing_and_succeeds() {
        let report = run(&Plan::default(), &Options::new(), 4);
        assert!(report.is_complete());
        assert!(report.ok.is_empty());
        assert_eq!(ExitCode::from(&report), ExitCode::Ok);
    }

    #[test]
    fn run_refuses_to_overwrite_without_force() {
        let dir = tempfile::tempdir().expect("tempdir");
        let input = dir.path().join("a.png");
        std::fs::write(&input, disc_png()).expect("write");
        let output = dir.path().join("a.svg");
        std::fs::write(&output, "theirs").expect("write");

        let plan = Plan {
            jobs: vec![Job {
                input,
                output: output.clone(),
            }],
        };

        let report = run(&plan, &Options::new(), 1);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&output).expect("reads"),
            "theirs",
            "their file survives"
        );

        let forced = Options {
            force: true,
            ..Options::new()
        };
        let report = run(&plan, &forced, 1);
        assert!(report.is_complete(), "{:?}", report.failed);
        assert!(
            std::fs::read_to_string(&output)
                .expect("reads")
                .contains("<svg")
        );
    }

    #[test]
    fn exit_code_never_reports_preflight_from_a_run() {
        // Preflight promises nothing was attempted; a run that failed every
        // job did attempt things, so it is a partial failure.
        let report = RunReport {
            ok: Vec::new(),
            stats: Vec::new(),
            elapsed_ms: 0,
            failed: vec![(
                Job {
                    input: PathBuf::from("a.png"),
                    output: PathBuf::from("a.svg"),
                },
                ConvertError::Trace {
                    path: PathBuf::from("a.png"),
                    source: crate::trace::TraceError::EmptyImage,
                },
            )],
        };
        assert_eq!(ExitCode::from(&report), ExitCode::Partial);
    }
}
