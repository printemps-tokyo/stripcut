//! Cut planning: turn a canvas height plus a list of gutters into an ordered
//! list of tiles.
//!
//! The rule, in one paragraph. Standing at cut position `y`, the next cut has
//! to land no less than `min_height` and no more than `max_height` below it.
//! Inside that window stripcut takes the gutter whose *centre* is closest to
//! the ideal tile height (`target_height`, or `max_height` when it is not
//! given) and cuts at that centre, so both neighbouring tiles keep an equal
//! margin of background. If no gutter centre falls inside the window, the cut
//! is forced to `max_height`; that forced cut is reported as a hard cut unless
//! it happens to land inside a gutter run anyway, in which case no artwork is
//! severed and it is reported as a gutter cut.
//!
//! Ties -- two gutters equally close to the ideal -- go to the higher one, so
//! the plan is a deterministic function of its inputs.

use crate::analyze::Gutter;
use crate::model::{Warning, WarningKind};

/// How the bottom edge of a tile was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutKind {
    /// The cut landed inside a run of background rows.
    Gutter,
    /// No gutter qualified; the cut was forced at `max_height`.
    Hard,
    /// The bottom of the canvas.
    End,
}

impl CutKind {
    /// Machine token used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            CutKind::Gutter => "gutter",
            CutKind::Hard => "hard-cut",
            CutKind::End => "end",
        }
    }
}

/// One planned tile: rows `[y_start, y_end)` of the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tile {
    /// 0-based position in the plan.
    pub index: usize,
    pub y_start: u32,
    /// Exclusive.
    pub y_end: u32,
    /// How this tile's bottom edge was chosen.
    pub cut: CutKind,
    /// The gutter run the bottom edge sits in, when it sits in one.
    pub gutter: Option<Gutter>,
}

impl Tile {
    pub fn height(&self) -> u32 {
        self.y_end.saturating_sub(self.y_start)
    }
}

/// Tunables for [`plan_cuts`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanOptions {
    /// Hard ceiling on a tile's height. Never exceeded.
    pub max_height: u32,
    /// Floor on the distance between two cuts.
    pub min_height: u32,
    /// The height stripcut aims for. Defaults to `max_height`.
    pub target_height: Option<u32>,
}

impl PlanOptions {
    /// The ideal tile height: `target_height` when set, else `max_height`.
    pub fn target(&self) -> u32 {
        self.target_height.unwrap_or(self.max_height)
    }
}

/// A complete plan plus whatever stripcut wants the artist to look at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub canvas_height: u32,
    pub tiles: Vec<Tile>,
    pub warnings: Vec<Warning>,
}

