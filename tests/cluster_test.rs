//! Integration tests for `cluster::build_clusters`.
//!
//! These tests exercise the two-stage clustering algorithm:
//! 1. Per-repo time-gap sessions
//! 2. Cross-repo merge when ≥2 keywords are shared
//!
//! We build `RepoReport` values by hand here rather than running the
//! scanners. That lets us control timestamps and keyword sets precisely,
//! which is what the clustering rules turn on.

mod common;

use gitwell::cluster::{build_clusters, Cluster};
use gitwell::config::Config;
use gitwell::report::RepoReport;
use gitwell::scanner::Finding;

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn default_config() -> Config {
    Config::default() // stale_days=30, session_window_hours=48
}

/// Build a RepoReport from a list of WIP findings. One "WIP Markers"
/// section, nothing else.
fn report_from_wips(repo_name: &str, wips: Vec<Finding>) -> RepoReport {
    RepoReport {
        repo_name: repo_name.to_string(),
        repo_path: format!("/tmp/{}", repo_name),
        sections: vec![("WIP Markers".to_string(), wips)],
    }
}

fn wip(sha: &str, ts: u64, message: &str) -> Finding {
    Finding::WipCommit {
        sha: sha.to_string(),
        ts,
        message: message.to_string(),
        marker: "WIP".to_string(),
    }
}

fn cluster_containing_sha<'a>(clusters: &'a [Cluster], sha: &str) -> Option<&'a Cluster> {
    clusters.iter().find(|c| {
        c.findings.iter().any(|(_, f)| match f {
            Finding::WipCommit { sha: s, .. } => s == sha,
            _ => false,
        })
    })
}

// ---------------------------------------------------------------------------
// Per-repo time-gap rule
// ---------------------------------------------------------------------------

#[test]
fn same_repo_within_48h_is_one_session() {
    // Two WIP commits in the same repo, 1 hour apart. Must be merged
    // by the per-repo time-gap rule (default session_window_hours=48).
    let base = 1_700_000_000;
    let a = wip("aaaaaaaaaaaa", base, "WIP first auth thing");
    let b = wip("bbbbbbbbbbbb", base + 3_600, "WIP second auth thing");

    let reports = vec![report_from_wips("alpha", vec![a, b])];
    let clusters = build_clusters(&reports, &default_config());

    assert_eq!(
        clusters.len(),
        1,
        "both findings should form a single session; got {:?}",
        clusters
    );
    assert_eq!(clusters[0].findings.len(), 2);
}

#[test]
fn same_repo_months_apart_are_separate_sessions() {
    // Two WIPs in the same repo, 90 days apart. The time-gap split
    // should produce two distinct sessions, each with one finding,
    // and each singleton gets dropped since clusters require ≥2 members.
    //
    // To actually get 2 sessions reported we give each burst 2 findings.
    let base = 1_700_000_000;
    let jan_a = wip("aaaaaaaaaaaa", base, "WIP jan auth first");
    let jan_b = wip("cccccccccccc", base + 3_600, "WIP jan auth second");
    let apr_a = wip("bbbbbbbbbbbb", base + 90 * 86_400, "WIP apr auth first");
    let apr_b = wip("dddddddddddd", base + 90 * 86_400 + 3_600, "WIP apr auth second");

    let reports = vec![report_from_wips(
        "alpha",
        vec![jan_a, jan_b, apr_a, apr_b],
    )];
    let clusters = build_clusters(&reports, &default_config());

    assert_eq!(
        clusters.len(),
        2,
        "two bursts 90 days apart should form two sessions; got {:?}",
        clusters
    );

    // The two clusters should not share any findings.
    let c0_shas: Vec<String> = clusters[0]
        .findings
        .iter()
        .filter_map(|(_, f)| match f {
            Finding::WipCommit { sha, .. } => Some(sha.clone()),
            _ => None,
        })
        .collect();
    let c1_shas: Vec<String> = clusters[1]
        .findings
        .iter()
        .filter_map(|(_, f)| match f {
            Finding::WipCommit { sha, .. } => Some(sha.clone()),
            _ => None,
        })
        .collect();
    for s in &c0_shas {
        assert!(
            !c1_shas.contains(s),
            "sessions must not share findings; {} appears in both",
            s
        );
    }
}

// ---------------------------------------------------------------------------
// Cross-repo keyword merge rule
// ---------------------------------------------------------------------------

