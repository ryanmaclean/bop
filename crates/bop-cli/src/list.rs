use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use chrono::Local;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use tokio::sync::mpsc as tokio_mpsc;

use crate::acplan;
use crate::colors::{state_ansi, DIM, RESET};
use crate::lock;
use crate::render;
use crate::termcaps::TermCaps;

fn resolve_states(state_filter: &str) -> Vec<&str> {
    match state_filter {
        "all" => vec!["drafts", "pending", "running", "done", "failed", "merged"],
        "active" => vec!["pending", "running", "done"],
        "drafts" => vec!["drafts"],
        other => vec![other],
    }
}

/// Summary statistics for cards in the workspace.
pub struct CardStats {
    pub total: usize,
    pub by_state: HashMap<String, usize>,
    pub avg_duration_s: Option<f64>,
    pub success_rate_pct: Option<f64>,
}

/// Calculate statistics: count cards by state and compute average duration.
pub fn calculate_stats(root: &Path) -> anyhow::Result<CardStats> {
    let states = ["drafts", "pending", "running", "done", "failed", "merged"];
    let mut by_state = HashMap::new();
    let mut total = 0;
    let mut durations = Vec::new();

    for state in &states {
        // Process root-level state directory
        collect_state_stats(root, state, &mut by_state, &mut total, &mut durations)?;

        // Process team-* directories
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                let team_path = entry.path();
                if team_path.is_dir() && entry.file_name().to_string_lossy().starts_with("team-") {
                    collect_state_stats(
                        &team_path,
                        state,
                        &mut by_state,
                        &mut total,
                        &mut durations,
                    )?;
                }
            }
        }
    }

    let avg_duration_s = if durations.is_empty() {
        None
    } else {
        Some(durations.iter().sum::<f64>() / durations.len() as f64)
    };

    let success_rate_pct = {
        let done = by_state.get("done").copied().unwrap_or(0);
        let failed = by_state.get("failed").copied().unwrap_or(0);
        if done + failed > 0 {
            Some((done as f64 / (done + failed) as f64) * 100.0)
        } else {
            None
        }
    };

    Ok(CardStats {
        total,
        by_state,
        avg_duration_s,
        success_rate_pct,
    })
}

