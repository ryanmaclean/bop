use chrono::Duration as ChronoDuration;
use std::fs;
use std::path::Path;
use std::time::Duration;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
use std::ffi::CString;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
use std::os::unix::ffi::OsStrExt;

use bop_core::{write_meta, StageStatus};

use crate::{decision::DecisionPlaneConfig, lock, quicklook};

pub async fn reap_orphans(
    running_dir: &Path,
    pending_dir: &Path,
    failed_dir: &Path,
    max_retries: u32,
    stale_lease_after: Duration,
) -> anyhow::Result<()> {
    let config = bop_core::load_config().ok();
    let decision_cfg = DecisionPlaneConfig::from_dispatch_config(
        config.as_ref().and_then(|cfg| cfg.dispatch.as_ref()),
    );
    let stale_after_chrono =
        ChronoDuration::from_std(stale_lease_after).unwrap_or_else(|_| ChronoDuration::seconds(30));
    let entries = match fs::read_dir(running_dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for ent in entries.flatten() {
        let card_dir = ent.path();
        if !card_dir.is_dir() {
            continue;
        }
        if card_dir.extension().and_then(|s| s.to_str()).unwrap_or("") != "bop" {
            continue;
        }

        let pid = match read_pid(&card_dir).await {
            Ok(pid) => pid,
            Err(error) => {
                eprintln!("skipping card {} with conflicting PID evidence: {error}", card_dir.display());
                continue;
            }
        };
        let pid_dead = match pid {
            Some(pid) => !is_alive(pid).await?,
            None => false,
        };
        let lease = lock::read_run_lease(&card_dir);
        let lease_stale = lease
            .as_ref()
            .map(|l| lock::lease_is_stale(l, stale_after_chrono))
            .unwrap_or(false);
        // A stale heartbeat cannot prove that a live process has exited.
        // Moving this card would allow a second adapter to start alongside it.
        if pid.is_some() && !pid_dead {
            continue;
        }
        if !pid_dead && !lease_stale {
            continue;
        }

        let mut meta = bop_core::read_meta(&card_dir).ok();
        let retry_count = meta.as_ref().and_then(|m| m.retry_count).unwrap_or(0);
        let next_retry = retry_count.saturating_add(1);
        let move_to_failed = next_retry > max_retries;
        if let Some(ref mut m) = meta {
            m.retry_count = Some(next_retry);
            if move_to_failed {
                m.failure_reason = Some("max_retries_exceeded".to_string());
            } else {
                m.failure_reason = None;
            }
            for stage in m.stages.values_mut() {
                if stage.status == StageStatus::Running {
                    stage.status = if move_to_failed {
                        StageStatus::Failed
                    } else {
                        StageStatus::Pending
                    };
                    stage.agent = None;
                    stage.provider = None;
                    stage.duration_s = None;
                    stage.started = None;
                    stage.blocked_by = None;
                }
            }
            crate::decision::record_orphan_recovery(
                m,
                &decision_cfg,
                pid_dead,
                lease_stale,
                move_to_failed,
            );
            let _ = write_meta(&card_dir, m);
        }

        let name = match card_dir.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let target = if move_to_failed {
            failed_dir.join(&name)
        } else {
            pending_dir.join(&name)
        };
        let _ = fs::rename(&card_dir, &target);
        quicklook::render_card_thumbnail(&target);
    }

    Ok(())
}

pub async fn read_pid(card_dir: &Path) -> anyhow::Result<Option<i32>> {
    let pid_path = card_dir.join("logs").join("pid");
    let file_pid = fs::read_to_string(pid_path)
        .ok()
        .and_then(|text| parse_safe_pid(&text));
    let lease_pid = lock::read_run_lease(card_dir)
        .and_then(|lease| (lease.pid > 1).then_some(lease.pid));

    // The old dispatcher could update only some of these best-effort stores.
    // Every valid source is evidence; disagreement must not pick a PID to
    // signal or use for orphan recovery.
    let xattr_pid = read_legacy_xattr_pid(card_dir)?;
    resolve_pid_sources(file_pid, lease_pid, xattr_pid)
}

fn resolve_pid_sources(
    file_pid: Option<i32>,
    lease_pid: Option<i32>,
    xattr_pid: Option<i32>,
) -> anyhow::Result<Option<i32>> {
    let mut chosen: Option<(&str, i32)> = None;
    for (source, pid) in [
        ("PID file", file_pid),
        ("lease", lease_pid),
        ("legacy xattr", xattr_pid),
    ] {
        if let Some(pid) = pid {
            if let Some((old_source, old_pid)) = chosen {
                if old_pid != pid {
                    anyhow::bail!(
                        "conflicting card PIDs: {old_source} ({old_pid}) and {source} ({pid})"
                    );
                }
            } else {
                chosen = Some((source, pid));
            }
        }
    }
    Ok(chosen.map(|(_, pid)| pid))
}

fn parse_safe_pid(text: &str) -> Option<i32> {
    text.trim().parse::<i32>().ok().filter(|pid| *pid > 1)
}

fn parse_legacy_xattr_bytes(value: &[u8]) -> Option<i32> {
    if value.len() > 32 {
        return None;
    }
    std::str::from_utf8(value).ok().and_then(parse_safe_pid)
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
fn read_legacy_xattr_pid(card_dir: &Path) -> anyhow::Result<Option<i32>> {
    let path = CString::new(card_dir.as_os_str().as_bytes())
        .map_err(|_| anyhow::anyhow!("card path contains NUL"))?;
    let mut value = [0_u8; 32];
    #[cfg(target_os = "linux")]
    let name: &[u8] = b"user.sh.bop.agent-pid\0";
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    let name: &[u8] = b"sh.bop.agent-pid\0";

    // The target-specific libc ABI is different on each platform. The
    // attribute contains a decimal PID, so an oversized value is invalid.
    #[cfg(target_os = "linux")]
    let len = unsafe {
        libc::getxattr(
            path.as_ptr(),
            name.as_ptr().cast(),
            value.as_mut_ptr().cast(),
            value.len(),
        )
    };
    #[cfg(target_os = "macos")]
    let len = unsafe {
        libc::getxattr(
            path.as_ptr(),
            name.as_ptr().cast(),
            value.as_mut_ptr().cast(),
            value.len(),
            0,
            0,
        )
    };
    #[cfg(target_os = "freebsd")]
    let len = unsafe {
        libc::extattr_get_file(
            path.as_ptr(),
            libc::EXTATTR_NAMESPACE_USER,
            name.as_ptr().cast(),
            value.as_mut_ptr().cast(),
            value.len(),
        )
    };
    if len < 0 {
        let error = std::io::Error::last_os_error();
        #[cfg(target_os = "linux")]
        let missing = libc::ENODATA;
        #[cfg(any(target_os = "macos", target_os = "freebsd"))]
        let missing = libc::ENOATTR;
        // ERANGE means the attribute exceeds our 32-byte decimal PID schema.
        if matches!(error.raw_os_error(), Some(code) if code == missing || code == libc::ERANGE || code == libc::ENOTSUP) {
            return Ok(None);
        }
        return Err(error.into());
    }
    let len = usize::try_from(len)?;
    let bytes = value
        .get(..len)
        .ok_or_else(|| anyhow::anyhow!("legacy PID xattr length exceeds buffer"))?;
    Ok(parse_legacy_xattr_bytes(bytes))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "freebsd")))]
