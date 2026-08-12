//! Turning a tile into bytes, in memory, including the byte-budget ladder.
//!
//! Encoding happens into a `Vec<u8>` rather than straight to a file for two
//! reasons: `--dry-run` needs the size of a tile it is never going to write,
//! and `--max-bytes` needs to try an encode, look at the result, and try again
//! at a lower quality. Only the binary layer touches the filesystem.
//!
//! # What "lossy" means here
//!
//! JPEG is the only lossy encoder stripcut has. The `image` crate's WebP
//! encoder is lossless-only (VP8L), so `--quality` does nothing for WebP and
//! PNG, and the byte ladder cannot help a WebP or PNG tile that is over budget.
//! stripcut says so rather than pretending otherwise.

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::codecs::webp::WebPEncoder;
use image::{ExtendedColorType, ImageEncoder, Rgb, RgbImage, Rgba, RgbaImage};

/// Quality steps tried, in order, when a lossy tile is over `--max-bytes`.
///
/// It stops at 30: below that a screentone-heavy comic page turns to mush, and
/// a tile that still does not fit at quality 30 needs a shorter tile, not a
/// worse one.
pub const QUALITY_LADDER: &[u8] = &[85, 75, 65, 55, 45, 38, 30];

/// Default JPEG quality when `--quality` is not given.
pub const DEFAULT_QUALITY: u8 = 85;

/// An output encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Webp,
}

impl Format {
    /// Parse a `--format` value or a file extension.
    pub fn parse(s: &str) -> Result<Format, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "png" => Ok(Format::Png),
            "jpg" | "jpeg" => Ok(Format::Jpeg),
            "webp" => Ok(Format::Webp),
            other => Err(format!(
                "unsupported format {other:?} (supported: png, jpg, webp)"
            )),
        }
    }

    /// The token used in `--format`, in filenames and in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpg",
            Format::Webp => "webp",
        }
    }

    /// Whether `--quality` and the byte ladder mean anything for this format.
    pub fn is_lossy(self) -> bool {
        matches!(self, Format::Jpeg)
    }

    /// Whether the encoder keeps an alpha channel.
    pub fn keeps_alpha(self) -> bool {
        !matches!(self, Format::Jpeg)
    }
}

/// One encoded tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    pub bytes: Vec<u8>,
    pub format: Format,
    /// The quality the encoder settled on. `None` for the lossless formats.
    pub quality: Option<u8>,
    /// False when a byte budget was given and this is still over it.
    pub within_limit: bool,
}

