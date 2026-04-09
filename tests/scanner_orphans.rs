//! Integration tests for `scanner::orphans::OrphanScanner`.
//!
//! Orphans are commits that still live in the object store (reachable
//! from the reflog) but no branch or tag points at them anymore —
//! typically the victims of `git reset --hard`, abandoned rebases, etc.

mod common;

use common::TempRepo;
use gitwell::git::Repo;
use gitwell::scanner::orphans::OrphanScanner;
use gitwell::scanner::{Finding, Scanner};

fn find_by_sha<'a>(findings: &'a [Finding], target: &str) -> Option<&'a Finding> {
    findings.iter().find(|f| match f {
        Finding::OrphanCommit { sha, .. } => sha == target,
        _ => false,
    })
}

#[test]
fn reset_hard_leaves_an_orphan_in_the_reflog() {
    let tr = TempRepo::new("orphans-reset");

    // Commit something on main.
    tr.commit_file("work.txt", "important progress\n", "feat: real work");
    let orphan_sha = tr.rev_parse("HEAD");
    assert_eq!(orphan_sha.len(), 40);

    // Undo it with reset --hard. The commit is no longer reachable
    // from main, but stays in the reflog.
    tr.reset_hard("HEAD~1");

    let repo = Repo::open(tr.path()).expect("open repo");
    let findings = OrphanScanner.scan(&repo);

    let hit = find_by_sha(&findings, &orphan_sha);
    assert!(
        hit.is_some(),
        "expected orphan {} to be found; got {:?}",
        orphan_sha,
        findings
    );

    if let Some(Finding::OrphanCommit { message, .. }) = hit {
        assert_eq!(message, "feat: real work");
    }
}

#[test]
fn reachable_commits_are_not_reported_as_orphans() {
    let tr = TempRepo::new("orphans-reachable");

    tr.commit_file("a.txt", "a\n", "commit A");
    let reachable = tr.rev_parse("HEAD");
    tr.commit_file("b.txt", "b\n", "commit B");

    let repo = Repo::open(tr.path()).expect("open repo");
    let findings = OrphanScanner.scan(&repo);

    assert!(
        find_by_sha(&findings, &reachable).is_none(),
        "commits reachable from HEAD must not be reported as orphans; got {:?}",
        findings
    );
}
