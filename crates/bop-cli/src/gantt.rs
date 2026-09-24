use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::Path;

use crate::colors::{state_ansi, state_ansi_bg, state_color, BOLD, DIM, RESET};
use crate::util::term_width;

struct Bar {
    id: String,
    glyph: String,
    state: String,
    stage: String,
    provider: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    dur_s: f64,
    tokens: Option<u64>,
    cost: Option<f64>,
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    // Try standard ISO, then strip Z and retry
    if let Ok(dt) = s.parse::<DateTime<Utc>>() {
        return Some(dt);
    }
    // chrono's parse with flexible format
    chrono::NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|n| n.and_utc())
}

fn collect_bars(root: &Path) -> Vec<Bar> {
    let states = ["running", "done", "merged", "failed", "pending"];
    let mut bars = Vec::new();

    for state in &states {
        let dir = root.join(state);
        if !dir.exists() {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("bop") {
                continue;
            }
            let Ok(meta) = bop_core::read_meta(&path) else {
                continue;
            };
            let Some(last_run) = meta.runs.last() else {
                continue;
            };
            let Some(start) = parse_ts(&last_run.started_at) else {
                continue;
            };

            let end = last_run
                .ended_at
                .as_deref()
                .and_then(parse_ts)
                .unwrap_or_else(|| {
                    // Cap open-ended: use duration_s or 10 min
                    let dur = last_run.duration_s.unwrap_or(0) as i64;
                    if dur > 0 {
                        start + Duration::seconds(dur)
                    } else {
                        start + Duration::minutes(10)
                    }
                });

            let dur_s = (end - start).num_milliseconds() as f64 / 1000.0;
            if dur_s < 2.0 {
                continue;
            }

            let tokens = match (last_run.prompt_tokens, last_run.completion_tokens) {
                (Some(p), Some(c)) => Some(p + c),
                (Some(p), None) => Some(p),
                (None, Some(c)) => Some(c),
                _ => None,
            };

            bars.push(Bar {
                id: meta.id.clone(),
                glyph: meta.glyph.or(meta.token).unwrap_or_default(),
                state: state.to_string(),
                stage: meta.stage.clone(),
                provider: last_run.provider.clone(),
                start,
                end,
                dur_s,
                tokens,
                cost: last_run.cost_usd,
            });
        }
    }

    bars.sort_by_key(|b| b.start);
    bars
}

/// Cluster bars: gap > 30 min = new cluster.
fn cluster(bars: &[Bar]) -> Vec<Vec<usize>> {
    if bars.is_empty() {
        return Vec::new();
    }
    let mut clusters: Vec<Vec<usize>> = vec![vec![0]];
    for i in 1..bars.len() {
        let prev_end = bars[clusters.last().unwrap().iter().copied().fold(0, |best, j| {
            if bars[j].end > bars[best].end {
                j
            } else {
                best
            }
        })]
        .end;
        if (bars[i].start - prev_end).num_seconds() > 1800 {
            clusters.push(vec![i]);
        } else {
            clusters.last_mut().unwrap().push(i);
        }
    }
    clusters
}

fn dur_label(s: f64) -> String {
    if s >= 60.0 {
        format!("{:.1}m", s / 60.0)
    } else {
        format!("{:.0}s", s)
    }
}

fn dur_label_precise(s: f64) -> String {
    let total = s.max(0.0).round() as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let sec = total % 60;
    if h > 0 {
        format!("{h}h {m}m {sec}s")
    } else if m > 0 {
        format!("{m}m {sec}s")
    } else {
        format!("{sec}s")
    }
}

