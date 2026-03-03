use std::path::Path;
use std::process::Command as StdCommand;

pub fn macos_notify(card_id: &str, card_dir: &Path) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let state = card_dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let (title, subtitle, sound, action) = match state {
        "done" => ("✓ Card Done", "Click to open session", "Glass", "session"),
        "failed" => ("✗ Card Failed", "Click to view logs", "Basso", "logs"),
        "merged" => ("⤴ Card Merged", "Click to open session", "Purr", "session"),
        _ => return,
    };
    let open_url = format!("bop://card/{}/{}", card_id, action);

    // Try terminal-notifier first (actionable toast)
    if StdCommand::new("which")
        .arg("terminal-notifier")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        let _ = StdCommand::new("terminal-notifier")
            .args([
                "-title",
                title,
                "-subtitle",
                card_id,
                "-message",
                subtitle,
                "-sound",
                sound,
                "-open",
                &open_url,
                "-group",
                &format!("bop-{}", card_id),
                "-sender",
                "sh.bop.host",
            ])
            .spawn();
        return;
    }

    // Fallback: osascript (shows toast but no click action)
    let _ = StdCommand::new("osascript")
        .arg("-e")
        .arg(format!(
            "display notification \"{}\" with title \"{}\" sound name \"{}\"",
            card_id, title, sound
        ))
        .spawn();
}
