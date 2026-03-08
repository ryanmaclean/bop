use anyhow::Context;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum StatsBy {
    Provider,
    Day,
}

#[derive(Debug, Clone)]
struct CardUsage {
    id: String,
    state: String,
    provider: String,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    tokens: Option<u64>,
    cost_usd: Option<f64>,
    has_cost_data: bool,
}

#[derive(Debug, Clone, Default)]
struct Totals {
    cards: usize,
    cost_usd: f64,
    tokens: u64,
    no_cost_cards: usize,
}

#[derive(Debug, Clone)]
struct GroupRow {
    key: String,
    totals: Totals,
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

#[derive(Debug, Serialize)]
struct JsonTotals {
    cards: usize,
    cost_usd: f64,
    tokens: u64,
    no_cost_cards: usize,
}

#[derive(Debug, Serialize)]
struct JsonSummary {
    today: JsonTotals,
    week: JsonTotals,
    all: JsonTotals,
}

#[derive(Debug, Serialize)]
struct JsonProviderRow {
    provider: String,
    cards: usize,
    cost_usd: f64,
    tokens: u64,
    no_cost_cards: usize,
}

#[derive(Debug, Serialize)]
struct JsonDayRow {
    day: String,
    cards: usize,
    cost_usd: f64,
    tokens: u64,
    no_cost_cards: usize,
}

#[derive(Debug, Serialize)]
struct JsonCardDetail {
    id: String,
    state: String,
    provider: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    cost_usd: Option<f64>,
    tokens: Option<u64>,
    has_cost_data: bool,
}

pub fn cmd_stats(
    root: &Path,
    by: Option<StatsBy>,
    json: bool,
    card_id: Option<&str>,
) -> anyhow::Result<()> {
    let cards = collect_cards(root)?;

    if let Some(id) = card_id {
        let detail = card_detail(&cards, id).with_context(|| format!("card not found: {id}"))?;
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({ "card": detail }))?
            );
        } else {
            print_card_detail(&detail);
        }
        return Ok(());
    }

    let today = Local::now().date_naive();
    let week_start = today - Duration::days(today.weekday().num_days_from_monday() as i64);

    let today_totals = totals_for(&cards, |d| d == today);
    let week_totals = totals_for(&cards, |d| d >= week_start && d <= today);
    let all_totals = totals_all(&cards);

    let group = by.unwrap_or(StatsBy::Provider);

    if json {
        print_json_report(
            &today_totals,
            &week_totals,
            &all_totals,
            &cards,
            group,
            today,
        )?;
    } else {
        print_text_report(
            &today_totals,
            &week_totals,
            &all_totals,
            &cards,
            group,
            today,
        );
    }

    Ok(())
}