/// Plan the cuts for a canvas of `canvas_height` rows given its `gutters`.
///
/// `gutters` must be sorted top to bottom and non-overlapping, which is what
/// [`crate::analyze::find_gutters`] produces.
pub fn plan_cuts(
    canvas_height: u32,
    gutters: &[Gutter],
    opts: &PlanOptions,
) -> Result<Plan, String> {
    if canvas_height == 0 {
        return Err("canvas has no rows".to_string());
    }
    if opts.max_height == 0 {
        return Err("--max-height must be at least 1".to_string());
    }
    if opts.min_height > opts.max_height {
        return Err(format!(
            "--min-height ({}) must not exceed --max-height ({})",
            opts.min_height, opts.max_height
        ));
    }
    if let Some(target) = opts.target_height {
        if target == 0 {
            return Err("--target-height must be at least 1".to_string());
        }
        if target > opts.max_height {
            return Err(format!(
                "--target-height ({target}) must not exceed --max-height ({})",
                opts.max_height
            ));
        }
    }

    let mut tiles: Vec<Tile> = Vec::new();
    let mut warnings: Vec<Warning> = Vec::new();
    let mut y = 0u32;

    while canvas_height - y > opts.max_height {
        // The window of allowed cut positions. `lo` is forced past `y` so the
        // plan always makes progress even with --min-height 0.
        let lo = (y + opts.min_height).max(y + 1);
        let hi = y + opts.max_height;
        let ideal = (y + opts.target()).clamp(lo, hi);

        let chosen = best_gutter(gutters, lo, hi, ideal);
        let (cut_y, kind, run) = match chosen {
            Some(g) => (g.centre(), CutKind::Gutter, Some(g)),
            None => {
                // Forced cut. If it happens to land inside a gutter anyway, no
                // artwork is severed, so do not cry wolf.
                let landed = gutters.iter().copied().find(|g| g.contains(hi));
                match landed {
                    Some(g) => (hi, CutKind::Gutter, Some(g)),
                    None => {
                        warnings.push(Warning {
                            kind: WarningKind::HardCut,
                            tile: Some(tiles.len()),
                            message: format!(
                                "no gutter between y={lo} and y={hi}; cut forced at y={hi} (inspect this seam)"
                            ),
                        });
                        (hi, CutKind::Hard, None)
                    }
                }
            }
        };

        tiles.push(Tile {
            index: tiles.len(),
            y_start: y,
            y_end: cut_y,
            cut: kind,
            gutter: run,
        });
        y = cut_y;
    }

    tiles.push(Tile {
        index: tiles.len(),
        y_start: y,
        y_end: canvas_height,
        cut: CutKind::End,
        gutter: None,
    });

    // A leftover strip shorter than --min-height gets one of three fates, in
    // order of preference:
    //
    // 1. merged back into the tile above it, when the merged tile still fits
    //    under --max-height;
    // 2. otherwise, if the seam above it was a hard cut -- a cut that severed
    //    artwork anyway -- that seam is pulled up so the tail reaches
    //    --min-height. Nothing is lost: one arbitrary seam is moved to a
    //    slightly less arbitrary place, and both tiles end up legal;
    // 3. otherwise the seam above it sits in a gutter, and moving it would
    //    start severing artwork to fix a cosmetic problem. The tail stays and
    //    is flagged.
    if tiles.len() >= 2 {
        let last = tiles[tiles.len() - 1];
        let prev = tiles[tiles.len() - 2];
        if last.height() < opts.min_height {
            let merged = prev.height() + last.height();
            let rebalanced = last.y_end.saturating_sub(opts.min_height);
            if merged <= opts.max_height {
                tiles.pop();
                let n = tiles.len() - 1;
                tiles[n].y_end = last.y_end;
                tiles[n].cut = CutKind::End;
                tiles[n].gutter = None;
                // Any hard-cut warning for the seam that just disappeared is
                // no longer true.
                warnings.retain(|w| w.tile != Some(n));
            } else if prev.cut == CutKind::Hard
                && rebalanced > prev.y_start
                && rebalanced - prev.y_start >= opts.min_height
                && rebalanced - prev.y_start <= opts.max_height
            {
                let n = tiles.len() - 1;
                tiles[n - 1].y_end = rebalanced;
                tiles[n].y_start = rebalanced;
                // The hard cut moved, so its warning has to name where it is
                // now rather than where the planner first put it.
                let index = n - 1;
                for warning in warnings.iter_mut() {
                    if warning.tile == Some(index) && warning.kind == WarningKind::HardCut {
                        warning.message = format!(
                            "no gutter available; cut forced at y={rebalanced} so the last tile \
                             reaches --min-height {} (inspect this seam)",
                            opts.min_height
                        );
                    }
                }
            } else {
                warnings.push(Warning {
                    kind: WarningKind::ShortTile,
                    tile: Some(tiles.len() - 1),
                    message: format!(
                        "last tile is {} px, shorter than --min-height {}. The seam above it sits \
                         in a gutter, so stripcut left it alone rather than cutting through \
                         artwork to even the tiles up",
                        last.height(),
                        opts.min_height
                    ),
                });
            }
        }
    }

    if gutters.is_empty() && tiles.len() > 1 {
        warnings.insert(
            0,
            Warning {
                kind: WarningKind::NoGutters,
                tile: None,
                message:
                    "no gutters were detected on this canvas; every cut is a hard cut. Check --bg, \
                     --tolerance, --edge-margin and --min-gutter, or accept that this page is \
                     full-bleed"
                        .to_string(),
            },
        );
    }

    Ok(Plan {
        canvas_height,
        tiles,
        warnings,
    })
}