impl Encoded {
    pub fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Composite an RGBA tile onto an opaque background, for encoders without
/// alpha. A fully transparent page area becomes the page colour instead of
/// black, which is what a JPEG upload of a transparent canvas should look like.
fn flatten(img: &RgbaImage, over: Rgba<u8>) -> RgbImage {
    // A transparent "background colour" cannot be shown on an opaque canvas;
    // white is the conventional stand-in and is what a comic page expects.
    let base = if over.0[3] == 0 {
        [255u8, 255, 255]
    } else {
        [over.0[0], over.0[1], over.0[2]]
    };
    let mut out = RgbImage::new(img.width(), img.height());
    for (x, y, px) in img.enumerate_pixels() {
        let a = u32::from(px.0[3]);
        let mix = |fg: u8, bg: u8| -> u8 {
            // Rounded source-over compositing in integer arithmetic.
            let v = u32::from(fg) * a + u32::from(bg) * (255 - a);
            ((v + 127) / 255) as u8
        };
        out.put_pixel(
            x,
            y,
            Rgb([
                mix(px.0[0], base[0]),
                mix(px.0[1], base[1]),
                mix(px.0[2], base[2]),
            ]),
        );
    }
    out
}

/// Encode one tile at a fixed quality.
///
/// `background` is only consulted for JPEG, which has no alpha channel. The
/// encoder settings are fixed rather than derived from the environment, so the
/// same tile always produces the same bytes.
pub fn encode_tile(
    img: &RgbaImage,
    format: Format,
    quality: u8,
    background: Rgba<u8>,
) -> Result<Vec<u8>, String> {
    if img.width() == 0 || img.height() == 0 {
        return Err("cannot encode a tile with no pixels".to_string());
    }
    let mut buf: Vec<u8> = Vec::new();
    let (w, h) = (img.width(), img.height());
    match format {
        Format::Png => {
            // Best/Adaptive rather than the defaults: a comic page is flat
            // colour, it compresses well, and the choice is pinned here so the
            // output does not move when the crate's defaults do.
            PngEncoder::new_with_quality(&mut buf, CompressionType::Best, FilterType::Adaptive)
                .write_image(img.as_raw(), w, h, ExtendedColorType::Rgba8)
                .map_err(|e| format!("png encode failed: {e}"))?;
        }
        Format::Jpeg => {
            let rgb = flatten(img, background);
            JpegEncoder::new_with_quality(&mut buf, quality.clamp(1, 100))
                .write_image(rgb.as_raw(), w, h, ExtendedColorType::Rgb8)
                .map_err(|e| format!("jpeg encode failed: {e}"))?;
        }
        Format::Webp => {
            WebPEncoder::new_lossless(&mut buf)
                .write_image(img.as_raw(), w, h, ExtendedColorType::Rgba8)
                .map_err(|e| format!("webp encode failed: {e}"))?;
        }
    }
    Ok(buf)
}

/// Encode a tile, walking the quality ladder down if a byte budget demands it.
///
/// With no budget, or with a lossless format, this is one encode at the
/// requested quality. With a budget and a lossy format it tries the requested
/// quality first, then each ladder step strictly below it, and stops at the
/// first result that fits. If nothing fits, the smallest attempt is returned
/// with `within_limit` false -- the caller decides whether that is a warning or
/// a failure, and the tile is never silently passed off as compliant.
pub fn encode_within(
    img: &RgbaImage,
    format: Format,
    quality: u8,
    background: Rgba<u8>,
    max_bytes: Option<u64>,
) -> Result<Encoded, String> {
    let record = |bytes: Vec<u8>, q: u8, within: bool| Encoded {
        bytes,
        format,
        quality: if format.is_lossy() { Some(q) } else { None },
        within_limit: within,
    };

    let first = encode_tile(img, format, quality, background)?;
    let Some(limit) = max_bytes else {
        return Ok(record(first, quality, true));
    };
    if first.len() as u64 <= limit {
        return Ok(record(first, quality, true));
    }
    if !format.is_lossy() {
        // Nothing to trade away: PNG and this crate's WebP are lossless.
        return Ok(record(first, quality, false));
    }

    let mut best = record(first, quality, false);
    for &step in QUALITY_LADDER {
        if step >= quality {
            continue;
        }
        let bytes = encode_tile(img, format, step, background)?;
        let len = bytes.len() as u64;
        let fits = len <= limit;
        if fits {
            return Ok(record(bytes, step, true));
        }
        if len < best.len() {
            best = record(bytes, step, false);
        }
    }
    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);

