use anyhow::Context;
use bop_core::{write_meta, Meta};
use chrono::{Local, Utc};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::{dispatcher, paths, providers};

const BENCHMARK_TEMPLATE_NAME: &str = "benchmark-template.bop";
const BENCHMARK_STAGE: &str = "implement";
const JUDGE_OUTPUT_MAX_CHARS: usize = 6000;

#[derive(Debug, Clone)]
struct BenchmarkRun {
    provider: String,
    run: usize,
    duration_secs: u64,
    tokens: Option<u64>,
    cost_usd: Option<f64>,
    exit: i32,
    score: Option<f64>,
    output_text: String,
}

#[derive(Debug, Clone)]
struct ProviderAggregate {
    provider: String,
    avg_duration_secs: f64,
    avg_tokens: Option<f64>,
    avg_cost_usd: Option<f64>,
    score: Option<f64>,
    runs_total: usize,
    failed_runs: usize,
}

#[derive(Debug, Clone)]
struct Recommendation {
    provider: String,
    detail: String,
}

#[derive(Debug, Serialize)]
struct BenchmarkRunJson {
    provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<usize>,
    duration_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_usd: Option<f64>,
    exit: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    score: Option<f64>,
}

#[derive(Debug, Serialize)]
struct BenchmarkJson {
    spec: String,
    runs: Vec<BenchmarkRunJson>,
    recommendation: String,
}

struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    fn new() -> anyhow::Result<Self> {
        let uniq = format!(
            "{}-{}-{}",
            Utc::now().timestamp_millis(),
            std::process::id(),
            dispatcher::short_run_id()
        );
        let root = std::env::temp_dir().join(format!("bop-benchmark-{uniq}"));
        fs::create_dir_all(&root)
            .with_context(|| format!("failed to create temp workspace {}", root.display()))?;
        Ok(Self { root })
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub async fn cmd_benchmark(
    cards_dir: &Path,
    spec_file: &str,
    provider_args: Vec<String>,
    runs: usize,
    judge: Option<&str>,
    json: bool,
) -> anyhow::Result<()> {
    if runs == 0 {
        anyhow::bail!("--runs must be >= 1");
    }

    let providers = normalize_provider_list(provider_args);
    if providers.len() < 2 {
        anyhow::bail!("--providers requires at least 2 providers");
    }

    let spec_path = PathBuf::from(spec_file);
    let spec_text = fs::read_to_string(&spec_path)
        .with_context(|| format!("failed to read spec file {}", spec_path.display()))?;

    let providers_file = providers::read_providers(cards_dir).with_context(|| {
        format!(
            "failed to read {}",
            cards_dir.join("providers.json").display()
        )
    })?;

    let temp = TempWorkspace::new()?;
    let temp_cards_dir = temp.root.join(".cards");
    paths::ensure_cards_layout(&temp_cards_dir)?;
    maybe_copy_system_context(cards_dir, &temp_cards_dir);

    let template_dir = build_benchmark_template(&temp_cards_dir, &spec_text)?;
    let mut all_runs = Vec::new();

    for provider in &providers {
        for run in 1..=runs {
            let cfg = providers_file.providers.get(provider);
            let run_result =
                execute_provider_run(&temp_cards_dir, &template_dir, provider, run, cfg, None)
                    .await;
            all_runs.push(run_result);
        }
    }

    let mut judge_error: Option<String> = None;
    if let Some(judge_provider) = judge {
        match run_judge(
            &temp_cards_dir,
            &template_dir,
            &providers,
            &all_runs,
            judge_provider,
            &providers_file,
            &spec_text,
        )
        .await
        {
            Ok(scores) => {
                for run in &mut all_runs {
                    run.score = scores.get(&run.provider).copied();
                }
            }
            Err(err) => {
                judge_error = Some(err.to_string());
            }
        }
    }

    let aggregates = aggregate_providers(&providers, &all_runs);
    let recommendation = pick_recommendation(&aggregates);

    let json_doc = BenchmarkJson {
        spec: spec_path.display().to_string(),
        runs: all_runs
            .iter()
            .map(|run| BenchmarkRunJson {
                provider: run.provider.clone(),
                run: (runs > 1).then_some(run.run),
                duration_secs: run.duration_secs,
                tokens: run.tokens,
                cost_usd: run.cost_usd,
                exit: run.exit,
                score: run.score,
            })
            .collect(),
        recommendation: recommendation
            .as_ref()
            .map(|r| r.provider.clone())
            .unwrap_or_else(|| "none".to_string()),
    };

    let results_path = write_results_file(&json_doc)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&json_doc)?);
        return Ok(());
    }

    print_table(&spec_path, &aggregates);
    if let Some(rec) = recommendation {
        println!("  Recommendation: {}  ({})", rec.provider, rec.detail);
    } else {
        println!("  Recommendation: none (no successful runs)");
    }
    if let Some(err) = judge_error {
        println!("  Judge warning: {}", err);
    }
    println!("  Saved: {}", results_path.display());
    Ok(())
}

