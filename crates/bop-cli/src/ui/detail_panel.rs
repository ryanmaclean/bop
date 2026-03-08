use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use serde_json::{Map, Value};

use crate::render::CardView;
use crate::ui::app::{App, DetailTab};

#[derive(Debug, Default)]
struct DetailMeta {
    raw: Option<Value>,
    merge_ref: Option<String>,
    worktree: Option<String>,
    provider: Option<String>,
    stage: Option<String>,
    retries: Option<u32>,
    total_tokens: Option<u64>,
    total_cost_usd: Option<f64>,
}

pub fn render_detail_panel(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    card_id: &str,
    card: Option<&CardView>,
    card_dir: Option<&Path>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    frame.render_widget(Clear, area);

    let border = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    frame.render_widget(border.clone(), area);

    let inner = border.inner(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let detail_meta = read_detail_meta(card_dir);

    let info_height = if inner.height > 9 { 4 } else { 2 };
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(info_height),
        Constraint::Min(0),
    ])
    .split(inner);

    let summary = Paragraph::new(build_summary_line(card_id, card, &detail_meta));
    frame.render_widget(summary, chunks[0]);

    let tabs = Paragraph::new(build_tab_line(app.detail_tab));
    frame.render_widget(tabs, chunks[1]);

    let info = Paragraph::new(build_info_lines(card, &detail_meta, info_height as usize));
    frame.render_widget(info, chunks[2]);

    let content_block = Block::default()
        .title(build_content_title(app.detail_tab))
        .borders(Borders::TOP);
    let content_inner = content_block.inner(chunks[3]);
    let content_lines = build_content_lines(app, card, card_dir, &detail_meta);

    let max_top = content_lines
        .len()
        .saturating_sub(content_inner.height as usize);
    let top = if app.detail_tab == DetailTab::Log && app.detail_log_follow {
        max_top
    } else {
        app.detail_scroll.min(max_top)
    };

    let content = Paragraph::new(content_lines)
        .block(content_block)
        .wrap(Wrap { trim: false })
        .scroll((to_u16(top), 0));
    frame.render_widget(content, chunks[3]);
}

fn to_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

fn build_summary_line(card_id: &str, card: Option<&CardView>, meta: &DetailMeta) -> Line<'static> {
    let state = card
        .map(|c| c.state.as_str())
        .unwrap_or("unknown")
        .to_string();
    let provider = card
        .and_then(|c| c.provider.as_deref())
        .map(str::to_owned)
        .or_else(|| meta.provider.clone())
        .unwrap_or_else(|| "-".to_string());
    let elapsed = card
        .and_then(|c| c.elapsed_s)
        .map(format_duration)
        .unwrap_or_else(|| "-".to_string());
    let cost = meta
        .total_cost_usd
        .map(|c| format!("${:.2}", c))
        .unwrap_or_else(|| "-".to_string());

    Line::from(vec![
        Span::styled(
            card_id.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  •  "),
        Span::styled(
            state,
            Style::default().fg(state_color(card.map(|c| c.state.as_str()))),
        ),
        Span::raw("  •  "),
        Span::styled(provider, Style::default().fg(Color::Cyan)),
        Span::raw("  •  "),
        Span::raw(elapsed),
        Span::raw("  •  "),
        Span::styled(cost, Style::default().fg(Color::Green)),
    ])
}

fn build_tab_line(active: DetailTab) -> Line<'static> {
    let mut spans = vec![
        tab_span('M', "eta", active == DetailTab::Meta),
        Span::raw("  "),
        tab_span('D', "iff", active == DetailTab::Diff),
        Span::raw("  "),
        tab_span('R', "eplay", active == DetailTab::Replay),
        Span::raw("  "),
        tab_span('O', "utput", active == DetailTab::Output),
        Span::raw("  "),
        tab_span('L', "og", active == DetailTab::Log),
    ];
    spans.push(Span::raw("   "));
    spans.push(Span::styled("Esc/Enter", Style::default().fg(Color::Cyan)));
    spans.push(Span::raw(" close"));
    Line::from(spans)
}

fn tab_span(key: char, label: &str, active: bool) -> Span<'static> {
    let style = if active {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Cyan)
    };
    Span::styled(format!("[{key}]{label}"), style)
}

