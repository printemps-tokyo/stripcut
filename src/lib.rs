//! stripcut -- slice a vertical-scroll comic canvas in the gutters, not
//! through the artwork.
//!
//! # The problem
//!
//! A webtoon episode is drawn as one continuous canvas -- 800 pixels wide and
//! tens of thousands of pixels tall -- but it has to be uploaded as a sequence
//! of images, each within the platform's height and file-size limits. Cutting
//! at fixed intervals is trivial and wrong: a cut at "every 1280 pixels" lands
//! wherever it lands, which sooner or later is through a face, a speech balloon
//! or the middle of a panel.
//!
//! stripcut looks for the *gutters* -- the runs of rows that contain nothing
//! but page background -- and cuts there, as close to the requested tile height
//! as the artwork allows.
//!
//! # How it works
//!
//! 1. [`analyze::detect_background`] picks the page background colour by taking
//!    the most common colour along the canvas edges (or you pass `--bg`).
//! 2. [`analyze::row_is_background`] decides, row by row, whether a row is
//!    nothing but background within a tolerance. It bails out on the first
//!    non-background pixel, so a busy canvas costs far less than a full scan.
//! 3. [`analyze::find_gutters`] collects maximal runs of such rows that are at
//!    least `min_gutter` tall.
//! 4. [`plan::plan_cuts`] walks down the canvas and, for each tile, picks the
//!    gutter whose centre sits closest to the ideal tile height while staying
//!    inside `[min_height, max_height]`. When no gutter qualifies it cuts at
//!    `max_height` and records a warning naming the tile and the y position.
//!
//! Everything above is a pure function of an in-memory RGBA buffer plus a few
//! numbers, so it is unit-testable without touching the filesystem. The binary
//! layer is the only part that decodes, encodes and writes.
//!
//! # Honest limitation
//!
//! Detection is pixel-based. It assumes the page has a mostly uniform
//! background, which is how vertical-scroll comics are normally laid out. A
//! full-bleed illustration with no empty rows has no gutters to find, so
//! stripcut hard-cuts it at `max_height` -- and says so, per tile, in the
//! report and in the JSON. It never silently severs artwork without telling
//! you.
//!
//! # Shape
//!
//! - [`presets`] is data: one entry per platform, each carrying the official
//!   page its numbers came from and the date that page was read.
//! - [`analyze`] and [`plan`] are pure and take buffers, not paths.
//! - [`encode`] turns a tile into bytes in memory, including the quality ladder.
//! - [`check`] validates already-sliced files against a preset.
//! - [`report`] renders the human report and the JSON document.
//!
//! No network access happens at any point.

pub mod analyze;
pub mod check;
pub mod color;
pub mod encode;
pub mod json;
pub mod model;
pub mod plan;
pub mod presets;
pub mod report;

pub use analyze::{
    detect_background, find_gutters, is_background, row_is_background, Gutter, DEFAULT_MIN_GUTTER,
    DEFAULT_TOLERANCE,
};
pub use check::{check_episode, check_one, CheckItem, CheckLimits};
pub use color::{format_color, parse_color};
pub use encode::{encode_tile, encode_within, Encoded, Format, DEFAULT_QUALITY, QUALITY_LADDER};
pub use model::{
    CanvasInfo, RunReport, Settings, SourceInfo, TileReport, Warning, WarningKind, CAVEAT,
    SCHEMA_VERSION,
};
pub use plan::{plan_cuts, CutKind, Plan, PlanOptions, Tile};
pub use presets::{find_preset, preset_names, Preset, PRESETS};
pub use report::{
    render_json, render_presets_json, render_presets_text, render_text, summary_line, RenderOptions,
};

use image::RgbaImage;

/// Widest canvas stripcut will decode. A vertical-scroll page is normally
/// 690-1200 px wide; anything past this is a mistake, not a webtoon.
pub const MAX_WIDTH: u32 = 20_000;

/// Tallest canvas stripcut will decode. 200k rows is roughly five very long
/// episodes stacked, which is already past what any platform accepts.
pub const MAX_HEIGHT: u32 = 200_000;

/// Upper bound on total pixels in one canvas (including a `--join`ed one). An
/// RGBA buffer is 4 bytes per pixel, so 120 MP is ~480 MB resident.
pub const MAX_PIXELS: u64 = 120_000_000;

/// Reject a canvas that is too large to work on before allocating for it.
pub fn check_canvas_limits(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err(format!("empty canvas: {width}x{height}"));
    }
    if width > MAX_WIDTH {
        return Err(format!("canvas too wide: {width} px (max {MAX_WIDTH})"));
    }
    if height > MAX_HEIGHT {
        return Err(format!("canvas too tall: {height} px (max {MAX_HEIGHT})"));
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!(
            "canvas too large: {width}x{height} exceeds the {MAX_PIXELS} pixel cap"
        ));
    }
    Ok(())
}

