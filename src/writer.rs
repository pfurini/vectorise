//! Serialize a [`ShapeDoc`] to SVG.
//!
//! The writer knows nothing about tracing. It takes primitives and paths and
//! spends its effort on one thing: emitting the fewest bytes that still parse
//! as valid, readable SVG.
//!
//! # Output contract
//!
//! - `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 W H">`, with no
//!   `width` or `height` unless `--keep-size` asks for them. A document with
//!   only a `viewBox` scales to whatever box it is placed in.
//! - No XML prolog, no comments, no metadata, no `<g>`. Every byte has to earn
//!   its place, and a generator comment never does.
//! - Fills are `#rrggbb`, shortened to `#rgb` when that is exact.
//!   `fill-opacity` appears only when the alpha is below 255.
//! - Coordinates are rounded to `precision` decimals with trailing zeros
//!   stripped and the leading zero dropped: `10.50` becomes `10.5`, `10.0`
//!   becomes `10`, `0.5` becomes `.5`.
//! - Paths use whichever of the absolute and relative forms is shorter, plus
//!   the `h`, `v`, and `s` shorthands where they apply. Numbers are separated
//!   by a comma only when the next one does not start with a minus sign, which
//!   is self-separating.
//!
//! Consecutive shapes with the same fill are **not** merged into a `<g>`.
//! Merging would have to preserve paint order, and the shapes that share a
//! fill are rarely adjacent in it; see ADR-0005.

use std::fmt::Write as _;

use kurbo::{BezPath, PathEl, Point};

use crate::color::Rgba;
use crate::shapes::{Prim, ShapeDoc, ShapeEl};

/// The most decimals the writer will emit.
pub const MAX_PRECISION: u8 = 6;

/// Rotations smaller than this are not worth a `transform` attribute.
const MIN_ROTATION_DEG: f64 = 0.5;

/// How the document is serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriterOptions {
    /// Decimals kept on every coordinate.
    pub precision: u8,
    /// Emit `width` and `height` on the root element as well as `viewBox`.
    pub keep_size: bool,
}

impl Default for WriterOptions {
    fn default() -> Self {
        Self {
            precision: 2,
            keep_size: false,
        }
    }
}

/// Serialize a document.
///
/// # Examples
///
/// ```
/// use vectorise::color::Rgba;
/// use vectorise::shapes::{Prim, ShapeDoc, ShapeEl};
/// use vectorise::writer::{WriterOptions, write_svg};
///
/// let doc = ShapeDoc {
///     width: 100,
///     height: 100,
///     shapes: vec![ShapeEl {
///         prim: Prim::Circle { cx: 50.0, cy: 50.0, r: 40.0 },
///         fill: Rgba::new(255, 0, 0, 255),
///     }],
/// };
/// assert_eq!(
///     write_svg(&doc, &WriterOptions::default()),
///     r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="40" fill="#f00"/></svg>"##
/// );
/// ```
#[must_use]
pub fn write_svg(doc: &ShapeDoc, options: &WriterOptions) -> String {
    let precision = options.precision.min(MAX_PRECISION);
    let mut out = String::with_capacity(128 + doc.shapes.len() * 64);

    out.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 "#);
    let _ = write!(out, "{} {}\"", doc.width, doc.height);
    if options.keep_size {
        let _ = write!(out, " width=\"{}\" height=\"{}\"", doc.width, doc.height);
    }
    out.push('>');

    for shape in &doc.shapes {
        write_shape(&mut out, shape, precision);
    }

    out.push_str("</svg>");
    out
}

