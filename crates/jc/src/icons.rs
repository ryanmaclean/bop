use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use crate::factory::plist_path;
use crate::{quicklook, util};

pub const ICONS_LABEL: &str = "sh.bop.iconwatcher";

/// Run set_card_icon.swift on a single path (card bundle or state dir).
pub fn set_card_icon(path: &Path) {
    if !cfg!(target_os = "macos") {
        return;
    }
    if let Some(script) = util::find_repo_script(path, "scripts/set_card_icon.swift") {
        let _ = StdCommand::new("swift").arg(script).arg(path).status();
    }
}

/// Stamp icons for all state dirs + cards within a single team root.
/// Terminal-state cards also get compression refreshed.
pub fn sync_icons_in_root(
    team_root: &Path,
    n_cards: &mut usize,
    n_dirs: &mut usize,
    n_terminal_cards: &mut usize,
) {
    const STATES: &[&str] = &[
        "drafts",
        "pending",
        "running",
        "done",
        "merged",
        "failed",
        "templates",
    ];
    for &state in STATES {
        let dir = team_root.join(state);
        if !dir.exists() {
            continue;
        }
        // State directory itself gets an icon.
        set_card_icon(&dir);
        *n_dirs += 1;
        // Each .jobcard inside gets an icon.
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("jobcard") && path.is_dir() {
                    set_card_icon(&path);
                    if matches!(
                        quicklook::card_state_from_path(&path).as_deref(),
                        Some("done" | "failed" | "merged")
                    ) {
                        quicklook::compress_card(&path);
                        *n_terminal_cards += 1;
                    }
                    *n_cards += 1;
                }
            }
        }
    }
}

/// Batch: update icons for every state dir + .jobcard under .cards/.
pub fn cmd_icons_sync(root: &Path) -> anyhow::Result<()> {
    if !cfg!(target_os = "macos") {
        println!("icons: macOS only");
        return Ok(());
    }
    let mut n_cards = 0usize;
    let mut n_dirs = 0usize;
    let mut n_terminal_cards = 0usize;

    // Top-level state dirs
    sync_icons_in_root(root, &mut n_cards, &mut n_dirs, &mut n_terminal_cards);

    // team-* subdirs
    for entry in fs::read_dir(root)? {
        let team = entry?.path();
        if team.is_dir()
            && team
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("team-"))
                .unwrap_or(false)
        {
            sync_icons_in_root(&team, &mut n_cards, &mut n_dirs, &mut n_terminal_cards);
        }
    }
    println!(
        "✓ icons synced: {} state dirs, {} cards ({} terminal cards compression-refreshed)",
        n_dirs, n_cards, n_terminal_cards
    );
    Ok(())
}

/// FSEvents watcher (foreground): update icon immediately when any .jobcard dir moves.
pub fn cmd_icons_watch(root: &Path) -> anyhow::Result<()> {
    use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

    if !cfg!(target_os = "macos") {
        println!("icons watch: macOS only");
        return Ok(());
    }

    let cards_dir = root.to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = RecommendedWatcher::new(tx, Config::default())?;
    watcher.watch(&cards_dir, RecursiveMode::Recursive)?;

    println!(
        "bop icons watch — watching {} for card moves",
        cards_dir.display()
    );
    println!("Ctrl+C to stop\n");

    for res in rx {
        let event = match res {
            Ok(e) => e,
            Err(e) => {
                eprintln!("watch error: {}", e);
                continue;
            }
        };

        // Only care about create/rename events on .jobcard directories
        let is_relevant = matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(notify::event::ModifyKind::Name(_))
        );
        if !is_relevant {
            continue;
        }

        for path in &event.paths {
            if path.extension().and_then(|e| e.to_str()) == Some("jobcard") && path.is_dir() {
                set_card_icon(path);
                quicklook::compress_card(path);
                println!(
                    "  icon/compression updated: {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
            }
        }
    }
    Ok(())
}

/// Install a launchd WatchPaths agent: fires `bop icons sync` whenever .cards/ changes.
pub fn cmd_icons_install(root: &Path) -> anyhow::Result<()> {
    #[cfg(not(target_os = "macos"))]
    {
        println!("icons install: macOS only");
        return Ok(());
    }

    let bop_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/local/bin/bop"));
    let cards_dir = root.to_string_lossy().to_string();

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>

  <key>WatchPaths</key>
  <array>
    <string>{cards}</string>
  </array>

  <key>ProgramArguments</key>
  <array>
    <string>{bop}</string>
    <string>--cards-root</string>
    <string>{cards}</string>
    <string>icons</string>
    <string>sync</string>
  </array>

  <key>StandardOutPath</key>
  <string>/tmp/bop-iconwatcher.log</string>

  <key>StandardErrorPath</key>
  <string>/tmp/bop-iconwatcher.err</string>
</dict>
</plist>
"#,
        label = ICONS_LABEL,
        cards = cards_dir,
        bop = bop_bin.display(),
    );

    let dest = plist_path(ICONS_LABEL);
    fs::write(&dest, &plist)?;

    let _ = StdCommand::new("launchctl")
        .args(["unload", "-w"])
        .arg(&dest)
        .output();
    let status = StdCommand::new("launchctl")
        .args(["load", "-w"])
        .arg(&dest)
        .status()?;

    if status.success() {
        println!("✓ icon watcher installed: {}", dest.display());
        println!("  Fires `bop icons sync` whenever .cards/ changes.");
        println!("  Logs: /tmp/bop-iconwatcher.log");
        println!("\n  To remove: bop icons uninstall");
    } else {
        anyhow::bail!("launchctl load failed");
    }
    Ok(())
}

pub fn cmd_icons_uninstall() -> anyhow::Result<()> {
    let dest = plist_path(ICONS_LABEL);
    let _ = StdCommand::new("launchctl")
        .args(["unload", "-w"])
        .arg(&dest)
        .output();
    if dest.exists() {
        fs::remove_file(&dest)?;
        println!("✓ removed {}", dest.display());
    } else {
        println!("  (not installed)");
    }
    Ok(())
}
