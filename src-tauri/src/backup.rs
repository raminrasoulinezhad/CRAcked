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
use rusqlite::Connection;
use serde::Serialize;
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

/// Ensure the directory is a git repo with a commit identity available.
fn ensure_repo(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create dir: {e}"))?;
    if !dir.join(".git").exists() {
        let out = run(Command::new("git").arg("-C").arg(dir).arg("init").arg("-q"))
            .map_err(|e| format!("git init: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "git init failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    // Guarantee a commit identity even if the user has no global git config.
    ensure_local_identity(dir, "user.name", "CRAcked");
    ensure_local_identity(dir, "user.email", "cracked@localhost");
    Ok(())
}

fn ensure_local_identity(dir: &Path, key: &str, fallback: &str) {
    let has = run(Command::new("git").arg("-C").arg(dir).args(["config", key]))
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false);
    if !has {
        let _ = run(Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["config", key, fallback]));
    }
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

/// Stage everything and commit if there is anything to commit.
fn git_commit(dir: &Path, message: &str) -> Result<bool, String> {
    let add = run(Command::new("git").arg("-C").arg(dir).args(["add", "-A"]))
        .map_err(|e| format!("git add: {e}"))?;
    if !add.status.success() {
        return Err(format!(
            "git add failed: {}",
            String::from_utf8_lossy(&add.stderr)
        ));
    }
    // `diff --cached --quiet` exits non-zero when there ARE staged changes.
    let diff = run(Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["diff", "--cached", "--quiet"]))
    .map_err(|e| format!("git diff: {e}"))?;
    if diff.status.success() {
        return Ok(false); // nothing changed
    }
    let commit = run(Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-q", "-m", message]))
    .map_err(|e| format!("git commit: {e}"))?;
    if !commit.status.success() {
        return Err(format!(
            "git commit failed: {}",
            String::from_utf8_lossy(&commit.stderr)
        ));
    }
    Ok(true)
}

/// Append-only push to Google Drive via `rclone copy` (NEVER `sync`).
fn rclone_copy(cfg: &BackupConfig) -> (bool, String) {
    let dest = format!("{}:{}", cfg.rclone_remote, cfg.rclone_folder);
    match run(Command::new("rclone")
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

    fn git_available() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    #[test]
    fn commit_only_when_changed() {
        if !git_available() {
            return;
        }
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

        let log = Command::new("git")
            .arg("-C")
            .arg(&tmp)
            .args(["rev-list", "--count", "HEAD"])
            .output()
            .unwrap();
        let count: i32 = String::from_utf8_lossy(&log.stdout).trim().parse().unwrap();
        assert_eq!(count, 2, "exactly two commits expected");

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