fn build_info_lines(
    card: Option<&CardView>,
    meta: &DetailMeta,
    max_lines: usize,
) -> Vec<Line<'static>> {
    let glyph = card
        .and_then(|c| c.glyph.clone())
        .unwrap_or_else(|| "-".to_string());
    let token = card
        .and_then(|c| c.token.clone())
        .unwrap_or_else(|| "-".to_string());
    let provider = card
        .and_then(|c| c.provider.as_deref())
        .map(str::to_owned)
        .or_else(|| meta.provider.clone())
        .unwrap_or_else(|| "-".to_string());
    let cost = meta
        .total_cost_usd
        .map(|c| format!("${:.2}", c))
        .unwrap_or_else(|| "-".to_string());

    let tokens = meta
        .total_tokens
        .map(format_with_commas)
        .unwrap_or_else(|| "-".to_string());
    let retries = meta
        .retries
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string());
    let stage = card
        .map(|c| c.stage.clone())
        .or_else(|| meta.stage.clone())
        .unwrap_or_else(|| "-".to_string());
    let worktree = meta.worktree.clone().unwrap_or_else(|| "-".to_string());
    let merge_ref = meta.merge_ref.clone().unwrap_or_else(|| "-".to_string());

    let mut lines = vec![
        Line::from(format!(
            "glyph: {glyph}   token: {token}   provider: {provider}   cost: {cost}"
        )),
        Line::from(format!(
            "tokens: {tokens}   retries: {retries}   stage: {stage}"
        )),
        Line::from(format!("worktree: {worktree}")),
        Line::from(format!("merge_commit: {merge_ref}")),
    ];

    lines.truncate(max_lines);
    lines
}

fn build_content_title(tab: DetailTab) -> &'static str {
    match tab {
        DetailTab::Meta => " Meta ",
        DetailTab::Diff => " Diff ",
        DetailTab::Replay => " Replay ",
        DetailTab::Output => " Output ",
        DetailTab::Log => " Log ",
    }
}

fn build_content_lines(
    app: &App,
    card: Option<&CardView>,
    card_dir: Option<&Path>,
    meta: &DetailMeta,
) -> Vec<Line<'static>> {
    match app.detail_tab {
        DetailTab::Meta => build_meta_lines(meta.raw.as_ref()),
        DetailTab::Diff => build_diff_lines(app, card, meta),
        DetailTab::Replay => build_replay_lines(card, card_dir),
        DetailTab::Output => build_output_lines(card_dir),
        DetailTab::Log => build_log_lines(card_dir),
    }
}

fn build_meta_lines(raw_meta: Option<&Value>) -> Vec<Line<'static>> {
    let Some(Value::Object(map)) = raw_meta else {
        return unavailable_lines("meta not yet available");
    };

    if map.is_empty() {
        return unavailable_lines("meta is empty");
    }

    let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
    keys.sort_unstable();

    let key_width = keys.iter().map(|k| k.len()).max().unwrap_or(0).min(24);
    keys.into_iter()
        .map(|key| {
            let value = map
                .get(key)
                .map(format_meta_value)
                .unwrap_or_else(|| "-".to_string());
            Line::from(vec![
                Span::styled(
                    format!("{key:width$}", width = key_width),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw("  "),
                Span::raw(value),
            ])
        })
        .collect()
}

fn format_meta_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v.clone(),
        _ => {
            let compact = serde_json::to_string(value).unwrap_or_else(|_| "<invalid json>".into());
            truncate_with_ellipsis(&compact, 120)
        }
    }
}

fn build_diff_lines(app: &App, card: Option<&CardView>, meta: &DetailMeta) -> Vec<Line<'static>> {
    if card.is_some_and(|c| c.state == "pending") {
        return unavailable_lines("not yet available");
    }

    let Some(merge_ref) = meta.merge_ref.as_deref() else {
        return unavailable_lines("not yet available");
    };

    let git_cwd = app.cards_root.parent().unwrap_or(app.cards_root.as_path());
    let range = format!("{merge_ref}^..{merge_ref}");
    let output = Command::new("git")
        .args(["diff", "--no-ext-diff", "--no-color", &range])
        .current_dir(git_cwd)
        .output();

    let output = match output {
        Ok(output) => output,
        Err(err) => {
            return vec![Line::from(Span::styled(
                format!("unable to run git diff: {err}"),
                Style::default().fg(Color::Red),
            ))]
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return vec![Line::from(Span::styled(
            if stderr.is_empty() {
                "git diff failed".to_string()
            } else {
                format!("git diff failed: {stderr}")
            },
            Style::default().fg(Color::Red),
        ))];
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return vec![Line::from("(no diff output)")];
    }

    stdout.lines().map(stylize_diff_line).collect()
}

