//! Rendering: the human report, the `--list-presets` view and the JSON
//! document.
//!
//! Every renderer is a pure function of a [`RunReport`]. Nothing here reads the
//! clock, the terminal width or the environment, so the same run always renders
//! the same bytes -- which is what makes the output safe to diff in CI and what
//! the determinism tests pin.

use crate::analyze::Gutter;
use crate::check::CheckLimits;
use crate::json::{opt_num, opt_str, quote, sep, str_array};
use crate::model::{RunReport, Settings, CAVEAT, SCHEMA_VERSION};
use crate::presets::{Preset, PRESETS};

/// Rendering options for the human output.
#[derive(Debug, Clone, Copy, Default)]
pub struct RenderOptions {
    /// Emit ANSI color codes.
    pub color: bool,
}

fn paint(opts: RenderOptions, code: &str, s: &str) -> String {
    if opts.color {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// Bytes as a short human string. Binary units, one decimal place.
pub fn human_bytes(n: u64) -> String {
    const KIB: f64 = 1024.0;
    let n_f = n as f64;
    if n < 1024 {
        format!("{n} B")
    } else if n_f < KIB * KIB {
        format!("{:.1} KiB", n_f / KIB)
    } else if n_f < KIB * KIB * KIB {
        format!("{:.1} MiB", n_f / (KIB * KIB))
    } else {
        format!("{:.2} GiB", n_f / (KIB * KIB * KIB))
    }
}

/// `1 thing` / `3 things`, so the reports read like sentences.
fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {many}")
    }
}

// ---------------------------------------------------------------------------
// Human report
// ---------------------------------------------------------------------------

/// The full human-readable report for any mode, including the trailing caveat.
pub fn render_text(report: &RunReport, opts: RenderOptions) -> String {
    let mut out = format!("stripcut: {}\n", report.mode);

    if let Some(canvas) = &report.canvas {
        out.push_str(&format!(
            "canvas: {}x{} px from {}\n",
            canvas.width,
            canvas.height,
            plural(canvas.sources.len(), "file", "files")
        ));
        if canvas.sources.len() > 1 {
            for source in &canvas.sources {
                out.push_str(&format!(
                    "  {}  {}x{}\n",
                    source.path, source.width, source.height
                ));
            }
        }
    }
    if let Some(preset) = report.preset {
        out.push_str(&format!(
            "preset: {} ({}), verified on {}\n",
            preset.name, preset.platform, preset.verified_on
        ));
    }
    if let Some(settings) = &report.settings {
        out.push_str(&settings_lines(settings));
    }
    if let Some(dir) = &report.out_dir {
        out.push_str(&format!("out-dir: {dir}\n"));
    }
    out.push('\n');

    match report.mode {
        "gutters" => out.push_str(&gutter_table(&report.gutters, report.settings.as_ref())),
        "check" => out.push_str(&check_table(report, opts)),
        _ => out.push_str(&tile_table(report, opts)),
    }

    if !report.warnings.is_empty() {
        out.push('\n');
        out.push_str(&format!("{}:\n", paint(opts, "33", "warnings")));
        for warning in &report.warnings {
            let where_ = match warning.tile {
                Some(i) => format!("tile {}: ", i + 1),
                None => String::new(),
            };
            out.push_str(&format!(
                "  {} {where_}{}\n",
                paint(opts, "33", "!"),
                warning.message
            ));
        }
    }

    out.push('\n');
    out.push_str(&footer(report));
    out
}

fn settings_lines(s: &Settings) -> String {
    let mut out = String::new();
    if let (Some(max), Some(min)) = (s.max_height, s.min_height) {
        let target = match s.target_height {
            Some(t) => t.to_string(),
            None => format!("{max} (= max-height)"),
        };
        out.push_str(&format!(
            "settings: max-height {max}, min-height {min}, target-height {target}\n",
        ));
    }
    let lead = if out.is_empty() {
        "settings:"
    } else {
        "         "
    };
    out.push_str(&format!(
        "{lead} min-gutter {}, tolerance {}, edge-margin {}, background {} ({})\n",
        s.min_gutter, s.tolerance, s.edge_margin, s.background, s.background_source
    ));
    if let (Some(format), Some(quality)) = (s.format, s.quality) {
        let bytes = match s.max_bytes {
            Some(b) => format!("{b} ({})", human_bytes(b)),
            None => "unlimited".to_string(),
        };
        out.push_str(&format!(
            "          format {format}, quality {quality}, max-bytes {bytes}\n"
        ));
    }
    out
}

