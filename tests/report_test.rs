//! Integration tests for markdown report generation and trend tracking.

mod common;

use common::unique_path;
use gitwell::cluster::Cluster;
use gitwell::report::RepoReport;
use gitwell::report_md;
use gitwell::scanner::Finding;
use gitwell::trends::{self, Delta, History, HistoryEntry};
use std::fs;

fn sample_reports() -> (Vec<RepoReport>, Vec<Cluster>) {
    // Two WIPs in one repo, a hand-constructed cluster around them.
    // No need to run the actual clusterer here — the report generator
    // takes clusters as input.
    let wip_a = Finding::WipCommit {
        sha: "aaaaaaaa00000000".into(),
        ts: 1_577_836_800,
        message: "WIP auth one".into(),
        marker: "WIP".into(),
    };
    let wip_b = Finding::WipCommit {
        sha: "bbbbbbbb00000000".into(),
        ts: 1_577_840_400,
        message: "WIP auth two".into(),
        marker: "WIP".into(),
    };

    let report = RepoReport {
        repo_name: "alpha".into(),
        repo_path: "/tmp/alpha".into(),
        sections: vec![("WIP Markers".into(), vec![wip_a.clone(), wip_b.clone()])],
    };

    let cluster = Cluster {
        label: "auth".into(),
        start_ts: 1_577_836_800,
        end_ts: 1_577_840_400,
        repos: vec!["alpha".into()],
        findings: vec![
            ("alpha".into(), wip_a),
            ("alpha".into(), wip_b),
        ],
        top_keywords: vec!["auth".into()],
    };

    (vec![report], vec![cluster])
}

#[test]
fn markdown_report_contains_expected_sections() {
    let dir = unique_path("report-sections");
    fs::create_dir_all(&dir).unwrap();

    let (reports, clusters) = sample_reports();
    let path = report_md::generate(&dir, &reports, &clusters).expect("generate report");
    assert!(path.exists(), "report file should exist at {}", path.display());

    let md = fs::read_to_string(&path).expect("read report");

    assert!(md.contains("# GitWell Report"), "missing title: {}", md);
    assert!(
        md.contains("## Executive Summary"),
        "missing Executive Summary: {}",
        md
    );
    assert!(
        md.contains("**Repos scanned:** 1"),
        "executive summary should show 1 repo; got {}",
        md
    );
    assert!(
        md.contains("**Findings:** 2"),
        "executive summary should show 2 findings; got {}",
        md
    );
    assert!(
        md.contains("## Sessions of Abandoned Work"),
        "missing sessions section"
    );
    assert!(md.contains("auth"), "cluster label should appear");
    assert!(
        md.contains("## Per-Scanner Findings"),
        "missing per-scanner section"
    );
    assert!(md.contains("alpha"), "repo name should appear");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn history_json_is_appended_on_report() {
    let dir = unique_path("report-history");
    fs::create_dir_all(&dir).unwrap();

    // First report — no history yet.
    let (reports, clusters) = sample_reports();
    report_md::generate(&dir, &reports, &clusters).expect("first report");

    let history_path = History::path(&dir);
    assert!(history_path.exists(), "history.json should be created");

    let h1 = History::load(&dir).expect("load history after first report");
    assert_eq!(h1.entries.len(), 1, "one entry after first report");
    assert_eq!(h1.entries[0].repos, 1);
    assert_eq!(h1.entries[0].findings, 2);
    assert_eq!(h1.entries[0].sessions, 1);

    // Second report — history should gain another entry.
    report_md::generate(&dir, &reports, &clusters).expect("second report");
    let h2 = History::load(&dir).expect("load history after second report");
    assert_eq!(h2.entries.len(), 2, "two entries after second report");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn trend_delta_computes_net_change_between_two_entries() {
    let prev = HistoryEntry {
        date: "2026-04-07".into(),
        timestamp: 1_775_580_000,
        repos: 28,
        findings: 227,
        sessions: 32,
    };
    let cur = HistoryEntry {
        date: "2026-04-09".into(),
        timestamp: 1_775_753_000, // ~2 days later
        repos: 28,
        findings: 213,
        sessions: 29,
    };

    let delta = Delta {
        previous: &prev,
        current: &cur,
    };
    let summary = delta.summary_line();

    assert!(
        summary.contains("227 → 213"),
        "summary should show old → new findings; got {:?}",
        summary
    );
    assert!(summary.contains("(-14)"), "findings delta; got {:?}", summary);
    assert!(
        summary.contains("32 → 29"),
        "summary should show old → new sessions; got {:?}",
        summary
    );
    assert!(summary.contains("(-3)"), "sessions delta; got {:?}", summary);
}

#[test]
fn history_previous_before_picks_most_recent_earlier_entry() {
    let mut h = History::default();
    h.append(trends::current_entry(1, 10, 1));
    // Manually construct entries with known timestamps to avoid clock.
    let mut h = History::default();
    for (ts, findings) in [(100u64, 50usize), (200, 75), (300, 100), (400, 125)] {
        h.append(HistoryEntry {
            date: format!("ts-{}", ts),
            timestamp: ts,
            repos: 1,
            findings,
            sessions: 1,
        });
    }

    let prev = h.previous_before(300).expect("should find previous");
    assert_eq!(prev.timestamp, 200);
    assert_eq!(prev.findings, 75);
}
