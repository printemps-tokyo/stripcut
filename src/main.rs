//! Command-line entry point for stripcut.
//!
//! This layer does three things and nothing else: it turns flags into settings,
//! it moves bytes on and off the filesystem, and it prints. Every decision
//! about where a cut goes, whether a row is background, how big a tile encodes
//! to and whether a file passes a platform's rules lives in the library, over
//! in-memory buffers, where it is testable without a temp directory.

use std::collections::BTreeSet;
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use image::{DynamicImage, ImageFormat, ImageReader, Rgba, RgbaImage};

use stripcut::analyze::{detect_background, find_gutters, DEFAULT_MIN_GUTTER, DEFAULT_TOLERANCE};
use stripcut::check::{check_episode, check_one, CheckItem, CheckLimits};
use stripcut::color::{format_color, parse_color};
use stripcut::encode::{encode_within, Format, DEFAULT_QUALITY};
use stripcut::model::{
    CanvasInfo, RunReport, Settings, SourceInfo, TileReport, Warning, WarningKind,
};
use stripcut::plan::{plan_cuts, PlanOptions};
use stripcut::presets::{find_preset, preset_names, Preset};
use stripcut::report::{
    render_json, render_presets_json, render_presets_text, render_text, summary_line, RenderOptions,
};
use stripcut::{check_canvas_limits, crop_rows, join_canvases};

/// File extensions picked up when an input is a directory.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];

/// Default `--min-height`, as a fraction of `--max-height`.
///
/// A floor stops the planner taking a gutter 40 rows down just because it is
/// the only one in range. A quarter of the ceiling is loose enough never to
/// fight a normal page and tight enough to catch that case.
const MIN_HEIGHT_DIVISOR: u32 = 4;

const AFTER_HELP: &str = "\
EXIT CODES:
  0  the run did what it was asked to do
  1  a limit was breached: a --check violation, or a tile that cannot be
     brought under --max-bytes
  2  usage error, unreadable input, or a refusal to overwrite without --force

  A hard cut is a warning, not a failure. It means stripcut could not find a
  gutter and cut at --max-height anyway; the report names the tile and the y
  position so you can look at that seam. Exit stays 0.

MODES:
  (default)    slice the canvas and write the tiles
  --dry-run    print the planned cuts and write nothing
  --gutters    print the detected runs of background rows and stop
  --check      validate images that are already sliced, and stop

TUNING DETECTION:
  A row counts as a gutter row when every pixel in it is the page background
  within --tolerance. Raise --tolerance for a scanned or JPEG-compressed page
  whose white is not exactly white. Use --edge-margin to ignore a printed
  border down the sides. Use --bg when auto-detection picks the wrong colour
  (--bg 0000 means 'the background is fully transparent'). Raise --min-gutter
  to stop hairline seams between touching panels counting as cut points.

  Run --gutters first when a page is not cutting where you expect. It shows
  exactly what stripcut can see.

Preset numbers were read from the platform pages named in --list-presets on the
verification dates shown. Platforms change their guidelines; confirm against
the current page before uploading.";

/// Slice a vertical-scroll comic canvas in the gutters, not through the art.
#[derive(Parser, Debug)]
#[command(
    name = "stripcut",
    version,
    about,
    long_about = None,
    after_help = AFTER_HELP
)]
struct Cli {
    /// Image files, or directories whose images are taken in sorted order.
    #[arg(value_name = "INPUTS")]
    inputs: Vec<PathBuf>,

    /// Directory the tiles are written to.
    #[arg(short, long, value_name = "DIR", default_value = "out")]
    out_dir: PathBuf,

    /// Take limits from a platform preset (see --list-presets).
    #[arg(long, value_name = "NAME")]
    preset: Option<String>,

    /// Print the preset table with its sources and verification dates.
    #[arg(long)]
    list_presets: bool,

    /// Hard ceiling on a tile's height, in pixels. Never exceeded.
    #[arg(long, value_name = "PX")]
    max_height: Option<u32>,

    /// Floor on a tile's height (default: a quarter of --max-height).
    #[arg(long, value_name = "PX")]
    min_height: Option<u32>,

    /// The tile height to aim for (default: --max-height).
    #[arg(long, value_name = "PX")]
    target_height: Option<u32>,

    /// Byte budget per tile. Lossy formats drop quality to meet it.
    #[arg(long, value_name = "BYTES")]
    max_bytes: Option<u64>,

    /// Shortest run of background rows that counts as a gutter.
    #[arg(long, value_name = "PX")]
    min_gutter: Option<u32>,

