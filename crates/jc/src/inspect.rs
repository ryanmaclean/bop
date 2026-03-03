use anyhow::Context;
use std::fs;
use std::path::Path;

use crate::paths;

pub fn parse_latest_json_line(path: &Path) -> Option<serde_json::Value> {
    let content = fs::read_to_string(path).ok()?;
    content
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line).ok())
}

fn fmt_tokens(n: u64) -> String {
    if n >= 10_000 {
        format!("{}k", n / 1000)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        format!("{}", n)
    }
}

pub fn cmd_inspect(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let state = card
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    println!("=== meta ({}) ===", state);
    let meta = jobcard_core::read_meta(&card)?;
    println!("{}", serde_json::to_string_pretty(&meta)?);

    let spec_path = card.join("spec.md");
    if spec_path.exists() {
        let spec = fs::read_to_string(&spec_path)?;
        println!("\n=== spec.md ===");
        print!("{}", spec);
        if !spec.ends_with('\n') && !spec.is_empty() {
            println!();
        }
    }

    for (label, filename) in [("stdout", "stdout.log"), ("stderr", "stderr.log")] {
        let log_path = card.join("logs").join(filename);
        if log_path.exists() {
            let content = fs::read_to_string(&log_path)?;
            let lines: Vec<&str> = content.lines().collect();
            let tail_lines = if lines.len() > 20 {
                &lines[lines.len() - 20..]
            } else {
                &lines[..]
            };
            println!("\n=== {} (last {} lines) ===", label, tail_lines.len());
            for line in tail_lines {
                println!("{}", line);
            }
        }
    }

    // Cost summary from stdout.log JSON result line
    let stdout_log = card.join("logs").join("stdout.log");
    if stdout_log.exists() {
        if let Some(v) = parse_latest_json_line(&stdout_log) {
            if let Some(usage) = v.get("usage") {
                let cache_read = usage
                    .get("cache_read_input_tokens")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);
                let cache_create = usage
                    .get("cache_creation_input_tokens")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);
                let output = usage
                    .get("output_tokens")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);
                let cost = v
                    .get("total_cost_usd")
                    .and_then(|x| x.as_f64())
                    .unwrap_or(0.0);
                let turns = v.get("num_turns").and_then(|x| x.as_u64()).unwrap_or(0);
                println!(
                    "\nCost  ${:.2}  |  cache_read {}  cache_create {}  output {}  |  {} turns",
                    cost,
                    fmt_tokens(cache_read),
                    fmt_tokens(cache_create),
                    fmt_tokens(output),
                    turns,
                );
            }
        }
    }

    println!("\n=== runs ({} attempts) ===", meta.runs.len());
    for (idx, run) in meta.runs.iter().enumerate() {
        let started = if run.started_at.len() >= 19 {
            run.started_at[..19].to_string()
        } else if run.started_at.trim().is_empty() {
            "<unknown>".to_string()
        } else {
            run.started_at.clone()
        };
        let provider_model = match (run.provider.trim(), run.model.trim()) {
            ("", "") => "unknown".to_string(),
            ("", m) => m.to_string(),
            (p, "") => p.to_string(),
            (p, m) => format!("{}/{}", p, m),
        };
        let stage = if run.stage.trim().is_empty() {
            "unknown"
        } else {
            run.stage.as_str()
        };
        let outcome = if run.outcome.trim().is_empty() {
            "unknown"
        } else {
            run.outcome.as_str()
        };
        let duration = run
            .duration_s
            .map(|d| format!("{}s", d))
            .unwrap_or_else(|| "\u{2014}".to_string());
        let cost = run
            .cost_usd
            .map(|c| format!("${:.2}", c))
            .unwrap_or_else(|| "\u{2014}".to_string());
        println!(
            "  #{:<2}  {:<20}  {:<22}  {:<12}  {:<8}  {:<6}  {}",
            idx + 1,
            started,
            provider_model,
            stage,
            outcome,
            duration,
            cost
        );
    }

    Ok(())
}
