//! Pixel analysis: what the page background is, which rows are empty, and
//! where the gutters are.
//!
//! Every function here is pure and takes an in-memory [`RgbaImage`], so the
//! tests build canvases programmatically instead of reading fixtures.

use std::collections::HashMap;

use image::{Rgba, RgbaImage};

/// Default channel tolerance for "this pixel is the background colour".
///
/// Small on purpose: JPEG ringing and a soft paper texture move a white
/// background by a couple of levels, while real artwork moves it by far more.
pub const DEFAULT_TOLERANCE: u8 = 6;

/// Default minimum height of a run of empty rows before it counts as a gutter.
///
/// No platform publishes a required panel spacing, so this is stripcut's own
/// documented default rather than a number taken from anyone's guidelines. 12
/// rows is short enough to catch tight layouts and tall enough that a one-pixel
/// seam between two touching panels is not mistaken for a gutter.
pub const DEFAULT_MIN_GUTTER: u32 = 12;

/// A maximal run of consecutive background-only rows: `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gutter {
    /// First background row, inclusive.
    pub start: u32,
    /// One past the last background row.
    pub end: u32,
}

impl Gutter {
    /// Height of the run in rows.
    pub fn len(self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    /// Whether the run covers no rows at all.
    pub fn is_empty(self) -> bool {
        self.end <= self.start
    }

    /// The row stripcut cuts at: the middle of the run, so both tiles keep an
    /// equal margin of background. Always inside `[start, end)`.
    pub fn centre(self) -> u32 {
        self.start + self.len() / 2
    }

    /// Whether row `y` belongs to this run.
    pub fn contains(self, y: u32) -> bool {
        y >= self.start && y < self.end
    }
}

/// Whether one pixel counts as page background.
///
/// A fully transparent pixel is always background: on an RGBA canvas the empty
/// space between panels is usually alpha 0 and its colour channels are
/// meaningless. When the page background itself is transparent, only
/// transparent pixels match -- an opaque white pixel on a transparent page is
/// artwork.
pub fn is_background(px: Rgba<u8>, bg: Rgba<u8>, tolerance: u8) -> bool {
    if px.0[3] == 0 {
        return true;
    }
    if bg.0[3] == 0 {
        return false;
    }
    px.0.iter()
        .zip(bg.0.iter())
        .all(|(p, b)| p.abs_diff(*b) <= tolerance)
}

/// Clamp `--edge-margin` so at least one column is always inspected.
fn usable_columns(width: u32, edge_margin: u32) -> (u32, u32) {
    let margin = edge_margin.min(width.saturating_sub(1) / 2);
    (margin, width - margin)
}

/// Read one pixel out of the raw RGBA buffer.
#[inline]
fn pixel_at(raw: &[u8], width: u32, x: u32, y: u32) -> Rgba<u8> {
    let i = (y as usize * width as usize + x as usize) * 4;
    Rgba([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]])
}

/// Guess the page background: the most common colour along the left and right
/// edge columns and the top and bottom rows.
///
/// Ties break toward the numerically smaller RGBA value so the result does not
/// depend on hash iteration order -- identical input always yields identical
/// output. `edge_margin` shifts the sampled columns inward, which is what you
/// want when the canvas has a thin printed border.
pub fn detect_background(img: &RgbaImage, edge_margin: u32) -> Rgba<u8> {
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Rgba([255, 255, 255, 255]);
    }
    let raw = img.as_raw();
    let (x0, x1) = usable_columns(w, edge_margin);
    let mut counts: HashMap<[u8; 4], u64> = HashMap::new();
    let mut note = |px: Rgba<u8>| {
        // Any fully transparent pixel is the same "transparent" background,
        // whatever its colour channels happen to hold.
        let key = if px.0[3] == 0 { [0, 0, 0, 0] } else { px.0 };
        *counts.entry(key).or_insert(0) += 1;
    };

    let right = x1.saturating_sub(1);
    for y in 0..h {
        note(pixel_at(raw, w, x0, y));
        if right != x0 {
            note(pixel_at(raw, w, right, y));
        }
    }
    let bottom = h - 1;
    for x in x0..x1 {
        note(pixel_at(raw, w, x, 0));
        if bottom != 0 {
            note(pixel_at(raw, w, x, bottom));
        }
    }

    let mut best = ([255u8, 255, 255, 255], 0u64);
    for (colour, count) in counts {
        if count > best.1 || (count == best.1 && colour < best.0) {
            best = (colour, count);
        }
    }
    Rgba(best.0)
}

/// Whether every inspected pixel in row `y` is background.
///
/// Returns on the first non-background pixel, which is what keeps a 40,000-row
/// canvas cheap: rows that hold artwork usually fail within a few pixels.
pub fn row_is_background(
    img: &RgbaImage,
    y: u32,
    bg: Rgba<u8>,
    tolerance: u8,
    edge_margin: u32,
) -> bool {
    let (w, h) = (img.width(), img.height());
    if y >= h || w == 0 {
        return false;
    }
    let (x0, x1) = usable_columns(w, edge_margin);
    let raw = img.as_raw();
    let row = y as usize * w as usize * 4;
    for x in x0..x1 {
        let i = row + x as usize * 4;
        let px = Rgba([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]);
        if !is_background(px, bg, tolerance) {
            return false;
        }
    }
    true
}

