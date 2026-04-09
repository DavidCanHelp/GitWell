//! Integration tests for `narrative::summarize`.
//!
//! These tests build `Cluster` values by hand (since narrative is a
//! pure function) and assert on template-output substrings.

mod common;

use gitwell::cluster::Cluster;
use gitwell::narrative;
use gitwell::scanner::Finding;

fn wip(sha: &str, ts: u64, message: &str) -> Finding {
    Finding::WipCommit {
        sha: sha.to_string(),
        ts,
        message: message.to_string(),
        marker: "WIP".to_string(),
    }
}

fn stale_branch(name: &str, ts: u64) -> Finding {
    Finding::StaleBranch {
        name: name.to_string(),
        last_commit_ts: ts,
        last_commit_message: "msg".to_string(),
        ahead: 1,
        behind: 0,
    }
}

#[test]
fn cross_repo_cluster_mentions_repo_count() {
    // Use a timestamp in 2020 so the month name in the narrative is
    // deterministic and independent of today's clock.
    // 2020-01-01T00:00:00Z = 1_577_836_800
    // 2020-01-02T00:00:00Z = 1_577_923_200
    let start = 1_577_836_800;
    let end = 1_577_923_200;

    let cluster = Cluster {
        label: "auth".to_string(),
        start_ts: start,
        end_ts: end,
        repos: vec!["alpha".into(), "beta".into(), "gamma".into()],
        findings: vec![
            ("alpha".into(), stale_branch("feature/auth-alpha", start)),
            ("beta".into(), stale_branch("feature/auth-beta", start + 3600)),
            ("gamma".into(), stale_branch("feature/auth-gamma", end)),
        ],
        top_keywords: vec!["auth".into(), "refactor".into()],
    };

    let summary = narrative::summarize(&cluster, 1_800_000_000);
    assert!(
        summary.contains("across 3 repos"),
        "cross-repo narrative must mention repo count; got {:?}",
        summary
    );
}

#[test]
fn wip_sprint_cluster_mentions_single_weekend() {
    // 3 WIPs in the same repo all within a few hours of each other
    // should trigger the "single weekend" wording in the WIP-sprint
    // template.
    let start = 1_577_836_800; // 2020-01-01
    let cluster = Cluster {
        label: "auth".to_string(),
        start_ts: start,
        end_ts: start + 4 * 3600, // 4 hours later
        repos: vec!["alpha".into()],
        findings: vec![
            ("alpha".into(), wip("a1111111a1", start, "WIP auth A")),
            ("alpha".into(), wip("a2222222a2", start + 2 * 3600, "WIP auth B")),
            ("alpha".into(), wip("a3333333a3", start + 4 * 3600, "WIP auth C")),
        ],
        top_keywords: vec!["auth".into()],
    };

    let summary = narrative::summarize(&cluster, 1_800_000_000);
    assert!(
        summary.contains("single weekend"),
        "short-span WIP burst must read as 'a single weekend'; got {:?}",
        summary
    );
    assert!(
        summary.contains("sprint that stalled"),
        "narrative should call it a stalled sprint; got {:?}",
        summary
    );
}

#[test]
fn big_stash_cluster_mentions_file_count() {
    // A single stash with a large file count + insertions should hit
    // the "big stash" template and mention the file count explicitly.
    //
    // Note: narrative's big_stash template fires when total()==1 AND
    // it's a stash AND (files >= 10 or ins+dels >= 200). A singleton
    // would normally be filtered out by `build_clusters`, but
    // `narrative::summarize` is called on already-built clusters and
    // doesn't enforce that — so we hand-build the cluster directly.
    let ts = 1_577_836_800;
    let stash = Finding::Stash {
        index: "stash@{0}".to_string(),
        sha: "cafebabe00000000".to_string(),
        ts,
        message: "WIP huge refactor".to_string(),
        files_changed: 14,
        insertions: 300,
        deletions: 45,
    };

    let cluster = Cluster {
        label: "refactor".to_string(),
        start_ts: ts,
        end_ts: ts,
        repos: vec!["chatdelta".into()],
        findings: vec![("chatdelta".into(), stash)],
        top_keywords: vec!["refactor".into()],
    };

    let summary = narrative::summarize(&cluster, 1_800_000_000);
    assert!(
        summary.contains("14 files"),
        "big-stash narrative must mention file count; got {:?}",
        summary
    );
    assert!(
        summary.to_lowercase().contains("big effort"),
        "big-stash narrative should call it a 'big effort'; got {:?}",
        summary
    );
}
