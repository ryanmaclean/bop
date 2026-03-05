use std::collections::BTreeMap;
use zellij_tile::prelude::*;

#[derive(Default)]
struct BopPlugin {
    /// team_name → (running, pending, done_or_merged, failed)
    counts: BTreeMap<String, (usize, usize, usize, usize)>,
    cards_dir: String,
}

register_plugin!(BopPlugin);

impl ZellijPlugin for BopPlugin {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        self.cards_dir = configuration
            .get("cards_dir")
            .cloned()
            .unwrap_or_else(|| ".cards".to_string());
        subscribe(&[EventType::Timer]);
        set_timeout(2.0);
        self.refresh_counts();
    }

    fn update(&mut self, event: Event) -> bool {
        if let Event::Timer(_) = event {
            self.refresh_counts();
            set_timeout(2.0);
            return true;
        }
        false
    }

    fn render(&mut self, _rows: usize, cols: usize) {
        let mut parts: Vec<String> = self
            .counts
            .iter()
            .map(|(team, &(running, pending, done, failed))| {
                let short = team.strip_prefix("team-").unwrap_or(team);
                let indicator = if running > 0 {
                    format!("{}\u{25b6}", running) // ▶
                } else if pending > 0 {
                    format!("{}·", pending)
                } else if failed > 0 {
                    format!("{}✗", failed)
                } else {
                    format!("{}✓", done)
                };
                format!("{}:{}", short, indicator)
            })
            .collect();

        // Totals summary
        let total_running: usize = self.counts.values().map(|&(r, _, _, _)| r).sum();
        let total_pending: usize = self.counts.values().map(|&(_, p, _, _)| p).sum();
        let total_done: usize = self.counts.values().map(|&(_, _, d, _)| d).sum();
        let total_failed: usize = self.counts.values().map(|&(_, _, _, f)| f).sum();

        if !parts.is_empty() {
            parts.push(format!(
                "| {}▶ {}· {}✓ {}✗",
                total_running, total_pending, total_done, total_failed
            ));
        }

        let mut bar = parts.join("  ");
        // Truncate to available columns using char boundaries to avoid
        // panicking mid-multi-byte character (▶ = 3 bytes, · = 2, ✓ = 3).
        if bar.chars().count() > cols {
            bar = bar.chars().take(cols).collect();
        }
        print!("{}", bar);
    }
}

impl BopPlugin {
    fn refresh_counts(&mut self) {
        self.counts.clear();
        let base = std::path::Path::new(&self.cards_dir);

        let Ok(teams) = std::fs::read_dir(base)
            .map_err(|e| eprintln!("[bop-plugin] failed to read cards_dir {:?}: {e}", base))
        else {
            return;
        };

        for team_entry in teams.flatten() {
            let path = team_entry.path();
            if !path.is_dir() {
                continue;
            }
            let team_name = team_entry.file_name().to_string_lossy().to_string();
            // Skip non-team directories (e.g. flat state dirs at root)
            if ["pending", "running", "done", "merged", "failed"].contains(&team_name.as_str()) {
                continue;
            }

            let mut running = 0usize;
            let mut pending = 0usize;
            let mut done = 0usize;
            let mut failed = 0usize;

            for state in ["running", "pending", "done", "merged", "failed"] {
                let dir = path.join(state);
                if let Ok(cards) = std::fs::read_dir(&dir) {
                    let count = cards
                        .flatten()
                        .filter(|e| {
                            e.path()
                                .extension()
                                .map(|x| x == "bop")
                                .unwrap_or(false)
                        })
                        .count();
                    match state {
                        "running" => running += count,
                        "pending" => pending += count,
                        "done" | "merged" => done += count,
                        "failed" => failed += count,
                        _ => {}
                    }
                }
            }

            if running > 0 || pending > 0 || done > 0 || failed > 0 {
                self.counts.insert(team_name, (running, pending, done, failed));
            }
        }
    }
}
