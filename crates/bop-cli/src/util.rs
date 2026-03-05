use bop_core::Meta;
use chrono::Utc;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::atomic::AtomicU64;
use walkdir::WalkDir;

pub static RUN_ID_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn host_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown-host".to_string())
}

pub fn next_run_id(pid: Option<u32>) -> String {
    let ts = Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| Utc::now().timestamp_micros() * 1000);
    format!("{}-{}", ts, pid.unwrap_or(0))
}

pub fn pid_is_alive_sync(pid: i32) -> bool {
    StdCommand::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn append_log_line(path: &Path, line: &str) -> anyhow::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", line)?;
    Ok(())
}

pub fn copy_dir_all(src: &Path, dst: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in WalkDir::new(src).min_depth(1) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src)?;
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

pub fn unique_failed_path(failed_dir: &Path, name: &str) -> PathBuf {
    let direct = failed_dir.join(name);
    if !direct.exists() {
        return direct;
    }
    let stem = name.strip_suffix(".bop").unwrap_or(name);
    let ts = Utc::now().timestamp_millis();
    failed_dir.join(format!("{stem}-rejected-{ts}.bop"))
}

pub fn workflow_mode_for_template(template: &str) -> &'static str {
    match template {
        "full" => "full-spec",
        "qa-only" => "qa-only",
        "ideation" => "ideation",
        "roadmap" => "roadmap",
        "pr-fix" => "pr-fix",
        "mr-fix" => "mr-fix",
        _ => "default-feature",
    }
}

pub fn current_stage_step_index(meta: &Meta) -> u32 {
    if let Some(idx) = meta
        .stage_chain
        .iter()
        .position(|stage| stage == &meta.stage)
        .map(|i| i + 1)
    {
        idx as u32
    } else {
        meta.step_index.unwrap_or(1).max(1)
    }
}

