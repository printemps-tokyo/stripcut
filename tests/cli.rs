//! Black-box tests of the binary: modes, flags, exit codes, JSON shape,
//! determinism and the overwrite rules.
//!
//! Every canvas is built programmatically, so the suite carries no fixtures,
//! needs no network and produces the same bytes on every machine.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use image::{Rgba, RgbaImage};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Path to the binary under test, as cargo built it.
fn bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe path");
    p.pop(); // the deps/ directory
    if p.ends_with("deps") {
        p.pop();
    }
    p.join(format!("stripcut{}", std::env::consts::EXE_SUFFIX))
}

fn run(args: &[&str]) -> Output {
    Command::new(bin())
        .args(args)
        .env_remove("NO_COLOR")
        .output()
        .expect("run stripcut")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("utf-8 stdout")
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("utf-8 stderr")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
const CLEAR: Rgba<u8> = Rgba([0, 0, 0, 0]);

/// Side margin left as background either side of a panel.
///
/// Real vertical-scroll pages are laid out with a margin, and background
/// auto-detection samples the edge columns -- a synthetic canvas whose panels
/// bleed to both edges would tell stripcut the ink colour is the page colour.
/// The margin keeps the fixtures honest. A row is still non-background as soon
/// as any inspected pixel is ink, so panel rows never become gutter rows.
const PANEL_INSET: u32 = 4;

/// Paint rows `[y0, y1)` edge to edge.
fn fill_rows(img: &mut RgbaImage, y0: u32, y1: u32, colour: Rgba<u8>) {
    for y in y0..y1.min(img.height()) {
        for x in 0..img.width() {
            img.put_pixel(x, y, colour);
        }
    }
}

/// Draw a panel across rows `[y0, y1)`, inset from both edges.
fn band(img: &mut RgbaImage, y0: u32, y1: u32, colour: Rgba<u8>) {
    let right = img.width().saturating_sub(PANEL_INSET);
    for y in y0..y1.min(img.height()) {
        for x in PANEL_INSET..right {
            img.put_pixel(x, y, colour);
        }
    }
}

/// A canvas of `bg` with each `(y0, y1)` painted as a solid panel.
fn strip(width: u32, height: u32, bg: Rgba<u8>, panels: &[(u32, u32)]) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(width, height, bg);
    for &(y0, y1) in panels {
        band(&mut img, y0, y1, Rgba([20, 30, 40, 255]));
    }
    img
}

/// Panels of `panel` rows separated by gutters of `gap` rows, from y=0.
fn regular_strip(width: u32, panel: u32, gap: u32, count: u32) -> (RgbaImage, Vec<(u32, u32)>) {
    let height = count * (panel + gap);
    let mut bands = Vec::new();
    let mut y = 0;
    while y + panel <= height {
        bands.push((y, y + panel));
        y += panel + gap;
    }
    (strip(width, height, WHITE, &bands), bands)
}

fn write_png(dir: &Path, name: &str, img: &RgbaImage) -> PathBuf {
    let path = dir.join(name);
    img.save(&path).expect("write png");
    path
}

fn scratch() -> TempDir {
    tempfile::tempdir().expect("tempdir")
}

