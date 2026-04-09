//! Install / remove a post-commit hook that runs `gitwell --quiet`.
//!
//! The hook prints a one-line nudge after each commit if the repo has
//! accumulated stale work:
//!
//! ```
//! GitWell: 3 sessions, 11 findings across 1 repo
//! ```
//!
//! and stays silent otherwise. It's designed to be non-invasive.
//!
//! We don't overwrite a user's existing `post-commit` hook — we append
//! a clearly-delimited block so `gitwell hook remove` can strip it out
//! cleanly while leaving the rest of the hook untouched.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::git::Repo;

const BEGIN_MARKER: &str = "# >>> gitwell-hook (managed) >>>";
const END_MARKER: &str = "# <<< gitwell-hook (managed) <<<";
const HOOK_NAME: &str = "post-commit";

pub fn install(scan_path: &Path) -> io::Result<()> {
    let repo = Repo::open(scan_path)
        .map_err(|e| io::Error::new(io::ErrorKind::NotFound, format!("{}: {}", scan_path.display(), e)))?;

    let hook_path = hook_path(&repo)?;

    // Existing contents (if any) — we always append.
    let existing = if hook_path.exists() {
        fs::read_to_string(&hook_path)?
    } else {
        String::new()
    };

    if existing.contains(BEGIN_MARKER) {
        println!("gitwell: hook already installed at {}", hook_path.display());
        return Ok(());
    }

    // Use the absolute path to the currently-running binary so the hook
    // keeps working even if the user's PATH changes in a fresh shell.
    let bin = env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "gitwell".to_string());

    let block = build_block(&bin);

    let mut new_contents = String::new();
    if existing.is_empty() {
        new_contents.push_str("#!/bin/sh\n");
    } else {
        new_contents.push_str(&existing);
        if !new_contents.ends_with('\n') {
            new_contents.push('\n');
        }
    }
    new_contents.push('\n');
    new_contents.push_str(&block);

    if let Some(parent) = hook_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&hook_path, &new_contents)?;
    make_executable(&hook_path)?;

    println!(
        "gitwell: installed post-commit hook at {}",
        hook_path.display()
    );
    println!("       commit something to try it out");
    Ok(())
}

pub fn remove(scan_path: &Path) -> io::Result<()> {
    let repo = Repo::open(scan_path)
        .map_err(|e| io::Error::new(io::ErrorKind::NotFound, format!("{}: {}", scan_path.display(), e)))?;

    let hook_path = hook_path(&repo)?;
    if !hook_path.exists() {
        println!("gitwell: no post-commit hook found");
        return Ok(());
    }

    let contents = fs::read_to_string(&hook_path)?;
    if !contents.contains(BEGIN_MARKER) {
        println!("gitwell: hook file exists but no managed block found; leaving alone");
        return Ok(());
    }

    let cleaned = strip_block(&contents);
    let trimmed = cleaned.trim();

    // If stripping leaves nothing but a shebang (or nothing at all),
    // remove the file entirely so we don't litter the repo.
    if trimmed.is_empty() || trimmed == "#!/bin/sh" {
        fs::remove_file(&hook_path)?;
        println!("gitwell: removed {}", hook_path.display());
    } else {
        fs::write(&hook_path, cleaned)?;
        println!(
            "gitwell: removed managed block from {}",
            hook_path.display()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn hook_path(repo: &Repo) -> io::Result<PathBuf> {
    // Resolve via `git rev-parse --git-path hooks/post-commit` so this
    // works for worktrees and bare repos, not just the naive .git/hooks.
    let raw = repo.run(&["rev-parse", "--git-path", &format!("hooks/{}", HOOK_NAME)])?;
    let trimmed = raw.trim();
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(repo.path.join(path))
    }
}

fn build_block(gitwell_bin: &str) -> String {
    let mut s = String::new();
    s.push_str(BEGIN_MARKER);
    s.push('\n');
    s.push_str("# Installed by `gitwell hook install`. Remove with `gitwell hook remove`.\n");
    s.push_str(&format!("if [ -x \"{bin}\" ]; then\n    \"{bin}\" --quiet . 2>/dev/null || true\nfi\n", bin = gitwell_bin));
    s.push_str(END_MARKER);
    s.push('\n');
    s
}

/// Remove the managed block (inclusive of the marker lines) from `contents`.
fn strip_block(contents: &str) -> String {
    let mut out = String::with_capacity(contents.len());
    let mut inside = false;
    for line in contents.lines() {
        if line.trim() == BEGIN_MARKER {
            inside = true;
            continue;
        }
        if line.trim() == END_MARKER {
            inside = false;
            continue;
        }
        if inside {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    // Collapse a trailing blank that may have been left behind.
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

#[cfg(unix)]
fn make_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> io::Result<()> {
    // On non-Unix, just no-op.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_block_removes_managed_lines_only() {
        let input = format!(
            "#!/bin/sh\necho before\n\n{}\nguff\n{}\necho after\n",
            BEGIN_MARKER, END_MARKER
        );
        let out = strip_block(&input);
        assert!(out.contains("echo before"));
        assert!(out.contains("echo after"));
        assert!(!out.contains("guff"));
        assert!(!out.contains("gitwell-hook"));
    }

    #[test]
    fn strip_block_handles_only_managed_block() {
        let input = format!(
            "#!/bin/sh\n\n{}\nguff\n{}\n",
            BEGIN_MARKER, END_MARKER
        );
        let out = strip_block(&input);
        assert!(out.trim().starts_with("#!/bin/sh"));
        assert!(!out.contains("guff"));
    }
}