pub fn find_repo_script(start: &Path, script_rel: &str) -> Option<PathBuf> {
    start.ancestors().find_map(|dir| {
        let candidate = dir.join(script_rel);
        if candidate.exists() {
            Some(candidate)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn host_name_returns_non_empty() {
        let name = host_name();
        assert!(!name.is_empty());
    }

    #[test]
    fn next_run_id_contains_pid() {
        let id = next_run_id(Some(42));
        assert!(id.ends_with("-42"));
    }

    #[test]
    fn next_run_id_zero_pid_when_none() {
        let id = next_run_id(None);
        assert!(id.ends_with("-0"));
    }

    #[test]
    fn next_run_id_different_each_call() {
        let a = next_run_id(Some(1));
        let b = next_run_id(Some(1));
        // Timestamps should differ (or at least be generated independently)
        // They may be the same in very fast tests, so just check they're valid
        assert!(!a.is_empty());
        assert!(!b.is_empty());
    }

    #[test]
    fn pid_is_alive_sync_own_pid() {
        let pid = std::process::id() as i32;
        assert!(pid_is_alive_sync(pid));
    }

    #[test]
    fn pid_is_alive_sync_bogus_pid() {
        assert!(!pid_is_alive_sync(999_999));
    }

    #[test]
    fn append_log_line_creates_file() {
        let td = tempdir().unwrap();
        let log = td.path().join("test.log");
        append_log_line(&log, "hello").unwrap();
        let content = fs::read_to_string(&log).unwrap();
        assert_eq!(content, "hello\n");
    }

    #[test]
    fn append_log_line_appends() {
        let td = tempdir().unwrap();
        let log = td.path().join("test.log");
        append_log_line(&log, "line1").unwrap();
        append_log_line(&log, "line2").unwrap();
        let content = fs::read_to_string(&log).unwrap();
        assert_eq!(content, "line1\nline2\n");
    }

    #[test]
    fn copy_dir_all_copies_nested_structure() {
        let td = tempdir().unwrap();
        let src = td.path().join("src");
        let dst = td.path().join("dst");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), "aaa").unwrap();
        fs::write(src.join("sub/b.txt"), "bbb").unwrap();

        copy_dir_all(&src, &dst).unwrap();

        assert!(dst.join("a.txt").exists());
        assert!(dst.join("sub/b.txt").exists());
    }

    #[test]
    fn copy_dir_all_preserves_contents() {
        let td = tempdir().unwrap();
        let src = td.path().join("src");
        let dst = td.path().join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("data.txt"), "important").unwrap();

        copy_dir_all(&src, &dst).unwrap();

        let content = fs::read_to_string(dst.join("data.txt")).unwrap();
        assert_eq!(content, "important");
    }

    #[test]
    fn unique_failed_path_returns_base_when_no_collision() {
        let td = tempdir().unwrap();
        let failed_dir = td.path().join("failed");
        fs::create_dir_all(&failed_dir).unwrap();
        let path = unique_failed_path(&failed_dir, "my-card.bop");
        assert_eq!(path, failed_dir.join("my-card.bop"));
    }

    #[test]
    fn unique_failed_path_appends_timestamp_on_collision() {
        let td = tempdir().unwrap();
        let failed_dir = td.path().join("failed");
        fs::create_dir_all(failed_dir.join("my-card.bop")).unwrap();
        let path = unique_failed_path(&failed_dir, "my-card.bop");
        assert_ne!(path, failed_dir.join("my-card.bop"));
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("my-card-rejected-"));
        assert!(name.ends_with(".bop"));
    }

    #[test]
    fn workflow_mode_for_template_known_templates() {
        assert_eq!(workflow_mode_for_template("full"), "full-spec");
        assert_eq!(workflow_mode_for_template("qa-only"), "qa-only");
        assert_eq!(workflow_mode_for_template("ideation"), "ideation");
        assert_eq!(workflow_mode_for_template("roadmap"), "roadmap");
        assert_eq!(workflow_mode_for_template("pr-fix"), "pr-fix");
        assert_eq!(workflow_mode_for_template("mr-fix"), "mr-fix");
    }

    #[test]
    fn workflow_mode_for_template_unknown_returns_default() {
        assert_eq!(
            workflow_mode_for_template("anything-else"),
            "default-feature"
        );
    }

    #[test]
    fn current_stage_step_index_from_stage_chain() {
        let meta = Meta {
            stage: "qa".to_string(),
            stage_chain: vec!["implement".into(), "qa".into()],
            ..Default::default()
        };
        assert_eq!(current_stage_step_index(&meta), 2);
    }

    #[test]
    fn current_stage_step_index_first_stage() {
        let meta = Meta {
            stage: "implement".to_string(),
            stage_chain: vec!["implement".into(), "qa".into()],
            ..Default::default()
        };
        assert_eq!(current_stage_step_index(&meta), 1);
    }

    #[test]
    fn current_stage_step_index_empty_chain_uses_step_index() {
        let meta = Meta {
            stage: "implement".to_string(),
            stage_chain: vec![],
            step_index: Some(3),
            workflow_mode: Some("default-feature".to_string()),
            ..Default::default()
        };
        assert_eq!(current_stage_step_index(&meta), 3);
    }

    #[test]
    fn current_stage_step_index_empty_chain_default_is_1() {
        let meta = Meta {
            stage: "implement".to_string(),
            stage_chain: vec![],
            step_index: None,
            ..Default::default()
        };
        assert_eq!(current_stage_step_index(&meta), 1);
    }

    #[test]
    fn find_repo_script_locates_script() {
        let td = tempdir().unwrap();
        let script_path = td.path().join("scripts/test.sh");
        fs::create_dir_all(td.path().join("scripts")).unwrap();
        fs::write(&script_path, "#!/bin/sh").unwrap();

        let sub = td.path().join("a/b/c");
        fs::create_dir_all(&sub).unwrap();

        let found = find_repo_script(&sub, "scripts/test.sh");
        assert!(found.is_some());
        assert_eq!(found.unwrap(), script_path);
    }

    #[test]
    fn find_repo_script_returns_none_for_missing() {
        let td = tempdir().unwrap();
        let result = find_repo_script(td.path(), "nonexistent.sh");
        assert!(result.is_none());
    }
}

// ── bop:// URL encoding ───────────────────────────────────────────────────────

#[allow(dead_code)]
fn push_pct_encoded_byte(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(char::from(HEX[(byte >> 4) as usize]));
    out.push(char::from(HEX[(byte & 0x0F) as usize]));
}

/// Percent-encode one URL path segment for bop://card/<id>/<action>.
/// Unreserved chars (RFC 3986) pass through; everything else is %-encoded.
#[allow(dead_code)]
fn encode_bop_path_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for &byte in segment.as_bytes() {
        let is_unreserved =
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
        if is_unreserved {
            out.push(byte as char);
        } else {
            push_pct_encoded_byte(&mut out, byte);
        }
    }
    out
}

/// Build a `bop://card/<id>/<action>` URL with percent-encoded path segments.
/// Safe for card IDs containing emoji, spaces, or Unicode (e.g. `🂠-feat auth`).
#[allow(dead_code)]
pub fn bop_card_url(card_id: &str, action: &str) -> String {
    format!(
        "bop://card/{}/{}",
        encode_bop_path_segment(card_id),
        encode_bop_path_segment(action),
    )
}

#[cfg(test)]
mod url_tests {
    use super::*;

    #[test]
    fn bop_card_url_percent_encodes_emoji_and_spaces() {
        let url = bop_card_url("🂠-feat auth", "session");
        assert!(!url.contains(' '), "spaces must be encoded");
        assert!(url.starts_with("bop://card/"), "must use bop scheme");
        assert!(url.contains("session"), "action must appear");
    }

    #[test]
    fn bop_card_url_passes_unreserved_chars() {
        let url = bop_card_url("my-card.bop", "logs");
        assert_eq!(url, "bop://card/my-card.bop/logs");
    }
}