/// Every file in a directory, sorted, as (name, bytes).
fn listing(dir: &Path) -> Vec<(String, Vec<u8>)> {
    if !dir.exists() {
        return Vec::new();
    }
    let mut out: Vec<(String, Vec<u8>)> = fs::read_dir(dir)
        .expect("read_dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_file())
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                fs::read(&p).expect("read"),
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Pull the first numeric `"key": <integer>` out of a JSON fragment.
///
/// Keys whose value is an array or an object (`"gutters": [...]`) are skipped,
/// so the same name can be both a collection and a counter.
fn json_int(doc: &str, key: &str) -> i64 {
    let needle = format!("\"{key}\": ");
    let mut from = 0;
    while let Some(rel) = doc[from..].find(&needle) {
        let at = from + rel + needle.len();
        let rest = &doc[at..];
        let end = rest
            .find(|c: char| !c.is_ascii_digit() && c != '-')
            .unwrap_or(rest.len());
        if end > 0 {
            return rest[..end].parse().expect("integer");
        }
        from = at;
    }
    panic!("no numeric {key} in\n{doc}");
}

/// The same, but only inside the run's `totals` object.
fn json_total(doc: &str, key: &str) -> i64 {
    let at = doc
        .find("\"totals\":")
        .unwrap_or_else(|| panic!("no totals in\n{doc}"));
    json_int(&doc[at..], key)
}

/// The tiles a plan produced, as `(y_start, y_end, cut)`.
fn planned_tiles(doc: &str) -> Vec<(u32, u32, String)> {
    let mut out = Vec::new();
    for chunk in doc.split("\"y_start\": ").skip(1) {
        let y_start: u32 = chunk[..chunk.find(',').unwrap()].parse().expect("y_start");
        let after = &chunk[chunk.find("\"y_end\": ").expect("y_end") + 9..];
        let y_end: u32 = after[..after.find(',').unwrap()].parse().expect("y_end");
        let after = &chunk[chunk.find("\"cut\": \"").expect("cut") + 8..];
        let cut = after[..after.find('"').unwrap()].to_string();
        out.push((y_start, y_end, cut));
    }
    out
}

/// The encoded size of each tile, in plan order.
fn tile_bytes(doc: &str) -> Vec<u64> {
    doc.split("\"y_start\": ")
        .skip(1)
        .map(|chunk| {
            let at = chunk.find("\"bytes\": ").expect("bytes") + 9;
            let rest = &chunk[at..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            rest[..end].parse().expect("tile bytes")
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Basics
// ---------------------------------------------------------------------------

#[test]
fn version_and_help_work() {
    let out = run(&["--version"]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).contains("stripcut"));

    let out = run(&["--help"]);
    assert_eq!(code(&out), 0);
    let help = stdout(&out);
    for flag in [
        "--preset",
        "--list-presets",
        "--max-height",
        "--min-height",
        "--target-height",
        "--max-bytes",
        "--min-gutter",
        "--tolerance",
        "--edge-margin",
        "--bg",
        "--join",
        "--format",
        "--quality",
        "--prefix",
        "--start-index",
        "--dry-run",
        "--check",
        "--gutters",
        "--json",
        "--force",
        "--no-color",
        "--quiet",
    ] {
        assert!(help.contains(flag), "--help does not mention {flag}");
    }
    assert!(help.contains("EXIT CODES"));
}

#[test]
fn no_input_is_a_usage_error() {
    let out = run(&[]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("no input given"));
}

#[test]
fn a_missing_file_is_an_io_error() {
    let out = run(&["/definitely/not/here.png", "--max-height", "100"]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("cannot read"));
}

#[test]
fn an_unknown_preset_names_the_known_ones() {
    let dir = scratch();
    let page = write_png(dir.path(), "page.png", &strip(40, 100, WHITE, &[(10, 90)]));
    let out = run(&[page.to_str().unwrap(), "--preset", "nope"]);
    assert_eq!(code(&out), 2);
    let err = stderr(&out);
    assert!(err.contains("unknown preset"));
    assert!(err.contains("webtoon-canvas"));
}

#[test]
fn slicing_without_a_height_ceiling_is_refused() {
    let dir = scratch();
    let page = write_png(dir.path(), "page.png", &strip(40, 100, WHITE, &[(10, 90)]));
    let out = run(&[page.to_str().unwrap(), "--dry-run"]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("--max-height is required"));

    // A preset that publishes no ceiling says so by name.
    let out = run(&[page.to_str().unwrap(), "--preset", "tapas", "--dry-run"]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("publishes no maximum image height"));
}

// ---------------------------------------------------------------------------
// --list-presets
// ---------------------------------------------------------------------------

#[test]
fn list_presets_shows_sources_dates_and_the_caveat() {
    let out = run(&["--list-presets", "--no-color"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(text.contains("webtoon-canvas"));
    assert!(text.contains("https://"));
    assert!(text.contains("verified on 2026-"));
    assert!(text.contains("Confirm against the platform's current guidelines"));
    assert!(!text.contains('\x1b'), "piped output must not be coloured");
}

#[test]
fn list_presets_json_is_machine_readable() {
    let out = run(&["--list-presets", "--json"]);
    assert_eq!(code(&out), 0);
    let doc = stdout(&out);
    assert!(doc.starts_with("{\n"));
    assert!(doc.contains("\"presets\": ["));
    assert!(doc.contains("\"source_url\":"));
    assert!(doc.contains("\"verified_on\":"));
    // Every preset carries both, or the table has no business shipping.
    let presets = doc.matches("\"name\":").count();
    assert_eq!(doc.matches("\"source_url\":").count(), presets);
    assert_eq!(doc.matches("\"verified_on\":").count(), presets);
}

// ---------------------------------------------------------------------------
// --gutters
// ---------------------------------------------------------------------------

#[test]
fn gutters_lists_the_runs_between_panels() {
    let dir = scratch();
    let (img, _) = regular_strip(40, 180, 20, 5);
    let page = write_png(dir.path(), "page.png", &img);
    let out = run(&[page.to_str().unwrap(), "--gutters", "--json"]);
    assert_eq!(code(&out), 0);
    let doc = stdout(&out);
    // Four internal gaps plus the trailing one at the bottom of the canvas.
    assert_eq!(json_total(&doc, "gutters"), 5);
    assert!(doc.contains("\"start\": 180, \"end\": 200, \"height\": 20, \"centre\": 190"));
    // Detection only; nothing is planned or encoded, so those knobs are null.
    assert!(doc.contains("\"max_height\": null"));
    assert!(doc.contains("\"format\": null"));
}

#[test]
fn min_gutter_filters_a_hairline_gap() {
    let dir = scratch();
    // A 4 px gap at 100 and a 40 px gap at 300.
    let img = strip(40, 500, WHITE, &[(0, 100), (104, 300), (340, 500)]);
    let page = write_png(dir.path(), "page.png", &img);

    let out = run(&[
        page.to_str().unwrap(),
        "--gutters",
        "--json",
        "--min-gutter",
        "1",
    ]);
    assert_eq!(json_total(&stdout(&out), "gutters"), 2);

    let out = run(&[
        page.to_str().unwrap(),
        "--gutters",
        "--json",
        "--min-gutter",
        "20",
    ]);
    let doc = stdout(&out);
    assert_eq!(json_total(&doc, "gutters"), 1);
    assert!(doc.contains("\"start\": 300, \"end\": 340"));
}

#[test]
fn a_full_bleed_page_reports_no_gutters_and_says_what_to_try() {
    let dir = scratch();
    // A vertical gradient: no two rows alike, no empty rows anywhere.
    let mut img = RgbaImage::new(40, 600);
    for y in 0..600u32 {
        let v = (y % 251) as u8;
        fill_rows(&mut img, y, y + 1, Rgba([v, v, v, 255]));
    }
    let page = write_png(dir.path(), "page.png", &img);
    let out = run(&[page.to_str().unwrap(), "--gutters", "--no-color"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(text.contains("No gutters were detected"));
    assert!(text.contains("--tolerance"));
    assert!(text.contains("full-bleed"));
}

// ---------------------------------------------------------------------------
// Cut quality: the core promise
// ---------------------------------------------------------------------------

#[test]
fn every_cut_lands_in_a_gutter_and_never_inside_a_panel() {
    let dir = scratch();
    // 12 panels of 400 px with 60 px gutters: 5520 px of canvas.
    let (img, panels) = regular_strip(80, 400, 60, 12);
    let page = write_png(dir.path(), "page.png", &img);
    let out = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--max-height",
        "1000",
        "--min-height",
        "300",
    ]);
    assert_eq!(code(&out), 0);
    let doc = stdout(&out);
    let tiles = planned_tiles(&doc);
    assert!(tiles.len() > 3, "expected several tiles, got {tiles:?}");

    for (y_start, y_end, cut) in &tiles {
        assert!(y_end > y_start);
        assert!(y_end - y_start <= 1000, "tile {y_start}..{y_end} too tall");
        assert!(y_end - y_start >= 300, "tile {y_start}..{y_end} too short");
        assert_ne!(cut, "hard-cut", "a page full of gutters must not hard-cut");
        // The seam itself must not be inside a panel.
        for &(p0, p1) in &panels {
            assert!(
                *y_end <= p0 || *y_end >= p1,
                "cut at {y_end} is inside panel {p0}..{p1}"
            );
        }
    }
    // The tiles tile the canvas exactly, with no gap and no overlap.
    assert_eq!(tiles[0].0, 0);
    assert_eq!(tiles.last().unwrap().1, img.height());
    for pair in tiles.windows(2) {
        assert_eq!(pair[0].1, pair[1].0);
    }
    assert_eq!(json_total(&doc, "hard_cuts"), 0);
}

#[test]
fn a_speech_balloon_inside_a_panel_is_never_used_as_a_cut() {
    let dir = scratch();
    // One tall panel with a white balloon in the middle of it. The balloon's
    // rows look like background, but they do not span the width.
    let mut img = strip(120, 3000, WHITE, &[(100, 2900)]);
    for y in 1400..1600 {
        for x in 20..100 {
            img.put_pixel(x, y, WHITE);
        }
    }
    let page = write_png(dir.path(), "page.png", &img);
    let out = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--max-height",
        "1600",
        "--min-height",
        "400",
    ]);
    let doc = stdout(&out);
    for (_, y_end, _) in planned_tiles(&doc) {
        assert!(
            !(1400..1600).contains(&y_end),
            "cut at {y_end} landed inside the speech balloon"
        );
    }
    // And the balloon never showed up as a gutter run in the first place.
    let out = run(&[page.to_str().unwrap(), "--gutters", "--json"]);
    assert!(!stdout(&out).contains("\"start\": 1400"));
}

#[test]
fn a_gradient_hard_cuts_but_stays_inside_the_limits() {
    let dir = scratch();
    let mut img = RgbaImage::new(40, 3000);
    for y in 0..3000u32 {
        let v = (y % 251) as u8;
        fill_rows(&mut img, y, y + 1, Rgba([v, 128, 255 - v, 255]));
    }
    let page = write_png(dir.path(), "page.png", &img);
    let out = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--max-height",
        "800",
        "--min-height",
        "200",
    ]);
    // A hard cut is a warning, not a failure.
    assert_eq!(code(&out), 0);
    let doc = stdout(&out);
    assert!(json_total(&doc, "hard_cuts") > 0);
    assert!(doc.contains("\"kind\": \"no_gutters\""));
    assert!(doc.contains("\"kind\": \"hard_cut\""));
    for (y_start, y_end, _) in planned_tiles(&doc) {
        assert!(y_end - y_start <= 800);
        assert!(y_end - y_start >= 200);
    }
    // The warning names the tile and the y position, as promised.
    assert!(doc.contains("cut forced at y="));
}

#[test]
fn a_gutter_at_the_window_edge_is_taken_and_one_past_it_is_not() {
    let dir = scratch();
    // Gutter rows 990..1011, centre 1000, exactly at --max-height 1000.
    let at_edge = strip(40, 3000, WHITE, &[(0, 990), (1011, 3000)]);
    let page = write_png(dir.path(), "edge.png", &at_edge);
    let out = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--max-height",
        "1000",
        "--min-height",
        "100",
    ]);
    let tiles = planned_tiles(&stdout(&out));
    assert_eq!((tiles[0].1, tiles[0].2.as_str()), (1000, "gutter"));

    // Now the same run shifted so its centre is 1002 and the run itself starts
    // past y=1000: nothing qualifies and nothing covers the forced cut.
    let past = strip(40, 3000, WHITE, &[(0, 1001), (1004, 3000)]);
    let page = write_png(dir.path(), "past.png", &past);
    let out = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--max-height",
        "1000",
        "--min-height",
        "100",
        "--min-gutter",
        "1",
    ]);
    let tiles = planned_tiles(&stdout(&out));
    assert_eq!((tiles[0].1, tiles[0].2.as_str()), (1000, "hard-cut"));
}