fn normalize_provider_list(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();

    for value in values {
        for part in value.split(',') {
            let name = part.trim();
            if name.is_empty() || seen.contains(name) {
                continue;
            }
            seen.insert(name.to_string());
            out.push(name.to_string());
        }
    }

    out
}

fn maybe_copy_system_context(src_cards_dir: &Path, dst_cards_dir: &Path) {
    let src = src_cards_dir.join("system_context.md");
    let dst = dst_cards_dir.join("system_context.md");
    if !src.exists() {
        return;
    }
    if let Err(err) = paths::cow_copy_file(&src, &dst) {
        eprintln!(
            "benchmark: warning: failed to clone {}: {}",
            src.display(),
            err
        );
    }
}

fn build_benchmark_template(cards_dir: &Path, spec_text: &str) -> anyhow::Result<PathBuf> {
    let template_dir = cards_dir.join("templates").join(BENCHMARK_TEMPLATE_NAME);
    fs::create_dir_all(template_dir.join("logs"))?;
    fs::create_dir_all(template_dir.join("output"))?;
    fs::write(template_dir.join("spec.md"), spec_text)?;
    fs::write(template_dir.join("prompt.md"), "{{spec}}\n")?;

    let meta = Meta {
        id: "benchmark-template".to_string(),
        created: Utc::now(),
        stage: BENCHMARK_STAGE.to_string(),
        worktree_branch: Some("job/benchmark-template".to_string()),
        retry_count: Some(0),
        ..Default::default()
    };
    write_meta(&template_dir, &meta)?;

    Ok(template_dir)
}