/// The `--gutters` view: every detected run of background rows.
fn gutter_table(gutters: &[Gutter], settings: Option<&Settings>) -> String {
    if gutters.is_empty() {
        return "No gutters were detected. Every cut on this canvas would be a hard cut.\n\
                Try a larger --tolerance, a smaller --min-gutter, an --edge-margin, or an\n\
                explicit --bg; or accept that this page is full-bleed.\n"
            .to_string();
    }
    let mut out = String::from("    #  start        end     height     centre\n");
    for (i, g) in gutters.iter().enumerate() {
        out.push_str(&format!(
            "  {:>3}  {:>7}  {:>9}  {:>9}  {:>9}\n",
            i + 1,
            g.start,
            g.end,
            g.len(),
            g.centre()
        ));
    }
    let rows: u32 = gutters.iter().map(|g| g.len()).sum();
    let tallest = gutters.iter().map(|g| g.len()).max().unwrap_or(0);
    let min_gutter = settings.map_or(0, |s| s.min_gutter);
    out.push_str(&format!(
        "\n{} at --min-gutter {min_gutter}, covering {rows} rows; tallest is {tallest} px\n",
        plural(gutters.len(), "gutter run", "gutter runs")
    ));
    out
}

/// The slice and dry-run view: the planned or written tiles.
fn tile_table(report: &RunReport, opts: RenderOptions) -> String {
    if report.tiles.is_empty() {
        return "No tiles were planned.\n".to_string();
    }
    let show_bytes = report.tiles.iter().any(|t| t.bytes.is_some());
    let mut out =
        String::from("    #  file                            y-range         height  cut");
    if show_bytes {
        out.push_str("            bytes");
    }
    out.push('\n');

    for tile in &report.tiles {
        let cut = match tile.cut {
            "hard-cut" => paint(opts, "33", "HARD-CUT"),
            "end" => "end     ".to_string(),
            other => format!("{other:<8}"),
        };
        let range = format!("{}-{}", tile.y_start, tile.y_end);
        out.push_str(&format!(
            "  {:>3}  {:<30}  {:>13}  {:>6}  {cut}",
            tile.number, tile.file, range, tile.height
        ));
        if show_bytes {
            let size = match tile.bytes {
                Some(b) => format!("{:>9} {:>9}", b, human_bytes(b)),
                None => String::new(),
            };
            out.push_str(&format!("  {size}"));
            if let Some(q) = tile.quality {
                out.push_str(&format!("  q{q}"));
            }
        }
        out.push('\n');
    }

    let hard = report.tiles.iter().filter(|t| t.cut == "hard-cut").count();
    let written = report.tiles.iter().filter(|t| t.written).count();
    let mut summary = plural(report.tiles.len(), "tile", "tiles");
    summary.push_str(&format!(
        ", {}",
        if hard == 0 {
            paint(opts, "32", "no hard cuts")
        } else {
            paint(opts, "33", &plural(hard, "hard cut", "hard cuts"))
        }
    ));
    if show_bytes {
        summary.push_str(&format!(", {} total", human_bytes(report.total_bytes())));
    }
    if written > 0 {
        summary.push_str(&format!(", {written} written"));
    } else if report.mode == "dry-run" {
        summary.push_str(", nothing written");
    }
    out.push('\n');
    out.push_str(&summary);
    out.push('\n');
    out
}

