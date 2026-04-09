//! Cross-scanner clustering: group findings that look like the same
//! abandoned effort.
//!
//! ## Algorithm
//!
//! Clustering runs in two stages so we don't get giant transitive chains:
//!
//! ### Stage 1 — per-repo sessions by time gap
//!
//! Within each repo, sort findings by timestamp and walk forward. Start a
//! new session whenever the gap to the previous finding exceeds
//! `session_window_hours`. This captures the "I was deep in this repo that
//! weekend" case without chaining years of weekly commits into one run.
//!
//! Findings with no usable timestamp (e.g. an orphan commit whose object
//! has been GC'd) are grouped into a single per-repo "unknown-time"
//! session.
//!
//! ### Stage 2 — cross-repo merge on shared themes
//!
//! Each session has a keyword set (the union of its findings' tokens).
//! Two sessions from *different* repos merge if their keyword sets
//! intersect on **at least two** non-stopword tokens. A single shared
//! word like "broken" or "cleanup" is too weak to justify linking — it
//! would transitively merge thousands of unrelated commits. Merging is
//! done with union-find over sessions (not individual findings).
//!
//! After both stages, sessions with fewer than two findings are dropped
//! (singletons already show up in the per-scanner sections).
//!
//! The produced [`Cluster`] carries a human label (top keyword), the date
//! range, the set of affected repos, and the findings themselves. The
//! [`crate::narrative`] module turns that into a one-line summary.

use std::collections::{HashMap, HashSet};

use crate::config::Config;
use crate::report::RepoReport;
use crate::scanner::Finding;

/// A group of findings that appear to belong to the same abandoned effort.
#[derive(Debug, Clone)]
pub struct Cluster {
    /// Short label for the cluster — usually the top keyword ("auth",
    /// "refactor"), or a fallback like "abandoned work".
    pub label: String,
    /// Earliest finding timestamp in the cluster.
    pub start_ts: u64,
    /// Latest finding timestamp in the cluster.
    pub end_ts: u64,
    /// Distinct repo names involved, sorted.
    pub repos: Vec<String>,
    /// Every finding in the cluster, paired with its repo name.
    pub findings: Vec<(String, Finding)>,
    /// Top keywords by frequency (up to 3), useful for narratives.
    pub top_keywords: Vec<String>,
}

/// Build clusters from scan reports. Returns clusters sorted by size
/// (descending), breaking ties by most recent activity.
pub fn build_clusters(reports: &[RepoReport], config: &Config) -> Vec<Cluster> {
    let window = config.session_window_secs();

    // --- Stage 1: per-repo time-gap sessions ----------------------------
    let mut sessions: Vec<Session> = Vec::new();
    for report in reports {
        let mut items: Vec<Item> = Vec::new();
        for (_section, findings) in &report.sections {
            for f in findings {
                items.push(Item {
                    repo: report.repo_name.clone(),
                    timestamp: f.timestamp(),
                    keywords: keywords_from_finding(f, &report.repo_name),
                    finding: f.clone(),
                });
            }
        }

        // Separate findings with usable timestamps from ones we can't place.
        let (mut dated, undated): (Vec<Item>, Vec<Item>) =
            items.into_iter().partition(|i| i.timestamp > 0);
        dated.sort_by_key(|i| i.timestamp);

        // Walk forward, starting a new session whenever the gap exceeds
        // the configured window. This is what prevents one-WIP-a-week for
        // five years from collapsing into a single cluster.
        let mut current: Vec<Item> = Vec::new();
        for item in dated {
            if let Some(last) = current.last() {
                if item.timestamp.saturating_sub(last.timestamp) > window {
                    sessions.push(Session::from_items(std::mem::take(&mut current)));
                }
            }
            current.push(item);
        }
        if !current.is_empty() {
            sessions.push(Session::from_items(current));
        }

        // Undated findings in the same repo go together as one session.
        if !undated.is_empty() {
            sessions.push(Session::from_items(undated));
        }
    }

    // --- Stage 2: cross-repo merge via shared keywords (≥2) ------------
    let n = sessions.len();
    let mut uf = UnionFind::new(n);
    for i in 0..n {
        for j in (i + 1)..n {
            if sessions[i].repo == sessions[j].repo {
                continue;
            }
            if shared_keyword_count(&sessions[i].keyword_set, &sessions[j].keyword_set) >= 2 {
                uf.union(i, j);
            }
        }
    }

    // --- Stage 3: collect union-find buckets into final clusters --------
    let mut buckets: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        buckets.entry(uf.find(i)).or_default().push(i);
    }

    let mut clusters: Vec<Cluster> = Vec::new();
    for (_, indices) in buckets {
        let cluster = build_cluster(&indices, &mut sessions);
        if cluster.findings.len() < 2 {
            continue;
        }
        clusters.push(cluster);
    }

    // Sort: biggest first, then most recent.
    clusters.sort_by(|a, b| {
        b.findings
            .len()
            .cmp(&a.findings.len())
            .then_with(|| b.end_ts.cmp(&a.end_ts))
    });

    clusters
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

