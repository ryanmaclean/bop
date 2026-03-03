use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::util;

pub const LEASE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
pub const LEASE_STALE_FLOOR: Duration = Duration::from_secs(30);
pub const DISPATCHER_LOCK_REL: &str = ".locks/dispatcher.lock";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunLease {
    pub run_id: String,
    pub pid: i32,
    pub pid_start_time: DateTime<Utc>,
    pub started_at: DateTime<Utc>,
    pub heartbeat_at: DateTime<Utc>,
    pub host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatcherLockOwner {
    pub pid: i32,
    pub host: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct DispatcherLockGuard {
    pub path: PathBuf,
}

impl Drop for DispatcherLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn lock_owner_path(lock_dir: &Path) -> PathBuf {
    lock_dir.join("owner.json")
}

pub fn lease_path(card_dir: &Path) -> PathBuf {
    card_dir.join("logs").join("lease.json")
}

pub fn acquire_dispatcher_lock(cards_dir: &Path) -> anyhow::Result<DispatcherLockGuard> {
    let lock_dir = cards_dir.join(DISPATCHER_LOCK_REL);
    if let Some(parent) = lock_dir.parent() {
        fs::create_dir_all(parent)?;
    }

    let owner = DispatcherLockOwner {
        pid: std::process::id() as i32,
        host: util::host_name(),
        started_at: Utc::now(),
    };
    let owner_json = serde_json::to_vec_pretty(&owner)?;

    for _ in 0..2 {
        match fs::create_dir(&lock_dir) {
            Ok(()) => {
                if let Err(err) = fs::write(lock_owner_path(&lock_dir), &owner_json) {
                    let _ = fs::remove_dir_all(&lock_dir);
                    return Err(err.into());
                }
                return Ok(DispatcherLockGuard { path: lock_dir });
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                let lock_owner = fs::read(lock_owner_path(&lock_dir))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<DispatcherLockOwner>(&bytes).ok());

                let stale = lock_owner
                    .as_ref()
                    .map(|o| !util::pid_is_alive_sync(o.pid))
                    .unwrap_or(true);
                if stale {
                    let _ = fs::remove_dir_all(&lock_dir);
                    continue;
                }

                if let Some(owner) = lock_owner {
                    anyhow::bail!(
                        "dispatcher lock already held by pid {} on {} (started {})",
                        owner.pid,
                        owner.host,
                        owner.started_at
                    );
                }
                anyhow::bail!(
                    "dispatcher lock already exists at {}; remove stale lock if no dispatcher is running",
                    lock_dir.display()
                );
            }
            Err(err) => return Err(err.into()),
        }
    }

    anyhow::bail!(
        "failed to acquire dispatcher lock at {}",
        lock_dir.display()
    )
}

pub fn write_run_lease(card_dir: &Path, lease: &RunLease) -> anyhow::Result<()> {
    let path = lease_path(card_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(lease)?)?;
    Ok(())
}

pub fn read_run_lease(card_dir: &Path) -> Option<RunLease> {
    let bytes = fs::read(lease_path(card_dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn lease_is_stale(lease: &RunLease, stale_after: ChronoDuration) -> bool {
    Utc::now().signed_duration_since(lease.heartbeat_at) > stale_after
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_lease() -> RunLease {
        RunLease {
            run_id: "run-001".to_string(),
            pid: std::process::id() as i32,
            pid_start_time: Utc::now(),
            started_at: Utc::now(),
            heartbeat_at: Utc::now(),
            host: "testhost".to_string(),
        }
    }

    #[test]
    fn run_lease_serialization_roundtrip() {
        let lease = sample_lease();
        let json = serde_json::to_vec_pretty(&lease).unwrap();
        let decoded: RunLease = serde_json::from_slice(&json).unwrap();
        assert_eq!(decoded.run_id, lease.run_id);
        assert_eq!(decoded.pid, lease.pid);
        assert_eq!(decoded.host, lease.host);
    }

    #[test]
    fn write_and_read_run_lease() {
        let td = tempdir().unwrap();
        let card_dir = td.path().join("test.jobcard");
        fs::create_dir_all(card_dir.join("logs")).unwrap();

        let lease = sample_lease();
        write_run_lease(&card_dir, &lease).unwrap();

        let read_back = read_run_lease(&card_dir).unwrap();
        assert_eq!(read_back.run_id, "run-001");
        assert_eq!(read_back.host, "testhost");
    }

    #[test]
    fn write_run_lease_creates_logs_dir() {
        let td = tempdir().unwrap();
        let card_dir = td.path().join("test.jobcard");
        // Don't create logs/ — write_run_lease should do it
        let lease = sample_lease();
        write_run_lease(&card_dir, &lease).unwrap();
        assert!(lease_path(&card_dir).exists());
    }

    #[test]
    fn read_run_lease_returns_none_for_missing() {
        let td = tempdir().unwrap();
        assert!(read_run_lease(td.path()).is_none());
    }

    #[test]
    fn read_run_lease_returns_none_for_corrupt_json() {
        let td = tempdir().unwrap();
        let card_dir = td.path().join("card.jobcard");
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        fs::write(lease_path(&card_dir), "not valid json!!!").unwrap();
        assert!(read_run_lease(&card_dir).is_none());
    }

    #[test]
    fn lease_is_stale_returns_true_for_old_heartbeat() {
        let mut lease = sample_lease();
        lease.heartbeat_at = Utc::now() - ChronoDuration::seconds(120);
        assert!(lease_is_stale(&lease, ChronoDuration::seconds(60)));
    }

    #[test]
    fn lease_is_stale_returns_false_for_fresh_heartbeat() {
        let lease = sample_lease(); // heartbeat_at = now
        assert!(!lease_is_stale(&lease, ChronoDuration::seconds(60)));
    }

    #[test]
    fn lease_is_stale_edge_at_threshold() {
        // Just barely past threshold — should be stale
        let mut lease = sample_lease();
        lease.heartbeat_at = Utc::now() - ChronoDuration::seconds(61);
        assert!(lease_is_stale(&lease, ChronoDuration::seconds(60)));

        // Just under threshold — should be fresh
        let mut lease2 = sample_lease();
        lease2.heartbeat_at = Utc::now() - ChronoDuration::seconds(59);
        assert!(!lease_is_stale(&lease2, ChronoDuration::seconds(60)));
    }

    #[test]
    fn acquire_dispatcher_lock_succeeds_when_no_lock() {
        let td = tempdir().unwrap();
        let cards_dir = td.path();
        let guard = acquire_dispatcher_lock(cards_dir).unwrap();
        assert!(guard.path.exists());
        // owner.json should exist
        assert!(lock_owner_path(&guard.path).exists());
    }

    #[test]
    fn acquire_dispatcher_lock_fails_when_held_by_live_pid() {
        let td = tempdir().unwrap();
        let cards_dir = td.path();
        // Acquire once
        let _guard = acquire_dispatcher_lock(cards_dir).unwrap();
        // Second attempt should fail (our own PID is alive)
        let result = acquire_dispatcher_lock(cards_dir);
        assert!(result.is_err());
    }

    #[test]
    fn acquire_dispatcher_lock_reclaims_dead_pid_lock() {
        let td = tempdir().unwrap();
        let cards_dir = td.path();
        let lock_dir = cards_dir.join(DISPATCHER_LOCK_REL);
        fs::create_dir_all(&lock_dir).unwrap();

        // Write an owner with a PID that's almost certainly dead
        let dead_owner = DispatcherLockOwner {
            pid: 999_999,
            host: "ghost".to_string(),
            started_at: Utc::now(),
        };
        fs::write(
            lock_owner_path(&lock_dir),
            serde_json::to_vec_pretty(&dead_owner).unwrap(),
        )
        .unwrap();

        // Should reclaim stale lock
        let guard = acquire_dispatcher_lock(cards_dir).unwrap();
        assert!(guard.path.exists());
    }

    #[test]
    fn dispatcher_lock_guard_drop_releases_lock() {
        let td = tempdir().unwrap();
        let cards_dir = td.path();
        let lock_path;
        {
            let guard = acquire_dispatcher_lock(cards_dir).unwrap();
            lock_path = guard.path.clone();
            assert!(lock_path.exists());
        }
        // After drop, lock dir should be gone
        assert!(!lock_path.exists());
    }

    #[test]
    fn lease_path_returns_correct_path() {
        let p = lease_path(Path::new("/tmp/card.jobcard"));
        assert_eq!(p, PathBuf::from("/tmp/card.jobcard/logs/lease.json"));
    }

    #[test]
    fn lock_owner_path_returns_correct_path() {
        let p = lock_owner_path(Path::new("/tmp/lock"));
        assert_eq!(p, PathBuf::from("/tmp/lock/owner.json"));
    }
}
