use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", t)
    }
}

// ---------------------------------------------------------------------------
// Glob matching (supports `*` wildcards only).
// ---------------------------------------------------------------------------

/// Match `s` against a simple glob pattern. Supports `*` as wildcard
/// (matching any run of characters, including slashes). No `?`, no
/// character classes — we intentionally keep this tiny.
pub fn glob_match(pattern: &str, s: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == s;
    }

    let mut rest = s;

    // First part must match at the start.
    if !parts[0].is_empty() {
        if !rest.starts_with(parts[0]) {
            return false;
        }
        rest = &rest[parts[0].len()..];
    }

    // Middle parts must appear in order.
    for part in &parts[1..parts.len() - 1] {
        if part.is_empty() {
            continue;
        }
        if let Some(pos) = rest.find(part) {
            rest = &rest[pos + part.len()..];
        } else {
            return false;
        }
    }

    // Last part must match at the end.
    let last = parts[parts.len() - 1];
    if !last.is_empty() {
        if !rest.ends_with(last) {
            return false;
        }
        // Ensure the last part doesn't overlap what we already consumed.
        if rest.len() < last.len() {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Date math — Howard Hinnant's civil_from_days, no-dep version.
// ---------------------------------------------------------------------------

pub const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Convert days since Unix epoch (1970-01-01) to `(year, month, day)`.
/// Uses Howard Hinnant's civil_from_days algorithm.
pub fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = (y + if m <= 2 { 1 } else { 0 }) as i32;
    (year, m, d)
}

/// Convert a unix timestamp (seconds) to `(year, month, day)`.
pub fn ymd_from_unix(ts: u64) -> (i32, u32, u32) {
    civil_from_days((ts / 86400) as i64)
}

/// Format a unix timestamp as e.g. "March 2025".
pub fn format_month_year(ts: u64) -> String {
    let (y, m, _) = ymd_from_unix(ts);
    format!("{} {}", MONTHS[(m - 1) as usize], y)
}

/// Format a unix timestamp as ISO 8601, e.g. "2026-04-09T10:30:00Z".
pub fn format_iso8601(ts: u64) -> String {
    let (y, mo, d) = ymd_from_unix(ts);
    let tod = ts % 86_400;
    let h = tod / 3600;
    let mi = (tod % 3600) / 60;
    let s = tod % 60;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, mo, d, h, mi, s
    )
}

/// Format a timestamp range as a human-readable period.
pub fn format_period(start: u64, end: u64) -> String {
    let (sy, sm, _) = ymd_from_unix(start);
    let (ey, em, _) = ymd_from_unix(end);
    if sy == ey && sm == em {
        format!("{} {}", MONTHS[(sm - 1) as usize], sy)
    } else if sy == ey {
        format!(
            "{}–{} {}",
            MONTHS[(sm - 1) as usize],
            MONTHS[(em - 1) as usize],
            sy
        )
    } else {
        format!(
            "{} {} – {} {}",
            MONTHS[(sm - 1) as usize],
            sy,
            MONTHS[(em - 1) as usize],
            ey
        )
    }
}

/// Format a timestamp as "N days/months/years ago" relative to `now`.
pub fn format_age_ago(ts: u64, now: u64) -> String {
    let secs = now.saturating_sub(ts);
    let days = secs / 86_400;
    if days < 1 {
        "today".to_string()
    } else if days == 1 {
        "yesterday".to_string()
    } else if days < 30 {
        format!("{} days ago", days)
    } else if days < 365 {
        let m = days / 30;
        format!("{} month{} ago", m, if m == 1 { "" } else { "s" })
    } else {
        let y = days / 365;
        format!("{} year{} ago", y, if y == 1 { "" } else { "s" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_exact() {
        assert!(glob_match("main", "main"));
        assert!(!glob_match("main", "master"));
    }

    #[test]
    fn glob_prefix() {
        assert!(glob_match("dependabot/*", "dependabot/npm/lodash"));
        assert!(!glob_match("dependabot/*", "renovate/npm/lodash"));
    }

    #[test]
    fn glob_suffix() {
        assert!(glob_match("*.cache", "build.cache"));
        assert!(!glob_match("*.cache", "build.log"));
    }

    #[test]
    fn glob_middle() {
        assert!(glob_match("feat/*/wip", "feat/auth/wip"));
        assert!(!glob_match("feat/*/wip", "feat/auth/done"));
    }

    #[test]
    fn glob_double_star() {
        // Since we only support `*` (no **), "*/*" is still useful.
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn civil_epoch() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn civil_known_date() {
        // 2025-03-15 is 20162 days after epoch.
        let days = 20162;
        assert_eq!(civil_from_days(days), (2025, 3, 15));
    }
}