struct Item {
    repo: String,
    timestamp: u64,
    keywords: Vec<String>,
    finding: Finding,
}

/// A burst of findings from a single repo that lived close together in time.
struct Session {
    repo: String,
    items: Vec<Item>,
    /// Union of every item's keyword list, for cross-repo merge tests.
    keyword_set: Vec<String>,
    /// Running frequency count so the merged cluster can label itself.
    keyword_freq: HashMap<String, usize>,
    start_ts: u64,
    end_ts: u64,
}

impl Session {
    fn from_items(items: Vec<Item>) -> Self {
        let repo = items
            .first()
            .map(|i| i.repo.clone())
            .unwrap_or_default();

        let mut keyword_set: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut keyword_freq: HashMap<String, usize> = HashMap::new();
        let mut start_ts = u64::MAX;
        let mut end_ts = 0u64;

        for item in &items {
            if item.timestamp > 0 {
                start_ts = start_ts.min(item.timestamp);
                end_ts = end_ts.max(item.timestamp);
            }
            for kw in &item.keywords {
                if seen.insert(kw.clone()) {
                    keyword_set.push(kw.clone());
                }
                *keyword_freq.entry(kw.clone()).or_insert(0) += 1;
            }
        }

        if start_ts == u64::MAX {
            start_ts = 0;
        }

        Session {
            repo,
            items,
            keyword_set,
            keyword_freq,
            start_ts,
            end_ts,
        }
    }
}

/// Merge the sessions at `indices` into one `Cluster`. Sessions are
/// consumed (drained) so their items can move without cloning.
fn build_cluster(indices: &[usize], sessions: &mut [Session]) -> Cluster {
    let mut findings: Vec<(String, Finding)> = Vec::new();
    let mut repos: HashSet<String> = HashSet::new();
    let mut start_ts = u64::MAX;
    let mut end_ts = 0u64;
    let mut freq: HashMap<String, usize> = HashMap::new();

    for idx in indices {
        let session = &mut sessions[*idx];
        repos.insert(session.repo.clone());
        if session.end_ts > 0 {
            start_ts = start_ts.min(session.start_ts);
            end_ts = end_ts.max(session.end_ts);
        }
        for (k, v) in session.keyword_freq.drain() {
            *freq.entry(k).or_insert(0) += v;
        }
        for item in session.items.drain(..) {
            findings.push((item.repo, item.finding));
        }
    }

    // Chronological so the narrative reads forward in time.
    findings.sort_by_key(|(_, f)| f.timestamp());

    let mut repos: Vec<String> = repos.into_iter().collect();
    repos.sort();

    let top_keywords = top_keywords(&freq, 3);
    let label = top_keywords
        .first()
        .cloned()
        .unwrap_or_else(|| "abandoned work".to_string());

    if start_ts == u64::MAX {
        start_ts = 0;
    }

    Cluster {
        label,
        start_ts,
        end_ts,
        repos,
        findings,
        top_keywords,
    }
}

fn shared_keyword_count(a: &[String], b: &[String]) -> usize {
    let mut count = 0;
    for ka in a {
        if b.iter().any(|kb| kb == ka) {
            count += 1;
        }
    }
    count
}

fn top_keywords(freq: &HashMap<String, usize>, limit: usize) -> Vec<String> {
    let mut entries: Vec<(&String, &usize)> = freq.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    entries.into_iter().take(limit).map(|(k, _)| k.clone()).collect()
}

// ---------------------------------------------------------------------------
// Keyword extraction
// ---------------------------------------------------------------------------

/// Pull likely-meaningful keywords out of a finding.
pub fn keywords_from_finding(f: &Finding, repo_name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    let push_from = |src: &str, out: &mut Vec<String>, seen: &mut HashSet<String>| {
        for word in src.split(|c: char| !c.is_alphanumeric()) {
            if word.len() < 3 {
                continue;
            }
            let lower = word.to_lowercase();
            if is_stopword(&lower) {
                continue;
            }
            if seen.insert(lower.clone()) {
                out.push(lower);
            }
        }
    };

    match f {
        Finding::StaleBranch {
            name,
            last_commit_message,
            ..
        } => {
            push_from(name, &mut out, &mut seen);
            push_from(last_commit_message, &mut out, &mut seen);
        }
        Finding::Stash { message, .. } => {
            push_from(message, &mut out, &mut seen);
        }
        Finding::OrphanCommit { message, .. } => {
            push_from(message, &mut out, &mut seen);
        }
        Finding::WipCommit { message, .. } => {
            push_from(message, &mut out, &mut seen);
        }
        Finding::DormantRepo { .. } => {
            // Fall through to repo_name below.
        }
    }

    // Repo name contributes to clustering too — a "helpme" repo and a
    // "helpme-docs" repo share the "helpme" keyword.
    push_from(repo_name, &mut out, &mut seen);

    out
}

