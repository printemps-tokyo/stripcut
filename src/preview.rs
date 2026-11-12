//! The cut preview: a scaled-down picture of the whole canvas with the plan
//! drawn on it, for checking every seam at a glance before uploading.
//!
//! - gutter runs are tinted blue across the art,
//! - each cut is a line across the full width: green when it sits in a
//!   gutter, red when it was forced (a hard cut, the seams worth inspecting),
//! - a ribbon down the left alternates shades tile by tile, so tile lengths
//!   read at a glance.
//!
//! Like everything in the library this is a pure function of its inputs.

use image::imageops::{resize, FilterType};
use image::{Rgba, RgbaImage};

use crate::analyze::Gutter;
use crate::plan::{CutKind, Tile};

/// Default width of the preview's canvas area, in pixels.
pub const DEFAULT_PREVIEW_WIDTH: u32 = 240;
/// The preview never gets taller than this; very tall canvases scale down
/// further to fit.
pub const MAX_PREVIEW_HEIGHT: u32 = 30_000;
/// Width of the tile ribbon on the left.
pub const RIBBON: u32 = 14;
/// The widest preview `--preview-width` may ask for.
pub const MAX_PREVIEW_WIDTH: u32 = 4096;

const GUTTER_TINT: Rgba<u8> = Rgba([70, 140, 230, 255]);
const GUTTER_ALPHA: f32 = 0.35;
const CUT_GUTTER: Rgba<u8> = Rgba([26, 158, 60, 255]);
const CUT_HARD: Rgba<u8> = Rgba([214, 40, 40, 255]);
const RIBBON_A: Rgba<u8> = Rgba([208, 208, 208, 255]);
const RIBBON_B: Rgba<u8> = Rgba([150, 150, 150, 255]);

/// The scale factor that fits a `width` x `height` canvas into a preview
/// `target_width` wide and at most [`MAX_PREVIEW_HEIGHT`] tall. A preview
/// only ever shrinks: the factor never exceeds 1, so it is never larger than
/// the canvas it shows.
pub fn preview_scale(width: u32, height: u32, target_width: u32) -> f64 {
    let by_width = f64::from(target_width.max(1)) / f64::from(width.max(1));
    let by_height = f64::from(MAX_PREVIEW_HEIGHT) / f64::from(height.max(1));
    by_width.min(by_height).min(1.0)
}

