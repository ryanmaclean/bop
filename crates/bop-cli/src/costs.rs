use anyhow::Context;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, Utc};
use clap::ValueEnum;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const ALL_STATES: [&str; 6] = ["drafts", "pending", "running", "done", "merged", "failed"];

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum CostsBy {
    Provider,
    Day,
    Card,
}

#[derive(Debug, Clone)]
struct CostEntry {
    provider: String,
    card_id: String,
    day: NaiveDate,
    cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
struct CostRow {
    key: String,
    cost_usd: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct BudgetStatus {
    daily_usd: f64,
    today_spend_usd: f64,
    exceeded: bool,
}

pub fn cmd_costs(
    cards_root: &Path,
    by: Option<CostsBy>,
    json: bool,
    budget: Option<f64>,
) -> anyhow::Result<()> {
    if let Some(v) = budget {
        write_daily_budget(cards_root, v)?;
        if !json {
            println!("Daily budget set to ${v:.2}");
        }
    }

    let entries = collect_cost_entries(cards_root)?;
    let day_today = Local::now().date_naive();
    let week_start = day_today - Duration::days(day_today.weekday().num_days_from_monday() as i64);
    let mode = by.unwrap_or(CostsBy::Provider);

    let rows = match mode {
        CostsBy::Provider => group_by_provider_week(&entries, week_start, day_today),
        CostsBy::Day => group_by_day(&entries),
        CostsBy::Card => group_by_card(&entries),
    };

    let total_cost_usd = normalized_zero(rows.iter().map(|r| r.cost_usd).sum::<f64>());
    let daily_budget = read_daily_budget(cards_root)?;
    let today_spend_usd = normalized_zero(
        entries
            .iter()
            .filter(|e| e.day == day_today)
            .map(|e| e.cost_usd)
            .sum::<f64>(),
    );
    let budget_status = daily_budget.map(|daily_usd| BudgetStatus {
        daily_usd,
        today_spend_usd,
        exceeded: today_spend_usd > daily_usd,
    });

    if json {
        let rows_json: Vec<_> = rows
            .into_iter()
            .map(|row| match mode {
                CostsBy::Provider => {
                    serde_json::json!({ "provider": row.key, "cost_usd": row.cost_usd })
                }
                CostsBy::Day => serde_json::json!({ "day": row.key, "cost_usd": row.cost_usd }),
                CostsBy::Card => {
                    serde_json::json!({ "card": row.key, "cost_usd": row.cost_usd })
                }
            })
            .collect();

        let mut payload = serde_json::json!({
            "by": mode_label(mode),
            "total_cost_usd": total_cost_usd,
            "rows": rows_json,
        });

        if let Some(status) = budget_status {
            payload["budget"] = serde_json::to_value(status)?;
        }

        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    print_rows(mode, &rows, total_cost_usd, week_start, day_today);

    if let Some(status) = budget_status {
        if status.exceeded {
            println!(
                "[warn] Daily budget ${:.2} exceeded — ${:.2} spent today",
                status.daily_usd, status.today_spend_usd
            );
        }
    }

    Ok(())
}

fn mode_label(mode: CostsBy) -> &'static str {
    match mode {
        CostsBy::Provider => "provider",
        CostsBy::Day => "day",
        CostsBy::Card => "card",
    }
}

fn normalized_zero(value: f64) -> f64 {
    if value.abs() < 1e-12 {
        0.0
    } else {
        value
    }
}

fn print_rows(
    mode: CostsBy,
    rows: &[CostRow],
    total_cost_usd: f64,
    week_start: NaiveDate,
    today: NaiveDate,
) {
    match mode {
        CostsBy::Provider => {
            println!("Spend by provider (this week: {} to {})", week_start, today);
            println!("{:<22} {:>12}", "PROVIDER", "COST (USD)");
        }
        CostsBy::Day => {
            println!("Spend by day");
            println!("{:<22} {:>12}", "DAY", "COST (USD)");
        }
        CostsBy::Card => {
            println!("Spend by card");
            println!("{:<22} {:>12}", "CARD", "COST (USD)");
        }
    }

    if rows.is_empty() {
        println!("{:<22} {:>12}", "-", "$0.00");
    } else {
        for row in rows {
            println!("{:<22} ${:>11.2}", row.key, row.cost_usd);
        }
    }

    println!("{:<22} ${:>11.2}", "TOTAL", total_cost_usd);
}

fn group_by_provider_week(
    entries: &[CostEntry],
    week_start: NaiveDate,
    today: NaiveDate,
) -> Vec<CostRow> {
    let mut by = BTreeMap::<String, f64>::new();
    for entry in entries {
        if entry.day < week_start || entry.day > today {
            continue;
        }
        *by.entry(entry.provider.clone()).or_insert(0.0) += entry.cost_usd;
    }

    sorted_desc_rows(by)
}

fn group_by_day(entries: &[CostEntry]) -> Vec<CostRow> {
    let mut by = BTreeMap::<String, f64>::new();
    for entry in entries {
        *by.entry(entry.day.to_string()).or_insert(0.0) += entry.cost_usd;
    }

    let mut rows: Vec<_> = by
        .into_iter()
        .map(|(key, cost_usd)| CostRow {
            key,
            cost_usd: normalized_zero(cost_usd),
        })
        .collect();
    rows.sort_by(|a, b| a.key.cmp(&b.key));
    rows
}

fn group_by_card(entries: &[CostEntry]) -> Vec<CostRow> {
    let mut by = BTreeMap::<String, f64>::new();
    for entry in entries {
        *by.entry(entry.card_id.clone()).or_insert(0.0) += entry.cost_usd;
    }

    sorted_desc_rows(by)
}

fn sorted_desc_rows(by: BTreeMap<String, f64>) -> Vec<CostRow> {
    let mut rows: Vec<_> = by
        .into_iter()
        .map(|(key, cost_usd)| CostRow {
            key,
            cost_usd: normalized_zero(cost_usd),
        })
        .collect();
    rows.sort_by(|a, b| {
        b.cost_usd
            .total_cmp(&a.cost_usd)
            .then_with(|| a.key.cmp(&b.key))
    });
    rows
}

fn collect_cost_entries(cards_root: &Path) -> anyhow::Result<Vec<CostEntry>> {
    let mut out = Vec::new();

    for card_dir in collect_all_card_dirs(cards_root) {
        let events_path = card_dir.join("logs").join("events.jsonl");
        if !events_path.exists() {
            continue;
        }

        let content = fs::read_to_string(&events_path)
            .with_context(|| format!("failed to read {}", events_path.display()))?;
        let card_id = canonical_card_id(cards_root, &card_dir);
        let fallback_provider = read_card_provider(&card_dir).unwrap_or_else(|| "unknown".into());

        out.extend(parse_cost_events(&content, &card_id, &fallback_provider));
    }

    Ok(out)
}

fn parse_cost_events(content: &str, card_id: &str, fallback_provider: &str) -> Vec<CostEntry> {
    let mut out = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(obj) = value.as_object() else {
            continue;
        };

        let Some(cost_usd) = extract_cost_usd(obj) else {
            continue;
        };
        if !cost_usd.is_finite() || cost_usd < 0.0 {
            continue;
        }

        let Some(ts) = extract_timestamp(obj) else {
            continue;
        };
        let day = ts.with_timezone(&Local).date_naive();

        let provider = extract_provider(obj)
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| fallback_provider.to_string());

