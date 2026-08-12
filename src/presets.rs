//! Platform presets.
//!
//! This table is data, and it follows the same discipline as the org's
//! sunsetscan catalog: a number is only in here if it was read off a page the
//! platform itself publishes, and every entry records that page and the date it
//! was read. Where a platform does not publish a figure, the field is `None`
//! rather than a guess, and the flag has to supply it. Where a platform
//! publishes nothing usable at all, there is no preset -- see the README for
//! the list of platforms deliberately left out and why.
//!
//! Widths get three separate fields on purpose, because the platforms mean
//! three different things by "width": a hard exact width, a permitted range,
//! and a recommendation. Collapsing them would turn a recommendation into a
//! validation failure, which is exactly the kind of invented rule this table
//! exists to avoid.
//!
//! Nothing here is required to use stripcut: `--max-height`, `--min-height`,
//! `--target-height` and `--max-bytes` cover every case a preset would.

/// One platform's published upload limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    /// Value passed to `--preset`.
    pub name: &'static str,
    /// Human name of the platform.
    pub platform: &'static str,
    /// A width the platform states as required, exactly. Checked as a rule.
    pub width: Option<u32>,
    /// Smallest permitted width, when the platform states a range.
    pub min_width: Option<u32>,
    /// Largest permitted width, when the platform states one.
    pub max_width: Option<u32>,
    /// A width the platform only recommends. Reported, never enforced.
    pub recommended_width: Option<u32>,
    /// Smallest accepted image height, when the platform states one.
    pub min_height: Option<u32>,
    /// Largest accepted image height, when the platform states one.
    pub max_height: Option<u32>,
    /// Largest accepted file size per image in bytes, when stated.
    pub max_bytes: Option<u64>,
    /// Largest accepted total size for one episode in bytes, when stated.
    pub max_total_bytes: Option<u64>,
    /// Largest number of images per episode, when stated.
    pub max_tiles: Option<u32>,
    /// Accepted upload formats, when the source page lists them.
    pub formats: &'static [&'static str],
    /// What the source page actually says, including what it does not say.
    pub notes: &'static str,
    /// The platform-owned page the numbers came from.
    pub source_url: &'static str,
    /// The date that page was read, ISO 8601.
    pub verified_on: &'static str,
}

impl Preset {
    /// Whether this preset alone is enough to plan a slice.
    ///
    /// Only a published maximum height can drive the planner. The rest of the
    /// presets are still useful for `--check`; they just need `--max-height`.
    pub fn can_plan(&self) -> bool {
        self.max_height.is_some()
    }
}

/// Every preset stripcut ships with, in a stable order.
pub static PRESETS: &[Preset] = &[
    Preset {
        name: "webtoon-canvas",
        platform: "WEBTOON CANVAS",
        width: None,
        min_width: None,
        max_width: Some(800),
        recommended_width: None,
        min_height: None,
        max_height: Some(1280),
        max_bytes: None,
        max_total_bytes: None,
        max_tiles: None,
        formats: &[],
        notes:
            "The CANVAS notice states one number: \"the maximum image dimensions, 800x1280px\", \
                and says the uploader automatically optimizes and slices anything larger. So 800 \
                is a maximum width, not a required canvas width, and it is recorded that way. \
                Slicing locally is how you keep the cut points instead of letting the uploader \
                choose them. The notice is dated 2021-05-18 and states no per-file byte limit, no \
                image count and no format list, so none of those are encoded here.",
        source_url: "https://www.webtoons.com/en/notice/detail?noticeNo=1766",
        verified_on: "2026-08-12",
    },
    Preset {
        name: "line-manga-indies",
        platform: "LINE マンガ インディーズ (vertical / 縦読み)",
        width: None,
        min_width: Some(400),
        max_width: Some(10_000),
        recommended_width: Some(800),
        min_height: Some(600),
        max_height: Some(10_000),
        max_bytes: Some(2_000_000),
        max_total_bytes: None,
        max_tiles: Some(1000),
        formats: &["png", "jpg"],
        notes: "The guideline page gives the vertical-scroll body image as: recommended width \
                800 px; required 400-10000 px wide and 600-10000 px tall; 2 MB per image; up to \
                1000 images; PNG or JPG. 800 px is 推奨 (recommended), so it is reported and not \
                enforced. 2 MB is encoded as 2,000,000 bytes, the stricter of the two readings. \
                10000 px is the platform ceiling, not a comfortable reading length -- pass \
                --max-height or --target-height for the tile height you actually want.",
        source_url: "https://manga.line.me/indies/guideline",
        verified_on: "2026-08-12",
    },
    Preset {
        name: "comico",
        platform: "comico (マンガ / vertical)",
        width: None,
        min_width: None,
        max_width: None,
        recommended_width: Some(800),
        min_height: None,
        max_height: Some(10_000),
        max_bytes: Some(2_000_000),
        max_total_bytes: Some(20_000_000),
        max_tiles: Some(100),
        formats: &["jpg", "png"],
        notes: "comico's manga production guideline gives the manuscript as: JPG or PNG, \
                recommended 800 px wide, up to 10,000 px tall, up to 2 MB per image and 20 MB per \
                episode, up to 100 images. The page adds that an over-wide image is resized \
                automatically, so no maximum width is encoded. The page does not itself say \
                whether it covers comico チャレンジ or 公式 works; the numbers are the ones it \
                publishes for manga manuscripts either way. The often-quoted 690 px figure is \
                stale -- this page says 800.",
        source_url: "https://resources.comico.io/html/legal/manga_guideline.html",
        verified_on: "2026-08-12",
    },
    Preset {
        name: "tapas",
        platform: "Tapas",
        width: Some(940),
        min_width: None,
        max_width: None,
        recommended_width: None,
        min_height: None,
        max_height: None,
        max_bytes: Some(10_000_000),
        max_total_bytes: None,
        max_tiles: None,
        formats: &["png", "jpg", "gif"],
        notes: "Tapas states episode page size as \"940 px wide, no height limit\", formats PNG, \
                JPG or GIF, and \"Size: Under 10MB\". (The 1000 px height cap on the same page \
                applies to GIF only, which stripcut does not write.) Because no maximum height is \
                published, --max-height is required with this preset. The help article blocks \
                non-browser clients, so open the URL in a browser to re-read it.",
        source_url: "https://help.tapas.io/hc/en-us/articles/360052100813-File-Size-Overview",
        verified_on: "2026-08-12",
    },
    Preset {
        name: "mechacomic-creators",
        platform: "めちゃコミック クリエイターズ (vertical / 縦読み)",
        width: None,
        min_width: None,
        max_width: None,
        recommended_width: Some(300),
        min_height: None,
        max_height: None,
        max_bytes: Some(8_000_000),
        max_total_bytes: Some(150_000_000),
        max_tiles: Some(60),
        formats: &["jpg", "png", "gif"],
        notes: "The submission manual gives, for a vertical-scroll manuscript: 300 px wide or \
                more recommended (1080 px is the recommendation for the non-vertical format), up \
                to 60 pages per episode, 8 MB per page and 150 MB per episode, JPG/PNG/GIF. The \
                300 px figure is 推奨, so it is reported and not enforced. No fixed width and no \
                maximum height are published, so --max-height is required with this preset.",
        source_url: "https://creators.mechacomic.jp/manual",
        verified_on: "2026-08-12",
    },
];

