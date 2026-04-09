//! Generate a standalone markdown report at `.gitwell/report-YYYY-MM-DD.md`.
//!
//! The markdown report is the monthly-checkin / share-with-a-collaborator
//! format — it's designed to be readable on its own, no ANSI colors, no
//! terminal assumptions. Unlike the live terminal report, it also carries
//! the delta-since-last-scan summary and any triage history.
//!
//! The generator also appends a row to `.gitwell/history.json` so future
//! reports can show trends.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::cluster::Cluster;
use crate::narrative;
use crate::report::RepoReport;
use crate::scanner::Finding;
use crate::trends::{self, Delta, History};
use crate::triage_state::{DecisionKind, TriageState};
use crate::util;

/// Build the markdown report text and write it to
/// `<scan_path>/.gitwell/report-YYYY-MM-DD.md`. Returns the path written.
pub fn generate(
    scan_path: &Path,
    reports: &[RepoReport],
    clusters: &[Cluster],
) -> io::Result<PathBuf> {
    let now = util::now_unix();

    // ---- history bookkeeping ---------------------------------------------
    let finding_total: usize = reports.iter().map(|r| r.total()).sum();
    let current_entry = trends::current_entry(reports.len(), finding_total, clusters.len());

    let mut history = History::load(scan_path).unwrap_or_default();
    // The previous entry is whatever predates "now". We compute the delta
    // *before* appending current, so "before" means "strictly earlier".
    let previous_entry = history
        .previous_before(current_entry.timestamp)
        .cloned();

    // ---- build markdown body ---------------------------------------------
    let mut md = String::new();
    write_header(&mut md, &current_entry);
    if let Some(prev) = &previous_entry {
        write_delta(&mut md, prev, &current_entry);
    }
    write_executive_summary(&mut md, reports, clusters, now);
    write_sessions(&mut md, clusters, now);
    write_per_scanner(&mut md, reports, now);
    write_triage_history(&mut md, scan_path);

    // ---- write to disk ---------------------------------------------------
    let dir = scan_path.join(".gitwell");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("report-{}.md", current_entry.date));
    fs::write(&path, md)?;

    // ---- append to history.json ------------------------------------------
    history.append(current_entry);
    history.save(scan_path)?;

    Ok(path)
}

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

fn write_header(md: &mut String, cur: &trends::HistoryEntry) {
    md.push_str("# GitWell Report\n\n");
    md.push_str(&format!("**{}**\n\n", cur.date));
}

fn write_delta(md: &mut String, prev: &trends::HistoryEntry, cur: &trends::HistoryEntry) {
    let delta = Delta {
        previous: prev,
        current: cur,
    };
    md.push_str("## Since last scan\n\n");
    md.push_str(&format!("_{}_\n\n", delta.summary_line()));

    // Extra per-dimension line if the delta is non-trivial.
    let fdelta = cur.findings as i64 - prev.findings as i64;
    let sdelta = cur.sessions as i64 - prev.sessions as i64;
    let rdelta = cur.repos as i64 - prev.repos as i64;
    if fdelta != 0 || sdelta != 0 || rdelta != 0 {
        md.push_str(&format!(
            "- Findings: **{}** (net {}{})\n",
            cur.findings,
            if fdelta >= 0 { "+" } else { "" },
            fdelta
        ));
        md.push_str(&format!(
            "- Sessions: **{}** (net {}{})\n",
            cur.sessions,
            if sdelta >= 0 { "+" } else { "" },
            sdelta
        ));
        md.push_str(&format!(
            "- Repos: **{}** (net {}{})\n\n",
            cur.repos,
            if rdelta >= 0 { "+" } else { "" },
            rdelta
        ));
    }
}

fn write_executive_summary(
    md: &mut String,
    reports: &[RepoReport],
    clusters: &[Cluster],
    now: u64,
) {
    let finding_total: usize = reports.iter().map(|r| r.total()).sum();
    md.push_str("## Executive Summary\n\n");
    md.push_str(&format!("- **Repos scanned:** {}\n", reports.len()));
    md.push_str(&format!("- **Findings:** {}\n", finding_total));
    md.push_str(&format!("- **Sessions:** {}\n", clusters.len()));

    if let Some((oldest_ts, oldest_repo)) = oldest_finding(reports) {
        let age = now.saturating_sub(oldest_ts);
        md.push_str(&format!(
            "- **Oldest abandoned work:** {} (in `{}`)\n",
            human_age(age),
            oldest_repo
        ));
    }
    md.push('\n');
}

fn write_sessions(md: &mut String, clusters: &[Cluster], now: u64) {
    if clusters.is_empty() {
        return;
    }
    md.push_str("## Sessions of Abandoned Work\n\n");
    for (i, cluster) in clusters.iter().enumerate() {
        let narr = narrative::summarize(cluster, now);
        md.push_str(&format!(
            "### {}. [{}] {}\n\n",
            i + 1,
            cluster.findings.len(),
            cluster.label
        ));
        md.push_str(&format!("> {}\n\n", narr));
        md.push_str(&format!(
            "**Repos:** {}  \n",
            cluster.repos.join(", ")
        ));
        if !cluster.top_keywords.is_empty() {
            md.push_str(&format!(
                "**Keywords:** {}\n\n",
                cluster.top_keywords.join(", ")
            ));
        } else {
            md.push('\n');
        }
        for (repo, f) in &cluster.findings {
            md.push_str(&format!("- `{}` — {}\n", repo, format_finding_md(f, now)));
        }
        md.push('\n');
    }
}