/// The `--check` view: one line per file, then the failures spelled out.
fn check_table(report: &RunReport, opts: RenderOptions) -> String {
    let mut out = String::new();
    if let Some(limits) = &report.limits {
        let described = limits.describe();
        if described.is_empty() {
            out.push_str(
                "This preset publishes no limit stripcut can check. Pass --max-height,\n\
                 --min-height or --max-bytes to check something concrete.\n\n",
            );
        } else {
            out.push_str(&format!("limits: {}\n\n", described.join(", ")));
        }
    }
    if report.checked.is_empty() {
        out.push_str("No images were found to check.\n");
        return out;
    }

    for item in &report.checked {
        let verdict = if item.ok() {
            paint(opts, "32", "OK  ")
        } else {
            paint(opts, "31", "FAIL")
        };
        out.push_str(&format!(
            "  {verdict}  {:<34}  {:>5}x{:<6}  {:>10}  {}\n",
            item.path,
            item.width,
            item.height,
            item.bytes,
            item.format.as_deref().unwrap_or("?")
        ));
        for problem in &item.problems {
            out.push_str(&format!("          - {problem}\n"));
        }
        for note in &item.notes {
            out.push_str(&paint(opts, "2", &format!("          note: {note}\n")));
        }
    }

    for problem in &report.episode_problems {
        out.push_str(&format!("  {}  {problem}\n", paint(opts, "31", "FAIL")));
    }

    let failed = report.checked.iter().filter(|i| !i.ok()).count() + report.episode_problems.len();
    let total: u64 = report.checked.iter().map(|i| i.bytes).sum();
    out.push('\n');
    out.push_str(&format!(
        "{} checked, {} total, {}\n",
        plural(report.checked.len(), "image", "images"),
        human_bytes(total),
        if failed == 0 {
            paint(opts, "32", "all within the limits")
        } else {
            paint(opts, "31", &format!("{failed} failed"))
        }
    ));
    out
}

/// The trailing caveat, naming the sources the run's numbers came from.
pub fn footer(report: &RunReport) -> String {
    match report.preset {
        Some(p) => format!(
            "Source: {} -- verified on {}; confirm against the platform's current guidelines.\n",
            p.source_url, p.verified_on
        ),
        None => String::new(),
    }
}

/// The one-line `--quiet` summary.
pub fn summary_line(report: &RunReport, opts: RenderOptions) -> String {
    match report.mode {
        "gutters" => plural(report.gutters.len(), "gutter run", "gutter runs"),
        "check" => {
            let failed =
                report.checked.iter().filter(|i| !i.ok()).count() + report.episode_problems.len();
            format!(
                "{} checked, {}",
                plural(report.checked.len(), "image", "images"),
                if failed == 0 {
                    paint(opts, "32", "all within the limits")
                } else {
                    paint(opts, "31", &format!("{failed} failed"))
                }
            )
        }
        _ => {
            let hard = report.tiles.iter().filter(|t| t.cut == "hard-cut").count();
            let mut line = plural(report.tiles.len(), "tile", "tiles");
            if hard > 0 {
                line.push_str(&format!(
                    ", {}",
                    paint(opts, "33", &plural(hard, "hard cut", "hard cuts"))
                ));
            }
            if report.total_bytes() > 0 {
                line.push_str(&format!(", {}", human_bytes(report.total_bytes())));
            }
            line
        }
    }
}

// ---------------------------------------------------------------------------
// --list-presets
// ---------------------------------------------------------------------------

/// A preset field as a number with a unit, or a dash when unpublished.
fn field(value: Option<u64>, unit: &str) -> String {
    match value {
        Some(v) => format!("{v}{unit}"),
        None => "-".to_string(),
    }
}

/// The `--list-presets` view.
pub fn render_presets_text(opts: RenderOptions) -> String {
    let mut out = String::from("stripcut presets\n\n");
    for preset in PRESETS {
        out.push_str(&format!(
            "{}  {}\n",
            paint(opts, "1", preset.name),
            preset.platform
        ));
        let width = match (
            preset.width,
            preset.min_width,
            preset.max_width,
            preset.recommended_width,
        ) {
            (Some(w), _, _, _) => format!("{w} px (required)"),
            (_, lo, hi, rec) if lo.is_some() || hi.is_some() => {
                let range = format!(
                    "{}-{} px",
                    lo.map_or("?".to_string(), |v| v.to_string()),
                    hi.map_or("?".to_string(), |v| v.to_string())
                );
                match rec {
                    Some(r) => format!("{range} ({r} px recommended)"),
                    None => range,
                }
            }
            (_, _, _, Some(r)) => format!("{r} px (recommended)"),
            _ => "-".to_string(),
        };
        out.push_str(&format!("  width         {width}\n"));
        out.push_str(&format!(
            "  height        {} .. {}\n",
            field(preset.min_height.map(u64::from), " px"),
            field(preset.max_height.map(u64::from), " px")
        ));
        out.push_str(&format!(
            "  bytes         {} per image, {} per episode\n",
            field(preset.max_bytes, ""),
            field(preset.max_total_bytes, "")
        ));
        out.push_str(&format!(
            "  images        {}\n",
            field(preset.max_tiles.map(u64::from), " max")
        ));
        out.push_str(&format!(
            "  formats       {}\n",
            if preset.formats.is_empty() {
                "-".to_string()
            } else {
                preset.formats.join("/")
            }
        ));
        out.push_str(&format!(
            "  plans alone   {}\n",
            if preset.can_plan() {
                "yes"
            } else {
                "no -- pass --max-height"
            }
        ));
        out.push_str(&format!("  notes         {}\n", preset.notes));
        out.push_str(&format!(
            "  source        {} (verified on {})\n\n",
            preset.source_url, preset.verified_on
        ));
    }
    out.push_str(&format!("{CAVEAT}\n"));
    out
}