/// Copy rows `[y_start, y_end)` out of a canvas into a new image.
///
/// This is the only pixel copying stripcut does. The rows come out untouched:
/// no resampling, no colour conversion, no recompression at this stage, so a
/// PNG in and a PNG out is pixel-identical inside the tile.
pub fn crop_rows(img: &RgbaImage, y_start: u32, y_end: u32) -> Result<RgbaImage, String> {
    if y_end <= y_start || y_end > img.height() {
        return Err(format!(
            "invalid tile rows {y_start}..{y_end} on a {}-row canvas",
            img.height()
        ));
    }
    let width = img.width();
    let height = y_end - y_start;
    let row_bytes = width as usize * 4;
    let from = y_start as usize * row_bytes;
    let to = y_end as usize * row_bytes;
    let raw = img.as_raw()[from..to].to_vec();
    RgbaImage::from_raw(width, height, raw)
        .ok_or_else(|| "internal error: cropped buffer has the wrong length".to_string())
}

/// Stack canvases top to bottom into one continuous canvas, for `--join`.
///
/// Every input must be the same width -- a webtoon episode split across files
/// is one canvas cut into pieces, so a width mismatch means the files do not
/// belong to the same episode and stripcut says so rather than padding.
pub fn join_canvases(parts: &[RgbaImage]) -> Result<RgbaImage, String> {
    let Some(first) = parts.first() else {
        return Err("nothing to join".to_string());
    };
    let width = first.width();
    let mut total: u64 = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.width() != width {
            return Err(format!(
                "--join needs one width for every input: input 1 is {width} px wide, \
                 input {} is {} px wide",
                i + 1,
                part.width()
            ));
        }
        total += u64::from(part.height());
    }
    let height = u32::try_from(total).map_err(|_| format!("joined canvas is {total} rows tall"))?;
    check_canvas_limits(width, height)?;

    let mut raw: Vec<u8> = Vec::with_capacity(width as usize * height as usize * 4);
    for part in parts {
        raw.extend_from_slice(part.as_raw());
    }
    RgbaImage::from_raw(width, height, raw)
        .ok_or_else(|| "internal error: joined buffer has the wrong length".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn crop_copies_exactly_the_asked_rows() {
        let mut img = RgbaImage::from_pixel(4, 10, Rgba([1, 2, 3, 255]));
        for x in 0..4 {
            img.put_pixel(x, 5, Rgba([9, 9, 9, 255]));
        }
        let tile = crop_rows(&img, 4, 7).unwrap();
        assert_eq!((tile.width(), tile.height()), (4, 3));
        assert_eq!(*tile.get_pixel(0, 0), Rgba([1, 2, 3, 255]));
        assert_eq!(*tile.get_pixel(0, 1), Rgba([9, 9, 9, 255]));
        assert_eq!(*tile.get_pixel(3, 2), Rgba([1, 2, 3, 255]));
    }

    #[test]
    fn crop_rejects_rows_outside_the_canvas() {
        let img = RgbaImage::new(4, 10);
        assert!(crop_rows(&img, 5, 5).is_err());
        assert!(crop_rows(&img, 7, 4).is_err());
        assert!(crop_rows(&img, 0, 11).is_err());
        assert!(crop_rows(&img, 0, 10).is_ok());
    }

    #[test]
    fn join_stacks_equal_width_canvases() {
        let a = RgbaImage::from_pixel(4, 3, Rgba([1, 0, 0, 255]));
        let b = RgbaImage::from_pixel(4, 5, Rgba([0, 1, 0, 255]));
        let joined = join_canvases(&[a, b]).unwrap();
        assert_eq!((joined.width(), joined.height()), (4, 8));
        assert_eq!(*joined.get_pixel(0, 2), Rgba([1, 0, 0, 255]));
        assert_eq!(*joined.get_pixel(0, 3), Rgba([0, 1, 0, 255]));
    }

    #[test]
    fn join_refuses_a_width_mismatch_and_an_empty_list() {
        let a = RgbaImage::new(4, 3);
        let b = RgbaImage::new(5, 3);
        let err = join_canvases(&[a, b]).unwrap_err();
        assert!(err.contains("one width for every input"), "{err}");
        assert!(join_canvases(&[]).is_err());
    }

    #[test]
    fn canvas_limits_reject_absurd_inputs() {
        assert!(check_canvas_limits(800, 40_000).is_ok());
        assert!(check_canvas_limits(0, 10).is_err());
        assert!(check_canvas_limits(10, 0).is_err());
        assert!(check_canvas_limits(MAX_WIDTH + 1, 10).is_err());
        assert!(check_canvas_limits(10, MAX_HEIGHT + 1).is_err());
        // Within both per-side caps but past the total-pixel cap.
        assert!(check_canvas_limits(20_000, 100_000).is_err());
    }
}
