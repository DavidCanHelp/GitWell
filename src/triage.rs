//! Interactive triage walk-through.
//!
//! Renders each session one at a time and prompts the user for a single
//! keypress decision. The raw/`cbreak` terminal mode is set via `stty` —
//! we deliberately avoid the `libc` crate. An RAII guard (`RawMode`)
//! restores the terminal on normal exit.
//!
//! Decisions are written to `.gitwell/triage.json` incrementally, so even
//! if the process is killed mid-session the earlier decisions are safe.
//!
//! NOTE on Ctrl+C: because Rust's default SIGINT handler terminates the
//! process without running destructors, a Ctrl+C mid-triage will leave
//! the terminal in cbreak/-echo mode. Users should press **q** to quit
//! cleanly. If they do hit Ctrl+C, `stty sane` restores the terminal.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::Command;

use crate::cluster::Cluster;
use crate::narrative;
use crate::report::{BOLD, DIM, RESET};
use crate::scanner::Finding;
use crate::triage_state::{
    session_key_for, Decision, DecisionFinding, DecisionKind, TriageState,
};
use crate::util;

#[derive(Debug, Default)]
pub struct TriageSummary {
    pub resume: usize,
    pub archive: usize,
    pub delete: usize,
    pub skip: usize,
    pub already_decided: usize,
    pub quit_early: bool,
}

impl TriageSummary {
    pub fn total_decided(&self) -> usize {
        self.resume + self.archive + self.delete + self.skip
    }
}

pub fn run(
    scan_path: &Path,
    clusters: &[Cluster],
    repo_paths: &HashMap<String, String>,
) -> io::Result<TriageSummary> {
    let mut state = TriageState::load(scan_path).unwrap_or_default();
    let mut summary = TriageSummary::default();

    if clusters.is_empty() {
        println!("No sessions to triage. Your git house is in order.");
        return Ok(summary);
    }

    // Enter cbreak mode for single-keypress input. Dropped at function
    // exit (and thus the terminal restored) via RAII.
    let _raw = RawMode::enter()?;

    let total = clusters.len();
    let now = util::now_unix();

    for (i, cluster) in clusters.iter().enumerate() {
        let idx = i + 1;
        let key = session_key_for(cluster);

        if state.is_decided(&key) {
            println!();
            println!(
                "{dim}[{idx}/{total}] skipping {label} (already decided){reset}",
                idx = idx,
                total = total,
                label = cluster.label,
                dim = DIM,
                reset = RESET,
            );
            summary.already_decided += 1;
            continue;
        }

        render_cluster(cluster, idx, total, now);

        let kind = prompt_decision(&mut summary)?;
        match kind {
            None => {
                summary.quit_early = true;
                println!();
                println!(
                    "Quitting. Progress saved to {}.",
                    TriageState::state_path(scan_path).display()
                );
                break;
            }
            Some(k) => {
                let decision = Decision {
                    session_key: key,
                    session_label: cluster.label.clone(),
                    decision: k,
                    decided_at: util::format_iso8601(now),
                    findings: cluster
                        .findings
                        .iter()
                        .map(|(repo, f)| {
                            let path = repo_paths
                                .get(repo)
                                .cloned()
                                .unwrap_or_else(|| repo.clone());
                            DecisionFinding::from_finding(repo, &path, f)
                        })
                        .collect(),
                    executed: false,
                };
                state.decisions.push(decision);
                state.save(scan_path)?;
            }
        }
    }

    Ok(summary)
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_cluster(cluster: &Cluster, idx: usize, total: usize, now: u64) {
    let narrative = narrative::summarize(cluster, now);
    println!();
    println!(
        "{bold}[{idx}/{total}]{reset} {narrative}",
        idx = idx,
        total = total,
        narrative = narrative,
        bold = BOLD,
        reset = RESET,
    );
    println!(
        "  {dim}repos: {repos} · label: {label}{reset}",
        repos = cluster.repos.join(", "),
        label = cluster.label,
        dim = DIM,
        reset = RESET,
    );
    for (repo, f) in &cluster.findings {
        println!(
            "  {kind:<8} {dim}{repo:<16}{reset} {detail}",
            kind = kind_tag(f),
            repo = util::truncate(repo, 16),
            detail = format_detail(f),
            dim = DIM,
            reset = RESET,
        );
    }
}

fn kind_tag(f: &Finding) -> &'static str {
    match f {
        Finding::StaleBranch { .. } => "branch",
        Finding::Stash { .. } => "stash",
        Finding::WipCommit { .. } => "wip",
        Finding::OrphanCommit { .. } => "orphan",
        Finding::DormantRepo { .. } => "dormant",
    }
}

