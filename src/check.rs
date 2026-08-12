//! `--check`: validate images that have already been sliced.
//!
//! This is the pre-upload gate. You point it at the directory you are about to
//! upload and it says, per file, whether the platform will take it -- so the
//! answer arrives in your terminal rather than from an upload form after
//! twenty files have gone through.
//!
//! Nothing here reads a file. The binary decodes the header and stats the file;
//! these functions take the numbers, so the rules are testable without a
//! filesystem.

use crate::presets::Preset;

/// The limits a set of files is being held to.
///
/// Every field is optional because platforms do not all publish every number,
/// and stripcut refuses to invent the ones they leave out. A `None` field is
/// simply not checked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckLimits {
    /// Exact canvas width the platform requires.
    pub width: Option<u32>,
    /// Smallest permitted width.
    pub min_width: Option<u32>,
    /// Largest permitted width.
    pub max_width: Option<u32>,
    /// A width the platform recommends. Reported as a note, never a violation.
    pub recommended_width: Option<u32>,
    /// Smallest accepted image height.
    pub min_height: Option<u32>,
    /// Largest accepted image height.
    pub max_height: Option<u32>,
    /// Largest accepted file size, in bytes.
    pub max_bytes: Option<u64>,
    /// Largest accepted total size for one episode, in bytes.
    pub max_total_bytes: Option<u64>,
    /// Largest accepted number of images in one episode.
    pub max_tiles: Option<u32>,
    /// Accepted upload formats, lowercase, empty when the platform lists none.
    pub formats: Vec<String>,
}

impl CheckLimits {
    /// The limits a preset publishes.
    pub fn from_preset(preset: &Preset) -> CheckLimits {
        CheckLimits {
            width: preset.width,
            min_width: preset.min_width,
            max_width: preset.max_width,
            recommended_width: preset.recommended_width,
            min_height: preset.min_height,
            max_height: preset.max_height,
            max_bytes: preset.max_bytes,
            max_total_bytes: preset.max_total_bytes,
            max_tiles: preset.max_tiles,
            formats: preset.formats.iter().map(|f| (*f).to_string()).collect(),
        }
    }

    /// Whether any field would actually be checked.
    pub fn is_empty(&self) -> bool {
        self.width.is_none()
            && self.min_width.is_none()
            && self.max_width.is_none()
            && self.min_height.is_none()
            && self.max_height.is_none()
            && self.max_bytes.is_none()
            && self.max_total_bytes.is_none()
            && self.max_tiles.is_none()
            && self.formats.is_empty()
    }

    /// A human list of what is being enforced, for the report header.
    pub fn describe(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(w) = self.width {
            out.push(format!("width = {w} px"));
        }
        if let Some(w) = self.min_width {
            out.push(format!("width >= {w} px"));
        }
        if let Some(w) = self.max_width {
            out.push(format!("width <= {w} px"));
        }
        if let Some(h) = self.min_height {
            out.push(format!("height >= {h} px"));
        }
        if let Some(h) = self.max_height {
            out.push(format!("height <= {h} px"));
        }
        if let Some(b) = self.max_bytes {
            out.push(format!("size <= {b} bytes"));
        }
        if let Some(b) = self.max_total_bytes {
            out.push(format!("episode total <= {b} bytes"));
        }
        if let Some(n) = self.max_tiles {
            out.push(format!("at most {n} images"));
        }
        if !self.formats.is_empty() {
            out.push(format!("format in {}", self.formats.join("/")));
        }
        if let Some(w) = self.recommended_width {
            out.push(format!("width {w} px recommended (not enforced)"));
        }
        out
    }
}

/// One inspected file and everything wrong with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckItem {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    /// The decoded format token (`png`, `jpg`, `webp`), when it was recognised.
    pub format: Option<String>,
    /// One sentence per violation. Empty means the file passes.
    pub problems: Vec<String>,
    /// Advisory remarks. These never affect the exit code.
    pub notes: Vec<String>,
}

