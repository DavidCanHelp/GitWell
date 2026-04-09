//! Template-based narrative summaries for clusters.
//!
//! Given a [`Cluster`], classify its shape and render a one-line human
//! summary. No AI, no ML — just heuristics over the finding mix, repo
//! count, and date range.
//!
//! The templates roughly correspond to:
//!
//! - **cross-repo effort** — multi-repo, keyword-themed
//! - **WIP sprint** — single repo, mostly WIP commits
//! - **big stash** — single repo, a single large stash
//! - **orphan graveyard** — single repo, mostly orphan commits
//! - **generic abandoned effort** — catch-all for mixed single-repo clusters
//! - **dormant sweep** — cluster of dormant repos

use crate::cluster::Cluster;
use crate::scanner::Finding;
use crate::util;

/// Produce a one-line narrative for a cluster.
pub fn summarize(cluster: &Cluster, now: u64) -> String {
    let counts = Counts::from(&cluster.findings);

    // All-dormant sweep (usually cross-repo).
    if counts.dormant == counts.total() && counts.dormant >= 2 {
        return dormant_sweep(cluster, &counts);
    }

    // Cross-repo clusters are (almost) always about a shared theme.
    if cluster.repos.len() >= 2 {
        return cross_repo_effort(cluster, &counts);
    }

    // Single-repo, big lone stash.
    if counts.total() == 1 {
        if let Some(Finding::Stash {
            files_changed,
            insertions,
            deletions,
            ts,
            ..
        }) = cluster.findings.first().map(|(_, f)| f)
        {
            if *files_changed >= 10 || *insertions + *deletions >= 200 {
                return big_stash(cluster, *files_changed, *insertions, *deletions, *ts, now);
            }
        }
    }

    // Single-repo WIP-heavy sprint.
    if counts.wip >= 3 && counts.wip * 2 >= counts.total() {
        return wip_sprint(cluster, &counts);
    }

    // Single-repo orphan graveyard.
    if counts.orphan >= 3 && counts.orphan * 2 >= counts.total() {
        return orphan_graveyard(cluster, &counts);
    }

    generic_effort(cluster, &counts)
}

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

fn cross_repo_effort(cluster: &Cluster, counts: &Counts) -> String {
    let label = effort_label(cluster);
    let period = util::format_period(cluster.start_ts, cluster.end_ts);
    let repos = cluster.repos.len();
    let breakdown = counts.breakdown();
    let tail = if counts.stale_branches > 0 {
        ", never merged"
    } else if counts.stash > 0 {
        ", never applied"
    } else {
        ""
    };
    format!(
        "In {}, you started {} across {} repos. {}{}.",
        period, label, repos, breakdown, tail
    )
}

fn wip_sprint(cluster: &Cluster, counts: &Counts) -> String {
    let repo = cluster
        .repos
        .first()
        .cloned()
        .unwrap_or_else(|| "this repo".to_string());
    let period = period_or_weekend(cluster);
    format!(
        "You have {} WIP commits in {} from {}. Looks like a sprint that stalled.",
        counts.wip, repo, period
    )
}

fn orphan_graveyard(cluster: &Cluster, counts: &Counts) -> String {
    let repo = cluster
        .repos
        .first()
        .cloned()
        .unwrap_or_else(|| "this repo".to_string());
    let period = util::format_period(cluster.start_ts, cluster.end_ts);
    format!(
        "{} orphan commits in {} from {} — dead reflog entries, nothing references them.",
        counts.orphan, repo, period
    )
}

fn big_stash(
    cluster: &Cluster,
    files: usize,
    ins: usize,
    dels: usize,
    ts: u64,
    now: u64,
) -> String {
    let repo = cluster
        .repos
        .first()
        .cloned()
        .unwrap_or_else(|| "this repo".to_string());
    let age = util::format_age_ago(ts, now);
    format!(
        "{} has a stash from {} with changes to {} files (+{}/-{}). Big effort, never applied.",
        repo, age, files, ins, dels
    )
}

fn dormant_sweep(cluster: &Cluster, counts: &Counts) -> String {
    let period = util::format_period(cluster.start_ts, cluster.end_ts);
    format!(
        "{} dormant repos, last touched {}. Nothing pushed there in ages.",
        counts.dormant, period
    )
}

