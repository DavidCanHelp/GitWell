//! Track scan totals over time in `.gitwell/history.json`.
//!
//! Each successful `gitwell report` run appends an entry:
//!
//! ```json
//! [
//!   {"date": "2026-04-07", "timestamp": 1712563200, "repos": 28, "findings": 215, "sessions": 30},
//!   {"date": "2026-04-09", "timestamp": 1712736000, "repos": 28, "findings": 213, "sessions": 29}
//! ]
//! ```
//!
//! Reports use the most recent previous entry to compute a "since last
//! scan" delta, e.g.:
//!
//! > _2 days ago: 215 → 213 findings (−2), 30 → 29 sessions (−1)_

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::json::{self, Value};
use crate::util;

pub const HISTORY_FILE: &str = "history.json";

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub date: String,
    pub timestamp: u64,
    pub repos: usize,
    pub findings: usize,
    pub sessions: usize,
}

#[derive(Debug, Clone, Default)]
pub struct History {
    pub entries: Vec<HistoryEntry>,
}

impl History {
    pub fn path(scan_path: &Path) -> PathBuf {
        scan_path.join(".gitwell").join(HISTORY_FILE)
    }

    pub fn load(scan_path: &Path) -> io::Result<Self> {
        let p = Self::path(scan_path);
        if !p.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&p)?;
        let v = json::parse(&content)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Self::from_json(&v).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    pub fn save(&self, scan_path: &Path) -> io::Result<()> {
        let p = Self::path(scan_path);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&p, json::to_pretty_string(&self.to_json()))?;
        Ok(())
    }

    /// Append a new entry and keep history sorted by timestamp.
    pub fn append(&mut self, entry: HistoryEntry) {
        self.entries.push(entry);
        self.entries.sort_by_key(|e| e.timestamp);
    }

    /// Most recent entry **before** `timestamp`.
    pub fn previous_before(&self, timestamp: u64) -> Option<&HistoryEntry> {
        self.entries.iter().filter(|e| e.timestamp < timestamp).last()
    }

    fn from_json(v: &Value) -> Result<Self, String> {
        let arr = v
            .as_array()
            .ok_or_else(|| "history.json must be an array".to_string())?;
        let mut entries = Vec::with_capacity(arr.len());
        for item in arr {
            let date = item.get("date").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let timestamp = item
                .get("timestamp")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as u64;
            let repos = item.get("repos").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
            let findings = item.get("findings").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
            let sessions = item.get("sessions").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
            entries.push(HistoryEntry {
                date,
                timestamp,
                repos,
                findings,
                sessions,
            });
        }
        Ok(History { entries })
    }

    fn to_json(&self) -> Value {
        let items: Vec<Value> = self
            .entries
            .iter()
            .map(|e| {
                Value::Object(vec![
                    ("date".into(), Value::String(e.date.clone())),
                    ("timestamp".into(), Value::Int(e.timestamp as i64)),
                    ("repos".into(), Value::Int(e.repos as i64)),
                    ("findings".into(), Value::Int(e.findings as i64)),
                    ("sessions".into(), Value::Int(e.sessions as i64)),
                ])
            })
            .collect();
        Value::Array(items)
    }
}

// ---------------------------------------------------------------------------
// Delta computation
// ---------------------------------------------------------------------------

pub struct Delta<'a> {
    pub previous: &'a HistoryEntry,
    pub current: &'a HistoryEntry,
}

impl<'a> Delta<'a> {
    /// Age of the previous entry relative to the current one.
    pub fn age_phrase(&self) -> String {
        let delta_secs = self.current.timestamp.saturating_sub(self.previous.timestamp);
        let days = delta_secs / 86_400;
        match days {
            0 => "earlier today".to_string(),
            1 => "yesterday".to_string(),
            n if n < 7 => format!("{} days ago", n),
            n if n < 60 => format!("{} days ago", n),
            n if n < 365 => format!("{} months ago", n / 30),
            n => format!("{} years ago", n / 365),
        }
    }

    /// Formatted "X → Y (±Δ)" for a numeric field.
    pub fn fmt_pair(label: &str, old: usize, new: usize) -> String {
        let diff = new as i64 - old as i64;
        let sign = if diff > 0 { "+" } else { "" };
        format!("{} → {} {} ({}{})", old, new, label, sign, diff)
    }

    /// Full one-line summary, ready to embed in a markdown report.
    pub fn summary_line(&self) -> String {
        let findings = Self::fmt_pair("findings", self.previous.findings, self.current.findings);
        let sessions = Self::fmt_pair("sessions", self.previous.sessions, self.current.sessions);
        format!("{}: {}, {}", self.age_phrase(), findings, sessions)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn current_entry(repos: usize, findings: usize, sessions: usize) -> HistoryEntry {
    let ts = util::now_unix();
    let iso = util::format_iso8601(ts);
    // YYYY-MM-DD portion of the ISO string.
    let date = iso[..10].to_string();
    HistoryEntry {
        date,
        timestamp: ts,
        repos,
        findings,
        sessions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let mut h = History::default();
        h.append(HistoryEntry {
            date: "2026-04-07".into(),
            timestamp: 1_712_563_200,
            repos: 28,
            findings: 215,
            sessions: 30,
        });
        h.append(HistoryEntry {
            date: "2026-04-09".into(),
            timestamp: 1_712_736_000,
            repos: 28,
            findings: 213,
            sessions: 29,
        });
        let s = json::to_pretty_string(&h.to_json());
        let parsed = json::parse(&s).unwrap();
        let reloaded = History::from_json(&parsed).unwrap();
        assert_eq!(reloaded.entries.len(), 2);
        assert_eq!(reloaded.entries[1].findings, 213);
    }

    #[test]
    fn previous_before_picks_most_recent() {
        let mut h = History::default();
        h.append(HistoryEntry {
            date: "a".into(),
            timestamp: 100,
            repos: 1,
            findings: 1,
            sessions: 1,
        });
        h.append(HistoryEntry {
            date: "b".into(),
            timestamp: 200,
            repos: 2,
            findings: 2,
            sessions: 2,
        });
        h.append(HistoryEntry {
            date: "c".into(),
            timestamp: 300,
            repos: 3,
            findings: 3,
            sessions: 3,
        });
        let prev = h.previous_before(300).unwrap();
        assert_eq!(prev.timestamp, 200);
    }

    #[test]
    fn delta_summary_formats() {
        let prev = HistoryEntry {
            date: "2026-04-07".into(),
            timestamp: 1_712_563_200,
            repos: 28,
            findings: 215,
            sessions: 30,
        };
        let cur = HistoryEntry {
            date: "2026-04-09".into(),
            timestamp: 1_712_736_000,
            repos: 28,
            findings: 213,
            sessions: 29,
        };
        let d = Delta { previous: &prev, current: &cur };
        let line = d.summary_line();
        assert!(line.contains("215 → 213"));
        assert!(line.contains("(-2)"));
        assert!(line.contains("(-1)"));
    }
}
