use anyhow::Context;
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use crate::quicklook;
use crate::util;

pub fn ensure_cards_layout(root: &Path) -> anyhow::Result<()> {
    for dir in [
        "templates",
        "drafts",
        "pending",
        "running",
        "done",
        "merged",
        "failed",
        "memory",
    ] {
        fs::create_dir_all(root.join(dir))?;
    }
    Ok(())
}

pub fn clone_template(src: &Path, dst: &Path) -> anyhow::Result<()> {
    // Ensure destination parent directory exists
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }

    // Prefer APFS clone semantics on macOS (ditto --clone), then cp -c.
    if cfg!(target_os = "macos") {
        let status = StdCommand::new("ditto")
            .arg("--clone")
            .arg(src)
            .arg(dst)
            .status();
        if matches!(status, Ok(s) if s.success()) {
            return Ok(());
        }

        let status = StdCommand::new("cp")
            .arg("-c") // COW clone on APFS
            .arg("-R") // Recursive
            .arg(src)
            .arg(dst)
            .status();
        if matches!(status, Ok(s) if s.success()) {
            return Ok(());
        }

        anyhow::bail!(
            "APFS clone copy failed (required on macOS): {} -> {}",
            src.display(),
            dst.display()
        );
    } else {
        // Try Btrfs reflink on Linux
        let status = StdCommand::new("cp")
            .arg("--reflink=auto") // Try COW, fallback to regular copy
            .arg("-r")
            .arg(src)
            .arg(dst)
            .status();
        if matches!(status, Ok(s) if s.success()) {
            return Ok(());
        }

        // Fallback to regular copy if reflink fails
        let status = StdCommand::new("cp").arg("-r").arg(src).arg(dst).status();
        if matches!(status, Ok(s) if s.success()) {
            return Ok(());
        }
    }

    // Final fallback to manual copy
    util::copy_dir_all(src, dst)
}

/// Copy a single file using APFS COW clone when possible.
/// On macOS this requires `cp -c`; on other OSes it falls back to regular copy.
pub fn cow_copy_file(src: &Path, dst: &Path) -> anyhow::Result<()> {
    if cfg!(target_os = "macos") {
        let status = StdCommand::new("cp")
            .arg("-c") // APFS clone
            .arg(src)
            .arg(dst)
            .status();
        if matches!(status, Ok(s) if s.success()) {
            return Ok(());
        }
        anyhow::bail!(
            "APFS COW copy failed (required on macOS): {} -> {}",
            src.display(),
            dst.display()
        );
    }
    fs::copy(src, dst)
        .with_context(|| format!("failed to copy {} -> {}", src.display(), dst.display()))?;
    Ok(())
}

pub fn find_card(root: &Path, id: &str) -> Option<PathBuf> {
    let suffix = format!("-{}.jobcard", id);
    let exact = format!("{}.jobcard", id);
    for dir in ["drafts", "pending", "running", "done", "merged", "failed"] {
        let state_dir = root.join(dir);
        // Exact match (legacy / no-glyph prefix)
        let p = state_dir.join(&exact);
        if p.exists() {
            return Some(p);
        }
        // Glyph-prefixed match: {glyph}-{id}.jobcard
        if let Ok(entries) = fs::read_dir(&state_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir()
                    && path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.ends_with(&suffix))
                        .unwrap_or(false)
                {
                    return Some(path);
                }
            }
        }
    }
    None
}

pub fn find_card_in_dir(state_dir: &Path, id: &str) -> Option<PathBuf> {
    let exact = state_dir.join(format!("{}.jobcard", id));
    if exact.exists() {
        return Some(exact);
    }
    let suffix = format!("-{}.jobcard", id);
    fs::read_dir(state_dir).ok()?.flatten().find_map(|e| {
        let p = e.path();
        if p.is_dir()
            && p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(&suffix))
        {
            Some(p)
        } else {
            None
        }
    })
}

pub fn card_exists_in(root: &Path, state: &str, id: &str) -> bool {
    let state_dir = root.join(state);
    let exact = format!("{}.jobcard", id);
    if state_dir.join(&exact).exists() {
        return true;
    }
    let suffix = format!("-{}.jobcard", id);
    fs::read_dir(&state_dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            e.path().is_dir()
                && e.file_name()
                    .to_str()
                    .map(|n| n.ends_with(&suffix))
                    .unwrap_or(false)
        })
}

pub fn find_card_in_state(root: &Path, id: &str, state: &str) -> bool {
    let state_dir = root.join(state);
    if state_dir.join(format!("{}.jobcard", id)).exists() {
        return true;
    }
    let suffix = format!("-{}.jobcard", id);
    fs::read_dir(&state_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            e.path()
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(&suffix))
                .unwrap_or(false)
        })
}

pub fn quarantine_invalid_pending_card(
    pending_path: &Path,
    failed_dir: &Path,
    reason: &str,
) -> anyhow::Result<()> {
    let name = pending_path
        .file_name()
        .and_then(|n| n.to_str())
        .with_context(|| format!("invalid card path: {}", pending_path.display()))?;
    let failed_path = util::unique_failed_path(failed_dir, name);
    fs::rename(pending_path, &failed_path).with_context(|| {
        format!(
            "failed moving invalid pending card {} to failed/",
            pending_path.display()
        )
    })?;
    fs::create_dir_all(failed_path.join("logs"))?;
    fs::create_dir_all(failed_path.join("output"))?;
    let marker = format!("[{}] dispatcher rejected card: {}\n", Utc::now(), reason);
    util::append_log_line(
        &failed_path.join("logs").join("rejected.log"),
        marker.trim_end(),
    )?;
    let _ = fs::write(failed_path.join("output").join("qa_report.md"), marker);
    quicklook::render_card_thumbnail(&failed_path);
    Ok(())
}