fn stylize_diff_line(line: &str) -> Line<'static> {
    let style = if line.starts_with('+') && !line.starts_with("+++") {
        Style::default().fg(Color::Green)
    } else if line.starts_with('-') && !line.starts_with("---") {
        Style::default().fg(Color::Red)
    } else if line.starts_with("@@") {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if line.starts_with("diff --git")
        || line.starts_with("index ")
        || line.starts_with("---")
        || line.starts_with("+++")
    {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    Line::from(Span::styled(line.to_string(), style))
}

fn build_replay_lines(card: Option<&CardView>, card_dir: Option<&Path>) -> Vec<Line<'static>> {
    if card.is_some_and(|c| c.state == "pending") {
        return unavailable_lines("not yet available");
    }

    let Some(card_dir) = card_dir else {
        return unavailable_lines("not yet available");
    };

    let events_path = card_dir.join("logs").join("events.jsonl");
    let content = match fs::read_to_string(&events_path) {
        Ok(content) => content,
        Err(_) => return unavailable_lines("not yet available"),
    };

    let mut lines = vec![Line::from(vec![Span::styled(
        format!(
            "{:<20}  {:<14}  {:<10}  {}",
            "timestamp", "event", "state", "details"
        ),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )])];

    let mut row_count = 0usize;
    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Ok(mut obj) = serde_json::from_str::<Map<String, Value>>(trimmed) else {
            continue;
        };

        let ts = pop_string(&mut obj, "ts")
            .or_else(|| pop_string(&mut obj, "timestamp"))
            .unwrap_or_else(|| "-".to_string());
        let event = pop_string(&mut obj, "event").unwrap_or_else(|| "-".to_string());
        let from = pop_string(&mut obj, "from");
        let to = pop_string(&mut obj, "to");
        let state = pop_string(&mut obj, "state")
            .or_else(|| to.clone())
            .or_else(|| from.clone())
            .unwrap_or_else(|| "-".to_string());

        let details = build_replay_details(&mut obj);

        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<20}  ", short_timestamp(&ts)),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(format!("{:<14}  ", event), Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("{:<10}  ", state),
                Style::default().fg(state_color(Some(state.as_str()))),
            ),
            Span::raw(details),
        ]));

        row_count = row_count.saturating_add(1);
    }

    if row_count == 0 {
        return unavailable_lines("not yet available");
    }

    lines
}

fn short_timestamp(ts: &str) -> String {
    if ts.len() >= 20 {
        ts[..20].to_string()
    } else {
        ts.to_string()
    }
}

fn build_replay_details(obj: &mut Map<String, Value>) -> String {
    let mut details = Vec::new();

    if let Some(stage) = pop_string(obj, "stage") {
        details.push(format!("stage={stage}"));
    }
    if let Some(provider) = pop_string(obj, "provider") {
        details.push(format!("provider={provider}"));
    }
    if let Some(pid) = pop_u64(obj, "pid") {
        details.push(format!("pid={pid}"));
    }
    if let Some(exit_code) = pop_i64(obj, "exit_code").or_else(|| pop_i64(obj, "exit")) {
        details.push(format!("exit={exit_code}"));
    }
    if let Some(cooldown) = pop_u64(obj, "cooldown_s")
        .or_else(|| pop_u64(obj, "cooldown"))
        .or_else(|| pop_u64(obj, "cooldown_seconds"))
    {
        details.push(format!("cooldown={cooldown}s"));
    }
    if let Some(rotated) = pop_string(obj, "rotated_to").or_else(|| pop_string(obj, "rotated")) {
        details.push(format!("rotated->{rotated}"));
    }

    let tokens = pop_u64(obj, "tokens")
        .or_else(|| pop_u64(obj, "tokens_used"))
        .or_else(|| {
            let prompt = pop_u64(obj, "prompt_tokens").unwrap_or(0);
            let completion = pop_u64(obj, "completion_tokens").unwrap_or(0);
            if prompt > 0 || completion > 0 {
                Some(prompt.saturating_add(completion))
            } else {
                None
            }
        });

    if let Some(tokens) = tokens {
        details.push(format!("tokens={}", format_with_commas(tokens)));
    }

    if let Some(cost) = pop_f64(obj, "cost_usd").or_else(|| pop_f64(obj, "cost")) {
        details.push(format!("cost=${:.2}", cost));
    }

    details.join("  ")
}

fn build_output_lines(card_dir: Option<&Path>) -> Vec<Line<'static>> {
    let Some(card_dir) = card_dir else {
        return unavailable_lines("not yet available");
    };

    let result_path = card_dir.join("output").join("result.md");
    let content = match fs::read_to_string(&result_path) {
        Ok(content) => content,
        Err(_) => return unavailable_lines("not yet available"),
    };

    let mut lines = Vec::new();
    let mut in_code_block = false;

    for line in content.lines() {
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Gray).bg(Color::DarkGray),
            )));
            continue;
        }

        if trimmed.starts_with('#') {
            let header = trimmed.trim_start_matches('#').trim_start();
            lines.push(Line::from(Span::styled(
                header.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            )));
            continue;
        }

        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            let indent = " ".repeat(line.len().saturating_sub(trimmed.len()));
            lines.push(Line::from(format!("{indent}• {rest}")));
            continue;
        }

        lines.push(Line::from(line.to_string()));
    }

    if lines.is_empty() {
        return vec![Line::from("(empty output)")];
    }

    lines
}