#[test]
fn target_height_moves_the_preferred_cut() {
    let dir = scratch();
    // Gutters centred near 600 and near 1400.
    let img = strip(40, 4000, WHITE, &[(0, 580), (620, 1380), (1420, 4000)]);
    let page = write_png(dir.path(), "page.png", &img);

    let near = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--max-height",
        "1600",
        "--min-height",
        "100",
        "--target-height",
        "600",
    ]);
    assert_eq!(planned_tiles(&stdout(&near))[0].1, 600);

    let far = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--max-height",
        "1600",
        "--min-height",
        "100",
    ]);
    assert_eq!(planned_tiles(&stdout(&far))[0].1, 1400);
}

// ---------------------------------------------------------------------------
// Detection knobs
// ---------------------------------------------------------------------------

#[test]
fn a_transparent_background_is_detected_on_its_own() {
    let dir = scratch();
    let img = strip(40, 2000, CLEAR, &[(0, 900), (1000, 2000)]);
    let page = write_png(dir.path(), "page.png", &img);
    let out = run(&[page.to_str().unwrap(), "--gutters", "--json"]);
    let doc = stdout(&out);
    assert!(doc.contains("\"background\": \"#00000000\""));
    assert!(doc.contains("\"background_source\": \"auto\""));
    assert!(doc.contains("\"start\": 900, \"end\": 1000"));
}

#[test]
fn bg_overrides_detection() {
    let dir = scratch();
    // A page whose background is red; the panels are white.
    let red = Rgba([220, 30, 30, 255]);
    let mut img = RgbaImage::from_pixel(40, 2000, red);
    fill_rows(&mut img, 0, 900, WHITE);
    fill_rows(&mut img, 1000, 2000, WHITE);
    let page = write_png(dir.path(), "page.png", &img);

    let auto = run(&[page.to_str().unwrap(), "--gutters", "--json"]);
    assert!(stdout(&auto).contains("\"background_source\": \"auto\""));

    let forced = run(&[
        page.to_str().unwrap(),
        "--gutters",
        "--json",
        "--bg",
        "#dc1e1e",
    ]);
    let doc = stdout(&forced);
    assert!(doc.contains("\"background\": \"#dc1e1eff\""));
    assert!(doc.contains("\"background_source\": \"flag\""));
    assert!(doc.contains("\"start\": 900, \"end\": 1000"));

    // Telling it the wrong colour finds nothing, which is the honest answer.
    let wrong = run(&[
        page.to_str().unwrap(),
        "--gutters",
        "--json",
        "--bg",
        "00ff00",
    ]);
    assert_eq!(json_total(&stdout(&wrong), "gutters"), 0);

    let bad = run(&[page.to_str().unwrap(), "--gutters", "--bg", "nothex"]);
    assert_eq!(code(&bad), 2);
    assert!(stderr(&bad).contains("invalid hex colour"));
}

#[test]
fn tolerance_absorbs_a_not_quite_white_page() {
    let dir = scratch();
    // The gutter rows are 250,250,250 against a pure-white detected page.
    let near = Rgba([250, 250, 250, 255]);
    let mut img = RgbaImage::from_pixel(40, 2000, WHITE);
    fill_rows(&mut img, 0, 900, Rgba([20, 20, 20, 255]));
    fill_rows(&mut img, 900, 1000, near);
    fill_rows(&mut img, 1000, 2000, Rgba([20, 20, 20, 255]));
    let page = write_png(dir.path(), "page.png", &img);

    let strict = run(&[
        page.to_str().unwrap(),
        "--gutters",
        "--json",
        "--tolerance",
        "0",
        "--bg",
        "ffffff",
    ]);
    assert_eq!(json_total(&stdout(&strict), "gutters"), 0);

    let loose = run(&[
        page.to_str().unwrap(),
        "--gutters",
        "--json",
        "--tolerance",
        "6",
        "--bg",
        "ffffff",
    ]);
    let doc = stdout(&loose);
    assert_eq!(json_total(&doc, "gutters"), 1);
    assert!(doc.contains("\"start\": 900, \"end\": 1000"));
}