fn generic_effort(cluster: &Cluster, counts: &Counts) -> String {
    let repo = cluster
        .repos
        .first()
        .cloned()
        .unwrap_or_else(|| "this repo".to_string());
    let period = util::format_period(cluster.start_ts, cluster.end_ts);
    let breakdown = counts.breakdown();
    format!("{} has {} from {}. An abandoned effort.", repo, breakdown, period)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// "an auth refactor" / "a migration effort" / "a push" (fallback).
fn effort_label(cluster: &Cluster) -> String {
    let kws = &cluster.top_keywords;
    // Look for a known effort-shape word to anchor the phrase.
    let shape_words = [
        "refactor",
        "migration",
        "rewrite",
        "redesign",
        "cleanup",
        "overhaul",
        "upgrade",
    ];
    let shape = kws.iter().find(|k| shape_words.contains(&k.as_str()));
    let topic = kws.iter().find(|k| !shape_words.contains(&k.as_str()));

    match (topic, shape) {
        (Some(t), Some(s)) => format!("{} {}", article(t), phrase(t, s)),
        (Some(t), None) => format!("{} {} effort", article(t), t),
        (None, Some(s)) => format!("{} {}", article(s), s),
        (None, None) => "an effort".to_string(),
    }
}

fn phrase(topic: &str, shape: &str) -> String {
    format!("{} {}", topic, shape)
}

/// Very rough "a" vs "an" picker — good enough for keyword labels.
fn article(word: &str) -> &'static str {
    match word.chars().next() {
        Some(c) if "aeiou".contains(c.to_ascii_lowercase()) => "an",
        _ => "a",
    }
}

/// "a single weekend" if the span is short, otherwise the normal period.
fn period_or_weekend(cluster: &Cluster) -> String {
    let span = cluster.end_ts.saturating_sub(cluster.start_ts);
    if span <= 3 * 86_400 {
        format!("a single weekend in {}", util::format_month_year(cluster.start_ts))
    } else if span <= 7 * 86_400 {
        format!("the week of {}", util::format_month_year(cluster.start_ts))
    } else {
        util::format_period(cluster.start_ts, cluster.end_ts)
    }
}

// ---------------------------------------------------------------------------
// Count breakdown
// ---------------------------------------------------------------------------

struct Counts {
    stale_branches: usize,
    stash: usize,
    wip: usize,
    orphan: usize,
    dormant: usize,
}

impl Counts {
    fn from(findings: &[(String, Finding)]) -> Self {
        let mut c = Counts {
            stale_branches: 0,
            stash: 0,
            wip: 0,
            orphan: 0,
            dormant: 0,
        };
        for (_, f) in findings {
            match f {
                Finding::StaleBranch { .. } => c.stale_branches += 1,
                Finding::Stash { .. } => c.stash += 1,
                Finding::WipCommit { .. } => c.wip += 1,
                Finding::OrphanCommit { .. } => c.orphan += 1,
                Finding::DormantRepo { .. } => c.dormant += 1,
            }
        }
        c
    }

    fn total(&self) -> usize {
        self.stale_branches + self.stash + self.wip + self.orphan + self.dormant
    }

    /// Natural-language breakdown: "3 branches, 2 stashes and 7 WIP commits".
    fn breakdown(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.stale_branches > 0 {
            parts.push(plural(self.stale_branches, "branch", "branches"));
        }
        if self.stash > 0 {
            parts.push(plural(self.stash, "stash", "stashes"));
        }
        if self.wip > 0 {
            parts.push(plural(self.wip, "WIP commit", "WIP commits"));
        }
        if self.orphan > 0 {
            parts.push(plural(self.orphan, "orphan commit", "orphan commits"));
        }
        if self.dormant > 0 {
            parts.push(plural(self.dormant, "dormant repo", "dormant repos"));
        }
        join_with_and(&parts)
    }
}

fn plural(n: usize, singular: &str, plural: &str) -> String {
    if n == 1 {
        format!("{} {}", n, singular)
    } else {
        format!("{} {}", n, plural)
    }
}

fn join_with_and(parts: &[String]) -> String {
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        2 => format!("{} and {}", parts[0], parts[1]),
        _ => {
            let head = parts[..parts.len() - 1].join(", ");
            format!("{} and {}", head, parts[parts.len() - 1])
        }
    }
}
