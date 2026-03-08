use anyhow::Context;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const SEARCH_STATES: [&str; 4] = ["done", "merged", "failed", "running"];
const ALL_STATES: [&str; 6] = ["drafts", "pending", "running", "done", "merged", "failed"];

#[derive(Debug, Clone, Copy)]
pub struct ReplayOptions {
    pub json: bool,
    pub errors: bool,
    pub relative: bool,
    pub all: bool,
}

#[derive(Debug, Clone)]
struct CardCandidate {
    path: PathBuf,
    state: String,
    canonical_id: String,
    meta_id: Option<String>,
    dir_stem: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RawRun {
    #[serde(default)]
    provider: String,
    #[serde(default)]
    started_at: String,
    #[serde(default)]
    ended_at: Option<String>,
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct ReplayEvent {
    card_id: String,
    ts: String,
    event: String,
    state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cooldown_s: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rotated_to: Option<String>,
    #[serde(skip_serializing_if = "Map::is_empty")]
    extras: Map<String, Value>,
    #[serde(skip)]
    ts_parsed: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct Summary {
    retries: usize,
    total_cost_usd: f64,
    duration: Duration,
}

pub fn cmd_replay(cards_root: &Path, id: Option<&str>, opts: ReplayOptions) -> anyhow::Result<()> {
    if opts.all {
        return cmd_replay_all(cards_root, opts);
    }

    let query = id.map(str::trim).filter(|s| !s.is_empty());
    let Some(query) = query else {
        anyhow::bail!("replay requires a card id or --all");
    };

    let card = resolve_card(cards_root, query)?;
    let events_path = card.path.join("logs").join("events.jsonl");
    let mut events = if events_path.exists() {
        parse_events_file(&events_path, &card.canonical_id, &card.state)?
    } else {
        Vec::new()
    };
    events.sort_by_key(|e| e.ts_parsed);

    let runs = read_runs(&card.path);
    enrich_with_runs(&mut events, &runs);

    let filtered = filter_events(events, opts.errors);
    if opts.json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
        return Ok(());
    }

    if filtered.is_empty() {
        println!("no events recorded");
        return Ok(());
    }

    let summary = summarize(&filtered);
    print_card_timeline(
        &card.canonical_id,
        &card.state,
        &filtered,
        &summary,
        opts.relative,
    );
    Ok(())
}

fn cmd_replay_all(cards_root: &Path, opts: ReplayOptions) -> anyhow::Result<()> {
    let now = Utc::now();
    let cutoff = now - Duration::hours(24);
    let mut all = Vec::new();

    for card_dir in collect_all_card_dirs(cards_root) {
        let events_path = card_dir.join("logs").join("events.jsonl");
        if !events_path.exists() {
            continue;
        }
        let card_id = canonical_card_id(cards_root, &card_dir);
        let state = card_state_from_path(cards_root, &card_dir);
        let mut events = parse_events_file(&events_path, &card_id, &state)?;
        let runs = read_runs(&card_dir);
        enrich_with_runs(&mut events, &runs);
        all.extend(events.into_iter().filter(|e| e.ts_parsed >= cutoff));
    }
    all.sort_by_key(|e| e.ts_parsed);
    let filtered = filter_events(all, opts.errors);

    if opts.json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
        return Ok(());
    }

    if filtered.is_empty() {
        println!("no events recorded");
        return Ok(());
    }

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  replay --all (last 24h)  •  {} events", filtered.len());
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let base = filtered
        .first()
        .map(|e| e.ts_parsed)
        .unwrap_or_else(Utc::now);
    for ev in &filtered {
        let ts = if opts.relative {
            format_relative(base, ev.ts_parsed)
        } else {
            ev.ts_parsed.format("%Y-%m-%d %H:%M:%SZ").to_string()
        };
        let name = display_event_name(ev);
        let details = format_details(ev);
        if details.is_empty() {
            println!(
                "  {:<20}  {:<24}  {:<12}  {}",
                ts, ev.card_id, name, ev.state
            );
        } else {
            println!(
                "  {:<20}  {:<24}  {:<12}  {:<8}  {}",
                ts, ev.card_id, name, ev.state, details
            );
        }
    }
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    let summary = summarize(&filtered);
    println!(
        "  Cards: {}  •  Retries: {}  •  Cost: ${:.2}",
        filtered
            .iter()
            .map(|e| e.card_id.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        summary.retries,
        summary.total_cost_usd
    );
    Ok(())
}

fn print_card_timeline(
    id: &str,
    state: &str,
    events: &[ReplayEvent],
    summary: &Summary,
    relative: bool,
) {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  {}  •  {}  •  {} events", id, state, events.len());
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    let base = events.first().map(|e| e.ts_parsed).unwrap_or_else(Utc::now);
    for ev in events {
        let ts = if relative {
            format_relative(base, ev.ts_parsed)
        } else {
            ev.ts_parsed.format("%Y-%m-%d %H:%M:%SZ").to_string()
        };
        let name = display_event_name(ev);
        let details = format_details(ev);
        if details.is_empty() {
            println!("  {:<20}  {:<12}  {}", ts, name, ev.state);
        } else {
            println!("  {:<20}  {:<12}  {:<8}  {}", ts, name, ev.state, details);
        }
    }
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!(
        "  Total duration: {}  •  Retries: {}  •  Cost: ${:.2}",
        format_duration(summary.duration),
        summary.retries,
        summary.total_cost_usd
    );
}

fn parse_events_file(
    events_path: &Path,
    card_id: &str,
    default_state: &str,
) -> anyhow::Result<Vec<ReplayEvent>> {
    let content = fs::read_to_string(events_path)
        .with_context(|| format!("failed to read {}", events_path.display()))?;
    Ok(parse_events_content(&content, card_id, default_state))
}

fn parse_events_content(content: &str, card_id: &str, default_state: &str) -> Vec<ReplayEvent> {
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(mut obj) = serde_json::from_str::<Map<String, Value>>(line) else {
            continue;
        };

        let Some(ts) = pop_string(&mut obj, "ts") else {
            continue;
        };
        let Ok(ts_parsed) = DateTime::parse_from_rfc3339(&ts).map(|d| d.with_timezone(&Utc)) else {
            continue;
        };
        let Some(event) = pop_string(&mut obj, "event") else {
            continue;
        };

        let from = pop_string(&mut obj, "from");
        let to = pop_string(&mut obj, "to");
        let state = pop_string(&mut obj, "state")
            .or_else(|| to.clone())
            .or_else(|| from.clone())
            .unwrap_or_else(|| default_state.to_string());

        let mut tokens = pop_u64(&mut obj, "tokens");
        if tokens.is_none() {
            tokens = pop_u64(&mut obj, "tokens_used");
        }
        if tokens.is_none() {
            let prompt = pop_u64(&mut obj, "prompt_tokens").unwrap_or(0);
            let completion = pop_u64(&mut obj, "completion_tokens").unwrap_or(0);
            if prompt > 0 || completion > 0 {
                tokens = Some(prompt.saturating_add(completion));
            }
        }

        let cost_usd = pop_f64(&mut obj, "cost_usd").or_else(|| pop_f64(&mut obj, "cost"));
        let cooldown_s = pop_u64(&mut obj, "cooldown_s")
            .or_else(|| pop_u64(&mut obj, "cooldown"))
            .or_else(|| pop_u64(&mut obj, "cooldown_seconds"));

        out.push(ReplayEvent {
            card_id: card_id.to_string(),
            ts,
            event,
            state,
            stage: pop_string(&mut obj, "stage"),
            provider: pop_string(&mut obj, "provider"),
            pid: pop_u64(&mut obj, "pid").and_then(|v| u32::try_from(v).ok()),
            exit_code: pop_i32(&mut obj, "exit_code").or_else(|| pop_i32(&mut obj, "exit")),
            from,
            to,
            tokens,
            cost_usd,
            cooldown_s,
            rotated_to: pop_string(&mut obj, "rotated_to")
                .or_else(|| pop_string(&mut obj, "rotated")),
            extras: obj,
            ts_parsed,
        });
    }
    out
}

fn filter_events(events: Vec<ReplayEvent>, errors_only: bool) -> Vec<ReplayEvent> {
    if !errors_only {
        return events;
    }
    events.into_iter().filter(is_error_or_retry_event).collect()
}

fn summarize(events: &[ReplayEvent]) -> Summary {
    if events.is_empty() {
        return Summary {
            retries: 0,
            total_cost_usd: 0.0,
            duration: Duration::zero(),
        };
    }
    let retries = events.iter().filter(|ev| is_retry_event(ev)).count();
    let total_cost_usd = events.iter().filter_map(|ev| ev.cost_usd).sum::<f64>();
    let first = events.first().map(|e| e.ts_parsed).unwrap_or_else(Utc::now);
    let last = events.last().map(|e| e.ts_parsed).unwrap_or(first);
    Summary {
        retries,
        total_cost_usd,
        duration: last.signed_duration_since(first),
    }
}

fn enrich_with_runs(events: &mut [ReplayEvent], runs: &[RawRun]) {
    if runs.is_empty() {
        return;
    }
    events.sort_by_key(|e| e.ts_parsed);
    let mut dispatch_idx = 0usize;
    for ev in events {
        if is_dispatch_event(ev) {
            if let Some(run) = runs.get(dispatch_idx) {
                if ev.provider.is_none() && !run.provider.trim().is_empty() {
                    ev.provider = Some(run.provider.clone());
                }
            }
            dispatch_idx = dispatch_idx.saturating_add(1);
            continue;
        }
        if is_attempt_terminal(ev) {
            let idx = dispatch_idx.saturating_sub(1);
            if let Some(run) = runs.get(idx) {
                if ev.provider.is_none() && !run.provider.trim().is_empty() {
                    ev.provider = Some(run.provider.clone());
                }
                if ev.tokens.is_none() {
                    let pt = run.prompt_tokens.unwrap_or(0);
                    let ct = run.completion_tokens.unwrap_or(0);
                    if pt > 0 || ct > 0 {
                        ev.tokens = Some(pt.saturating_add(ct));
                    }
                }
                if ev.cost_usd.is_none() {
                    ev.cost_usd = run.cost_usd;
                }
            }
        }
    }
}

fn read_runs(card_dir: &Path) -> Vec<RawRun> {
    let meta_path = card_dir.join("meta.json");
    let Ok(raw) = fs::read_to_string(meta_path) else {
        return Vec::new();
    };
    #[derive(Debug, Deserialize, Default)]
    struct RawMeta {
        #[serde(default)]
        runs: Vec<RawRun>,
    }
    serde_json::from_str::<RawMeta>(&raw)
        .map(|m| m.runs)
        .unwrap_or_default()
}

fn format_relative(base: DateTime<Utc>, ts: DateTime<Utc>) -> String {
    let delta = ts.signed_duration_since(base);
    if delta <= Duration::zero() {
        return "0s".to_string();
    }
    format!("+{}", format_duration(delta))
}

fn format_duration(d: Duration) -> String {
    let total = d.num_seconds().max(0);
    let hours = total / 3600;
    let mins = (total % 3600) / 60;
    let secs = total % 60;
    if hours > 0 {
        format!("{}h {}m {}s", hours, mins, secs)
    } else if mins > 0 {
        format!("{}m {}s", mins, secs)
    } else {
        format!("{}s", secs)
    }
}

fn display_event_name(ev: &ReplayEvent) -> String {
    if ev.event == "stage_transition" {
        return match (ev.from.as_deref(), ev.to.as_deref()) {
            (Some("pending"), Some("running")) => "dispatched".to_string(),
            (Some("running"), Some("pending")) => {
                if ev.exit_code == Some(75) {
                    "rate-limited".to_string()
                } else {
                    "retry".to_string()
                }
            }
            (Some("running"), Some("done")) => "completed".to_string(),
            (Some("running"), Some("failed")) => "failed".to_string(),
            (Some("done"), Some("merged")) => "merged".to_string(),
            _ => "transition".to_string(),
        };
    }
    if ev.event == "meta_written" && ev.from.is_none() && ev.to.is_none() {
        return "meta_written".to_string();
    }
    ev.event.clone()
}

fn format_details(ev: &ReplayEvent) -> String {
    let mut details = Vec::new();
    if let Some(provider) = ev.provider.as_deref().filter(|s| !s.trim().is_empty()) {
        details.push(format!("provider={provider}"));
    }
    if let Some(pid) = ev.pid {
        details.push(format!("pid={pid}"));
    }
    if let Some(exit) = ev.exit_code {
        details.push(format!("exit={exit}"));
    }
    if let Some(cooldown) = ev.cooldown_s {
        details.push(format!("cooldown={}s", cooldown));
    }
    if let Some(rotated) = ev.rotated_to.as_deref().filter(|s| !s.trim().is_empty()) {
        details.push(format!("rotated->{rotated}"));
    }
    if let Some(tokens) = ev.tokens {
        details.push(format!("tokens={tokens}"));
    }
    if let Some(cost) = ev.cost_usd {
        details.push(format!("cost=${:.2}", cost));
    }
    details.join("  ")
}

fn is_dispatch_event(ev: &ReplayEvent) -> bool {
    ev.event == "stage_transition"
        && ev.from.as_deref() == Some("pending")
        && ev.to.as_deref() == Some("running")
}

fn is_attempt_terminal(ev: &ReplayEvent) -> bool {
    ev.event == "stage_transition"
        && ev.from.as_deref() == Some("running")
        && matches!(
            ev.to.as_deref(),
            Some("done") | Some("pending") | Some("failed")
        )
}

fn is_retry_event(ev: &ReplayEvent) -> bool {
    if ev.event.contains("retry") || ev.event.contains("rate") {
        return true;
    }
    ev.event == "stage_transition"
        && ev.from.as_deref() == Some("running")
        && ev.to.as_deref() == Some("pending")
        && ev.exit_code.unwrap_or(1) != 0
}

fn is_error_or_retry_event(ev: &ReplayEvent) -> bool {
    if is_retry_event(ev) {
        return true;
    }
    let event = ev.event.to_ascii_lowercase();
    if event.contains("fail") || event.contains("error") || event.contains("timeout") {
        return true;
    }
    ev.to.as_deref() == Some("failed")
}

fn pop_string(map: &mut Map<String, Value>, key: &str) -> Option<String> {
    map.remove(key)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
}

fn pop_u64(map: &mut Map<String, Value>, key: &str) -> Option<u64> {
    let val = map.remove(key)?;
    match val {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn pop_i32(map: &mut Map<String, Value>, key: &str) -> Option<i32> {
    let val = map.remove(key)?;
    match val {
        Value::Number(n) => n.as_i64().and_then(|v| i32::try_from(v).ok()),
        Value::String(s) => s.trim().parse::<i32>().ok(),
        _ => None,
    }
}

fn pop_f64(map: &mut Map<String, Value>, key: &str) -> Option<f64> {
    let val = map.remove(key)?;
    match val {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn resolve_card(cards_root: &Path, id: &str) -> anyhow::Result<CardCandidate> {
    let query = id.trim();
    if query.is_empty() {
        anyhow::bail!("card id cannot be empty");
    }

    let candidates = collect_candidates(cards_root)?;
    let exact: Vec<_> = candidates
        .iter()
        .filter(|c| matches_id_exact(c, query))
        .cloned()
        .collect();
    if exact.len() == 1 {
        return Ok(exact[0].clone());
    }
    if exact.len() > 1 {
        return ambiguous_error(query, &exact);
    }

    let prefix: Vec<_> = candidates
        .iter()
        .filter(|c| matches_id_prefix(c, query))
        .cloned()
        .collect();
    if prefix.len() == 1 {
        return Ok(prefix[0].clone());
    }
    if prefix.is_empty() {
        anyhow::bail!("card id not found: {query}");
    }
    ambiguous_error(query, &prefix)
}

fn ambiguous_error(query: &str, matches: &[CardCandidate]) -> anyhow::Result<CardCandidate> {
    eprintln!("multiple cards match '{query}':");
    for m in matches {
        let rel = m
            .path
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches("./")
            .to_string();
        eprintln!("  - {} ({})", m.canonical_id, rel);
    }
    anyhow::bail!("card id is ambiguous")
}

fn collect_candidates(cards_root: &Path) -> anyhow::Result<Vec<CardCandidate>> {
    let mut out = Vec::new();
    for state in SEARCH_STATES {
        for state_dir in state_dirs(cards_root, state)? {
            let state_name = state.to_string();
            let team_name = team_from_state_dir(cards_root, &state_dir);
            let entries = match fs::read_dir(&state_dir) {
                Ok(it) => it,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !is_card_dir(&path) {
                    continue;
                }
                let dir_stem = file_stem_string(&path);
                let meta_id = read_meta_id(&path);
                let id_base = meta_id.clone().unwrap_or_else(|| dir_stem.clone());
                let canonical_id = if let Some(team) = &team_name {
                    format!("{team}/{id_base}")
                } else {
                    id_base
                };
                out.push(CardCandidate {
                    path,
                    state: state_name.clone(),
                    canonical_id,
                    meta_id,
                    dir_stem,
                });
            }
        }
    }
    out.sort_by(|a, b| a.canonical_id.cmp(&b.canonical_id));
    Ok(out)
}

fn matches_id_exact(c: &CardCandidate, query: &str) -> bool {
    c.canonical_id == query || c.meta_id.as_deref() == Some(query) || c.dir_stem == query
}

fn matches_id_prefix(c: &CardCandidate, query: &str) -> bool {
    c.canonical_id.starts_with(query)
        || c.meta_id
            .as_deref()
            .map(|m| m.starts_with(query))
            .unwrap_or(false)
        || c.dir_stem.starts_with(query)
}

fn state_dirs(cards_root: &Path, state: &str) -> anyhow::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    let root_state = cards_root.join(state);
    if root_state.exists() {
        dirs.push(root_state);
    }
    let entries = fs::read_dir(cards_root)?;
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if p.is_dir() && name.starts_with("team-") {
            let team_state = p.join(state);
            if team_state.exists() {
                dirs.push(team_state);
            }
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn team_from_state_dir(cards_root: &Path, state_dir: &Path) -> Option<String> {
    let parent = state_dir.parent()?;
    if parent == cards_root {
        return None;
    }
    let name = parent.file_name()?.to_string_lossy().to_string();
    if name.starts_with("team-") {
        Some(name)
    } else {
        None
    }
}

fn collect_all_card_dirs(cards_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in WalkDir::new(cards_root)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        let path = entry.path();
        if !is_card_dir(path) {
            continue;
        }
        if card_state_from_path(cards_root, path) == "unknown" {
            continue;
        }
        out.push(path.to_path_buf());
    }
    out.sort();
    out
}

fn card_state_from_path(cards_root: &Path, card_dir: &Path) -> String {
    let Ok(rel) = card_dir.strip_prefix(cards_root) else {
        return "unknown".to_string();
    };
    let comps: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if let Some(first) = comps.first().filter(|c| ALL_STATES.contains(&c.as_str())) {
        return first.clone();
    }
    if comps.len() >= 2 && comps[0].starts_with("team-") && ALL_STATES.contains(&comps[1].as_str())
    {
        return comps[1].clone();
    }
    "unknown".to_string()
}

fn canonical_card_id(cards_root: &Path, card_dir: &Path) -> String {
    let id_base = read_meta_id(card_dir).unwrap_or_else(|| file_stem_string(card_dir));
    let Ok(rel) = card_dir.strip_prefix(cards_root) else {
        return id_base;
    };
    let comps: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if comps.first().is_some_and(|c| c.starts_with("team-")) {
        format!("{}/{}", comps[0], id_base)
    } else {
        id_base
    }
}

fn read_meta_id(card_dir: &Path) -> Option<String> {
    let raw = fs::read_to_string(card_dir.join("meta.json")).ok()?;
    serde_json::from_str::<Value>(&raw)
        .ok()?
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn file_stem_string(path: &Path) -> String {
    path.file_stem()
        .map(OsString::from)
        .and_then(|s| s.into_string().ok())
        .unwrap_or_else(|| String::from("unknown"))
}

fn is_card_dir(path: &Path) -> bool {
    path.is_dir()
        && matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("bop") | Some("jobcard") | Some("card")
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parse_minimal_event_line() {
        let content = r#"{"ts":"2026-03-08T14:00:00Z","event":"created","state":"pending"}"#;
        let events = parse_events_content(content, "spec-001", "done");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "created");
        assert_eq!(events[0].state, "pending");
        assert_eq!(events[0].provider, None);
    }

    #[test]
    fn filter_errors_keeps_retries_and_failures() {
        let content = r#"
{"ts":"2026-03-08T14:00:00Z","event":"stage_transition","from":"pending","to":"running"}
{"ts":"2026-03-08T14:01:43Z","event":"stage_transition","from":"running","to":"pending","exit_code":75}
{"ts":"2026-03-08T14:06:44Z","event":"stage_transition","from":"running","to":"failed","exit_code":1}
"#;
        let mut events = parse_events_content(content, "spec-002", "failed");
        events.sort_by_key(|e| e.ts_parsed);
        let filtered = filter_events(events, true);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|e| e.to.as_deref() == Some("pending")));
        assert!(filtered.iter().any(|e| e.to.as_deref() == Some("failed")));
    }

    #[test]
    fn summarize_counts_retries_and_cost() {
        let content = r#"
{"ts":"2026-03-08T14:00:00Z","event":"stage_transition","from":"pending","to":"running"}
{"ts":"2026-03-08T14:01:43Z","event":"stage_transition","from":"running","to":"pending","exit_code":75}
{"ts":"2026-03-08T14:06:44Z","event":"stage_transition","from":"running","to":"done","cost_usd":0.22}
"#;
        let mut events = parse_events_content(content, "spec-003", "done");
        events.sort_by_key(|e| e.ts_parsed);
        let summary = summarize(&events);
        assert_eq!(summary.retries, 1);
        assert!((summary.total_cost_usd - 0.22).abs() < 1e-9);
        assert_eq!(summary.duration.num_seconds(), 404);
    }

    #[test]
    fn format_relative_outputs_plus_notation() {
        let base = Utc.with_ymd_and_hms(2026, 3, 8, 14, 0, 0).unwrap();
        let ts = Utc.with_ymd_and_hms(2026, 3, 8, 14, 1, 43).unwrap();
        assert_eq!(format_relative(base, ts), "+1m 43s");
        assert_eq!(format_relative(base, base), "0s");
    }
}