fn write_per_scanner(md: &mut String, reports: &[RepoReport], now: u64) {
    md.push_str("## Per-Scanner Findings\n\n");
    for report in reports {
        if report.total() == 0 {
            continue;
        }
        md.push_str(&format!(
            "### `{}`\n\n_{}_\n\n",
            report.repo_name, report.repo_path
        ));
        for (section_name, findings) in &report.sections {
            if findings.is_empty() {
                continue;
            }
            md.push_str(&format!(
                "**{}** ({})\n\n",
                section_name,
                findings.len()
            ));
            for f in findings {
                md.push_str(&format!("- {}\n", format_finding_md(f, now)));
            }
            md.push('\n');
        }
    }
}

fn write_triage_history(md: &mut String, scan_path: &Path) {
    let state = match TriageState::load(scan_path) {
        Ok(s) => s,
        Err(_) => return,
    };
    if state.decisions.is_empty() {
        return;
    }
    md.push_str("## Triage History\n\n");
    md.push_str("| Date | Session | Decision | Status | Findings |\n");
    md.push_str("| --- | --- | --- | --- | --- |\n");
    for d in &state.decisions {
        let status = if d.executed { "executed" } else { "pending" };
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            d.decided_at,
            escape_table(&d.session_label),
            decision_label(d.decision),
            status,
            d.findings.len(),
        ));
    }
    md.push('\n');
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn oldest_finding(reports: &[RepoReport]) -> Option<(u64, String)> {
    let mut oldest: Option<(u64, String)> = None;
    for report in reports {
        for (_, findings) in &report.sections {
            for f in findings {
                let ts = f.timestamp();
                if ts == 0 {
                    continue;
                }
                match &oldest {
                    None => oldest = Some((ts, report.repo_name.clone())),
                    Some((o, _)) if ts < *o => {
                        oldest = Some((ts, report.repo_name.clone()))
                    }
                    _ => {}
                }
            }
        }
    }
    oldest
}

fn human_age(secs: u64) -> String {
    const DAY: u64 = 86_400;
    if secs < DAY {
        "less than a day ago".to_string()
    } else if secs < 30 * DAY {
        let d = secs / DAY;
        format!("{} day{} ago", d, if d == 1 { "" } else { "s" })
    } else if secs < 365 * DAY {
        let m = secs / (30 * DAY);
        format!("{} month{} ago", m, if m == 1 { "" } else { "s" })
    } else {
        let y = secs / (365 * DAY);
        format!("{} year{} ago", y, if y == 1 { "" } else { "s" })
    }
}

fn format_finding_md(f: &Finding, now: u64) -> String {
    let ts = f.timestamp();
    let age = if ts > 0 {
        human_age(now.saturating_sub(ts))
    } else {
        "unknown age".to_string()
    };
    match f {
        Finding::StaleBranch {
            name,
            last_commit_message,
            ahead,
            behind,
            ..
        } => format!(
            "**Stale branch** `{}` ({}) — {} (+{}/-{})",
            name,
            age,
            escape_md(last_commit_message),
            ahead,
            behind
        ),
        Finding::Stash {
            index,
            message,
            files_changed,
            insertions,
            deletions,
            ..
        } => format!(
            "**Stash** `{}` ({}) — {} ({} file{}, +{}/-{})",
            index,
            age,
            escape_md(message),
            files_changed,
            if *files_changed == 1 { "" } else { "s" },
            insertions,
            deletions
        ),
        Finding::WipCommit {
            sha,
            message,
            marker,
            ..
        } => format!(
            "**WIP** `{}` [{}] ({}) — {}",
            short_sha(sha),
            marker,
            age,
            escape_md(message)
        ),
        Finding::OrphanCommit { sha, message, .. } => format!(
            "**Orphan commit** `{}` ({}) — {}",
            short_sha(sha),
            age,
            escape_md(message)
        ),
        Finding::DormantRepo { path, .. } => format!("**Dormant** ({}) — `{}`", age, path),
    }
}

fn short_sha(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}

/// Escape characters that break inline markdown. Not a full sanitizer —
/// just enough to keep backticks and pipes from doing weird things.
fn escape_md(s: &str) -> String {
    s.replace('`', "'").replace('|', "\\|")
}

fn escape_table(s: &str) -> String {
    s.replace('|', "\\|")
}

fn decision_label(k: DecisionKind) -> &'static str {
    match k {
        DecisionKind::Resume => "resume",
        DecisionKind::Archive => "archive",
        DecisionKind::Delete => "delete",
        DecisionKind::Skip => "skip",
    }
}
