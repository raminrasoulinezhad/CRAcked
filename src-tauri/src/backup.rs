//! Backup: the "personal blockchain of information".
//!
//! The user's data directory (`<data-dir>/CRAcked/`, which holds `cracked.db`)
//! is itself a **git repository**. After every change we:
//!
//! 1. write a deterministic plain-text `snapshot.json` (readable, diffs cleanly),
//! 2. `git add -A && git commit` — the full, tamper-evident version history
//!    lives *inside* the repo (git chains each commit to its parent by hash),
//! 3. `rclone copy` the whole directory (including `.git`) to Google Drive.
//!
//! The Drive step uses `rclone copy`, **never `sync`** — copy only ever *adds*
//! to the destination, so deleting files locally can never delete them from the
//! backup. That is the core durability guarantee.
//!
//! `rclone` is optional: if no remote is configured the local git history still
//! accrues, and the Drive push is simply skipped.

use crate::db;
use git2::{IndexAddOption, Repository, Signature};
use rusqlite::Connection;
use serde::Serialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Backup configuration, resolved from settings + the DB location.
#[derive(Debug, Clone)]
pub struct BackupConfig {
    /// The data directory that is also the git repo (contains cracked.db).
    pub dir: PathBuf,
    /// rclone remote name (e.g. "gdrive"). Empty string disables the Drive push.
    pub rclone_remote: String,
    /// Destination folder on the remote (e.g. "CRAcked").
    pub rclone_folder: String,
}

const KEY_RCLONE_REMOTE: &str = "backup_rclone_remote";
const KEY_RCLONE_FOLDER: &str = "backup_rclone_folder";

impl BackupConfig {
    /// Load config from the database, defaulting the folder to "CRAcked".
    pub fn load(conn: &Connection) -> Self {
        let dir = db::default_db_path()
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let rclone_remote = db::get_setting(conn, KEY_RCLONE_REMOTE)
            .ok()
            .flatten()
            .unwrap_or_default();
        let rclone_folder = db::get_setting(conn, KEY_RCLONE_FOLDER)
            .ok()
            .flatten()
            .unwrap_or_else(|| "CRAcked".to_string());
        BackupConfig {
            dir,
            rclone_remote,
            rclone_folder,
        }
    }

    pub fn save(conn: &Connection, remote: &str, folder: &str) -> db::Result<()> {
        db::set_setting(conn, KEY_RCLONE_REMOTE, remote)?;
        db::set_setting(conn, KEY_RCLONE_FOLDER, folder)?;
        Ok(())
    }

    /// Whether the Drive push is configured.
    pub fn rclone_enabled(&self) -> bool {
        !self.rclone_remote.trim().is_empty()
    }
}

/// Outcome of a backup run, surfaced to the UI.
#[derive(Debug, Clone, Serialize, Default)]
pub struct BackupReport {
    pub committed: bool,
    pub git_message: String,
    pub rclone_attempted: bool,
    pub rclone_ok: bool,
    pub rclone_message: String,
}

fn run(cmd: &mut Command) -> std::io::Result<std::process::Output> {
    cmd.output()
}

/// Ensure the data directory exists (the git repo is created lazily on commit).
fn ensure_repo(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create dir: {e}"))
}

/// Open the repo at `dir`, initialising it if it doesn't exist yet.
fn open_or_init(dir: &Path) -> Result<Repository, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create dir: {e}"))?;
    Repository::open(dir)
        .or_else(|_| Repository::init(dir))
        .map_err(|e| format!("git init/open: {e}"))
}

/// A commit signature: prefer the repo/global git identity, else a sensible
/// built-in default (so commits work even on a machine with no git config).
fn signature(repo: &Repository) -> Result<Signature<'static>, String> {
    repo.signature()
        .or_else(|_| Signature::now("CRAcked", "cracked@localhost"))
        .map_err(|e| format!("signature: {e}"))
}

/// Write the plain-text snapshot into the repo directory.
fn write_snapshot(conn: &Connection, dir: &Path) -> Result<(), String> {
    let snapshot = db::export_json(conn).map_err(|e| format!("export: {e}"))?;
    let pretty = serde_json::to_string_pretty(&snapshot).map_err(|e| format!("json: {e}"))?;
    std::fs::write(dir.join("snapshot.json"), pretty).map_err(|e| format!("write snapshot: {e}"))?;

    // A small README so the Drive folder is self-explanatory if ever inspected.
    let readme = "# CRAcked data backup\n\nThis directory is a git repository holding your CRAcked data.\n\n- `snapshot.json` — human-readable snapshot (full version history is in git).\n- `cracked.db` — the SQLite database used by the app.\n\nRestore: copy this whole folder back to the app's data directory.\n";
    let _ = std::fs::write(dir.join("README.md"), readme);
    Ok(())
}

