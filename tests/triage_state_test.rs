//! Integration tests for triage state persistence.

mod common;

use common::unique_path;
use gitwell::cluster::Cluster;
use gitwell::scanner::Finding;
use gitwell::triage_state::{
    session_key_for, Decision, DecisionFinding, DecisionKind, TriageState,
};
use std::fs;

fn sample_cluster(label: &str, start_ts: u64) -> Cluster {
    Cluster {
        label: label.to_string(),
        start_ts,
        end_ts: start_ts + 3_600,
        repos: vec!["sample".to_string()],
        findings: vec![(
            "sample".to_string(),
            Finding::WipCommit {
                sha: "cafebabe00000000".to_string(),
                ts: start_ts,
                message: "WIP sample".to_string(),
                marker: "WIP".to_string(),
            },
        )],
        top_keywords: vec!["sample".to_string()],
    }
}

#[test]
fn save_then_load_round_trips() {
    let dir = unique_path("triage-roundtrip");
    fs::create_dir_all(&dir).unwrap();

    let state = TriageState {
        decisions: vec![Decision {
            session_key: "auth:1700000000".to_string(),
            session_label: "auth".to_string(),
            decision: DecisionKind::Archive,
            decided_at: "2026-04-09T10:30:00Z".to_string(),
            findings: vec![DecisionFinding {
                repo: "myapp".to_string(),
                repo_path: "/tmp/myapp".to_string(),
                kind: "stale_branch".to_string(),
                detail: "feature/auth-v2".to_string(),
                stash_sha: None,
            }],
            executed: false,
        }],
    };

    state.save(&dir).expect("save should succeed");

    let reloaded = TriageState::load(&dir).expect("load should succeed");
    assert_eq!(reloaded.decisions.len(), 1);
    assert_eq!(reloaded.decisions[0].session_key, "auth:1700000000");
    assert_eq!(reloaded.decisions[0].session_label, "auth");
    assert_eq!(reloaded.decisions[0].decision, DecisionKind::Archive);
    assert_eq!(reloaded.decisions[0].findings.len(), 1);
    assert_eq!(reloaded.decisions[0].findings[0].detail, "feature/auth-v2");
    assert_eq!(reloaded.decisions[0].executed, false);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn is_decided_matches_by_session_key() {
    let dir = unique_path("triage-is-decided");
    fs::create_dir_all(&dir).unwrap();

    let cluster = sample_cluster("sample", 1_700_000_000);
    let key = session_key_for(&cluster);

    // Fresh state — not decided yet.
    let fresh = TriageState::load(&dir).unwrap();
    assert!(!fresh.is_decided(&key));

    // Record a decision and save.
    let state = TriageState {
        decisions: vec![Decision {
            session_key: key.clone(),
            session_label: cluster.label.clone(),
            decision: DecisionKind::Skip,
            decided_at: "2026-04-09T10:30:00Z".to_string(),
            findings: Vec::new(),
            executed: false,
        }],
    };
    state.save(&dir).unwrap();

    // Reload — same key now decided.
    let reloaded = TriageState::load(&dir).unwrap();
    assert!(
        reloaded.is_decided(&key),
        "re-loaded state should recognize the key as decided"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reset_removes_the_state_file() {
    let dir = unique_path("triage-reset");
    fs::create_dir_all(&dir).unwrap();

    let state = TriageState {
        decisions: vec![Decision {
            session_key: "k:0".to_string(),
            session_label: "k".to_string(),
            decision: DecisionKind::Skip,
            decided_at: "2026-04-09T10:30:00Z".to_string(),
            findings: Vec::new(),
            executed: false,
        }],
    };
    state.save(&dir).unwrap();

    assert!(
        TriageState::state_path(&dir).exists(),
        "state file must exist before reset"
    );

    let removed = TriageState::reset(&dir).unwrap();
    assert!(removed, "reset should report true when a file was removed");
    assert!(
        !TriageState::state_path(&dir).exists(),
        "state file should be gone after reset"
    );

    // Idempotent: reset on an already-cleared dir is a no-op.
    let removed2 = TriageState::reset(&dir).unwrap();
    assert!(!removed2);

    let _ = fs::remove_dir_all(&dir);
}
