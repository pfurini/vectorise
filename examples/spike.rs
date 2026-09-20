//! Phase 0 throwaway spike. Answers every VERIFY item in IMPLEMENTATION_PLAN.md
//! §Phase 0 by exercising the pinned APIs. Deleted at the end of Phase 1.

use std::collections::BTreeSet;

use kurbo::{Affine, BezPath, Circle, Ellipse, ParamCurveMoments, Point, Rect, Shape, Vec2};
use tiny_skia::{Color as SkColor, FillRule, Paint as SkPaint, PathBuilder, Pixmap, Transform};
use vtracer::ir::{PathCmd, VectorDoc};
use vtracer::{ColorImage, Config, FitMode, Preset};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tiny_skia_fixture();
    let png = fixture_png();
    let img = decode(&png);
    println!("== decoded {}x{} pixels={}", img.width, img.height, img.pixels.len());

    trace_defaults(&img)?;
    trace_simplify(&img)?;
    trace_max_colors(&img)?;
    trace_alpha()?;
    kurbo_checks();
    oxvg_check();
    Ok(())
}

// --- fixture generation (also verifies tiny-skia's Pixmap/PathBuilder API) ---

fn fixture_pixmap() -> Pixmap {
    let mut pm = Pixmap::new(128, 128).expect("128x128 pixmap");
    pm.fill(SkColor::WHITE);

    let mut paint = SkPaint::default();
    paint.anti_alias = false;

    let mut pb = PathBuilder::new();
    pb.push_circle(64.0, 48.0, 32.0);
    let circle = pb.finish().expect("circle path");
    paint.set_color_rgba8(255, 0, 0, 255);
    pm.fill_path(&circle, &paint, FillRule::Winding, Transform::identity(), None);

    let mut pb = PathBuilder::new();
    pb.push_rect(tiny_skia::Rect::from_xywh(16.0, 96.0, 40.0, 24.0).expect("rect"));
    let rect = pb.finish().expect("rect path");
    paint.set_color_rgba8(0, 0, 255, 255);
    pm.fill_path(&rect, &paint, FillRule::Winding, Transform::identity(), None);

    pm
}

fn tiny_skia_fixture() {
    let pm = fixture_pixmap();
    // Pixmap::data() is straight RGBA? tiny-skia stores premultiplied.
    let px = pm.pixel(64, 48).expect("center pixel");
    println!(
        "== tiny-skia: center px demul=({},{},{},{}) data_len={}",
        px.red(),
        px.green(),
        px.blue(),
        px.alpha(),
        pm.data().len()
    );
}

fn fixture_png() -> Vec<u8> {
    let pm = fixture_pixmap();
    let rgba: Vec<u8> = pm
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    let buf = image::RgbaImage::from_raw(128, 128, rgba).expect("raw buffer");
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(buf)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("png encode");
    out.into_inner()
}

fn decode(png: &[u8]) -> ColorImage {
    let dynamic = image::load_from_memory(png).expect("decode");
    let rgba = dynamic.to_rgba8();
    ColorImage {
        width: rgba.width() as usize,
        height: rgba.height() as usize,
        pixels: rgba.into_raw(),
    }
}

// --- vtracer -----------------------------------------------------------------

fn describe(label: &str, doc: &VectorDoc) {
    let mut colors = BTreeSet::new();
    let mut cmds = 0usize;
    let mut subpaths = 0usize;
    let mut multi = 0usize;
    for shape in &doc.shapes {
        let c = shape.paint.color();
        colors.insert((c.r, c.g, c.b, c.a));
        subpaths += shape.path.subpaths.len();
        if shape.path.subpaths.len() > 1 {
            multi += 1;
        }
        for sub in &shape.path.subpaths {
            cmds += sub.commands.len();
        }
    }
    println!(
        "== {label}: {}x{} shapes={} subpaths={} multi-subpath={} colors={} cmds={}",
        doc.width,
        doc.height,
        doc.shapes.len(),
        subpaths,
        multi,
        colors.len(),
        cmds
    );
    println!("   colors: {colors:?}");
    for (i, shape) in doc.shapes.iter().enumerate() {
        let areas: Vec<String> = shape
            .path
            .subpaths
            .iter()
            .map(|s| format!("{:.2}", subpath_to_bezpath(s).area()))
            .collect();
        println!("   shape[{i}] signed subpath areas (kurbo): {areas:?}");
    }
}