#[test]
fn two_repos_sharing_two_keywords_merge() {
    // Each repo has an in-window burst of 2 WIP commits. The bursts
    // share TWO keywords: "auth" and "refactor". Expected: one merged
    // cross-repo cluster with 4 findings and 2 repos.
    let base = 1_700_000_000;

    let alpha_findings = vec![
        wip("a111111111a1", base, "WIP auth refactor alpha login"),
        wip("a222222222a2", base + 3_600, "WIP auth refactor alpha session"),
    ];
    let beta_findings = vec![
        wip("b111111111b1", base, "WIP auth refactor beta login"),
        wip("b222222222b2", base + 3_600, "WIP auth refactor beta session"),
    ];

    let reports = vec![
        report_from_wips("alpha", alpha_findings),
        report_from_wips("beta", beta_findings),
    ];
    let clusters = build_clusters(&reports, &default_config());

    assert_eq!(
        clusters.len(),
        1,
        "two keyword-sharing bursts must merge into ONE cluster; got {:?}",
        clusters
    );
    assert_eq!(clusters[0].findings.len(), 4);
    assert_eq!(clusters[0].repos.len(), 2);
    assert!(clusters[0].repos.contains(&"alpha".to_string()));
    assert!(clusters[0].repos.contains(&"beta".to_string()));
}

#[test]
fn two_repos_sharing_only_one_keyword_do_not_merge() {
    // Each repo has 2 WIPs (enough to form a session). They share
    // exactly ONE meaningful keyword ("auth") — below the ≥2 threshold
    // that prevents noise-word transitive clustering.
    //
    // Expected: two separate cross-repo sessions, NOT one merged cluster.
    let base = 1_700_000_000;

    let alpha_findings = vec![
        wip("a111111111a1", base, "WIP auth alpha login"),
        wip("a222222222a2", base + 3_600, "WIP auth alpha session"),
    ];
    let beta_findings = vec![
        wip("b111111111b1", base, "WIP auth beta payment"),
        wip("b222222222b2", base + 3_600, "WIP auth beta checkout"),
    ];

    let reports = vec![
        report_from_wips("alpha", alpha_findings),
        report_from_wips("beta", beta_findings),
    ];
    let clusters = build_clusters(&reports, &default_config());

    assert_eq!(
        clusters.len(),
        2,
        "single shared keyword must NOT merge bursts; got {:?}",
        clusters
    );

    // Each cluster should stay within its own repo.
    for c in &clusters {
        assert_eq!(
            c.repos.len(),
            1,
            "each cluster should be single-repo; got {:?}",
            c.repos
        );
    }
}

// ---------------------------------------------------------------------------
// Sanity: singletons are dropped
// ---------------------------------------------------------------------------

#[test]
fn singleton_findings_are_not_reported_as_sessions() {
    // A single finding in a repo is not a "session" — it should NOT
    // produce a cluster.
    let f = wip("cafebabe0000", 1_700_000_000, "WIP only one");
    let reports = vec![report_from_wips("solo", vec![f])];
    let clusters = build_clusters(&reports, &default_config());

    assert!(
        clusters.is_empty(),
        "singleton clusters must be dropped; got {:?}",
        clusters
    );
}

// ---------------------------------------------------------------------------
// Sanity: clusters come back sorted by size descending
// ---------------------------------------------------------------------------

#[test]
fn clusters_sorted_by_size_descending() {
    let base = 1_700_000_000;

    // Repo with 3 findings in one burst.
    let big = report_from_wips(
        "big",
        vec![
            wip("b1111111b111", base, "WIP one"),
            wip("b2222222b222", base + 60, "WIP two"),
            wip("b3333333b333", base + 120, "WIP three"),
        ],
    );
    // Repo with 2 findings in one burst (unrelated keywords to avoid merge).
    let small = report_from_wips(
        "small",
        vec![
            wip("s1111111s111", base, "WIP xyz orange"),
            wip("s2222222s222", base + 60, "WIP xyz purple"),
        ],
    );

    let clusters = build_clusters(&[big, small], &default_config());
    assert_eq!(clusters.len(), 2);
    assert!(
        clusters[0].findings.len() >= clusters[1].findings.len(),
        "bigger cluster should come first; got sizes {} and {}",
        clusters[0].findings.len(),
        clusters[1].findings.len()
    );
    let _ = cluster_containing_sha; // silence dead_code if unused in a build
}