#[test]
fn edge_margin_ignores_a_printed_border() {
    let dir = scratch();
    // A 2 px red rule down both edges makes every row non-empty.
    let mut img = strip(40, 2000, WHITE, &[(0, 900), (1000, 2000)]);
    for y in 0..2000 {
        for x in [0, 1, 38, 39] {
            img.put_pixel(x, y, Rgba([200, 0, 0, 255]));
        }
    }
    let page = write_png(dir.path(), "page.png", &img);

    let out = run(&[
        page.to_str().unwrap(),
        "--gutters",
        "--json",
        "--bg",
        "ffffff",
    ]);
    assert_eq!(json_total(&stdout(&out), "gutters"), 0);

    let out = run(&[
        page.to_str().unwrap(),
        "--gutters",
        "--json",
        "--bg",
        "ffffff",
        "--edge-margin",
        "2",
    ]);
    let doc = stdout(&out);
    assert_eq!(json_total(&doc, "gutters"), 1);
    assert!(doc.contains("\"start\": 900, \"end\": 1000"));
    assert!(doc.contains("\"edge_margin\": 2"));
}

// ---------------------------------------------------------------------------
// Writing tiles
// ---------------------------------------------------------------------------

#[test]
fn slicing_writes_zero_padded_tiles_that_reassemble_the_canvas() {
    let dir = scratch();
    let out_dir = dir.path().join("out");
    let (img, _) = regular_strip(60, 300, 40, 8);
    let page = write_png(dir.path(), "episode.png", &img);
    let out = run(&[
        page.to_str().unwrap(),
        "-o",
        out_dir.to_str().unwrap(),
        "--max-height",
        "800",
        "--min-height",
        "200",
        "--no-color",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));

    let files = listing(&out_dir);
    assert!(files.len() > 2);
    assert_eq!(files[0].0, "episode_001.png");
    assert_eq!(files[1].0, "episode_002.png");

    // The tiles stack back into exactly the source canvas.
    let mut rows = 0u32;
    for (name, _) in &files {
        let tile = image::open(out_dir.join(name))
            .expect("decode tile")
            .to_rgba8();
        assert_eq!(tile.width(), img.width());
        for y in 0..tile.height() {
            for x in 0..tile.width() {
                assert_eq!(
                    tile.get_pixel(x, y),
                    img.get_pixel(x, rows + y),
                    "{name} row {y} differs from source row {}",
                    rows + y
                );
            }
        }
        rows += tile.height();
    }
    assert_eq!(rows, img.height());
}

