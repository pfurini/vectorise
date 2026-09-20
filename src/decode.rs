//! Read a raster file into exactly what the tracer expects.
//!
//! The tracer takes a [`ColorImage`]: four straight (non-premultiplied) RGBA
//! bytes per pixel, row-major. Everything this module does is in service of
//! producing one faithfully.
//!
//! # Formats
//!
//! PNG, JPEG, WebP, GIF, BMP, and TIFF. The format is decided by content, not
//! by extension: `plan` already rejects unknown extensions, and this second
//! check catches a `.png` that is really something else.
//!
//! # Orientation
//!
//! `image` 0.25 does not apply EXIF orientation on its own, so this module
//! reads it from the decoder and applies it. A photograph tagged "rotate 90"
//! is traced the way a viewer shows it, not the way the sensor stored it.
//!
//! # Alpha
//!
//! Verified against the tracer in `docs/api-notes.md` §6:
//!
//! | Input pixel | What we emit |
//! |---|---|
//! | `a == 255` | unchanged |
//! | `0 < a < 255` | composited over the background, `a = 255` |
//! | `a == 0` | RGB replaced by the background, `a = 0` kept |
//!
//! Keeping `a == 0` matters: the tracer keys fully transparent pixels out by
//! itself, so compositing them away would paint a background rectangle behind
//! every logo. But it only does so when about a fifth of the image is
//! transparent, and it ignores partial alpha entirely (it clusters on RGB).
//! Normalizing the RGB of transparent pixels makes the sub-threshold case fade
//! into the background instead of into the black that PNG encoders usually
//! leave there.

use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Seek};
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage};
use vtracer::ColorImage;

use crate::color::Rgb;

/// Formats we read. Kept in step with the `image` features in `Cargo.toml` and
/// with the extension list in [`crate::plan`].
const SUPPORTED_FORMATS: [ImageFormat; 6] = [
    ImageFormat::Png,
    ImageFormat::Jpeg,
    ImageFormat::WebP,
    ImageFormat::Gif,
    ImageFormat::Bmp,
    ImageFormat::Tiff,
];

/// How to read a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DecodeOptions {
    /// The color semi-transparent pixels are composited onto, and the color
    /// fully transparent pixels are normalized to. Default white.
    pub background: Rgb,
}

/// Why a file could not be read.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The file could not be opened or read.
    #[error("{path}: {source}")]
    Io {
        /// The file we tried to read.
        path: PathBuf,
        /// The underlying I/O failure.
        source: std::io::Error,
    },
    /// The content is not one of the six raster formats we read.
    #[error("{path}: not a supported raster image (expected PNG, JPEG, WebP, GIF, BMP, or TIFF)")]
    UnsupportedFormat {
        /// The file we tried to read.
        path: PathBuf,
    },
    /// The content claims a supported format but does not decode.
    #[error("{path}: {source}")]
    Malformed {
        /// The file we tried to read.
        path: PathBuf,
        /// What the decoder objected to.
        source: image::ImageError,
    },
    /// The image decoded, but has no pixels.
    #[error("{path}: image is {width}x{height}; there is nothing to trace")]
    Empty {
        /// The file we tried to read.
        path: PathBuf,
        /// Decoded width.
        width: u32,
        /// Decoded height.
        height: u32,
    },
}

impl DecodeError {
    /// The input this error is about.
    pub fn path(&self) -> &Path {
        match self {
            Self::Io { path, .. }
            | Self::UnsupportedFormat { path }
            | Self::Malformed { path, .. }
            | Self::Empty { path, .. } => path,
        }
    }
}

