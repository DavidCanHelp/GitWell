//! Persistent state for the triage subsystem.
//!
//! Decisions made during `gitwell triage` are written to
//! `<scan_path>/.gitwell/triage.json`. On a re-run, sessions whose
//! "session key" already appears in state are skipped so the user isn't
//! asked the same question twice. The `gitwell execute` subcommand
//! reads the same file to perform the queued actions.
//!
//! Schema (version 1):
//!
//! ```json
//! {
//!   "version": 1,
//!   "decisions": [
//!     {
//!       "session_key": "auth:1710000000",
//!       "session_label": "auth",
//!       "decision": "archive",
//!       "decided_at": "2026-04-09T10:30:00Z",
//!       "findings": [
//!         {
//!           "repo": "myapp",
//!           "repo_path": "/Users/me/code/myapp",
//!           "kind": "stale_branch",
//!           "detail": "feature/auth-v2"
//!         }
//!       ],
//!       "executed": false
//!     }
//!   ]
//! }
//! ```

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::cluster::Cluster;
use crate::json::{self, Value};
use crate::scanner::Finding;

pub const STATE_DIR: &str = ".gitwell";
pub const STATE_FILE: &str = "triage.json";
pub const ARCHIVE_SUBDIR: &str = "archives";

pub const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionKind {
    Resume,
    Archive,
    Delete,
    Skip,
}

impl DecisionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Resume => "resume",
            Self::Archive => "archive",
            Self::Delete => "delete",
            Self::Skip => "skip",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "resume" => Some(Self::Resume),
            "archive" => Some(Self::Archive),
            "delete" => Some(Self::Delete),
            "skip" => Some(Self::Skip),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecisionFinding {
    pub repo: String,
    pub repo_path: String,
    pub kind: String, // "stale_branch" | "stash" | "wip_commit" | "orphan_commit" | "dormant_repo"
    pub detail: String,
    /// Only populated for stashes — lets the executor find the stash by
    /// commit SHA even if its index has shifted.
    pub stash_sha: Option<String>,
}

impl DecisionFinding {
    pub fn from_finding(repo: &str, repo_path: &str, f: &Finding) -> Self {
        match f {
            Finding::StaleBranch { name, .. } => DecisionFinding {
                repo: repo.to_string(),
                repo_path: repo_path.to_string(),
                kind: "stale_branch".into(),
                detail: name.clone(),
                stash_sha: None,
            },
            Finding::Stash {
                index,
                sha,
                message,
                ..
            } => DecisionFinding {
                repo: repo.to_string(),
                repo_path: repo_path.to_string(),
                kind: "stash".into(),
                detail: format!("{}: {}", index, message),
                stash_sha: Some(sha.clone()),
            },
            Finding::WipCommit { sha, message, .. } => DecisionFinding {
                repo: repo.to_string(),
                repo_path: repo_path.to_string(),
                kind: "wip_commit".into(),
                detail: format!("{} {}", short_sha(sha), message),
                stash_sha: None,
            },
            Finding::OrphanCommit { sha, message, .. } => DecisionFinding {
                repo: repo.to_string(),
                repo_path: repo_path.to_string(),
                kind: "orphan_commit".into(),
                detail: format!("{} {}", short_sha(sha), message),
                stash_sha: None,
            },
            Finding::DormantRepo { path, .. } => DecisionFinding {
                repo: repo.to_string(),
                repo_path: repo_path.to_string(),
                kind: "dormant_repo".into(),
                detail: path.clone(),
                stash_sha: None,
            },
        }
    }
}

fn short_sha(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}

#[derive(Debug, Clone)]
pub struct Decision {
    /// Stable identifier for the session — used to skip already-decided
    /// sessions on re-run. Format: `{label}:{start_ts}`.
    pub session_key: String,
    pub session_label: String,
    pub decision: DecisionKind,
    pub decided_at: String,
    pub findings: Vec<DecisionFinding>,
    pub executed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct TriageState {
    pub decisions: Vec<Decision>,
}

impl TriageState {
    pub fn state_path(scan_path: &Path) -> PathBuf {
        scan_path.join(STATE_DIR).join(STATE_FILE)
    }

    pub fn archive_dir(scan_path: &Path) -> PathBuf {
        scan_path.join(STATE_DIR).join(ARCHIVE_SUBDIR)
    }