#[test]
fn prefix_and_start_index_control_the_names() {
    let dir = scratch();
    let out_dir = dir.path().join("out");
    let (img, _) = regular_strip(40, 200, 40, 6);
    let page = write_png(dir.path(), "page.png", &img);
    let out = run(&[
        page.to_str().unwrap(),
        "-o",
        out_dir.to_str().unwrap(),
        "--max-height",
        "500",
        "--min-height",
        "150",
        "--prefix",
        "ep01",
        "--start-index",
        "7",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let names: Vec<String> = listing(&out_dir).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names[0], "ep01_007.png");
    assert!(names.iter().all(|n| n.starts_with("ep01_")));

    // Past 999 the padding grows so the sequence still sorts correctly.
    let wide = dir.path().join("wide");
    let out = run(&[
        page.to_str().unwrap(),
        "-o",
        wide.to_str().unwrap(),
        "--max-height",
        "500",
        "--min-height",
        "150",
        "--start-index",
        "999",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let names: Vec<String> = listing(&wide).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names[0], "page_0999.png");
    assert_eq!(names[1], "page_1000.png");
}

#[test]
fn existing_files_are_never_overwritten_without_force() {
    let dir = scratch();
    let out_dir = dir.path().join("out");
    let (img, _) = regular_strip(40, 200, 40, 4);
    let page = write_png(dir.path(), "page.png", &img);
    let args = [
        page.to_str().unwrap(),
        "-o",
        out_dir.to_str().unwrap(),
        "--max-height",
        "500",
        "--min-height",
        "150",
    ];

    assert_eq!(code(&run(&args)), 0);
    let first = listing(&out_dir);
    assert!(!first.is_empty());

    // A second run refuses, and touches nothing.
    let out = run(&args);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("already exists"));
    assert!(stderr(&out).contains("--force"));
    assert_eq!(listing(&out_dir), first);

    // With --force it goes through and produces the same bytes.
    let mut forced: Vec<&str> = args.to_vec();
    forced.push("--force");
    let out = run(&forced);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(listing(&out_dir), first);
}

#[test]
fn a_refusal_to_overwrite_happens_before_anything_is_written() {
    let dir = scratch();
    let out_dir = dir.path().join("out");
    fs::create_dir_all(&out_dir).unwrap();
    // Only the third tile's name is taken.
    fs::write(out_dir.join("page_003.png"), b"not an image").unwrap();

    let (img, _) = regular_strip(40, 200, 40, 6);
    let page = write_png(dir.path(), "page.png", &img);
    let out = run(&[
        page.to_str().unwrap(),
        "-o",
        out_dir.to_str().unwrap(),
        "--max-height",
        "500",
        "--min-height",
        "150",
    ]);
    assert_eq!(code(&out), 2);
    // Tiles 1 and 2 must not have been written on the way to discovering it.
    assert_eq!(
        listing(&out_dir),
        vec![("page_003.png".to_string(), b"not an image".to_vec())]
    );
}

#[test]
fn dry_run_writes_absolutely_nothing() {
    let dir = scratch();
    let out_dir = dir.path().join("out");
    let (img, _) = regular_strip(40, 200, 40, 6);
    let page = write_png(dir.path(), "page.png", &img);
    let out = run(&[
        page.to_str().unwrap(),
        "-o",
        out_dir.to_str().unwrap(),
        "--dry-run",
        "--max-height",
        "500",
        "--min-height",
        "150",
        "--no-color",
    ]);
    assert_eq!(code(&out), 0);
    assert!(!out_dir.exists(), "--dry-run created the output directory");
    let text = stdout(&out);
    assert!(text.contains("nothing written"));
    // It still reports the estimated size of each tile.
    assert!(text.contains("page_001.png"));
    assert!(stdout(&out).contains("KiB") || stdout(&out).contains(" B"));
}

#[test]
fn the_same_input_and_flags_produce_byte_identical_output() {
    let dir = scratch();
    let (img, _) = regular_strip(60, 250, 40, 6);
    let page = write_png(dir.path(), "page.png", &img);
    let mut runs = Vec::new();
    for n in 0..2 {
        let out_dir = dir.path().join(format!("out{n}"));
        let out = run(&[
            page.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
            "--max-height",
            "600",
            "--min-height",
            "200",
            "--format",
            "jpg",
            "--quality",
            "80",
        ]);
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        runs.push(listing(&out_dir));
    }
    assert_eq!(runs[0], runs[1], "two identical runs differ");
    assert!(runs[0][0].0.ends_with(".jpg"));

    // The reports match too.
    let report = |name: &str| {
        stdout(&run(&[
            page.to_str().unwrap(),
            "-o",
            dir.path().join(name).to_str().unwrap(),
            "--dry-run",
            "--json",
            "--max-height",
            "600",
            "--min-height",
            "200",
        ]))
    };
    assert_eq!(report("a"), report("b"));
}

#[test]
fn format_is_preserved_by_default_and_overridden_by_the_flag() {
    let dir = scratch();
    let (img, _) = regular_strip(40, 200, 40, 4);
    let png = write_png(dir.path(), "page.png", &img);
    let jpg = dir.path().join("page.jpg");
    image::DynamicImage::ImageRgba8(img.clone())
        .to_rgb8()
        .save(&jpg)
        .unwrap();

    let out_dir = dir.path().join("kept");
    let out = run(&[
        jpg.to_str().unwrap(),
        "-o",
        out_dir.to_str().unwrap(),
        "--max-height",
        "500",
        "--min-height",
        "150",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(listing(&out_dir)[0].0.ends_with(".jpg"));

    for (format, ext, magic) in [
        ("png", ".png", &b"\x89PNG"[..]),
        ("webp", ".webp", &b"RIFF"[..]),
        ("jpg", ".jpg", &b"\xff\xd8"[..]),
    ] {
        let out_dir = dir.path().join(format!("as-{format}"));
        let out = run(&[
            png.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
            "--max-height",
            "500",
            "--min-height",
            "150",
            "--format",
            format,
        ]);
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        let files = listing(&out_dir);
        assert!(files[0].0.ends_with(ext), "{format}: {}", files[0].0);
        assert!(files[0].1.starts_with(magic), "{format} magic bytes");
    }

    let out = run(&[
        png.to_str().unwrap(),
        "-o",
        dir.path().join("bad").to_str().unwrap(),
        "--max-height",
        "500",
        "--format",
        "tiff",
    ]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("unsupported format"));
}

#[test]
fn max_bytes_drives_the_quality_ladder_and_reports_an_impossible_budget() {
    let dir = scratch();
    // A noisy canvas, so JPEG cannot compress it away for free.
    let mut img = RgbaImage::new(400, 1600);
    let mut state: u32 = 0x9e37_79b9;
    for px in img.pixels_mut() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let b = state.to_le_bytes();
        *px = Rgba([b[0], b[1], b[2], 255]);
    }
    let page = write_png(dir.path(), "noise.png", &img);

    let base = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--max-height",
        "800",
        "--min-height",
        "200",
        "--format",
        "jpg",
        "--quality",
        "95",
    ]);
    let biggest = *tile_bytes(&stdout(&base)).iter().max().expect("tiles");

    // A budget the top quality cannot meet, but the ladder can. --max-bytes is
    // a per-tile ceiling, so it is compared against the largest single tile.
    let budget = biggest * 2 / 3;
    let out = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--max-height",
        "800",
        "--min-height",
        "200",
        "--format",
        "jpg",
        "--quality",
        "95",
        "--max-bytes",
        &budget.to_string(),
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = stdout(&out);
    assert!(
        tile_bytes(&doc).iter().all(|b| *b <= budget),
        "a tile is still over the budget: {:?} > {budget}",
        tile_bytes(&doc)
    );
    assert!(tile_bytes(&doc).iter().max() < Some(&biggest));
    assert!(!doc.contains("\"kind\": \"oversize\""));

    // A budget nothing can meet: exit 1, and a warning that names the way out.
    let out = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--max-height",
        "800",
        "--min-height",
        "200",
        "--format",
        "jpg",
        "--max-bytes",
        "500",
    ]);
    assert_eq!(code(&out), 1);
    let doc = stdout(&out);
    assert!(doc.contains("\"kind\": \"oversize\""));
    assert!(doc.contains("over --max-bytes 500"));
    assert!(doc.contains("\"ok\": false"));

    // The same budget on a lossless format explains why the ladder cannot help.
    let out = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--max-height",
        "800",
        "--min-height",
        "200",
        "--format",
        "png",
        "--max-bytes",
        "500",
    ]);
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).contains("lossless"));
}

// ---------------------------------------------------------------------------
// Multiple inputs and --join
// ---------------------------------------------------------------------------