/// The Phase 5 `geom/convert.rs` conversion, sketched here to check winding.
fn subpath_to_bezpath(sub: &vtracer::ir::SubPath) -> BezPath {
    let mut p = BezPath::new();
    for cmd in &sub.commands {
        match *cmd {
            PathCmd::MoveTo(q) => p.move_to((q.x, q.y)),
            PathCmd::LineTo(q) => p.line_to((q.x, q.y)),
            PathCmd::CubicTo(a, b, e) => p.curve_to((a.x, a.y), (b.x, b.y), (e.x, e.y)),
            PathCmd::Close => p.close_path(),
        }
    }
    p
}

fn trace_defaults(img: &ColorImage) -> Result<(), vtracer::Error> {
    let cfg = Config::default();
    println!("== Config::default() = {cfg:?}");
    let pipeline = cfg.build()?;
    let doc = pipeline.run(img)?;
    describe("default", &doc);

    if let Some(shape) = doc.shapes.first() {
        let sub = &shape.path.subpaths[0];
        let head: Vec<String> = sub
            .commands
            .iter()
            .take(4)
            .map(|c| match c {
                PathCmd::MoveTo(p) => format!("M({:.2},{:.2})", p.x, p.y),
                PathCmd::LineTo(p) => format!("L({:.2},{:.2})", p.x, p.y),
                PathCmd::CubicTo(a, b, e) => format!(
                    "C({:.2},{:.2} {:.2},{:.2} {:.2},{:.2})",
                    a.x, a.y, b.x, b.y, e.x, e.y
                ),
                PathCmd::Close => "Z".to_string(),
            })
            .collect();
        println!("   first subpath head: {head:?}");
        println!(
            "   last cmd is Close: {}",
            matches!(sub.commands.last(), Some(PathCmd::Close))
        );
    }

    let svg = pipeline.to_svg(img)?;
    println!("== to_svg len={} head={:?}", svg.len(), &svg[..svg.len().min(220)]);

    // Preset coverage.
    for preset in [Preset::Bw, Preset::Poster, Preset::Photo] {
        let c = Config::from_preset(preset);
        println!(
            "== preset {preset:?}: clustering={:?} color_precision={} filter_speckle={} layer_difference={} corner_threshold={} mode={:?}",
            c.clustering, c.color_precision, c.filter_speckle, c.layer_difference, c.corner_threshold, c.mode
        );
    }
    Ok(())
}

fn trace_simplify(img: &ColorImage) -> Result<(), vtracer::Error> {
    for tol in [0.0_f64, 1.0, 2.5] {
        let mut cfg = Config::default();
        cfg.mode = FitMode::Spline;
        cfg.simplify = if tol > 0.0 { Some(tol) } else { None };
        let doc = cfg.build()?.run(img)?;
        describe(&format!("simplify={tol}"), &doc);
    }
    Ok(())
}

fn trace_max_colors(img: &ColorImage) -> Result<(), vtracer::Error> {
    let mut cfg = Config::default();
    cfg.max_colors = Some(2);
    let doc = cfg.build()?.run(img)?;
    describe("max_colors=2", &doc);

    let mut cfg = Config::default();
    cfg.palette = vec![
        vtracer::Color::new(255, 255, 255),
        vtracer::Color::new(0, 0, 0),
    ];
    let doc = cfg.build()?.run(img)?;
    describe("palette=[white,black]", &doc);
    Ok(())
}

