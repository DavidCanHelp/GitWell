//! Execute queued triage decisions.
//!
//! Reads `.gitwell/triage.json`, walks the non-executed decisions, and
//! performs the mapped git action per finding. Dry-run by default — pass
//! `--confirm` from main to actually run the destructive commands.
//!
//! Failures are non-fatal: branches that have already been deleted,
//! stashes whose indexes have shifted away, etc. are reported with a
//! warning and the rest of the queue keeps going.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::git::Repo;
use crate::report::{BOLD, DIM, GREEN, RED, RESET, YELLOW};
use crate::triage_state::{DecisionFinding, DecisionKind, TriageState};
use crate::util;

#[derive(Debug, Default)]
pub struct ExecuteSummary {
    pub actions_total: usize,
    pub succeeded: usize,
    pub skipped: usize,
    pub failed: usize,
    pub decisions_executed: usize,
}

pub fn run(scan_path: &Path, confirm: bool) -> io::Result<ExecuteSummary> {
    let mut state = TriageState::load(scan_path)?;
    let mut summary = ExecuteSummary::default();

    if state.decisions.is_empty() {
        println!("No triage decisions to execute. Run `gitwell triage` first.");
        return Ok(summary);
    }

    let mode_banner = if confirm {
        format!("{}EXECUTING{} (changes will be applied)", RED, RESET)
    } else {
        format!("{}DRY RUN{} — pass --confirm to execute", YELLOW, RESET)
    };
    println!();
    println!("{}", mode_banner);

    let archive_dir = TriageState::archive_dir(scan_path);

    for decision in state.decisions.iter_mut() {
        if decision.executed {
            continue;
        }

        println!();
        println!(
            "{bold}[{kind}]{reset} {label}",
            kind = decision.decision.as_str(),
            label = decision.session_label,
            bold = BOLD,
            reset = RESET,
        );

        // Skip decisions are instantly "done" with no work.
        if matches!(decision.decision, DecisionKind::Skip) {
            println!("  {dim}(skipped during triage — no action){reset}", dim = DIM, reset = RESET);
            if confirm {
                decision.executed = true;
                summary.decisions_executed += 1;
            }
            continue;
        }

        let mut all_ok = true;
        for finding in &decision.findings {
            summary.actions_total += 1;
            match perform_action(decision.decision, finding, confirm, &archive_dir) {
                ActionResult::Succeeded(msg) => {
                    println!("  {}✓{} {}", GREEN, RESET, msg);
                    summary.succeeded += 1;
                }
                ActionResult::NoOp(msg) => {
                    println!("  {}·{} {}", DIM, RESET, msg);
                    summary.skipped += 1;
                }
                ActionResult::Skipped(reason) => {
                    println!("  {}~{} {}", YELLOW, RESET, reason);
                    summary.skipped += 1;
                }
                ActionResult::Failed(err) => {
                    println!("  {}✗{} {}", RED, RESET, err);
                    summary.failed += 1;
                    all_ok = false;
                }
            }
        }

        if confirm && all_ok {
            decision.executed = true;
            summary.decisions_executed += 1;
        }
    }

    // Persist the updated `executed` flags.
    if confirm {
        state.save(scan_path)?;
    }

    Ok(summary)
}

// ---------------------------------------------------------------------------
// Per-finding action dispatch
// ---------------------------------------------------------------------------

enum ActionResult {
    /// Action ran successfully (or dry-run described it).
    Succeeded(String),
    /// Nothing to do for this (decision, finding kind) pair — e.g. resume
    /// on a WIP commit.
    NoOp(String),
    /// Something changed underneath us; not an error, just a warning.
    Skipped(String),
    /// The action was attempted and git rejected it.
    Failed(String),
}