fn read_legacy_xattr_pid(_card_dir: &Path) -> anyhow::Result<Option<i32>> {
    Ok(None)
}

pub fn refresh_legacy_xattr_pid(card_dir: &Path, pid: i32) -> anyhow::Result<()> {
    if pid <= 1 {
        anyhow::bail!("unsafe PID for legacy xattr refresh: {pid}");
    }
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
    {
        let path = CString::new(card_dir.as_os_str().as_bytes())
            .map_err(|_| anyhow::anyhow!("card path contains NUL"))?;
        let value = pid.to_string();
        #[cfg(target_os = "linux")]
        let name: &[u8] = b"user.sh.bop.agent-pid\0";
        #[cfg(any(target_os = "macos", target_os = "freebsd"))]
        let name: &[u8] = b"sh.bop.agent-pid\0";

        #[cfg(target_os = "linux")]
        let written = unsafe {
            libc::setxattr(
                path.as_ptr(),
                name.as_ptr().cast(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        };
        #[cfg(target_os = "macos")]
        let written = unsafe {
            libc::setxattr(
                path.as_ptr(),
                name.as_ptr().cast(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        #[cfg(target_os = "freebsd")]
        let written = unsafe {
            libc::extattr_set_file(
                path.as_ptr(),
                libc::EXTATTR_NAMESPACE_USER,
                name.as_ptr().cast(),
                value.as_ptr().cast(),
                value.len(),
            )
        };
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if written != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        #[cfg(target_os = "freebsd")]
        if written != value.len() as isize {
            if written < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            anyhow::bail!("legacy PID xattr short write: {written} of {} bytes", value.len());
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "freebsd")))]
    {
        let _ = card_dir;
    }
    Ok(())
}

pub async fn is_alive(pid: i32) -> anyhow::Result<bool> {
    if pid <= 1 {
        anyhow::bail!("unsafe PID for liveness check: {}", pid);
    }
    #[cfg(unix)]
    {
        if unsafe { libc::kill(pid, 0) } == 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        return match error.raw_os_error() {
            Some(libc::EPERM) => Ok(true),
            Some(libc::ESRCH) => Ok(false),
            _ => Err(error.into()),
        };
    }
    #[cfg(not(unix))]
    anyhow::bail!("native PID liveness is unsupported on this platform")
}

pub async fn recover_orphans(
    running_dir: &Path,
    pending_dir: &Path,
) -> anyhow::Result<Vec<String>> {
    let mut recovered = Vec::new();
    let config = bop_core::load_config().ok();
    let decision_cfg = DecisionPlaneConfig::from_dispatch_config(
        config.as_ref().and_then(|cfg| cfg.dispatch.as_ref()),
    );
    let entries = match fs::read_dir(running_dir) {
        Ok(e) => e,
        Err(_) => return Ok(recovered),
    };

    for ent in entries.flatten() {
        let card_dir = ent.path();
        if !card_dir.is_dir() {
            continue;
        }
        if card_dir.extension().and_then(|s| s.to_str()).unwrap_or("") != "bop" {
            continue;
        }

        let pid = match read_pid(&card_dir).await {
            Ok(pid) => pid,
            Err(error) => {
                eprintln!("skipping card {} with conflicting PID evidence: {error}", card_dir.display());
                continue;
            }
        };
        let pid_dead = match pid {
            Some(pid) => !is_alive(pid).await?,
            None => true, // No PID means orphaned
        };

        if !pid_dead {
            continue;
        }

        // Try to read meta.json, create minimal one if corrupt/missing
        let meta = match bop_core::read_meta(&card_dir) {
            Ok(m) => m,
            Err(_) => {
                // Corrupt or missing meta.json - create minimal recovery meta
                let id = card_dir
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                let minimal_meta = bop_core::Meta {
                    id: id.clone(),
                    stage: "pending".to_string(),
                    ..Default::default()
                };
                let _ = bop_core::write_meta(&card_dir, &minimal_meta);
                minimal_meta
            }
        };
        let mut meta = meta;
        crate::decision::record_orphan_recovery(&mut meta, &decision_cfg, pid_dead, false, false);
        let _ = bop_core::write_meta(&card_dir, &meta);

        let name = match card_dir.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let target = pending_dir.join(&name);
        let _ = fs::rename(&card_dir, &target);
        quicklook::render_card_thumbnail(&target);
        recovered.push(meta.id);
    }

    Ok(recovered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bop_core::Meta;
    use tempfile::tempdir;

    // ── read_pid ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn read_pid_falls_back_to_logs_pid_file() {
        let td = tempdir().unwrap();
        let card_dir = td.path().join("test.bop");
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        fs::write(card_dir.join("logs").join("pid"), "12345").unwrap();
        let pid = read_pid(&card_dir).await.unwrap();
        assert_eq!(pid, Some(12345));
    }

    #[tokio::test]
    async fn read_pid_falls_back_to_lease() {
        let td = tempdir().unwrap();
        let card_dir = td.path().join("test.bop");
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        let lease = lock::RunLease {
            run_id: "test-run".to_string(),
            pid: 54321,
            pid_start_time: chrono::Utc::now(),
            started_at: chrono::Utc::now(),
            heartbeat_at: chrono::Utc::now(),
            host: "test-host".to_string(),
        };
        lock::write_run_lease(&card_dir, &lease).unwrap();
        let pid = read_pid(&card_dir).await.unwrap();
        assert_eq!(pid, Some(54321));
    }

    #[tokio::test]
    async fn read_pid_returns_none_when_no_source() {
        let td = tempdir().unwrap();
        let card_dir = td.path().join("test.bop");
        fs::create_dir_all(&card_dir).unwrap();
        let pid = read_pid(&card_dir).await.unwrap();
        assert_eq!(pid, None);
    }

    #[tokio::test]
    async fn read_pid_accepts_matching_pid_file_and_lease() {
        let td = tempdir().unwrap();
        let card_dir = td.path().join("test.bop");
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        fs::write(card_dir.join("logs").join("pid"), "12345").unwrap();
        let lease = lock::RunLease {
            run_id: "older-run".to_string(),
            pid: 12345,
            pid_start_time: chrono::Utc::now(),
            started_at: chrono::Utc::now(),
            heartbeat_at: chrono::Utc::now(),
            host: "test-host".to_string(),
        };
        lock::write_run_lease(&card_dir, &lease).unwrap();
        assert_eq!(read_pid(&card_dir).await.unwrap(), Some(12345));
    }

    #[tokio::test]
    async fn read_pid_rejects_conflicting_pid_file_and_lease() {
        let td = tempdir().unwrap();
        let card_dir = td.path().join("test.bop");
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        fs::write(card_dir.join("logs").join("pid"), "12345").unwrap();
        let lease = lock::RunLease {
            run_id: "other-run".to_string(),
            pid: 54321,
            pid_start_time: chrono::Utc::now(),
            started_at: chrono::Utc::now(),
            heartbeat_at: chrono::Utc::now(),
            host: "test-host".to_string(),
        };
        lock::write_run_lease(&card_dir, &lease).unwrap();
        assert!(read_pid(&card_dir).await.is_err());
    }

    #[test]
    fn pid_sources_quarantine_legacy_xattr_disagreement() {
        // A pre-upgrade run can update xattr while file or lease writes fail.
        assert!(resolve_pid_sources(Some(1111), None, Some(2222)).is_err());
        assert!(resolve_pid_sources(None, Some(1111), Some(2222)).is_err());
        // A stale xattr from an earlier run must not override two current files.
        assert!(resolve_pid_sources(Some(2222), Some(2222), Some(1111)).is_err());
    }

    #[test]
    fn pid_sources_accept_matching_or_single_evidence() {
        assert_eq!(
            resolve_pid_sources(Some(2222), Some(2222), Some(2222)).unwrap(),
            Some(2222)
        );
        assert_eq!(resolve_pid_sources(None, None, Some(2222)).unwrap(), Some(2222));
        assert_eq!(resolve_pid_sources(None, None, None).unwrap(), None);
    }

    #[test]
    fn malformed_or_oversize_xattr_is_not_a_valid_pid() {
        assert_eq!(parse_legacy_xattr_bytes(b"not-a-pid"), None);
        assert_eq!(parse_legacy_xattr_bytes(b"123\0"), None);
        assert_eq!(
            parse_legacy_xattr_bytes(b"000000000000000000000000000000012"),
            None
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
    #[tokio::test]
    #[ignore = "run on an approved off-i9 native xattr-capable filesystem"]
    async fn native_xattr_conflict_and_refresh_coexistence() {
        let td = tempdir().unwrap();
        let card_dir = td.path().join("test.bop");
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        fs::write(card_dir.join("logs").join("pid"), "2222").unwrap();
        refresh_legacy_xattr_pid(&card_dir, 1111).unwrap();
        assert!(read_pid(&card_dir).await.is_err());

        // Force a refresh error while retaining the old xattr. A later
        // reader must keep quarantining the conflicting PID evidence.
        let moved = td.path().join("moved.bop");
        fs::rename(&card_dir, &moved).unwrap();
        assert!(refresh_legacy_xattr_pid(&card_dir, 2222).is_err());
        fs::rename(&moved, &card_dir).unwrap();
        assert!(read_pid(&card_dir).await.is_err());

        refresh_legacy_xattr_pid(&card_dir, 2222).unwrap();
        assert_eq!(read_pid(&card_dir).await.unwrap(), Some(2222));
    }

    #[tokio::test]
    async fn read_pid_rejects_group_and_init_pids() {
        let td = tempdir().unwrap();
        let card_dir = td.path().join("test.bop");
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        for bad in ["0", "-2", "1"] {
            fs::write(card_dir.join("logs").join("pid"), bad).unwrap();
            assert_eq!(read_pid(&card_dir).await.unwrap(), None);
        }
    }

    #[tokio::test]
    async fn is_alive_rejects_group_pid() {
        assert!(is_alive(0).await.is_err());
        assert!(is_alive(-2).await.is_err());
    }

    // ── is_alive ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn is_alive_returns_true_for_own_pid() {
        let pid = std::process::id() as i32;
        assert!(is_alive(pid).await.unwrap());
    }

    #[tokio::test]
    async fn is_alive_returns_false_for_dead_pid() {
        assert!(!is_alive(999999).await.unwrap());
    }

    // ── reap_orphans ──────────────────────────────────────────────────────────

    fn setup_card_dirs(td: &Path) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let running = td.join("running");
        let pending = td.join("pending");
        let failed = td.join("failed");
        fs::create_dir_all(&running).unwrap();
        fs::create_dir_all(&pending).unwrap();
        fs::create_dir_all(&failed).unwrap();
        (running, pending, failed)
    }

    fn test_meta(id: &str, retry_count: Option<u32>) -> Meta {
        Meta {
            id: id.to_string(),
            stage: "implement".to_string(),
            retry_count,
            ..Default::default()
        }
    }

    fn create_running_card(running_dir: &Path, name: &str, pid: i32, meta: &Meta) {
        let card_dir = running_dir.join(format!("{}.bop", name));
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        fs::write(card_dir.join("logs").join("pid"), pid.to_string()).unwrap();
        write_meta(&card_dir, meta).unwrap();
    }

    #[tokio::test]
    async fn reap_orphans_keeps_live_pid_even_with_stale_lease() {
        let td = tempdir().unwrap();
        let (running, pending, failed) = setup_card_dirs(td.path());
        let meta = test_meta("live-stale", Some(0));
        let pid = std::process::id() as i32;
        create_running_card(&running, "live-stale", pid, &meta);
        let card_dir = running.join("live-stale.bop");
        let old = chrono::Utc::now() - chrono::Duration::minutes(5);
        let lease = lock::RunLease {
            run_id: "live-run".to_string(),
            pid,
            pid_start_time: old,
            started_at: old,
            heartbeat_at: old,
            host: "test-host".to_string(),
        };
        lock::write_run_lease(&card_dir, &lease).unwrap();

        reap_orphans(&running, &pending, &failed, 3, Duration::from_secs(30))
            .await
            .unwrap();

        assert!(card_dir.exists());
        assert!(!pending.join("live-stale.bop").exists());
    }

    #[tokio::test]
    async fn reap_orphans_skips_conflict_and_recovers_other_card() {
        let td = tempdir().unwrap();
        let (running, pending, failed) = setup_card_dirs(td.path());
        let meta = test_meta("conflict", Some(0));
        create_running_card(&running, "conflict", 12345, &meta);
        let conflict = running.join("conflict.bop");
        let now = chrono::Utc::now();
        let lease = lock::RunLease {
            run_id: "other-run".to_string(),
            pid: 54321,
            pid_start_time: now,
            started_at: now,
            heartbeat_at: now,
            host: "test-host".to_string(),
        };
        lock::write_run_lease(&conflict, &lease).unwrap();
        create_running_card(&running, "dead", 999999, &test_meta("dead", Some(0)));

        reap_orphans(&running, &pending, &failed, 3, Duration::from_secs(30))
            .await
            .unwrap();

        assert!(conflict.exists());
        assert!(!pending.join("conflict.bop").exists());
        assert!(pending.join("dead.bop").exists());
    }

    #[tokio::test]
    async fn reap_orphans_moves_dead_pid_card_to_pending() {
        let td = tempdir().unwrap();
        let (running, pending, failed) = setup_card_dirs(td.path());

        let meta = test_meta("test-card", Some(0));
        create_running_card(&running, "test-card", 999999, &meta);

        reap_orphans(&running, &pending, &failed, 3, Duration::from_secs(30))
            .await
            .unwrap();

        assert!(pending.join("test-card.bop").exists());
        assert!(!running.join("test-card.bop").exists());
    }

    #[tokio::test]
    async fn reap_orphans_moves_to_failed_when_max_retries_exceeded() {
        let td = tempdir().unwrap();
        let (running, pending, failed) = setup_card_dirs(td.path());

        let meta = test_meta("retry-card", Some(3)); // already at max
        create_running_card(&running, "retry-card", 999999, &meta);

        reap_orphans(
            &running,
            &pending,
            &failed,
            3, // max_retries = 3, next will be 4 > 3
            Duration::from_secs(30),
        )
        .await
        .unwrap();

        assert!(failed.join("retry-card.bop").exists());
        assert!(!running.join("retry-card.bop").exists());
    }

    #[tokio::test]
    async fn reap_orphans_increments_retry_count() {
        let td = tempdir().unwrap();
        let (running, pending, failed) = setup_card_dirs(td.path());

        let meta = test_meta("inc-card", Some(1));
        create_running_card(&running, "inc-card", 999999, &meta);

        reap_orphans(&running, &pending, &failed, 5, Duration::from_secs(30))
            .await
            .unwrap();

        let moved_meta = bop_core::read_meta(&pending.join("inc-card.bop")).unwrap();
        assert_eq!(moved_meta.retry_count, Some(2));
    }

    #[tokio::test]
    async fn reap_orphans_normalizes_running_stage_to_pending() {
        let td = tempdir().unwrap();
        let (running, pending, failed) = setup_card_dirs(td.path());

        let mut meta = test_meta("stage-card", Some(0));
        meta.stages.insert(
            "implement".to_string(),
            bop_core::StageRecord {
                status: StageStatus::Running,
                agent: Some("test-agent".to_string()),
                provider: Some("test-provider".to_string()),
                duration_s: Some(100),
                started: Some(chrono::Utc::now()),
                blocked_by: None,
            },
        );
        create_running_card(&running, "stage-card", 999999, &meta);

        reap_orphans(&running, &pending, &failed, 5, Duration::from_secs(30))
            .await
            .unwrap();

        let moved_meta = bop_core::read_meta(&pending.join("stage-card.bop")).unwrap();
        let stage = moved_meta.stages.get("implement").unwrap();
        assert_eq!(stage.status, StageStatus::Pending);
        assert!(stage.agent.is_none());
        assert!(stage.provider.is_none());
    }

    #[tokio::test]
    async fn reap_orphans_skips_non_bop_directories() {
        let td = tempdir().unwrap();
        let (running, pending, failed) = setup_card_dirs(td.path());

        // Create a non-bop directory with a dead PID
        let non_card = running.join("something-else");
        fs::create_dir_all(non_card.join("logs")).unwrap();
        fs::write(non_card.join("logs").join("pid"), "999999").unwrap();

        reap_orphans(&running, &pending, &failed, 3, Duration::from_secs(30))
            .await
            .unwrap();

        // Non-bop dir should remain untouched
        assert!(running.join("something-else").exists());
    }

    #[tokio::test]
    async fn reap_orphans_handles_empty_running_dir() {
        let td = tempdir().unwrap();
        let (running, pending, failed) = setup_card_dirs(td.path());

        let result = reap_orphans(&running, &pending, &failed, 3, Duration::from_secs(30)).await;

        assert!(result.is_ok());
    }

    // ── recover_orphans ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn recover_orphans_moves_dead_pid_card_to_pending() {
        let td = tempdir().unwrap();
        let (running, pending, _failed) = setup_card_dirs(td.path());

        let meta = test_meta("orphan-card", None);
        create_running_card(&running, "orphan-card", 999999, &meta);

        let recovered = recover_orphans(&running, &pending).await.unwrap();

        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0], "orphan-card");
        assert!(pending.join("orphan-card.bop").exists());
        assert!(!running.join("orphan-card.bop").exists());
    }

    #[tokio::test]
    async fn recover_orphans_handles_corrupt_meta_json() {
        let td = tempdir().unwrap();
        let (running, pending, _failed) = setup_card_dirs(td.path());

        // Create card with corrupt meta.json
        let card_dir = running.join("corrupt-card.bop");
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        fs::write(card_dir.join("logs").join("pid"), "999999").unwrap();
        fs::write(card_dir.join("meta.json"), "{ invalid json }").unwrap();

        let recovered = recover_orphans(&running, &pending).await.unwrap();

        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0], "corrupt-card");
        assert!(pending.join("corrupt-card.bop").exists());

        // Verify minimal meta was created
        let recovered_meta = bop_core::read_meta(&pending.join("corrupt-card.bop")).unwrap();
        assert_eq!(recovered_meta.id, "corrupt-card");
        assert_eq!(recovered_meta.stage, "pending");
    }

    #[tokio::test]
    async fn recover_orphans_handles_missing_meta_json() {
        let td = tempdir().unwrap();
        let (running, pending, _failed) = setup_card_dirs(td.path());

        // Create card without meta.json
        let card_dir = running.join("missing-meta.bop");
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        fs::write(card_dir.join("logs").join("pid"), "999999").unwrap();

        let recovered = recover_orphans(&running, &pending).await.unwrap();

        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0], "missing-meta");
        assert!(pending.join("missing-meta.bop").exists());

        // Verify minimal meta was created
        let recovered_meta = bop_core::read_meta(&pending.join("missing-meta.bop")).unwrap();
        assert_eq!(recovered_meta.id, "missing-meta");
        assert_eq!(recovered_meta.stage, "pending");
    }

    #[tokio::test]
    async fn recover_orphans_skips_live_pid_cards() {
        let td = tempdir().unwrap();
        let (running, pending, _failed) = setup_card_dirs(td.path());

        // Create card with live PID (own process)
        let meta = test_meta("live-card", None);
        let live_pid = std::process::id() as i32;
        create_running_card(&running, "live-card", live_pid, &meta);

        let recovered = recover_orphans(&running, &pending).await.unwrap();

        assert_eq!(recovered.len(), 0);
        assert!(running.join("live-card.bop").exists());
        assert!(!pending.join("live-card.bop").exists());
    }

    #[tokio::test]
    async fn recover_orphans_handles_empty_running_dir() {
        let td = tempdir().unwrap();
        let (running, pending, _failed) = setup_card_dirs(td.path());

        let recovered = recover_orphans(&running, &pending).await.unwrap();

        assert_eq!(recovered.len(), 0);
    }

    #[tokio::test]
    async fn recover_orphans_handles_card_without_pid_file() {
        let td = tempdir().unwrap();
        let (running, pending, _failed) = setup_card_dirs(td.path());

        // Create card without PID file (treated as orphan)
        let card_dir = running.join("no-pid.bop");
        fs::create_dir_all(&card_dir).unwrap();
        let meta = test_meta("no-pid", None);
        write_meta(&card_dir, &meta).unwrap();

        let recovered = recover_orphans(&running, &pending).await.unwrap();

        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0], "no-pid");
        assert!(pending.join("no-pid.bop").exists());
    }
}