fn collect_state_stats(
    dir: &Path,
    state: &str,
    by_state: &mut HashMap<String, usize>,
    total: &mut usize,
    durations: &mut Vec<f64>,
) -> anyhow::Result<()> {
    let state_dir = dir.join(state);
    if !state_dir.exists() {
        return Ok(());
    }

    let Ok(entries) = fs::read_dir(&state_dir) else {
        return Ok(());
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir()
            && path
                .extension()
                .is_some_and(|e| e == "bop" || e == "jobcard")
        {
            if let Ok(meta) = bop_core::read_meta(&path) {
                *by_state.entry(state.to_string()).or_insert(0) += 1;
                *total += 1;

                // Collect duration from last run if available
                if let Some(last_run) = meta.runs.last() {
                    if let Some(dur) = last_run.duration_s {
                        durations.push(dur as f64);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Collect all cards for the given state filter into `(label, Vec<CardView>)` groups.
fn collect_card_groups(
    root: &Path,
    state_filter: &str,
) -> anyhow::Result<Vec<(String, Vec<render::CardView>)>> {
    let states = resolve_states(state_filter);
    let git_root = acplan::find_git_root(root);
    let mut groups = Vec::new();

    for state in &states {
        // Collect root-level cards
        let root_cards = collect_card_views(root, state, git_root.as_deref())?;
        if !root_cards.is_empty() {
            groups.push((state.to_string(), root_cards));
        }

        // Collect team-* directory cards
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
                let team_name = entry.file_name().to_string_lossy().into_owned();
                let team_cards = collect_card_views(&entry.path(), state, git_root.as_deref())?;
                if !team_cards.is_empty() {
                    groups.push((format!("{}/{}", team_name, state), team_cards));
                }
            }
        }
    }

    Ok(groups)
}

/// Collect [`CardView`] structs from a single state directory.
///
/// When `git_root` is `Some`, cards with `ac_spec_id` are enriched with
/// Auto-Claude plan data (phase name, progress fraction, subtask counts).
fn collect_card_views(
    dir: &Path,
    state: &str,
    git_root: Option<&Path>,
) -> anyhow::Result<Vec<render::CardView>> {
    let state_dir = dir.join(state);
    if !state_dir.exists() {
        return Ok(Vec::new());
    }

    let mut cards = Vec::new();
    if let Ok(entries) = fs::read_dir(&state_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() && p.extension().is_some_and(|e| e == "bop" || e == "jobcard") {
                if let Ok(meta) = bop_core::read_meta(&p) {
                    let mut view = render::from_meta(&meta, state);
                    if let Some(gr) = git_root {
                        acplan::enrich_card_view(&mut view, &meta, gr);
                    }
                    cards.push(view);
                }
            }
        }
    }

    Ok(cards)
}

/// Convert [`CardStats`] to [`render::Stats`].
fn stats_to_render_stats(stats: &CardStats) -> render::Stats {
    render::Stats {
        total: stats.total,
        by_state: stats.by_state.clone(),
        avg_duration_s: stats.avg_duration_s,
        success_rate_pct: stats.success_rate_pct,
    }
}

pub fn list_cards(root: &Path, state_filter: &str) -> anyhow::Result<()> {
    let caps = TermCaps::detect();
    let groups = collect_card_groups(root, state_filter)?;
    let card_stats = calculate_stats(root)?;
    let stats = stats_to_render_stats(&card_stats);
    let output = render::render_board(&caps, &groups, &stats);
    print!("{}", output);
    Ok(())
}

/// Print clock header with current time.
fn print_clock_header() {
    let now = Local::now();
    let time_str = now.format("%H:%M:%S");
    println!("{}bop · {} · watching .cards/{}", DIM, time_str, RESET);
}

/// Watch for filesystem changes and continuously redraw the card list.
/// This function sets up watchers on all state directories and redraws
/// the display whenever cards are added, removed, or modified.
pub async fn list_cards_watch(root: &Path, state_filter: &str) -> anyhow::Result<()> {
    // Singleton guard — only one watch instance per cards dir.
    // Uses mkdir-atomicity (same pattern as dispatcher lock).
    // Guard is held for the lifetime of the function; Drop removes the lock dir.
    let _watch_lock = lock::acquire_watch_lock(root).map_err(|e| {
        eprintln!("{}", e);
        e
    })?;

    // Collect all state directories to watch
    let states = ["drafts", "pending", "running", "done", "failed", "merged"];
    let mut watch_dirs = Vec::new();

    // Add root-level state directories
    for state in &states {
        let state_dir = root.join(state);
        if state_dir.exists() {
            watch_dirs.push(state_dir);
        }
    }

    // Add team-* state directories
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let team_path = entry.path();
            if team_path.is_dir() && entry.file_name().to_string_lossy().starts_with("team-") {
                for state in &states {
                    let state_dir = team_path.join(state);
                    if state_dir.exists() {
                        watch_dirs.push(state_dir);
                    }
                }
            }
        }
    }

    // Also watch .auto-claude/specs/ for implementation_plan.json changes
    if let Some(git_root) = acplan::find_git_root(root) {
        let specs_dir = git_root.join(".auto-claude").join("specs");
        if specs_dir.exists() {
            watch_dirs.push(specs_dir);
        }
    }

    // Set up filesystem watcher with 100ms debounce.
    // Channel carries a bool: true = plan changed (force redraw),
    // false = card changed (redraw only if stats differ).
    let (tx, mut rx) = tokio_mpsc::unbounded_channel::<bool>();
    let watch_dirs_clone = watch_dirs.clone();

    std::thread::spawn(move || {
        let (std_tx, std_rx) = std::sync::mpsc::channel();
        let mut debouncer = match new_debouncer(Duration::from_millis(100), std_tx) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[list_cards_watch] failed to create watcher: {}", e);
                return;
            }
        };

        for watch_dir in &watch_dirs_clone {
            if let Err(e) = debouncer
                .watcher()
                .watch(watch_dir, notify::RecursiveMode::Recursive)
            {
                eprintln!(
                    "[list_cards_watch] failed to watch {}: {}",
                    watch_dir.display(),
                    e
                );
            }
        }

        for res in std_rx {
            match res {
                Ok(events) => {
                    let is_event = |e: &notify_debouncer_mini::DebouncedEvent| -> bool {
                        matches!(
                            e.kind,
                            DebouncedEventKind::Any | DebouncedEventKind::AnyContinuous
                        )
                    };

                    // Plan file changed — force redraw even if card stats are unchanged
                    let plan_changed = events.iter().any(|e| {
                        is_event(e)
                            && e.path.file_name().and_then(|s| s.to_str())
                                == Some("implementation_plan.json")
                    });

                    // Card directory changed (.bop extension)
                    let card_changed = events.iter().any(|e| {
                        is_event(e) && e.path.extension().and_then(|s| s.to_str()) == Some("bop")
                    });

                    if plan_changed {
                        let _ = tx.send(true);
                    } else if card_changed {
                        let _ = tx.send(false);
                    }
                }
                Err(e) => {
                    eprintln!("[list_cards_watch] watch error: {}", e);
                }
            }
        }
    });

    // Track previous state for minimal redraw
    let mut prev_stats;
    let mut prev_line_count;

    // Initial render
    {
        let caps = TermCaps::detect();
        print_clock_header();

        let current_stats = calculate_stats(root)?;
        let groups = collect_card_groups(root, state_filter)?;
        let rstats = stats_to_render_stats(&current_stats);
        let output = render::render_board(&caps, &groups, &rstats);

        // Count lines for cursor-up positioning (1 for clock header + rendered output)
        let line_count = 1 + output.lines().count();
        print!("{}", output);
        io::stdout().flush().ok();

        prev_line_count = line_count;
        prev_stats = Some(current_stats);
    }

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                // Clean shutdown on Ctrl-C
                // Move cursor down past last output to avoid overwriting
                if prev_line_count > 0 {
                    print!("\x1b[{}B", 1);
                    io::stdout().flush().ok();
                }
                println!("\nShutting down...");
                return Ok(());
            }
            event = rx.recv() => {
                let force_redraw = match event {
                    None => {
                        // Channel closed, watcher thread exited
                        return Ok(());
                    }
                    Some(force) => force,
                };

                // Calculate current stats
                let current_stats = match calculate_stats(root) {
                    Ok(stats) => stats,
                    Err(e) => {
                        eprintln!("[list_cards_watch] error calculating stats: {}", e);
                        continue;
                    }
                };

                // Redraw if stats changed or plan data changed (force_redraw).
                // Plan changes update the enriched card views but don't affect
                // card counts, so they need to force a redraw.
                let stats_changed = prev_stats
                    .as_ref()
                    .map(|prev| {
                        prev.total != current_stats.total || prev.by_state != current_stats.by_state
                    })
                    .unwrap_or(true);

                if force_redraw || stats_changed {
                    // Move cursor up to previous position if not first render
                    if prev_line_count > 0 {
                        print!("\x1b[{}A", prev_line_count);
                        io::stdout().flush().ok();
                    }

                    // Re-detect terminal capabilities on every redraw (never cache)
                    let caps = TermCaps::detect();
                    print_clock_header();

                    let groups = match collect_card_groups(root, state_filter) {
                        Ok(g) => g,
                        Err(e) => {
                            eprintln!("[list_cards_watch] error collecting cards: {}", e);
                            prev_stats = Some(current_stats);
                            continue;
                        }
                    };
                    let rstats = stats_to_render_stats(&current_stats);
                    let output = render::render_board(&caps, &groups, &rstats);

                    // Count lines for cursor-up positioning (1 for clock header + rendered output)
                    let line_count = 1 + output.lines().count();
                    print!("{}", output);
                    io::stdout().flush().ok();

                    prev_line_count = line_count;
                    prev_stats = Some(current_stats);
                }
            }
        }
    }
}