/// `--list-presets --json`.
pub fn render_presets_json() -> String {
    let mut out = String::from("{\n");
    out.push_str("  \"tool\": \"stripcut\",\n");
    out.push_str(&format!(
        "  \"version\": {},\n",
        quote(env!("CARGO_PKG_VERSION"))
    ));
    out.push_str(&format!("  \"schema\": {SCHEMA_VERSION},\n"));
    out.push_str("  \"presets\": [\n");
    for (i, preset) in PRESETS.iter().enumerate() {
        out.push_str(&preset_json(preset, 4));
        out.push_str(sep(i, PRESETS.len()));
    }
    out.push_str("  ],\n");
    out.push_str(&format!("  \"caveat\": {}\n", quote(CAVEAT)));
    out.push_str("}\n");
    out
}

fn preset_json(preset: &Preset, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let inner = " ".repeat(indent + 2);
    let mut out = format!("{pad}{{\n");
    out.push_str(&format!("{inner}\"name\": {},\n", quote(preset.name)));
    out.push_str(&format!(
        "{inner}\"platform\": {},\n",
        quote(preset.platform)
    ));
    out.push_str(&format!("{inner}\"width\": {},\n", opt_num(preset.width)));
    out.push_str(&format!(
        "{inner}\"min_width\": {},\n",
        opt_num(preset.min_width)
    ));
    out.push_str(&format!(
        "{inner}\"max_width\": {},\n",
        opt_num(preset.max_width)
    ));
    out.push_str(&format!(
        "{inner}\"recommended_width\": {},\n",
        opt_num(preset.recommended_width)
    ));
    out.push_str(&format!(
        "{inner}\"min_height\": {},\n",
        opt_num(preset.min_height)
    ));
    out.push_str(&format!(
        "{inner}\"max_height\": {},\n",
        opt_num(preset.max_height)
    ));
    out.push_str(&format!(
        "{inner}\"max_bytes\": {},\n",
        opt_num(preset.max_bytes)
    ));
    out.push_str(&format!(
        "{inner}\"max_total_bytes\": {},\n",
        opt_num(preset.max_total_bytes)
    ));
    out.push_str(&format!(
        "{inner}\"max_tiles\": {},\n",
        opt_num(preset.max_tiles)
    ));
    out.push_str(&format!(
        "{inner}\"formats\": {},\n",
        str_array(preset.formats)
    ));
    out.push_str(&format!("{inner}\"can_plan\": {},\n", preset.can_plan()));
    out.push_str(&format!("{inner}\"notes\": {},\n", quote(preset.notes)));
    out.push_str(&format!(
        "{inner}\"source_url\": {},\n",
        quote(preset.source_url)
    ));
    out.push_str(&format!(
        "{inner}\"verified_on\": {}\n",
        quote(preset.verified_on)
    ));
    out.push_str(&format!("{pad}}}"));
    out
}

// ---------------------------------------------------------------------------
// JSON report
// ---------------------------------------------------------------------------

fn gutter_json(g: &Gutter) -> String {
    format!(
        "{{\"start\": {}, \"end\": {}, \"height\": {}, \"centre\": {}}}",
        g.start,
        g.end,
        g.len(),
        g.centre()
    )
}