/// One element.
fn write_shape(out: &mut String, shape: &ShapeEl, precision: u8) {
    let n = |value: f64| format_number(value, precision);

    match &shape.prim {
        Prim::Circle { cx, cy, r } => {
            let _ = write!(
                out,
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\"",
                n(*cx),
                n(*cy),
                n(*r)
            );
            write_fill(out, shape.fill);
            out.push_str("/>");
        }
        Prim::Ellipse {
            cx,
            cy,
            rx,
            ry,
            rotate_deg,
        } => {
            let _ = write!(
                out,
                "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"",
                n(*cx),
                n(*cy),
                n(*rx),
                n(*ry)
            );
            write_fill(out, shape.fill);
            if rotate_deg.abs() > MIN_ROTATION_DEG {
                let _ = write!(
                    out,
                    " transform=\"rotate({} {} {})\"",
                    n(*rotate_deg),
                    n(*cx),
                    n(*cy)
                );
            }
            out.push_str("/>");
        }
        Prim::Rect { x, y, w, h, rx, ry } => {
            let _ = write!(
                out,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
                n(*x),
                n(*y),
                n(*w),
                n(*h)
            );
            // SVG defaults `ry` to `rx`, so a symmetric radius needs one
            // attribute, not two.
            if *rx > 0.0 || *ry > 0.0 {
                let _ = write!(out, " rx=\"{}\"", n(*rx));
                if (rx - ry).abs() > f64::EPSILON {
                    let _ = write!(out, " ry=\"{}\"", n(*ry));
                }
            }
            write_fill(out, shape.fill);
            out.push_str("/>");
        }
        Prim::Path(path) => {
            let data = encode_path(path, precision);
            if data.is_empty() {
                return;
            }
            let _ = write!(out, "<path d=\"{data}\"");
            write_fill(out, shape.fill);
            out.push_str("/>");
        }
    }
}

/// The `fill` attribute, and `fill-opacity` when the color is not opaque.
fn write_fill(out: &mut String, fill: Rgba) {
    let _ = write!(out, " fill=\"{}\"", format_color(fill));
    if !fill.is_opaque() {
        let _ = write!(
            out,
            " fill-opacity=\"{}\"",
            format_number(f64::from(fill.a) / 255.0, 3)
        );
    }
}

/// `#rgb` when every channel's two digits are equal, `#rrggbb` otherwise.
fn format_color(fill: Rgba) -> String {
    let short = |channel: u8| channel >> 4 == channel & 0x0f;
    if short(fill.r) && short(fill.g) && short(fill.b) {
        format!("#{:x}{:x}{:x}", fill.r & 0x0f, fill.g & 0x0f, fill.b & 0x0f)
    } else {
        format!("#{:02x}{:02x}{:02x}", fill.r, fill.g, fill.b)
    }
}

/// A number in its shortest SVG spelling.
fn format_number(value: f64, precision: u8) -> String {
    let factor = 10f64.powi(i32::from(precision));
    let rounded = (value * factor).round() / factor;

    // Normalize negative zero, which is valid but never shorter.
    if rounded == 0.0 {
        return "0".to_owned();
    }
    if !rounded.is_finite() {
        return "0".to_owned();
    }

    let mut text = format!("{rounded:.*}", usize::from(precision));
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }

    if let Some(rest) = text.strip_prefix("0.") {
        text = format!(".{rest}");
    } else if let Some(rest) = text.strip_prefix("-0.") {
        text = format!("-.{rest}");
    }
    text
}

/// Join formatted numbers with the fewest separators SVG allows.
fn join(numbers: &[String]) -> String {
    let mut out = String::new();
    for (index, number) in numbers.iter().enumerate() {
        if index > 0 && !number.starts_with('-') {
            out.push(',');
        }
        out.push_str(number);
    }
    out
}

/// The `d` attribute of a path.
fn encode_path(path: &BezPath, precision: u8) -> String {
    let mut encoder = Encoder::new(precision);
    for element in path.elements() {
        encoder.push(*element);
    }
    encoder.out
}

/// Streaming path encoder. Tracks what SVG's relative commands need to know.
struct Encoder {
    precision: u8,
    out: String,
    current: Point,
    subpath_start: Point,
    started: bool,
    /// The previous cubic's second control point, for detecting a smooth
    /// continuation that `s` can express in two numbers instead of four.
    previous_control: Option<Point>,
}

