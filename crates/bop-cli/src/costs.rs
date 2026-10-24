use anyhow::Context;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum CostsBy {
    Provider,
    Day,
    Card,
}

// ── Budget config ──────────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct BudgetInner {
    #[serde(skip_serializing_if = "Option::is_none")]
    daily_usd: Option<f64>,
}

/// Stored in `<cards_root>/.bop/config.json` as `{"budget": {...}}`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct CostsBudgetFile {
    #[serde(default)]
    budget: BudgetInner,
}

fn budget_config_path(root: &Path) -> PathBuf {
    root.join(".bop").join("config.json")
}

fn read_daily_budget(root: &Path) -> Option<f64> {
    let path = budget_config_path(root);
    let raw = fs::read_to_string(path).ok()?;
    let parsed: CostsBudgetFile = serde_json::from_str(&raw).ok()?;
    parsed.budget.daily_usd.filter(|&v| v > 0.0)
}

fn write_daily_budget(root: &Path, daily_usd: f64) -> anyhow::Result<()> {
    let path = budget_config_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create dir {}", parent.display()))?;
    }

    // Merge: read existing file and update only the budget field.
    let existing_raw = fs::read_to_string(&path).unwrap_or_default();
    let mut existing: serde_json::Value =
        serde_json::from_str(&existing_raw).unwrap_or(serde_json::json!({}));

    existing["budget"]["daily_usd"] = serde_json::json!(daily_usd);

    fs::write(&path, serde_json::to_string_pretty(&existing)?)
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

// ── Card data collection ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct CardCost {
    id: String,
    provider: String,
    day: Option<NaiveDate>,
    cost_usd: Option<f64>,
    tokens: Option<u64>,
    has_cost_data: bool,
}

#[derive(Debug, Deserialize, Default)]
struct RawMeta {
    #[serde(default)]
    id: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    provider_chain: Vec<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    finished_at: Option<String>,
    #[serde(default)]
    tokens_used: Option<u64>,
    #[serde(default)]
    cost_usd: Option<f64>,
    #[serde(default)]
    runs: Vec<RawRun>,
}

#[derive(Debug, Deserialize, Default)]
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

fn parse_ts(raw: &str) -> Option<DateTime<Utc>> {
    if raw.trim().is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|d| d.with_timezone(&Utc))
        .or_else(|| {
            NaiveDateTime::parse_from_str(raw.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S")
                .ok()
                .map(|n| n.and_utc())
        })
}

fn collect_cards(root: &Path) -> anyhow::Result<Vec<CardCost>> {
    let mut out = Vec::new();
    for state in ["done", "merged", "failed"] {
        for state_dir in state_dirs(root, state) {
            let entries = match fs::read_dir(&state_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let card_dir = entry.path();
                if !is_card_dir(&card_dir) {
                    continue;
                }
                let meta_path = card_dir.join("meta.json");
                if !meta_path.exists() {
                    continue;
                }
                match parse_card_cost(&meta_path, &card_dir) {
                    Ok(c) => out.push(c),
                    Err(err) => eprintln!(
                        "costs: skipped {} ({err:#})",
                        meta_path.strip_prefix(root).unwrap_or(&meta_path).display()
                    ),
                }
            }
        }
    }
    Ok(out)
}

fn state_dirs(root: &Path, state: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let top = root.join(state);
    if top.exists() {
        dirs.push(top);
    }
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if !entry.file_name().to_string_lossy().starts_with("team-") {
                continue;
            }
            let scoped = path.join(state);
            if scoped.exists() {
                dirs.push(scoped);
            }
        }
    }
    dirs.sort();
    dirs
}

fn is_card_dir(path: &Path) -> bool {
    path.is_dir()
        && matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("bop") | Some("jobcard") | Some("card")
        )
}

