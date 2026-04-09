//! GitWell configuration.
//!
//! Supports an optional `.gitwell.toml` in the scanned directory or
//! `~/.config/gitwell/config.toml`. Only a flat subset of TOML is supported:
//!
//! ```toml
//! stale_days = 30
//! dormant_months = 6
//! session_window_hours = 48
//! ignore_repos = ["node_modules", ".cache"]
//! ignore_branches = ["dependabot/*"]
//! ```
//!
//! Unknown keys are silently ignored so future versions can add fields
//! without breaking older configs.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Config {
    pub stale_days: u64,
    pub dormant_months: u64,
    pub session_window_hours: u64,
    pub ignore_repos: Vec<String>,
    pub ignore_branches: Vec<String>,
    pub loaded_from: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            stale_days: 30,
            dormant_months: 6,
            session_window_hours: 48,
            ignore_repos: Vec::new(),
            ignore_branches: Vec::new(),
            loaded_from: None,
        }
    }
}

impl Config {
    /// Seconds a branch can go untouched before it's considered stale.
    pub fn stale_secs(&self) -> u64 {
        self.stale_days.saturating_mul(24 * 60 * 60)
    }

    /// Seconds of inactivity before a repo is considered dormant.
    /// Uses the common "30 days per month" approximation.
    pub fn dormant_secs(&self) -> u64 {
        self.dormant_months.saturating_mul(30 * 24 * 60 * 60)
    }

    /// Session window in seconds (for clustering).
    pub fn session_window_secs(&self) -> u64 {
        self.session_window_hours.saturating_mul(60 * 60)
    }
}

/// Load config, trying (in order): `<scan_path>/.gitwell.toml`,
/// then `~/.config/gitwell/config.toml`. Falls back to defaults.
/// Parse errors are printed to stderr but do not abort — we return defaults.
pub fn load(scan_path: &Path) -> Config {
    let mut config = Config::default();

    let found = find_config(scan_path);
    if let Some(path) = found {
        match fs::read_to_string(&path) {
            Ok(content) => {
                if let Err(e) = parse_into(&content, &mut config) {
                    eprintln!("gitwell: {}: {}", path.display(), e);
                }
                config.loaded_from = Some(path);
            }
            Err(e) => {
                eprintln!("gitwell: could not read {}: {}", path.display(), e);
            }
        }
    }

    config
}

fn find_config(scan_path: &Path) -> Option<PathBuf> {
    let local = scan_path.join(".gitwell.toml");
    if local.is_file() {
        return Some(local);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let global = PathBuf::from(home).join(".config/gitwell/config.toml");
        if global.is_file() {
            return Some(global);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Hand-written TOML parser (flat key-value + simple string arrays only).
// ---------------------------------------------------------------------------

fn parse_into(content: &str, config: &mut Config) -> Result<(), String> {
    for (i, raw) in content.lines().enumerate() {
        let line_num = i + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        // We don't support [sections] — skip them so a stray header doesn't
        // error out, but warn.
        if line.starts_with('[') {
            eprintln!(
                "gitwell: line {}: TOML sections are not supported, skipping",
                line_num
            );
            continue;
        }

        let eq = line
            .find('=')
            .ok_or_else(|| format!("line {}: expected `=`", line_num))?;
        let key = line[..eq].trim();
        let value = line[eq + 1..].trim();

        match key {
            "stale_days" => {
                config.stale_days = parse_u64(value)
                    .ok_or_else(|| format!("line {}: stale_days must be an integer", line_num))?;
            }
            "dormant_months" => {
                config.dormant_months = parse_u64(value).ok_or_else(|| {
                    format!("line {}: dormant_months must be an integer", line_num)
                })?;
            }
            "session_window_hours" => {
                config.session_window_hours = parse_u64(value).ok_or_else(|| {
                    format!("line {}: session_window_hours must be an integer", line_num)
                })?;
            }
            "ignore_repos" => {
                config.ignore_repos = parse_string_array(value).ok_or_else(|| {
                    format!("line {}: ignore_repos must be a string array", line_num)
                })?;
            }
            "ignore_branches" => {
                config.ignore_branches = parse_string_array(value).ok_or_else(|| {
                    format!("line {}: ignore_branches must be a string array", line_num)
                })?;
            }
            _ => {
                // Unknown key — tolerate for forward compatibility.
            }
        }
    }
    Ok(())
}

/// Remove `#` comments from a line, respecting `"..."` strings.
fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    let mut escape = false;
    for (i, c) in line.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if in_str {
            match c {
                '\\' => escape = true,
                '"' => in_str = false,
                _ => {}
            }
        } else {
            match c {
                '"' => in_str = true,
                '#' => return &line[..i],
                _ => {}
            }
        }
    }
    line
}

fn parse_u64(s: &str) -> Option<u64> {
    s.trim().parse::<u64>().ok()
}

/// Parse `["foo", "bar"]` style arrays. Escapes `\\` and `\"` in strings.
fn parse_string_array(s: &str) -> Option<Vec<String>> {
    let s = s.trim();
    if !s.starts_with('[') || !s.ends_with(']') {
        return None;
    }
    let inner = &s[1..s.len() - 1];

    let mut out = Vec::new();
    let mut in_str = false;
    let mut current = String::new();
    let mut escape = false;
    let mut expecting_comma = false;

    for c in inner.chars() {
        if escape {
            current.push(c);
            escape = false;
            continue;
        }
        if in_str {
            match c {
                '\\' => escape = true,
                '"' => {
                    in_str = false;
                    out.push(std::mem::take(&mut current));
                    expecting_comma = true;
                }
                _ => current.push(c),
            }
        } else if c.is_whitespace() {
            continue;
        } else if c == ',' {
            if !expecting_comma {
                return None;
            }
            expecting_comma = false;
        } else if c == '"' {
            if expecting_comma {
                return None;
            }
            in_str = true;
        } else {
            return None;
        }
    }

    if in_str {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_known_keys() {
        let src = r#"
            # a comment
            stale_days = 14
            dormant_months = 3
            session_window_hours = 72
            ignore_repos = ["node_modules", ".cache"]
            ignore_branches = ["dependabot/*", "renovate/*"]
        "#;
        let mut c = Config::default();
        parse_into(src, &mut c).unwrap();
        assert_eq!(c.stale_days, 14);
        assert_eq!(c.dormant_months, 3);
        assert_eq!(c.session_window_hours, 72);
        assert_eq!(c.ignore_repos, vec!["node_modules", ".cache"]);
        assert_eq!(c.ignore_branches, vec!["dependabot/*", "renovate/*"]);
    }

    #[test]
    fn strips_trailing_comment() {
        assert_eq!(strip_comment("stale_days = 14  # comment"), "stale_days = 14  ");
    }

    #[test]
    fn preserves_hash_inside_string() {
        let input = r##"k = "#hash""##;
        assert_eq!(strip_comment(input), input);
    }

    #[test]
    fn rejects_malformed_array() {
        assert!(parse_string_array("[foo]").is_none());
        assert!(parse_string_array(r#"["a" "b"]"#).is_none());
    }
}