fn limits_json(limits: &CheckLimits, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let inner = " ".repeat(indent + 2);
    let mut out = String::from("{\n");
    out.push_str(&format!("{inner}\"width\": {},\n", opt_num(limits.width)));
    out.push_str(&format!(
        "{inner}\"min_width\": {},\n",
        opt_num(limits.min_width)
    ));
    out.push_str(&format!(
        "{inner}\"max_width\": {},\n",
        opt_num(limits.max_width)
    ));
    out.push_str(&format!(
        "{inner}\"recommended_width\": {},\n",
        opt_num(limits.recommended_width)
    ));
    out.push_str(&format!(
        "{inner}\"min_height\": {},\n",
        opt_num(limits.min_height)
    ));
    out.push_str(&format!(
        "{inner}\"max_height\": {},\n",
        opt_num(limits.max_height)
    ));
    out.push_str(&format!(
        "{inner}\"max_bytes\": {},\n",
        opt_num(limits.max_bytes)
    ));
    out.push_str(&format!(
        "{inner}\"max_total_bytes\": {},\n",
        opt_num(limits.max_total_bytes)
    ));
    out.push_str(&format!(
        "{inner}\"max_tiles\": {},\n",
        opt_num(limits.max_tiles)
    ));
    out.push_str(&format!(
        "{inner}\"formats\": {}\n",
        str_array(&limits.formats)
    ));
    out.push_str(&format!("{pad}}}"));
    out
}

/// Shift every non-empty line of a rendered block right, for nesting.
fn indent_block(s: &str, by: usize) -> String {
    let pad = " ".repeat(by);
    s.lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{pad}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The machine-readable document.
///
/// Always the same envelope, whether one canvas was processed or twenty: a
/// `runs` array with one object per canvas. A consumer never has to branch on
/// how many inputs were given. The shape is documented in the README and
/// pinned by `SCHEMA_VERSION`.
pub fn render_json(reports: &[RunReport]) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("{\n");
    out.push_str("  \"tool\": \"stripcut\",\n");
    out.push_str(&format!(
        "  \"version\": {},\n",
        quote(env!("CARGO_PKG_VERSION"))
    ));
    out.push_str(&format!("  \"schema\": {SCHEMA_VERSION},\n"));
    out.push_str(&format!("  \"ok\": {},\n", reports.iter().all(|r| r.ok)));
    out.push_str("  \"runs\": [\n");
    for (i, report) in reports.iter().enumerate() {
        out.push_str(&indent_block(&run_json(report), 4));
        out.push_str(sep(i, reports.len()));
    }
    out.push_str("  ],\n");
    out.push_str(&format!("  \"caveat\": {}\n", quote(CAVEAT)));
    out.push_str("}\n");
    out
}