fn parse_card_cost(meta_path: &Path, card_dir: &Path) -> anyhow::Result<CardCost> {
    let raw = fs::read_to_string(meta_path)
        .with_context(|| format!("failed to read {}", meta_path.display()))?;
    let meta: RawMeta = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", meta_path.display()))?;

    let last_run = meta.runs.last();

    let ts = meta
        .finished_at
        .as_deref()
        .and_then(parse_ts)
        .or_else(|| meta.started_at.as_deref().and_then(parse_ts))
        .or_else(|| {
            last_run.and_then(|r| {
                r.ended_at
                    .as_deref()
                    .and_then(parse_ts)
                    .or_else(|| parse_ts(&r.started_at))
            })
        });

    let day = ts.map(|dt| dt.with_timezone(&Local).date_naive());

    let tokens = meta.tokens_used.or_else(|| {
        last_run.and_then(|run| {
            if run.prompt_tokens.is_none() && run.completion_tokens.is_none() {
                None
            } else {
                Some(run.prompt_tokens.unwrap_or(0) + run.completion_tokens.unwrap_or(0))
            }
        })
    });

    let cost_usd = meta.cost_usd.or_else(|| last_run.and_then(|r| r.cost_usd));
    let has_cost_data = matches!(cost_usd, Some(v) if v > 0.0);

    let provider = meta
        .provider
        .filter(|p| !p.trim().is_empty())
        .or_else(|| {
            last_run.and_then(|r| {
                let p = r.provider.trim();
                if p.is_empty() { None } else { Some(p.to_string()) }
            })
        })
        .or_else(|| meta.provider_chain.first().cloned())
        .unwrap_or_else(|| "unknown".to_string());

    let id = if meta.id.trim().is_empty() {
        card_dir
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string()
    } else {
        meta.id
    };

    Ok(CardCost {
        id,
        provider,
        day,
        cost_usd,
        tokens,
        has_cost_data,
    })
}

// ── Aggregation helpers ────────────────────────────────────────────────────

fn today_spend(cards: &[CardCost]) -> f64 {
    let today = Local::now().date_naive();
    cards
        .iter()
        .filter(|c| c.day == Some(today) && c.has_cost_data)
        .filter_map(|c| c.cost_usd)
        .sum()
}

fn week_spend(cards: &[CardCost]) -> f64 {
    let today = Local::now().date_naive();
    let week_start = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    cards
        .iter()
        .filter(|c| matches!(c.day, Some(d) if d >= week_start && d <= today))
        .filter_map(|c| c.cost_usd)
        .sum()
}

// ── JSON output types ──────────────────────────────────────────────────────

#[derive(Serialize)]
struct JsonSummary {
    today_usd: f64,
    week_usd: f64,
    all_usd: f64,
    budget_daily_usd: Option<f64>,
    budget_exceeded: bool,
}

#[derive(Serialize)]
struct JsonProviderRow {
    provider: String,
    cards: usize,
    cost_usd: f64,
    tokens: u64,
}

#[derive(Serialize)]
struct JsonDayRow {
    day: String,
    cards: usize,
    cost_usd: f64,
    tokens: u64,
}

#[derive(Serialize)]
struct JsonCardRow {
    id: String,
    provider: String,
    day: Option<String>,
    cost_usd: f64,
    tokens: u64,
}

// ── Public command ─────────────────────────────────────────────────────────

pub fn cmd_costs(
    root: &Path,
    by: Option<CostsBy>,
    budget_set: Option<f64>,
    json: bool,
) -> anyhow::Result<()> {
    // Persist budget if --budget flag provided
    if let Some(daily) = budget_set {
        write_daily_budget(root, daily)?;
        println!("[info] Daily budget set to ${:.2}", daily);
        return Ok(());
    }

    let cards = collect_cards(root)?;
    let budget = read_daily_budget(root);
    let today_usd = today_spend(&cards);
    let week_usd = week_spend(&cards);
    let all_usd: f64 = cards
        .iter()
        .filter(|c| c.has_cost_data)
        .filter_map(|c| c.cost_usd)
        .sum();
    let exceeded = budget.is_some_and(|b| today_usd > b);

    if json {
        print_json(&cards, by, budget, today_usd, week_usd, all_usd, exceeded)?;
    } else {
        print_text(&cards, by, budget, today_usd, week_usd, all_usd, exceeded);
    }

    Ok(())
}