fn collect_cards(root: &Path) -> anyhow::Result<Vec<CardUsage>> {
    let mut out = Vec::new();

    for state in ["done", "merged", "failed"] {
        for state_dir in state_dirs(root, state) {
            let entries = match fs::read_dir(&state_dir) {
                Ok(entries) => entries,
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

                match parse_card_usage(&meta_path, state, &card_dir) {
                    Ok(card) => out.push(card),
                    Err(err) => eprintln!(
                        "stats: skipped {} ({err:#})",
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

    let root_state = root.join(state);
    if root_state.exists() {
        dirs.push(root_state);
    }

    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name();
            if !name.to_string_lossy().starts_with("team-") {
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
    if !path.is_dir() {
        return false;
    }
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("bop") | Some("jobcard") | Some("card")
    )
}

fn parse_card_usage(meta_path: &Path, state: &str, card_dir: &Path) -> anyhow::Result<CardUsage> {
    let raw = fs::read_to_string(meta_path)
        .with_context(|| format!("failed to read {}", meta_path.display()))?;
    let meta: RawMeta = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", meta_path.display()))?;

    let last_run = meta.runs.last();

    let started_at = meta
        .started_at
        .as_deref()
        .and_then(parse_ts)
        .or_else(|| last_run.and_then(|r| parse_ts(&r.started_at)));

    let finished_at = meta.finished_at.as_deref().and_then(parse_ts).or_else(|| {
        last_run
            .and_then(|r| r.ended_at.as_deref())
            .and_then(parse_ts)
    });

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
                if p.is_empty() {
                    None
                } else {
                    Some(p.to_string())
                }
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

    Ok(CardUsage {
        id,
        state: state.to_string(),
        provider,
        started_at,
        finished_at,
        tokens,
        cost_usd,
        has_cost_data,
    })
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

fn card_date(card: &CardUsage) -> Option<NaiveDate> {
    card.finished_at
        .or(card.started_at)
        .map(|dt| dt.with_timezone(&Local).date_naive())
}

fn totals_for<F>(cards: &[CardUsage], mut predicate: F) -> Totals
where
    F: FnMut(NaiveDate) -> bool,
{
    let mut out = Totals::default();
    for card in cards {
        let Some(day) = card_date(card) else {
            continue;
        };
        if predicate(day) {
            add_card(&mut out, card);
        }
    }
    out
}

fn totals_all(cards: &[CardUsage]) -> Totals {
    let mut out = Totals::default();
    for card in cards {
        add_card(&mut out, card);
    }
    out
}

fn add_card(totals: &mut Totals, card: &CardUsage) {
    totals.cards += 1;
    totals.tokens += card.tokens.unwrap_or(0);

    if card.has_cost_data {
        totals.cost_usd += card.cost_usd.unwrap_or(0.0);
    } else {
        totals.no_cost_cards += 1;
    }
}

fn by_provider(cards: &[CardUsage]) -> Vec<GroupRow> {
    let mut by: BTreeMap<String, Totals> = BTreeMap::new();
    for card in cards {
        add_card(by.entry(card.provider.clone()).or_default(), card);
    }

    let mut rows: Vec<GroupRow> = by
        .into_iter()
        .map(|(key, totals)| GroupRow { key, totals })
        .collect();

    rows.sort_by(|a, b| {
        b.totals
            .cards
            .cmp(&a.totals.cards)
            .then_with(|| a.key.cmp(&b.key))
    });
    rows
}

fn last_n_days(cards: &[CardUsage], today: NaiveDate, n: i64) -> Vec<GroupRow> {
    let mut rows = Vec::new();

    for offset in (0..n).rev() {
        let day = today - Duration::days(offset);
        let totals = totals_for(cards, |d| d == day);
        rows.push(GroupRow {
            key: day.to_string(),
            totals,
        });
    }

    rows
}

fn card_detail(cards: &[CardUsage], id: &str) -> Option<JsonCardDetail> {
    let card = cards.iter().find(|c| c.id == id)?;
    Some(JsonCardDetail {
        id: card.id.clone(),
        state: card.state.clone(),
        provider: card.provider.clone(),
        started_at: card.started_at.map(|d| d.to_rfc3339()),
        finished_at: card.finished_at.map(|d| d.to_rfc3339()),
        cost_usd: card.cost_usd,
        tokens: card.tokens,
        has_cost_data: card.has_cost_data,
    })
}

fn print_card_detail(detail: &JsonCardDetail) {
    println!("Card {}", detail.id);
    println!("  State:     {}", detail.state);
    println!("  Provider:  {}", detail.provider);
    println!(
        "  Started:   {}",
        detail.started_at.as_deref().unwrap_or("-")
    );
    println!(
        "  Finished:  {}",
        detail.finished_at.as_deref().unwrap_or("-")
    );

    if let Some(cost) = detail.cost_usd {
        println!("  Cost:      ${:.2}", cost);
    } else {
        println!("  Cost:      -");
    }

    if let Some(tokens) = detail.tokens {
        println!("  Tokens:    {}", fmt_int(tokens));
    } else {
        println!("  Tokens:    -");
    }

    println!(
        "  Cost data: {}",
        if detail.has_cost_data { "yes" } else { "no" }
    );
}

fn print_text_report(
    today: &Totals,
    week: &Totals,
    all: &Totals,
    cards: &[CardUsage],
    by: StatsBy,
    now_day: NaiveDate,
) {
    let line = "━".repeat(32);

    println!("{line}");
    println!("  Cost summary");
    println!("{line}");
    print_totals_row("Today", today);
    print_totals_row("This week", week);
    print_totals_row("All time", all);

    match by {
        StatsBy::Provider => {
            println!("\n  By provider (all time)");
            for row in by_provider(cards) {
                print_totals_row(&row.key, &row.totals);
            }
        }
        StatsBy::Day => {
            println!("\n  By day (last 7 days)");
            for row in last_n_days(cards, now_day, 7) {
                print_totals_row(&row.key, &row.totals);
            }
        }
    }

    println!("{line}");
    if all.cards > 0 {
        println!("  Avg cost/card: ${:.2}", all.cost_usd / all.cards as f64);
        println!(
            "  Avg tokens:    {}",
            fmt_int(all.tokens / all.cards as u64)
        );
    } else {
        println!("  Avg cost/card: $0.00");
        println!("  Avg tokens:    0");
    }
}

fn print_totals_row(label: &str, totals: &Totals) {
    let tokens = if totals.tokens == 0 {
        "—".to_string()
    } else {
        format!("{} tokens", fmt_compact_tokens(totals.tokens))
    };

    println!(
        "  {:<12} {:>4} cards    ${:>6.2}   {}",
        label, totals.cards, totals.cost_usd, tokens
    );

    if totals.no_cost_cards > 0 {
        println!(
            "                {} cards (no cost data)",
            totals.no_cost_cards
        );
    }
}

fn print_json_report(
    today: &Totals,
    week: &Totals,
    all: &Totals,
    cards: &[CardUsage],
    by: StatsBy,
    now_day: NaiveDate,
) -> anyhow::Result<()> {
    let summary = JsonSummary {
        today: to_json_totals(today),
        week: to_json_totals(week),
        all: to_json_totals(all),
    };

    let value = match by {
        StatsBy::Provider => {
            let rows: Vec<JsonProviderRow> = by_provider(cards)
                .into_iter()
                .map(|row| JsonProviderRow {
                    provider: row.key,
                    cards: row.totals.cards,
                    cost_usd: row.totals.cost_usd,
                    tokens: row.totals.tokens,
                    no_cost_cards: row.totals.no_cost_cards,
                })
                .collect();
            serde_json::json!({ "summary": summary, "by_provider": rows })
        }
        StatsBy::Day => {
            let rows: Vec<JsonDayRow> = last_n_days(cards, now_day, 7)
                .into_iter()
                .map(|row| JsonDayRow {
                    day: row.key,
                    cards: row.totals.cards,
                    cost_usd: row.totals.cost_usd,
                    tokens: row.totals.tokens,
                    no_cost_cards: row.totals.no_cost_cards,
                })
                .collect();
            serde_json::json!({ "summary": summary, "by_day": rows })
        }
    };

    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn to_json_totals(t: &Totals) -> JsonTotals {
    JsonTotals {
        cards: t.cards,
        cost_usd: t.cost_usd,
        tokens: t.tokens,
        no_cost_cards: t.no_cost_cards,
    }
}

fn fmt_compact_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        let m = n as f64 / 1_000_000.0;
        if (m.fract() - 0.0).abs() < f64::EPSILON {
            format!("{m:.0}M")
        } else {
            format!("{m:.1}M")
        }
    } else if n >= 1_000 {
        format!("{:.0}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn fmt_int(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    let mut count = 0usize;
    for ch in s.chars().rev() {
        if count == 3 {
            out.push(',');
            count = 0;
        }
        out.push(ch);
        count += 1;
    }
    out.chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_card(root: &Path, state: &str, dir: &str, meta_json: &str) {
        let card_dir = root.join(state).join(dir);
        fs::create_dir_all(&card_dir).unwrap();
        fs::write(card_dir.join("meta.json"), meta_json).unwrap();
    }

    #[test]
    fn stats_totals_exclude_zero_and_null_costs() {
        let td = tempdir().unwrap();
        let root = td.path();
        fs::create_dir_all(root.join("done")).unwrap();
        fs::create_dir_all(root.join("merged")).unwrap();
        fs::create_dir_all(root.join("failed")).unwrap();

        write_card(
            root,
            "done",
            "a.bop",
            r#"{"id":"a","provider":"codex","started_at":"2026-03-08T00:00:00Z","finished_at":"2026-03-08T00:10:00Z","tokens_used":1000,"cost_usd":1.25}"#,
        );
        write_card(
            root,
            "done",
            "b.bop",
            r#"{"id":"b","provider":"ollama","started_at":"2026-03-08T01:00:00Z","finished_at":"2026-03-08T01:10:00Z","tokens_used":500,"cost_usd":0.0}"#,
        );
        write_card(
            root,
            "failed",
            "c.bop",
            r#"{"id":"c","provider":"claude","started_at":"2026-03-07T01:00:00Z","finished_at":"2026-03-07T01:10:00Z","tokens_used":300,"cost_usd":null}"#,
        );

        let cards = collect_cards(root).unwrap();
        let all = totals_all(&cards);

        assert_eq!(all.cards, 3);
        assert_eq!(all.no_cost_cards, 2);
        assert!((all.cost_usd - 1.25).abs() < 1e-9);
        assert_eq!(all.tokens, 1800);
    }

    #[test]
    fn stats_group_by_provider() {
        let td = tempdir().unwrap();
        let root = td.path();
        fs::create_dir_all(root.join("done")).unwrap();
        fs::create_dir_all(root.join("merged")).unwrap();
        fs::create_dir_all(root.join("failed")).unwrap();

        write_card(
            root,
            "done",
            "a.bop",
            r#"{"id":"a","provider":"codex","started_at":"2026-03-08T00:00:00Z","finished_at":"2026-03-08T00:10:00Z","tokens_used":1000,"cost_usd":1.0}"#,
        );
        write_card(
            root,
            "merged",
            "b.bop",
            r#"{"id":"b","provider":"codex","started_at":"2026-03-08T00:00:00Z","finished_at":"2026-03-08T00:10:00Z","tokens_used":2000,"cost_usd":2.0}"#,
        );
        write_card(
            root,
            "failed",
            "c.bop",
            r#"{"id":"c","provider":"claude","started_at":"2026-03-08T00:00:00Z","finished_at":"2026-03-08T00:10:00Z","tokens_used":3000,"cost_usd":3.0}"#,
        );

        let cards = collect_cards(root).unwrap();
        let by = by_provider(&cards);

        assert_eq!(by.len(), 2);
        assert_eq!(by[0].key, "codex");
        assert_eq!(by[0].totals.cards, 2);
        assert!((by[0].totals.cost_usd - 3.0).abs() < 1e-9);
        assert_eq!(by[0].totals.tokens, 3000);
        assert_eq!(by[1].key, "claude");
    }

    #[test]
    fn stats_by_day_last_seven_days() {
        let today = Local::now().date_naive();
        let td = tempdir().unwrap();
        let root = td.path();
        fs::create_dir_all(root.join("done")).unwrap();
        fs::create_dir_all(root.join("merged")).unwrap();
        fs::create_dir_all(root.join("failed")).unwrap();

        let day0 = today;
        let day2 = today - Duration::days(2);

        write_card(
            root,
            "done",
            "a.bop",
            &format!(
                r#"{{"id":"a","provider":"codex","finished_at":"{}T12:00:00Z","tokens_used":100,"cost_usd":1.0}}"#,
                day0
            ),
        );
        write_card(
            root,
            "done",
            "b.bop",
            &format!(
                r#"{{"id":"b","provider":"codex","finished_at":"{}T12:00:00Z","tokens_used":200,"cost_usd":2.0}}"#,
                day2
            ),
        );

        let cards = collect_cards(root).unwrap();
        let rows = last_n_days(&cards, today, 7);

        assert_eq!(rows.len(), 7);
        let today_row = rows
            .iter()
            .find(|r| r.key == day0.to_string())
            .expect("today row");
        assert_eq!(today_row.totals.cards, 1);
        let day2_row = rows
            .iter()
            .find(|r| r.key == day2.to_string())
            .expect("day2 row");
        assert_eq!(day2_row.totals.cards, 1);
    }

    #[test]
    fn card_detail_reads_from_runs_when_top_level_missing() {
        let td = tempdir().unwrap();
        let root = td.path();
        fs::create_dir_all(root.join("done")).unwrap();
        fs::create_dir_all(root.join("merged")).unwrap();
        fs::create_dir_all(root.join("failed")).unwrap();

        write_card(
            root,
            "done",
            "fallback.bop",
            r#"{
                "id":"fallback",
                "runs":[{
                    "provider":"gemini",
                    "started_at":"2026-03-08T00:00:00Z",
                    "ended_at":"2026-03-08T00:10:00Z",
                    "prompt_tokens":123,
                    "completion_tokens":77,
                    "cost_usd":0.42
                }]
            }"#,
        );

        let cards = collect_cards(root).unwrap();
        let detail = card_detail(&cards, "fallback").unwrap();
        assert_eq!(detail.provider, "gemini");
        assert_eq!(detail.tokens, Some(200));
        assert_eq!(detail.cost_usd, Some(0.42));
        assert!(detail.has_cost_data);
    }
}