/// Words that carry no thematic signal on their own. Filtered so we don't
/// link together every commit that happens to say "fix" or "update".
fn is_stopword(s: &str) -> bool {
    matches!(
        s,
        // Common English
        "the" | "and" | "for" | "with" | "from" | "into" | "this" | "that"
        | "was" | "are" | "not" | "but" | "you" | "all" | "any" | "can"
        | "has" | "had" | "its" | "out" | "via" | "use" | "uses" | "used"
        | "now" | "one" | "two" | "three" | "some" | "more" | "less"
        | "when" | "then" | "them" | "they" | "our" | "your" | "about"
        | "over" | "under" | "only" | "also" | "just" | "back"
        // Git verbs / commit fluff
        | "add" | "adds" | "added" | "adding"
        | "fix" | "fixes" | "fixed" | "fixing"
        | "update" | "updates" | "updated" | "updating"
        | "remove" | "removes" | "removed" | "removing"
        | "delete" | "deletes" | "deleted" | "deleting"
        | "change" | "changes" | "changed" | "changing"
        | "merge" | "merges" | "merged" | "merging"
        | "revert" | "reverts" | "reverted" | "reverting"
        | "bump" | "bumps" | "bumped"
        | "rename" | "renames" | "renamed"
        | "move" | "moves" | "moved" | "moving"
        | "clean" | "cleans" | "cleaned" | "cleanup"
        | "tweak" | "tweaks" | "tweaked"
        // Generic commit nouns
        | "wip" | "todo" | "fixme" | "tmp" | "temp"
        | "new" | "old" | "test" | "tests" | "testing"
        | "doc" | "docs" | "readme"
        | "initial" | "commit" | "commits"
        | "file" | "files" | "code" | "source"
        | "branch" | "branches" | "master" | "main" | "develop" | "trunk"
        // Conventional commit prefixes
        | "feat" | "chore" | "style" | "build" | "builds" | "building"
        // Issue tracker noise
        | "issue" | "issues" | "bug" | "bugs"
        // WIP-marker words themselves — these appear as signals, not topics.
        // Without this, every commit whose message says "broken" or "hack"
        // would link into one giant transitive cluster.
        | "broken" | "hack" | "hacks" | "hacky"
        | "experiment" | "experiments" | "experimental"
        | "trying" | "tried" | "attempt" | "attempts" | "attempting"
        | "fail" | "fails" | "failed" | "failing" | "failure"
        | "pass" | "passes" | "passed" | "passing"
        | "work" | "works" | "worked" | "working"
        | "make" | "makes" | "made" | "making"
        | "get" | "gets" | "got" | "getting"
        | "set" | "sets" | "setting"
        | "run" | "runs" | "ran" | "running"
        | "start" | "starts" | "started" | "starting"
        | "stop" | "stops" | "stopped" | "stopping"
        | "check" | "checks" | "checked" | "checking"
        | "enable" | "enabled" | "disable" | "disabled"
        | "allow" | "allows" | "allowed"
        | "show" | "shows" | "showed" | "showing"
        | "hide" | "hides" | "hidden"
        | "implement" | "implements" | "implemented"
    )
}

// ---------------------------------------------------------------------------
// Union-find
// ---------------------------------------------------------------------------

struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    fn find(&mut self, x: usize) -> usize {
        // Iterative path compression.
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut cur = x;
        while self.parent[cur] != root {
            let next = self.parent[cur];
            self.parent[cur] = root;
            cur = next;
        }
        root
    }

    fn union(&mut self, x: usize, y: usize) {
        let rx = self.find(x);
        let ry = self.find(y);
        if rx == ry {
            return;
        }
        match self.rank[rx].cmp(&self.rank[ry]) {
            std::cmp::Ordering::Less => self.parent[rx] = ry,
            std::cmp::Ordering::Greater => self.parent[ry] = rx,
            std::cmp::Ordering::Equal => {
                self.parent[ry] = rx;
                self.rank[rx] += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uf_links() {
        let mut uf = UnionFind::new(5);
        uf.union(0, 1);
        uf.union(1, 2);
        uf.union(3, 4);
        assert_eq!(uf.find(0), uf.find(2));
        assert_ne!(uf.find(0), uf.find(3));
    }

    #[test]
    fn stopwords_filtered() {
        let f = Finding::WipCommit {
            sha: "abc".into(),
            ts: 0,
            message: "WIP fix auth flow for new login".into(),
            marker: "WIP".into(),
        };
        let kws = keywords_from_finding(&f, "my-repo");
        assert!(kws.contains(&"auth".to_string()));
        assert!(kws.contains(&"flow".to_string()));
        assert!(kws.contains(&"login".to_string()));
        assert!(!kws.contains(&"fix".to_string()));
        assert!(!kws.contains(&"wip".to_string()));
    }
}