    /// Load state. Returns an empty state if the file doesn't exist yet.
    pub fn load(scan_path: &Path) -> io::Result<Self> {
        let path = Self::state_path(scan_path);
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)?;
        let v = json::parse(&content)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Self::from_json(&v).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    pub fn save(&self, scan_path: &Path) -> io::Result<()> {
        let path = Self::state_path(scan_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let v = self.to_json();
        fs::write(&path, json::to_pretty_string(&v))?;
        Ok(())
    }

    /// Delete the state file (used by `gitwell triage --reset`).
    pub fn reset(scan_path: &Path) -> io::Result<bool> {
        let path = Self::state_path(scan_path);
        if path.exists() {
            fs::remove_file(&path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn is_decided(&self, session_key: &str) -> bool {
        self.decisions.iter().any(|d| d.session_key == session_key)
    }

    fn from_json(v: &Value) -> Result<Self, String> {
        let version = v
            .get("version")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| "missing 'version'".to_string())?;
        if version != SCHEMA_VERSION {
            return Err(format!("unsupported schema version {}", version));
        }
        let decisions_v = v
            .get("decisions")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "missing 'decisions' array".to_string())?;

        let mut decisions = Vec::with_capacity(decisions_v.len());
        for d in decisions_v {
            let session_label = d
                .get("session_label")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let session_key = d
                .get("session_key")
                .and_then(|v| v.as_str())
                .map(String::from)
                // Fall back to label for any legacy file.
                .unwrap_or_else(|| session_label.clone());
            let decision_str = d
                .get("decision")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "decision missing 'decision'".to_string())?;
            let decision = DecisionKind::parse(decision_str)
                .ok_or_else(|| format!("unknown decision kind: {}", decision_str))?;
            let decided_at = d
                .get("decided_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let executed = d.get("executed").and_then(|v| v.as_bool()).unwrap_or(false);

            let findings_v = d
                .get("findings")
                .and_then(|v| v.as_array())
                .ok_or_else(|| "decision missing 'findings'".to_string())?;
            let mut findings = Vec::with_capacity(findings_v.len());
            for f in findings_v {
                findings.push(DecisionFinding {
                    repo: f.get("repo").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    repo_path: f
                        .get("repo_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    kind: f.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    detail: f.get("detail").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    stash_sha: f.get("stash_sha").and_then(|v| v.as_str()).map(String::from),
                });
            }

            decisions.push(Decision {
                session_key,
                session_label,
                decision,
                decided_at,
                findings,
                executed,
            });
        }
        Ok(TriageState { decisions })
    }

    fn to_json(&self) -> Value {
        let mut decisions = Vec::with_capacity(self.decisions.len());
        for d in &self.decisions {
            let mut findings_json = Vec::with_capacity(d.findings.len());
            for f in &d.findings {
                let mut entries = vec![
                    ("repo".to_string(), Value::String(f.repo.clone())),
                    ("repo_path".to_string(), Value::String(f.repo_path.clone())),
                    ("kind".to_string(), Value::String(f.kind.clone())),
                    ("detail".to_string(), Value::String(f.detail.clone())),
                ];
                if let Some(sha) = &f.stash_sha {
                    entries.push(("stash_sha".to_string(), Value::String(sha.clone())));
                }
                findings_json.push(Value::Object(entries));
            }
            let decision = Value::Object(vec![
                ("session_key".to_string(), Value::String(d.session_key.clone())),
                ("session_label".to_string(), Value::String(d.session_label.clone())),
                ("decision".to_string(), Value::String(d.decision.as_str().to_string())),
                ("decided_at".to_string(), Value::String(d.decided_at.clone())),
                ("findings".to_string(), Value::Array(findings_json)),
                ("executed".to_string(), Value::Bool(d.executed)),
            ]);
            decisions.push(decision);
        }
        Value::Object(vec![
            ("version".to_string(), Value::Int(SCHEMA_VERSION)),
            ("decisions".to_string(), Value::Array(decisions)),
        ])
    }
}

/// Stable-ish identifier for a cluster across runs. If the same cluster
/// reappears with the same label and earliest timestamp, we treat it as
/// already decided.
pub fn session_key_for(cluster: &Cluster) -> String {
    format!("{}:{}", cluster.label, cluster.start_ts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let state = TriageState {
            decisions: vec![Decision {
                session_key: "auth:1710000000".into(),
                session_label: "auth".into(),
                decision: DecisionKind::Archive,
                decided_at: "2026-04-09T10:30:00Z".into(),
                findings: vec![DecisionFinding {
                    repo: "myapp".into(),
                    repo_path: "/tmp/myapp".into(),
                    kind: "stale_branch".into(),
                    detail: "feature/auth-v2".into(),
                    stash_sha: None,
                }],
                executed: false,
            }],
        };
        let s = json::to_pretty_string(&state.to_json());
        let parsed = json::parse(&s).unwrap();
        let reloaded = TriageState::from_json(&parsed).unwrap();
        assert_eq!(reloaded.decisions.len(), 1);
        assert_eq!(reloaded.decisions[0].session_key, "auth:1710000000");
        assert_eq!(reloaded.decisions[0].decision, DecisionKind::Archive);
        assert_eq!(reloaded.decisions[0].findings[0].detail, "feature/auth-v2");
    }
}