#[test]
fn join_slices_across_the_seam_between_two_files() {
    let dir = scratch();
    // Part A ends mid-panel; part B starts with the rest of it. The gutter
    // that matters straddles the file boundary only when the parts are joined.
    let a = strip(40, 1000, WHITE, &[(0, 980)]);
    let b = strip(40, 1000, WHITE, &[(40, 1000)]);
    let pa = write_png(dir.path(), "part_a.png", &a);
    let pb = write_png(dir.path(), "part_b.png", &b);

    let out = run(&[
        pa.to_str().unwrap(),
        pb.to_str().unwrap(),
        "--join",
        "--dry-run",
        "--json",
        "--max-height",
        "1500",
        "--min-height",
        "300",
        "--min-gutter",
        "20",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = stdout(&out);
    // One canvas, 2000 rows, built from two sources.
    assert_eq!(doc.matches("\"mode\":").count(), 1);
    assert!(doc.contains("\"height\": 2000"));
    assert!(doc.contains("part_a.png"));
    assert!(doc.contains("part_b.png"));
    // The 60-row gutter spanning 980..1040 crosses the file boundary.
    assert!(doc.contains("\"start\": 980, \"end\": 1040"));
    let tiles = planned_tiles(&doc);
    assert_eq!(tiles[0].1, 1010);
    assert_eq!(tiles[0].2, "gutter");
}

#[test]
fn join_refuses_inputs_of_different_widths() {
    let dir = scratch();
    let a = write_png(dir.path(), "a.png", &strip(40, 500, WHITE, &[(0, 400)]));
    let b = write_png(dir.path(), "b.png", &strip(50, 500, WHITE, &[(0, 400)]));
    let out = run(&[
        a.to_str().unwrap(),
        b.to_str().unwrap(),
        "--join",
        "--dry-run",
        "--max-height",
        "300",
    ]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("one width for every input"));
}

#[test]
fn a_directory_is_taken_in_sorted_order_as_separate_canvases() {
    let dir = scratch();
    let pages = dir.path().join("pages");
    fs::create_dir_all(&pages).unwrap();
    let (img, _) = regular_strip(40, 200, 40, 4);
    // Written out of order on purpose.
    write_png(&pages, "p03.png", &img);
    write_png(&pages, "p01.png", &img);
    write_png(&pages, "p02.png", &img);
    fs::write(pages.join("notes.txt"), b"ignored").unwrap();

    let out = run(&[
        pages.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--max-height",
        "500",
        "--min-height",
        "150",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = stdout(&out);
    // Three canvases, in sorted order, and the .txt was skipped.
    assert_eq!(doc.matches("\"mode\": \"dry-run\"").count(), 3);
    assert!(!doc.contains("notes.txt"));
    let first = doc.find("p01.png").expect("p01");
    let second = doc.find("p02.png").expect("p02");
    let third = doc.find("p03.png").expect("p03");
    assert!(first < second && second < third);
    // Numbering runs on across the episode rather than restarting.
    assert!(doc.contains("\"file\": \"p01_001.png\""));
    assert!(doc.contains("\"file\": \"p02_003.png\""));
}

#[test]
fn an_empty_directory_is_a_usage_error() {
    let dir = scratch();
    let empty = dir.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    let out = run(&[empty.to_str().unwrap(), "--dry-run", "--max-height", "500"]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("no png/jpg/webp files"));
}

// ---------------------------------------------------------------------------
// --check
// ---------------------------------------------------------------------------

#[test]
fn check_passes_a_compliant_set_and_fails_a_bad_one() {
    let dir = scratch();
    let good = dir.path().join("good");
    fs::create_dir_all(&good).unwrap();
    for i in 1..=3 {
        write_png(
            &good,
            &format!("t{i}.png"),
            &RgbaImage::from_pixel(800, 1200, WHITE),
        );
    }
    let out = run(&[
        good.to_str().unwrap(),
        "--check",
        "--preset",
        "webtoon-canvas",
        "--no-color",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("all within the limits"));
    assert!(text.contains("limits: width <= 800 px, height <= 1280 px"));

    // Too tall.
    let bad = dir.path().join("tall");
    fs::create_dir_all(&bad).unwrap();
    write_png(&bad, "t1.png", &RgbaImage::from_pixel(800, 4000, WHITE));
    let out = run(&[
        bad.to_str().unwrap(),
        "--check",
        "--preset",
        "webtoon-canvas",
        "--no-color",
    ]);
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).contains("height is 4000 px, maximum is 1280 px"));

    // Too wide.
    let wide = dir.path().join("wide");
    fs::create_dir_all(&wide).unwrap();
    write_png(&wide, "t1.png", &RgbaImage::from_pixel(1200, 1000, WHITE));
    let out = run(&[
        wide.to_str().unwrap(),
        "--check",
        "--preset",
        "webtoon-canvas",
        "--no-color",
    ]);
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).contains("width is 1200 px, maximum is 800 px"));
}

#[test]
fn check_enforces_a_byte_ceiling_and_a_required_width() {
    let dir = scratch();
    let files = dir.path().join("files");
    fs::create_dir_all(&files).unwrap();
    // Noisy, so the PNG is genuinely large.
    let mut img = RgbaImage::new(940, 900);
    let mut state: u32 = 7;
    for px in img.pixels_mut() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let b = state.to_le_bytes();
        *px = Rgba([b[0], b[1], b[2], 255]);
    }
    write_png(&files, "t1.png", &img);

    // Tapas requires exactly 940 px, which this is, and under 10 MB, also fine
    // -- but PNG is on its accepted list, so this passes.
    let out = run(&[
        files.to_str().unwrap(),
        "--check",
        "--preset",
        "tapas",
        "--no-color",
    ]);
    assert_eq!(code(&out), 0, "{}", stdout(&out));

    // An explicit byte ceiling it cannot meet.
    let out = run(&[
        files.to_str().unwrap(),
        "--check",
        "--max-bytes",
        "1000",
        "--no-color",
    ]);
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).contains("maximum is 1000 bytes"));

    // A recommended width is reported, never enforced. LINE recommends 800 px
    // and permits 400-10000, so a flat 940x900 page inside that range passes
    // with a note rather than a failure.
    let flat = dir.path().join("flat");
    fs::create_dir_all(&flat).unwrap();
    write_png(&flat, "t1.png", &RgbaImage::from_pixel(940, 900, WHITE));
    let out = run(&[
        flat.to_str().unwrap(),
        "--check",
        "--preset",
        "line-manga-indies",
        "--no-color",
    ]);
    let text = stdout(&out);
    assert_eq!(code(&out), 0, "{text}");
    assert!(text.contains("recommends 800 px"));
    assert!(text.contains("all within the limits"));

    // The same preset does enforce its 2 MB per-image ceiling, which the noisy
    // page above is well over.
    let out = run(&[
        files.to_str().unwrap(),
        "--check",
        "--preset",
        "line-manga-indies",
        "--no-color",
    ]);
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).contains("maximum is 2000000 bytes"));

    // And a width outside the permitted range is a real failure.
    let narrow = dir.path().join("narrow");
    fs::create_dir_all(&narrow).unwrap();
    write_png(&narrow, "t1.png", &RgbaImage::from_pixel(300, 900, WHITE));
    let out = run(&[
        narrow.to_str().unwrap(),
        "--check",
        "--preset",
        "line-manga-indies",
        "--no-color",
    ]);
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).contains("width is 300 px, minimum is 400 px"));
}

#[test]
fn check_counts_the_episode_as_a_whole() {
    let dir = scratch();
    let files = dir.path().join("files");
    fs::create_dir_all(&files).unwrap();
    for i in 1..=4 {
        write_png(
            &files,
            &format!("t{i:02}.png"),
            &RgbaImage::from_pixel(100, 100, WHITE),
        );
    }
    let out = run(&[
        files.to_str().unwrap(),
        "--check",
        "--max-bytes",
        "1000000",
        "--json",
    ]);
    assert_eq!(code(&out), 0);
    assert_eq!(json_total(&stdout(&out), "checked"), 4);
}

