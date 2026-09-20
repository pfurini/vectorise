//! What the conversion did, in numbers.
//!
//! `--stats` prints one line per file and a total, on stderr, for a person.
//! `--stats-json` prints one JSON object per file on stdout, for a script. The
//! two are the same data; only the rendering differs.
//!
//! The JSON schema is part of the interface. Fields may be added; existing
//! field names and meanings will not change without a major version.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::shapes::{Prim, ShapeDoc};

/// A byte count as a float, for a ratio.
///
/// Exact for every file that fits in memory: `f64` represents integers up to
/// 2^53 without loss, which is 9 petabytes.
#[expect(
    clippy::cast_precision_loss,
    reason = "byte counts never approach 2^53"
)]
const fn as_f64(bytes: u64) -> f64 {
    bytes as f64
}

/// How many of each kind of element the document ended up with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Primitives {
    /// `<circle>` elements.
    pub circles: usize,
    /// `<ellipse>` elements.
    pub ellipses: usize,
    /// `<rect>` elements with sharp corners.
    pub rects: usize,
    /// `<rect>` elements with corner radii.
    pub rounded_rects: usize,
    /// `<path>` elements: everything that was not one of the above.
    pub paths: usize,
}

impl Primitives {
    /// How many elements are native shapes rather than paths.
    #[must_use]
    pub const fn shapes(&self) -> usize {
        self.circles + self.ellipses + self.rects + self.rounded_rects
    }

    /// Every element, shapes and paths together.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.shapes() + self.paths
    }

    fn count(&mut self, prim: &Prim) {
        match prim {
            Prim::Circle { .. } => self.circles += 1,
            Prim::Ellipse { .. } => self.ellipses += 1,
            Prim::Rect { rx, .. } if *rx > 0.0 => self.rounded_rects += 1,
            Prim::Rect { .. } => self.rects += 1,
            Prim::Path(_) => self.paths += 1,
        }
    }
}

/// The size of a document in bytes.
///
/// Exact: a `usize` length always fits a `u64` on every target we build for.
fn svg_bytes(svg: &str) -> u64 {
    u64::try_from(svg.len()).unwrap_or(u64::MAX)
}

/// What one conversion did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stats {
    /// The file that was read.
    pub input: PathBuf,
    /// The file that was written.
    pub output: PathBuf,
    /// Size of the input, in bytes.
    pub input_bytes: u64,
    /// Size of the output, in bytes.
    pub output_bytes: u64,
    /// Distinct fill colors in the output.
    pub colors: usize,
    /// Elements by kind.
    pub primitives: Primitives,
    /// Drawing commands across every `<path>`.
    pub path_commands: usize,
    /// Wall-clock time for this file.
    pub elapsed_ms: u64,
    /// `1 - mean absolute error` against the input, when `--verify` asked.
    pub fidelity: Option<f64>,
}

impl Stats {
    /// Measure a converted document.
    ///
    /// `elapsed_ms` and `fidelity` are filled in by the caller, which is the
    /// only part of this that is not a property of the document.
    #[must_use]
    pub fn of(input: &Path, output: &Path, doc: &ShapeDoc, svg: &str) -> Self {
        let mut primitives = Primitives::default();
        let mut path_commands = 0;
        let mut colors = BTreeSet::new();

        for shape in &doc.shapes {
            primitives.count(&shape.prim);
            colors.insert((shape.fill.r, shape.fill.g, shape.fill.b, shape.fill.a));
            if let Prim::Path(path) = &shape.prim {
                path_commands += path.elements().len();
            }
        }

        Self {
            input: input.to_path_buf(),
            output: output.to_path_buf(),
            input_bytes: std::fs::metadata(input).map_or(0, |meta| meta.len()),
            output_bytes: svg_bytes(svg),
            colors: colors.len(),
            primitives,
            path_commands,
            elapsed_ms: 0,
            fidelity: None,
        }
    }

    /// Output size as a fraction of input size. `None` for an empty input.
    #[must_use]
    pub fn ratio(&self) -> Option<f64> {
        (self.input_bytes > 0).then(|| as_f64(self.output_bytes) / as_f64(self.input_bytes))
    }

