//! Integration tests for `scanner::wip::WipScanner`.

mod common;

use common::TempRepo;
use gitwell::git::Repo;
use gitwell::scanner::wip::WipScanner;
use gitwell::scanner::{Finding, Scanner};

fn find_by_message<'a>(findings: &'a [Finding], needle: &str) -> Option<&'a Finding> {
    findings.iter().find(|f| match f {
        Finding::WipCommit { message, .. } => message.contains(needle),
        _ => false,
    })
}

#[test]
fn wip_commit_message_is_detected() {
    let tr = TempRepo::new("wip-basic");
    tr.commit_file("a.txt", "content\n", "WIP: trying something");

    let repo = Repo::open(tr.path()).expect("open repo");
    let findings = WipScanner.scan(&repo);

    let hit = find_by_message(&findings, "trying something");
    assert!(hit.is_some(), "WIP: trying something should match; got {:?}", findings);
    if let Some(Finding::WipCommit { marker, .. }) = hit {
        // Either WIP or trying could match — both are markers.
        assert!(
            marker == "WIP" || marker == "trying",
            "expected WIP or trying marker, got {:?}",
            marker
        );
    }
}

#[test]
fn todo_marker_is_detected() {
    let tr = TempRepo::new("wip-todo");
    tr.commit_file("a.txt", "content\n", "TODO: finish auth module");

    let repo = Repo::open(tr.path()).expect("open repo");
    let findings = WipScanner.scan(&repo);

    let hit = find_by_message(&findings, "finish auth");
    assert!(hit.is_some(), "TODO: should be detected; got {:?}", findings);
    if let Some(Finding::WipCommit { marker, .. }) = hit {
        assert_eq!(marker, "TODO");
    }
}

#[test]
fn word_boundary_prevents_substring_matches() {
    // "Attempting" should NOT match "temp" even though "temp" is inside it.
    let tr = TempRepo::new("wip-word-boundary");
    tr.commit_file("a.txt", "content\n", "Attempting a fix for the parser");

    let repo = Repo::open(tr.path()).expect("open repo");
    let findings = WipScanner.scan(&repo);

    assert!(
        find_by_message(&findings, "Attempting").is_none(),
        "'Attempting' must not match 'temp' (word-boundary filter broken); got {:?}",
        findings
    );
}

#[test]
fn same_commit_reachable_from_two_branches_reported_once() {
    let tr = TempRepo::new("wip-dedup");

    // Make the WIP commit on main.
    tr.commit_file("w.txt", "stuff\n", "WIP: shared work");

    // Branch pointing at the same commit. `git log --all` will walk
    // every ref and visit this commit twice; the scanner must dedup.
    tr.branch("feature/copy");
    tr.checkout("main");

    let repo = Repo::open(tr.path()).expect("open repo");
    let findings = WipScanner.scan(&repo);

    let matches: Vec<&Finding> = findings
        .iter()
        .filter(|f| matches!(f, Finding::WipCommit { message, .. } if message.contains("shared work")))
        .collect();

    assert_eq!(
        matches.len(),
        1,
        "the same commit reachable from multiple refs must be reported once; got {:?}",
        findings
    );
}
