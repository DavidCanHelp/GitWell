//! Surface commits whose messages betray unfinished intentions.
//!
//! We search commit subjects (across all refs) for WIP-style markers.
//! Matches are word-level and case-insensitive to avoid false positives
//! like "Attempt" matching "temp".

use super::{Finding, Scanner};
use crate::git::Repo;

pub struct WipScanner;

const MARKERS: &[&str] = &[
    "WIP",
    "TODO",
    "FIXME",
    "temp",
    "hack",
    "experiment",
    "trying",
    "broken",
];

impl Scanner for WipScanner {
    fn name(&self) -> &'static str {
        "WIP Markers"
    }

    fn scan(&self, repo: &Repo) -> Vec<Finding> {
        // sha<TAB>committer_ts<TAB>subject across every ref.
        let out = match repo.run(&["log", "--all", "--format=%H%x09%ct%x09%s"]) {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };

        let mut results = Vec::new();
        for line in out.lines() {
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() < 3 {
                continue;
            }
            let sha = parts[0];
            let ts: u64 = parts[1].parse().unwrap_or(0);
            let msg = parts[2];

            if let Some(marker) = match_marker(msg) {
                results.push(Finding::WipCommit {
                    sha: sha.to_string(),
                    ts,
                    message: msg.to_string(),
                    marker,
                });
            }
        }

        // Deduplicate: a commit reachable from multiple refs shows up once
        // per ref in `log --all`. Dedup on SHA, keep first.
        results.sort_by(|a, b| {
            let sa = match a {
                Finding::WipCommit { sha, .. } => sha.as_str(),
                _ => "",
            };
            let sb = match b {
                Finding::WipCommit { sha, .. } => sha.as_str(),
                _ => "",
            };
            sa.cmp(sb)
        });
        results.dedup_by(|a, b| match (a, b) {
            (Finding::WipCommit { sha: sa, .. }, Finding::WipCommit { sha: sb, .. }) => sa == sb,
            _ => false,
        });

        results.sort_by_key(|f| f.timestamp());
        results
    }
}

/// Check if `msg` contains any marker as a whole word (case-insensitive).
/// Returns the canonical marker form if found.
fn match_marker(msg: &str) -> Option<String> {
    let words: Vec<String> = msg
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect();

    for marker in MARKERS {
        let lower = marker.to_lowercase();
        if words.iter().any(|w| *w == lower) {
            return Some((*marker).to_string());
        }
    }
    None
}