impl CheckItem {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// Hold one already-sliced image against the limits.
pub fn check_one(
    path: &str,
    width: u32,
    height: u32,
    bytes: u64,
    format: Option<&str>,
    limits: &CheckLimits,
) -> CheckItem {
    let mut problems = Vec::new();
    let mut notes = Vec::new();

    if let Some(want) = limits.width {
        if width != want {
            problems.push(format!("width is {width} px, must be {want} px"));
        }
    }
    if let Some(min) = limits.min_width {
        if width < min {
            problems.push(format!("width is {width} px, minimum is {min} px"));
        }
    }
    if let Some(max) = limits.max_width {
        if width > max {
            problems.push(format!("width is {width} px, maximum is {max} px"));
        }
    }
    if let Some(want) = limits.recommended_width {
        if width != want {
            notes.push(format!(
                "width is {width} px; the platform recommends {want} px (not a requirement)"
            ));
        }
    }
    if let Some(min) = limits.min_height {
        if height < min {
            problems.push(format!("height is {height} px, minimum is {min} px"));
        }
    }
    if let Some(max) = limits.max_height {
        if height > max {
            problems.push(format!("height is {height} px, maximum is {max} px"));
        }
    }
    if let Some(max) = limits.max_bytes {
        if bytes > max {
            problems.push(format!(
                "file is {bytes} bytes, maximum is {max} bytes (over by {})",
                bytes - max
            ));
        }
    }
    if !limits.formats.is_empty() {
        match format {
            Some(f) if limits.formats.iter().any(|a| a == f) => {}
            Some(f) => problems.push(format!(
                "format is {f}, accepted formats are {}",
                limits.formats.join("/")
            )),
            None => problems.push(format!(
                "format was not recognised, accepted formats are {}",
                limits.formats.join("/")
            )),
        }
    }

    CheckItem {
        path: path.to_string(),
        width,
        height,
        bytes,
        format: format.map(str::to_string),
        problems,
        notes,
    }
}

/// The episode-level rules: too many images, or too many bytes in total.
///
/// Returns one sentence per violation; an empty vector means the set is fine
/// or the platform publishes no episode-level ceiling.
pub fn check_episode(count: usize, total_bytes: u64, limits: &CheckLimits) -> Vec<String> {
    let mut problems = Vec::new();
    if let Some(max) = limits.max_tiles {
        if count as u64 > u64::from(max) {
            problems.push(format!(
                "{count} images, but at most {max} are accepted per episode"
            ));
        }
    }
    if let Some(max) = limits.max_total_bytes {
        if total_bytes > max {
            problems.push(format!(
                "{total_bytes} bytes in total, but at most {max} are accepted per episode \
                 (over by {})",
                total_bytes - max
            ));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> CheckLimits {
        CheckLimits {
            width: Some(800),
            min_height: Some(600),
            max_height: Some(1280),
            max_bytes: Some(2_000_000),
            max_total_bytes: Some(5_000_000),
            max_tiles: Some(3),
            formats: vec!["png".to_string(), "jpg".to_string()],
            ..CheckLimits::default()
        }
    }

    #[test]
    fn a_compliant_file_passes() {
        let item = check_one("a.png", 800, 1200, 900_000, Some("png"), &limits());
        assert!(item.ok(), "{:?}", item.problems);
    }

    #[test]
    fn each_rule_fires_on_its_own() {
        let too_wide = check_one("a.png", 900, 1200, 100, Some("png"), &limits());
        assert_eq!(too_wide.problems.len(), 1);
        assert!(too_wide.problems[0].contains("width"));

        let too_tall = check_one("a.png", 800, 5000, 100, Some("png"), &limits());
        assert!(too_tall.problems[0].contains("maximum is 1280"));

        let too_short = check_one("a.png", 800, 10, 100, Some("png"), &limits());
        assert!(too_short.problems[0].contains("minimum is 600"));

        let too_big = check_one("a.png", 800, 1200, 3_000_000, Some("png"), &limits());
        assert!(too_big.problems[0].contains("over by 1000000"));

        let wrong_format = check_one("a.gif", 800, 1200, 100, Some("gif"), &limits());
        assert!(wrong_format.problems[0].contains("accepted formats are png/jpg"));

        let unknown = check_one("a.bin", 800, 1200, 100, None, &limits());
        assert!(unknown.problems[0].contains("not recognised"));
    }

    #[test]
    fn violations_accumulate() {
        let item = check_one("a.gif", 900, 5000, 9_000_000, Some("gif"), &limits());
        assert_eq!(item.problems.len(), 4);
        assert!(!item.ok());
    }

    #[test]
    fn unpublished_limits_are_not_invented() {
        let empty = CheckLimits::default();
        assert!(empty.is_empty());
        assert!(empty.describe().is_empty());
        let item = check_one("a.png", 1, 1, 1, Some("gif"), &empty);
        assert!(item.ok());
    }

    #[test]
    fn the_episode_level_rules_are_checked_too() {
        assert!(check_episode(3, 1_000, &limits()).is_empty());
        assert!(check_episode(4, 1_000, &limits())[0].contains("at most 3"));
        assert!(check_episode(1, 6_000_000, &limits())[0].contains("over by 1000000"));
        assert_eq!(check_episode(4, 6_000_000, &limits()).len(), 2);
        assert!(check_episode(9999, u64::MAX, &CheckLimits::default()).is_empty());
    }

    #[test]
    fn a_width_range_is_enforced_but_a_recommendation_is_not() {
        let ranged = CheckLimits {
            min_width: Some(400),
            max_width: Some(10_000),
            recommended_width: Some(800),
            ..CheckLimits::default()
        };
        // Inside the range but off the recommendation: a note, not a failure.
        let item = check_one("a.png", 720, 100, 100, Some("png"), &ranged);
        assert!(item.ok());
        assert_eq!(item.notes.len(), 1);
        assert!(item.notes[0].contains("recommends 800"));

        // On the recommendation: nothing to say.
        let item = check_one("a.png", 800, 100, 100, Some("png"), &ranged);
        assert!(item.ok() && item.notes.is_empty());

        // Outside the range: a real failure at each end.
        assert!(
            check_one("a.png", 399, 100, 100, Some("png"), &ranged).problems[0]
                .contains("minimum is 400")
        );
        assert!(
            check_one("a.png", 10_001, 100, 100, Some("png"), &ranged).problems[0]
                .contains("maximum is 10000")
        );
    }

    #[test]
    fn limits_come_off_a_preset_verbatim() {
        let preset = crate::presets::find_preset("webtoon-canvas").expect("preset");
        let limits = CheckLimits::from_preset(preset);
        assert_eq!(limits.width, preset.width);
        assert_eq!(limits.max_width, preset.max_width);
        assert_eq!(limits.max_height, preset.max_height);
        assert_eq!(limits.max_bytes, preset.max_bytes);
        assert!(!limits.is_empty());
    }
}