    /// Per-channel slack when deciding a pixel is background (0-255).
    #[arg(long, value_name = "N")]
    tolerance: Option<u8>,

    /// Ignore this many pixels at the left and right edge of every row.
    #[arg(long, value_name = "PX")]
    edge_margin: Option<u32>,

    /// Page background as hex, e.g. ffffff or 0000 for fully transparent.
    #[arg(long, value_name = "HEX")]
    bg: Option<String>,

    /// Treat every input as one continuous canvas, stacked in order.
    #[arg(long)]
    join: bool,

    /// Output format (default: keep the source format).
    #[arg(long, value_name = "FMT")]
    format: Option<String>,

    /// Encoder quality for jpg, 1-100. PNG and WebP are lossless.
    #[arg(long, value_name = "N")]
    quality: Option<u8>,

    /// Output filename prefix (default: the input's file stem).
    #[arg(long, value_name = "NAME")]
    prefix: Option<String>,

    /// Number the first tile from here.
    #[arg(long, value_name = "N", default_value_t = 1)]
    start_index: u32,

    /// Print the planned cuts and write nothing.
    #[arg(long, conflicts_with_all = ["check", "gutters"])]
    dry_run: bool,

    /// Validate already-sliced images against the limits, and stop.
    #[arg(long, conflicts_with = "gutters")]
    check: bool,

    /// Print the detected gutter runs, and stop.
    #[arg(long)]
    gutters: bool,

    /// Emit the machine-readable JSON document instead of the human report.
    #[arg(long)]
    json: bool,

    /// Overwrite existing output files.
    #[arg(long)]
    force: bool,

    /// Never emit ANSI color codes.
    #[arg(long)]
    no_color: bool,

    /// Print only the one-line summary.
    #[arg(short, long)]
    quiet: bool,
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("stripcut: {err:#}");
            std::process::exit(2);
        }
    }
}

fn run() -> Result<i32> {
    let cli = Cli::parse();

    // Color is opt-out via the flag, the NO_COLOR convention, or a non-tty
    // stdout; JSON is never colored.
    let opts = RenderOptions {
        color: !cli.no_color
            && !cli.json
            && std::env::var_os("NO_COLOR").is_none()
            && std::io::stdout().is_terminal(),
    };
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    if cli.list_presets {
        let text = if cli.json {
            render_presets_json()
        } else {
            render_presets_text(opts)
        };
        out.write_all(text.as_bytes())?;
        out.flush()?;
        return Ok(0);
    }

    let preset = resolve_preset(cli.preset.as_deref())?;
    if cli.inputs.is_empty() {
        bail!("no input given. Pass one or more image files or a directory, or --list-presets");
    }
    let inputs = collect_inputs(&cli.inputs)?;

    let reports = if cli.check {
        vec![run_check(&cli, preset, &inputs)?]
    } else {
        run_slice(&cli, preset, &inputs)?
    };

    if cli.json {
        out.write_all(render_json(&reports).as_bytes())?;
    } else if cli.quiet {
        for report in &reports {
            writeln!(out, "stripcut: {}", summary_line(report, opts))?;
        }
    } else {
        for (i, report) in reports.iter().enumerate() {
            if i > 0 {
                writeln!(out)?;
            }
            out.write_all(render_text(report, opts).as_bytes())?;
        }
    }
    out.flush()?;

    Ok(i32::from(!reports.iter().all(|r| r.ok)))
}

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

fn resolve_preset(name: Option<&str>) -> Result<Option<&'static Preset>> {
    let Some(name) = name else {
        return Ok(None);
    };
    find_preset(name).map(Some).ok_or_else(|| {
        anyhow!(
            "unknown preset {name:?}. Known presets: {}",
            preset_names().join(", ")
        )
    })
}

fn has_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.as_str()))
}

/// Expand directories into their images, in sorted order.
///
/// A file named on the command line is taken as given, in the order given: if
/// you type three files for `--join` you mean that order. A directory is
/// sorted, so the same directory always produces the same sequence.
fn collect_inputs(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for path in paths {
        let meta = fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
        if !meta.is_dir() {
            out.push(path.clone());
            continue;
        }
        // BTreeSet sorts and de-duplicates in one step.
        let mut found: BTreeSet<PathBuf> = BTreeSet::new();
        for entry in
            fs::read_dir(path).with_context(|| format!("cannot list {}", path.display()))?
        {
            let entry = entry.with_context(|| format!("cannot list {}", path.display()))?;
            let child = entry.path();
            if child.is_file() && has_image_extension(&child) {
                found.insert(child);
            }
        }
        if found.is_empty() {
            bail!(
                "no png/jpg/webp files in {} (stripcut does not recurse into subdirectories)",
                path.display()
            );
        }
        out.extend(found);
    }
    Ok(out)
}

