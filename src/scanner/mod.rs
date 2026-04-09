//! Scanner trait and registry.
//!
//! Each scanner examines a [`Repo`] and returns a list of [`Finding`]s
//! describing a particular flavor of abandoned work.

use crate::config::Config;
use crate::git::Repo;

pub mod branches;
pub mod dormant;
pub mod orphans;
pub mod stashes;
pub mod wip;

/// A single piece of abandoned work surfaced by a scanner.
#[derive(Debug, Clone)]
pub enum Finding {
    StaleBranch {
        name: String,
        last_commit_ts: u64,
        last_commit_message: String,
        ahead: usize,
        behind: usize,
    },
    Stash {
        index: String,
        /// Commit SHA of the stash. Captured at scan time so the executor
        /// can find the stash later even if its index has shifted.
        sha: String,
        ts: u64,
        message: String,
        files_changed: usize,
        insertions: usize,
        deletions: usize,
    },
    OrphanCommit {
        sha: String,
        ts: u64,
        message: String,
    },
    WipCommit {
        sha: String,
        ts: u64,
        message: String,
        marker: String,
    },
    DormantRepo {
        path: String,
        last_activity_ts: u64,
    },
}

impl Finding {
    /// The timestamp used for age-based sorting and coloring.
    pub fn timestamp(&self) -> u64 {
        match self {
            Finding::StaleBranch { last_commit_ts, .. } => *last_commit_ts,
            Finding::Stash { ts, .. } => *ts,
            Finding::OrphanCommit { ts, .. } => *ts,
            Finding::WipCommit { ts, .. } => *ts,
            Finding::DormantRepo { last_activity_ts, .. } => *last_activity_ts,
        }
    }
}

pub trait Scanner {
    /// Short human-readable section title, e.g. "Stale Branches".
    fn name(&self) -> &'static str;

    /// Scan a single repo. Implementations should swallow expected git
    /// errors (missing refs, empty repos, etc.) and return an empty Vec
    /// rather than panicking.
    fn scan(&self, repo: &Repo) -> Vec<Finding>;
}

/// The registry of scanners run on every repo, in report order.
/// Takes a [`Config`] so scanners get their thresholds from one source.
pub fn registry(config: &Config) -> Vec<Box<dyn Scanner>> {
    vec![
        Box::new(dormant::DormantScanner {
            dormant_secs: config.dormant_secs(),
        }),
        Box::new(branches::BranchScanner {
            stale_secs: config.stale_secs(),
            ignore_branches: config.ignore_branches.clone(),
        }),
        Box::new(stashes::StashScanner),
        Box::new(orphans::OrphanScanner),
        Box::new(wip::WipScanner),
    ]
}