        out.push(CostEntry {
            provider,
            card_id: card_id.to_string(),
            day,
            cost_usd: normalized_zero(cost_usd),
        });
    }

    out
}

fn extract_cost_usd(obj: &Map<String, Value>) -> Option<f64> {
    get_f64(obj, "cost_usd")
        .or_else(|| get_f64(obj, "cost"))
        .or_else(|| get_f64(obj, "costUsd"))
        .or_else(|| {
            obj.get("usage")
                .and_then(Value::as_object)
                .and_then(|usage| {
                    get_f64(usage, "total_cost_usd")
                        .or_else(|| get_f64(usage, "cost_usd"))
                        .or_else(|| get_f64(usage, "cost"))
                })
        })
}

fn extract_provider(obj: &Map<String, Value>) -> Option<String> {
    obj.get("provider")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            obj.get("run")
                .and_then(Value::as_object)
                .and_then(|run| run.get("provider"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn extract_timestamp(obj: &Map<String, Value>) -> Option<DateTime<Utc>> {
    for key in ["ts", "timestamp", "event_time", "eventTime", "time"] {
        let Some(raw) = obj.get(key).and_then(Value::as_str) else {
            continue;
        };
        if let Some(ts) = parse_ts(raw) {
            return Some(ts);
        }
    }
    None
}

fn get_f64(obj: &Map<String, Value>, key: &str) -> Option<f64> {
    let value = obj.get(key)?;
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
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

fn config_path(cards_root: &Path) -> PathBuf {
    cards_root.join(".bop").join("config.json")
}

fn read_daily_budget(cards_root: &Path) -> anyhow::Result<Option<f64>> {
    let path = config_path(cards_root);
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read config: {}", path.display()))?;
    let value: Value = serde_json::from_str(&content)
        .with_context(|| format!("invalid JSON in {}", path.display()))?;

    let daily = value
        .get("budget")
        .and_then(Value::as_object)
        .and_then(|budget| get_f64(budget, "daily_usd"));

    Ok(daily)
}

fn write_daily_budget(cards_root: &Path, daily_usd: f64) -> anyhow::Result<()> {
    if !daily_usd.is_finite() || daily_usd < 0.0 {
        anyhow::bail!("--budget must be a non-negative number");
    }

    let path = config_path(cards_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config dir: {}", parent.display()))?;
    }

    let mut root = if path.exists() {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config: {}", path.display()))?;
        serde_json::from_str::<Value>(&content)
            .with_context(|| format!("invalid JSON in {}", path.display()))?
    } else {
        Value::Object(Map::new())
    };

    let Some(root_obj) = root.as_object_mut() else {
        anyhow::bail!("config root must be a JSON object: {}", path.display());
    };

    let budget_entry = root_obj
        .entry("budget")
        .or_insert_with(|| Value::Object(Map::new()));

    let Some(budget_obj) = budget_entry.as_object_mut() else {
        anyhow::bail!(
            "config field 'budget' must be a JSON object: {}",
            path.display()
        );
    };

    budget_obj.insert("daily_usd".into(), Value::from(daily_usd));

    let json = serde_json::to_string_pretty(&root)?;
    fs::write(&path, json)
        .with_context(|| format!("failed to write config: {}", path.display()))?;

    Ok(())
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
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn read_card_provider(card_dir: &Path) -> Option<String> {
    let raw = fs::read_to_string(card_dir.join("meta.json")).ok()?;
    let value = serde_json::from_str::<Value>(&raw).ok()?;

    value
        .get("provider")
        .and_then(Value::as_str)
        .filter(|p| !p.trim().is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get("runs")
                .and_then(Value::as_array)
                .and_then(|runs| runs.last())
                .and_then(Value::as_object)
                .and_then(|run| run.get("provider"))
                .and_then(Value::as_str)
                .filter(|p| !p.trim().is_empty())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            value
                .get("provider_chain")
                .and_then(Value::as_array)
                .and_then(|chain| chain.first())
                .and_then(Value::as_str)
                .filter(|p| !p.trim().is_empty())
                .map(ToOwned::to_owned)
        })
}

fn file_stem_string(path: &Path) -> String {
    path.file_stem()
        .map(OsString::from)
        .and_then(|s| s.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string())
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
    use tempfile::tempdir;

    fn write_card(root: &Path, state: &str, dir: &str, meta_json: &str, events_jsonl: &str) {
        let card_dir = root.join(state).join(dir);
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        fs::write(card_dir.join("meta.json"), meta_json).unwrap();
        fs::write(card_dir.join("logs").join("events.jsonl"), events_jsonl).unwrap();
    }

    #[test]
    fn parse_cost_events_accepts_cost_variants() {
        let card_id = "card-1";
        let content = r#"
{"ts":"2026-03-08T10:00:00Z","provider":"codex","cost_usd":0.25}
{"ts":"2026-03-08T11:00:00Z","provider":"claude","cost":0.75}
{"ts":"2026-03-08T12:00:00Z","provider":"gemini","usage":{"total_cost_usd":1.00}}
{"ts":"2026-03-08T13:00:00Z","provider":"opencode","costUsd":"1.5"}
"#;

        let entries = parse_cost_events(content, card_id, "fallback");
        assert_eq!(entries.len(), 4);
        assert!((entries.iter().map(|e| e.cost_usd).sum::<f64>() - 3.5).abs() < 1e-9);
    }

    #[test]
    fn provider_week_groups_only_current_week() {
        let today = Local::now().date_naive();
        let week_start = today - Duration::days(today.weekday().num_days_from_monday() as i64);
        let older = week_start - Duration::days(1);

        let entries = vec![
            CostEntry {
                provider: "codex".into(),
                card_id: "a".into(),
                day: week_start,
                cost_usd: 1.0,
            },
            CostEntry {
                provider: "codex".into(),
                card_id: "b".into(),
                day: today,
                cost_usd: 2.0,
            },
            CostEntry {
                provider: "claude".into(),
                card_id: "c".into(),
                day: older,
                cost_usd: 5.0,
            },
        ];

        let rows = group_by_provider_week(&entries, week_start, today);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "codex");
        assert!((rows[0].cost_usd - 3.0).abs() < 1e-9);
    }

    #[test]
    fn write_and_read_daily_budget_preserves_other_fields() {
        let td = tempdir().unwrap();
        let cards_root = td.path();
        let cfg_dir = cards_root.join(".bop");
        fs::create_dir_all(&cfg_dir).unwrap();
        fs::write(
            cfg_dir.join("config.json"),
            r#"{"max_workers":4,"budget":{"daily_usd":1.25}}"#,
        )
        .unwrap();

        write_daily_budget(cards_root, 5.0).unwrap();
        let daily = read_daily_budget(cards_root).unwrap();
        assert_eq!(daily, Some(5.0));

        let raw = fs::read_to_string(cfg_dir.join("config.json")).unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["max_workers"], 4);
        assert_eq!(value["budget"]["daily_usd"], 5.0);
    }

    #[test]
    fn collect_cost_entries_reads_card_logs() {
        let td = tempdir().unwrap();
        let root = td.path();
        fs::create_dir_all(root.join("done")).unwrap();

        write_card(
            root,
            "done",
            "sample.bop",
            r#"{"id":"sample","provider":"codex"}"#,
            r#"{"ts":"2026-03-08T10:00:00Z","cost_usd":0.42}"#,
        );

        let entries = collect_cost_entries(root).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].card_id, "sample");
        assert_eq!(entries[0].provider, "codex");
        assert!((entries[0].cost_usd - 0.42).abs() < 1e-9);
    }
}