fn print_text(
    cards: &[CardCost],
    by: Option<CostsBy>,
    budget: Option<f64>,
    today_usd: f64,
    week_usd: f64,
    all_usd: f64,
    exceeded: bool,
) {
    if exceeded {
        if let Some(b) = budget {
            println!(
                "[warn] Daily budget ${:.2} exceeded — ${:.2} spent today",
                b, today_usd
            );
        }
    }

    let line = "━".repeat(36);
    println!("{line}");
    println!("  bop costs");
    println!("{line}");
    println!("  Today     ${:.2}", today_usd);
    println!("  This week ${:.2}", week_usd);
    println!("  All time  ${:.2}", all_usd);
    if let Some(b) = budget {
        let pct = if b > 0.0 {
            (today_usd / b * 100.0).min(999.0)
        } else {
            0.0
        };
        println!("  Budget    ${:.2}/day  ({:.0}% used)", b, pct);
    }

    match by.unwrap_or(CostsBy::Provider) {
        CostsBy::Provider => {
            println!("\n  By provider");
            let rows = by_provider(cards);
            for row in &rows {
                println!(
                    "  {:<16} {:>4} cards  ${:>7.2}  {}",
                    row.0,
                    row.1,
                    row.2,
                    fmt_tokens(row.3)
                );
            }
        }
        CostsBy::Day => {
            println!("\n  By day (last 7 days)");
            let today = Local::now().date_naive();
            for row in by_day(cards, today, 7) {
                println!(
                    "  {:<12} {:>4} cards  ${:>7.2}  {}",
                    row.0,
                    row.1,
                    row.2,
                    fmt_tokens(row.3)
                );
            }
        }
        CostsBy::Card => {
            println!("\n  Per card");
            let rows = by_card(cards);
            for row in &rows {
                println!(
                    "  {:<30} {:<12} ${:>7.2}  {}",
                    truncate(&row.0, 30),
                    truncate(&row.1, 12),
                    row.2,
                    fmt_tokens(row.3)
                );
            }
        }
    }

    println!("{line}");
}

