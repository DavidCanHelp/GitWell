//! Walk the reflog for commits no longer reachable from any branch or tag.
//!
//! Abandoned rebases, resets, and amended commits often linger in the reflog
//! long after the references that held them have moved on. These are the
//! "orphans" — work that's still in the object store but not pointed to by
//! any ref.

use super::{Finding, Scanner};
use crate::git::Repo;
use std::collections::HashSet;

pub struct OrphanScanner;

impl Scanner for OrphanScanner {
    fn name(&self) -> &'static str {
        "Orphan Commits"
    }

    fn scan(&self, repo: &Repo) -> Vec<Finding> {
        // Every commit that ever appeared in any ref's reflog.
        let reflog = match repo.run(&["reflog", "--all", "--format=%H"]) {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };
        let seen: HashSet<String> = reflog
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        // Every commit currently reachable from any ref (branches + tags).
        let reachable_out = match repo.run(&["rev-list", "--all"]) {
            Ok(o) => o,
            Err(_) => String::new(),
        };
        let reachable: HashSet<String> = reachable_out
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let mut results = Vec::new();
        for sha in seen.difference(&reachable) {
            // A commit may have existed in the reflog but since been GC'd;
            // `git log -1` will fail for those — just skip.
            if let Ok(info) = repo.run(&["log", "-1", "--format=%ct%x09%s", sha]) {
                let parts: Vec<&str> = info.trim().splitn(2, '\t').collect();
                if parts.len() == 2 {
                    let ts = parts[0].parse().unwrap_or(0);
                    let msg = parts[1].to_string();
                    results.push(Finding::OrphanCommit {
                        sha: sha.clone(),
                        ts,
                        message: msg,
                    });
                }
            }
        }

        results.sort_by_key(|f| f.timestamp());
        results
    }
}
