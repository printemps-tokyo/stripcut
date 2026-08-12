//! The small amount of JSON writing stripcut needs.
//!
//! stripcut only ever *writes* JSON -- it has no catalog file to read -- so
//! this is a handful of helpers rather than `serde` plus `serde_json` in the
//! dependency tree. The document shapes are fixed and are spelled out in the
//! README.

use std::fmt::Write as _;

/// Escape a string and wrap it in double quotes, ready to splice into output.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            // Control characters have no short escape; the \u form is required.
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A string field, or `null` when it is absent.
pub fn opt_str(value: Option<&str>) -> String {
    match value {
        Some(s) => quote(s),
        None => "null".to_string(),
    }
}

/// A numeric field, or `null` when it is absent.
pub fn opt_num<T: std::fmt::Display>(value: Option<T>) -> String {
    match value {
        Some(n) => n.to_string(),
        None => "null".to_string(),
    }
}

/// A JSON array of strings on one line: `["a", "b"]`.
pub fn str_array<S: AsRef<str>>(items: &[S]) -> String {
    let parts: Vec<String> = items.iter().map(|s| quote(s.as_ref())).collect();
    format!("[{}]", parts.join(", "))
}

/// `",\n"` between array elements and `"\n"` after the last one.
pub fn sep(index: usize, len: usize) -> &'static str {
    if index + 1 < len {
        ",\n"
    } else {
        "\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_escapes_the_json_specials() {
        assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(quote("a\nb\tc\r"), r#""a\nb\tc\r""#);
        assert_eq!(quote("\u{1}"), "\"\\u0001\"");
        assert_eq!(quote("日本語 ok"), "\"日本語 ok\"");
    }

    #[test]
    fn optional_fields_render_null() {
        assert_eq!(opt_str(None), "null");
        assert_eq!(opt_str(Some("x")), "\"x\"");
        assert_eq!(opt_num(None::<u32>), "null");
        assert_eq!(opt_num(Some(7u32)), "7");
    }

    #[test]
    fn arrays_and_separators() {
        assert_eq!(str_array::<&str>(&[]), "[]");
        assert_eq!(str_array(&["png", "jpg"]), "[\"png\", \"jpg\"]");
        assert_eq!(sep(0, 2), ",\n");
        assert_eq!(sep(1, 2), "\n");
    }
}
