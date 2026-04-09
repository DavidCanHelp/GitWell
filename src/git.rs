//! Thin wrapper around the `git` CLI.
//!
//! Every git interaction in GitWell flows through this module. We shell out
//! via `std::process::Command` rather than parsing `.git` internals so we
//! stay compatible with future git changes for free.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Repo {
    pub path: PathBuf,
}

impl Repo {
    /// Open a repository rooted at `path`. Verifies with `git rev-parse`.
    /// The stored path is canonicalized so `repo.name()` works for `.`.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let raw = path.as_ref().to_path_buf();
        let path = raw.canonicalize().unwrap_or(raw);
        let out = Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(["rev-parse", "--git-dir"])
            .output()?;
        if !out.status.success() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} is not a git repository", path.display()),
            ));
        }
        Ok(Repo { path })
    }

    /// Run `git <args>` in the repo. Returns stdout as a String on success.
    pub fn run(&self, args: &[&str]) -> io::Result<String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(args)
            .output()?;
        if !out.status.success() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "git {} failed: {}",
                    args.join(" "),
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Run `git <args>` and return only whether it succeeded.
    /// Useful for predicates like `merge-base --is-ancestor`.
    pub fn succeeds(&self, args: &[&str]) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(args)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Best-effort default branch detection.
    /// Tries `origin/HEAD`, then common names as fallbacks.
    pub fn default_branch(&self) -> Option<String> {
        if let Ok(out) = self.run(&["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) {
            let name = out.trim();
            if let Some(stripped) = name.strip_prefix("origin/") {
                return Some(stripped.to_string());
            }
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
        for candidate in ["main", "master", "trunk", "develop"] {
            if self.succeeds(&["show-ref", "--verify", &format!("refs/heads/{}", candidate)]) {
                return Some(candidate.to_string());
            }
        }
        None
    }

    /// Human-friendly repo name derived from its directory.
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string()
    }

    // -----------------------------------------------------------------
    // Action helpers (used by the execute subcommand)
    // -----------------------------------------------------------------

    pub fn apply_stash(&self, stash_ref: &str) -> io::Result<String> {
        self.run(&["stash", "apply", stash_ref])
    }

    pub fn drop_stash(&self, stash_ref: &str) -> io::Result<String> {
        self.run(&["stash", "drop", stash_ref])
    }

    pub fn delete_branch(&self, name: &str) -> io::Result<String> {
        self.run(&["branch", "-D", name])
    }

    pub fn create_tag(&self, tag: &str, target: &str) -> io::Result<String> {
        self.run(&["tag", tag, target])
    }

    /// Produce the `-p` diff of a stash for archival.
    pub fn stash_diff(&self, stash_ref: &str) -> io::Result<String> {
        self.run(&["stash", "show", "-p", stash_ref])
    }

    /// Check whether a local branch currently exists.
    pub fn branch_exists(&self, name: &str) -> bool {
        self.succeeds(&["show-ref", "--verify", &format!("refs/heads/{}", name)])
    }

    /// Look up the current `stash@{N}` ref for a given stash commit SHA.
    /// Returns `None` if no stash with that SHA still exists — the stash
    /// was dropped, or its index shifted and we can't match it anymore.
    pub fn find_stash_by_sha(&self, sha: &str) -> Option<String> {
        let out = self.run(&["stash", "list", "--format=%H%x09%gd"]).ok()?;
        for line in out.lines() {
            let parts: Vec<&str> = line.splitn(2, '\t').collect();
            if parts.len() == 2 && parts[0] == sha {
                return Some(parts[1].to_string());
            }
        }
        None
    }
}

/// Returns true if `path` looks like a git worktree root (has `.git`)
/// or a bare repo (has `HEAD` at the top level).
pub fn is_repo_root(path: &Path) -> bool {
    path.join(".git").exists() || (path.join("HEAD").exists() && path.join("objects").is_dir())
}
