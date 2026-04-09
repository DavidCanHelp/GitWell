//! Flag repos with no commits to any branch in N+ months.

use super::{Finding, Scanner};
use crate::git::Repo;
use crate::util;

pub struct DormantScanner {
    pub dormant_secs: u64,
}

impl Scanner for DormantScanner {
    fn name(&self) -> &'static str {
        "Dormant Repos"
    }

    fn scan(&self, repo: &Repo) -> Vec<Finding> {
        let out = match repo.run(&[
            "for-each-ref",
            "--format=%(committerdate:unix)",
            "refs/heads/",
        ]) {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };

        let latest = out
            .lines()
            .filter_map(|l| l.trim().parse::<u64>().ok())
            .max()
            .unwrap_or(0);

        // Empty or freshly-initialized repos (no commits yet) aren't "dormant",
        // they're just new. Skip them.
        if latest == 0 {
            return Vec::new();
        }

        let now = util::now_unix();
        if now.saturating_sub(latest) < self.dormant_secs {
            return Vec::new();
        }

        vec![Finding::DormantRepo {
            path: repo.path.display().to_string(),
            last_activity_ts: latest,
        }]
    }
}
