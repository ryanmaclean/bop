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
    let base = name.strip_suffix(".jobcard").unwrap_or(name);
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

pub fn sync_card_action_links(card_dir: &Path) {
    let meta = jobcard_core::read_meta(card_dir).ok();
    let id = meta
        .as_ref()
        .map(|m| m.id.clone())
        .or_else(|| infer_card_id_from_path(card_dir))
        .unwrap_or_default();
    if id.trim().is_empty() {
        return;
    }

    let state = card_state_from_path(card_dir).unwrap_or_else(|| "unknown".to_string());
    let done_like = matches!(state.as_str(), "done" | "merged");
    let logs_action = if done_like { "logs" } else { "tail" };
    let logs_url = format!("bop://card/{id}/{logs_action}");
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
    if state == "running" {
        if let Some(session) = session {
            let session_url = format!("bop://card/{id}/session");
            links_md.push_str(&format!("- Session: [Attach zellij]({session_url})\n"));
            links_md.push_str(&format!("- Session command: `zellij attach {session}`\n"));
            let _ = write_webloc(&session_webloc, &session_url);
        } else {
            let _ = fs::remove_file(&session_webloc);
        }
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

    let status = StdCommand::new("ditto")
        .arg("--hfsCompression")
        .arg(card_dir)
        .arg(&compressed)
        .status();
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
}
