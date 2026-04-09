//! Test harness for the integration tests in `tests/*.rs`.
//!
//! Every test gets its own isolated temp directory and a real git repo
//! inside it. No mocks — all scanner logic exercises real `git` commands.
//!
//! Drop cleans up temp dirs automatically, so failing tests don't leave
//! litter behind.
//!
//! # Unique temp paths
//!
//! We build paths from `{system_temp}/gitwell-tests/{pid}-{nanos}-{counter}`
//! so parallel test threads (cargo's default) never collide.

#![allow(dead_code)] // Not every test file uses every helper.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Reserve a unique temp path. The directory is NOT created here —
/// callers create it (or let `git init` create it).
pub fn unique_path(prefix: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let base = std::env::temp_dir().join("gitwell-tests");
    let _ = fs::create_dir_all(&base);
    base.join(format!("{}-{}-{}-{}", prefix, pid, nanos, n))
}

// ---------------------------------------------------------------------------
// TempRepo
// ---------------------------------------------------------------------------

/// A real git repository in an isolated temp directory.
///
/// The repo is initialized with `git init -b main`, a local `user.name`
/// and `user.email` are configured, and an initial commit is made on
/// `main` so the repo has a default branch HEAD to work against.
pub struct TempRepo {
    path: PathBuf,
    /// If true, `Drop` removes the directory. Tests can set this to
    /// false via `keep()` when debugging.
    cleanup: bool,
}