/// Does vtracer key transparent pixels itself? Build an image whose outer ring
/// is fully transparent and see whether a background shape survives.
fn trace_alpha() -> Result<(), vtracer::Error> {
    let mut img = ColorImage::new_w_h(64, 64);
    for y in 0..64 {
        for x in 0..64 {
            let inside = (16..48).contains(&x) && (16..48).contains(&y);
            let c = if inside {
                vtracer::Color::new_rgba(255, 0, 0, 255)
            } else {
                vtracer::Color::new_rgba(0, 0, 0, 0)
            };
            img.set_pixel(x, y, &c);
        }
    }
    let doc = Config::default().build()?.run(&img)?;
    describe("alpha: transparent border around red square", &doc);

    // Same geometry, opaque white border: expect an extra background shape.
    let mut img2 = ColorImage::new_w_h(64, 64);
    for y in 0..64 {
        for x in 0..64 {
            let inside = (16..48).contains(&x) && (16..48).contains(&y);
            let c = if inside {
                vtracer::Color::new_rgba(255, 0, 0, 255)
            } else {
                vtracer::Color::new_rgba(255, 255, 255, 255)
            };
            img2.set_pixel(x, y, &c);
        }
    }
    let doc2 = Config::default().build()?.run(&img2)?;
    describe("alpha: opaque white border around red square", &doc2);

    // 1x1 must not panic.
    let mut tiny = ColorImage::new_w_h(1, 1);
    tiny.set_pixel(0, 0, &vtracer::Color::new(1, 2, 3));
    match Config::default().build()?.run(&tiny) {
        Ok(d) => println!("== 1x1 ok: shapes={}", d.shapes.len()),
        Err(e) => println!("== 1x1 err: {e}"),
    }

    // 0x0 error variant.
    let empty = ColorImage::new_w_h(0, 0);
    println!("== 0x0: {:?}", Config::default().build()?.run(&empty).err());
    Ok(())
}

// --- kurbo -------------------------------------------------------------------

fn kurbo_checks() {
    let circle = Circle::new(Point::new(10.0, 20.0), 5.0);
    let ccw: BezPath = circle.to_path(1e-6);
    let area = ccw.area();
    println!(
        "== kurbo Circle r=5 area={area:.6} (pi*r^2={:.6}) sign={}",
        std::f64::consts::PI * 25.0,
        area.signum()
    );

    let flipped = Affine::scale_non_uniform(1.0, -1.0) * ccw.clone();
    println!("   y-flipped (opposite winding) area={:.6}", flipped.area());

    // Moments: trait is implemented on paths, not on individual segments.
    let e = Ellipse::new(Point::new(30.0, 40.0), Vec2::new(20.0, 8.0), 0.0);
    let p: BezPath = e.to_path(1e-4);
    let m = p.moments();
    let a = p.area();
    let cx = m.moment_x / a;
    let cy = m.moment_y / a;
    let mxx = m.moment_xx / a - cx * cx;
    let myy = m.moment_yy / a - cy * cy;
    let mxy = m.moment_xy / a - cx * cy;
    // For an ellipse: central second moments are rx^2/4 and ry^2/4.
    println!(
        "== kurbo ellipse rx=20 ry=8: area={a:.4} (pi*rx*ry={:.4}) centroid=({cx:.4},{cy:.4}) rx'={:.4} ry'={:.4} mxy={mxy:.6}",
        std::f64::consts::PI * 160.0,
        2.0 * mxx.sqrt(),
        2.0 * myy.sqrt()
    );

    let rot = Ellipse::new(Point::new(0.0, 0.0), Vec2::new(20.0, 8.0), 0.6);
    let pr: BezPath = rot.to_path(1e-4);
    let mr = pr.moments();
    let ar = pr.area();
    let rcx = mr.moment_x / ar;
    let rcy = mr.moment_y / ar;
    let sxx = mr.moment_xx / ar - rcx * rcx;
    let syy = mr.moment_yy / ar - rcy * rcy;
    let sxy = mr.moment_xy / ar - rcx * rcy;
    let theta = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    println!(
        "== kurbo rotated ellipse (x_rotation=0.6): recovered theta={theta:.6} rad ({:.3} deg)",
        theta.to_degrees()
    );

    let r = Rect::new(0.0, 0.0, 10.0, 4.0);
    println!("== kurbo Rect area={} path_els={}", r.area(), r.to_path(1e-6).elements().len());
    let rr = kurbo::RoundedRect::new(0.0, 0.0, 10.0, 4.0, 1.5);
    println!(
        "== kurbo RoundedRect area={:.4} els={}",
        rr.area(),
        rr.to_path(1e-6).elements().len()
    );

    // simplify module
    let opts = kurbo::simplify::SimplifyOptions::default();
    let simplified = kurbo::simplify::simplify_bezpath(p.clone(), 0.5, &opts);
    println!(
        "== kurbo simplify_bezpath: {} els -> {} els",
        p.elements().len(),
        simplified.elements().len()
    );
}