fn print_json(
    cards: &[CardCost],
    by: Option<CostsBy>,
    budget: Option<f64>,
    today_usd: f64,
    week_usd: f64,
    all_usd: f64,
    exceeded: bool,
) -> anyhow::Result<()> {
    let summary = JsonSummary {
        today_usd,
        week_usd,
        all_usd,
        budget_daily_usd: budget,
        budget_exceeded: exceeded,
    };

    let value = match by.unwrap_or(CostsBy::Provider) {
        CostsBy::Provider => {
            let rows: Vec<JsonProviderRow> = by_provider(cards)
                .into_iter()
                .map(|(p, c, cost, tok)| JsonProviderRow {
                    provider: p,
                    cards: c,
                    cost_usd: cost,
                    tokens: tok,
                })
                .collect();
            serde_json::json!({ "summary": summary, "by_provider": rows })
        }
        CostsBy::Day => {
            let today = Local::now().date_naive();
            let rows: Vec<JsonDayRow> = by_day(cards, today, 7)
                .into_iter()
                .map(|(d, c, cost, tok)| JsonDayRow {
                    day: d,
                    cards: c,
                    cost_usd: cost,
                    tokens: tok,
                })
                .collect();
            serde_json::json!({ "summary": summary, "by_day": rows })
        }
        CostsBy::Card => {
            let rows: Vec<JsonCardRow> = by_card(cards)
                .into_iter()
                .map(|(id, prov, cost, tok)| JsonCardRow {
                    id,
                    provider: prov,
                    day: None,
                    cost_usd: cost,
                    tokens: tok,
                })
                .collect();
            serde_json::json!({ "summary": summary, "by_card": rows })
        }
    };

    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

// ── Group-by helpers ───────────────────────────────────────────────────────

/// Returns (provider, card_count, cost_usd, tokens), sorted by card count desc.
fn by_provider(cards: &[CardCost]) -> Vec<(String, usize, f64, u64)> {
    let mut map: BTreeMap<String, (usize, f64, u64)> = BTreeMap::new();
    for c in cards {
        let entry = map.entry(c.provider.clone()).or_default();
        entry.0 += 1;
        if c.has_cost_data {
            entry.1 += c.cost_usd.unwrap_or(0.0);
        }
        entry.2 += c.tokens.unwrap_or(0);
    }
    let mut rows: Vec<(String, usize, f64, u64)> = map
        .into_iter()
        .map(|(p, (cnt, cost, tok))| (p, cnt, cost, tok))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    rows
}

/// Returns (day_string, card_count, cost_usd, tokens) for the last `n` days.
fn by_day(cards: &[CardCost], today: NaiveDate, n: i64) -> Vec<(String, usize, f64, u64)> {
    let mut rows = Vec::new();
    for offset in (0..n).rev() {
        let day = today - Duration::days(offset);
        let mut cnt = 0usize;
        let mut cost = 0.0f64;
        let mut tok = 0u64;
        for c in cards.iter().filter(|c| c.day == Some(day)) {
            cnt += 1;
            if c.has_cost_data {
                cost += c.cost_usd.unwrap_or(0.0);
            }
            tok += c.tokens.unwrap_or(0);
        }
        rows.push((day.to_string(), cnt, cost, tok));
    }
    rows
}

/// Returns (id, provider, cost_usd, tokens), sorted by cost desc.
fn by_card(cards: &[CardCost]) -> Vec<(String, String, f64, u64)> {
    let mut rows: Vec<(String, String, f64, u64)> = cards
        .iter()
        .map(|c| {
            (
                c.id.clone(),
                c.provider.clone(),
                c.cost_usd.unwrap_or(0.0),
                c.tokens.unwrap_or(0),
            )
        })
        .collect();
    rows.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    rows
}

fn fmt_tokens(n: u64) -> String {
    if n == 0 {
        return "—".to_string();
    }
    if n >= 1_000_000 {
        format!("{:.1}M tok", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}K tok", n as f64 / 1_000.0)
    } else {
        format!("{n} tok")
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_card(root: &Path, state: &str, name: &str, meta_json: &str) {
        let dir = root.join(state).join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("meta.json"), meta_json).unwrap();
    }

    fn make_root(td: &tempfile::TempDir) -> &Path {
        let root = td.path();
        fs::create_dir_all(root.join("done")).unwrap();
        fs::create_dir_all(root.join("merged")).unwrap();
        fs::create_dir_all(root.join("failed")).unwrap();
        root
    }

    #[test]
    fn costs_collect_basic() {
        let td = tempdir().unwrap();
        let root = make_root(&td);
        write_card(
            root,
            "done",
            "a.bop",
            r#"{"id":"a","provider":"codex","finished_at":"2026-03-08T10:00:00Z","cost_usd":1.50,"tokens_used":1000}"#,
        );
        write_card(
            root,
            "merged",
            "b.bop",
            r#"{"id":"b","provider":"claude","finished_at":"2026-03-08T11:00:00Z","cost_usd":0.75,"tokens_used":500}"#,
        );
        let cards = collect_cards(root).unwrap();
        assert_eq!(cards.len(), 2);
        let total: f64 = cards.iter().filter_map(|c| c.cost_usd).sum();
        assert!((total - 2.25).abs() < 1e-9);
    }

    #[test]
    fn costs_by_provider_groups_correctly() {
        let td = tempdir().unwrap();
        let root = make_root(&td);
        write_card(
            root,
            "done",
            "a.bop",
            r#"{"id":"a","provider":"codex","finished_at":"2026-03-08T10:00:00Z","cost_usd":1.0,"tokens_used":100}"#,
        );
        write_card(
            root,
            "done",
            "b.bop",
            r#"{"id":"b","provider":"codex","finished_at":"2026-03-08T11:00:00Z","cost_usd":2.0,"tokens_used":200}"#,
        );
        write_card(
            root,
            "done",
            "c.bop",
            r#"{"id":"c","provider":"claude","finished_at":"2026-03-08T12:00:00Z","cost_usd":3.0,"tokens_used":300}"#,
        );
        let cards = collect_cards(root).unwrap();
        let rows = by_provider(&cards);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "codex");
        assert_eq!(rows[0].1, 2);
        assert!((rows[0].2 - 3.0).abs() < 1e-9);
    }

    #[test]
    fn costs_by_day_last_seven() {
        let td = tempdir().unwrap();
        let root = make_root(&td);
        let today = Local::now().date_naive();
        let day2 = today - Duration::days(2);
        write_card(
            root,
            "done",
            "a.bop",
            &format!(
                r#"{{"id":"a","provider":"codex","finished_at":"{}T10:00:00Z","cost_usd":1.0,"tokens_used":100}}"#,
                today
            ),
        );
        write_card(
            root,
            "done",
            "b.bop",
            &format!(
                r#"{{"id":"b","provider":"codex","finished_at":"{}T10:00:00Z","cost_usd":2.0,"tokens_used":200}}"#,
                day2
            ),
        );
        let cards = collect_cards(root).unwrap();
        let rows = by_day(&cards, today, 7);
        assert_eq!(rows.len(), 7);
        let today_row = rows.iter().find(|r| r.0 == today.to_string()).unwrap();
        assert_eq!(today_row.1, 1);
        let day2_row = rows.iter().find(|r| r.0 == day2.to_string()).unwrap();
        assert_eq!(day2_row.1, 1);
    }

    #[test]
    fn costs_by_card_sorted_by_cost_desc() {
        let td = tempdir().unwrap();
        let root = make_root(&td);
        write_card(
            root,
            "done",
            "cheap.bop",
            r#"{"id":"cheap","provider":"ollama","finished_at":"2026-03-08T10:00:00Z","cost_usd":0.10}"#,
        );
        write_card(
            root,
            "done",
            "expensive.bop",
            r#"{"id":"expensive","provider":"claude","finished_at":"2026-03-08T10:00:00Z","cost_usd":5.00}"#,
        );
        let cards = collect_cards(root).unwrap();
        let rows = by_card(&cards);
        assert_eq!(rows[0].0, "expensive");
        assert_eq!(rows[1].0, "cheap");
    }

    #[test]
    fn budget_persist_and_read() {
        let td = tempdir().unwrap();
        let root = td.path();
        write_daily_budget(root, 5.00).unwrap();
        let loaded = read_daily_budget(root);
        assert_eq!(loaded, Some(5.00));
    }

    #[test]
    fn budget_merge_preserves_existing_keys() {
        let td = tempdir().unwrap();
        let root = td.path();
        fs::create_dir_all(root.join(".bop")).unwrap();
        fs::write(
            root.join(".bop").join("config.json"),
            r#"{"some_key":"value"}"#,
        )
        .unwrap();
        write_daily_budget(root, 10.0).unwrap();
        let raw = fs::read_to_string(root.join(".bop").join("config.json")).unwrap();
        let val: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(val["some_key"], "value");
        assert_eq!(val["budget"]["daily_usd"], 10.0);
    }

    #[test]
    fn costs_json_output_valid() {
        let td = tempdir().unwrap();
        let root = make_root(&td);
        write_card(
            root,
            "done",
            "x.bop",
            r#"{"id":"x","provider":"codex","finished_at":"2026-03-08T10:00:00Z","cost_usd":1.23,"tokens_used":100}"#,
        );
        // just ensure cmd_costs runs without error in json mode
        cmd_costs(root, None, None, true).unwrap();
    }

    #[test]
    fn budget_warning_text_shown_when_exceeded() {
        // This tests the logic, not stdout capture — just verify no panic and
        // exceeded flag is computed correctly.
        let td = tempdir().unwrap();
        let root = make_root(&td);
        let today = Local::now().date_naive();
        write_card(
            root,
            "done",
            "big.bop",
            &format!(
                r#"{{"id":"big","provider":"claude","finished_at":"{}T10:00:00Z","cost_usd":9.99}}"#,
                today
            ),
        );
        write_daily_budget(root, 5.00).unwrap();
        let cards = collect_cards(root).unwrap();
        let budget = read_daily_budget(root);
        let today_usd = today_spend(&cards);
        assert!(budget.is_some_and(|b| today_usd > b));
    }
}