/// Every maximal run of background-only rows at least `min_gutter` rows tall,
/// in top-to-bottom order.
pub fn find_gutters(
    img: &RgbaImage,
    bg: Rgba<u8>,
    tolerance: u8,
    edge_margin: u32,
    min_gutter: u32,
) -> Vec<Gutter> {
    let min_gutter = min_gutter.max(1);
    let mut runs = Vec::new();
    let mut open: Option<u32> = None;
    for y in 0..img.height() {
        if row_is_background(img, y, bg, tolerance, edge_margin) {
            open.get_or_insert(y);
        } else if let Some(start) = open.take() {
            if y - start >= min_gutter {
                runs.push(Gutter { start, end: y });
            }
        }
    }
    if let Some(start) = open {
        let end = img.height();
        if end - start >= min_gutter {
            runs.push(Gutter { start, end });
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A canvas of `bg`, with each `(y0, y1)` band painted solid black.
    fn strip(width: u32, height: u32, bg: Rgba<u8>, panels: &[(u32, u32)]) -> RgbaImage {
        let mut img = RgbaImage::from_pixel(width, height, bg);
        for &(y0, y1) in panels {
            for y in y0..y1 {
                for x in 0..width {
                    img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
                }
            }
        }
        img
    }

    const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);

    #[test]
    fn gutter_geometry() {
        let g = Gutter {
            start: 100,
            end: 140,
        };
        assert_eq!(g.len(), 40);
        assert!(!g.is_empty());
        assert_eq!(g.centre(), 120);
        assert!(g.contains(100));
        assert!(g.contains(139));
        assert!(!g.contains(140));
        // A one-row run still has its centre inside itself.
        let one = Gutter { start: 7, end: 8 };
        assert_eq!(one.centre(), 7);
        assert!(one.contains(one.centre()));
    }

    #[test]
    fn detects_a_white_background() {
        let img = strip(40, 200, WHITE, &[(50, 150)]);
        assert_eq!(detect_background(&img, 0), WHITE);
    }

    #[test]
    fn detects_a_transparent_background() {
        let clear = Rgba([0, 0, 0, 0]);
        let img = strip(40, 200, clear, &[(50, 150)]);
        assert_eq!(detect_background(&img, 0), clear);
    }

    #[test]
    fn edge_margin_ignores_a_printed_border() {
        // Two columns of red down both sides make every row non-empty...
        let mut img = strip(40, 120, WHITE, &[(40, 80)]);
        for y in 0..120 {
            for x in [0, 1, 38, 39] {
                img.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        assert!(!row_is_background(&img, 0, WHITE, 0, 0));
        // ...until the border is excluded.
        assert!(row_is_background(&img, 0, WHITE, 0, 2));
        assert!(!row_is_background(&img, 50, WHITE, 0, 2));
    }

    #[test]
    fn tolerance_absorbs_near_white() {
        let near = Rgba([250, 250, 250, 255]);
        let img = RgbaImage::from_pixel(20, 4, near);
        assert!(!row_is_background(&img, 0, WHITE, 0, 0));
        assert!(!row_is_background(&img, 0, WHITE, 4, 0));
        assert!(row_is_background(&img, 0, WHITE, 5, 0));
        assert!(row_is_background(&img, 0, WHITE, DEFAULT_TOLERANCE, 0));
    }

    #[test]
    fn transparent_pixels_are_always_background() {
        assert!(is_background(Rgba([9, 9, 9, 0]), WHITE, 0));
        // But an opaque pixel on a transparent page is artwork.
        let clear = Rgba([0, 0, 0, 0]);
        assert!(!is_background(WHITE, clear, 255));
        assert!(is_background(clear, clear, 0));
    }

    #[test]
    fn finds_the_runs_between_panels() {
        // Panels at 0-100 and 140-300; the gutter is rows 100..140.
        let img = strip(40, 300, WHITE, &[(0, 100), (140, 300)]);
        let runs = find_gutters(&img, WHITE, 0, 0, 10);
        assert_eq!(
            runs,
            vec![Gutter {
                start: 100,
                end: 140
            }]
        );
    }

    #[test]
    fn min_gutter_filters_a_hairline_gap() {
        // A 4-row gap and a 40-row gap.
        let img = strip(40, 300, WHITE, &[(0, 50), (54, 150), (190, 300)]);
        let all = find_gutters(&img, WHITE, 0, 0, 1);
        assert_eq!(all.len(), 2);
        let filtered = find_gutters(&img, WHITE, 0, 0, 20);
        assert_eq!(
            filtered,
            vec![Gutter {
                start: 150,
                end: 190
            }]
        );
    }

    #[test]
    fn trailing_background_closes_the_last_run() {
        let img = strip(40, 200, WHITE, &[(0, 150)]);
        let runs = find_gutters(&img, WHITE, 0, 0, 10);
        assert_eq!(
            runs,
            vec![Gutter {
                start: 150,
                end: 200
            }]
        );
    }

    #[test]
    fn a_balloon_inside_a_panel_is_not_a_gutter() {
        // A black panel with a white balloon that does not span the full width.
        // The balloon's rows are background-coloured but not background-only,
        // so they must not become a cut point; only the margins above and below
        // the panel are gutters.
        let mut img = strip(40, 200, WHITE, &[(20, 180)]);
        for y in 60..100 {
            for x in 10..30 {
                img.put_pixel(x, y, WHITE);
            }
        }
        let runs = find_gutters(&img, WHITE, 0, 0, 10);
        assert_eq!(
            runs,
            vec![
                Gutter { start: 0, end: 20 },
                Gutter {
                    start: 180,
                    end: 200
                }
            ]
        );
        for y in 60..100 {
            assert!(
                !runs.iter().any(|g| g.contains(y)),
                "row {y} is inside the balloon and must not be a gutter row"
            );
        }
    }
}