// --- oxvg -------------------------------------------------------------------

fn oxvg_check() {
    use oxvg_ast::{parse::roxmltree::parse, serialize::Node as _, visitor::Info};
    use oxvg_optimiser::Jobs;

    let input = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50.000" cy="50.0" r="40" fill="#ff0000"/><path d="M 10 10 L 20 10 L 20 20 L 10 20 Z" fill="rgb(0,0,255)"/></svg>"##;

    let defaulted: String = parse(input, |dom, allocator| {
        let jobs = Jobs::default();
        jobs.run(dom, &Info::new(allocator)).expect("jobs");
        dom.serialize().expect("serialize")
    })
    .expect("parse");
    println!("== oxvg default jobs -> {defaulted}");

    let tuned: String = parse(input, |dom, allocator| {
        let mut jobs = Jobs::default();
        jobs.convert_shape_to_path = None;
        jobs.merge_paths = None;
        jobs.remove_view_box = None;
        jobs.sort_attrs = None;
        jobs.run(dom, &Info::new(allocator)).expect("jobs");
        dom.serialize().expect("serialize")
    })
    .expect("parse");
    println!("== oxvg tuned jobs -> {tuned}");
    println!(
        "   circle preserved: {} | viewBox preserved: {}",
        tuned.contains("<circle"),
        tuned.contains("viewBox")
    );

    // Does convert_shape_to_path touch <rect>? (SVGO converts rect/line/poly*,
    // and circle/ellipse only with convertArcs.)
    let rect_in = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="1" y="2" width="30" height="40" fill="#ff0000"/></svg>"##;
    let rect_default: String = parse(rect_in, |dom, allocator| {
        let jobs = Jobs::default();
        jobs.run(dom, &Info::new(allocator)).expect("jobs");
        dom.serialize().expect("serialize")
    })
    .expect("parse");
    println!("== oxvg default on <rect> -> {rect_default}");

    // Marginal byte win on a real vtracer output (already relative + shorthands).
    let img = decode(&fixture_png());
    let mut cfg = Config::default();
    cfg.optimize = 2;
    let raw = cfg.build().expect("build").to_svg(&img).expect("svg");
    // Approximate what our Phase 6 writer emits: no prolog, no comment, no
    // width/height, viewBox instead. This is the fair baseline for oxvg's win.
    let body = raw
        .lines()
        .filter(|l| !l.starts_with("<?xml") && !l.starts_with("<!--"))
        .collect::<Vec<_>>()
        .join("");
    let svg = body.replace(
        "<svg version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\" width=\"128\" height=\"128\">",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 128 128\">",
    );
    let optimized: String = parse(&svg, |dom, allocator| {
        let mut jobs = Jobs::default();
        jobs.convert_shape_to_path = None;
        jobs.merge_paths = None;
        jobs.remove_view_box = None;
        jobs.sort_attrs = None;
        jobs.run(dom, &Info::new(allocator)).expect("jobs");
        dom.serialize().expect("serialize")
    })
    .expect("parse");
    println!(
        "== oxvg on vtracer output: {} -> {} bytes ({:+.1}%)",
        svg.len(),
        optimized.len(),
        (optimized.len() as f64 / svg.len() as f64 - 1.0) * 100.0
    );

    // Idempotence.
    let twice: String = parse(&optimized, |dom, allocator| {
        let mut jobs = Jobs::default();
        jobs.convert_shape_to_path = None;
        jobs.merge_paths = None;
        jobs.remove_view_box = None;
        jobs.sort_attrs = None;
        jobs.run(dom, &Info::new(allocator)).expect("jobs");
        dom.serialize().expect("serialize")
    })
    .expect("parse");
    println!("   idempotent: {}", twice == optimized);
}
