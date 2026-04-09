//! GitWell — surface abandoned work in git repositories.
//!
//! The CLI is a thin driver over the `gitwell` library crate. The
//! modules (`cluster`, `config`, `scanner`, …) live in `src/lib.rs` and
//! are re-exported from there so integration tests in `tests/` can
//! import them as part of the public API.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use gitwell::config::{self, Config};
use gitwell::git::{is_repo_root, Repo};
use gitwell::report::{self, RepoReport};
use gitwell::triage_state::TriageState;
use gitwell::{cluster, execute, hook, report_md, scanner, triage, util};

enum Command {
    Scan {
        path: PathBuf,
        json: bool,
        quiet: bool,
    },
    Triage {
        path: PathBuf,
        reset: bool,
    },
    Execute {
        path: PathBuf,
        confirm: bool,
    },
    Report {
        path: PathBuf,
    },
    Hook {
        action: HookAction,
        path: PathBuf,
    },
    Help,
}

#[derive(Debug, Clone, Copy)]
enum HookAction {
    Install,
    Remove,
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let command = match parse_args(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gitwell: {}", e);
            print_help();
            return ExitCode::from(2);
        }
    };

    match command {
        Command::Help => {
            print_help();
            ExitCode::SUCCESS
        }
        Command::Scan { path, json, quiet } => run_scan(path, json, quiet),
        Command::Triage { path, reset } => run_triage(path, reset),
        Command::Execute { path, confirm } => run_execute(path, confirm),
        Command::Report { path } => run_report(path),
        Command::Hook { action, path } => run_hook(action, path),
    }
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

fn parse_args(args: &[String]) -> Result<Command, String> {
    let mut subcommand: Option<&str> = None;
    let mut hook_action: Option<HookAction> = None;
    let mut path: Option<PathBuf> = None;
    let mut json = false;
    let mut confirm = false;
    let mut reset = false;
    let mut quiet = false;

    // The first positional slot may be a subcommand; the next may be
    // "install"/"remove" for the hook subcommand.
    let mut seen_positional = 0usize;

    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--json" => json = true,
            "--confirm" => confirm = true,
            "--reset" => reset = true,
            "--quiet" | "-q" => quiet = true,
            s if s.starts_with('-') => {
                return Err(format!("unknown flag: {}", s));
            }
            s => {
                // 1st positional: maybe a subcommand, maybe a path.
                if seen_positional == 0 {
                    match s {
                        "triage" | "execute" | "report" | "hook" => {
                            subcommand = Some(match s {
                                "triage" => "triage",
                                "execute" => "execute",
                                "report" => "report",
                                "hook" => "hook",
                                _ => unreachable!(),
                            });
                        }
                        _ => {
                            path = Some(PathBuf::from(s));
                        }
                    }
                } else if subcommand == Some("hook") && hook_action.is_none() {
                    // 2nd positional after "hook": install|remove
                    match s {
                        "install" => hook_action = Some(HookAction::Install),
                        "remove" | "uninstall" => hook_action = Some(HookAction::Remove),
                        other => {
                            return Err(format!(
                                "unknown hook action '{}' (use 'install' or 'remove')",
                                other
                            ))
                        }
                    }
                } else {
                    if path.is_some() {
                        return Err(format!("unexpected argument: {}", s));
                    }
                    path = Some(PathBuf::from(s));
                }
                seen_positional += 1;
            }
        }
    }

    let path = path.unwrap_or_else(|| PathBuf::from("."));

    match subcommand {
        None => Ok(Command::Scan { path, json, quiet }),
        Some("triage") => Ok(Command::Triage { path, reset }),
        Some("execute") => Ok(Command::Execute { path, confirm }),
        Some("report") => Ok(Command::Report { path }),
        Some("hook") => {
            let action = hook_action
                .ok_or_else(|| "hook requires 'install' or 'remove'".to_string())?;
            Ok(Command::Hook { action, path })
        }
        Some(other) => Err(format!("unknown subcommand: {}", other)),
    }
}

// ---------------------------------------------------------------------------
// Shared scan pipeline
// ---------------------------------------------------------------------------