/// Look a preset up by name.
pub fn find_preset(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name == name)
}

/// Every preset name, for the error message when a name is not one of them.
pub fn preset_names() -> Vec<&'static str> {
    PRESETS.iter().map(|p| p.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_cites_a_source_and_a_date() {
        for p in PRESETS {
            assert!(
                p.source_url.starts_with("https://"),
                "{} has no https source",
                p.name
            );
            assert_eq!(p.verified_on.len(), 10, "{} date is not ISO", p.name);
            assert!(p.verified_on.starts_with("20"), "{}", p.name);
            assert!(!p.notes.is_empty(), "{} has no notes", p.name);
            assert!(!p.platform.is_empty(), "{} has no platform", p.name);
        }
    }

    #[test]
    fn names_are_unique_and_lookup_works() {
        let mut names = preset_names();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
        assert_eq!(find_preset("webtoon-canvas").unwrap().max_width, Some(800));
        assert!(find_preset("nope").is_none());
    }

    #[test]
    fn published_ranges_are_self_consistent() {
        for p in PRESETS {
            if let (Some(lo), Some(hi)) = (p.min_height, p.max_height) {
                assert!(lo <= hi, "{} has min > max height", p.name);
            }
            if let (Some(lo), Some(hi)) = (p.min_width, p.max_width) {
                assert!(lo <= hi, "{} has min > max width", p.name);
            }
            if let (Some(per), Some(total)) = (p.max_bytes, p.max_total_bytes) {
                assert!(
                    per <= total,
                    "{} has a per-image cap over its total",
                    p.name
                );
            }
            // A required exact width and a permitted range are alternatives;
            // no source states both, and mixing them would be a contradiction.
            assert!(
                p.width.is_none() || (p.min_width.is_none() && p.max_width.is_none()),
                "{} states both an exact width and a width range",
                p.name
            );
        }
    }

    #[test]
    fn only_presets_with_a_published_max_height_can_plan() {
        assert!(find_preset("webtoon-canvas").unwrap().can_plan());
        assert!(find_preset("line-manga-indies").unwrap().can_plan());
        assert!(find_preset("comico").unwrap().can_plan());
        assert!(!find_preset("tapas").unwrap().can_plan());
        assert!(!find_preset("mechacomic-creators").unwrap().can_plan());
    }

    #[test]
    fn format_lists_are_lowercase_tokens() {
        for p in PRESETS {
            for f in p.formats {
                assert_eq!(*f, f.to_ascii_lowercase(), "{} lists {f}", p.name);
                assert!(!f.starts_with('.'), "{} lists {f} with a dot", p.name);
            }
        }
    }
}
