use std::collections::BTreeMap;
use zellij_tile::prelude::*;

#[derive(Default)]
struct JobCardPlugin {
    /// team_name → (running, pending, done_or_merged)
    counts: BTreeMap<String, (usize, usize, usize)>,
    cards_dir: String,
    initialized: bool,
}

register_plugin!(JobCardPlugin);

impl ZellijPlugin for JobCardPlugin {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        self.cards_dir = configuration
            .get("cards_dir")
            .cloned()
            .unwrap_or_else(|| ".cards".to_string());
        subscribe(&[EventType::Timer]);
        set_timeout(2.0);
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
        if !self.initialized {
            self.refresh_counts();
            self.initialized = true;
        }

        let mut parts: Vec<String> = self
            .counts
            .iter()
            .map(|(team, &(running, pending, done))| {
                let short = team.strip_prefix("team-").unwrap_or(team);
                let indicator = if running > 0 {
                    format!("{}\u{25b6}", running) // ▶
                } else if pending > 0 {
                    format!("{}·", pending)
                } else {
                    format!("{}✓", done)
                };
                format!("{}:{}", short, indicator)
            })
            .collect();

        // Totals summary
        let total_running: usize = self.counts.values().map(|&(r, _, _)| r).sum();
        let total_pending: usize = self.counts.values().map(|&(_, p, _)| p).sum();
        let total_done: usize = self.counts.values().map(|&(_, _, d)| d).sum();

        if !parts.is_empty() {
            parts.push(format!(
                "| {}▶ {}· {}✓",
                total_running, total_pending, total_done
            ));
        }

        let bar = parts.join("  ");
        // Truncate to available columns
        let display = if bar.len() > cols { &bar[..cols] } else { &bar };
        print!("{}", display);
    }
}

impl JobCardPlugin {
    fn refresh_counts(&mut self) {
        self.counts.clear();
        let base = std::path::Path::new(&self.cards_dir);

        let Ok(teams) = std::fs::read_dir(base) else {
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

            for state in ["running", "pending", "done", "merged"] {
                let dir = path.join(state);
                if let Ok(cards) = std::fs::read_dir(&dir) {
                    let count = cards
                        .flatten()
                        .filter(|e| {
                            e.path()
                                .extension()
                                .map(|x| x == "jobcard")
                                .unwrap_or(false)
                        })
                        .count();
                    match state {
                        "running" => running += count,
                        "pending" => pending += count,
                        "done" | "merged" => done += count,
                        _ => {}
                    }
                }
            }

            if running > 0 || pending > 0 || done > 0 {
                self.counts.insert(team_name, (running, pending, done));
            }
        }
    }
}