#[test]
fn check_needs_something_to_check_against() {
    let dir = scratch();
    let page = write_png(dir.path(), "t.png", &RgbaImage::from_pixel(10, 10, WHITE));
    let out = run(&[page.to_str().unwrap(), "--check"]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("nothing to check against"));
}

#[test]
fn check_reports_in_json_too() {
    let dir = scratch();
    let files = dir.path().join("files");
    fs::create_dir_all(&files).unwrap();
    write_png(&files, "t1.png", &RgbaImage::from_pixel(800, 4000, WHITE));
    let out = run(&[
        files.to_str().unwrap(),
        "--check",
        "--preset",
        "webtoon-canvas",
        "--json",
    ]);
    assert_eq!(code(&out), 1);
    let doc = stdout(&out);
    assert!(doc.contains("\"mode\": \"check\""));
    assert!(doc.contains("\"ok\": false"));
    assert!(doc.contains("\"problems\": ["));
    assert!(doc.contains("\"limits\": {"));
    assert!(doc.contains("\"format\": \"png\""));
    assert_eq!(json_total(&doc, "failed"), 1);
}

// ---------------------------------------------------------------------------
// Output surfaces
// ---------------------------------------------------------------------------

#[test]
fn the_json_envelope_is_stable_for_one_canvas_and_for_many() {
    let dir = scratch();
    let (img, _) = regular_strip(40, 200, 40, 4);
    let a = write_png(dir.path(), "a.png", &img);
    let b = write_png(dir.path(), "b.png", &img);

    for args in [
        vec![a.to_str().unwrap()],
        vec![a.to_str().unwrap(), b.to_str().unwrap()],
    ] {
        let mut full = args.clone();
        full.extend([
            "--dry-run",
            "--json",
            "--max-height",
            "500",
            "--min-height",
            "150",
        ]);
        let out = run(&full);
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        let doc = stdout(&out);
        assert!(doc.starts_with("{\n  \"tool\": \"stripcut\",\n"));
        assert!(doc.ends_with("}\n"));
        for key in [
            "\"version\":",
            "\"schema\":",
            "\"ok\":",
            "\"runs\": [",
            "\"caveat\":",
            "\"mode\":",
            "\"inputs\":",
            "\"out_dir\":",
            "\"preset\":",
            "\"canvas\":",
            "\"settings\":",
            "\"gutters\":",
            "\"tiles\":",
            "\"checked\":",
            "\"limits\":",
            "\"episode_problems\":",
            "\"warnings\":",
            "\"totals\":",
        ] {
            assert!(doc.contains(key), "missing {key} for {args:?}");
        }
        assert_eq!(doc.matches("\"runs\": [").count(), 1);
        assert_eq!(doc.matches("\"mode\": \"dry-run\"").count(), args.len());
    }
}

#[test]
fn quiet_prints_one_line_and_json_is_never_coloured() {
    let dir = scratch();
    let (img, _) = regular_strip(40, 200, 40, 4);
    let page = write_png(dir.path(), "page.png", &img);
    let out = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--quiet",
        "--max-height",
        "500",
        "--min-height",
        "150",
    ]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert_eq!(text.lines().count(), 1);
    assert!(text.starts_with("stripcut: "));
    assert!(text.contains("tiles"));

    let out = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--max-height",
        "500",
    ]);
    assert!(!stdout(&out).contains('\x1b'));
}

#[test]
fn the_report_footer_cites_the_preset_source() {
    let dir = scratch();
    let (img, _) = regular_strip(800, 400, 60, 4);
    let page = write_png(dir.path(), "page.png", &img);
    let out = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--preset",
        "webtoon-canvas",
        "--no-color",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("preset: webtoon-canvas"));
    assert!(text.contains("https://www.webtoons.com/en/notice/detail?noticeNo=1766"));
    assert!(text.contains("verified on 2026-08-12"));
    assert!(text.contains("confirm against the platform's current guidelines"));
}

#[test]
fn a_preset_supplies_the_limits_a_run_does_not_state() {
    let dir = scratch();
    let (img, _) = regular_strip(800, 500, 60, 6);
    let page = write_png(dir.path(), "page.png", &img);
    let out = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--preset",
        "webtoon-canvas",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = stdout(&out);
    assert!(doc.contains("\"max_height\": 1280"));
    // The documented default floor is a quarter of the ceiling.
    assert!(doc.contains("\"min_height\": 320"));
    for (y_start, y_end, _) in planned_tiles(&doc) {
        assert!(y_end - y_start <= 1280);
    }
}

#[test]
fn quality_on_a_lossless_format_is_flagged_rather_than_silently_ignored() {
    let dir = scratch();
    let (img, _) = regular_strip(40, 200, 40, 4);
    let page = write_png(dir.path(), "page.png", &img);
    let out = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--max-height",
        "500",
        "--format",
        "png",
        "--quality",
        "50",
    ]);
    assert_eq!(code(&out), 0);
    let doc = stdout(&out);
    assert!(doc.contains("\"kind\": \"input\""));
    assert!(doc.contains("lossless"));
}

#[test]
fn modes_that_contradict_each_other_are_rejected() {
    let dir = scratch();
    let page = write_png(
        dir.path(),
        "page.png",
        &RgbaImage::from_pixel(10, 10, WHITE),
    );
    for pair in [
        ["--dry-run", "--check"],
        ["--dry-run", "--gutters"],
        ["--check", "--gutters"],
    ] {
        let out = run(&[page.to_str().unwrap(), pair[0], pair[1]]);
        assert_eq!(code(&out), 2, "{pair:?} should conflict");
    }
}

// ---------------------------------------------------------------------------
// Performance sanity
// ---------------------------------------------------------------------------

#[test]
fn a_tall_canvas_plans_without_trouble() {
    // 800x20000 is a realistic episode. This asserts correctness, not speed --
    // wall-clock assertions belong nowhere near CI.
    let dir = scratch();
    let (img, _) = regular_strip(800, 900, 100, 20);
    assert_eq!(img.height(), 20_000);
    let page = write_png(dir.path(), "long.png", &img);
    let out = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--json",
        "--max-height",
        "1280",
        "--min-height",
        "400",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = stdout(&out);
    assert_eq!(json_total(&doc, "hard_cuts"), 0);
    let tiles = planned_tiles(&doc);
    assert!(tiles.len() >= 16, "{} tiles", tiles.len());
    assert_eq!(tiles.last().unwrap().1, 20_000);
}

// ---------------------------------------------------------------------------
// --episodes and --preview
// ---------------------------------------------------------------------------