/// The gutter whose centre lies in `[lo, hi]` and is closest to `ideal`.
/// Ties go to the higher gutter, which keeps the plan deterministic.
fn best_gutter(gutters: &[Gutter], lo: u32, hi: u32, ideal: u32) -> Option<Gutter> {
    let mut best: Option<(Gutter, u32, u32)> = None;
    for &g in gutters {
        let centre = g.centre();
        if centre < lo || centre > hi {
            continue;
        }
        let distance = centre.abs_diff(ideal);
        let better = match best {
            None => true,
            Some((_, best_distance, best_centre)) => {
                distance < best_distance || (distance == best_distance && centre < best_centre)
            }
        };
        if better {
            best = Some((g, distance, centre));
        }
    }
    best.map(|(g, _, _)| g)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(max: u32, min: u32) -> PlanOptions {
        PlanOptions {
            max_height: max,
            min_height: min,
            target_height: None,
        }
    }

    #[test]
    fn short_canvas_is_a_single_tile() {
        let plan = plan_cuts(500, &[], &opts(1280, 320)).unwrap();
        assert_eq!(plan.tiles.len(), 1);
        assert_eq!(plan.tiles[0].cut, CutKind::End);
        assert!(plan.warnings.is_empty());
    }

    #[test]
    fn cuts_at_the_gutter_closest_to_the_target() {
        // Gutters centred at 500 and 900; the ideal is 1000, so 900 wins.
        let gutters = vec![
            Gutter {
                start: 480,
                end: 520,
            },
            Gutter {
                start: 880,
                end: 920,
            },
        ];
        let plan = plan_cuts(2000, &gutters, &opts(1000, 100)).unwrap();
        assert_eq!(plan.tiles[0].y_end, 900);
        assert_eq!(plan.tiles[0].cut, CutKind::Gutter);
        // The first cut found its gutter, so nothing is flagged against it.
        // (There are no gutters left further down, so the tile after it is a
        // hard cut -- that is a separate seam and a separate warning.)
        assert!(!plan
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::HardCut && w.tile == Some(0)));
    }

    #[test]
    fn gutter_exactly_at_the_window_edge_is_chosen() {
        // Centre lands exactly on max_height.
        let gutters = vec![Gutter {
            start: 980,
            end: 1021,
        }];
        assert_eq!(gutters[0].centre(), 1000);
        let plan = plan_cuts(3000, &gutters, &opts(1000, 100)).unwrap();
        assert_eq!(plan.tiles[0].y_end, 1000);
        assert_eq!(plan.tiles[0].cut, CutKind::Gutter);

        // And exactly on min_height.
        let low = vec![Gutter {
            start: 90,
            end: 111,
        }];
        assert_eq!(low[0].centre(), 100);
        let plan = plan_cuts(3000, &low, &opts(1000, 100)).unwrap();
        assert_eq!(plan.tiles[0].y_end, 100);
        assert_eq!(plan.tiles[0].cut, CutKind::Gutter);
    }

    #[test]
    fn gutter_just_outside_the_window_falls_back_to_a_hard_cut() {
        // Centre at 1001 with max_height 1000: one pixel too far, and the run
        // does not reach back to y=1000 either.
        let gutters = vec![Gutter {
            start: 1001,
            end: 1002,
        }];
        let plan = plan_cuts(3000, &gutters, &opts(1000, 100)).unwrap();
        assert_eq!(plan.tiles[0].y_end, 1000);
        assert_eq!(plan.tiles[0].cut, CutKind::Hard);
        assert!(plan
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::HardCut && w.tile == Some(0)));
    }

    #[test]
    fn a_forced_cut_landing_inside_a_run_is_not_called_hard() {
        // The run's centre (1200) is out of the window, but the run still
        // covers y=1000, so the forced cut severs nothing.
        let gutters = vec![Gutter {
            start: 900,
            end: 1500,
        }];
        let plan = plan_cuts(3000, &gutters, &opts(1000, 990)).unwrap();
        assert_eq!(plan.tiles[0].y_end, 1000);
        assert_eq!(plan.tiles[0].cut, CutKind::Gutter);
        assert!(!plan
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::HardCut && w.tile == Some(0)));
    }

    #[test]
    fn no_gutters_means_hard_cuts_within_the_limits() {
        let plan = plan_cuts(2500, &[], &opts(1000, 100)).unwrap();
        assert_eq!(plan.tiles.len(), 3);
        assert_eq!(plan.tiles[0].y_end, 1000);
        assert_eq!(plan.tiles[1].y_end, 2000);
        assert_eq!(plan.tiles[2].y_end, 2500);
        assert!(plan.tiles.iter().all(|t| t.height() <= 1000));
        assert!(plan
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::NoGutters));
        assert_eq!(
            plan.warnings
                .iter()
                .filter(|w| w.kind == WarningKind::HardCut)
                .count(),
            2
        );
    }

    #[test]
    fn a_short_tail_is_merged_back_when_it_fits() {
        // 2400 rows, max 2500: one tile, nothing to merge.
        let plan = plan_cuts(2400, &[], &opts(2500, 300)).unwrap();
        assert_eq!(plan.tiles.len(), 1);

        // 2050 rows, max 2000: the tail is 50 rows and 2000 + 50 is over the
        // ceiling, so it cannot merge -- but the seam above it was a hard cut,
        // so the planner slides that seam up and both tiles come out legal.
        let plan = plan_cuts(2050, &[], &opts(2000, 300)).unwrap();
        assert_eq!(plan.tiles.len(), 2);
        assert_eq!(plan.tiles[0].y_end, 1750);
        assert_eq!(plan.tiles[1].y_start, 1750);
        assert_eq!(plan.tiles[1].height(), 300);
        assert!(plan
            .warnings
            .iter()
            .all(|w| w.kind != WarningKind::ShortTile));
        assert!(plan
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::HardCut && w.message.contains("y=1750")));

        // Same shape, but the seam above the tail sits in a gutter. Moving it
        // would sever artwork, so the short tail stays and is flagged instead.
        let gutters = vec![Gutter {
            start: 1990,
            end: 2011,
        }];
        let plan = plan_cuts(2050, &gutters, &opts(2000, 300)).unwrap();
        assert_eq!(plan.tiles.len(), 2);
        assert_eq!(plan.tiles[0].y_end, 2000);
        assert_eq!(plan.tiles[0].cut, CutKind::Gutter);
        assert_eq!(plan.tiles[1].height(), 50);
        assert!(plan
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::ShortTile));
    }

    #[test]
    fn a_short_tail_that_fits_is_absorbed_by_the_tile_above_it() {
        // 1050 rows, max 2000: the loop never fires, so there is a single
        // tile and nothing to absorb.
        assert_eq!(
            plan_cuts(1050, &[], &opts(2000, 300)).unwrap().tiles.len(),
            1
        );

        // 1100 rows, max 1000, min 300: the first cut is forced at 1000 and
        // the 100-row tail merges straight back in, because 1100 <= max is
        // false... it is not, so instead the seam slides up to 800.
        let plan = plan_cuts(1100, &[], &opts(1000, 300)).unwrap();
        assert_eq!(plan.tiles.len(), 2);
        assert_eq!(plan.tiles[0].height(), 800);
        assert_eq!(plan.tiles[1].height(), 300);

        // 1100 rows, max 1200: the tail merges into a single legal tile.
        let plan = plan_cuts(1100, &[], &opts(1200, 300)).unwrap();
        assert_eq!(plan.tiles.len(), 1);
        assert_eq!(plan.tiles[0].height(), 1100);
    }

    #[test]
    fn every_plan_keeps_its_tiles_inside_the_limits() {
        // A sweep over canvas heights: whatever the leftover, no tile may end
        // up over --max-height, and only the documented gutter-seam case may
        // leave one under --min-height.
        for height in [301, 999, 1000, 1001, 1300, 2050, 2051, 5000, 5001] {
            let plan = plan_cuts(height, &[], &opts(1000, 300)).unwrap();
            let mut y = 0;
            for tile in &plan.tiles {
                assert_eq!(tile.y_start, y, "gap at {height}");
                assert!(tile.height() <= 1000, "{height}: {tile:?} is too tall");
                if plan.tiles.len() > 1 {
                    assert!(tile.height() >= 300, "{height}: {tile:?} is too short");
                }
                y = tile.y_end;
            }
            assert_eq!(y, height);
        }
    }

    #[test]
    fn tiles_are_contiguous_and_inside_the_limits() {
        let gutters: Vec<Gutter> = (1..20)
            .map(|i| Gutter {
                start: i * 700,
                end: i * 700 + 30,
            })
            .collect();
        let plan = plan_cuts(14_000, &gutters, &opts(1500, 400)).unwrap();
        let mut y = 0;
        for tile in &plan.tiles {
            assert_eq!(tile.y_start, y);
            assert!(tile.height() <= 1500, "{tile:?}");
            y = tile.y_end;
        }
        assert_eq!(y, 14_000);
    }

    #[test]
    fn target_height_moves_the_preferred_cut() {
        let gutters = vec![
            Gutter {
                start: 490,
                end: 510,
            },
            Gutter {
                start: 1490,
                end: 1510,
            },
        ];
        let near = plan_cuts(
            4000,
            &gutters,
            &PlanOptions {
                max_height: 1600,
                min_height: 100,
                target_height: Some(500),
            },
        )
        .unwrap();
        assert_eq!(near.tiles[0].y_end, 500);
        let far = plan_cuts(4000, &gutters, &opts(1600, 100)).unwrap();
        assert_eq!(far.tiles[0].y_end, 1500);
    }

    #[test]
    fn rejects_contradictory_options() {
        assert!(plan_cuts(0, &[], &opts(100, 0)).is_err());
        assert!(plan_cuts(100, &[], &opts(0, 0)).is_err());
        assert!(plan_cuts(100, &[], &opts(100, 200)).is_err());
        assert!(plan_cuts(
            100,
            &[],
            &PlanOptions {
                max_height: 100,
                min_height: 0,
                target_height: Some(200),
            }
        )
        .is_err());
    }

    #[test]
    fn ties_go_to_the_higher_gutter() {
        // Centres 900 and 1100 are both 100 from the ideal 1000.
        let gutters = vec![
            Gutter {
                start: 895,
                end: 906,
            },
            Gutter {
                start: 1095,
                end: 1106,
            },
        ];
        let plan = plan_cuts(4000, &gutters, &opts(1200, 100)).unwrap();
        let ideal = 1200_u32;
        assert_eq!(gutters[0].centre().abs_diff(ideal), 300);
        // With the ideal at max_height the lower gutter is not a tie; force one
        // by setting the target between the two.
        let plan_tie = plan_cuts(
            4000,
            &gutters,
            &PlanOptions {
                max_height: 1200,
                min_height: 100,
                target_height: Some(1000),
            },
        )
        .unwrap();
        assert_eq!(plan_tie.tiles[0].y_end, 900);
        assert_eq!(plan.tiles[0].y_end, 1100);
    }
}