    /// One human-readable line.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    /// use vectorise::shapes::ShapeDoc;
    /// use vectorise::stats::Stats;
    ///
    /// let doc = ShapeDoc { width: 1, height: 1, shapes: Vec::new() };
    /// let stats = Stats::of(Path::new("a.png"), Path::new("a.svg"), &doc, "<svg/>");
    /// assert!(stats.to_line().contains("a.png"));
    /// ```
    #[must_use]
    pub fn to_line(&self) -> String {
        let ratio = self
            .ratio()
            .map_or_else(|| "-".to_owned(), |ratio| format!("{:.0}%", ratio * 100.0));
        let fidelity = self
            .fidelity
            .map_or_else(String::new, |score| format!(" fidelity {score:.4}"));

        format!(
            "{} -> {}  {} -> {} bytes ({ratio})  {} colors  {} shapes ({} circle, {} ellipse, {} rect, {} rounded, {} path, {} cmds)  {} ms{fidelity}",
            self.input.display(),
            self.output.display(),
            self.input_bytes,
            self.output_bytes,
            self.colors,
            self.primitives.total(),
            self.primitives.circles,
            self.primitives.ellipses,
            self.primitives.rects,
            self.primitives.rounded_rects,
            self.primitives.paths,
            self.path_commands,
            self.elapsed_ms,
        )
    }
}

/// The sum of a batch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Totals {
    /// How many files were converted.
    pub files: usize,
    /// Total input bytes.
    pub input_bytes: u64,
    /// Total output bytes.
    pub output_bytes: u64,
    /// Elements by kind, across the batch.
    pub primitives: Primitives,
    /// Drawing commands across the batch.
    pub path_commands: usize,
    /// Wall-clock time for the batch.
    pub elapsed_ms: u64,
}

