//! The four calibration fixtures and six degradations of
//! `CLEANUP_IMPLEMENTATION_PLAN.md` §10.7.
//!
//! Every number in that plan's §9 came from these drawings. The crate's own
//! unit tests include this file through a `#[path]` module in
//! `src/cleanup/mod.rs`, so there is one drawing of each fixture, not two.
//!
//! All four are 256x256, anti-aliased, on an opaque white background.

#![allow(dead_code, unreachable_pub, clippy::expect_used)]

use tiny_skia::{Color, FillRule, Paint, Path, PathBuilder, Pixmap, Rect, Transform};
use vtracer::ColorImage;

/// One of the four drawings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fixture {
    /// One anti-aliased circle. The easiest possible win for shape detection.
    Disc,
    /// An axis-aligned rect with a circle on top: two primitives, two colours.
    /// Its clean `edge_width` is 0.80, because the rect has hard edges.
    Badge,
    /// A 14-vertex star. No primitive will ever match it, so this is where a
    /// preprocessor can do the most damage.
    Star,
    /// Four colours, mixed geometry: the realistic "logo" case.
    Logo,
}

impl Fixture {
    /// Every fixture, in the order §9 reports them.
    pub const ALL: [Self; 4] = [Self::Disc, Self::Badge, Self::Star, Self::Logo];

    /// The name §9 uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Disc => "disc",
            Self::Badge => "badge",
            Self::Star => "star",
            Self::Logo => "logo",
        }
    }

    /// The drawing, pristine.
    pub fn pixmap(self) -> Pixmap {
        match self {
            Self::Disc => disc(),
            Self::Badge => badge(),
            Self::Star => star(),
            Self::Logo => logo(),
        }
    }

    /// The drawing, pristine, as the tracer's image type.
    pub fn pristine(self) -> ColorImage {
        pristine(&self.pixmap())
    }
}

/// One way of damaging a fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Degradation {
    /// The pristine fixture.
    Clean,
    /// Gaussian blur, sigma 1.5.
    Blur15,
    /// Gaussian blur, sigma 3.
    Blur3,
    /// A low-resolution asset somebody stretched: downscaled to a third and
    /// back, both with a triangle filter.
    Soft,
    /// JPEG round-trip at quality 30.
    Jpeg30,
    /// Blur at sigma 1.5, then JPEG at quality 50.
    Blur15Jpeg50,
}

impl Degradation {
    /// Every degradation, in the order §9 reports them.
    pub const ALL: [Self; 6] = [
        Self::Clean,
        Self::Blur15,
        Self::Blur3,
        Self::Soft,
        Self::Jpeg30,
        Self::Blur15Jpeg50,
    ];

    /// The degradations that blur: `edge_width` must find every one of them.
    pub const BLURRED: [Self; 3] = [Self::Blur15, Self::Blur3, Self::Soft];

    /// The degradations that add noise: `flat_noise` must find every one.
    pub const NOISY: [Self; 2] = [Self::Jpeg30, Self::Blur15Jpeg50];

    /// The name §9 uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Blur15 => "blur1.5",
            Self::Blur3 => "blur3",
            Self::Soft => "soft",
            Self::Jpeg30 => "jpeg30",
            Self::Blur15Jpeg50 => "blur1.5+jpeg50",
        }
    }
}

/// A fixture after a degradation.
pub fn degrade(fixture: Fixture, degradation: Degradation) -> ColorImage {
    let pristine = fixture.pristine();
    let buf = to_rgba(&pristine);
    match degradation {
        Degradation::Clean => pristine,
        Degradation::Blur15 => from_rgba(&image::imageops::blur(&buf, 1.5)),
        Degradation::Blur3 => from_rgba(&image::imageops::blur(&buf, 3.0)),
        Degradation::Soft => {
            let small = image::imageops::resize(
                &buf,
                buf.width() / 3,
                buf.height() / 3,
                image::imageops::FilterType::Triangle,
            );
            from_rgba(&image::imageops::resize(
                &small,
                buf.width(),
                buf.height(),
                image::imageops::FilterType::Triangle,
            ))
        }
        Degradation::Jpeg30 => jpeg(&buf, 30),
        Degradation::Blur15Jpeg50 => jpeg(&image::imageops::blur(&buf, 1.5), 50),
    }
}

/// JPEG round-trip in memory. This is what produces the ringing that makes
/// `flat_noise` the deciding signal.
pub fn jpeg(buf: &image::RgbaImage, quality: u8) -> ColorImage {
    let rgb = image::DynamicImage::ImageRgba8(buf.clone()).to_rgb8();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
        .encode_image(&rgb)
        .expect("jpeg encodes");
    from_rgba(
        &image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
            .expect("jpeg decodes")
            .to_rgba8(),
    )
}