/// Stage everything and commit if there is anything to commit. Returns whether
/// a new commit was actually created (false when nothing changed).
fn git_commit(dir: &Path, message: &str) -> Result<bool, String> {
    let repo = open_or_init(dir)?;

    // Stage all files in the working directory.
    let mut index = repo.index().map_err(|e| format!("index: {e}"))?;
    index
        .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
        .map_err(|e| format!("add_all: {e}"))?;
    index.write().map_err(|e| format!("index write: {e}"))?;
    let tree_oid = index.write_tree().map_err(|e| format!("write_tree: {e}"))?;
    let tree = repo.find_tree(tree_oid).map_err(|e| format!("find_tree: {e}"))?;

    // Find the current HEAD commit, if any, as the parent.
    let parent = match repo.head() {
        Ok(head) => Some(head.peel_to_commit().map_err(|e| format!("peel: {e}"))?),
        Err(_) => None, // unborn HEAD -> this is the first commit
    };

    // Nothing changed since the last commit? Then don't create an empty one.
    if let Some(ref p) = parent {
        if p.tree().map_err(|e| format!("parent tree: {e}"))?.id() == tree_oid {
            return Ok(false);
        }
    }

    let sig = signature(&repo)?;
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
        .map_err(|e| format!("commit: {e}"))?;
    Ok(true)
}

/// Which `rclone` to invoke: prefer the copy bundled next to the app executable
/// (the sidecar shipped in the installer), falling back to a system `rclone` on
/// PATH (handy during development).
fn rclone_program() -> OsString {
    let name = if cfg!(windows) { "rclone.exe" } else { "rclone" };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(name);
            if candidate.exists() {
                return candidate.into_os_string();
            }
        }
    }
    OsString::from("rclone")
}

/// Append-only push to Google Drive via `rclone copy` (NEVER `sync`).
fn rclone_copy(cfg: &BackupConfig) -> (bool, String) {
    let dest = format!("{}:{}", cfg.rclone_remote, cfg.rclone_folder);
    match run(Command::new(rclone_program())
        .arg("copy")
        .arg(&cfg.dir)
        .arg(&dest)
        .arg("--transfers=4"))
    {
        Ok(out) if out.status.success() => (true, format!("Copied to {dest}")),
        Ok(out) => (
            false,
            format!("rclone failed: {}", String::from_utf8_lossy(&out.stderr).trim()),
        ),
        Err(e) => (false, format!("rclone not run: {e}")),
    }
}

/// Full backup: snapshot -> local commit -> (optional) Drive copy.
pub fn back_up(conn: &Connection, cfg: &BackupConfig, message: &str) -> Result<BackupReport, String> {
    ensure_repo(&cfg.dir)?;
    write_snapshot(conn, &cfg.dir)?;
    let committed = git_commit(&cfg.dir, message)?;

    let mut report = BackupReport {
        committed,
        git_message: message.to_string(),
        ..Default::default()
    };

    if cfg.rclone_enabled() {
        report.rclone_attempted = true;
        let (ok, msg) = rclone_copy(cfg);
        report.rclone_ok = ok;
        report.rclone_message = msg;
    } else {
        report.rclone_message = "Google Drive not configured — local git history only.".into();
    }
    Ok(report)
}

/// Fire-and-forget backup used after routine mutations: always commits locally
/// (fast); pushes to Drive on a background thread so the UI never blocks. Errors
/// are swallowed here — the manual "Back up now" path surfaces real status.
pub fn back_up_async(conn: &Connection, cfg: BackupConfig, message: String) {
    // Local snapshot + commit synchronously (cheap, keeps history exact).
    if ensure_repo(&cfg.dir).is_ok() {
        let _ = write_snapshot(conn, &cfg.dir);
        let _ = git_commit(&cfg.dir, &message);
    }
    // Drive push off-thread.
    if cfg.rclone_enabled() {
        std::thread::spawn(move || {
            let _ = rclone_copy(&cfg);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Count commits reachable from HEAD using the embedded git.
    fn commit_count(dir: &Path) -> usize {
        let repo = Repository::open(dir).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_head().unwrap();
        walk.count()
    }

    #[test]
    fn commit_only_when_changed() {
        let tmp = std::env::temp_dir().join(format!("cracked_test_repo_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        ensure_repo(&tmp).unwrap();

        std::fs::write(tmp.join("snapshot.json"), "{\"a\":1}").unwrap();
        assert!(git_commit(&tmp, "first").unwrap(), "first write should commit");
        // No change -> no commit.
        assert!(!git_commit(&tmp, "again").unwrap(), "no change should not commit");
        // Change -> commit.
        std::fs::write(tmp.join("snapshot.json"), "{\"a\":2}").unwrap();
        assert!(git_commit(&tmp, "second").unwrap(), "changed write should commit");

        assert_eq!(commit_count(&tmp), 2, "exactly two commits expected");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rclone_disabled_when_remote_blank() {
        let cfg = BackupConfig {
            dir: PathBuf::from("/tmp"),
            rclone_remote: "".into(),
            rclone_folder: "CRAcked".into(),
        };
        assert!(!cfg.rclone_enabled());
        let cfg2 = BackupConfig {
            rclone_remote: "gdrive".into(),
            ..cfg
        };
        assert!(cfg2.rclone_enabled());
    }
}
