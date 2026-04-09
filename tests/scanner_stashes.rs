//! Integration tests for `scanner::stashes::StashScanner`.

mod common;

use common::TempRepo;
use gitwell::git::Repo;
use gitwell::scanner::stashes::StashScanner;
use gitwell::scanner::{Finding, Scanner};

#[test]
fn single_stash_is_captured_with_diff_stats() {
    let tr = TempRepo::new("stashes-one");
    tr.stash("draft.txt", "first line\nsecond line\n", "WIP draft work");

    let repo = Repo::open(tr.path()).expect("open repo");
    let findings = StashScanner.scan(&repo);

    assert_eq!(
        findings.len(),
        1,
        "expected exactly one stash; got {:?}",
        findings
    );

    match &findings[0] {
        Finding::Stash {
            index,
            sha,
            message,
            files_changed,
            insertions,
            deletions,
            ..
        } => {
            assert_eq!(index, "stash@{0}");
            assert!(
                !sha.is_empty(),
                "stash SHA must be captured for execute-time resolution"
            );
            assert_eq!(sha.len(), 40, "expected full 40-char SHA, got {}", sha);
            assert!(
                message.contains("WIP draft work"),
                "stash message should contain the user's text; got {:?}",
                message
            );
            assert_eq!(*files_changed, 1, "one file staged → one file in numstat");
            assert_eq!(*insertions, 2, "two lines added");
            assert_eq!(*deletions, 0);
        }
        other => panic!("expected Finding::Stash; got {:?}", other),
    }
}

#[test]
fn no_stashes_returns_empty() {
    let tr = TempRepo::new("stashes-none");

    let repo = Repo::open(tr.path()).expect("open repo");
    let findings = StashScanner.scan(&repo);

    assert!(
        findings.is_empty(),
        "repo with no stashes should yield no findings; got {:?}",
        findings
    );
}

#[test]
fn multiple_stashes_all_found_with_distinct_shas() {
    let tr = TempRepo::new("stashes-many");

    tr.stash("a.txt", "alpha\n", "WIP alpha work");
    tr.stash("b.txt", "beta line\nbeta line two\n", "WIP beta work");
    tr.stash("c.txt", "gamma\n", "WIP gamma work");

    let repo = Repo::open(tr.path()).expect("open repo");
    let findings = StashScanner.scan(&repo);

    assert_eq!(findings.len(), 3, "expected 3 stashes; got {:?}", findings);

    let mut shas: Vec<String> = Vec::new();
    for f in &findings {
        match f {
            Finding::Stash { sha, index, .. } => {
                assert!(
                    !sha.is_empty(),
                    "every stash should have a SHA (got {} with empty sha)",
                    index
                );
                shas.push(sha.clone());
            }
            other => panic!("expected Finding::Stash, got {:?}", other),
        }
    }
    // All distinct.
    shas.sort();
    shas.dedup();
    assert_eq!(shas.len(), 3, "stashes should have distinct SHAs");
}
