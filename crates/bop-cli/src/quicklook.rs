use crate::util;
use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

pub fn card_state_from_path(card_dir: &Path) -> Option<String> {
    card_dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

pub fn infer_card_id_from_path(card_dir: &Path) -> Option<String> {
    let name = card_dir.file_name()?.to_str()?;
    let base = name.strip_suffix(".bop").unwrap_or(name);
    Some(base.to_string())
}

pub fn write_webloc(path: &Path, target_url: &str) -> anyhow::Result<()> {
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>URL</key>
  <string>{target_url}</string>
</dict>
</plist>
"#
    );
    fs::write(path, body)?;
    Ok(())
}

fn push_pct_encoded_byte(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(char::from(HEX[(byte >> 4) as usize]));
    out.push(char::from(HEX[(byte & 0x0F) as usize]));
}

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

fn bop_card_url(card_id: &str, action: &str) -> String {
    format!(
        "bop://card/{}/{}",
        encode_bop_path_segment(card_id),
        encode_bop_path_segment(action),
    )
}

pub fn sync_card_action_links(card_dir: &Path) {
    let meta = bop_core::read_meta(card_dir).ok();
    let id = meta
        .as_ref()
        .map(|m| m.id.clone())
        .or_else(|| infer_card_id_from_path(card_dir))
        .unwrap_or_default();
    if id.trim().is_empty() {
        return;
    }

    let state = card_state_from_path(card_dir).unwrap_or_else(|| "unknown".to_string());
    let done_like = matches!(state.as_str(), "done" | "merged" | "failed");
    let logs_action = if done_like { "logs" } else { "tail" };
    let logs_url = bop_card_url(&id, logs_action);
    let logs_label = if done_like { "Open logs" } else { "Tail logs" };
    let logs_cmd = if done_like {
        format!("bop logs {id}")
    } else {
        format!("bop logs {id} --follow")
    };

    let session = meta
        .as_ref()
        .and_then(|m| m.zellij_session.as_ref())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let mut links_md = String::from("# Card Links\n\n");
    links_md.push_str(&format!("- Logs: [{logs_label}]({logs_url})\n"));
    links_md.push_str(&format!("- Logs command: `{logs_cmd}`\n"));

    let session_webloc = card_dir.join("Session.webloc");
    if let Some(session) = session {
        let session_url = bop_card_url(&id, "session");
        links_md.push_str(&format!("- Session: [Attach zellij]({session_url})\n"));
        links_md.push_str(&format!(
            "- Session command: `zellij attach {session} 2>/dev/null || zellij -s {session}`\n"
        ));
        let _ = write_webloc(&session_webloc, &session_url);
    } else {
        let _ = fs::remove_file(&session_webloc);
    }

    let _ = fs::write(card_dir.join("links.md"), links_md);
    let _ = write_webloc(&card_dir.join("Logs.webloc"), &logs_url);
}

pub fn render_card_thumbnail(card_dir: &Path) {
    sync_card_action_links(card_dir);

    if !cfg!(target_os = "macos") {
        return;
    }
    let meta = card_dir.join("meta.json");
    if !meta.exists() {
        return;
    }
    let ql_dir = card_dir.join("QuickLook");
    let _ = fs::create_dir_all(&ql_dir);
    let out = ql_dir.join("Thumbnail.png");
    let Some(script) = util::find_repo_script(card_dir, "scripts/render_card_thumbnail.swift")
    else {
        return;
    };

    let _ = StdCommand::new("swift")
        .arg(script)
        .arg(&meta)
        .arg(out)
        .status();

    // Update Finder folder icon: stage colour + glyph, set on every state transition
    if let Some(icon_script) = util::find_repo_script(card_dir, "scripts/set_card_icon.swift") {
        let _ = StdCommand::new("swift")
            .arg(icon_script)
            .arg(card_dir)
            .status();
    }

    compress_card(card_dir);
}