async fn execute_provider_run(
    temp_cards_dir: &Path,
    template_dir: &Path,
    provider: &str,
    run: usize,
    provider_cfg: Option<&providers::AdapterConfig>,
    spec_override: Option<&str>,
) -> BenchmarkRun {
    let mut result = BenchmarkRun {
        provider: provider.to_string(),
        run,
        duration_secs: 0,
        tokens: None,
        cost_usd: None,
        exit: 1,
        score: None,
        output_text: String::new(),
    };

    let Some(cfg) = provider_cfg else {
        result.exit = 127;
        result.output_text = format!("provider '{}' not found in providers.json", provider);
        return result;
    };

    let safe_provider = sanitize_for_filename(provider);
    let card_name = format!("bench-{}-run-{}.bop", safe_provider, run);
    let card_dir = temp_cards_dir.join("running").join(card_name);

    if let Err(err) = paths::clone_template(template_dir, &card_dir) {
        result.exit = 1;
        result.output_text = format!("failed to clone template: {err}");
        return result;
    }

    if let Some(spec) = spec_override {
        let _ = fs::write(card_dir.join("spec.md"), spec);
    }

    let mut meta = bop_core::read_meta(&card_dir).unwrap_or_default();
    meta.id = format!("bench-{}-run-{}", safe_provider, run);
    meta.created = Utc::now();
    meta.stage = BENCHMARK_STAGE.to_string();
    meta.provider_chain = vec![provider.to_string()];
    meta.retry_count = Some(0);
    meta.failure_reason = None;
    meta.exit_code = None;
    meta.runs.clear();

    if let Err(err) = write_meta(&card_dir, &meta) {
        result.exit = 1;
        result.output_text = format!("failed to write run meta: {err}");
        return result;
    }

    let started = Instant::now();
    let run_result = dispatcher::run_card(
        temp_cards_dir,
        &card_dir,
        &cfg.command,
        provider,
        &cfg.env,
        cfg.model.as_deref(),
        cfg.rate_limit_exit,
    )
    .await;
    result.duration_secs = started.elapsed().as_secs();

    match run_result {
        Ok((exit, meta_after)) => {
            result.exit = exit;
            let (tokens, cost) = extract_usage(meta_after.as_ref());
            result.tokens = tokens;
            result.cost_usd = cost.or_else(|| default_cost_for_provider(provider));
        }
        Err(err) => {
            result.exit = 1;
            result.output_text = format!("failed to execute provider '{}': {err}", provider);
        }
    }

    if result.output_text.trim().is_empty() {
        result.output_text = read_output_text(&card_dir);
    }

    result
}

fn extract_usage(meta: Option<&Meta>) -> (Option<u64>, Option<f64>) {
    let Some(meta) = meta else {
        return (None, None);
    };
    let Some(run) = meta.runs.last() else {
        return (None, None);
    };
    let tokens = match (run.prompt_tokens, run.completion_tokens) {
        (None, None) => None,
        (prompt, completion) => Some(prompt.unwrap_or(0) + completion.unwrap_or(0)),
    };
    (tokens, run.cost_usd)
}

fn read_output_text(card_dir: &Path) -> String {
    let paths = [
        card_dir.join("output").join("result.md"),
        card_dir.join("logs").join("stdout.log"),
        card_dir.join("logs").join("stderr.log"),
    ];
    for path in &paths {
        if let Ok(content) = fs::read_to_string(path) {
            if !content.trim().is_empty() {
                return content;
            }
        }
    }
    String::new()
}

fn default_cost_for_provider(provider: &str) -> Option<f64> {
    match provider {
        "ollama" | "ollama-local" => Some(0.0),
        _ => None,
    }
}

async fn run_judge(
    temp_cards_dir: &Path,
    template_dir: &Path,
    providers: &[String],
    runs: &[BenchmarkRun],
    judge_provider: &str,
    providers_file: &providers::ProvidersFile,
    spec_text: &str,
) -> anyhow::Result<BTreeMap<String, f64>> {
    let Some(judge_cfg) = providers_file.providers.get(judge_provider) else {
        anyhow::bail!(
            "judge provider '{}' not found in providers.json",
            judge_provider
        );
    };

    let judge_prompt = build_judge_prompt(spec_text, providers, runs);
    let judge_run = execute_provider_run(
        temp_cards_dir,
        template_dir,
        judge_provider,
        1,
        Some(judge_cfg),
        Some(&judge_prompt),
    )
    .await;

    if judge_run.exit != 0 {
        anyhow::bail!(
            "judge provider '{}' failed with exit {}",
            judge_provider,
            judge_run.exit
        );
    }

    parse_scores_map(&judge_run.output_text)
        .with_context(|| "judge output did not include a valid scores JSON object")
}

