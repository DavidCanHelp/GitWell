//! List all stashes with date, message, and diff stats.

use super::{Finding, Scanner};
use crate::git::Repo;

pub struct StashScanner;

impl Scanner for StashScanner {
    fn name(&self) -> &'static str {
        "Stashes"
    }

    fn scan(&self, repo: &Repo) -> Vec<Finding> {
        // stash@{N}<TAB>commit_sha<TAB>committer_ts<TAB>subject
        let out = match repo.run(&["stash", "list", "--format=%gd%x09%H%x09%ct%x09%s"]) {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };

        let mut results = Vec::new();
        for line in out.lines() {
            let parts: Vec<&str> = line.splitn(4, '\t').collect();
            if parts.len() < 4 {
                continue;
            }
            let index = parts[0].to_string();
            let sha = parts[1].to_string();
            let ts: u64 = parts[2].parse().unwrap_or(0);
            let msg = parts[3].to_string();

            let (files_changed, insertions, deletions) = stash_stats(repo, &index);

            results.push(Finding::Stash {
                index,
                sha,
                ts,
                message: msg,
                files_changed,
                insertions,
                deletions,
            });
        }

        results.sort_by_key(|f| f.timestamp());
        results
    }
}

/// Parse `git stash show --numstat <ref>` output:
///   <added>\t<deleted>\t<path>
fn stash_stats(repo: &Repo, index: &str) -> (usize, usize, usize) {
    let out = match repo.run(&["stash", "show", "--numstat", index]) {
        Ok(o) => o,
        Err(_) => return (0, 0, 0),
    };

    let mut files = 0usize;
    let mut ins = 0usize;
    let mut dels = 0usize;
    for line in out.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            files += 1;
            // Binary files show "-"; treat as 0 additions/deletions.
            ins += parts[0].parse::<usize>().unwrap_or(0);
            dels += parts[1].parse::<usize>().unwrap_or(0);
        }
    }
    (files, ins, dels)
}
