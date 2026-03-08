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
    let suffix = format!("-{}.bop", id);
    let exact = format!("{}.bop", id);
    for dir in ["drafts", "pending", "running", "done", "merged", "failed"] {
        let state_dir = root.join(dir);
        // Exact match (legacy / no-glyph prefix)
        let p = state_dir.join(&exact);
        if p.exists() {
            return Some(p);
        }
        // Glyph-prefixed match: {glyph}-{id}.bop
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

/// Find a card by ID or return an error with a helpful hint.
pub fn require_card(root: &Path, id: &str) -> anyhow::Result<PathBuf> {
    find_card(root, id).with_context(|| format!("card not found: {}\nTry: bop list", id))
}

pub fn find_card_in_dir(state_dir: &Path, id: &str) -> Option<PathBuf> {
    let exact = state_dir.join(format!("{}.bop", id));
    if exact.exists() {
        return Some(exact);
    }
    let suffix = format!("-{}.bop", id);
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
    let exact = format!("{}.bop", id);
    if state_dir.join(&exact).exists() {
        return true;
    }
    let suffix = format!("-{}.bop", id);
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
    if state_dir.join(format!("{}.bop", id)).exists() {
        return true;
    }
    let suffix = format!("-{}.bop", id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ensure_cards_layout_creates_all_dirs() {
        let td = tempdir().unwrap();
        let root = td.path();
        ensure_cards_layout(root).unwrap();
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
            assert!(root.join(dir).is_dir(), "missing dir: {dir}");
        }
    }

    #[test]
    fn ensure_cards_layout_is_idempotent() {
        let td = tempdir().unwrap();
        let root = td.path();
        ensure_cards_layout(root).unwrap();
        // Place a marker file to verify it isn't destroyed
        fs::write(root.join("pending").join("marker.txt"), "ok").unwrap();
        ensure_cards_layout(root).unwrap();
        assert!(root.join("pending").join("marker.txt").exists());
    }

    #[test]
    fn clone_template_copies_dir_recursively() {
        let td = tempdir().unwrap();
        let src = td.path().join("tmpl");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), "hello").unwrap();
        fs::write(src.join("sub").join("b.txt"), "world").unwrap();

        let dst = td.path().join("out");
        clone_template(&src, &dst).unwrap();

        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
        assert_eq!(
            fs::read_to_string(dst.join("sub").join("b.txt")).unwrap(),
            "world"
        );
    }

    #[test]
    fn clone_template_errors_on_missing_source() {
        let td = tempdir().unwrap();
        let src = td.path().join("nonexistent");
        let dst = td.path().join("out");
        assert!(clone_template(&src, &dst).is_err());
    }

    #[test]
    fn cow_copy_file_copies_contents() {
        let td = tempdir().unwrap();
        let src = td.path().join("src.txt");
        let dst = td.path().join("dst.txt");
        fs::write(&src, "cow data").unwrap();
        cow_copy_file(&src, &dst).unwrap();
        assert_eq!(fs::read_to_string(&dst).unwrap(), "cow data");
    }

    #[test]
    fn find_card_locates_exact_match() {
        let td = tempdir().unwrap();
        let root = td.path();
        ensure_cards_layout(root).unwrap();
        let card = root.join("pending").join("my-card.bop");
        fs::create_dir_all(&card).unwrap();

        let found = find_card(root, "my-card");
        assert_eq!(found.unwrap(), card);
    }

    #[test]
    fn find_card_locates_glyph_prefixed_card() {
        let td = tempdir().unwrap();
        let root = td.path();
        ensure_cards_layout(root).unwrap();
        let card = root.join("done").join("A-test-thing.bop");
        fs::create_dir_all(&card).unwrap();

        let found = find_card(root, "test-thing");
        assert_eq!(found.unwrap(), card);
    }

    #[test]
    fn find_card_returns_none_for_missing() {
        let td = tempdir().unwrap();
        let root = td.path();
        ensure_cards_layout(root).unwrap();
        assert!(find_card(root, "no-such-card").is_none());
    }

    #[test]
    fn find_card_in_dir_exact_match() {
        let td = tempdir().unwrap();
        let dir = td.path();
        let card = dir.join("hello.bop");
        fs::create_dir_all(&card).unwrap();

        assert_eq!(find_card_in_dir(dir, "hello").unwrap(), card);
    }

    #[test]
    fn find_card_in_dir_suffix_match() {
        let td = tempdir().unwrap();
        let dir = td.path();
        let card = dir.join("X-hello.bop");
        fs::create_dir_all(&card).unwrap();

        assert_eq!(find_card_in_dir(dir, "hello").unwrap(), card);
    }

    #[test]
    fn find_card_in_dir_returns_none_for_missing() {
        let td = tempdir().unwrap();
        assert!(find_card_in_dir(td.path(), "nope").is_none());
    }

    #[test]
    fn card_exists_in_true_when_present() {
        let td = tempdir().unwrap();
        let root = td.path();
        fs::create_dir_all(root.join("running").join("foo.bop")).unwrap();
        assert!(card_exists_in(root, "running", "foo"));
    }

    #[test]
    fn card_exists_in_false_when_absent() {
        let td = tempdir().unwrap();
        let root = td.path();
        fs::create_dir_all(root.join("running")).unwrap();
        assert!(!card_exists_in(root, "running", "foo"));
    }

    #[test]
    fn card_exists_in_glyph_prefix() {
        let td = tempdir().unwrap();
        let root = td.path();
        fs::create_dir_all(root.join("pending").join("G-bar.bop")).unwrap();
        assert!(card_exists_in(root, "pending", "bar"));
    }

    #[test]
    fn find_card_in_state_true() {
        let td = tempdir().unwrap();
        let root = td.path();
        fs::create_dir_all(root.join("done").join("abc.bop")).unwrap();
        assert!(find_card_in_state(root, "abc", "done"));
    }

    #[test]
    fn find_card_in_state_false() {
        let td = tempdir().unwrap();
        let root = td.path();
        fs::create_dir_all(root.join("done")).unwrap();
        assert!(!find_card_in_state(root, "abc", "done"));
    }

    #[test]
    fn quarantine_invalid_pending_card_moves_and_logs() {
        let td = tempdir().unwrap();
        let root = td.path();
        ensure_cards_layout(root).unwrap();

        // Create a pending card
        let pending = root.join("pending").join("bad.bop");
        fs::create_dir_all(&pending).unwrap();
        fs::write(pending.join("spec.md"), "x").unwrap();

        quarantine_invalid_pending_card(&pending, &root.join("failed"), "test reason").unwrap();

        // Pending card should be gone
        assert!(!pending.exists());

        // Should exist in failed/ (possibly with collision suffix)
        let mut found = false;
        for entry in fs::read_dir(root.join("failed")).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("bad.bop") {
                found = true;
                let log = entry.path().join("logs").join("rejected.log");
                assert!(log.exists(), "rejected.log should exist");
                let contents = fs::read_to_string(log).unwrap();
                assert!(
                    contents.contains("test reason"),
                    "rejected.log should contain reason"
                );
            }
        }
        assert!(found, "card should exist in failed/");
    }
}
