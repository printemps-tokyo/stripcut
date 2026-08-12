//! Hex colour parsing and formatting, matching the org's other image tools.

use image::Rgba;

/// Parse a hex colour into RGBA, with or without a leading `#`.
///
/// Accepted digit counts:
/// - 3 (`abc`) and 6 (`aabbcc`): opaque RGB
/// - 4 (`abcd`) and 8 (`aabbccdd`): RGBA with an explicit alpha channel
///
/// Short forms expand each digit (`f00a` -> `ff0000aa`). `0000` is the way to
/// say "the page background is fully transparent".
pub fn parse_color(input: &str) -> Result<Rgba<u8>, String> {
    let trimmed = input.trim();
    let h = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if h.is_empty() || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("invalid hex colour: {input:?}"));
    }
    let expanded = match h.len() {
        3 | 4 => h.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 | 8 => h.to_string(),
        _ => return Err(format!("invalid hex colour: {input:?}")),
    };
    // Default to fully opaque when no alpha digits were given.
    let rgba = if expanded.len() == 6 {
        format!("{expanded}ff")
    } else {
        expanded
    };
    let val =
        u32::from_str_radix(&rgba, 16).map_err(|_| format!("invalid hex colour: {input:?}"))?;
    Ok(Rgba([
        ((val >> 24) & 0xff) as u8,
        ((val >> 16) & 0xff) as u8,
        ((val >> 8) & 0xff) as u8,
        (val & 0xff) as u8,
    ]))
}

/// Render a colour as `#rrggbbaa`, the form `--bg` accepts back.
pub fn format_color(c: Rgba<u8>) -> String {
    format!("#{:02x}{:02x}{:02x}{:02x}", c.0[0], c.0[1], c.0[2], c.0[3])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_accepted_digit_counts() {
        assert_eq!(parse_color("#ffffff").unwrap(), Rgba([255, 255, 255, 255]));
        assert_eq!(parse_color("fff").unwrap(), Rgba([255, 255, 255, 255]));
        assert_eq!(parse_color("0000").unwrap(), Rgba([0, 0, 0, 0]));
        assert_eq!(
            parse_color("aabbccdd").unwrap(),
            Rgba([0xaa, 0xbb, 0xcc, 0xdd])
        );
    }

    #[test]
    fn rejects_everything_else() {
        for bad in ["", "#", "xyz", "12345", "1234567", "+abc12", "gggggg"] {
            assert!(parse_color(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn formats_round_trip() {
        let c = parse_color("#12345678").unwrap();
        assert_eq!(format_color(c), "#12345678");
        assert_eq!(parse_color(&format_color(c)).unwrap(), c);
    }
}