pub fn compress_card(card_dir: &Path) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let state = card_state_from_path(card_dir).unwrap_or_default();
    if !matches!(state.as_str(), "done" | "failed" | "merged") {
        return;
    }

    let Some(name) = card_dir.file_name().and_then(|s| s.to_str()) else {
        return;
    };
    let Some(parent) = card_dir.parent() else {
        return;
    };
    let compressed = parent.join(format!("{}.hfs.tmp", name));
    let backup = parent.join(format!("{}.bak.tmp", name));
    let _ = fs::remove_dir_all(&compressed);
    let _ = fs::remove_dir_all(&backup);

    // Temporarily clear the immutable flag on meta.json so ditto can clone it.
    let meta_path = card_dir.join("meta.json");
    bop_core::meta_unprotect(&meta_path);

    let status = StdCommand::new("ditto")
        .arg("--hfsCompression")
        .arg(card_dir)
        .arg(&compressed)
        .status();

    // Re-protect regardless of ditto outcome.
    bop_core::meta_protect(&meta_path);

    if !matches!(status, Ok(s) if s.success()) {
        let _ = fs::remove_dir_all(&compressed);
        return;
    }

    if fs::rename(card_dir, &backup).is_err() {
        let _ = fs::remove_dir_all(&compressed);
        return;
    }
    if fs::rename(&compressed, card_dir).is_err() {
        let _ = fs::rename(&backup, card_dir);
        let _ = fs::remove_dir_all(&compressed);
        return;
    }
    let _ = fs::remove_dir_all(&backup);

    // The rename replaced the original with the compressed copy. The new
    // meta.json was created by ditto (no immutable flag). Re-protect it.
    bop_core::meta_protect(&card_dir.join("meta.json"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn card_state_from_path_extracts_state() {
        let p = Path::new("/cards/pending/my-card.bop");
        assert_eq!(card_state_from_path(p).unwrap(), "pending");

        let p = Path::new("/cards/running/my-card.bop");
        assert_eq!(card_state_from_path(p).unwrap(), "running");

        let p = Path::new("/cards/done/my-card.bop");
        assert_eq!(card_state_from_path(p).unwrap(), "done");

        let p = Path::new("/cards/failed/my-card.bop");
        assert_eq!(card_state_from_path(p).unwrap(), "failed");
    }

    #[test]
    fn card_state_from_path_root_returns_none() {
        // A path with no parent dir component returns None
        let p = Path::new("my-card.bop");
        // parent is "" which has no file_name
        assert!(card_state_from_path(p).is_none());
    }

    #[test]
    fn infer_card_id_strips_bop_extension() {
        let p = Path::new("/cards/pending/my-card.bop");
        assert_eq!(infer_card_id_from_path(p).unwrap(), "my-card");
    }

    #[test]
    fn infer_card_id_with_glyph_prefix() {
        // The function just strips .bop, doesn't strip glyph prefix
        let p = Path::new("/cards/pending/X-my-card.bop");
        assert_eq!(infer_card_id_from_path(p).unwrap(), "X-my-card");
    }

    #[test]
    fn infer_card_id_no_extension() {
        let p = Path::new("/cards/pending/plain-dir");
        assert_eq!(infer_card_id_from_path(p).unwrap(), "plain-dir");
    }

    #[test]
    fn write_webloc_creates_valid_plist() {
        let td = tempdir().unwrap();
        let path = td.path().join("Test.webloc");
        write_webloc(&path, "https://example.com").unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("<?xml version=\"1.0\""));
        assert!(content.contains("<plist version=\"1.0\">"));
        assert!(content.contains("<key>URL</key>"));
        assert!(content.contains("<string>https://example.com</string>"));
    }

    #[test]
    fn write_webloc_url_embedded_correctly() {
        let td = tempdir().unwrap();
        let path = td.path().join("Link.webloc");
        let url = "bop://card/test-id/logs";
        write_webloc(&path, url).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains(&format!("<string>{url}</string>")));
    }

    #[test]
    fn sync_card_action_links_creates_logs_webloc_for_failed() {
        let td = tempdir().unwrap();
        // Create a card inside a "failed" parent so card_state_from_path works
        let failed_dir = td.path().join("failed");
        let card = failed_dir.join("test-card.bop");
        fs::create_dir_all(&card).unwrap();

        // Write a minimal meta.json (stage is required by validation)
        let meta = bop_core::Meta {
            id: "test-card".to_string(),
            stage: "implement".to_string(),
            ..Default::default()
        };
        bop_core::write_meta(&card, &meta).unwrap();

        sync_card_action_links(&card);

        let logs_webloc = card.join("Logs.webloc");
        assert!(logs_webloc.exists(), "Logs.webloc should be created");
        let content = fs::read_to_string(&logs_webloc).unwrap();
        assert!(content.contains("bop://card/test-card/logs"));
    }

    #[test]
    fn sync_card_action_links_creates_links_md() {
        let td = tempdir().unwrap();
        let pending_dir = td.path().join("pending");
        let card = pending_dir.join("abc.bop");
        fs::create_dir_all(&card).unwrap();

        let meta = bop_core::Meta {
            id: "abc".to_string(),
            stage: "implement".to_string(),
            ..Default::default()
        };
        bop_core::write_meta(&card, &meta).unwrap();

        sync_card_action_links(&card);

        let links = card.join("links.md");
        assert!(links.exists(), "links.md should be created");
        let content = fs::read_to_string(&links).unwrap();
        assert!(content.contains("# Card Links"));
        assert!(content.contains("bop://card/abc/tail"));
    }

    #[test]
    fn sync_card_action_links_done_uses_logs_not_tail() {
        let td = tempdir().unwrap();
        let done_dir = td.path().join("done");
        let card = done_dir.join("xyz.bop");
        fs::create_dir_all(&card).unwrap();

        let meta = bop_core::Meta {
            id: "xyz".to_string(),
            stage: "implement".to_string(),
            ..Default::default()
        };
        bop_core::write_meta(&card, &meta).unwrap();

        sync_card_action_links(&card);

        let content = fs::read_to_string(card.join("links.md")).unwrap();
        assert!(content.contains("bop://card/xyz/logs"));
        assert!(content.contains("Open logs"));
    }

    #[test]
    fn bop_card_url_percent_encodes_emoji_and_spaces() {
        let url = bop_card_url("🂠-feat auth", "session");
        assert_eq!(url, "bop://card/%F0%9F%82%A0-feat%20auth/session");
    }

    #[test]
    fn sync_links_keep_session_for_merged_when_meta_has_session() {
        let td = tempdir().unwrap();
        let merged_dir = td.path().join("merged");
        let card = merged_dir.join("🂠-feature.bop");
        fs::create_dir_all(&card).unwrap();

        let meta = bop_core::Meta {
            id: "🂠-feature".to_string(),
            stage: "implement".to_string(),
            zellij_session: Some("bop-feature".to_string()),
            ..Default::default()
        };
        bop_core::write_meta(&card, &meta).unwrap();

        sync_card_action_links(&card);

        let links = fs::read_to_string(card.join("links.md")).unwrap();
        assert!(links.contains("bop://card/%F0%9F%82%A0-feature/logs"));
        assert!(links.contains("bop://card/%F0%9F%82%A0-feature/session"));
        assert!(card.join("Session.webloc").exists());
    }

    #[test]
    fn render_card_thumbnail_no_panic_without_meta() {
        let td = tempdir().unwrap();
        let card = td.path().join("pending").join("empty.bop");
        fs::create_dir_all(&card).unwrap();
        // Should not panic even without meta.json
        render_card_thumbnail(&card);
    }

    #[test]
    fn compress_card_no_fail_without_output() {
        let td = tempdir().unwrap();
        let card = td.path().join("done").join("tiny.bop");
        fs::create_dir_all(&card).unwrap();
        // compress_card should not panic on a card without output/
        compress_card(&card);
    }

    #[test]
    fn compress_card_skips_non_terminal_states() {
        let td = tempdir().unwrap();
        // Card in "running" state — compress should be a no-op
        let card = td.path().join("running").join("active.bop");
        fs::create_dir_all(&card).unwrap();
        fs::write(card.join("data.txt"), "keep").unwrap();
        compress_card(&card);
        // Data should remain unchanged (no compression attempted)
        assert_eq!(fs::read_to_string(card.join("data.txt")).unwrap(), "keep");
    }
}