/// The format token for an already-encoded file, when stripcut can write it.
fn format_token(format: Option<ImageFormat>) -> Option<&'static str> {
    match format {
        Some(ImageFormat::Png) => Some("png"),
        Some(ImageFormat::Jpeg) => Some("jpg"),
        Some(ImageFormat::WebP) => Some("webp"),
        _ => None,
    }
}

/// Decode one file into an RGBA canvas, refusing anything absurdly large
/// before the allocation happens.
fn load_canvas(path: &Path) -> Result<(RgbaImage, Option<&'static str>)> {
    let reader = ImageReader::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?
        .with_guessed_format()
        .with_context(|| format!("cannot read {}", path.display()))?;
    let token = format_token(reader.format());

    let mut reader = reader;
    // stripcut applies its own, stricter cap below; the crate's default
    // allocation limit would otherwise reject a legitimately tall episode.
    reader.no_limits();
    let decoder = reader
        .into_decoder()
        .with_context(|| format!("cannot decode {}", path.display()))?;
    let (width, height) = image::ImageDecoder::dimensions(&decoder);
    check_canvas_limits(width, height).map_err(|e| anyhow!("{}: {e}", path.display()))?;

    let img = DynamicImage::from_decoder(decoder)
        .with_context(|| format!("cannot decode {}", path.display()))?
        .to_rgba8();
    Ok((img, token))
}

/// Read an already-sliced file's dimensions and format without decoding it.
fn probe_image(path: &Path) -> Result<(u32, u32, Option<&'static str>)> {
    let reader = ImageReader::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?
        .with_guessed_format()
        .with_context(|| format!("cannot read {}", path.display()))?;
    let token = format_token(reader.format());
    let (width, height) = reader
        .into_dimensions()
        .with_context(|| format!("cannot read the image header of {}", path.display()))?;
    Ok((width, height, token))
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

// ---------------------------------------------------------------------------
// --check
// ---------------------------------------------------------------------------

fn run_check(cli: &Cli, preset: Option<&'static Preset>, inputs: &[PathBuf]) -> Result<RunReport> {
    let mut limits = match preset {
        Some(p) => CheckLimits::from_preset(p),
        None => CheckLimits::default(),
    };
    // An explicit flag overrides whatever the preset published.
    if let Some(v) = cli.max_height {
        limits.max_height = Some(v);
    }
    if let Some(v) = cli.min_height {
        limits.min_height = Some(v);
    }
    if let Some(v) = cli.max_bytes {
        limits.max_bytes = Some(v);
    }
    if limits.is_empty() {
        bail!(
            "--check has nothing to check against. Pass --preset, or give limits with \
             --max-height / --min-height / --max-bytes"
        );
    }

    let mut report = RunReport::new("check");
    report.inputs = cli.inputs.iter().map(|p| display(p)).collect();
    report.preset = preset;

    let mut total_bytes = 0u64;
    for path in inputs {
        let (width, height, format) = probe_image(path)?;
        let bytes = fs::metadata(path)
            .with_context(|| format!("cannot stat {}", path.display()))?
            .len();
        total_bytes += bytes;
        report.checked.push(check_one(
            &display(path),
            width,
            height,
            bytes,
            format,
            &limits,
        ));
    }
    report.episode_problems = check_episode(report.checked.len(), total_bytes, &limits);
    report.ok = report.checked.iter().all(CheckItem::ok) && report.episode_problems.is_empty();
    report.limits = Some(limits);
    Ok(report)
}

// ---------------------------------------------------------------------------
// slice / --dry-run / --gutters
// ---------------------------------------------------------------------------

/// One canvas to work on: its sources, its pixels, and the format it came in.
struct Canvas {
    pixels: RgbaImage,
    sources: Vec<SourceInfo>,
    /// The source format, when every input shared one stripcut can write.
    source_format: Option<&'static str>,
    /// The stem the output filenames default to.
    stem: String,
}

fn canvas_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "tile".to_string())
}

