//! Integration tests for `scanner::dormant::DormantScanner`.

mod common;

use common::TempRepo;
use gitwell::git::Repo;
use gitwell::scanner::dormant::DormantScanner;
use gitwell::scanner::{Finding, Scanner};

// 6 months worth of seconds — matches the default threshold.
const SIX_MONTHS: u64 = 6 * 30 * 86_400;

fn scanner() -> DormantScanner {
    DormantScanner {
        dormant_secs: SIX_MONTHS,
    }
}

#[test]
fn repo_with_only_old_commits_is_dormant() {
    let tr = TempRepo::new("dormant-old");

    // Age the ONLY tip commit (the initial commit from TempRepo::new) by
    // rewriting with an old date. Easiest: `git commit --amend` can't
    // reset committer date cleanly, so just make a new empty commit with
    // an old date and reset --hard to it.
    tr.empty_commit_at("ancient commit", 1_577_836_800); // 2020-01-01

    let repo = Repo::open(tr.path()).expect("open repo");
    let findings = scanner().scan(&repo);

    assert_eq!(findings.len(), 1, "expected one dormant finding; got {:?}", findings);
    assert!(
        matches!(findings[0], Finding::DormantRepo { .. }),
        "expected Finding::DormantRepo; got {:?}",
        findings[0]
    );
}

#[test]
fn repo_with_recent_commit_is_not_dormant() {
    let tr = TempRepo::new("dormant-fresh");
    // TempRepo::new already made a commit with the current wall-clock
    // time, so the repo is trivially fresh.

    let repo = Repo::open(tr.path()).expect("open repo");
    let findings = scanner().scan(&repo);

    assert!(
        findings.is_empty(),
        "a repo with a recent commit must not be dormant; got {:?}",
        findings
    );
}

#[test]
fn dormant_threshold_respects_config() {
    let tr = TempRepo::new("dormant-threshold");
    tr.empty_commit_at("ancient commit", 1_577_836_800);

    let repo = Repo::open(tr.path()).expect("open repo");

    // With a ridiculously long threshold (100 years), nothing should
    // be dormant.
    let far_future = DormantScanner {
        dormant_secs: 100 * 365 * 86_400,
    };
    let findings = far_future.scan(&repo);
    assert!(
        findings.is_empty(),
        "100y threshold should exclude everything; got {:?}",
        findings
    );
}