impl TempRepo {
    /// Create a new repo in a fresh temp dir.
    pub fn new(name: &str) -> Self {
        let path = unique_path(name);
        fs::create_dir_all(&path).expect("create temp repo dir");

        run_git(&path, &["init", "-q", "-b", "main"]);
        run_git(&path, &["config", "user.email", "test@gitwell.test"]);
        run_git(&path, &["config", "user.name", "GitWell Test"]);
        // commit.gpgsign on the host would break non-interactive commits.
        run_git(&path, &["config", "commit.gpgsign", "false"]);

        // Initial commit so `git for-each-ref refs/heads/` has something
        // to return. We write a README rather than using --allow-empty
        // so the worktree has real content.
        fs::write(path.join("README.md"), "# test\n").expect("write README");
        run_git(&path, &["add", "README.md"]);
        run_git(&path, &["commit", "-q", "-m", "Initial commit"]);

        TempRepo { path, cleanup: true }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Disable Drop-time cleanup (useful when debugging a failing test).
    pub fn keep(mut self) -> Self {
        self.cleanup = false;
        self
    }

    // -- content helpers ---------------------------------------------------

    pub fn write_file(&self, rel: &str, contents: &str) {
        let p = self.path.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(p, contents).expect("write file");
    }

    pub fn append_file(&self, rel: &str, contents: &str) {
        let p = self.path.join(rel);
        let mut existing = fs::read_to_string(&p).unwrap_or_default();
        existing.push_str(contents);
        fs::write(p, existing).expect("append file");
    }

    // -- commit helpers ---------------------------------------------------

    /// Make a commit against the current worktree (expects staged or
    /// unstaged changes that will be auto-staged via `-a`, or an empty
    /// commit if nothing has changed).
    pub fn commit(&self, msg: &str) {
        // Stage any unstaged worktree changes, then commit.
        run_git(&self.path, &["add", "-A"]);
        run_git(
            &self.path,
            &["commit", "-q", "--allow-empty", "-m", msg],
        );
    }

    /// Write `rel` with `contents` and commit with `msg`.
    pub fn commit_file(&self, rel: &str, contents: &str, msg: &str) {
        self.write_file(rel, contents);
        self.commit(msg);
    }

    /// Commit with a specific committer/author date. `unix_ts` is seconds
    /// since epoch — use far-past values (e.g. `1577836800` for 2020-01-01)
    /// for deterministic "old" tests independent of wall clock.
    pub fn commit_file_at(&self, rel: &str, contents: &str, msg: &str, unix_ts: i64) {
        self.write_file(rel, contents);
        run_git(&self.path, &["add", "-A"]);
        let date = format!("@{} +0000", unix_ts);
        run_git_env(
            &self.path,
            &["commit", "-q", "--allow-empty", "-m", msg],
            &[
                ("GIT_AUTHOR_DATE", &date),
                ("GIT_COMMITTER_DATE", &date),
            ],
        );
    }

    /// Empty commit at a specific date.
    pub fn empty_commit_at(&self, msg: &str, unix_ts: i64) {
        let date = format!("@{} +0000", unix_ts);
        run_git_env(
            &self.path,
            &["commit", "-q", "--allow-empty", "-m", msg],
            &[
                ("GIT_AUTHOR_DATE", &date),
                ("GIT_COMMITTER_DATE", &date),
            ],
        );
    }

    // -- branch / tag / stash ---------------------------------------------

    /// Create a new branch from HEAD and check it out.
    pub fn branch(&self, name: &str) {
        run_git(&self.path, &["checkout", "-q", "-b", name]);
    }

    /// Check out an existing branch.
    pub fn checkout(&self, name: &str) {
        run_git(&self.path, &["checkout", "-q", name]);
    }

    /// Merge `name` into the current branch (fast-forward or no-ff).
    pub fn merge(&self, name: &str) {
        run_git(
            &self.path,
            &["merge", "-q", "--no-edit", "--no-ff", name],
        );
    }

    pub fn tag(&self, name: &str) {
        run_git(&self.path, &["tag", name]);
    }

    /// Write a file, `git add`, and `git stash push -m msg`.
    pub fn stash(&self, file_rel: &str, contents: &str, msg: &str) {
        self.write_file(file_rel, contents);
        run_git(&self.path, &["add", file_rel]);
        run_git(&self.path, &["stash", "push", "-q", "-m", msg]);
    }

    /// `git reset --hard <ref>`.
    pub fn reset_hard(&self, target: &str) {
        run_git(&self.path, &["reset", "--hard", "-q", target]);
    }

    // -- introspection ----------------------------------------------------

    pub fn rev_parse(&self, refname: &str) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(["rev-parse", refname])
            .output()
            .expect("git rev-parse");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        if self.cleanup && self.path.exists() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

// ---------------------------------------------------------------------------
// TempDir (directory of repos)
// ---------------------------------------------------------------------------

/// A directory that holds several child repos. Used for multi-repo scan
/// and clustering tests.
///
/// The child repos' cleanup is disabled — this parent removes the whole
/// directory tree at once on Drop.
pub struct TempDir {
    path: PathBuf,
    repos: Vec<TempRepo>,
    cleanup: bool,
}

impl TempDir {
    pub fn new(name: &str) -> Self {
        let path = unique_path(name);
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir {
            path,
            repos: Vec::new(),
            cleanup: true,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Create a fresh repo inside this directory.
    pub fn add_repo(&mut self, name: &str) -> &TempRepo {
        let repo_path = self.path.join(name);
        fs::create_dir_all(&repo_path).expect("create child repo dir");

        run_git(&repo_path, &["init", "-q", "-b", "main"]);
        run_git(&repo_path, &["config", "user.email", "test@gitwell.test"]);
        run_git(&repo_path, &["config", "user.name", "GitWell Test"]);
        run_git(&repo_path, &["config", "commit.gpgsign", "false"]);

        fs::write(repo_path.join("README.md"), "# test\n").expect("write README");
        run_git(&repo_path, &["add", "README.md"]);
        run_git(&repo_path, &["commit", "-q", "-m", "Initial commit"]);

        // The child repo's own cleanup is disabled — parent TempDir
        // removes the whole tree on Drop.
        let repo = TempRepo {
            path: repo_path,
            cleanup: false,
        };
        self.repos.push(repo);
        self.repos.last().expect("just pushed")
    }

    pub fn repos(&self) -> &[TempRepo] {
        &self.repos
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if self.cleanup && self.path.exists() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn run_git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn git {:?}: {}", args, e));
    if !status.success() {
        panic!("git {:?} in {} failed: {}", args, cwd.display(), status);
    }
}

fn run_git_env(cwd: &Path, args: &[&str], env: &[(&str, &str)]) {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(cwd).args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn git {:?}: {}", args, e));
    if !status.success() {
        panic!("git {:?} in {} failed: {}", args, cwd.display(), status);
    }
}
