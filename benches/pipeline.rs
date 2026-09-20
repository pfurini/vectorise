//! Where the time goes.
//!
//! Not a gate: these numbers move with the machine, and a regression in them is
//! a reason to look, not a reason to fail a build. `docs/perf.md` records the
//! baseline they are compared against.
//!
//! Run with `cargo bench`.

#![allow(clippy::expect_used, clippy::cast_possible_truncation)]

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};
use vtracer::ColorImage;

use vectorise::shapes::{ShapeFitOptions, fit_shapes};
use vectorise::trace::{TraceOptions, trace};
use vectorise::writer::{WriterOptions, write_svg};

/// A poster-like scene: a background, a disc, a rectangle, and a star.
fn poster(size: u32) -> ColorImage {
    let mut pixmap = Pixmap::new(size, size).expect("pixmap");
    pixmap.fill(Color::WHITE);

    let scale = f64::from(size) / 512.0;
    let at = |value: f64| (value * scale) as f32;
    let mut paint = Paint {
        anti_alias: false,
        ..Paint::default()
    };

    let mut builder = PathBuilder::new();
    builder.push_circle(at(140.0), at(150.0), at(90.0));
    paint.set_color_rgba8(220, 30, 40, 255);
    fill(&mut pixmap, &builder, &paint);

    let mut builder = PathBuilder::new();
    builder.push_rect(Rect::from_xywh(at(280.0), at(60.0), at(180.0), at(120.0)).expect("rect"));
    paint.set_color_rgba8(30, 80, 220, 255);
    fill(&mut pixmap, &builder, &paint);

    let mut builder = PathBuilder::new();
    let (cx, cy) = (at(256.0), at(360.0));
    for index in 0..10_u8 {
        let radius = if index % 2 == 0 { at(110.0) } else { at(45.0) };
        let angle = std::f32::consts::PI * f32::from(index) / 5.0 - std::f32::consts::FRAC_PI_2;
        let (x, y) = (
            radius.mul_add(angle.cos(), cx),
            radius.mul_add(angle.sin(), cy),
        );
        if index == 0 {
            builder.move_to(x, y);
        } else {
            builder.line_to(x, y);
        }
    }
    builder.close();
    paint.set_color_rgba8(240, 180, 20, 255);
    fill(&mut pixmap, &builder, &paint);

    ColorImage {
        pixels: pixmap
            .pixels()
            .iter()
            .flat_map(|pixel| {
                let c = pixel.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect(),
        width: size as usize,
        height: size as usize,
    }
}

fn fill(pixmap: &mut Pixmap, builder: &PathBuilder, paint: &Paint) {
    pixmap.fill_path(
        &builder.clone().finish().expect("path"),
        paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn bench_stages(c: &mut Criterion) {
    let image = poster(512);
    let traced = trace(&image, &TraceOptions::default()).expect("traces");
    let fitted = fit_shapes(&traced, &ShapeFitOptions::default());
    let written = write_svg(&fitted, &WriterOptions::default());

    let mut group = c.benchmark_group("stage");
    group.bench_function("trace 512x512 poster", |b| {
        b.iter(|| trace(black_box(&image), &TraceOptions::default()).expect("traces"));
    });
    group.bench_function("shape pass", |b| {
        b.iter(|| fit_shapes(black_box(&traced), &ShapeFitOptions::default()));
    });
    group.bench_function("write", |b| {
        b.iter(|| write_svg(black_box(&fitted), &WriterOptions::default()));
    });
    group.bench_function("optimize", |b| {
        b.iter(|| vectorise::optimize::optimize(black_box(&written)).expect("optimizes"));
    });
    group.finish();
}

fn bench_whole_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline");
    for size in [256_u32, 512, 1024] {
        let image = poster(size);
        group.bench_with_input(BenchmarkId::from_parameter(size), &image, |b, image| {
            b.iter(|| {
                let traced = trace(black_box(image), &TraceOptions::default()).expect("traces");
                let fitted = fit_shapes(&traced, &ShapeFitOptions::default());
                let written = write_svg(&fitted, &WriterOptions::default());
                vectorise::optimize::optimize(&written).expect("optimizes")
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_stages, bench_whole_pipeline);
criterion_main!(benches);
