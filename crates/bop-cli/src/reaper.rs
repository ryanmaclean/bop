use chrono::Duration as ChronoDuration;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command as TokioCommand;

use bop_core::{write_meta, StageStatus};

use crate::{lock, quicklook};

pub async fn reap_orphans(
    running_dir: &Path,
    pending_dir: &Path,
    failed_dir: &Path,
    max_retries: u32,
    stale_lease_after: Duration,
) -> anyhow::Result<()> {
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

        let pid = read_pid(&card_dir).await?;
        let pid_dead = match pid {
            Some(pid) => !is_alive(pid).await?,
            None => false,
        };
        let lease = lock::read_run_lease(&card_dir);
        let lease_stale = lease
            .as_ref()
            .map(|l| lock::lease_is_stale(l, stale_after_chrono))
            .unwrap_or(false);
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
    let out = TokioCommand::new("xattr")
        .arg("-p")
        .arg("sh.bop.agent-pid")
        .arg(card_dir)
        .output()
        .await;
    if let Ok(out) = out {
        if out.status.success() {
            if let Ok(s) = String::from_utf8(out.stdout) {
                if let Ok(pid) = s.trim().parse::<i32>() {
                    return Ok(Some(pid));
                }
            }
        }
    }

    let pid_path = card_dir.join("logs").join("pid");
    if let Ok(s) = fs::read_to_string(pid_path) {
        if let Ok(pid) = s.trim().parse::<i32>() {
            return Ok(Some(pid));
        }
    }

    if let Some(lease) = lock::read_run_lease(card_dir) {
        return Ok(Some(lease.pid));
    }

    Ok(None)
}

pub async fn is_alive(pid: i32) -> anyhow::Result<bool> {
    let status = TokioCommand::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .await?;
    Ok(status.success())
}

pub async fn recover_orphans(
    running_dir: &Path,
    pending_dir: &Path,
) -> anyhow::Result<Vec<String>> {
    let mut recovered = Vec::new();
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

        let pid = read_pid(&card_dir).await?;
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