fn perform_action(
    decision: DecisionKind,
    finding: &DecisionFinding,
    confirm: bool,
    archive_dir: &Path,
) -> ActionResult {
    let repo = match Repo::open(&finding.repo_path) {
        Ok(r) => r,
        Err(e) => {
            return ActionResult::Skipped(format!(
                "cannot open {}: {}",
                finding.repo_path, e
            ))
        }
    };

    match (decision, finding.kind.as_str()) {
        // ---------------- resume ----------------
        (DecisionKind::Resume, "stash") => resume_stash(&repo, finding, confirm),
        (DecisionKind::Resume, kind) => ActionResult::NoOp(format!(
            "resume is flag-only for {} ({} in {})",
            kind, finding.detail, finding.repo
        )),

        // ---------------- archive ----------------
        (DecisionKind::Archive, "stale_branch") => archive_branch(&repo, finding, confirm),
        (DecisionKind::Archive, "stash") => archive_stash(&repo, finding, confirm, archive_dir),
        (DecisionKind::Archive, kind) => ActionResult::NoOp(format!(
            "archive is a no-op for {} ({} in {})",
            kind, finding.detail, finding.repo
        )),

        // ---------------- delete ----------------
        (DecisionKind::Delete, "stale_branch") => delete_branch(&repo, finding, confirm),
        (DecisionKind::Delete, "stash") => delete_stash(&repo, finding, confirm),
        (DecisionKind::Delete, kind) => ActionResult::NoOp(format!(
            "delete is a no-op for {} ({} in {})",
            kind, finding.detail, finding.repo
        )),

        // ---------------- skip ----------------
        (DecisionKind::Skip, _) => ActionResult::NoOp("skipped".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Individual action implementations
// ---------------------------------------------------------------------------

fn resume_stash(repo: &Repo, finding: &DecisionFinding, confirm: bool) -> ActionResult {
    let Some(stash_ref) = resolve_stash_ref(repo, finding) else {
        return ActionResult::Skipped(format!("stash gone in {}", finding.repo));
    };

    if !confirm {
        return ActionResult::Succeeded(format!(
            "would `git stash apply {}` in {}",
            stash_ref, finding.repo
        ));
    }
    match repo.apply_stash(&stash_ref) {
        Ok(_) => ActionResult::Succeeded(format!(
            "applied {} in {}",
            stash_ref, finding.repo
        )),
        Err(e) => ActionResult::Failed(format!("stash apply failed: {}", e)),
    }
}

fn archive_branch(repo: &Repo, finding: &DecisionFinding, confirm: bool) -> ActionResult {
    let branch = &finding.detail;
    let tag = format!("archive/{}", branch);

    if !repo.branch_exists(branch) {
        return ActionResult::Skipped(format!(
            "branch {} already gone from {}",
            branch, finding.repo
        ));
    }

    if !confirm {
        return ActionResult::Succeeded(format!(
            "would tag {} and delete branch {} in {}",
            tag, branch, finding.repo
        ));
    }

    if let Err(e) = repo.create_tag(&tag, branch) {
        return ActionResult::Failed(format!("tag {} failed: {}", tag, e));
    }
    match repo.delete_branch(branch) {
        Ok(_) => ActionResult::Succeeded(format!(
            "tagged {} and deleted {} in {}",
            tag, branch, finding.repo
        )),
        Err(e) => ActionResult::Failed(format!("delete failed: {}", e)),
    }
}

fn archive_stash(
    repo: &Repo,
    finding: &DecisionFinding,
    confirm: bool,
    archive_dir: &Path,
) -> ActionResult {
    let Some(stash_ref) = resolve_stash_ref(repo, finding) else {
        return ActionResult::Skipped(format!("stash gone in {}", finding.repo));
    };

    if !confirm {
        return ActionResult::Succeeded(format!(
            "would archive {} and drop in {}",
            stash_ref, finding.repo
        ));
    }

    let diff = match repo.stash_diff(&stash_ref) {
        Ok(d) => d,
        Err(e) => return ActionResult::Failed(format!("stash diff failed: {}", e)),
    };

    if let Err(e) = fs::create_dir_all(archive_dir) {
        return ActionResult::Failed(format!("mkdir {}: {}", archive_dir.display(), e));
    }

    let path = archive_path(archive_dir, finding);
    if let Err(e) = fs::write(&path, diff) {
        return ActionResult::Failed(format!("write {}: {}", path.display(), e));
    }

    match repo.drop_stash(&stash_ref) {
        Ok(_) => ActionResult::Succeeded(format!(
            "archived to {} and dropped from {}",
            path.display(),
            finding.repo
        )),
        Err(e) => ActionResult::Failed(format!("stash drop failed: {}", e)),
    }
}

fn delete_branch(repo: &Repo, finding: &DecisionFinding, confirm: bool) -> ActionResult {
    let branch = &finding.detail;
    if !repo.branch_exists(branch) {
        return ActionResult::Skipped(format!(
            "branch {} already gone from {}",
            branch, finding.repo
        ));
    }
    if !confirm {
        return ActionResult::Succeeded(format!(
            "would delete branch {} in {}",
            branch, finding.repo
        ));
    }
    match repo.delete_branch(branch) {
        Ok(_) => ActionResult::Succeeded(format!(
            "deleted {} in {}",
            branch, finding.repo
        )),
        Err(e) => ActionResult::Failed(format!("delete failed: {}", e)),
    }
}

fn delete_stash(repo: &Repo, finding: &DecisionFinding, confirm: bool) -> ActionResult {
    let Some(stash_ref) = resolve_stash_ref(repo, finding) else {
        return ActionResult::Skipped(format!("stash gone in {}", finding.repo));
    };
    if !confirm {
        return ActionResult::Succeeded(format!(
            "would drop {} in {}",
            stash_ref, finding.repo
        ));
    }
    match repo.drop_stash(&stash_ref) {
        Ok(_) => ActionResult::Succeeded(format!(
            "dropped {} in {}",
            stash_ref, finding.repo
        )),
        Err(e) => ActionResult::Failed(format!("stash drop failed: {}", e)),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the persisted finding to the **current** `stash@{N}` ref.
/// Prefer the stored commit SHA (immune to index shifts); fall back to
/// parsing the stash ref out of the detail string.
fn resolve_stash_ref(repo: &Repo, finding: &DecisionFinding) -> Option<String> {
    if let Some(sha) = &finding.stash_sha {
        if let Some(r) = repo.find_stash_by_sha(sha) {
            return Some(r);
        }
        return None;
    }
    // Legacy fallback: "stash@{0}: WIP on main" -> "stash@{0}".
    finding.detail.split(':').next().map(|s| s.trim().to_string())
}

fn archive_path(archive_dir: &Path, finding: &DecisionFinding) -> PathBuf {
    let date = util::format_iso8601(util::now_unix());
    let date = &date[..10]; // YYYY-MM-DD
    let summary: String = finding
        .detail
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(40)
        .collect();
    let summary = summary.trim_matches('-');
    let summary = if summary.is_empty() { "stash" } else { summary };
    archive_dir.join(format!("{}-{}-{}.patch", date, finding.repo, summary))
}
