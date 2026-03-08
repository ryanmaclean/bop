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
.chart {{ display:flex }}
.labels {{ flex-shrink:0; width:190px; padding-top:26px }}
.label {{ height:24px; display:flex; align-items:center; font-size:11px; color:#666; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; margin-bottom:1px }}
.label .g {{ margin-right:4px }}
.bars-area {{ flex:1; overflow:visible; position:relative }}
.axis {{ height:22px; border-bottom:1px solid #1F1F1F; position:relative; margin-bottom:2px }}
.axis-label {{ position:absolute; top:2px; font-size:10px; color:#444; transform:translateX(-50%); white-space:nowrap }}
.grid {{ position:absolute; top:24px; bottom:0; width:1px; background:#151515; pointer-events:none }}
.row {{ height:24px; position:relative; margin-bottom:1px }}
.bar {{ position:absolute; height:16px; top:4px; border-radius:3px; min-width:4px; cursor:default; opacity:.85 }}
.bar:hover {{ opacity:1; filter:brightness(1.3) }}
.dur {{ position:absolute; left:calc(100% + 5px); top:50%; transform:translateY(-50%); font-size:10px; color:#555; white-space:nowrap }}
.legend {{ display:flex; gap:16px; margin-top:16px; padding-top:10px; border-top:1px solid #1A1A1A }}
.legend-item {{ display:flex; align-items:center; gap:5px; font-size:10px; color:#555; text-transform:uppercase; letter-spacing:.05em }}
.sw {{ width:10px; height:10px; border-radius:2px }}
.stats {{ margin-top:10px; font-size:10px; color:#444 }}
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
        html.push_str("<div class=\"chart\">\n<div class=\"labels\">\n");

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
        for &i in indices {
            let b = &bars[i];
            let lp = pct(b.start);
            let wp = (pct(b.end) - lp).max(0.5);
            let dl = dur_label(b.dur_s);
            let tip = format!(
                "{}&#10;{} · {}&#10;{} · {}",
                b.id, b.state, b.stage, b.provider, dl
            );
            let _ = writeln!(
                html,
                "<div class=\"row\"><div class=\"bar\" style=\"left:{:.1}%;width:{:.1}%;background:{}\" title=\"{}\"><span class=\"dur\">{}</span></div></div>",
                lp, wp, state_color(&b.state), tip, dl,
            );
        }

        html.push_str("</div>\n</div>\n</div>\n");
    }

    // Legend
    html.push_str("<div class=\"legend\">\n");
    for (st, col) in [
        ("pending", "#3A5A8A"),
        ("running", "#B8690F"),
        ("done", "#1E8A45"),
        ("failed", "#C43030"),
        ("merged", "#6B3DB8"),
    ] {
        let _ = writeln!(
            html,
            "  <div class=\"legend-item\"><div class=\"sw\" style=\"background:{}\"></div>{}</div>",
            col, st
        );
    }
    html.push_str("</div>\n");

    // Stats
    let total_dur: f64 = bars.iter().map(|b| b.dur_s).sum();
    let total_tok: u64 = bars.iter().filter_map(|b| b.tokens).sum();
    let total_cost: f64 = bars.iter().filter_map(|b| b.cost).sum();
    let _ = write!(
        html,
        "<div class=\"stats\">{} runs · {:.0} min compute",
        total_bars,
        total_dur / 60.0
    );
    if total_tok > 0 {
        let _ = write!(html, " · {:.1}k tokens", total_tok as f64 / 1000.0);
    }
    if total_cost > 0.0 {
        let _ = write!(html, " · ${:.2}", total_cost);
    }
    html.push_str("</div>\n</body></html>\n");

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
        assert!(html.contains("#1E8A45")); // done color
        assert!(html.contains("#B8690F")); // running color
    }
}