/// Format duration in human-readable form (e.g., "4m32s" or "45s").
#[allow(dead_code)] // Used by tests and print_summary (legacy fallback)
fn format_duration(seconds: f64) -> String {
    if seconds >= 60.0 {
        let mins = (seconds / 60.0).floor() as u64;
        let secs = (seconds % 60.0).round() as u64;
        if secs > 0 {
            format!("{}m{}s", mins, secs)
        } else {
            format!("{}m", mins)
        }
    } else {
        format!("{}s", seconds.round() as u64)
    }
}

/// Display summary statistics line.
#[allow(dead_code)] // Used by tests; legacy fallback before render system
fn print_summary(stats: &CardStats) {
    let mut parts = vec![format!("{} total", stats.total)];

    // Add key state counts
    for state in ["running", "done", "failed", "pending"] {
        if let Some(&count) = stats.by_state.get(state) {
            if count > 0 {
                parts.push(format!("{} {}", count, state));
            }
        }
    }

    // Add average duration if available
    if let Some(avg) = stats.avg_duration_s {
        parts.push(format!("avg {}", format_duration(avg)));
    }

    // Add success rate if available
    if let Some(rate) = stats.success_rate_pct {
        parts.push(format!("success rate {}%", rate.round() as u64));
    }

    println!("{}", parts.join(" · "));
}

