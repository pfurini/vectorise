//! Test fixtures, generated in code.
//!
//! Nothing here is committed as a binary blob. A fixture you cannot regenerate
//! is a fixture nobody can change, and every one of these is three lines of
//! `tiny-skia` away from being redrawn.

#![allow(dead_code, unreachable_pub, clippy::expect_used)]

use std::path::{Path, PathBuf};

use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

/// A white canvas with a red disc, large enough to be detected as a circle.
#[must_use]
pub fn disc_png() -> Vec<u8> {
    let mut pixmap = canvas(80, 80);
    let mut builder = PathBuilder::new();
    builder.push_circle(40.0, 40.0, 25.0);
    fill(&mut pixmap, &builder, (220, 30, 40));
    pixmap.encode_png().expect("png")
}

/// A white canvas with a blue rectangle.
#[must_use]
pub fn rect_png() -> Vec<u8> {
    let mut pixmap = canvas(80, 60);
    let mut builder = PathBuilder::new();
    builder.push_rect(Rect::from_xywh(12.0, 10.0, 50.0, 36.0).expect("rect"));
    fill(&mut pixmap, &builder, (30, 80, 220));
    pixmap.encode_png().expect("png")
}

/// The disc, as a JPEG, for the tests that care about more than one format.
#[must_use]
pub fn disc_jpeg() -> Vec<u8> {
    let png = disc_png();
    let decoded = image::load_from_memory(&png).expect("decode");
    let mut out = std::io::Cursor::new(Vec::new());
    decoded
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .expect("encode");
    out.into_inner()
}

/// Bytes that are not an image at all.
#[must_use]
pub fn corrupt() -> Vec<u8> {
    b"PNG? no. This is not an image.".to_vec()
}

/// Write `contents` into `directory` and return the path.
pub fn place(directory: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = directory.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(&path, contents).expect("write");
    path
}

fn canvas(width: u32, height: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(width, height).expect("pixmap");
    pixmap.fill(Color::WHITE);
    pixmap
}

fn fill(pixmap: &mut Pixmap, builder: &PathBuilder, color: (u8, u8, u8)) {
    let mut paint = Paint {
        anti_alias: false,
        ..Paint::default()
    };
    paint.set_color_rgba8(color.0, color.1, color.2, 255);
    pixmap.fill_path(
        &builder.clone().finish().expect("path"),
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}