impl Totals {
    /// Add up a batch.
    #[must_use]
    pub fn of<'a>(stats: impl IntoIterator<Item = &'a Stats>) -> Self {
        let mut totals = Self::default();
        for one in stats {
            totals.files += 1;
            totals.input_bytes += one.input_bytes;
            totals.output_bytes += one.output_bytes;
            totals.primitives.circles += one.primitives.circles;
            totals.primitives.ellipses += one.primitives.ellipses;
            totals.primitives.rects += one.primitives.rects;
            totals.primitives.rounded_rects += one.primitives.rounded_rects;
            totals.primitives.paths += one.primitives.paths;
            totals.path_commands += one.path_commands;
        }
        totals
    }

    /// Output size as a fraction of input size. `None` for an empty batch.
    #[must_use]
    pub fn ratio(&self) -> Option<f64> {
        (self.input_bytes > 0).then(|| as_f64(self.output_bytes) / as_f64(self.input_bytes))
    }

    /// One human-readable line.
    #[must_use]
    pub fn to_line(&self) -> String {
        let ratio = self
            .ratio()
            .map_or_else(|| "-".to_owned(), |ratio| format!("{:.0}%", ratio * 100.0));
        format!(
            "total: {} file(s)  {} -> {} bytes ({ratio})  {} shapes, {} paths  {} ms",
            self.files,
            self.input_bytes,
            self.output_bytes,
            self.primitives.shapes(),
            self.primitives.paths,
            self.elapsed_ms,
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use std::path::Path;

    use kurbo::BezPath;

    use super::{Stats, Totals};
    use crate::color::Rgba;
    use crate::shapes::{Prim, ShapeDoc, ShapeEl};

    fn sample_doc() -> ShapeDoc {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((10.0, 0.0));
        path.line_to((10.0, 10.0));
        path.close_path();

        ShapeDoc {
            width: 40,
            height: 30,
            shapes: vec![
                ShapeEl {
                    prim: Prim::Circle {
                        cx: 5.0,
                        cy: 5.0,
                        r: 4.0,
                    },
                    fill: Rgba::new(255, 0, 0, 255),
                },
                ShapeEl {
                    prim: Prim::Ellipse {
                        cx: 20.0,
                        cy: 10.0,
                        rx: 8.0,
                        ry: 4.0,
                        rotate_deg: 0.0,
                    },
                    fill: Rgba::new(0, 255, 0, 255),
                },
                ShapeEl {
                    prim: Prim::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 10.0,
                        h: 10.0,
                        rx: 0.0,
                        ry: 0.0,
                    },
                    // The same red again: colors are counted distinctly.
                    fill: Rgba::new(255, 0, 0, 255),
                },
                ShapeEl {
                    prim: Prim::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 10.0,
                        h: 10.0,
                        rx: 2.0,
                        ry: 2.0,
                    },
                    fill: Rgba::new(0, 0, 255, 255),
                },
                ShapeEl {
                    prim: Prim::Path(path),
                    fill: Rgba::new(1, 2, 3, 255),
                },
            ],
        }
    }

    #[test]
    fn stats_counts_match_writer_output() {
        let doc = sample_doc();
        let svg = crate::writer::write_svg(&doc, &crate::writer::WriterOptions::default());
        let stats = Stats::of(Path::new("a.png"), Path::new("a.svg"), &doc, &svg);

        // The document says one of each; so does the SVG the writer produced.
        assert_eq!(stats.primitives.circles, 1);
        assert_eq!(stats.primitives.ellipses, 1);
        assert_eq!(stats.primitives.rects, 1);
        assert_eq!(stats.primitives.rounded_rects, 1);
        assert_eq!(stats.primitives.paths, 1);
        assert_eq!(stats.primitives.total(), 5);
        assert_eq!(stats.primitives.shapes(), 4);

        assert_eq!(svg.matches("<circle").count(), stats.primitives.circles);
        assert_eq!(svg.matches("<ellipse").count(), stats.primitives.ellipses);
        assert_eq!(
            svg.matches("<rect").count(),
            stats.primitives.rects + stats.primitives.rounded_rects
        );
        assert_eq!(svg.matches("<path").count(), stats.primitives.paths);

        // And the parser agrees with both.
        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).expect("parses");
        assert_eq!(count_paths(tree.root()), stats.primitives.total());

        assert_eq!(stats.colors, 4, "the repeated red counts once");
        assert_eq!(stats.path_commands, 4, "M, L, L, Z");
        assert_eq!(stats.output_bytes, svg.len() as u64);
    }

    fn count_paths(group: &usvg::Group) -> usize {
        group
            .children()
            .iter()
            .map(|node| match node {
                usvg::Node::Path(_) => 1,
                usvg::Node::Group(inner) => count_paths(inner),
                usvg::Node::Image(_) | usvg::Node::Text(_) => 0,
            })
            .sum()
    }

    #[test]
    fn stats_ratio_is_output_over_input() {
        let doc = ShapeDoc {
            width: 1,
            height: 1,
            shapes: Vec::new(),
        };
        let mut stats = Stats::of(Path::new("a.png"), Path::new("a.svg"), &doc, "<svg/>");
        assert_eq!(stats.input_bytes, 0, "a path that does not exist");
        assert_eq!(stats.ratio(), None, "no ratio without an input size");

        stats.input_bytes = 1000;
        stats.output_bytes = 250;
        assert!((stats.ratio().expect("a ratio") - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn stats_line_names_both_files_and_every_count() {
        let doc = sample_doc();
        let svg = crate::writer::write_svg(&doc, &crate::writer::WriterOptions::default());
        let mut stats = Stats::of(Path::new("logo.png"), Path::new("logo.svg"), &doc, &svg);
        stats.input_bytes = 2000;
        stats.elapsed_ms = 7;
        stats.fidelity = Some(0.9876);

        let line = stats.to_line();
        for fragment in [
            "logo.png",
            "logo.svg",
            "7 ms",
            "fidelity 0.9876",
            "4 colors",
        ] {
            assert!(line.contains(fragment), "{fragment} missing from {line}");
        }
    }

    #[test]
    fn stats_line_omits_fidelity_when_it_was_not_measured() {
        let doc = sample_doc();
        let stats = Stats::of(Path::new("a.png"), Path::new("a.svg"), &doc, "<svg/>");
        assert!(!stats.to_line().contains("fidelity"));
    }

    #[test]
    fn totals_add_up_a_batch() {
        let doc = sample_doc();
        let svg = crate::writer::write_svg(&doc, &crate::writer::WriterOptions::default());
        let mut one = Stats::of(Path::new("a.png"), Path::new("a.svg"), &doc, &svg);
        one.input_bytes = 1000;
        let mut two = one.clone();
        two.input_bytes = 3000;

        let totals = Totals::of([&one, &two]);
        assert_eq!(totals.files, 2);
        assert_eq!(totals.input_bytes, 4000);
        assert_eq!(totals.output_bytes, one.output_bytes * 2);
        assert_eq!(totals.primitives.circles, 2);
        assert_eq!(totals.primitives.shapes(), 8);
        assert_eq!(totals.path_commands, 8);
        assert!(totals.to_line().contains("2 file(s)"));

        assert_eq!(Totals::of([]).ratio(), None);
    }

    #[test]
    fn stats_json_lines_are_valid_and_schema_stable() {
        let doc = sample_doc();
        let svg = crate::writer::write_svg(&doc, &crate::writer::WriterOptions::default());
        let mut stats = Stats::of(Path::new("logo.png"), Path::new("logo.svg"), &doc, &svg);
        stats.input_bytes = 2048;
        // Zeroed, because a wall clock is not a stable snapshot.
        stats.elapsed_ms = 0;
        stats.fidelity = Some(0.9876);

        let line = serde_json::to_string(&stats).expect("serializes");
        assert!(!line.contains('\n'), "one object per line");

        let parsed: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
        assert_eq!(parsed["input"], "logo.png");
        assert_eq!(parsed["primitives"]["circles"], 1);

        insta::assert_snapshot!(line);
    }
}
