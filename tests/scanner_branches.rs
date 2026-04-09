//! Integration tests for `scanner::branches::BranchScanner`.

mod common;

use common::TempRepo;
use gitwell::git::Repo;
use gitwell::scanner::branches::BranchScanner;
use gitwell::scanner::{Finding, Scanner};

// 60 days in seconds — well past the 30-day default stale threshold.
const SIXTY_DAYS: u64 = 60 * 86_400;

fn very_old_ts() -> i64 {
    // Fixed far-past timestamp: 2020-01-01T00:00:00Z. Any scan run with
    // a reasonable current-time clock will see this as ~5+ years old,
    // far past any stale_days threshold.
    1_577_836_800
}

fn scanner(stale_secs: u64) -> BranchScanner {
    BranchScanner {
        stale_secs,
        ignore_branches: Vec::new(),
    }
}

fn scanner_with_ignores(stale_secs: u64, ignores: Vec<String>) -> BranchScanner {
    BranchScanner {
        stale_secs,
        ignore_branches: ignores,
    }
}

fn find_branch<'a>(findings: &'a [Finding], name: &str) -> Option<&'a Finding> {
    findings.iter().find(|f| match f {
        Finding::StaleBranch { name: n, .. } => n == name,
        _ => false,
    })
}

#[test]
fn stale_unmerged_branch_is_found() {
    let tr = TempRepo::new("branches-stale");
    tr.branch("feature/old-work");
    tr.commit_file_at(
        "feat.txt",
        "unfinished\n",
        "WIP old feature",
        very_old_ts(),
    );
    tr.checkout("main");

    let repo = Repo::open(tr.path()).expect("open repo");
    let findings = scanner(SIXTY_DAYS).scan(&repo);

    let branch = find_branch(&findings, "feature/old-work");
    assert!(
        branch.is_some(),
        "expected feature/old-work to be reported as stale; got {:?}",
        findings
    );

    if let Some(Finding::StaleBranch {
        last_commit_message,
        ahead,
        behind,
        ..
    }) = branch
    {
        assert_eq!(last_commit_message, "WIP old feature");
        assert!(
            *ahead >= 1,
            "branch should be ahead of main (got {})",
            ahead
        );
        assert_eq!(*behind, 0, "main hasn't moved, branch shouldn't be behind");
    }
}

#[test]
fn branch_merged_into_default_is_not_found() {
    let tr = TempRepo::new("branches-merged");

    // Create branch, commit, merge into main.
    tr.branch("feature/done");
    tr.commit_file_at(
        "done.txt",
        "finished\n",
        "Feature complete",
        very_old_ts(),
    );
    tr.checkout("main");
    tr.merge("feature/done");

    let repo = Repo::open(tr.path()).expect("open repo");
    let findings = scanner(SIXTY_DAYS).scan(&repo);

    assert!(
        find_branch(&findings, "feature/done").is_none(),
        "merged branch should not be reported; got {:?}",
        findings
    );
}

#[test]
fn recent_branch_is_not_stale() {
    let tr = TempRepo::new("branches-recent");
    tr.branch("feature/fresh");
    // No committer date override → "now", well within the 30-day window.
    tr.commit_file("fresh.txt", "just today\n", "Fresh work");
    tr.checkout("main");

    let repo = Repo::open(tr.path()).expect("open repo");
    // Use a 30-day threshold — the commit is seconds old.
    let findings = scanner(30 * 86_400).scan(&repo);

    assert!(
        find_branch(&findings, "feature/fresh").is_none(),
        "recent branch should not be stale; got {:?}",
        findings
    );
}

#[test]
fn ignore_branches_glob_excludes_matches() {
    let tr = TempRepo::new("branches-ignore");

    // Stale but on the ignore list.
    tr.branch("dependabot/npm/lodash");
    tr.commit_file_at(
        "dep.txt",
        "bump\n",
        "chore(deps): bump lodash",
        very_old_ts(),
    );
    tr.checkout("main");

    // Stale and NOT on the ignore list.
    tr.branch("feature/keeper");
    tr.commit_file_at(
        "keep.txt",
        "real work\n",
        "WIP real work",
        very_old_ts(),
    );
    tr.checkout("main");

    let repo = Repo::open(tr.path()).expect("open repo");
    let ignores = vec!["dependabot/*".to_string()];
    let findings = scanner_with_ignores(SIXTY_DAYS, ignores).scan(&repo);

    assert!(
        find_branch(&findings, "dependabot/npm/lodash").is_none(),
        "dependabot/* should be filtered; got {:?}",
        findings
    );
    assert!(
        find_branch(&findings, "feature/keeper").is_some(),
        "unmatched stale branch should still be reported; got {:?}",
        findings
    );
}

#[test]
fn default_branch_itself_is_never_reported() {
    let tr = TempRepo::new("branches-default-excluded");
    // Age the initial commit so main itself would match a naive stale check.
    tr.empty_commit_at("old main commit", very_old_ts());

    let repo = Repo::open(tr.path()).expect("open repo");
    let findings = scanner(SIXTY_DAYS).scan(&repo);

    assert!(
        find_branch(&findings, "main").is_none(),
        "the default branch must never be reported as stale"
    );
}
