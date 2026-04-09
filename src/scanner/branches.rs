//! Find stale branches: unmerged local branches with no recent commits.

use super::{Finding, Scanner};
use crate::git::Repo;
use crate::util;

pub struct BranchScanner {
    pub stale_secs: u64,
    pub ignore_branches: Vec<String>,
}

impl Scanner for BranchScanner {
    fn name(&self) -> &'static str {
        "Stale Branches"
    }

    fn scan(&self, repo: &Repo) -> Vec<Finding> {
        let default = match repo.default_branch() {
            Some(d) => d,
            None => return Vec::new(),
        };

        // One line per local branch: name<TAB>committerdate(unix)<TAB>subject
        let out = match repo.run(&[
            "for-each-ref",
            "--format=%(refname:short)%09%(committerdate:unix)%09%(contents:subject)",
            "refs/heads/",
        ]) {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };

        let now = util::now_unix();
        let mut results = Vec::new();

        for line in out.lines() {
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() < 3 {
                continue;
            }
            let name = parts[0];
            if name == default {
                continue;
            }

            // Config-driven ignore list (e.g. "dependabot/*").
            if self
                .ignore_branches
                .iter()
                .any(|p| util::glob_match(p, name))
            {
                continue;
            }

            let ts: u64 = parts[1].parse().unwrap_or(0);
            let msg = parts[2].trim().to_string();

            // Only branches untouched past the stale threshold count.
            if now.saturating_sub(ts) < self.stale_secs {
                continue;
            }

            // Skip branches already merged into default.
            if repo.succeeds(&["merge-base", "--is-ancestor", name, &default]) {
                continue;
            }

            let (ahead, behind) = rev_list_counts(repo, &default, name);

            results.push(Finding::StaleBranch {
                name: name.to_string(),
                last_commit_ts: ts,
                last_commit_message: msg,
                ahead,
                behind,
            });
        }

        results.sort_by_key(|f| f.timestamp());
        results
    }
}

/// Returns `(ahead, behind)` — how many commits `branch` is ahead/behind of `base`.
fn rev_list_counts(repo: &Repo, base: &str, branch: &str) -> (usize, usize) {
    let range = format!("{}...{}", base, branch);
    if let Ok(s) = repo.run(&["rev-list", "--left-right", "--count", &range]) {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() == 2 {
            let behind = parts[0].parse().unwrap_or(0);
            let ahead = parts[1].parse().unwrap_or(0);
            return (ahead, behind);
        }
    }
    (0, 0)
}