#[test]
fn episodes_slice_each_directory_into_its_own_folder() {
    let tmp = scratch();
    let ep1 = tmp.path().join("ep01");
    let ep2 = tmp.path().join("ep02");
    fs::create_dir_all(&ep1).unwrap();
    fs::create_dir_all(&ep2).unwrap();
    let (part, _) = regular_strip(120, 160, 40, 5); // 1000 rows
    write_png(&ep1, "a.png", &part);
    write_png(&ep1, "b.png", &part);
    write_png(&ep2, "only.png", &part);
    let out_dir = tmp.path().join("out");
    let out = run(&[
        "--episodes",
        ep1.to_str().unwrap(),
        ep2.to_str().unwrap(),
        "--max-height",
        "400",
        "-o",
        out_dir.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = stdout(&out);
    assert!(doc.contains("\"episode\": \"ep01\""));
    assert!(doc.contains("\"episode\": \"ep02\""));
    // ep01's two parts are stacked into one 2000-row canvas.
    assert!(doc.contains("\"height\": 2000"));
    let one = listing(&out_dir.join("ep01"));
    let two = listing(&out_dir.join("ep02"));
    assert!(one.iter().any(|(n, _)| n == "ep01_001.png"));
    // Numbering starts over for each episode.
    assert!(two.iter().any(|(n, _)| n == "ep02_001.png"));
    assert!(
        listing(&out_dir).is_empty(),
        "nothing loose in the top out-dir"
    );

    // Each sliced folder checks as one episode.
    let out = run(&[
        "--check",
        "--episodes",
        out_dir.join("ep01").to_str().unwrap(),
        out_dir.join("ep02").to_str().unwrap(),
        "--max-height",
        "400",
        "--no-color",
    ]);
    assert_eq!(code(&out), 0, "{}", stdout(&out));
    let text = stdout(&out);
    assert!(text.contains("episode: ep01") && text.contains("episode: ep02"));
}

#[test]
fn episodes_with_the_same_name_are_refused_before_writing() {
    let tmp = scratch();
    let a = tmp.path().join("a").join("ep");
    let b = tmp.path().join("b").join("ep");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let (part, _) = regular_strip(80, 100, 20, 3);
    write_png(&a, "p.png", &part);
    write_png(&b, "p.png", &part);
    let out_dir = tmp.path().join("out");
    let out = run(&[
        "--episodes",
        a.to_str().unwrap(),
        b.to_str().unwrap(),
        "--max-height",
        "200",
        "-o",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("both named \"ep\""));
    assert!(!out_dir.exists());

    let out = run(&["--episodes", "--join", a.to_str().unwrap()]);
    assert_eq!(code(&out), 2, "--episodes and --join conflict");
}

#[test]
fn dry_run_preview_writes_only_the_preview() {
    let tmp = scratch();
    // A full-bleed block in the middle forces a hard cut.
    let mut img = strip(200, 3000, WHITE, &[(0, 800), (850, 1700), (1750, 3000)]);
    fill_rows(&mut img, 1800, 2900, Rgba([0, 0, 0, 255]));
    let page = write_png(tmp.path(), "page.png", &img);
    let out_dir = tmp.path().join("out");
    let out = run(&[
        page.to_str().unwrap(),
        "--dry-run",
        "--preview",
        "--preview-width",
        "100",
        "--max-height",
        "1000",
        "-o",
        out_dir.to_str().unwrap(),
        "--no-color",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("preview: "));
    let files = listing(&out_dir);
    assert_eq!(
        files.len(),
        1,
        "only the preview: {:?}",
        files.iter().map(|f| &f.0).collect::<Vec<_>>()
    );
    assert_eq!(files[0].0, "page_preview.png");
    let preview = image::load_from_memory(&files[0].1).unwrap().to_rgba8();
    assert_eq!(preview.width(), stripcut::preview::RIBBON + 100);
    assert_eq!(preview.height(), 1500);
    let has = |want: [u8; 3]| {
        preview
            .pixels()
            .any(|p| p[0] == want[0] && p[1] == want[1] && p[2] == want[2])
    };
    assert!(has([26, 158, 60]), "a green gutter cut");
    assert!(has([214, 40, 40]), "a red hard cut");

    // A second run refuses to overwrite the preview without --force.
    let again = run(&[
        page.to_str().unwrap(),
        "--preview",
        "--max-height",
        "1000",
        "-o",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(code(&again), 2);
    assert!(stderr(&again).contains("page_preview.png already exists"));

    // --check over the folder skips the preview.
    let sliced = run(&[
        page.to_str().unwrap(),
        "--max-height",
        "1000",
        "-o",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(code(&sliced), 0, "{}", stderr(&sliced));
    let check = run(&[
        out_dir.to_str().unwrap(),
        "--check",
        "--max-height",
        "1000",
        "--json",
    ]);
    assert_eq!(code(&check), 0, "{}", stdout(&check));
    assert!(!stdout(&check).contains("page_preview.png"));
}

#[test]
fn slicing_holds_an_episode_to_the_presets_image_count() {
    let tmp = scratch();
    // mechacomic-creators accepts at most 60 images per episode.
    let (img, _) = regular_strip(300, 80, 20, 70);
    let page = write_png(tmp.path(), "long.png", &img);
    let out = run(&[
        page.to_str().unwrap(),
        "--preset",
        "mechacomic-creators",
        "--max-height",
        "100",
        "--dry-run",
        "--no-color",
    ]);
    assert_eq!(code(&out), 1, "{}", stdout(&out));
    let text = stdout(&out);
    assert!(text.contains("episode limits:"), "{text}");
    assert!(text.contains("at most 60 are accepted per episode"));
}

#[test]
fn several_canvases_get_numbered_previews() {
    let tmp = scratch();
    let (img, _) = regular_strip(100, 100, 20, 4);
    let a = write_png(tmp.path(), "a.png", &img);
    let b = write_png(tmp.path(), "b.png", &img);
    let out_dir = tmp.path().join("out");
    let out = run(&[
        a.to_str().unwrap(),
        b.to_str().unwrap(),
        "--prefix",
        "ep",
        "--preview",
        "--max-height",
        "300",
        "-o",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let names: Vec<String> = listing(&out_dir).into_iter().map(|(n, _)| n).collect();
    assert!(names.contains(&"ep_1_preview.png".to_string()), "{names:?}");
    assert!(names.contains(&"ep_2_preview.png".to_string()), "{names:?}");

    let wide = run(&[
        a.to_str().unwrap(),
        "--gutters",
        "--preview",
        "--preview-width",
        "5000",
    ]);
    assert_eq!(code(&wide), 2);
    assert!(stderr(&wide).contains("--preview-width must be between 1 and 4096"));
}
