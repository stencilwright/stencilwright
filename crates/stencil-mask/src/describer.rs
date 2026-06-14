//! Describer pipeline: pure functions that label a value's shape.
//! Output is the `<description>` portion of `[$<id> <description>]`.

use regex::Regex;
use std::sync::OnceLock;

/// Run the describer set against a value. Returns the most-specific
/// matching description, or `"text"` if nothing matched (used for
/// values caught only by selector blacklist).
pub fn describe(value: &str) -> String {
    for d in DESCRIBERS.get_or_init(build_describers) {
        if d.regex.is_match(value) {
            return (d.label)(value);
        }
    }
    // Pure-digits fallback: "<N>-digit numeric"
    if !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()) && value.len() >= 8 {
        return format!("{}-digit numeric", value.len());
    }
    "text".to_string()
}

struct Describer {
    regex: Regex,
    label: fn(&str) -> String,
}

static DESCRIBERS: OnceLock<Vec<Describer>> = OnceLock::new();

fn build_describers() -> Vec<Describer> {
    // Order matters: more specific first.
    vec![
        Describer { regex: Regex::new(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$").unwrap(), label: |_| "uuid".into() },
        Describer { regex: Regex::new(r"^[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}$").unwrap(), label: |_| "email".into() },
        Describer { regex: Regex::new(r"^(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)(?:\.(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)){3}$").unwrap(), label: |_| "ipv4".into() },
        Describer { regex: Regex::new(r"^(?:[0-9A-Fa-f]{1,4}:){2,7}[0-9A-Fa-f]{1,4}$").unwrap(), label: |_| "ipv6".into() },
        Describer { regex: Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}").unwrap(), label: |_| "datetime".into() },
        Describer { regex: Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap(), label: |_| "date".into() },
        Describer { regex: Regex::new(r"^\d{1,2}:\d{2}(?::\d{2})?(?:\s?[APap][Mm])?$").unwrap(), label: |_| "time".into() },
        Describer { regex: Regex::new(r"^-?\$[0-9][0-9,]*(?:\.[0-9]+)?$").unwrap(), label: |_| "currency".into() },
        Describer { regex: Regex::new(r"^-?\d+(?:\.\d+)?%$").unwrap(), label: |_| "percent".into() },
        Describer { regex: Regex::new(r"^[0-9A-Fa-f]{16,}$").unwrap(), label: |_| "hex".into() },
        Describer { regex: Regex::new(r"^\+?\d{1,2}?[\s.\-]?\(?\d{3}\)?[\s.\-]?\d{3}[\s.\-]?\d{4}$").unwrap(), label: |_| "phone".into() },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn email() {
        assert_eq!(describe("a@b.co"), "email");
    }
    #[test]
    fn ipv4() {
        assert_eq!(describe("198.51.100.7"), "ipv4");
    }
    #[test]
    fn iso_date() {
        assert_eq!(describe("2026-04-27"), "date");
    }
    #[test]
    fn currency() {
        assert_eq!(describe("$1,234.56"), "currency");
    }
    #[test]
    fn long_digits() {
        assert_eq!(describe("12345678"), "8-digit numeric");
    }
}