fn format_detail(f: &Finding) -> String {
    match f {
        Finding::StaleBranch {
            name,
            last_commit_message,
            ..
        } => format!("{} — {}", name, util::truncate(last_commit_message, 50)),
        Finding::Stash {
            index,
            message,
            files_changed,
            insertions,
            deletions,
            ..
        } => format!(
            "{} — {} ({} files, +{}/-{})",
            index,
            util::truncate(message, 40),
            files_changed,
            insertions,
            deletions
        ),
        Finding::WipCommit {
            sha,
            message,
            marker,
            ..
        } => format!(
            "{} [{}] {}",
            &sha[..sha.len().min(8)],
            marker,
            util::truncate(message, 40)
        ),
        Finding::OrphanCommit { sha, message, .. } => format!(
            "{} — {}",
            &sha[..sha.len().min(8)],
            util::truncate(message, 50)
        ),
        Finding::DormantRepo { path, .. } => path.clone(),
    }
}

// ---------------------------------------------------------------------------
// Prompt / key reading
// ---------------------------------------------------------------------------

/// Returns `Ok(Some(kind))` for a decision, `Ok(None)` for quit.
fn prompt_decision(summary: &mut TriageSummary) -> io::Result<Option<DecisionKind>> {
    loop {
        print!(
            "\n  {bold}[r]esume [a]rchive [d]elete [s]kip [q]uit:{reset} ",
            bold = BOLD,
            reset = RESET,
        );
        io::stdout().flush()?;

        let c = read_char()?;
        // Echo the char manually since we disabled echo.
        println!("{}", c);

        match c.to_ascii_lowercase() {
            'r' => {
                summary.resume += 1;
                return Ok(Some(DecisionKind::Resume));
            }
            'a' => {
                summary.archive += 1;
                return Ok(Some(DecisionKind::Archive));
            }
            'd' => {
                print!("  {bold}Are you sure? (y/n):{reset} ", bold = BOLD, reset = RESET);
                io::stdout().flush()?;
                let yn = read_char()?;
                println!("{}", yn);
                if yn.to_ascii_lowercase() == 'y' {
                    summary.delete += 1;
                    return Ok(Some(DecisionKind::Delete));
                } else {
                    println!("  {dim}cancelled{reset}", dim = DIM, reset = RESET);
                    continue;
                }
            }
            's' => {
                summary.skip += 1;
                return Ok(Some(DecisionKind::Skip));
            }
            'q' => return Ok(None),
            _ => {
                println!("  {dim}(use r, a, d, s, or q){reset}", dim = DIM, reset = RESET);
                continue;
            }
        }
    }
}

fn read_char() -> io::Result<char> {
    let mut buf = [0u8; 1];
    io::stdin().read_exact(&mut buf)?;
    Ok(buf[0] as char)
}

// ---------------------------------------------------------------------------
// RawMode — RAII wrapper around `stty cbreak -echo`
// ---------------------------------------------------------------------------

/// Put the controlling terminal into cbreak mode with echo disabled for
/// the lifetime of the guard. On drop, the original terminal state is
/// restored (or `stty sane` is applied as a fallback).
///
/// We shell out to `stty` rather than linking `libc` so GitWell stays
/// dependency-free. `stty -g` dumps a compact, stty-consumable snapshot
/// of the current settings that we hand back to `stty` on restore.
struct RawMode {
    saved: Option<String>,
}

impl RawMode {
    fn enter() -> io::Result<Self> {
        let out = Command::new("stty").arg("-g").output()?;
        let saved = if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        };

        let status = Command::new("stty").args(["cbreak", "-echo"]).status()?;
        if !status.success() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "failed to enter cbreak mode (is stdin a terminal?)",
            ));
        }

        Ok(RawMode { saved })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if let Some(saved) = self.saved.take() {
            let _ = Command::new("stty").arg(saved).status();
        } else {
            let _ = Command::new("stty").arg("sane").status();
        }
    }
}
