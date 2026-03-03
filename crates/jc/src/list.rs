use std::fs;
use std::path::Path;

pub fn list_cards(root: &Path, state_filter: &str) -> anyhow::Result<()> {
    let states: Vec<&str> = match state_filter {
        "all" => vec!["drafts", "pending", "running", "done", "failed", "merged"],
        "active" => vec!["pending", "running", "done"],
        "drafts" => vec!["drafts"],
        other => vec![other],
    };

    for state in &states {
        print_state_group(root, state, None)?;

        // Also check team-* directories
        if let Ok(entries) = fs::read_dir(root) {
            let mut team_dirs: Vec<_> = entries
                .flatten()
                .filter(|e| {
                    let name = e.file_name();
                    let s = name.to_string_lossy();
                    e.path().is_dir() && s.starts_with("team-")
                })
                .collect();
            team_dirs.sort_by_key(|e| e.file_name());
            for entry in team_dirs {
                print_state_group(
                    &entry.path(),
                    state,
                    Some(&entry.file_name().to_string_lossy()),
                )?;
            }
        }
    }
    Ok(())
}

pub fn print_state_group(dir: &Path, state: &str, team_prefix: Option<&str>) -> anyhow::Result<()> {
    let state_dir = dir.join(state);
    if !state_dir.exists() {
        return Ok(());
    }

    let mut cards: Vec<jobcard_core::Meta> = Vec::new();
    if let Ok(entries) = fs::read_dir(&state_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() && p.extension().is_some_and(|e| e == "jobcard") {
                if let Ok(meta) = jobcard_core::read_meta(&p) {
                    cards.push(meta);
                }
            }
        }
    }

    let header = match team_prefix {
        Some(team) => format!("{}/{}", team, state),
        None => state.to_string(),
    };

    println!("{} ({})", header, cards.len());
    for meta in &cards {
        let glyph = meta.glyph.as_deref().unwrap_or("  ");
        let token = meta.token.as_deref().unwrap_or(" ");
        let id_display = if meta.id.len() > 32 {
            &meta.id[..32]
        } else {
            &meta.id
        };
        let pri = meta
            .priority
            .map(|p| format!("P{}", p))
            .unwrap_or_else(|| "--".into());
        let pct = meta.progress.unwrap_or(0);
        let filled = (pct as usize * 8) / 100;
        let bar: String = (0..8)
            .map(|i| if i < filled { '\u{2588}' } else { '\u{2591}' })
            .collect();
        let pct_str = if pct > 0 {
            format!("{}%", pct)
        } else {
            String::new()
        };
        println!(
            "  {} {}  {:<32}  {:<12} {:<3} {} {}",
            glyph, token, id_display, meta.stage, pri, bar, pct_str
        );
    }
    println!();
    Ok(())
}