fn build_canvases(cli: &Cli, inputs: &[PathBuf]) -> Result<Vec<Canvas>> {
    let mut loaded = Vec::with_capacity(inputs.len());
    for path in inputs {
        let (pixels, format) = load_canvas(path)?;
        loaded.push((path.clone(), pixels, format));
    }

    if !cli.join {
        return Ok(loaded
            .into_iter()
            .map(|(path, pixels, format)| Canvas {
                sources: vec![SourceInfo {
                    path: display(&path),
                    width: pixels.width(),
                    height: pixels.height(),
                }],
                stem: canvas_stem(&path),
                pixels,
                source_format: format,
            })
            .collect());
    }

    let sources: Vec<SourceInfo> = loaded
        .iter()
        .map(|(path, pixels, _)| SourceInfo {
            path: display(path),
            width: pixels.width(),
            height: pixels.height(),
        })
        .collect();
    let stem = canvas_stem(&loaded[0].0);
    // A single format only carries over when every part agrees on one.
    let first_format = loaded[0].2;
    let source_format = if loaded.iter().all(|(_, _, f)| *f == first_format) {
        first_format
    } else {
        None
    };
    let parts: Vec<RgbaImage> = loaded.into_iter().map(|(_, pixels, _)| pixels).collect();
    let pixels = join_canvases(&parts).map_err(|e| anyhow!("{e}"))?;
    Ok(vec![Canvas {
        pixels,
        sources,
        source_format,
        stem,
    }])
}

fn run_slice(
    cli: &Cli,
    preset: Option<&'static Preset>,
    inputs: &[PathBuf],
) -> Result<Vec<RunReport>> {
    let canvases = build_canvases(cli, inputs)?;
    let mode = if cli.gutters {
        "gutters"
    } else if cli.dry_run {
        "dry-run"
    } else {
        "slice"
    };

    let mut reports = Vec::with_capacity(canvases.len());
    // Numbering runs across the whole episode, so a directory of parts comes
    // out as one continuous sequence rather than restarting at 1 per file.
    let mut next_index = cli.start_index;
    for canvas in &canvases {
        let (report, used) = slice_one(cli, preset, canvas, mode, next_index)?;
        next_index = used;
        reports.push(report);
    }
    Ok(reports)
}

/// Resolved background colour plus where it came from.
fn resolve_background(
    cli: &Cli,
    canvas: &Canvas,
    edge_margin: u32,
) -> Result<(Rgba<u8>, &'static str)> {
    match &cli.bg {
        Some(hex) => {
            let colour = parse_color(hex).map_err(|e| anyhow!("{e}"))?;
            Ok((colour, "flag"))
        }
        None => Ok((detect_background(&canvas.pixels, edge_margin), "auto")),
    }
}

/// The height ceiling, from the flag or the preset, or a clear complaint.
fn resolve_max_height(cli: &Cli, preset: Option<&Preset>) -> Result<u32> {
    if let Some(v) = cli.max_height {
        if v == 0 {
            bail!("--max-height must be at least 1");
        }
        return Ok(v);
    }
    match preset {
        Some(p) => p.max_height.ok_or_else(|| {
            anyhow!(
                "preset {:?} publishes no maximum image height, so --max-height is required. \
                 See --list-presets",
                p.name
            )
        }),
        None => Err(anyhow!(
            "--max-height is required (or use --preset; see --list-presets)"
        )),
    }
}