impl Encoder {
    const fn new(precision: u8) -> Self {
        Self {
            precision,
            out: String::new(),
            current: Point::ZERO,
            subpath_start: Point::ZERO,
            started: false,
            previous_control: None,
        }
    }

    fn push(&mut self, element: PathEl) {
        match element {
            PathEl::MoveTo(p) => self.move_to(p),
            PathEl::LineTo(p) => self.line_to(p),
            PathEl::QuadTo(c, end) => {
                // Elevate: SVG has `Q`, but our geometry only ever holds
                // cubics, and mixing the two would complicate `s` detection
                // for no gain.
                let c1 = self.current + (c - self.current) * (2.0 / 3.0);
                let c2 = end + (c - end) * (2.0 / 3.0);
                self.curve_to(c1, c2, end);
            }
            PathEl::CurveTo(c1, c2, end) => self.curve_to(c1, c2, end),
            PathEl::ClosePath => {
                self.out.push('Z');
                // After `Z` the current point returns to the subpath's start,
                // which the next relative command is measured from.
                self.current = self.subpath_start;
                self.previous_control = None;
            }
        }
    }

    fn move_to(&mut self, p: Point) {
        let absolute = format!("M{}", self.coordinate(p));
        if self.started {
            let relative = format!("m{}", self.delta(p));
            self.out.push_str(&shortest([relative, absolute]));
        } else {
            self.out.push_str(&absolute);
            self.started = true;
        }
        self.current = p;
        self.subpath_start = p;
        self.previous_control = None;
    }

    fn line_to(&mut self, p: Point) {
        let mut candidates = Vec::with_capacity(6);
        if approx(p.y, self.current.y) {
            candidates.push(format!("h{}", self.number(p.x - self.current.x)));
            candidates.push(format!("H{}", self.number(p.x)));
        }
        if approx(p.x, self.current.x) {
            candidates.push(format!("v{}", self.number(p.y - self.current.y)));
            candidates.push(format!("V{}", self.number(p.y)));
        }
        candidates.push(format!("l{}", self.delta(p)));
        candidates.push(format!("L{}", self.coordinate(p)));

        self.out.push_str(&shortest(candidates));
        self.current = p;
        self.previous_control = None;
    }

    fn curve_to(&mut self, c1: Point, c2: Point, end: Point) {
        let mut candidates = Vec::with_capacity(4);
        if let Some(previous) = self.previous_control {
            let reflection = Point::new(
                2.0f64.mul_add(self.current.x, -previous.x),
                2.0f64.mul_add(self.current.y, -previous.y),
            );
            if approx(reflection.x, c1.x) && approx(reflection.y, c1.y) {
                candidates.push(format!("s{}", self.deltas(&[c2, end])));
                candidates.push(format!("S{}", self.coordinates(&[c2, end])));
            }
        }
        candidates.push(format!("c{}", self.deltas(&[c1, c2, end])));
        candidates.push(format!("C{}", self.coordinates(&[c1, c2, end])));

        self.out.push_str(&shortest(candidates));
        self.current = end;
        self.previous_control = Some(c2);
    }

    fn number(&self, value: f64) -> String {
        format_number(value, self.precision)
    }

    fn coordinate(&self, p: Point) -> String {
        join(&[self.number(p.x), self.number(p.y)])
    }

    fn delta(&self, p: Point) -> String {
        join(&[
            self.number(p.x - self.current.x),
            self.number(p.y - self.current.y),
        ])
    }

    fn coordinates(&self, points: &[Point]) -> String {
        let numbers: Vec<String> = points
            .iter()
            .flat_map(|p| [self.number(p.x), self.number(p.y)])
            .collect();
        join(&numbers)
    }

    fn deltas(&self, points: &[Point]) -> String {
        let numbers: Vec<String> = points
            .iter()
            .flat_map(|p| {
                [
                    self.number(p.x - self.current.x),
                    self.number(p.y - self.current.y),
                ]
            })
            .collect();
        join(&numbers)
    }
}