fn format_u64_commas(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + (s.len() / 3));
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn escape_html_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn percentile_threshold(durations: &[f64], p: f64) -> f64 {
    if durations.is_empty() {
        return 0.0;
    }
    let mut sorted = durations.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let idx = ((sorted.len().saturating_sub(1)) as f64 * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn peak_parallelism(bars: &[Bar]) -> usize {
    let mut events: Vec<(DateTime<Utc>, i32)> = Vec::with_capacity(bars.len() * 2);
    for b in bars {
        events.push((b.start, 1));
        events.push((b.end, -1));
    }
    events.sort_by(|(ta, da), (tb, db)| ta.cmp(tb).then_with(|| da.cmp(db)));

    let mut cur = 0_i32;
    let mut peak = 0_i32;
    for (_, delta) in events {
        cur += delta;
        if cur > peak {
            peak = cur;
        }
    }
    peak.max(0) as usize
}

// ── ANSI terminal rendering ─────────────────────────────────────────────────

fn render_ansi(bars: &[Bar], term_width: usize) {
    let clusters = cluster(bars);
    let label_width = 24_usize;
    let dur_width = 7_usize;
    // bar_cols = total width - label - " │ " (3) - " " (1) - dur_label - " " (1)
    let bar_cols = term_width
        .saturating_sub(label_width + 3 + dur_width + 2)
        .max(20);

    // Header
    println!(
        "{}bop · agent timeline{} {} — {} runs{}",
        BOLD,
        RESET,
        DIM,
        bars.len(),
        RESET
    );
    println!();

    for indices in &clusters {
        let c_min = indices.iter().map(|&i| bars[i].start).min().unwrap();
        let c_max = indices.iter().map(|&i| bars[i].end).max().unwrap();
        let c_span = (c_max - c_min).num_seconds().max(1) as f64;

        // Cluster header
        println!(
            "{}{} — {} · {} runs{}",
            DIM,
            c_min.format("%b %d, %H:%M"),
            c_max.format("%H:%M"),
            indices.len(),
            RESET,
        );

        // Time axis
        let mut axis = String::with_capacity(label_width + 3 + bar_cols);
        for _ in 0..label_width {
            axis.push(' ');
        }
        axis.push_str(" │ ");

        // Place time labels on the axis
        let mut axis_chars: Vec<char> = vec![' '; bar_cols];
        // We'll place a few evenly-spaced time markers
        let n_marks = (bar_cols / 15).clamp(2, 8);
        for m in 0..n_marks {
            let frac = m as f64 / (n_marks - 1).max(1) as f64;
            let col = (frac * (bar_cols - 5) as f64) as usize;
            let t = c_min + Duration::milliseconds((frac * c_span * 1000.0) as i64);
            let label = t.format("%H:%M").to_string();
            for (j, ch) in label.chars().enumerate() {
                if col + j < bar_cols {
                    axis_chars[col + j] = ch;
                }
            }
        }
        let axis_str: String = axis_chars.into_iter().collect();
        println!("{}{}{}{}", axis, DIM, axis_str, RESET);

        // Separator
        let mut sep = String::with_capacity(label_width + 3 + bar_cols);
        for _ in 0..label_width {
            sep.push(' ');
        }
        sep.push_str(" ├─");
        for _ in 0..bar_cols {
            sep.push('─');
        }
        println!("{}{}{}", DIM, sep, RESET);

        // Bars
        for &i in indices {
            let b = &bars[i];

            // Label: glyph + truncated id
            let mut label = String::with_capacity(label_width);
            if !b.glyph.is_empty() {
                label.push_str(&b.glyph);
                label.push(' ');
            }
            let id_budget = label_width.saturating_sub(label.chars().count());
            let truncated: String = b.id.chars().take(id_budget).collect();
            label.push_str(&truncated);
            // Pad to label_width (by char count)
            let pad = label_width.saturating_sub(label.chars().count());
            for _ in 0..pad {
                label.push(' ');
            }

            // Bar position
            let start_frac = (b.start - c_min).num_milliseconds() as f64 / (c_span * 1000.0);
            let end_frac = (b.end - c_min).num_milliseconds() as f64 / (c_span * 1000.0);
            let col_start = (start_frac * bar_cols as f64).round() as usize;
            let col_end = ((end_frac * bar_cols as f64).round() as usize).max(col_start + 1);
            let col_end = col_end.min(bar_cols);

            let color = state_ansi(b.state.as_str());
            let bg = state_ansi_bg(b.state.as_str());

            // Build the bar line
            let mut bar_line = String::with_capacity(bar_cols + 32);
            // Leading space
            for _ in 0..col_start {
                bar_line.push(' ');
            }
            // Filled block using ▓ with background color
            bar_line.push_str(bg);
            bar_line.push_str(color);
            for _ in col_start..col_end {
                bar_line.push('▓');
            }
            bar_line.push_str(RESET);
            // Trailing space
            let used = col_end;
            for _ in used..bar_cols {
                bar_line.push(' ');
            }

            let dl = dur_label(b.dur_s);

            println!(
                "{}{}{} │ {} {}{:>width$}{}",
                color,
                label,
                RESET,
                bar_line,
                DIM,
                dl,
                RESET,
                width = dur_width,
            );
        }
        println!();
    }

    // Stats footer
    let total_dur: f64 = bars.iter().map(|b| b.dur_s).sum();
    let total_tok: u64 = bars.iter().filter_map(|b| b.tokens).sum();
    let total_cost: f64 = bars.iter().filter_map(|b| b.cost).sum();
    let mut stats = format!("{} runs · {:.0} min compute", bars.len(), total_dur / 60.0);
    if total_tok > 0 {
        let _ = write!(stats, " · {:.1}k tokens", total_tok as f64 / 1000.0);
    }
    if total_cost > 0.0 {
        let _ = write!(stats, " · ${:.2}", total_cost);
    }
    println!("{}{}{}", DIM, stats, RESET);

    // Legend
    let legend_items: Vec<String> = [
        ("pending", "●"),
        ("running", "●"),
        ("done", "●"),
        ("failed", "●"),
        ("merged", "●"),
    ]
    .iter()
    .map(|(st, dot)| format!("{}{}{} {}", state_ansi(st), dot, RESET, st))
    .collect();
    println!("{}", legend_items.join("  "));
}

fn generate_html(bars: &[Bar]) -> String {
    let clusters = cluster(bars);
    let total_bars = bars.len();
    let completed = bars
        .iter()
        .filter(|b| matches!(b.state.as_str(), "done" | "merged"))
        .count();
    let failed = bars.iter().filter(|b| b.state == "failed").count();
    let total_dur: f64 = bars.iter().map(|b| b.dur_s).sum();
    let avg_dur = if total_bars > 0 {
        total_dur / total_bars as f64
    } else {
        0.0
    };
    let total_cost: f64 = bars.iter().filter_map(|b| b.cost).sum();
    let peak = peak_parallelism(bars);

    let mut html = String::with_capacity(8192);
    let _ = write!(
        html,
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>bop · agent timeline</title>
<style>
* {{ margin:0; padding:0; box-sizing:border-box }}
body {{
  background:#0D0D0D; color:#B0B0B0;
  font-family:'SF Mono','JetBrains Mono','Menlo',monospace;
  font-size:12px; padding:24px 32px;
}}
h1 {{ font-size:13px; font-weight:500; letter-spacing:.08em; text-transform:uppercase; color:#666; margin-bottom:6px }}
h1 span {{ color:#999; font-weight:400 }}
.cluster {{ margin-bottom:28px }}
.cluster-hdr {{ font-size:10px; color:#444; text-transform:uppercase; letter-spacing:.1em; margin-bottom:8px; padding-left:2px }}
.chart-wrap {{ max-width:100%; overflow-x:auto; }}
.chart {{ display:flex; min-width:0 }}
.labels {{ flex-shrink:0; width:190px; padding-top:26px }}
.label {{ height:24px; display:flex; align-items:center; font-size:11px; color:#666; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; margin-bottom:1px }}
.label .g {{ margin-right:4px }}
.bars-area {{ flex:1; min-width:0; overflow:visible; position:relative }}
.axis {{ height:22px; border-bottom:1px solid #1F1F1F; position:relative; margin-bottom:2px }}
.axis-label {{ position:absolute; top:2px; font-size:10px; color:#444; transform:translateX(-50%); white-space:nowrap }}
.grid {{ position:absolute; top:24px; bottom:0; width:1px; background:#151515; pointer-events:none }}
.row {{ height:24px; position:relative; margin-bottom:1px }}
.bar {{ position:absolute; height:16px; top:4px; border-radius:3px; min-width:4px; cursor:default; opacity:.85 }}
.bar:hover {{ opacity:1; filter:brightness(1.3) }}
.legend {{ display:flex; gap:16px; margin-top:16px; padding-top:10px; border-top:1px solid #1A1A1A; flex-wrap:wrap }}
.legend-item {{ display:flex; align-items:center; gap:5px; font-size:10px; color:#555; text-transform:uppercase; letter-spacing:.05em }}
.sw {{ width:10px; height:10px; border-radius:2px }}
.summary {{ margin-top:14px; width:100%; max-width:640px; border-collapse:collapse; font-size:11px; }}
.summary th, .summary td {{ border:1px solid #1F1F1F; padding:6px 8px; text-align:left; }}
.summary th {{ color:#777; font-weight:500; width:190px; }}
.summary td {{ color:#B0B0B0; }}
</style>
</head>
<body>
<h1>bop · agent timeline <span>— {total_bars} runs</span></h1>
"#,
    );

    for indices in &clusters {
        let c_min = indices.iter().map(|&i| bars[i].start).min().unwrap();
        let c_max = indices.iter().map(|&i| bars[i].end).max().unwrap();
        let c_span = (c_max - c_min).num_seconds().max(60) as f64;

        let pct = |dt: DateTime<Utc>| -> f64 {
            ((dt - c_min).num_milliseconds() as f64 / (c_span * 1000.0)) * 100.0
        };

        // Pick time axis interval
        let interval_s: i64 = if c_span < 1800.0 {
            120
        } else if c_span < 3600.0 {
            300
        } else if c_span < 7200.0 {
            900
        } else {
            1800
        };

        // Header
        let _ = write!(
            html,
            "<div class=\"cluster\">\n<div class=\"cluster-hdr\">{} — {} · {} runs</div>\n",
            c_min.format("%b %d, %H:%M"),
            c_max.format("%H:%M"),
            indices.len()
        );
        html.push_str("<div class=\"chart-wrap\"><div class=\"chart\">\n<div class=\"labels\">\n");

        for &i in indices {
            let g = if bars[i].glyph.is_empty() {
                String::new()
            } else {
                format!("<span class=\"g\">{}</span>", bars[i].glyph)
            };
            let short_id: String = bars[i].id.chars().take(22).collect();
            let _ = writeln!(html, "  <div class=\"label\">{}{}</div>", g, short_id);
        }

        html.push_str("</div>\n<div class=\"bars-area\">\n<div class=\"axis\">\n");

        // Time marks
        let mut t = c_min + Duration::seconds(interval_s - (c_min.timestamp() % interval_s));
        while t < c_max {
            let p = pct(t);
            if p > 0.0 && p < 100.0 {
                let _ = writeln!(
                    html,
                    "  <span class=\"axis-label\" style=\"left:{:.1}%\">{}</span>",
                    p,
                    t.format("%H:%M")
                );
            }
            t += Duration::seconds(interval_s);
        }

        html.push_str("</div>\n");

        // Gridlines
        let mut t = c_min + Duration::seconds(interval_s - (c_min.timestamp() % interval_s));
        while t < c_max {
            let p = pct(t);
            if p > 0.0 && p < 100.0 {
                let _ = writeln!(html, "<div class=\"grid\" style=\"left:{:.1}%\"></div>", p);
            }
            t += Duration::seconds(interval_s);
        }

        // Bars
        let cluster_durations: Vec<f64> = indices.iter().map(|&i| bars[i].dur_s).collect();
        let p50 = percentile_threshold(&cluster_durations, 0.50);
        let p80 = percentile_threshold(&cluster_durations, 0.80);
        for &i in indices {
            let b = &bars[i];
            let lp = pct(b.start);
            let wp = (pct(b.end) - lp).max(0.5);
            let duration = dur_label_precise(b.dur_s);
            let tokens = b
                .tokens
                .map(format_u64_commas)
                .unwrap_or_else(|| "n/a".to_string());
            let cost = b
                .cost
                .map(|v| format!("${v:.2}"))
                .unwrap_or_else(|| "n/a".to_string());
            let tip = format!(
                "id: {}&#10;stage: {}&#10;provider: {}&#10;duration: {}&#10;tokens: {}&#10;cost: {}",
                escape_html_attr(&b.id),
                escape_html_attr(&b.stage),
                escape_html_attr(&b.provider),
                duration,
                tokens,
                cost
            );
            let heat_color = if b.dur_s <= p50 {
                "#4caf50"
            } else if b.dur_s <= p80 {
                "#ff9800"
            } else {
                "#f44336"
            };
            let _ = writeln!(
                html,
                "<div class=\"row\"><div class=\"bar\" style=\"left:{:.1}%;width:{:.1}%;background:{}\" title=\"{}\"></div></div>",
                lp, wp, heat_color, tip,
            );
        }

        html.push_str("</div>\n</div></div>\n</div>\n");
    }

    // Legend
    html.push_str("<div class=\"legend\">\n");
    for (label, col) in [
        ("fast (p0-p50)", "#4caf50"),
        ("medium (p50-p80)", "#ff9800"),
        ("slow (p80-p100)", "#f44336"),
    ] {
        let _ = writeln!(
            html,
            "  <div class=\"legend-item\"><div class=\"sw\" style=\"background:{}\"></div>{}</div>",
            col, label
        );
    }
    for st in ["pending", "running", "done", "failed", "merged"] {
        let _ = writeln!(
            html,
            "  <div class=\"legend-item\"><div class=\"sw\" style=\"background:{}\"></div>{}</div>",
            state_color(st),
            st
        );
    }
    html.push_str("</div>\n");

    let _ = write!(
        html,
        "<table class=\"summary\">
<tr><th>Metric</th><th>Value</th></tr>
<tr><td>Total cards</td><td>{}</td></tr>
<tr><td>Completed</td><td>{}</td></tr>
<tr><td>Failed</td><td>{}</td></tr>
<tr><td>Avg duration</td><td>{}</td></tr>
<tr><td>Total cost</td><td>${:.2}</td></tr>
<tr><td>Parallelism peak</td><td>{} concurrent</td></tr>
</table>\n</body></html>\n",
        total_bars,
        completed,
        failed,
        dur_label_precise(avg_dur),
        total_cost,
        peak
    );

    html
}

pub fn cmd_gantt(root: &Path, html: bool, open: bool, width_override: Option<usize>) -> Result<()> {
    let bars = collect_bars(root);
    if bars.is_empty() {
        println!("No card runs with duration data found.");
        return Ok(());
    }

    if html {
        let content = generate_html(&bars);
        let out_path = root.join("bop-gantt.html");
        fs::write(&out_path, &content)
            .with_context(|| format!("failed to write {}", out_path.display()))?;
        println!("Generated {} ({} runs)", out_path.display(), bars.len());
        if open {
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("open").arg(&out_path).spawn();
            }
        }
    } else {
        // Use explicit width, or auto-detect with 2-col padding per side
        let raw_width = width_override.unwrap_or_else(term_width);
        let padded = raw_width.saturating_sub(4); // 2 chars padding each side
        render_ansi(&bars, padded);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dur_label_formats() {
        assert_eq!(dur_label(30.0), "30s");
        assert_eq!(dur_label(90.0), "1.5m");
        assert_eq!(dur_label(300.0), "5.0m");
    }

    #[test]
    fn state_colors_all_mapped() {
        assert!(state_color("done").starts_with('#'));
        assert!(state_color("failed").starts_with('#'));
        assert!(state_color("merged").starts_with('#'));
        assert!(state_color("running").starts_with('#'));
        assert!(state_color("pending").starts_with('#'));
    }

    #[test]
    fn cluster_empty() {
        assert!(cluster(&[]).is_empty());
    }

    #[test]
    fn generate_html_produces_valid_structure() {
        use chrono::Duration;
        let now = Utc::now();
        let bars = vec![
            Bar {
                id: "test-1".into(),
                glyph: "♠".into(),
                state: "done".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: now - Duration::minutes(10),
                end: now - Duration::minutes(5),
                dur_s: 300.0,
                tokens: Some(5000),
                cost: Some(0.50),
            },
            Bar {
                id: "test-2".into(),
                glyph: "♥".into(),
                state: "running".into(),
                stage: "qa".into(),
                provider: "claude".into(),
                start: now - Duration::minutes(8),
                end: now - Duration::minutes(3),
                dur_s: 300.0,
                tokens: None,
                cost: None,
            },
        ];
        let html = generate_html(&bars);
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("2 runs"));
        assert!(html.contains("test-1"));
        assert!(html.contains("test-2"));
        assert!(html.contains("id: test-1"));
        assert!(html.contains("#4caf50") || html.contains("#ff9800") || html.contains("#f44336"));
        assert!(html.contains("Parallelism peak"));
    }

    #[test]
    fn escape_html_attr_handles_special_chars() {
        assert_eq!(escape_html_attr("foo & bar"), "foo &amp; bar");
        assert_eq!(escape_html_attr("<script>"), "&lt;script&gt;");
        assert_eq!(escape_html_attr("a\"b'c"), "a&quot;b&#39;c");
        assert_eq!(
            escape_html_attr("&<>\"'"),
            "&amp;&lt;&gt;&quot;&#39;"
        );
        assert_eq!(escape_html_attr("safe-text_123"), "safe-text_123");
    }

    #[test]
    fn format_u64_commas_adds_separators() {
        assert_eq!(format_u64_commas(0), "0");
        assert_eq!(format_u64_commas(999), "999");
        assert_eq!(format_u64_commas(1000), "1,000");
        assert_eq!(format_u64_commas(1234567), "1,234,567");
        assert_eq!(format_u64_commas(999999999), "999,999,999");
    }

    #[test]
    fn dur_label_precise_formats_correctly() {
        assert_eq!(dur_label_precise(0.0), "0s");
        assert_eq!(dur_label_precise(45.0), "45s");
        assert_eq!(dur_label_precise(90.0), "1m 30s");
        assert_eq!(dur_label_precise(3661.0), "1h 1m 1s");
        assert_eq!(dur_label_precise(7384.0), "2h 3m 4s");
        // Negative values should be clamped to 0
        assert_eq!(dur_label_precise(-10.0), "0s");
    }

    #[test]
    fn percentile_threshold_calculates_correctly() {
        let vals = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile_threshold(&vals, 0.0), 1.0);
        assert_eq!(percentile_threshold(&vals, 0.5), 3.0);
        assert_eq!(percentile_threshold(&vals, 1.0), 5.0);

        // Empty array
        assert_eq!(percentile_threshold(&[], 0.5), 0.0);

        // Single element
        assert_eq!(percentile_threshold(&[42.0], 0.5), 42.0);

        // Two elements
        let two = vec![10.0, 20.0];
        assert_eq!(percentile_threshold(&two, 0.0), 10.0);
        assert_eq!(percentile_threshold(&two, 1.0), 20.0);
    }

    #[test]
    fn peak_parallelism_detects_concurrent_tasks() {
        use chrono::Duration;
        let base = Utc::now();

        // Sequential tasks - peak should be 1
        let sequential = vec![
            Bar {
                id: "a".into(),
                glyph: "".into(),
                state: "done".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: base,
                end: base + Duration::minutes(5),
                dur_s: 300.0,
                tokens: None,
                cost: None,
            },
            Bar {
                id: "b".into(),
                glyph: "".into(),
                state: "done".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: base + Duration::minutes(6),
                end: base + Duration::minutes(10),
                dur_s: 240.0,
                tokens: None,
                cost: None,
            },
        ];
        assert_eq!(peak_parallelism(&sequential), 1);

        // Overlapping tasks - peak should be 2
        let overlapping = vec![
            Bar {
                id: "a".into(),
                glyph: "".into(),
                state: "done".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: base,
                end: base + Duration::minutes(10),
                dur_s: 600.0,
                tokens: None,
                cost: None,
            },
            Bar {
                id: "b".into(),
                glyph: "".into(),
                state: "done".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: base + Duration::minutes(5),
                end: base + Duration::minutes(15),
                dur_s: 600.0,
                tokens: None,
                cost: None,
            },
        ];
        assert_eq!(peak_parallelism(&overlapping), 2);

        // Three concurrent tasks
        let triple = vec![
            Bar {
                id: "a".into(),
                glyph: "".into(),
                state: "done".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: base,
                end: base + Duration::minutes(10),
                dur_s: 600.0,
                tokens: None,
                cost: None,
            },
            Bar {
                id: "b".into(),
                glyph: "".into(),
                state: "done".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: base + Duration::minutes(2),
                end: base + Duration::minutes(8),
                dur_s: 360.0,
                tokens: None,
                cost: None,
            },
            Bar {
                id: "c".into(),
                glyph: "".into(),
                state: "done".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: base + Duration::minutes(3),
                end: base + Duration::minutes(7),
                dur_s: 240.0,
                tokens: None,
                cost: None,
            },
        ];
        assert_eq!(peak_parallelism(&triple), 3);

        // Empty case
        assert_eq!(peak_parallelism(&[]), 0);
    }

    #[test]
    fn cluster_groups_by_time_gaps() {
        use chrono::Duration;
        let base = Utc::now();

        let bars = vec![
            Bar {
                id: "a".into(),
                glyph: "".into(),
                state: "done".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: base,
                end: base + Duration::minutes(5),
                dur_s: 300.0,
                tokens: None,
                cost: None,
            },
            Bar {
                id: "b".into(),
                glyph: "".into(),
                state: "done".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: base + Duration::minutes(10),
                end: base + Duration::minutes(15),
                dur_s: 300.0,
                tokens: None,
                cost: None,
            },
            // Gap of 35 minutes - should create new cluster
            Bar {
                id: "c".into(),
                glyph: "".into(),
                state: "done".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: base + Duration::minutes(50),
                end: base + Duration::minutes(55),
                dur_s: 300.0,
                tokens: None,
                cost: None,
            },
        ];

        let clusters = cluster(&bars);
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].len(), 2);
        assert_eq!(clusters[1].len(), 1);
        assert_eq!(clusters[0], vec![0, 1]);
        assert_eq!(clusters[1], vec![2]);
    }

    #[test]
    fn cluster_single_bar() {
        use chrono::Duration;
        let base = Utc::now();

        let bars = vec![Bar {
            id: "a".into(),
            glyph: "".into(),
            state: "done".into(),
            stage: "implement".into(),
            provider: "claude".into(),
            start: base,
            end: base + Duration::minutes(5),
            dur_s: 300.0,
            tokens: None,
            cost: None,
        }];

        let clusters = cluster(&bars);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0], vec![0]);
    }

    #[test]
    fn generate_html_empty_bars() {
        let html = generate_html(&[]);
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("0 runs"));
        assert!(html.contains("</html>"));
    }

    #[test]
    fn generate_html_includes_summary_table() {
        use chrono::Duration;
        let base = Utc::now();

        let bars = vec![
            Bar {
                id: "success".into(),
                glyph: "".into(),
                state: "done".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: base,
                end: base + Duration::minutes(5),
                dur_s: 300.0,
                tokens: Some(1000),
                cost: Some(0.10),
            },
            Bar {
                id: "failed-task".into(),
                glyph: "".into(),
                state: "failed".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: base + Duration::minutes(10),
                end: base + Duration::minutes(15),
                dur_s: 300.0,
                tokens: Some(2000),
                cost: Some(0.20),
            },
        ];

        let html = generate_html(&bars);
        assert!(html.contains("Total cards"));
        assert!(html.contains("Completed"));
        assert!(html.contains("Failed"));
        assert!(html.contains("Avg duration"));
        assert!(html.contains("Total cost"));
        assert!(html.contains("Parallelism peak"));
        // Check actual values
        assert!(html.contains("<td>2</td>")); // Total cards
        assert!(html.contains("<td>1</td>")); // Completed or Failed
        assert!(html.contains("$0.30")); // Total cost
    }

    #[test]
    fn generate_html_tooltips_escape_content() {
        use chrono::Duration;
        let base = Utc::now();

        let bars = vec![Bar {
            id: "<script>alert('xss')</script>".into(),
            glyph: "".into(),
            state: "done".into(),
            stage: "impl & test".into(),
            provider: "claude \"pro\"".into(),
            start: base,
            end: base + Duration::minutes(5),
            dur_s: 300.0,
            tokens: Some(1000),
            cost: Some(0.10),
        }];

        let html = generate_html(&bars);
        // Ensure special characters are escaped in tooltip attributes
        // The ID will appear in the label, but must be escaped in the tooltip
        assert!(html.contains("title="));
        assert!(html.contains("&lt;script&gt;")); // <script> escaped in tooltip
        assert!(html.contains("impl &amp; test")); // & escaped in stage
        assert!(html.contains("claude &quot;pro&quot;")); // " escaped in provider
    }

    #[test]
    fn generate_html_heat_colors_by_percentile() {
        use chrono::Duration;
        let base = Utc::now();

        // Create bars with varying durations to test heat coloring
        let bars = vec![
            Bar {
                id: "fast".into(),
                glyph: "".into(),
                state: "done".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: base,
                end: base + Duration::seconds(30),
                dur_s: 30.0,
                tokens: None,
                cost: None,
            },
            Bar {
                id: "medium".into(),
                glyph: "".into(),
                state: "done".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: base + Duration::minutes(1),
                end: base + Duration::minutes(3),
                dur_s: 120.0,
                tokens: None,
                cost: None,
            },
            Bar {
                id: "slow".into(),
                glyph: "".into(),
                state: "done".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                start: base + Duration::minutes(5),
                end: base + Duration::minutes(10),
                dur_s: 300.0,
                tokens: None,
                cost: None,
            },
        ];

        let html = generate_html(&bars);
        // Should contain heat colors
        assert!(html.contains("#4caf50")); // Fast (green)
        assert!(html.contains("#ff9800")); // Medium (orange)
        assert!(html.contains("#f44336")); // Slow (red)
    }

    #[test]
    fn parse_ts_handles_various_formats() {
        // Standard ISO 8601 with Z
        assert!(parse_ts("2024-01-15T10:30:45Z").is_some());

        // Without Z
        assert!(parse_ts("2024-01-15T10:30:45").is_some());

        // Invalid format
        assert!(parse_ts("not-a-date").is_none());
        assert!(parse_ts("").is_none());
    }
}