/// One canvas, as a JSON object with no trailing newline.
fn run_json(report: &RunReport) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str("{\n");
    out.push_str(&format!("  \"mode\": {},\n", quote(report.mode)));
    out.push_str(&format!("  \"ok\": {},\n", report.ok));
    out.push_str(&format!("  \"inputs\": {},\n", str_array(&report.inputs)));
    out.push_str(&format!(
        "  \"out_dir\": {},\n",
        opt_str(report.out_dir.as_deref())
    ));

    match report.preset {
        Some(p) => out.push_str(&format!(
            "  \"preset\": {},\n",
            preset_json(p, 2).trim_start()
        )),
        None => out.push_str("  \"preset\": null,\n"),
    }

    match &report.canvas {
        Some(c) => {
            out.push_str("  \"canvas\": {\n");
            out.push_str(&format!("    \"width\": {},\n", c.width));
            out.push_str(&format!("    \"height\": {},\n", c.height));
            out.push_str("    \"sources\": [\n");
            for (i, s) in c.sources.iter().enumerate() {
                out.push_str(&format!(
                    "      {{\"path\": {}, \"width\": {}, \"height\": {}}}",
                    quote(&s.path),
                    s.width,
                    s.height
                ));
                out.push_str(sep(i, c.sources.len()));
            }
            out.push_str("    ]\n  },\n");
        }
        None => out.push_str("  \"canvas\": null,\n"),
    }

    match &report.settings {
        Some(s) => {
            out.push_str("  \"settings\": {\n");
            out.push_str(&format!("    \"max_height\": {},\n", opt_num(s.max_height)));
            out.push_str(&format!("    \"min_height\": {},\n", opt_num(s.min_height)));
            out.push_str(&format!(
                "    \"target_height\": {},\n",
                opt_num(s.target_height)
            ));
            out.push_str(&format!("    \"min_gutter\": {},\n", s.min_gutter));
            out.push_str(&format!("    \"tolerance\": {},\n", s.tolerance));
            out.push_str(&format!("    \"edge_margin\": {},\n", s.edge_margin));
            out.push_str(&format!("    \"background\": {},\n", quote(&s.background)));
            out.push_str(&format!(
                "    \"background_source\": {},\n",
                quote(s.background_source)
            ));
            out.push_str(&format!("    \"format\": {},\n", opt_str(s.format)));
            out.push_str(&format!("    \"quality\": {},\n", opt_num(s.quality)));
            out.push_str(&format!("    \"max_bytes\": {}\n", opt_num(s.max_bytes)));
            out.push_str("  },\n");
        }
        None => out.push_str("  \"settings\": null,\n"),
    }

    out.push_str("  \"gutters\": [\n");
    for (i, g) in report.gutters.iter().enumerate() {
        out.push_str(&format!("    {}", gutter_json(g)));
        out.push_str(sep(i, report.gutters.len()));
    }
    out.push_str("  ],\n");

    out.push_str("  \"tiles\": [\n");
    for (i, t) in report.tiles.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!("      \"number\": {},\n", t.number));
        out.push_str(&format!("      \"file\": {},\n", quote(&t.file)));
        out.push_str(&format!("      \"y_start\": {},\n", t.y_start));
        out.push_str(&format!("      \"y_end\": {},\n", t.y_end));
        out.push_str(&format!("      \"height\": {},\n", t.height));
        out.push_str(&format!("      \"cut\": {},\n", quote(t.cut)));
        out.push_str(&format!(
            "      \"gutter\": {},\n",
            match &t.gutter {
                Some(g) => gutter_json(g),
                None => "null".to_string(),
            }
        ));
        out.push_str(&format!("      \"bytes\": {},\n", opt_num(t.bytes)));
        out.push_str(&format!("      \"quality\": {},\n", opt_num(t.quality)));
        out.push_str(&format!("      \"written\": {}\n", t.written));
        out.push_str("    }");
        out.push_str(sep(i, report.tiles.len()));
    }
    out.push_str("  ],\n");

    out.push_str("  \"checked\": [\n");
    for (i, c) in report.checked.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!("      \"path\": {},\n", quote(&c.path)));
        out.push_str(&format!("      \"width\": {},\n", c.width));
        out.push_str(&format!("      \"height\": {},\n", c.height));
        out.push_str(&format!("      \"bytes\": {},\n", c.bytes));
        out.push_str(&format!(
            "      \"format\": {},\n",
            opt_str(c.format.as_deref())
        ));
        out.push_str(&format!("      \"ok\": {},\n", c.ok()));
        out.push_str(&format!(
            "      \"problems\": {},\n",
            str_array(&c.problems)
        ));
        out.push_str(&format!("      \"notes\": {}\n", str_array(&c.notes)));
        out.push_str("    }");
        out.push_str(sep(i, report.checked.len()));
    }
    out.push_str("  ],\n");

    match &report.limits {
        Some(l) => out.push_str(&format!("  \"limits\": {},\n", limits_json(l, 2))),
        None => out.push_str("  \"limits\": null,\n"),
    }
    out.push_str(&format!(
        "  \"episode_problems\": {},\n",
        str_array(&report.episode_problems)
    ));

    out.push_str("  \"warnings\": [\n");
    for (i, w) in report.warnings.iter().enumerate() {
        out.push_str(&format!(
            "    {{\"kind\": {}, \"tile\": {}, \"message\": {}}}",
            quote(w.kind.as_str()),
            opt_num(w.tile),
            quote(&w.message)
        ));
        out.push_str(sep(i, report.warnings.len()));
    }
    out.push_str("  ],\n");

    let hard = report.tiles.iter().filter(|t| t.cut == "hard-cut").count();
    let failed = report.checked.iter().filter(|c| !c.ok()).count();
    // `totals` is the last member of a run object, so it takes no comma.
    out.push_str(&format!(
        "  \"totals\": {{\"tiles\": {}, \"written\": {}, \"bytes\": {}, \"hard_cuts\": {}, \
\"gutters\": {}, \"warnings\": {}, \"checked\": {}, \"failed\": {}}}\n",
        report.tiles.len(),
        report.tiles.iter().filter(|t| t.written).count(),
        report.total_bytes(),
        hard,
        report.gutters.len(),
        report.warnings.len(),
        report.checked.len(),
        failed + report.episode_problems.len(),
    ));
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CanvasInfo, SourceInfo, TileReport, Warning, WarningKind};

    fn tile(number: u32, y0: u32, y1: u32, cut: &'static str) -> TileReport {
        TileReport {
            number,
            file: format!("page_{number:03}.png"),
            y_start: y0,
            y_end: y1,
            height: y1 - y0,
            cut,
            gutter: None,
            bytes: Some(1234),
            quality: None,
            written: true,
        }
    }

    fn settings() -> Settings {
        Settings {
            max_height: Some(1280),
            min_height: Some(320),
            target_height: None,
            min_gutter: 12,
            tolerance: 6,
            edge_margin: 0,
            background: "#ffffffff".to_string(),
            background_source: "auto",
            format: Some("png"),
            quality: Some(85),
            max_bytes: Some(2_000_000),
        }
    }

    fn slice_report() -> RunReport {
        let mut report = RunReport::new("slice");
        report.inputs = vec!["page.png".to_string()];
        report.out_dir = Some("out".to_string());
        report.canvas = Some(CanvasInfo {
            width: 800,
            height: 2400,
            sources: vec![SourceInfo {
                path: "page.png".to_string(),
                width: 800,
                height: 2400,
            }],
        });
        report.settings = Some(settings());
        report.gutters = vec![Gutter {
            start: 1200,
            end: 1240,
        }];
        report.tiles = vec![tile(1, 0, 1220, "gutter"), tile(2, 1220, 2400, "end")];
        report
    }

    #[test]
    fn human_bytes_switches_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1_572_864), "1.5 MiB");
        assert!(human_bytes(3_221_225_472).ends_with("GiB"));
    }

    #[test]
    fn the_text_report_names_the_canvas_the_tiles_and_the_source() {
        let mut report = slice_report();
        report.preset = crate::presets::find_preset("webtoon-canvas");
        let text = render_text(&report, RenderOptions::default());
        assert!(text.contains("stripcut: slice"));
        assert!(text.contains("canvas: 800x2400 px from 1 file"));
        assert!(text.contains("page_001.png"));
        assert!(text.contains("0-1220"));
        assert!(text.contains("2 tiles, no hard cuts"));
        assert!(text.contains("confirm against the platform's current guidelines"));
    }

    #[test]
    fn a_hard_cut_is_shouted_and_a_warning_is_listed() {
        let mut report = slice_report();
        report.tiles[0].cut = "hard-cut";
        report.warnings.push(Warning {
            kind: WarningKind::HardCut,
            tile: Some(0),
            message: "no gutter between y=320 and y=1280".to_string(),
        });
        let text = render_text(&report, RenderOptions::default());
        assert!(text.contains("HARD-CUT"));
        assert!(text.contains("1 hard cut"));
        // Warnings are 0-based inside, 1-based on screen.
        assert!(text.contains("tile 1: no gutter between"));
    }

    #[test]
    fn colour_is_opt_in() {
        let report = slice_report();
        let plain = render_text(&report, RenderOptions { color: false });
        let painted = render_text(&report, RenderOptions { color: true });
        assert!(!plain.contains('\x1b'));
        assert!(painted.contains('\x1b'));
    }

    #[test]
    fn the_gutters_view_lists_runs_and_says_so_when_there_are_none() {
        let mut report = slice_report();
        report.mode = "gutters";
        report.tiles.clear();
        let text = render_text(&report, RenderOptions::default());
        assert!(text.contains("1200"));
        assert!(text.contains("1 gutter run at --min-gutter 12, covering 40 rows"));

        report.gutters.clear();
        let text = render_text(&report, RenderOptions::default());
        assert!(text.contains("No gutters were detected"));
    }

    #[test]
    fn the_check_view_separates_passes_from_failures() {
        let mut report = RunReport::new("check");
        report.limits = Some(CheckLimits {
            max_height: Some(1280),
            ..CheckLimits::default()
        });
        report.checked = vec![
            crate::check::check_one("a.png", 800, 1000, 10, Some("png"), &CheckLimits::default()),
            crate::check::check_one(
                "b.png",
                800,
                5000,
                10,
                Some("png"),
                &CheckLimits {
                    max_height: Some(1280),
                    ..CheckLimits::default()
                },
            ),
        ];
        report.ok = false;
        let text = render_text(&report, RenderOptions::default());
        assert!(text.contains("limits: height <= 1280 px"));
        assert!(text.contains("OK"));
        assert!(text.contains("FAIL"));
        assert!(text.contains("maximum is 1280 px"));
        assert!(text.contains("1 failed"));
    }

    #[test]
    fn dry_run_says_nothing_was_written() {
        let mut report = slice_report();
        report.mode = "dry-run";
        for t in &mut report.tiles {
            t.written = false;
        }
        let text = render_text(&report, RenderOptions::default());
        assert!(text.contains("nothing written"));
    }

    #[test]
    fn the_json_report_carries_every_top_level_key() {
        let report = slice_report();
        let json = render_json(std::slice::from_ref(&report));
        for key in [
            "\"tool\": \"stripcut\"",
            "\"schema\":",
            "\"runs\":",
            "\"mode\": \"slice\"",
            "\"ok\": true",
            "\"inputs\":",
            "\"out_dir\":",
            "\"preset\": null",
            "\"canvas\":",
            "\"settings\":",
            "\"gutters\":",
            "\"tiles\":",
            "\"checked\":",
            "\"limits\": null",
            "\"episode_problems\":",
            "\"warnings\":",
            "\"totals\":",
            "\"caveat\":",
        ] {
            assert!(json.contains(key), "missing {key} in\n{json}");
        }
        assert!(json.ends_with("}\n"));
    }

    #[test]
    fn the_envelope_holds_one_object_per_canvas() {
        let one = render_json(std::slice::from_ref(&slice_report()));
        assert_eq!(one.matches("\"mode\": \"slice\"").count(), 1);
        let two = render_json(&[slice_report(), slice_report()]);
        assert_eq!(two.matches("\"mode\": \"slice\"").count(), 2);
        // One failing canvas fails the envelope.
        let mut bad = slice_report();
        bad.ok = false;
        let mixed = render_json(&[slice_report(), bad]);
        assert!(mixed.starts_with("{\n  \"tool\": \"stripcut\","));
        assert!(mixed.contains("  \"ok\": false,\n"));
    }

    #[test]
    fn rendering_is_deterministic() {
        let reports = [slice_report()];
        assert_eq!(render_json(&reports), render_json(&reports));
        let opts = RenderOptions::default();
        assert_eq!(
            render_text(&reports[0], opts),
            render_text(&reports[0], opts)
        );
        assert_eq!(render_presets_text(opts), render_presets_text(opts));
        assert_eq!(render_presets_json(), render_presets_json());
    }

    #[test]
    fn the_preset_list_shows_sources_dates_and_the_caveat() {
        let text = render_presets_text(RenderOptions::default());
        for preset in PRESETS {
            assert!(text.contains(preset.name));
            assert!(text.contains(preset.source_url));
            assert!(text.contains(&format!("verified on {}", preset.verified_on)));
        }
        assert!(text.contains(CAVEAT));
        // A preset that cannot plan on its own has to say so.
        assert!(text.contains("no -- pass --max-height"));
    }

    #[test]
    fn the_preset_json_is_a_document_per_preset() {
        let json = render_presets_json();
        assert!(json.contains("\"can_plan\": true"));
        assert!(json.contains("\"can_plan\": false"));
        assert!(json.contains("\"recommended_width\": 800"));
        assert!(json.ends_with("}\n"));
    }

    #[test]
    fn the_quiet_summary_is_one_line_per_mode() {
        let opts = RenderOptions::default();
        let report = slice_report();
        assert_eq!(summary_line(&report, opts), "2 tiles, 2.4 KiB");
        let mut gutters = report.clone();
        gutters.mode = "gutters";
        assert_eq!(summary_line(&gutters, opts), "1 gutter run");
    }
}