/// The shortest of several encodings of the same segment.
///
/// Ties go to the first candidate, and callers list the relative and
/// shorthand forms first. Relative coordinates compress better and are what
/// every downstream SVG optimizer expects, so they are the right default when
/// the byte count is equal.
fn shortest<I: IntoIterator<Item = String>>(candidates: I) -> String {
    candidates
        .into_iter()
        .reduce(|best, candidate| {
            if candidate.len() < best.len() {
                candidate
            } else {
                best
            }
        })
        .unwrap_or_default()
}

/// Coordinate equality at the resolution any SVG consumer can see.
fn approx(a: f64, b: f64) -> bool {
    crate::geom::approx_eq(a, b, 1e-9)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use kurbo::BezPath;

    use super::{WriterOptions, format_number, write_svg};
    use crate::color::Rgba;
    use crate::shapes::{Prim, ShapeDoc, ShapeEl};

    fn doc(shapes: Vec<ShapeEl>) -> ShapeDoc {
        ShapeDoc {
            width: 100,
            height: 80,
            shapes,
        }
    }

    fn red() -> Rgba {
        Rgba::new(255, 0, 0, 255)
    }

    fn opts() -> WriterOptions {
        WriterOptions::default()
    }

    fn body(svg: &str) -> String {
        let start = svg.find('>').expect("a root element") + 1;
        let end = svg.rfind("</svg>").expect("a closing tag");
        svg.get(start..end).expect("a body").to_owned()
    }

    #[test]
    fn writer_empty_doc_emits_only_svg_root_with_viewbox() {
        assert_eq!(
            write_svg(&doc(Vec::new()), &opts()),
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 80"></svg>"#
        );
    }

    #[test]
    fn writer_keep_size_adds_width_and_height() {
        let options = WriterOptions {
            keep_size: true,
            ..opts()
        };
        assert_eq!(
            write_svg(&doc(Vec::new()), &options),
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 80" width="100" height="80"></svg>"#
        );
    }

    #[test]
    fn writer_circle_element_attributes_are_minimal_and_ordered() {
        let svg = write_svg(
            &doc(vec![ShapeEl {
                prim: Prim::Circle {
                    cx: 50.0,
                    cy: 50.0,
                    r: 40.0,
                },
                fill: red(),
            }]),
            &opts(),
        );
        assert_eq!(
            body(&svg),
            r##"<circle cx="50" cy="50" r="40" fill="#f00"/>"##
        );
    }

    #[test]
    fn writer_ellipse_with_rotation_emits_transform() {
        let rotated = ShapeEl {
            prim: Prim::Ellipse {
                cx: 20.0,
                cy: 30.0,
                rx: 10.0,
                ry: 5.0,
                rotate_deg: 30.0,
            },
            fill: red(),
        };
        assert_eq!(
            body(&write_svg(&doc(vec![rotated]), &opts())),
            r##"<ellipse cx="20" cy="30" rx="10" ry="5" fill="#f00" transform="rotate(30 20 30)"/>"##
        );

        let upright = ShapeEl {
            prim: Prim::Ellipse {
                cx: 20.0,
                cy: 30.0,
                rx: 10.0,
                ry: 5.0,
                // Below half a degree: not worth an attribute.
                rotate_deg: 0.2,
            },
            fill: red(),
        };
        assert_eq!(
            body(&write_svg(&doc(vec![upright]), &opts())),
            r##"<ellipse cx="20" cy="30" rx="10" ry="5" fill="#f00"/>"##
        );
    }

    #[test]
    fn writer_rect_rounded_emits_rx_only_when_rx_eq_ry() {
        let sharp = ShapeEl {
            prim: Prim::Rect {
                x: 1.0,
                y: 2.0,
                w: 30.0,
                h: 40.0,
                rx: 0.0,
                ry: 0.0,
            },
            fill: red(),
        };
        assert_eq!(
            body(&write_svg(&doc(vec![sharp]), &opts())),
            r##"<rect x="1" y="2" width="30" height="40" fill="#f00"/>"##
        );

        let symmetric = ShapeEl {
            prim: Prim::Rect {
                x: 1.0,
                y: 2.0,
                w: 30.0,
                h: 40.0,
                rx: 5.0,
                ry: 5.0,
            },
            fill: red(),
        };
        assert_eq!(
            body(&write_svg(&doc(vec![symmetric]), &opts())),
            r##"<rect x="1" y="2" width="30" height="40" rx="5" fill="#f00"/>"##,
            "SVG defaults ry to rx"
        );

        let asymmetric = ShapeEl {
            prim: Prim::Rect {
                x: 1.0,
                y: 2.0,
                w: 30.0,
                h: 40.0,
                rx: 5.0,
                ry: 3.0,
            },
            fill: red(),
        };
        assert_eq!(
            body(&write_svg(&doc(vec![asymmetric]), &opts())),
            r##"<rect x="1" y="2" width="30" height="40" rx="5" ry="3" fill="#f00"/>"##
        );
    }

    #[test]
    fn writer_path_uses_relative_commands_and_shorthands() {
        let mut path = BezPath::new();
        path.move_to((10.0, 10.0));
        path.line_to((30.0, 10.0)); // horizontal
        path.line_to((30.0, 25.0)); // vertical
        path.line_to((12.0, 18.0)); // neither
        path.close_path();

        let svg = write_svg(
            &doc(vec![ShapeEl {
                prim: Prim::Path(path),
                fill: red(),
            }]),
            &opts(),
        );
        assert_eq!(
            body(&svg),
            r##"<path d="M10,10h20v15l-18-7Z" fill="#f00"/>"##,
            "horizontal and vertical runs use h and v; the rest goes relative"
        );
    }

    #[test]
    fn writer_path_uses_the_smooth_shorthand_for_a_continued_curve() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.curve_to((10.0, 0.0), (20.0, 10.0), (20.0, 20.0));
        // The first control point mirrors the previous second control point.
        path.curve_to((20.0, 30.0), (30.0, 40.0), (40.0, 40.0));
        path.close_path();

        let svg = write_svg(
            &doc(vec![ShapeEl {
                prim: Prim::Path(path),
                fill: red(),
            }]),
            &opts(),
        );
        let data = body(&svg);
        assert!(data.contains('s') || data.contains('S'), "{data}");
    }

    #[test]
    fn writer_path_omits_a_separator_before_a_negative_number() {
        let mut path = BezPath::new();
        path.move_to((10.0, -5.0));
        path.line_to((-3.0, -8.0));
        path.close_path();

        let svg = write_svg(
            &doc(vec![ShapeEl {
                prim: Prim::Path(path),
                fill: red(),
            }]),
            &opts(),
        );
        // `L-3-8` is a byte shorter than `l-13-3` here, so the absolute form
        // wins. Either way, no separator precedes a minus sign.
        assert_eq!(body(&svg), r##"<path d="M10-5L-3-8Z" fill="#f00"/>"##);
    }

    #[test]
    fn writer_skips_an_empty_path() {
        let svg = write_svg(
            &doc(vec![ShapeEl {
                prim: Prim::Path(BezPath::new()),
                fill: red(),
            }]),
            &opts(),
        );
        assert_eq!(body(&svg), "");
    }

    #[test]
    fn writer_number_formatting_strips_trailing_zeros_and_leading_zero() {
        assert_eq!(format_number(10.50, 2), "10.5");
        assert_eq!(format_number(10.0, 2), "10");
        assert_eq!(format_number(0.5, 2), ".5");
        assert_eq!(format_number(-0.5, 2), "-.5");
        assert_eq!(format_number(0.0, 2), "0");
        assert_eq!(
            format_number(-0.0, 2),
            "0",
            "negative zero is never shorter"
        );
        assert_eq!(format_number(f64::NAN, 2), "0", "no NaN reaches the output");
    }

    #[test]
    fn writer_number_precision_respected() {
        assert_eq!(format_number(4.56789, 0), "5");
        assert_eq!(format_number(4.56789, 1), "4.6");
        assert_eq!(format_number(4.56789, 2), "4.57");
        assert_eq!(format_number(4.56789, 4), "4.5679");

        let shape = ShapeEl {
            prim: Prim::Circle {
                cx: 1.23456,
                cy: 2.0,
                r: 0.98765,
            },
            fill: red(),
        };
        let coarse = WriterOptions {
            precision: 1,
            ..opts()
        };
        assert_eq!(
            body(&write_svg(&doc(vec![shape.clone()]), &coarse)),
            r##"<circle cx="1.2" cy="2" r="1" fill="#f00"/>"##
        );

        // Above the cap the extra digits are silently dropped rather than
        // producing coordinates no renderer can use.
        let absurd = WriterOptions {
            precision: 200,
            ..opts()
        };
        assert_eq!(
            body(&write_svg(&doc(vec![shape]), &absurd)),
            r##"<circle cx="1.23456" cy="2" r=".98765" fill="#f00"/>"##
        );
    }

    #[test]
    fn writer_fill_shortens_to_3_digit_hex_when_possible() {
        for (color, expected) in [
            (Rgba::new(255, 0, 0, 255), "#f00"),
            (Rgba::new(255, 255, 255, 255), "#fff"),
            (Rgba::new(0x33, 0x66, 0xff, 255), "#36f"),
            (Rgba::new(0x12, 0x34, 0x56, 255), "#123456"),
            (Rgba::new(0xff, 0x00, 0x01, 255), "#ff0001"),
        ] {
            let svg = write_svg(
                &doc(vec![ShapeEl {
                    prim: Prim::Circle {
                        cx: 0.0,
                        cy: 0.0,
                        r: 1.0,
                    },
                    fill: color,
                }]),
                &opts(),
            );
            assert!(
                svg.contains(&format!("fill=\"{expected}\"")),
                "{color:?} should be {expected}, got {svg}"
            );
        }
    }

    #[test]
    fn writer_fill_opacity_emitted_only_for_alpha_lt_255() {
        let opaque = write_svg(
            &doc(vec![ShapeEl {
                prim: Prim::Circle {
                    cx: 0.0,
                    cy: 0.0,
                    r: 1.0,
                },
                fill: Rgba::new(255, 0, 0, 255),
            }]),
            &opts(),
        );
        assert!(!opaque.contains("fill-opacity"));

        let translucent = write_svg(
            &doc(vec![ShapeEl {
                prim: Prim::Circle {
                    cx: 0.0,
                    cy: 0.0,
                    r: 1.0,
                },
                fill: Rgba::new(255, 0, 0, 128),
            }]),
            &opts(),
        );
        assert!(
            translucent.contains(r#"fill-opacity=".502""#),
            "{translucent}"
        );
    }

    #[test]
    fn writer_preserves_paint_order() {
        let shapes = vec![
            ShapeEl {
                prim: Prim::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 10.0,
                    h: 10.0,
                    rx: 0.0,
                    ry: 0.0,
                },
                fill: Rgba::new(255, 0, 0, 255),
            },
            ShapeEl {
                prim: Prim::Circle {
                    cx: 5.0,
                    cy: 5.0,
                    r: 3.0,
                },
                fill: Rgba::new(0, 0, 255, 255),
            },
        ];
        let svg = write_svg(&doc(shapes), &opts());
        let rect = svg.find("<rect").expect("a rect");
        let circle = svg.find("<circle").expect("a circle");
        assert!(rect < circle, "the first shape is drawn first");
    }

    #[test]
    fn writer_never_emits_a_group() {
        // Two shapes with the same fill: a merging writer would group them.
        let same_fill = vec![
            ShapeEl {
                prim: Prim::Circle {
                    cx: 1.0,
                    cy: 1.0,
                    r: 1.0,
                },
                fill: red(),
            },
            ShapeEl {
                prim: Prim::Circle {
                    cx: 5.0,
                    cy: 5.0,
                    r: 1.0,
                },
                fill: red(),
            },
        ];
        let svg = write_svg(&doc(same_fill), &opts());
        assert!(!svg.contains("<g"), "ADR-0005: no groups in v1: {svg}");
    }

    #[test]
    fn writer_emits_no_prolog_comment_or_metadata() {
        let svg = write_svg(&doc(Vec::new()), &opts());
        assert!(!svg.contains("<?xml"));
        assert!(!svg.contains("<!--"));
        assert!(!svg.contains("<metadata"));
        assert!(svg.starts_with("<svg "));
    }

    // --- validity oracle -------------------------------------------------------

    /// usvg lowers every primitive to a path, and wraps anything carrying a
    /// transform in a group, so counting shapes means walking the tree.
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

    fn sample_doc() -> ShapeDoc {
        let mut path = BezPath::new();
        path.move_to((5.0, 5.0));
        path.line_to((25.0, 5.0));
        path.curve_to((30.0, 10.0), (30.0, 20.0), (25.0, 25.0));
        path.line_to((5.0, 25.0));
        path.close_path();

        ShapeDoc {
            width: 120,
            height: 90,
            shapes: vec![
                ShapeEl {
                    prim: Prim::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 120.0,
                        h: 90.0,
                        rx: 0.0,
                        ry: 0.0,
                    },
                    fill: Rgba::new(255, 255, 255, 255),
                },
                ShapeEl {
                    prim: Prim::Circle {
                        cx: 60.0,
                        cy: 45.0,
                        r: 20.5,
                    },
                    fill: Rgba::new(255, 0, 0, 255),
                },
                ShapeEl {
                    prim: Prim::Ellipse {
                        cx: 95.0,
                        cy: 30.0,
                        rx: 15.0,
                        ry: 7.5,
                        rotate_deg: 20.0,
                    },
                    fill: Rgba::new(0, 0x80, 0, 255),
                },
                ShapeEl {
                    prim: Prim::Rect {
                        x: 4.0,
                        y: 60.0,
                        w: 40.0,
                        h: 24.0,
                        rx: 6.0,
                        ry: 6.0,
                    },
                    fill: Rgba::new(0, 0, 255, 200),
                },
                ShapeEl {
                    prim: Prim::Path(path),
                    fill: Rgba::new(0x33, 0x66, 0xff, 255),
                },
            ],
        }
    }

    #[test]
    fn writer_output_parses_with_usvg() {
        let doc = sample_doc();
        let svg = write_svg(&doc, &opts());

        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default())
            .expect("the writer emits parseable SVG");

        assert_eq!(
            count_paths(tree.root()),
            doc.shapes.len(),
            "every shape survives parsing"
        );

        let size = tree.size();
        assert!((size.width() - 120.0).abs() < 0.01);
        assert!((size.height() - 90.0).abs() < 0.01);
    }

    #[test]
    fn writer_output_parses_with_usvg_at_every_precision() {
        for precision in 0..=super::MAX_PRECISION {
            let options = WriterOptions {
                precision,
                keep_size: precision % 2 == 0,
            };
            let svg = write_svg(&sample_doc(), &options);
            usvg::Tree::from_str(&svg, &usvg::Options::default())
                .unwrap_or_else(|error| unreachable!("precision {precision}: {error}: {svg}"));
        }
    }

    // --- snapshots --------------------------------------------------------------

    #[test]
    fn writer_snapshot_of_every_primitive() {
        insta::assert_snapshot!(write_svg(&sample_doc(), &opts()));
    }

    #[test]
    fn writer_snapshot_with_keep_size_and_full_precision() {
        let options = WriterOptions {
            precision: super::MAX_PRECISION,
            keep_size: true,
        };
        insta::assert_snapshot!(write_svg(&sample_doc(), &options));
    }
}