/// `tiny-skia` stores premultiplied RGBA; the tracer's type is straight RGBA.
pub fn pristine(pixmap: &Pixmap) -> ColorImage {
    let mut pixels = Vec::with_capacity((pixmap.width() * pixmap.height()) as usize * 4);
    for pixel in pixmap.pixels() {
        let straight = pixel.demultiply();
        pixels.extend_from_slice(&[
            straight.red(),
            straight.green(),
            straight.blue(),
            straight.alpha(),
        ]);
    }
    ColorImage {
        pixels,
        width: pixmap.width() as usize,
        height: pixmap.height() as usize,
    }
}

/// `ColorImage` and `image`'s buffer hold the same straight RGBA bytes, so
/// this and [`from_rgba`] are moves of the same data, not conversions.
pub fn to_rgba(image: &ColorImage) -> image::RgbaImage {
    image::ImageBuffer::from_raw(
        u32::try_from(image.width).expect("width fits"),
        u32::try_from(image.height).expect("height fits"),
        image.pixels.clone(),
    )
    .expect("a buffer of the right size")
}

/// See [`to_rgba`].
pub fn from_rgba(buf: &image::RgbaImage) -> ColorImage {
    ColorImage {
        pixels: buf.as_raw().clone(),
        width: buf.width() as usize,
        height: buf.height() as usize,
    }
}

/// An image encoded as PNG, for the tests that go through the file system.
pub fn png_bytes(image: &ColorImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    to_rgba(image)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("png encodes");
    out.into_inner()
}

fn paint_rgb(r: u8, g: u8, b: u8) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(r, g, b, 255);
    paint.anti_alias = true;
    paint
}

fn white_canvas() -> Pixmap {
    let mut pixmap = Pixmap::new(256, 256).expect("pixmap");
    pixmap.fill(Color::WHITE);
    pixmap
}

fn fill(pixmap: &mut Pixmap, path: &Path, paint: &Paint) {
    pixmap.fill_path(path, paint, FillRule::Winding, Transform::identity(), None);
}

fn disc() -> Pixmap {
    let mut pixmap = white_canvas();
    let mut builder = PathBuilder::new();
    builder.push_circle(128.0, 128.0, 90.0);
    fill(
        &mut pixmap,
        &builder.finish().expect("path"),
        &paint_rgb(214, 40, 40),
    );
    pixmap
}

fn badge() -> Pixmap {
    let mut pixmap = white_canvas();
    fill(
        &mut pixmap,
        &PathBuilder::from_rect(Rect::from_xywh(32.0, 48.0, 192.0, 160.0).expect("rect")),
        &paint_rgb(34, 87, 160),
    );
    let mut builder = PathBuilder::new();
    builder.push_circle(128.0, 128.0, 48.0);
    fill(
        &mut pixmap,
        &builder.finish().expect("path"),
        &paint_rgb(250, 204, 21),
    );
    pixmap
}

fn star() -> Pixmap {
    let mut pixmap = white_canvas();
    let mut builder = PathBuilder::new();
    let (cx, cy) = (128.0_f32, 128.0_f32);
    for index in 0..14_u8 {
        let radius = if index % 2 == 0 { 100.0 } else { 44.0 };
        let angle =
            std::f32::consts::PI * 2.0 * f32::from(index) / 14.0 - std::f32::consts::FRAC_PI_2;
        let (x, y) = (cx + radius * angle.cos(), cy + radius * angle.sin());
        if index == 0 {
            builder.move_to(x, y);
        } else {
            builder.line_to(x, y);
        }
    }
    builder.close();
    fill(
        &mut pixmap,
        &builder.finish().expect("path"),
        &paint_rgb(16, 122, 94),
    );
    pixmap
}

fn logo() -> Pixmap {
    let mut pixmap = white_canvas();
    fill(
        &mut pixmap,
        &PathBuilder::from_rect(Rect::from_xywh(24.0, 24.0, 96.0, 96.0).expect("rect")),
        &paint_rgb(220, 38, 38),
    );
    let mut circle = PathBuilder::new();
    circle.push_circle(184.0, 72.0, 48.0);
    fill(
        &mut pixmap,
        &circle.finish().expect("path"),
        &paint_rgb(37, 99, 235),
    );
    let mut triangle = PathBuilder::new();
    triangle.move_to(72.0, 232.0);
    triangle.line_to(24.0, 152.0);
    triangle.line_to(120.0, 152.0);
    triangle.close();
    fill(
        &mut pixmap,
        &triangle.finish().expect("path"),
        &paint_rgb(22, 163, 74),
    );
    let mut ellipse = PathBuilder::new();
    ellipse.push_oval(Rect::from_xywh(136.0, 152.0, 96.0, 64.0).expect("oval"));
    fill(
        &mut pixmap,
        &ellipse.finish().expect("path"),
        &paint_rgb(147, 51, 234),
    );
    pixmap
}