#[allow(dead_code)] // Used by tests; legacy fallback before render system
pub fn print_state_group(dir: &Path, state: &str, team_prefix: Option<&str>) -> anyhow::Result<()> {
    let state_dir = dir.join(state);
    if !state_dir.exists() {
        return Ok(());
    }

    let mut cards: Vec<bop_core::Meta> = Vec::new();
    if let Ok(entries) = fs::read_dir(&state_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() && p.extension().is_some_and(|e| e == "bop" || e == "jobcard") {
                if let Ok(meta) = bop_core::read_meta(&p) {
                    cards.push(meta);
                }
            }
        }
    }

    let header = match team_prefix {
        Some(team) => format!("{}/{}", team, state),
        None => state.to_string(),
    };

    println!(
        "{}● {}{} ({})",
        state_ansi(state),
        header,
        RESET,
        cards.len()
    );
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

#[derive(serde::Serialize)]
struct JsonCard<'a> {
    state: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    team: Option<String>,
    #[serde(flatten)]
    meta: &'a bop_core::Meta,
}

pub fn list_cards_json(
    root: &Path,
    state_filter: &str,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    let states = resolve_states(state_filter);

    for state in &states {
        emit_state_json(root, state, None, out)?;

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
                emit_state_json(
                    &entry.path(),
                    state,
                    Some(entry.file_name().to_string_lossy().into_owned()),
                    out,
                )?;
            }
        }
    }
    Ok(())
}