fn build_log_lines(card_dir: Option<&Path>) -> Vec<Line<'static>> {
    let Some(card_dir) = card_dir else {
        return unavailable_lines("not yet available");
    };

    let stdout_path = card_dir.join("logs").join("stdout.log");
    if !stdout_path.exists() {
        return unavailable_lines("not yet available");
    }

    let content = match fs::read_to_string(stdout_path) {
        Ok(content) => content,
        Err(_) => return vec![Line::from("(unable to read stdout.log)")],
    };

    let all_lines: Vec<&str> = content.lines().collect();
    let start = all_lines.len().saturating_sub(200);
    all_lines[start..]
        .iter()
        .map(|line| stylize_log_line(line))
        .collect()
}

fn stylize_log_line(line: &str) -> Line<'static> {
    let style = if line.contains("ERROR") || line.contains("error:") || line.contains("FAILED") {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else if line.contains("WARN") || line.contains("warning:") {
        Style::default().fg(Color::Yellow)
    } else if line.contains("INFO") {
        Style::default().fg(Color::Cyan)
    } else if line.contains("DEBUG") || line.contains("TRACE") {
        Style::default().fg(Color::DarkGray)
    } else if line.contains("→ done") || line.contains("-> done") {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else if line.contains("→ failed") || line.contains("-> failed") {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else if line.contains("→ merged") || line.contains("-> merged") {
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    Line::from(Span::styled(line.to_string(), style))
}

fn unavailable_lines(message: &str) -> Vec<Line<'static>> {
    vec![Line::from(Span::styled(
        message.to_string(),
        Style::default().fg(Color::DarkGray),
    ))]
}

fn read_detail_meta(card_dir: Option<&Path>) -> DetailMeta {
    let Some(card_dir) = card_dir else {
        return DetailMeta::default();
    };

    let raw = fs::read_to_string(card_dir.join("meta.json"))
        .ok()
        .and_then(|json| serde_json::from_str::<Value>(&json).ok());

    let typed = bop_core::read_meta(card_dir).ok();

    let merge_ref = raw
        .as_ref()
        .and_then(|v| v.get("merge_commit"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            typed
                .as_ref()
                .and_then(|m| m.change_ref.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        });

    let worktree = typed
        .as_ref()
        .and_then(|m| m.workspace_path.as_ref())
        .map(PathBuf::from)
        .or_else(|| {
            let default = card_dir.join("worktree");
            default.is_dir().then_some(default)
        })
        .map(|p| display_path(&p));

    let provider = typed
        .as_ref()
        .and_then(|m| m.runs.last())
        .map(|r| r.provider.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    let stage = typed
        .as_ref()
        .map(|m| m.stage.trim().to_string())
        .filter(|s| !s.is_empty());

    let retries = typed.as_ref().and_then(|m| m.retry_count);

    let total_tokens = typed.as_ref().and_then(|m| {
        let total = m
            .runs
            .iter()
            .map(|run| {
                run.prompt_tokens
                    .unwrap_or(0)
                    .saturating_add(run.completion_tokens.unwrap_or(0))
            })
            .sum::<u64>();
        (total > 0).then_some(total)
    });

    let total_cost_usd = typed.as_ref().and_then(|m| {
        let total = m.runs.iter().filter_map(|run| run.cost_usd).sum::<f64>();
        (total > 0.0).then_some(total)
    });

    DetailMeta {
        raw,
        merge_ref,
        worktree,
        provider,
        stage,
        retries,
        total_tokens,
        total_cost_usd,
    }
}

fn display_path(path: &Path) -> String {
    let cwd = std::env::current_dir().ok();
    if let Some(cwd) = cwd {
        if let Ok(rel) = path.strip_prefix(&cwd) {
            return rel.display().to_string();
        }
    }
    path.display().to_string()
}

fn state_color(state: Option<&str>) -> Color {
    match state.unwrap_or_default() {
        "pending" => Color::Blue,
        "running" => Color::Yellow,
        "done" => Color::Green,
        "failed" => Color::Red,
        "merged" => Color::Magenta,
        _ => Color::Gray,
    }
}

fn format_duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

fn format_with_commas(value: u64) -> String {
    let mut out = String::new();
    for (idx, ch) in value.to_string().chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn truncate_with_ellipsis(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }

    let mut out = s.chars().take(max.saturating_sub(1)).collect::<String>();
    out.push('…');
    out
}

fn pop_string(map: &mut Map<String, Value>, key: &str) -> Option<String> {
    map.remove(key).and_then(|v| v.as_str().map(str::to_owned))
}

fn pop_u64(map: &mut Map<String, Value>, key: &str) -> Option<u64> {
    let val = map.remove(key)?;
    match val {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn pop_i64(map: &mut Map<String, Value>, key: &str) -> Option<i64> {
    let val = map.remove(key)?;
    match val {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse::<i64>().ok(),
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
