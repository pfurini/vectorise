//! The 24-case matrix of `CLEANUP_IMPLEMENTATION_PLAN.md` §9.3, kept under
//! test: four fixtures, six degradations, the automatic rule against no
//! cleanup at all, through the real pipeline.
//!
//! Two properties hold on every cell, and the byte counts are a snapshot: a
//! change that moves them must be reviewed, not silently accepted.
//! The tracer's curve fitting differs by an ulp between x86 and arm64, which
//! moves a cell by a few bytes, so the snapshot rounds bytes to two
//! significant figures and fidelity to three decimals; the picks are exact.
//!
//! The counts are not the plan's. The plan's spike traced with
//! `TraceOptions::default()`, which is VTracer's own configuration and keeps
//! six bits per colour channel; the command line traces with the `auto`
//! preset, which is `poster` (ADR-0003) and keeps eight. This test runs the
//! command line's pipeline. Run with `TraceOptions::default()` instead, it
//! reproduces §9.3 to the byte in 23 of 24 cells (the 24th differs by 35
//! bytes) and its total of 447 582 to 43 762 bytes within 35 bytes.
// The clean cells must be exactly equal, not approximately: they are no-ops.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::float_cmp
)]

mod common;

use std::fmt::Write as _;

use common::degraded::{Degradation, Fixture, degrade};
use vectorise::cleanup::{CleanupOptions, cleanup};
use vectorise::color::Rgb;
use vectorise::shapes::{ShapeFitOptions, fit_shapes};
use vectorise::trace::{TraceOptions, trace};
use vectorise::writer::{WriterOptions, write_svg};
use vtracer::ColorImage;

/// The pipeline from a decoded image to the final document, as `convert_one`
/// runs it, minus the file system.
fn convert(image: &ColorImage) -> String {
    let traced = trace(
        image,
        &TraceOptions {
            preset: Some(vectorise::trace::Preset::Auto),
            ..TraceOptions::default()
        },
    )
    .expect("traces");
    let fitted = fit_shapes(&traced, &ShapeFitOptions::default());
    let written = write_svg(&fitted, &WriterOptions::default());
    vectorise::optimize::optimize(&written).expect("optimizes")
}

/// One cell of the matrix.
struct Cell {
    fixture: Fixture,
    degradation: Degradation,
    denoise: u8,
    sharpen: u8,
    bytes_off: usize,
    bytes_auto: usize,
    fidelity_off: f64,
    fidelity_auto: f64,
}

fn matrix() -> Vec<Cell> {
    let mut cells = Vec::new();
    for fixture in Fixture::ALL {
        let pristine = fixture.pristine();
        for degradation in Degradation::ALL {
            let input = degrade(fixture, degradation);
            let svg_off = convert(&input);
            let (cleaned, report) = cleanup(&input, CleanupOptions::default());
            let svg_auto = convert(&cleaned);
            // Fidelity against the pristine drawing: the only measure of
            // whether the result matches what was drawn.
            let fidelity = |svg: &str| {
                vectorise::verify::fidelity(&pristine, svg, Rgb::WHITE)
                    .expect("verifies")
                    .score
            };
            cells.push(Cell {
                fixture,
                degradation,
                denoise: report.denoise,
                sharpen: report.sharpen,
                bytes_off: svg_off.len(),
                bytes_auto: svg_auto.len(),
                fidelity_off: fidelity(&svg_off),
                fidelity_auto: fidelity(&svg_auto),
            });
        }
    }
    cells
}

#[test]
fn auto_never_increases_output_bytes_and_improves_fidelity_on_every_degraded_cell() {
    let cells = matrix();
    assert_eq!(cells.len(), 24);

    // Two significant figures: stable across platforms, still a regression
    // guard at the 5% level.
    let rounded = |bytes: usize| -> usize {
        let digits = u32::try_from(bytes.to_string().len().saturating_sub(2)).expect("small");
        let magnitude = 10_usize.pow(digits);
        (bytes + magnitude / 2) / magnitude * magnitude
    };
    let mut table =
        String::from("fixture  degradation      auto   off_bytes  auto_bytes  off_fid  auto_fid\n");
    for cell in &cells {
        let name = format!("{} {}", cell.fixture.name(), cell.degradation.name());
        assert!(
            cell.bytes_auto <= cell.bytes_off,
            "{name}: auto made the output larger, {} -> {}",
            cell.bytes_off,
            cell.bytes_auto
        );
        if cell.degradation == Degradation::Clean {
            assert_eq!((cell.denoise, cell.sharpen), (0, 0), "{name}: not a no-op");
            assert_eq!(
                cell.bytes_auto, cell.bytes_off,
                "{name}: not byte-identical"
            );
            assert_eq!(cell.fidelity_auto, cell.fidelity_off, "{name}");
        } else {
            assert!(
                cell.fidelity_auto > cell.fidelity_off,
                "{name}: fidelity against the pristine drawing did not improve, {:.4} -> {:.4}",
                cell.fidelity_off,
                cell.fidelity_auto
            );
        }
        writeln!(
            table,
            "{:<8} {:<15}  d{}s{}  {:>9}  {:>10}  {:.3}    {:.3}",
            cell.fixture.name(),
            cell.degradation.name(),
            cell.denoise,
            cell.sharpen,
            rounded(cell.bytes_off),
            rounded(cell.bytes_auto),
            cell.fidelity_off,
            cell.fidelity_auto,
        )
        .expect("writes");
    }
    let total_off: usize = cells.iter().map(|cell| cell.bytes_off).sum();
    let total_auto: usize = cells.iter().map(|cell| cell.bytes_auto).sum();
    writeln!(
        table,
        "total {} -> {} bytes",
        rounded(total_off),
        rounded(total_auto)
    )
    .expect("writes");
    assert!(
        total_auto * 4 < total_off,
        "the plan measured a 10x reduction; less than 4x is a regression: {table}"
    );

    insta::assert_snapshot!(table);
}

/// Write every cell's input as a PNG under `target/fixtures/`, for looking at
/// the pass by hand: `cargo nextest run --test cleanup --run-ignored all dump`.
#[test]
#[ignore = "writes files for manual acceptance; not a check"]
fn dump_fixtures_for_manual_acceptance() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/fixtures");
    std::fs::create_dir_all(&dir).expect("mkdir");
    for fixture in Fixture::ALL {
        for degradation in Degradation::ALL {
            let name = format!("{}-{}.png", fixture.name(), degradation.name());
            std::fs::write(
                dir.join(name),
                common::degraded::png_bytes(&degrade(fixture, degradation)),
            )
            .expect("write");
        }
    }
}