/// Render the preview. `tiles` may be empty (the `--gutters` view).
pub fn render_preview(
    canvas: &RgbaImage,
    gutters: &[Gutter],
    tiles: &[Tile],
    target_width: u32,
) -> RgbaImage {
    let scale = preview_scale(canvas.width(), canvas.height(), target_width);
    let sw = ((f64::from(canvas.width()) * scale).round() as u32).max(1);
    let sh = ((f64::from(canvas.height()) * scale).round() as u32).max(1);
    let scaled = resize(canvas, sw, sh, FilterType::Triangle);

    // Composite over white so transparent canvases preview as the page would.
    let mut out = RgbaImage::from_pixel(RIBBON + sw, sh, Rgba([255, 255, 255, 255]));
    for (x, y, px) in scaled.enumerate_pixels() {
        let a = f32::from(px[3]) / 255.0;
        let blend = |c: u8| (f32::from(c) * a + 255.0 * (1.0 - a)).round() as u8;
        out.put_pixel(
            RIBBON + x,
            y,
            Rgba([blend(px[0]), blend(px[1]), blend(px[2]), 255]),
        );
    }

    let row = |y: u32| -> u32 { ((f64::from(y) * scale).floor() as u32).min(sh.saturating_sub(1)) };
    // Preview rows covering canvas rows [start, end): at least one row.
    let span = |start: u32, end: u32| -> std::ops::Range<u32> {
        let top = row(start);
        let bottom = ((f64::from(end) * scale).ceil() as u32).clamp(top + 1, sh);
        top..bottom
    };

    for g in gutters {
        for y in span(g.start, g.end) {
            for x in RIBBON..RIBBON + sw {
                let p = out.get_pixel_mut(x, y);
                for c in 0..3 {
                    p[c] = (f32::from(p[c]) * (1.0 - GUTTER_ALPHA)
                        + f32::from(GUTTER_TINT[c]) * GUTTER_ALPHA)
                        .round() as u8;
                }
            }
        }
    }

    for (i, tile) in tiles.iter().enumerate() {
        let shade = if i % 2 == 0 { RIBBON_A } else { RIBBON_B };
        for y in span(tile.y_start, tile.y_end) {
            for x in 0..RIBBON {
                out.put_pixel(x, y, shade);
            }
        }
    }

    // Cuts last, on top of everything, two pixels thick where there is room.
    for tile in tiles {
        let colour = match tile.cut {
            CutKind::Gutter => CUT_GUTTER,
            CutKind::Hard => CUT_HARD,
            CutKind::End => continue,
        };
        let y = row(tile.y_end);
        for dy in 0..2 {
            let yy = (y + dy).min(sh - 1);
            for x in 0..RIBBON + sw {
                out.put_pixel(x, yy, colour);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(index: usize, y_start: u32, y_end: u32, cut: CutKind) -> Tile {
        Tile {
            index,
            y_start,
            y_end,
            cut,
            gutter: None,
        }
    }

    #[test]
    fn scale_fits_width_then_height() {
        assert!((preview_scale(800, 4000, 200) - 0.25).abs() < 1e-12);
        // 800 x 1,000,000 would be 250,000 tall at 0.25; height wins.
        let s = preview_scale(800, 1_000_000, 200);
        assert!((s - 0.03).abs() < 1e-12);
        // Never larger than the canvas.
        assert_eq!(preview_scale(100, 100, 1_000_000), 1.0);
    }

    #[test]
    fn a_narrow_canvas_is_not_blown_up() {
        let canvas = RgbaImage::from_pixel(10, 1, Rgba([0, 0, 0, 255]));
        let img = render_preview(
            &canvas,
            &[Gutter { start: 0, end: 1 }],
            &[tile(0, 0, 1, CutKind::End)],
            240,
        );
        assert_eq!((img.width(), img.height()), (RIBBON + 10, 1));
        assert_eq!(*img.get_pixel(1, 0), RIBBON_A);
    }

    #[test]
    fn spans_cover_every_preview_row_of_a_run() {
        // 0.5 scale: canvas rows [3, 6) cover preview rows 1 and 2.
        let canvas = RgbaImage::from_pixel(20, 10, Rgba([0, 0, 0, 255]));
        let img = render_preview(&canvas, &[Gutter { start: 3, end: 6 }], &[], 10);
        let tinted = |y: u32| img.get_pixel(RIBBON + 2, y)[2] > 0;
        assert!(!tinted(0) && tinted(1) && tinted(2) && !tinted(3));
    }

    #[test]
    fn draws_cuts_gutters_and_ribbon() {
        let canvas = RgbaImage::from_pixel(400, 2000, Rgba([0, 0, 0, 255]));
        let gutters = [Gutter {
            start: 980,
            end: 1020,
        }];
        let tiles = [
            tile(0, 0, 1000, CutKind::Gutter),
            tile(1, 1000, 1600, CutKind::Hard),
            tile(2, 1600, 2000, CutKind::End),
        ];
        let img = render_preview(&canvas, &gutters, &tiles, 100);
        assert_eq!((img.width(), img.height()), (RIBBON + 100, 500));
        // The gutter cut at y=1000 -> row 250 is green across the width.
        assert_eq!(*img.get_pixel(RIBBON + 50, 250), CUT_GUTTER);
        assert_eq!(*img.get_pixel(0, 250), CUT_GUTTER);
        // The hard cut at y=1600 -> row 400 is red.
        assert_eq!(*img.get_pixel(RIBBON + 50, 400), CUT_HARD);
        // Gutter rows away from the cut are tinted blue over black.
        let tinted = *img.get_pixel(RIBBON + 50, 246);
        assert!(tinted[2] > tinted[0] && tinted[2] > 50);
        // Ribbon alternates per tile; the end has no line.
        assert_eq!(*img.get_pixel(1, 100), RIBBON_A);
        assert_eq!(*img.get_pixel(1, 300), RIBBON_B);
        assert_eq!(*img.get_pixel(1, 480), RIBBON_A);
        assert_eq!(*img.get_pixel(RIBBON + 50, 100), Rgba([0, 0, 0, 255]));
    }

    #[test]
    fn transparent_canvas_previews_white() {
        let canvas = RgbaImage::from_pixel(80, 80, Rgba([0, 0, 0, 0]));
        let img = render_preview(&canvas, &[], &[], 40);
        assert_eq!(*img.get_pixel(RIBBON + 10, 10), Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn tiny_canvas_does_not_panic() {
        let canvas = RgbaImage::from_pixel(1, 1, Rgba([10, 10, 10, 255]));
        let tiles = [tile(0, 0, 1, CutKind::End)];
        let img = render_preview(&canvas, &[Gutter { start: 0, end: 1 }], &tiles, 1);
        assert_eq!(img.height(), 1);
    }
}