fn build_judge_prompt(spec_text: &str, providers: &[String], runs: &[BenchmarkRun]) -> String {
    let mut out = String::new();
    out.push_str("You are evaluating outputs from multiple providers for the same coding spec.\n");
    out.push_str(
        "Score each provider from 0.0 to 10.0 based on quality, correctness, and completeness.\n",
    );
    out.push_str("Return ONLY this JSON object format:\n");
    out.push_str("{\"scores\": {\"provider-name\": 0.0}}\n\n");
    out.push_str("Spec:\n");
    out.push_str(spec_text);
    out.push_str("\n\nProvider outputs:\n");

    for provider in providers {
        out.push_str(&format!("\n### {}\n", provider));
        let mut provider_runs = runs
            .iter()
            .filter(|run| run.provider == *provider)
            .peekable();
        if provider_runs.peek().is_none() {
            out.push_str("- no runs\n");
            continue;
        }

        for run in provider_runs {
            out.push_str(&format!(
                "\nRun {} (exit {}, {}s)\n",
                run.run, run.exit, run.duration_secs
            ));
            let snippet = truncate_for_judge(&run.output_text);
            if snippet.trim().is_empty() {
                out.push_str("[no output]\n");
            } else {
                out.push_str(&snippet);
                out.push('\n');
            }
        }
    }

    out
}

fn truncate_for_judge(text: &str) -> String {
    if text.chars().count() <= JUDGE_OUTPUT_MAX_CHARS {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars().take(JUDGE_OUTPUT_MAX_CHARS) {
        out.push(ch);
    }
    out.push_str("\n[truncated]");
    out
}

fn parse_scores_map(text: &str) -> Option<BTreeMap<String, f64>> {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        if let Some(scores) = extract_scores_from_value(&value) {
            return Some(scores);
        }
    }

    for candidate in json_object_candidates(text) {
        if let Ok(value) = serde_json::from_str::<Value>(candidate) {
            if let Some(scores) = extract_scores_from_value(&value) {
                return Some(scores);
            }
        }
    }

    None
}

fn json_object_candidates(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut starts = Vec::new();

    for (idx, ch) in text.char_indices() {
        if ch == '{' {
            starts.push(idx);
        }
    }

    for start in starts {
        let mut depth = 0i32;
        for (idx, ch) in text[start..].char_indices() {
            if ch == '{' {
                depth += 1;
            } else if ch == '}' {
                depth -= 1;
                if depth == 0 {
                    let end = start + idx + 1;
                    out.push(&text[start..end]);
                    break;
                }
            }
        }
    }

    out
}

