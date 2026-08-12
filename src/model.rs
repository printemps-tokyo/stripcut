//! The shapes the report renderers consume.
//!
//! Everything here is plain data with no clock and no absolute paths of its
//! own, so the same input and the same flags always render the same bytes.

use crate::analyze::Gutter;
use crate::check::{CheckItem, CheckLimits};
use crate::presets::Preset;

/// Version of the JSON document shape. Bumped when a field changes meaning.
pub const SCHEMA_VERSION: u32 = 1;

/// Printed under `--list-presets` and at the foot of every report.
pub const CAVEAT: &str = "Preset limits were read from the official pages listed above on the \
                          verification date shown; platforms change their guidelines. Confirm \
                          against the platform's current guidelines before uploading.";

/// What a warning is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningKind {
    /// A cut had to be forced at `--max-height` because no gutter qualified.
    HardCut,
    /// The canvas produced no gutters at all.
    NoGutters,
    /// The final tile came out shorter than `--min-height`.
    ShortTile,
    /// A tile could not be brought under `--max-bytes`.
    Oversize,
    /// Something about the input the artist should know.
    Input,
}

impl WarningKind {
    /// Machine token used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            WarningKind::HardCut => "hard_cut",
            WarningKind::NoGutters => "no_gutters",
            WarningKind::ShortTile => "short_tile",
            WarningKind::Oversize => "oversize",
            WarningKind::Input => "input",
        }
    }
}

/// One thing worth looking at, optionally tied to a tile by 0-based plan index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub kind: WarningKind,
    /// 0-based index into the plan, not the output file number.
    pub tile: Option<usize>,
    pub message: String,
}

/// One input file that fed the canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInfo {
    pub path: String,
    pub width: u32,
    pub height: u32,
}

/// The canvas that was actually analysed, after any `--join`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasInfo {
    pub width: u32,
    pub height: u32,
    pub sources: Vec<SourceInfo>,
}

/// Every knob the run used, resolved: flags beat preset beats default.
///
/// The detection knobs are always set. The planning and encoding knobs are
/// `None` in `--gutters` mode, which detects and stops without planning or
/// encoding anything -- reporting them there would be reporting values the run
/// never used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub max_height: Option<u32>,
    pub min_height: Option<u32>,
    pub target_height: Option<u32>,
    pub min_gutter: u32,
    pub tolerance: u8,
    pub edge_margin: u32,
    pub background: String,
    /// "auto" or "flag".
    pub background_source: &'static str,
    pub format: Option<&'static str>,
    pub quality: Option<u8>,
    pub max_bytes: Option<u64>,
}

/// One planned or written tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileReport {
    /// The number in the output filename.
    pub number: u32,
    pub file: String,
    pub y_start: u32,
    pub y_end: u32,
    pub height: u32,
    /// "gutter", "hard-cut" or "end".
    pub cut: &'static str,
    pub gutter: Option<Gutter>,
    pub bytes: Option<u64>,
    /// The quality the encoder settled on, for lossy formats only.
    pub quality: Option<u8>,
    pub written: bool,
}

/// The whole run, ready to render as text or JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    /// "slice", "dry-run", "gutters" or "check".
    pub mode: &'static str,
    pub inputs: Vec<String>,
    pub out_dir: Option<String>,
    pub preset: Option<&'static Preset>,
    pub canvas: Option<CanvasInfo>,
    pub settings: Option<Settings>,
    pub gutters: Vec<Gutter>,
    pub tiles: Vec<TileReport>,
    pub checked: Vec<CheckItem>,
    /// The limits `--check` held the files to.
    pub limits: Option<CheckLimits>,
    /// Episode-level `--check` violations: too many images, too many bytes.
    pub episode_problems: Vec<String>,
    pub warnings: Vec<Warning>,
    /// False when the run failed a limit it was asked to respect.
    pub ok: bool,
}

impl RunReport {
    pub fn new(mode: &'static str) -> Self {
        RunReport {
            mode,
            inputs: Vec::new(),
            out_dir: None,
            preset: None,
            canvas: None,
            settings: None,
            gutters: Vec::new(),
            tiles: Vec::new(),
            checked: Vec::new(),
            limits: None,
            episode_problems: Vec::new(),
            warnings: Vec::new(),
            ok: true,
        }
    }

    /// Total bytes across every tile that has a size.
    pub fn total_bytes(&self) -> u64 {
        self.tiles.iter().filter_map(|t| t.bytes).sum()
    }

    /// Warnings of one kind, for the summary counters.
    pub fn count_warnings(&self, kind: WarningKind) -> usize {
        self.warnings.iter().filter(|w| w.kind == kind).count()
    }

    /// 0 on success, 1 when a limit the run was asked to respect was breached.
    pub fn exit_code(&self) -> i32 {
        i32::from(!self.ok)
    }
}