/// Read `path` into a traceable image.
///
/// # Errors
///
/// [`DecodeError::Io`] if the file cannot be read,
/// [`DecodeError::UnsupportedFormat`] if its content is not one of the six
/// formats, [`DecodeError::Malformed`] if the decoder rejects it, and
/// [`DecodeError::Empty`] if it decodes to zero pixels.
pub fn decode(path: &Path, options: DecodeOptions) -> Result<ColorImage, DecodeError> {
    let file = File::open(path).map_err(|source| DecodeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    decode_reader(BufReader::new(file), path, options)
}

/// Read an in-memory image. `path` is used only for error messages.
///
/// # Errors
///
/// Same as [`decode`], minus [`DecodeError::Io`] for opening the file.
pub fn decode_bytes(
    bytes: &[u8],
    path: &Path,
    options: DecodeOptions,
) -> Result<ColorImage, DecodeError> {
    decode_reader(Cursor::new(bytes), path, options)
}

/// The shared body of [`decode`] and [`decode_bytes`].
fn decode_reader<R: BufRead + Seek>(
    reader: R,
    path: &Path,
    options: DecodeOptions,
) -> Result<ColorImage, DecodeError> {
    let reader = ImageReader::new(reader)
        .with_guessed_format()
        .map_err(|source| DecodeError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    match reader.format() {
        Some(format) if SUPPORTED_FORMATS.contains(&format) => {}
        _ => {
            return Err(DecodeError::UnsupportedFormat {
                path: path.to_path_buf(),
            });
        }
    }

    let mut decoder = reader
        .into_decoder()
        .map_err(|source| DecodeError::Malformed {
            path: path.to_path_buf(),
            source,
        })?;
    // Read the orientation before the decoder is consumed. A decoder that has
    // no opinion (PNG, BMP, GIF) reports `NoTransforms`.
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);

    let mut image =
        DynamicImage::from_decoder(decoder).map_err(|source| DecodeError::Malformed {
            path: path.to_path_buf(),
            source,
        })?;
    image.apply_orientation(orientation);

    let rgba = image.into_rgba8();
    if rgba.width() == 0 || rgba.height() == 0 {
        return Err(DecodeError::Empty {
            path: path.to_path_buf(),
            width: rgba.width(),
            height: rgba.height(),
        });
    }

    Ok(to_color_image(rgba, options.background))
}

/// Apply the alpha policy and hand the buffer to the tracer's type.
fn to_color_image(mut rgba: RgbaImage, background: Rgb) -> ColorImage {
    let width = rgba.width() as usize;
    let height = rgba.height() as usize;

    for pixel in rgba.pixels_mut() {
        let [r, g, b, a] = pixel.0;
        pixel.0 = match a {
            255 => [r, g, b, 255],
            0 => [background.r, background.g, background.b, 0],
            a => [
                over(r, background.r, a),
                over(g, background.g, a),
                over(b, background.b, a),
                255,
            ],
        };
    }

    ColorImage {
        pixels: rgba.into_raw(),
        width,
        height,
    }
}

/// One channel of `source` composited over `background` at opacity `alpha`.
///
/// Adding 127 before dividing rounds to nearest instead of truncating. The
/// result never exceeds 255, so the fallback is unreachable.
fn over(source: u8, background: u8, alpha: u8) -> u8 {
    let alpha = u32::from(alpha);
    let blended = (u32::from(source) * alpha + u32::from(background) * (255 - alpha) + 127) / 255;
    u8::try_from(blended).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use std::io::Cursor;
    use std::path::Path;

    use image::{
        DynamicImage, GrayImage, ImageFormat, Luma, Rgb as ImageRgb, RgbImage, Rgba, RgbaImage,
    };

    use super::{DecodeError, DecodeOptions, decode, decode_bytes};
    use crate::color::Rgb;

    /// A 3x2 test image with four distinguishable opaque colors, one fully
    /// transparent pixel, and one half-transparent pixel.
    fn rgba_fixture() -> RgbaImage {
        RgbaImage::from_fn(3, 2, |x, y| match (x, y) {
            (0, 0) => Rgba([255, 0, 0, 255]),
            (1, 0) => Rgba([0, 255, 0, 255]),
            (2, 0) => Rgba([0, 0, 255, 255]),
            (0, 1) => Rgba([10, 20, 30, 255]),
            (1, 1) => Rgba([0, 0, 0, 0]),
            _ => Rgba([200, 100, 50, 128]),
        })
    }

    fn encode(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, format).expect("encode");
        out.into_inner()
    }

    fn path() -> &'static Path {
        Path::new("fixture.png")
    }

    fn opts() -> DecodeOptions {
        DecodeOptions::default()
    }

    /// `Result::expect_err` needs `Debug` on the Ok side, which `ColorImage`
    /// does not implement.
    fn error_of(result: Result<vtracer::ColorImage, DecodeError>) -> DecodeError {
        result.err().expect("decoding should have failed")
    }

    /// The four bytes of one pixel of a decoded `ColorImage`.
    fn pixel(image: &vtracer::ColorImage, x: usize, y: usize) -> [u8; 4] {
        let base = (y * image.width + x) * 4;
        [
            image.pixels[base],
            image.pixels[base + 1],
            image.pixels[base + 2],
            image.pixels[base + 3],
        ]
    }

    #[test]
    fn decode_png_rgba_dimensions_and_pixels_roundtrip() {
        let bytes = encode(&DynamicImage::ImageRgba8(rgba_fixture()), ImageFormat::Png);
        let decoded = decode_bytes(&bytes, path(), opts()).expect("decodes");

        assert_eq!((decoded.width, decoded.height), (3, 2));
        assert_eq!(decoded.pixels.len(), 3 * 2 * 4);
        // Opaque pixels survive byte for byte.
        assert_eq!(pixel(&decoded, 0, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(&decoded, 1, 0), [0, 255, 0, 255]);
        assert_eq!(pixel(&decoded, 2, 0), [0, 0, 255, 255]);
        assert_eq!(pixel(&decoded, 0, 1), [10, 20, 30, 255]);
    }

    #[test]
    fn decode_jpeg_yields_opaque_alpha_255() {
        let source = RgbImage::from_pixel(4, 4, ImageRgb([200, 40, 60]));
        let bytes = encode(&DynamicImage::ImageRgb8(source), ImageFormat::Jpeg);
        let decoded = decode_bytes(&bytes, Path::new("f.jpg"), opts()).expect("decodes");

        assert_eq!((decoded.width, decoded.height), (4, 4));
        for index in 0..decoded.width * decoded.height {
            assert_eq!(
                decoded.pixels[index * 4 + 3],
                255,
                "pixel {index} is opaque"
            );
        }
    }

    #[test]
    fn decode_grayscale_png_expands_to_rgba() {
        let source = GrayImage::from_fn(2, 1, |x, _| Luma([if x == 0 { 0 } else { 200 }]));
        let bytes = encode(&DynamicImage::ImageLuma8(source), ImageFormat::Png);
        let decoded = decode_bytes(&bytes, path(), opts()).expect("decodes");

        assert_eq!(decoded.pixels.len(), 2 * 4);
        assert_eq!(pixel(&decoded, 0, 0), [0, 0, 0, 255]);
        assert_eq!(pixel(&decoded, 1, 0), [200, 200, 200, 255]);
    }

    #[test]
    fn decode_palette_png_expands_to_rgba() {
        // `image` has no palette encoder, so write the PNG by hand: a 2x1
        // indexed image with a two-entry palette.
        let bytes = palette_png();
        let decoded = decode_bytes(&bytes, path(), opts()).expect("decodes");

        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(pixel(&decoded, 0, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(&decoded, 1, 0), [0, 0, 255, 255]);
    }

    #[test]
    fn decode_16bit_png_downconverts_to_8bit() {
        let source = image::ImageBuffer::<Rgba<u16>, Vec<u16>>::from_pixel(
            1,
            1,
            Rgba([65535, 32768, 0, 65535]),
        );
        let bytes = encode(&DynamicImage::ImageRgba16(source), ImageFormat::Png);
        let decoded = decode_bytes(&bytes, path(), opts()).expect("decodes");

        assert_eq!(decoded.pixels.len(), 4);
        let [r, g, b, a] = pixel(&decoded, 0, 0);
        assert_eq!((r, b, a), (255, 0, 255));
        assert!(
            (127..=128).contains(&g),
            "0x8000 of 0xffff is about half: {g}"
        );
    }

    #[test]
    fn decode_webp_lossless_roundtrip() {
        let source = RgbaImage::from_fn(2, 2, |x, y| {
            Rgba([
                if x == 0 { 255 } else { 0 },
                if y == 0 { 255 } else { 0 },
                128,
                255,
            ])
        });
        let bytes = encode(&DynamicImage::ImageRgba8(source.clone()), ImageFormat::WebP);
        let decoded = decode_bytes(&bytes, Path::new("f.webp"), opts()).expect("decodes");

        assert_eq!((decoded.width, decoded.height), (2, 2));
        for y in 0..2u32 {
            for x in 0..2u32 {
                assert_eq!(
                    pixel(&decoded, x as usize, y as usize),
                    source.get_pixel(x, y).0,
                    "lossless WebP is exact at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn decode_gif_first_frame() {
        // A two-frame GIF: the first frame is red, the second green. We trace
        // stills, so only the first frame may come back.
        let bytes = two_frame_gif();
        let decoded = decode_bytes(&bytes, Path::new("f.gif"), opts()).expect("decodes");

        assert_eq!((decoded.width, decoded.height), (2, 2));
        assert_eq!(pixel(&decoded, 0, 0), [255, 0, 0, 255]);
    }

    #[test]
    fn decode_bmp() {
        let source = RgbImage::from_pixel(2, 3, ImageRgb([7, 8, 9]));
        let bytes = encode(&DynamicImage::ImageRgb8(source), ImageFormat::Bmp);
        let decoded = decode_bytes(&bytes, Path::new("f.bmp"), opts()).expect("decodes");

        assert_eq!((decoded.width, decoded.height), (2, 3));
        assert_eq!(pixel(&decoded, 1, 2), [7, 8, 9, 255]);
    }

    #[test]
    fn decode_tiff() {
        let source = RgbaImage::from_pixel(2, 2, Rgba([1, 2, 3, 255]));
        let bytes = encode(&DynamicImage::ImageRgba8(source), ImageFormat::Tiff);
        let decoded = decode_bytes(&bytes, Path::new("f.tiff"), opts()).expect("decodes");

        assert_eq!((decoded.width, decoded.height), (2, 2));
        assert_eq!(pixel(&decoded, 0, 0), [1, 2, 3, 255]);
    }

    #[test]
    fn decode_corrupt_file_is_typed_error_not_panic() {
        // A valid PNG signature and header, then garbage.
        let good = encode(&DynamicImage::ImageRgba8(rgba_fixture()), ImageFormat::Png);
        let mut corrupt = good[..30].to_vec();
        corrupt.extend_from_slice(&[0xde_u8, 0xad, 0xbe, 0xef].repeat(40));

        let error = error_of(decode_bytes(&corrupt, path(), opts()));
        assert!(
            matches!(error, DecodeError::Malformed { .. }),
            "expected Malformed, got {error:?}"
        );
        assert_eq!(error.path(), path());
    }

    #[test]
    fn decode_unsupported_content_is_typed_error() {
        let error = error_of(decode_bytes(b"this is not an image", path(), opts()));
        assert!(
            matches!(error, DecodeError::UnsupportedFormat { .. }),
            "expected UnsupportedFormat, got {error:?}"
        );
    }

    #[test]
    fn decode_missing_file_is_io_error() {
        let error = error_of(decode(Path::new("no/such/file.png"), opts()));
        assert!(matches!(error, DecodeError::Io { .. }), "got {error:?}");
    }

    #[test]
    fn decode_zero_size_image_is_error() {
        // A 0x0 PNG: `image` will not encode one, so write the chunks by hand.
        let bytes = zero_size_png();
        let error = error_of(decode_bytes(&bytes, path(), opts()));
        assert!(
            matches!(
                error,
                DecodeError::Empty { .. } | DecodeError::Malformed { .. }
            ),
            "a zero-size image must be rejected, not traced: {error:?}"
        );
    }

    #[test]
    fn decode_alpha_policy_matches_api_notes() {
        let bytes = encode(&DynamicImage::ImageRgba8(rgba_fixture()), ImageFormat::Png);
        let decoded = decode_bytes(&bytes, path(), opts()).expect("decodes");

        // Fully transparent: alpha kept at 0 so the tracer can key it out, RGB
        // normalized to the background instead of the encoder's black.
        assert_eq!(pixel(&decoded, 1, 1), [255, 255, 255, 0]);

        // Half transparent: composited onto white and made opaque.
        let [r, g, b, a] = pixel(&decoded, 2, 1);
        assert_eq!(a, 255);
        assert_eq!([r, g, b], [227, 177, 152]);
    }

    #[test]
    fn decode_alpha_policy_honours_a_custom_background() {
        let bytes = encode(&DynamicImage::ImageRgba8(rgba_fixture()), ImageFormat::Png);
        let options = DecodeOptions {
            background: Rgb::BLACK,
        };
        let decoded = decode_bytes(&bytes, path(), options).expect("decodes");

        assert_eq!(pixel(&decoded, 1, 1), [0, 0, 0, 0]);
        // 200 * 128/255 rounds to 100; a black background adds nothing.
        let [r, g, b, a] = pixel(&decoded, 2, 1);
        assert_eq!(a, 255);
        assert_eq!([r, g, b], [100, 50, 25]);
    }

    #[test]
    fn decode_opaque_pixels_are_untouched_by_the_alpha_policy() {
        let source = RgbaImage::from_pixel(1, 1, Rgba([12, 34, 56, 255]));
        let bytes = encode(&DynamicImage::ImageRgba8(source), ImageFormat::Png);
        let options = DecodeOptions {
            background: Rgb::BLACK,
        };
        assert_eq!(
            pixel(
                &decode_bytes(&bytes, path(), options).expect("decodes"),
                0,
                0
            ),
            [12, 34, 56, 255]
        );
    }

    #[test]
    fn decode_exif_orientation_is_applied() {
        // An 8x4 landscape image, left half red, tagged "rotate 90 clockwise".
        // It must come back 4x8 with the red half on top. The block is wide
        // enough to survive JPEG chroma subsampling.
        let source = RgbImage::from_fn(8, 4, |x, _| {
            if x < 4 {
                ImageRgb([255, 0, 0])
            } else {
                ImageRgb([255, 255, 255])
            }
        });
        let plain = encode(&DynamicImage::ImageRgb8(source), ImageFormat::Jpeg);

        let unrotated = decode_bytes(&plain, Path::new("f.jpg"), opts()).expect("decodes");
        assert_eq!(
            (unrotated.width, unrotated.height),
            (8, 4),
            "without the tag the image is untouched"
        );

        let rotated = decode_bytes(
            &with_exif_orientation(&plain, 6),
            Path::new("f.jpg"),
            opts(),
        )
        .expect("decodes");
        assert_eq!(
            (rotated.width, rotated.height),
            (4, 8),
            "orientation 6 turns 8x4 into 4x8"
        );

        // Rotating 90 degrees clockwise sends the left half to the top half.
        let [top_r, top_g, _, _] = pixel(&rotated, 2, 1);
        let [bottom_r, bottom_g, _, _] = pixel(&rotated, 2, 6);
        assert!(top_r > 200 && top_g < 80, "top is red: {top_r},{top_g}");
        assert!(
            bottom_r > 200 && bottom_g > 200,
            "bottom is white: {bottom_r},{bottom_g}"
        );
    }

    #[test]
    fn decode_reads_from_the_filesystem() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("one.png");
        let bytes = encode(&DynamicImage::ImageRgba8(rgba_fixture()), ImageFormat::Png);
        std::fs::write(&file, &bytes).expect("write");

        let decoded = decode(&file, opts()).expect("decodes");
        assert_eq!((decoded.width, decoded.height), (3, 2));
    }

    // --- hand-built fixtures -------------------------------------------------

    /// A 2x1 indexed-color PNG with a red and a blue palette entry.
    ///
    /// `image` cannot write a palette, and the point of the test is that
    /// *reading* an indexed PNG yields RGBA, so the file is built by hand.
    fn palette_png() -> Vec<u8> {
        let mut png = Vec::new();
        png.extend_from_slice(&PNG_SIGNATURE);

        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&2u32.to_be_bytes()); // width
        ihdr.extend_from_slice(&1u32.to_be_bytes()); // height
        ihdr.extend_from_slice(&[8, 3, 0, 0, 0]); // 8 bits, color type 3 (indexed)
        push_chunk(&mut png, *b"IHDR", &ihdr);

        push_chunk(&mut png, *b"PLTE", &[255, 0, 0, 0, 0, 255]);
        // One scanline: filter byte 0, then the indices 0 and 1.
        push_chunk(&mut png, *b"IDAT", &zlib_stored(&[0, 0, 1]));
        push_chunk(&mut png, *b"IEND", &[]);
        png
    }

    /// A 1x0 PNG: valid structure, no pixels.
    fn zero_size_png() -> Vec<u8> {
        let mut png = Vec::new();
        png.extend_from_slice(&PNG_SIGNATURE);
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.extend_from_slice(&0u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8 bits, truecolor
        push_chunk(&mut png, *b"IHDR", &ihdr);
        push_chunk(&mut png, *b"IDAT", &zlib_stored(&[]));
        push_chunk(&mut png, *b"IEND", &[]);
        png
    }

    /// A 2x2 GIF with a red first frame and a green second frame.
    fn two_frame_gif() -> Vec<u8> {
        use image::codecs::gif::GifEncoder;
        use image::{Delay, Frame};

        let mut out = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut out);
            let red = RgbaImage::from_pixel(2, 2, Rgba([255, 0, 0, 255]));
            let green = RgbaImage::from_pixel(2, 2, Rgba([0, 255, 0, 255]));
            let delay = Delay::from_numer_denom_ms(100, 1);
            encoder
                .encode_frame(Frame::from_parts(red, 0, 0, delay))
                .expect("frame 1");
            encoder
                .encode_frame(Frame::from_parts(green, 0, 0, delay))
                .expect("frame 2");
        }
        out
    }

    /// Insert an APP1 EXIF segment carrying `orientation` into a JPEG.
    fn with_exif_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
        // Minimal TIFF header: little-endian, one IFD, one entry (0x0112).
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II\x2a\x00");
        tiff.extend_from_slice(&8u32.to_le_bytes()); // offset of IFD0
        tiff.extend_from_slice(&1u16.to_le_bytes()); // entry count
        tiff.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation
        tiff.extend_from_slice(&3u16.to_le_bytes()); // type SHORT
        tiff.extend_from_slice(&1u32.to_le_bytes()); // count
        tiff.extend_from_slice(&orientation.to_le_bytes());
        tiff.extend_from_slice(&[0, 0]); // pad the 4-byte value field
        tiff.extend_from_slice(&0u32.to_le_bytes()); // no next IFD

        let mut payload = Vec::new();
        payload.extend_from_slice(b"Exif\0\0");
        payload.extend_from_slice(&tiff);

        let mut out = Vec::new();
        out.extend_from_slice(&jpeg[..2]); // SOI
        out.extend_from_slice(&[0xff, 0xe1]); // APP1
        let length = u16::try_from(payload.len() + 2).expect("segment fits");
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(&payload);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    /// The eight bytes every PNG starts with.
    const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

    /// Append a PNG chunk with its length and CRC.
    fn push_chunk(png: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
        let length = u32::try_from(data.len()).expect("chunk fits");
        png.extend_from_slice(&length.to_be_bytes());
        png.extend_from_slice(&kind);
        png.extend_from_slice(data);

        let mut crc_input = Vec::with_capacity(kind.len() + data.len());
        crc_input.extend_from_slice(&kind);
        crc_input.extend_from_slice(data);
        png.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    }

    /// A zlib stream with a single stored (uncompressed) deflate block.
    fn zlib_stored(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01]; // zlib header, no compression
        let length = u16::try_from(data.len()).expect("block fits");
        out.push(0x01); // final stored block
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&(!length).to_le_bytes());
        out.extend_from_slice(data);
        out.extend_from_slice(&adler32(data).to_be_bytes());
        out
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn adler32(data: &[u8]) -> u32 {
        let mut a = 1u32;
        let mut b = 0u32;
        for &byte in data {
            a = (a + u32::from(byte)) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16) | a
    }
}
