//! Format scan results for humans (ANSI terminal) or machines (JSON).

use crate::cluster::Cluster;
use crate::narrative;
use crate::scanner::Finding;
use crate::util;

// ANSI SGR escapes — defined inline so we don't pull in a color crate.
pub const RED: &str = "\x1b[31m";
pub const YELLOW: &str = "\x1b[33m";
pub const GREEN: &str = "\x1b[32m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const RESET: &str = "\x1b[0m";

const DAY: u64 = 86_400;

/// A scanned repo and its findings, grouped by section name.
pub struct RepoReport {
    pub repo_name: String,
    pub repo_path: String,
    pub sections: Vec<(String, Vec<Finding>)>,
}

impl RepoReport {
    pub fn total(&self) -> usize {
        self.sections.iter().map(|(_, f)| f.len()).sum()
    }
}

// ---------------------------------------------------------------------------
// Terminal output
// ---------------------------------------------------------------------------

pub fn print_terminal(reports: &[RepoReport], clusters: &[Cluster]) {
    let now = util::now_unix();

    let repo_count = reports.len();
    let finding_total: usize = reports.iter().map(|r| r.total()).sum();

    println!();
    println!(
        "{bold}GitWell{reset} — scanned {} repo{} · {} finding{} · {} session{}",
        repo_count,
        if repo_count == 1 { "" } else { "s" },
        finding_total,
        if finding_total == 1 { "" } else { "s" },
        clusters.len(),
        if clusters.len() == 1 { "" } else { "s" },
        bold = BOLD,
        reset = RESET,
    );

    if !clusters.is_empty() {
        print_sessions(clusters, now);
    }

    for report in reports {
        let total = report.total();
        println!();
        println!(
            "{bold}{name}{reset} {dim}{path}{reset}",
            name = report.repo_name,
            path = report.repo_path,
            bold = BOLD,
            dim = DIM,
            reset = RESET,
        );
        if total == 0 {
            println!("  {dim}(all clear){reset}", dim = DIM, reset = RESET);
            continue;
        }

        for (section_name, findings) in &report.sections {
            if findings.is_empty() {
                continue;
            }
            println!();
            println!(
                "  {bold}{name}{reset} ({count})",
                name = section_name,
                count = findings.len(),
                bold = BOLD,
                reset = RESET,
            );
            for finding in findings {
                println!("    {}", format_finding(finding, now));
            }
        }
    }
    println!();
}

// ---------------------------------------------------------------------------
// Sessions section
// ---------------------------------------------------------------------------

fn print_sessions(clusters: &[Cluster], now: u64) {
    println!();
    println!("{}Sessions of Abandoned Work{}", BOLD, RESET);

    for cluster in clusters {
        let narrative = narrative::summarize(cluster, now);
        println!();
        println!(
            "  {bold}[{count}]{reset} {narr}",
            count = cluster.findings.len(),
            narr = narrative,
            bold = BOLD,
            reset = RESET,
        );
        println!(
            "      {dim}repos: {repos} · label: {label}{reset}",
            repos = cluster.repos.join(", "),
            label = cluster.label,
            dim = DIM,
            reset = RESET,
        );
        for (repo, f) in &cluster.findings {
            println!(
                "      {dim}{repo}{reset}  {line}",
                repo = repo,
                line = format_finding_short(f, now),
                dim = DIM,
                reset = RESET,
            );
        }
    }
}

fn format_finding_short(f: &Finding, now: u64) -> String {
    let ts = f.timestamp();
    let age = now.saturating_sub(ts);
    let age_str = colored_age(age);

    match f {
        Finding::StaleBranch { name, .. } => {
            format!("[{}] branch {}", age_str, name)
        }
        Finding::Stash { index, files_changed, .. } => {
            format!(
                "[{}] {} ({} file{})",
                age_str,
                index,
                files_changed,
                if *files_changed == 1 { "" } else { "s" }
            )
        }
        Finding::OrphanCommit { sha, message, .. } => {
            format!(
                "[{}] orphan {} — {}",
                age_str,
                short_sha(sha),
                util::truncate(message, 45)
            )
        }
        Finding::WipCommit {
            sha,
            message,
            marker,
            ..
        } => {
            format!(
                "[{}] {} [{}] {}",
                age_str,
                short_sha(sha),
                marker,
                util::truncate(message, 40)
            )
        }
        Finding::DormantRepo { .. } => format!("[{}] dormant", age_str),
    }
}

fn age_color(age_secs: u64) -> &'static str {
    if age_secs > 365 * DAY {
        RED
    } else if age_secs > 180 * DAY {
        YELLOW
    } else if age_secs > 30 * DAY {
        GREEN
    } else {
        ""
    }
}

fn human_age(secs: u64) -> String {
    if secs < DAY {
        format!("{}h", secs / 3600)
    } else if secs < 30 * DAY {
        format!("{}d", secs / DAY)
    } else if secs < 365 * DAY {
        format!("{}mo", secs / (30 * DAY))
    } else {
        format!("{}y", secs / (365 * DAY))
    }
}

fn colored_age(age: u64) -> String {
    let color = age_color(age);
    let text = human_age(age);
    if color.is_empty() {
        text
    } else {
        format!("{}{}{}", color, text, RESET)
    }
}