fn slice_one(
    cli: &Cli,
    preset: Option<&'static Preset>,
    canvas: &Canvas,
    mode: &'static str,
    start_index: u32,
) -> Result<(RunReport, u32)> {
    let edge_margin = cli.edge_margin.unwrap_or(0);
    let tolerance = cli.tolerance.unwrap_or(DEFAULT_TOLERANCE);
    let min_gutter = cli.min_gutter.unwrap_or(DEFAULT_MIN_GUTTER);
    let (background, background_source) = resolve_background(cli, canvas, edge_margin)?;

    let mut report = RunReport::new(mode);
    report.inputs = cli.inputs.iter().map(|p| display(p)).collect();
    report.preset = preset;
    report.canvas = Some(CanvasInfo {
        width: canvas.pixels.width(),
        height: canvas.pixels.height(),
        sources: canvas.sources.clone(),
    });
    report.gutters = find_gutters(
        &canvas.pixels,
        background,
        tolerance,
        edge_margin,
        min_gutter,
    );

    if mode == "gutters" {
        report.settings = Some(Settings {
            max_height: None,
            min_height: None,
            target_height: None,
            min_gutter,
            tolerance,
            edge_margin,
            background: format_color(background),
            background_source,
            format: None,
            quality: None,
            max_bytes: None,
        });
        return Ok((report, start_index));
    }

    let max_height = resolve_max_height(cli, preset)?;
    let min_height = cli
        .min_height
        .unwrap_or(max_height / MIN_HEIGHT_DIVISOR)
        .min(max_height);
    let max_bytes = cli.max_bytes.or_else(|| preset.and_then(|p| p.max_bytes));

    // Preserve the source format unless --format says otherwise. A canvas whose
    // format stripcut cannot write (or a --join of mixed formats) falls back to
    // PNG, which is lossless and accepted everywhere the presets cover.
    let format = match &cli.format {
        Some(name) => Format::parse(name).map_err(|e| anyhow!("{e}"))?,
        None => match canvas.source_format {
            Some(token) => Format::parse(token).map_err(|e| anyhow!("{e}"))?,
            None => Format::Png,
        },
    };
    let quality = cli.quality.unwrap_or(DEFAULT_QUALITY);
    if !(1..=100).contains(&quality) {
        bail!("--quality must be between 1 and 100");
    }
    if cli.quality.is_some() && !format.is_lossy() {
        report.warnings.push(Warning {
            kind: WarningKind::Input,
            tile: None,
            message: format!(
                "--quality has no effect on {}: this encoder is lossless",
                format.as_str()
            ),
        });
    }

    report.settings = Some(Settings {
        max_height: Some(max_height),
        min_height: Some(min_height),
        target_height: cli.target_height,
        min_gutter,
        tolerance,
        edge_margin,
        background: format_color(background),
        background_source,
        format: Some(format.as_str()),
        quality: Some(quality),
        max_bytes,
    });

    let plan = plan_cuts(
        canvas.pixels.height(),
        &report.gutters,
        &PlanOptions {
            max_height,
            min_height,
            target_height: cli.target_height,
        },
    )
    .map_err(|e| anyhow!("{e}"))?;
    report.warnings.extend(plan.warnings.iter().cloned());

    let prefix = cli.prefix.clone().unwrap_or_else(|| canvas.stem.clone());
    let last_number = start_index.saturating_add(plan.tiles.len().saturating_sub(1) as u32);
    // Three digits unless the episode genuinely needs more, so tiles sort
    // correctly in every file browser and shell glob.
    let pad = last_number.to_string().len().max(3);

    let writing = mode == "slice";
    let out_dir = &cli.out_dir;
    if writing {
        report.out_dir = Some(display(out_dir));
    }

    // Every target path is resolved and checked before a single byte is
    // written, so a run either writes the whole episode or refuses up front.
    let mut targets = Vec::with_capacity(plan.tiles.len());
    for (i, _) in plan.tiles.iter().enumerate() {
        let number = start_index + i as u32;
        let name = format!("{prefix}_{number:0pad$}.{}", format.as_str());
        targets.push((number, name));
    }
    if writing && !cli.force {
        for (_, name) in &targets {
            let path = out_dir.join(name);
            if path.exists() {
                bail!(
                    "{} already exists. Pass --force to overwrite, or choose another --out-dir",
                    path.display()
                );
            }
        }
    }
    if writing {
        fs::create_dir_all(out_dir)
            .with_context(|| format!("cannot create {}", out_dir.display()))?;
    }

    for (tile, (number, name)) in plan.tiles.iter().zip(targets.iter()) {
        let pixels =
            crop_rows(&canvas.pixels, tile.y_start, tile.y_end).map_err(|e| anyhow!("{e}"))?;
        let encoded = encode_within(&pixels, format, quality, background, max_bytes)
            .map_err(|e| anyhow!("{e}"))?;

        if !encoded.within_limit {
            report.ok = false;
            let limit = max_bytes.unwrap_or(0);
            let advice = if format.is_lossy() {
                "even at the lowest quality on the ladder"
            } else {
                "and this format is lossless, so quality cannot be traded away \
                 (try --format jpg)"
            };
            report.warnings.push(Warning {
                kind: WarningKind::Oversize,
                tile: Some(tile.index),
                message: format!(
                    "{name} is {} bytes, over --max-bytes {limit} {advice}. Lower \
                     --max-height so the tile carries fewer rows",
                    encoded.len()
                ),
            });
        }

        if writing {
            let path = out_dir.join(name);
            fs::write(&path, &encoded.bytes)
                .with_context(|| format!("cannot write {}", path.display()))?;
        }

        report.tiles.push(TileReport {
            number: *number,
            file: name.clone(),
            y_start: tile.y_start,
            y_end: tile.y_end,
            height: tile.height(),
            cut: tile.cut.as_str(),
            gutter: tile.gutter,
            bytes: Some(encoded.len()),
            quality: encoded.quality,
            written: writing,
        });
    }

    let next_index = start_index.saturating_add(plan.tiles.len() as u32);
    Ok((report, next_index))
}