/// Load config, discover repos, run scanners, and build clusters. This is
/// the common prefix for `scan`, `triage`, and (not needed by execute).
fn scan_pipeline(root: &Path) -> Result<(Vec<RepoReport>, Vec<cluster::Cluster>, Config), ExitCode> {
    if !root.exists() {
        eprintln!("gitwell: path not found: {}", root.display());
        return Err(ExitCode::from(2));
    }

    let config = config::load(root);
    let repos = discover_repos(root, &config);
    if repos.is_empty() {
        eprintln!("gitwell: no git repositories found at {}", root.display());
        return Err(ExitCode::from(1));
    }

    let scanners = scanner::registry(&config);
    let mut reports: Vec<RepoReport> = Vec::with_capacity(repos.len());
    for repo in &repos {
        let mut sections = Vec::with_capacity(scanners.len());
        for s in &scanners {
            sections.push((s.name().to_string(), s.scan(repo)));
        }
        reports.push(RepoReport {
            repo_name: repo.name(),
            repo_path: repo.path.display().to_string(),
            sections,
        });
    }

    let clusters = cluster::build_clusters(&reports, &config);
    Ok((reports, clusters, config))
}

fn repo_path_map(reports: &[RepoReport]) -> HashMap<String, String> {
    reports
        .iter()
        .map(|r| (r.repo_name.clone(), r.repo_path.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// Subcommand: scan (default)
// ---------------------------------------------------------------------------

fn run_scan(root: PathBuf, json: bool, quiet: bool) -> ExitCode {
    let (reports, clusters, _config) = match scan_pipeline(&root) {
        Ok(v) => v,
        // In quiet mode, hooks shouldn't bark in unrelated directories.
        Err(_) if quiet => return ExitCode::SUCCESS,
        Err(code) => return code,
    };

    if quiet {
        let finding_total: usize = reports.iter().map(|r| r.total()).sum();
        if finding_total > 0 {
            let repo_count = reports.iter().filter(|r| r.total() > 0).count();
            println!(
                "GitWell: {} session{}, {} finding{} across {} repo{}",
                clusters.len(),
                if clusters.len() == 1 { "" } else { "s" },
                finding_total,
                if finding_total == 1 { "" } else { "s" },
                repo_count,
                if repo_count == 1 { "" } else { "s" },
            );
        }
        return ExitCode::SUCCESS;
    }

    if json {
        report::print_json(&reports, &clusters);
    } else {
        report::print_terminal(&reports, &clusters);
    }

    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// Subcommand: triage
// ---------------------------------------------------------------------------

fn run_triage(root: PathBuf, reset: bool) -> ExitCode {
    if reset {
        match TriageState::reset(&root) {
            Ok(true) => println!("gitwell: cleared {}", TriageState::state_path(&root).display()),
            Ok(false) => println!("gitwell: no triage state to clear"),
            Err(e) => {
                eprintln!("gitwell: failed to reset triage state: {}", e);
                return ExitCode::from(1);
            }
        }
        return ExitCode::SUCCESS;
    }

    let (reports, clusters, _config) = match scan_pipeline(&root) {
        Ok(v) => v,
        Err(code) => return code,
    };

    let paths = repo_path_map(&reports);
    let summary = match triage::run(&root, &clusters, &paths) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("gitwell: triage failed: {}", e);
            return ExitCode::from(1);
        }
    };

    println!();
    println!(
        "Triaged {} session{}: {} resume, {} archive, {} delete, {} skipped{}",
        summary.total_decided(),
        if summary.total_decided() == 1 { "" } else { "s" },
        summary.resume,
        summary.archive,
        summary.delete,
        summary.skip,
        if summary.already_decided > 0 {
            format!(" ({} already decided)", summary.already_decided)
        } else {
            String::new()
        },
    );

    if !summary.quit_early && summary.total_decided() > 0 {
        println!(
            "Run `gitwell execute {}` to preview, `gitwell execute {} --confirm` to apply.",
            root.display(),
            root.display(),
        );
    }

    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// Subcommand: execute
// ---------------------------------------------------------------------------

fn run_execute(root: PathBuf, confirm: bool) -> ExitCode {
    if !root.exists() {
        eprintln!("gitwell: path not found: {}", root.display());
        return ExitCode::from(2);
    }

    let summary = match execute::run(&root, confirm) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("gitwell: execute failed: {}", e);
            return ExitCode::from(1);
        }
    };

    println!();
    if confirm {
        println!(
            "Executed {} action{}: {} succeeded, {} skipped, {} failed ({} decision{} marked done).",
            summary.actions_total,
            if summary.actions_total == 1 { "" } else { "s" },
            summary.succeeded,
            summary.skipped,
            summary.failed,
            summary.decisions_executed,
            if summary.decisions_executed == 1 { "" } else { "s" },
        );
    } else {
        println!(
            "Dry-run: {} action{} would run ({} no-op/skipped). Re-run with --confirm to apply.",
            summary.actions_total,
            if summary.actions_total == 1 { "" } else { "s" },
            summary.skipped,
        );
    }

    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// Subcommand: report
// ---------------------------------------------------------------------------

fn run_report(root: PathBuf) -> ExitCode {
    let (reports, clusters, _config) = match scan_pipeline(&root) {
        Ok(v) => v,
        Err(code) => return code,
    };

    match report_md::generate(&root, &reports, &clusters) {
        Ok(path) => {
            println!("gitwell: wrote {}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gitwell: failed to write report: {}", e);
            ExitCode::from(1)
        }
    }
}

// ---------------------------------------------------------------------------
// Subcommand: hook
// ---------------------------------------------------------------------------

fn run_hook(action: HookAction, root: PathBuf) -> ExitCode {
    let result = match action {
        HookAction::Install => hook::install(&root),
        HookAction::Remove => hook::remove(&root),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gitwell: {}", e);
            ExitCode::from(1)
        }
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover_repos(root: &Path, config: &Config) -> Vec<Repo> {
    if is_repo_root(root) {
        if let Ok(r) = Repo::open(root) {
            if !repo_ignored(&r, config) {
                return vec![r];
            }
            return Vec::new();
        }
    }

    let mut repos = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && is_repo_root(&path) {
                if let Ok(r) = Repo::open(&path) {
                    if !repo_ignored(&r, config) {
                        repos.push(r);
                    }
                }
            }
        }
    }
    repos.sort_by(|a, b| a.path.cmp(&b.path));
    repos
}

fn repo_ignored(repo: &Repo, config: &Config) -> bool {
    let name = repo.name();
    config
        .ignore_repos
        .iter()
        .any(|pat| util::glob_match(pat, &name))
}

// ---------------------------------------------------------------------------
// Help
// ---------------------------------------------------------------------------

fn print_help() {
    println!("gitwell — surface abandoned work in git repositories");
    println!();
    println!("USAGE:");
    println!("    gitwell [PATH] [--json] [--quiet]");
    println!("    gitwell triage  [PATH] [--reset]");
    println!("    gitwell execute [PATH] [--confirm]");
    println!("    gitwell report  [PATH]");
    println!("    gitwell hook    install|remove [PATH]");
    println!();
    println!("SUBCOMMANDS:");
    println!("    (none)     Scan and print the terminal report.");
    println!("    triage     Interactively walk through sessions and decide.");
    println!("    execute    Execute queued triage decisions.");
    println!("    report     Write a markdown report to .gitwell/report-YYYY-MM-DD.md.");
    println!("    hook       Install or remove a post-commit nudge in the current repo.");
    println!();
    println!("FLAGS:");
    println!("    --json        Emit findings as JSON (scan only).");
    println!("    --quiet, -q   Print a one-line summary only (scan only; good for hooks).");
    println!("    --reset       Clear triage state and exit (triage only).");
    println!("    --confirm     Actually apply actions (execute only; default is dry-run).");
    println!("    -h, --help    Show this help.");
    println!();
    println!("ARGS:");
    println!("    PATH          A git repo or directory of git repos. Defaults to `.`.");
}