fn emit_state_json(
    dir: &Path,
    state: &str,
    team: Option<String>,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    let state_dir = dir.join(state);
    if !state_dir.exists() {
        return Ok(());
    }

    if let Ok(entries) = fs::read_dir(&state_dir) {
        let mut metas: Vec<bop_core::Meta> = entries
            .flatten()
            .filter(|e| {
                e.path().is_dir()
                    && e.path()
                        .extension()
                        .is_some_and(|x| x == "bop" || x == "jobcard")
            })
            .filter_map(|e| bop_core::read_meta(&e.path()).ok())
            .collect();
        metas.sort_by(|a, b| a.id.cmp(&b.id));

        for meta in &metas {
            let card = JsonCard {
                state,
                team: team.clone(),
                meta,
            };
            writeln!(out, "{}", serde_json::to_string(&card)?)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup_card_in_state(root: &Path, state: &str, id: &str) {
        let card_dir = root.join(state).join(format!("{}.bop", id));
        fs::create_dir_all(&card_dir).unwrap();
        let meta = bop_core::Meta {
            id: id.into(),
            stage: "implement".into(),
            ..Default::default()
        };
        bop_core::write_meta(&card_dir, &meta).unwrap();
    }

    #[test]
    fn list_cards_empty_state_dirs() {
        let td = tempdir().unwrap();
        // Create empty state dirs
        for state in ["pending", "running", "done"] {
            fs::create_dir_all(td.path().join(state)).unwrap();
        }
        list_cards(td.path(), "active").unwrap();
    }

    #[test]
    fn list_cards_active_filter() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "pending", "card-a");
        setup_card_in_state(td.path(), "running", "card-b");
        setup_card_in_state(td.path(), "done", "card-c");
        setup_card_in_state(td.path(), "failed", "card-d");
        // "active" = pending + running + done — should not error
        list_cards(td.path(), "active").unwrap();
    }

    #[test]
    fn list_cards_all_filter() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "pending", "card-a");
        setup_card_in_state(td.path(), "failed", "card-b");
        setup_card_in_state(td.path(), "merged", "card-c");
        list_cards(td.path(), "all").unwrap();
    }

    #[test]
    fn list_cards_single_state_filter() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "failed", "card-a");
        list_cards(td.path(), "failed").unwrap();
    }

    #[test]
    fn print_state_group_nonexistent_dir() {
        let td = tempdir().unwrap();
        // Should succeed silently for non-existent state dir
        print_state_group(td.path(), "pending", None).unwrap();
    }

    #[test]
    fn print_state_group_with_team_prefix() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "pending", "card-a");
        print_state_group(td.path(), "pending", Some("team-alpha")).unwrap();
    }

    #[test]
    fn list_cards_json_emits_ndjson_lines() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "pending", "task-alpha");
        setup_card_in_state(td.path(), "running", "task-beta");

        let mut out = Vec::<u8>::new();
        list_cards_json(td.path(), "all", &mut out).unwrap();

        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "one JSON line per card");

        for line in &lines {
            let v: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("invalid JSON line: {e}: {line}"));
            assert!(v.get("id").is_some(), "must have id field");
            assert!(v.get("state").is_some(), "must have state field");
        }
    }

    #[test]
    fn list_cards_json_state_field_matches_directory() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "failed", "my-card");

        let mut out = Vec::<u8>::new();
        list_cards_json(td.path(), "failed", &mut out).unwrap();

        let text = String::from_utf8(out).unwrap();
        let v: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(v["state"], "failed");
        assert_eq!(v["id"], "my-card");
    }

    #[test]
    fn list_cards_json_team_field_present_for_team_dirs() {
        let td = tempdir().unwrap();
        let team_root = td.path().join("team-cli");
        fs::create_dir_all(&team_root).unwrap();
        setup_card_in_state(&team_root, "pending", "cli-task");

        let mut out = Vec::<u8>::new();
        list_cards_json(td.path(), "pending", &mut out).unwrap();

        let text = String::from_utf8(out).unwrap();
        let v: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(v["team"], "team-cli");
    }

    #[test]
    fn list_cards_json_skips_card_with_missing_meta() {
        let td = tempdir().unwrap();
        let bad = td.path().join("pending").join("bad-card.bop");
        fs::create_dir_all(&bad).unwrap();
        setup_card_in_state(td.path(), "pending", "good-card");

        let mut out = Vec::<u8>::new();
        list_cards_json(td.path(), "pending", &mut out).unwrap();

        let text = String::from_utf8(out).unwrap();
        assert_eq!(
            text.lines().count(),
            1,
            "corrupt card should be silently skipped"
        );
        assert!(text.contains("good-card"));
    }

    #[test]
    fn list_cards_json_empty_produces_no_output() {
        let td = tempdir().unwrap();
        fs::create_dir_all(td.path().join("pending")).unwrap();

        let mut out = Vec::<u8>::new();
        list_cards_json(td.path(), "all", &mut out).unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.is_empty(), "no cards = no output");
    }

    #[test]
    fn calculate_stats_empty_workspace() {
        let td = tempdir().unwrap();
        let stats = calculate_stats(td.path()).unwrap();
        assert_eq!(stats.total, 0);
        assert!(stats.by_state.is_empty());
        assert!(stats.avg_duration_s.is_none());
    }

    #[test]
    fn calculate_stats_counts_cards_by_state() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "pending", "card-a");
        setup_card_in_state(td.path(), "pending", "card-b");
        setup_card_in_state(td.path(), "running", "card-c");
        setup_card_in_state(td.path(), "done", "card-d");
        setup_card_in_state(td.path(), "failed", "card-e");

        let stats = calculate_stats(td.path()).unwrap();
        assert_eq!(stats.total, 5);
        assert_eq!(*stats.by_state.get("pending").unwrap(), 2);
        assert_eq!(*stats.by_state.get("running").unwrap(), 1);
        assert_eq!(*stats.by_state.get("done").unwrap(), 1);
        assert_eq!(*stats.by_state.get("failed").unwrap(), 1);
    }

    #[test]
    fn calculate_stats_includes_team_directories() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "pending", "root-card");

        let team_root = td.path().join("team-alpha");
        fs::create_dir_all(&team_root).unwrap();
        setup_card_in_state(&team_root, "pending", "team-card");

        let stats = calculate_stats(td.path()).unwrap();
        assert_eq!(stats.total, 2);
        assert_eq!(*stats.by_state.get("pending").unwrap(), 2);
    }

    fn setup_card_with_duration(root: &Path, state: &str, id: &str, duration_s: u64) {
        let card_dir = root.join(state).join(format!("{}.bop", id));
        fs::create_dir_all(&card_dir).unwrap();
        let meta = bop_core::Meta {
            id: id.into(),
            stage: "implement".into(),
            runs: vec![bop_core::RunRecord {
                duration_s: Some(duration_s),
                ..Default::default()
            }],
            ..Default::default()
        };
        bop_core::write_meta(&card_dir, &meta).unwrap();
    }

    #[test]
    fn calculate_stats_computes_avg_duration() {
        let td = tempdir().unwrap();
        setup_card_with_duration(td.path(), "done", "card-a", 100);
        setup_card_with_duration(td.path(), "done", "card-b", 200);
        setup_card_with_duration(td.path(), "done", "card-c", 300);

        let stats = calculate_stats(td.path()).unwrap();
        assert_eq!(stats.total, 3);
        assert_eq!(stats.avg_duration_s, Some(200.0));
    }

    #[test]
    fn calculate_stats_ignores_cards_without_duration() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "pending", "no-run");
        setup_card_with_duration(td.path(), "done", "with-duration", 150);

        let stats = calculate_stats(td.path()).unwrap();
        assert_eq!(stats.total, 2);
        // Average should only consider cards with duration
        assert_eq!(stats.avg_duration_s, Some(150.0));
    }

    #[test]
    fn format_duration_seconds() {
        assert_eq!(super::format_duration(45.0), "45s");
        assert_eq!(super::format_duration(5.7), "6s");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(super::format_duration(60.0), "1m");
        assert_eq!(super::format_duration(90.0), "1m30s");
        assert_eq!(super::format_duration(272.0), "4m32s");
        assert_eq!(super::format_duration(120.0), "2m");
    }

    #[test]
    fn print_summary_displays_stats() {
        use std::collections::HashMap;
        let mut by_state = HashMap::new();
        by_state.insert("running".to_string(), 2);
        by_state.insert("done".to_string(), 8);
        by_state.insert("failed".to_string(), 1);
        by_state.insert("pending".to_string(), 3);

        let stats = CardStats {
            total: 14,
            by_state,
            avg_duration_s: Some(272.0),
            success_rate_pct: Some(88.9),
        };

        // This will print to stdout - we're just verifying it doesn't panic
        super::print_summary(&stats);
    }

    #[test]
    fn calculate_stats_computes_success_rate() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "done", "card-a");
        setup_card_in_state(td.path(), "done", "card-b");
        setup_card_in_state(td.path(), "done", "card-c");
        setup_card_in_state(td.path(), "failed", "card-d");

        let stats = calculate_stats(td.path()).unwrap();
        assert_eq!(stats.total, 4);
        // 3 done, 1 failed = 75% success rate
        assert_eq!(stats.success_rate_pct, Some(75.0));
    }

    #[test]
    fn calculate_stats_success_rate_no_completed_cards() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "pending", "card-a");
        setup_card_in_state(td.path(), "running", "card-b");

        let stats = calculate_stats(td.path()).unwrap();
        assert_eq!(stats.total, 2);
        // No done or failed cards = no success rate
        assert!(stats.success_rate_pct.is_none());
    }

    #[test]
    fn calculate_stats_success_rate_all_failed() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "failed", "card-a");
        setup_card_in_state(td.path(), "failed", "card-b");

        let stats = calculate_stats(td.path()).unwrap();
        assert_eq!(stats.total, 2);
        // 0 done, 2 failed = 0% success rate
        assert_eq!(stats.success_rate_pct, Some(0.0));
    }

    #[test]
    fn calculate_stats_success_rate_all_done() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "done", "card-a");
        setup_card_in_state(td.path(), "done", "card-b");

        let stats = calculate_stats(td.path()).unwrap();
        assert_eq!(stats.total, 2);
        // 2 done, 0 failed = 100% success rate
        assert_eq!(stats.success_rate_pct, Some(100.0));
    }
}