    /// A noisy tile, so the encoders have something incompressible to chew on.
    fn noisy(w: u32, h: u32) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        let mut state: u32 = 0x1234_5678;
        for px in img.pixels_mut() {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let b = state.to_le_bytes();
            *px = Rgba([b[0], b[1], b[2], 255]);
        }
        img
    }

    #[test]
    fn format_parsing_and_traits() {
        assert_eq!(Format::parse("PNG").unwrap(), Format::Png);
        assert_eq!(Format::parse("jpeg").unwrap(), Format::Jpeg);
        assert_eq!(Format::parse("jpg").unwrap(), Format::Jpeg);
        assert_eq!(Format::parse(" webp ").unwrap(), Format::Webp);
        assert!(Format::parse("gif").is_err());
        assert!(Format::Jpeg.is_lossy());
        assert!(!Format::Png.is_lossy());
        assert!(!Format::Webp.is_lossy());
        assert!(Format::Png.keeps_alpha());
        assert!(!Format::Jpeg.keeps_alpha());
    }

    #[test]
    fn encodes_each_format_to_its_magic_bytes() {
        let img = RgbaImage::from_pixel(16, 16, Rgba([10, 20, 30, 255]));
        let png = encode_tile(&img, Format::Png, 85, WHITE).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let jpg = encode_tile(&img, Format::Jpeg, 85, WHITE).unwrap();
        assert_eq!(&jpg[..2], b"\xff\xd8");
        let webp = encode_tile(&img, Format::Webp, 85, WHITE).unwrap();
        assert_eq!(&webp[..4], b"RIFF");
        assert_eq!(&webp[8..12], b"WEBP");
    }

    #[test]
    fn encoding_is_deterministic() {
        let img = noisy(64, 64);
        for format in [Format::Png, Format::Jpeg, Format::Webp] {
            let a = encode_tile(&img, format, 80, WHITE).unwrap();
            let b = encode_tile(&img, format, 80, WHITE).unwrap();
            assert_eq!(a, b, "{format:?} is not deterministic");
        }
    }

    #[test]
    fn empty_tiles_are_refused() {
        assert!(encode_tile(&RgbaImage::new(0, 4), Format::Png, 85, WHITE).is_err());
        assert!(encode_tile(&RgbaImage::new(4, 0), Format::Png, 85, WHITE).is_err());
    }

    #[test]
    fn transparency_is_flattened_onto_the_page_colour_for_jpeg() {
        let clear = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0]));
        let flat = flatten(&clear, Rgba([200, 100, 50, 255]));
        assert_eq!(*flat.get_pixel(0, 0), Rgb([200, 100, 50]));
        // A transparent page colour cannot be shown; white stands in.
        let flat = flatten(&clear, Rgba([0, 0, 0, 0]));
        assert_eq!(*flat.get_pixel(0, 0), Rgb([255, 255, 255]));
        // Half-transparent black over white is mid grey.
        let half = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 0, 128]));
        assert_eq!(*flatten(&half, WHITE).get_pixel(0, 0), Rgb([127, 127, 127]));
    }

    #[test]
    fn no_budget_means_one_encode_at_the_asked_quality() {
        let img = noisy(64, 64);
        let out = encode_within(&img, Format::Jpeg, 90, WHITE, None).unwrap();
        assert!(out.within_limit);
        assert_eq!(out.quality, Some(90));
        assert_eq!(
            out.bytes,
            encode_tile(&img, Format::Jpeg, 90, WHITE).unwrap()
        );
    }

    #[test]
    fn a_generous_budget_keeps_the_asked_quality() {
        let img = RgbaImage::from_pixel(64, 64, Rgba([120, 130, 140, 255]));
        let out = encode_within(&img, Format::Jpeg, 90, WHITE, Some(10_000_000)).unwrap();
        assert!(out.within_limit);
        assert_eq!(out.quality, Some(90));
    }

    #[test]
    fn the_ladder_drops_quality_until_a_tile_fits() {
        let img = noisy(200, 200);
        let full = encode_tile(&img, Format::Jpeg, 95, WHITE).unwrap().len() as u64;
        // A budget the top quality cannot meet but a lower step can.
        let out = encode_within(&img, Format::Jpeg, 95, WHITE, Some(full * 2 / 3)).unwrap();
        assert!(out.within_limit, "expected the ladder to find a fit");
        assert!(out.quality.unwrap() < 95);
        assert!(out.len() <= full * 2 / 3);
    }

    #[test]
    fn an_impossible_budget_reports_the_smallest_attempt_as_over() {
        let img = noisy(200, 200);
        let out = encode_within(&img, Format::Jpeg, 95, WHITE, Some(64)).unwrap();
        assert!(!out.within_limit);
        assert_eq!(out.quality, Some(*QUALITY_LADDER.last().unwrap()));
    }

    #[test]
    fn a_lossless_format_over_budget_does_not_pretend_to_shrink() {
        let img = noisy(200, 200);
        for format in [Format::Png, Format::Webp] {
            let out = encode_within(&img, format, 85, WHITE, Some(64)).unwrap();
            assert!(!out.within_limit);
            assert_eq!(out.quality, None, "{format:?} should report no quality");
        }
    }

    #[test]
    fn the_ladder_never_climbs_above_the_asked_quality() {
        let img = noisy(200, 200);
        let out = encode_within(&img, Format::Jpeg, 40, WHITE, Some(64)).unwrap();
        assert!(out.quality.unwrap() <= 40);
    }
}