fn extract_scores_from_value(value: &Value) -> Option<BTreeMap<String, f64>> {
    match value {
        Value::Object(obj) => {
            if let Some(scores_value) = obj.get("scores") {
                let scores_obj = scores_value.as_object()?;
                let mut scores = BTreeMap::new();
                for (provider, score) in scores_obj {
                    let parsed = score
                        .as_f64()
                        .or_else(|| score.as_str().and_then(|s| s.parse::<f64>().ok()));
                    if let Some(parsed) = parsed {
                        scores.insert(provider.clone(), parsed.clamp(0.0, 10.0));
                    }
                }
                if !scores.is_empty() {
                    return Some(scores);
                }
            }
            for child in obj.values() {
                if let Some(scores) = extract_scores_from_value(child) {
                    return Some(scores);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items {
                if let Some(scores) = extract_scores_from_value(item) {
                    return Some(scores);
                }
            }
            None
        }
        _ => None,
    }
}

fn aggregate_providers(provider_order: &[String], runs: &[BenchmarkRun]) -> Vec<ProviderAggregate> {
    let mut out = Vec::new();
    for provider in provider_order {
        let provider_runs: Vec<&BenchmarkRun> = runs
            .iter()
            .filter(|run| run.provider == *provider)
            .collect();
        if provider_runs.is_empty() {
            continue;
        }

        let runs_total = provider_runs.len();
        let failed_runs = provider_runs.iter().filter(|run| run.exit != 0).count();
        let avg_duration_secs = provider_runs
            .iter()
            .map(|run| run.duration_secs as f64)
            .sum::<f64>()
            / runs_total as f64;
        let avg_tokens = average(
            provider_runs
                .iter()
                .filter_map(|run| run.tokens.map(|v| v as f64)),
        );
        let avg_cost_usd = average(provider_runs.iter().filter_map(|run| run.cost_usd));
        let score = provider_runs.iter().find_map(|run| run.score);

        out.push(ProviderAggregate {
            provider: provider.clone(),
            avg_duration_secs,
            avg_tokens,
            avg_cost_usd,
            score,
            runs_total,
            failed_runs,
        });
    }
    out
}

fn average<I>(values: I) -> Option<f64>
where
    I: Iterator<Item = f64>,
{
    let mut count = 0usize;
    let mut sum = 0.0f64;
    for value in values {
        count += 1;
        sum += value;
    }
    (count > 0).then_some(sum / count as f64)
}

fn pick_recommendation(aggregates: &[ProviderAggregate]) -> Option<Recommendation> {
    let has_scores = aggregates.iter().any(|agg| agg.score.is_some());

    if has_scores {
        let mut ranked: Vec<(&ProviderAggregate, f64)> = aggregates
            .iter()
            .filter(|agg| agg.failed_runs < agg.runs_total)
            .filter_map(|agg| {
                let score = agg.score?;
                if score <= 0.0 {
                    return None;
                }
                let cost = agg.avg_cost_usd.unwrap_or(f64::INFINITY);
                Some((agg, cost / score))
            })
            .collect();

        ranked.sort_by(|(a_agg, a_ratio), (b_agg, b_ratio)| {
            a_ratio
                .partial_cmp(b_ratio)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    b_agg
                        .score
                        .unwrap_or(0.0)
                        .partial_cmp(&a_agg.score.unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| {
                    a_agg
                        .avg_duration_secs
                        .partial_cmp(&b_agg.avg_duration_secs)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        if let Some((best, ratio)) = ranked.first() {
            return Some(Recommendation {
                provider: best.provider.clone(),
                detail: format!("best cost/quality ratio: ${:.3}/point", ratio),
            });
        }
    }

    let mut ranked: Vec<&ProviderAggregate> = aggregates
        .iter()
        .filter(|agg| agg.failed_runs < agg.runs_total)
        .collect();
    ranked.sort_by(|a, b| {
        let a_cost = a.avg_cost_usd.unwrap_or(f64::INFINITY);
        let b_cost = b.avg_cost_usd.unwrap_or(f64::INFINITY);
        a_cost
            .partial_cmp(&b_cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a.avg_duration_secs
                    .partial_cmp(&b.avg_duration_secs)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    ranked.first().map(|best| Recommendation {
        provider: best.provider.clone(),
        detail: "lowest cost among successful runs".to_string(),
    })
}

fn write_results_file(doc: &BenchmarkJson) -> anyhow::Result<PathBuf> {
    let ts = Local::now().format("%Y%m%d-%H%M%S");
    let path = std::env::current_dir()?.join(format!("bop-benchmark-{}.json", ts));
    fs::write(&path, serde_json::to_vec_pretty(doc)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

fn print_table(spec_path: &Path, rows: &[ProviderAggregate]) {
    println!("bop benchmark results — {}", spec_path.display());
    println!("{}", "━".repeat(78));
    println!(
        "  {:<14} {:>8} {:>10} {:>9} {:>12} {:>8}",
        "Provider", "Time", "Tokens", "Cost", "Exit", "Score"
    );
    println!(
        "  {:<14} {:>8} {:>10} {:>9} {:>12} {:>8}",
        "────────", "────", "──────", "────", "────", "─────"
    );

    for row in rows {
        println!(
            "  {:<14} {:>8} {:>10} {:>9} {:>12} {:>8}",
            row.provider,
            fmt_duration(row.avg_duration_secs),
            fmt_tokens(row.avg_tokens),
            fmt_cost(row.avg_cost_usd),
            fmt_exit(row.failed_runs, row.runs_total),
            fmt_score(row.score)
        );
    }
    println!("{}", "━".repeat(78));
}

fn fmt_duration(secs: f64) -> String {
    let total = secs.round().max(0.0) as u64;
    let minutes = total / 60;
    let rem = total % 60;
    format!("{minutes}m {rem:02}s")
}

fn fmt_tokens(tokens: Option<f64>) -> String {
    let Some(tokens) = tokens else {
        return "—".to_string();
    };
    format_int(tokens.round().max(0.0) as u64)
}

fn fmt_cost(cost: Option<f64>) -> String {
    cost.map(|v| format!("${v:.2}"))
        .unwrap_or_else(|| "—".to_string())
}

fn fmt_score(score: Option<f64>) -> String {
    score
        .map(|v| format!("{v:.1}/10"))
        .unwrap_or_else(|| "—".to_string())
}

fn fmt_exit(failed: usize, total: usize) -> String {
    if failed == 0 {
        "0".to_string()
    } else {
        format!("{failed}/{total} failed")
    }
}

fn format_int(value: u64) -> String {
    let s = value.to_string();
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

fn sanitize_for_filename(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_provider_list_trims_and_dedupes() {
        let got = normalize_provider_list(vec![
            " codex,claude ".to_string(),
            "claude".to_string(),
            "ollama-local".to_string(),
        ]);
        assert_eq!(got, vec!["codex", "claude", "ollama-local"]);
    }

    #[test]
    fn parse_scores_map_from_direct_json() {
        let input = r#"{"scores":{"codex":8.2,"claude":"9.1"}}"#;
        let scores = parse_scores_map(input).expect("scores");
        assert_eq!(scores.get("codex").copied(), Some(8.2));
        assert_eq!(scores.get("claude").copied(), Some(9.1));
    }

    #[test]
    fn parse_scores_map_from_embedded_json_object() {
        let input = "Judge summary:\n```json\n{\"scores\":{\"codex\":7.5,\"claude\":8.0}}\n```";
        let scores = parse_scores_map(input).expect("scores");
        assert_eq!(scores.get("codex").copied(), Some(7.5));
        assert_eq!(scores.get("claude").copied(), Some(8.0));
    }

    #[test]
    fn aggregate_providers_counts_failures() {
        let runs = vec![
            BenchmarkRun {
                provider: "codex".to_string(),
                run: 1,
                duration_secs: 10,
                tokens: Some(100),
                cost_usd: Some(0.2),
                exit: 0,
                score: Some(8.0),
                output_text: String::new(),
            },
            BenchmarkRun {
                provider: "codex".to_string(),
                run: 2,
                duration_secs: 20,
                tokens: None,
                cost_usd: Some(0.4),
                exit: 1,
                score: Some(8.0),
                output_text: String::new(),
            },
        ];
        let rows = aggregate_providers(&["codex".to_string()], &runs);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].runs_total, 2);
        assert_eq!(rows[0].failed_runs, 1);
        assert_eq!(rows[0].avg_tokens, Some(100.0));
        let avg_cost = rows[0].avg_cost_usd.expect("avg cost");
        assert!((avg_cost - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn pick_recommendation_prefers_cost_per_score() {
        let rows = vec![
            ProviderAggregate {
                provider: "codex".to_string(),
                avg_duration_secs: 120.0,
                avg_tokens: Some(10_000.0),
                avg_cost_usd: Some(0.2),
                score: Some(8.0),
                runs_total: 1,
                failed_runs: 0,
            },
            ProviderAggregate {
                provider: "claude".to_string(),
                avg_duration_secs: 120.0,
                avg_tokens: Some(12_000.0),
                avg_cost_usd: Some(0.4),
                score: Some(8.0),
                runs_total: 1,
                failed_runs: 0,
            },
        ];

        let rec = pick_recommendation(&rows).expect("recommendation");
        assert_eq!(rec.provider, "codex");
    }
}