fn format_finding(f: &Finding, now: u64) -> String {
    let ts = f.timestamp();
    let age = now.saturating_sub(ts);
    let age_str = colored_age(age);

    match f {
        Finding::StaleBranch {
            name,
            last_commit_message,
            ahead,
            behind,
            ..
        } => format!(
            "[{}] {} — {} (+{}/-{})",
            age_str,
            name,
            util::truncate(last_commit_message, 60),
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
            "[{}] {} — {} ({} file{}, +{}/-{})",
            age_str,
            index,
            util::truncate(message, 60),
            files_changed,
            if *files_changed == 1 { "" } else { "s" },
            insertions,
            deletions
        ),
        Finding::OrphanCommit { sha, message, .. } => format!(
            "[{}] {} — {}",
            age_str,
            short_sha(sha),
            util::truncate(message, 60)
        ),
        Finding::WipCommit {
            sha,
            message,
            marker,
            ..
        } => format!(
            "[{}] {} [{}] {}",
            age_str,
            short_sha(sha),
            marker,
            util::truncate(message, 60)
        ),
        Finding::DormantRepo { path, .. } => format!("[{}] {}", age_str, path),
    }
}

fn short_sha(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}

// ---------------------------------------------------------------------------
// JSON output — handwritten so we stay dependency-free.
// ---------------------------------------------------------------------------

pub fn print_json(reports: &[RepoReport], clusters: &[Cluster]) {
    let now = util::now_unix();
    let mut out = String::from("{\n");

    // -- sessions/clusters --------------------------------------------------
    out.push_str("  \"sessions\": [");
    if clusters.is_empty() {
        out.push_str("],\n");
    } else {
        out.push('\n');
        for (i, cluster) in clusters.iter().enumerate() {
            out.push_str(&cluster_json(cluster, now));
            if i + 1 < clusters.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("  ],\n");
    }

    // -- per-repo reports ---------------------------------------------------
    out.push_str("  \"repos\": [\n");
    for (i, report) in reports.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!("      \"repo\": {},\n", json_str(&report.repo_name)));
        out.push_str(&format!("      \"path\": {},\n", json_str(&report.repo_path)));
        out.push_str("      \"sections\": {\n");
        for (j, (name, findings)) in report.sections.iter().enumerate() {
            out.push_str(&format!("        {}: [", json_str(name)));
            if findings.is_empty() {
                out.push(']');
            } else {
                out.push('\n');
                for (k, f) in findings.iter().enumerate() {
                    out.push_str(&format!("          {}", finding_json(f)));
                    if k + 1 < findings.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str("        ]");
            }
            if j + 1 < report.sections.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("      }\n");
        out.push_str("    }");
        if i + 1 < reports.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    print!("{}", out);
}

fn cluster_json(cluster: &Cluster, now: u64) -> String {
    let mut s = String::from("    {\n");
    s.push_str(&format!("      \"label\": {},\n", json_str(&cluster.label)));
    s.push_str(&format!(
        "      \"narrative\": {},\n",
        json_str(&narrative::summarize(cluster, now))
    ));
    s.push_str(&format!("      \"start_ts\": {},\n", cluster.start_ts));
    s.push_str(&format!("      \"end_ts\": {},\n", cluster.end_ts));
    s.push_str("      \"repos\": [");
    for (i, r) in cluster.repos.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&json_str(r));
    }
    s.push_str("],\n");
    s.push_str("      \"top_keywords\": [");
    for (i, k) in cluster.top_keywords.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&json_str(k));
    }
    s.push_str("],\n");
    s.push_str("      \"findings\": [");
    if cluster.findings.is_empty() {
        s.push(']');
    } else {
        s.push('\n');
        for (i, (repo, f)) in cluster.findings.iter().enumerate() {
            s.push_str(&format!(
                "        {{\"repo\": {}, \"finding\": {}}}",
                json_str(repo),
                finding_json(f)
            ));
            if i + 1 < cluster.findings.len() {
                s.push(',');
            }
            s.push('\n');
        }
        s.push_str("      ]");
    }
    s.push_str("\n    }");
    s
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn finding_json(f: &Finding) -> String {
    match f {
        Finding::StaleBranch {
            name,
            last_commit_ts,
            last_commit_message,
            ahead,
            behind,
        } => format!(
            "{{\"type\":\"stale_branch\",\"name\":{},\"last_commit_ts\":{},\"message\":{},\"ahead\":{},\"behind\":{}}}",
            json_str(name),
            last_commit_ts,
            json_str(last_commit_message),
            ahead,
            behind
        ),
        Finding::Stash {
            index,
            sha,
            ts,
            message,
            files_changed,
            insertions,
            deletions,
        } => format!(
            "{{\"type\":\"stash\",\"index\":{},\"sha\":{},\"ts\":{},\"message\":{},\"files_changed\":{},\"insertions\":{},\"deletions\":{}}}",
            json_str(index),
            json_str(sha),
            ts,
            json_str(message),
            files_changed,
            insertions,
            deletions
        ),
        Finding::OrphanCommit { sha, ts, message } => format!(
            "{{\"type\":\"orphan_commit\",\"sha\":{},\"ts\":{},\"message\":{}}}",
            json_str(sha),
            ts,
            json_str(message)
        ),
        Finding::WipCommit {
            sha,
            ts,
            message,
            marker,
        } => format!(
            "{{\"type\":\"wip_commit\",\"sha\":{},\"ts\":{},\"message\":{},\"marker\":{}}}",
            json_str(sha),
            ts,
            json_str(message),
            json_str(marker)
        ),
        Finding::DormantRepo {
            path,
            last_activity_ts,
        } => format!(
            "{{\"type\":\"dormant_repo\",\"path\":{},\"last_activity_ts\":{}}}",
            json_str(path),
            last_activity_ts
        ),
    }
}
