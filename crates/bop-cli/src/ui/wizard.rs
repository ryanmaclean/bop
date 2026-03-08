use anyhow::Context;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;
use std::fs;
use std::io;
use std::path::Path;

use super::event::TerminalGuard;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WizardResult {
    pub template: String,
    pub id: String,
    pub spec: String,
    pub provider_chain: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TemplateOption {
    name: String,
    description: &'static str,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ConfirmChoice {
    Yes,
    No,
    Edit,
}

type WizardTerminal = Terminal<CrosstermBackend<io::Stdout>>;

pub fn run_new_card_wizard(
    cards_dir: &Path,
    default_provider_chain: &[String],
) -> anyhow::Result<WizardResult> {
    let templates = discover_templates(cards_dir)?;
    let mut selected_template = templates
        .iter()
        .position(|t| t.name == "implement")
        .unwrap_or(0);
    let mut id = String::new();
    let mut spec = String::new();
    let mut provider_chain = default_provider_chain.to_vec();

    let _guard = TerminalGuard::new()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    loop {
        selected_template = template_selection_step(&mut terminal, &templates, selected_template)?;
        id = card_id_step(&mut terminal, &id)?;
        spec = spec_step(&mut terminal, &spec)?;
        provider_chain =
            provider_chain_step(&mut terminal, default_provider_chain, &provider_chain)?;

        match confirmation_step(
            &mut terminal,
            &templates[selected_template].name,
            &id,
            &provider_chain,
            &spec,
        )? {
            ConfirmChoice::Yes => {
                terminal.clear()?;
                return Ok(WizardResult {
                    template: templates[selected_template].name.clone(),
                    id: id.trim().to_string(),
                    spec,
                    provider_chain,
                });
            }
            ConfirmChoice::No => anyhow::bail!("card creation cancelled"),
            ConfirmChoice::Edit => {}
        }
    }
}

fn template_selection_step(
    terminal: &mut WizardTerminal,
    templates: &[TemplateOption],
    selected: usize,
) -> anyhow::Result<usize> {
    let mut selected = selected.min(templates.len().saturating_sub(1));
    let mut list_state = ListState::default();
    list_state.select(Some(selected));

    loop {
        list_state.select(Some(selected));

        terminal.draw(|frame| {
            let area = frame.area();
            let block = Block::default()
                .borders(Borders::ALL)
                .title("Select template:");
            let inner = block.inner(area);
            frame.render_widget(block, area);

            let items: Vec<ListItem<'_>> = templates
                .iter()
                .map(|template| {
                    ListItem::new(Line::from(format!(
                        "{:<11} - {}",
                        template.name, template.description
                    )))
                })
                .collect();

            let list = List::new(items).highlight_symbol("▶ ").highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );
            frame.render_stateful_widget(list, inner, &mut list_state);
        })?;

        match event::read()? {
            Event::Key(key) if is_key_press(&key) => {
                if is_cancel_key(&key) {
                    anyhow::bail!("card creation cancelled");
                }

                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        if selected == 0 {
                            selected = templates.len().saturating_sub(1);
                        } else {
                            selected = selected.saturating_sub(1);
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        selected = (selected + 1) % templates.len();
                    }
                    KeyCode::Enter => return Ok(selected),
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn card_id_step(terminal: &mut WizardTerminal, initial: &str) -> anyhow::Result<String> {
    let mut input = initial.to_string();

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let block = Block::default().borders(Borders::ALL).title("Card ID");
            let inner = block.inner(area);
            frame.render_widget(block, area);

            let lines = vec![
                Line::from(format!("Card ID: {}█", input)),
                Line::from(
                    "(hint: use team-arch/, team-cli/, team-quality/, team-platform/ prefixes)",
                ),
            ];
            let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
            frame.render_widget(paragraph, inner);
        })?;

        match event::read()? {
            Event::Key(key) if is_key_press(&key) => {
                if is_cancel_key(&key) {
                    anyhow::bail!("card creation cancelled");
                }

                match key.code {
                    KeyCode::Enter => {
                        let trimmed = input.trim();
                        if !trimmed.is_empty() {
                            return Ok(trimmed.to_string());
                        }
                    }
                    KeyCode::Backspace => {
                        input.pop();
                    }
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        input.push(c);
                    }
                    _ => {}
                }
            }
            Event::Paste(pasted) => {
                input.push_str(&pasted);
            }
            _ => {}
        }
    }
}

fn spec_step(terminal: &mut WizardTerminal, initial: &str) -> anyhow::Result<String> {
    let mut input = initial.to_string();

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let rows = Layout::vertical([
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(1),
            ])
            .split(area);

            let header = Paragraph::new(vec![
                Line::from("Paste spec (Ctrl+D to finish, Enter for blank spec):"),
                Line::from(""),
                Line::from(">"),
            ]);
            frame.render_widget(header, rows[0]);

            let mut content = input.clone();
            content.push('█');
            let editor = Paragraph::new(content)
                .block(Block::default().borders(Borders::ALL).title("spec.md"))
                .wrap(Wrap { trim: false });
            frame.render_widget(editor, rows[1]);
        })?;

        match event::read()? {
            Event::Key(key) if is_key_press(&key) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('d') {
                    return Ok(input);
                }
                if is_cancel_key(&key) {
                    anyhow::bail!("card creation cancelled");
                }

                match key.code {
                    KeyCode::Enter => {
                        if input.is_empty() {
                            return Ok(String::new());
                        }
                        input.push('\n');
                    }
                    KeyCode::Backspace => {
                        input.pop();
                    }
                    KeyCode::Tab => {
                        input.push('\t');
                    }
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        input.push(c);
                    }
                    _ => {}
                }
            }
            Event::Paste(pasted) => {
                input.push_str(&pasted);
            }
            _ => {}
        }
    }
}

fn provider_chain_step(
    terminal: &mut WizardTerminal,
    default_provider_chain: &[String],
    current_provider_chain: &[String],
) -> anyhow::Result<Vec<String>> {
    let default_display = default_provider_chain.join(", ");
    let mut input = if current_provider_chain == default_provider_chain {
        String::new()
    } else {
        current_provider_chain.join(", ")
    };

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let block = Block::default()
                .borders(Borders::ALL)
                .title("Provider chain");
            let inner = block.inner(area);
            frame.render_widget(block, area);

            let lines = vec![
                Line::from("Provider chain (comma-separated, or Enter for default):"),
                Line::from(format!("Default: [{}]", default_display)),
                Line::from(format!("> {}█", input)),
            ];
            let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
            frame.render_widget(paragraph, inner);
        })?;

        match event::read()? {
            Event::Key(key) if is_key_press(&key) => {
                if is_cancel_key(&key) {
                    anyhow::bail!("card creation cancelled");
                }

                match key.code {
                    KeyCode::Enter => {
                        if input.trim().is_empty() {
                            return Ok(default_provider_chain.to_vec());
                        }
                        let parsed = parse_provider_chain(&input);
                        if parsed.is_empty() {
                            return Ok(default_provider_chain.to_vec());
                        }
                        return Ok(parsed);
                    }
                    KeyCode::Backspace => {
                        input.pop();
                    }
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        input.push(c);
                    }
                    _ => {}
                }
            }
            Event::Paste(pasted) => {
                input.push_str(&pasted);
            }
            _ => {}
        }
    }
}

fn confirmation_step(
    terminal: &mut WizardTerminal,
    template: &str,
    id: &str,
    provider_chain: &[String],
    spec: &str,
) -> anyhow::Result<ConfirmChoice> {
    let provider_display = provider_chain.join(", ");
    let spec_summary = if spec.trim().is_empty() {
        "(empty - edit output/spec.md after creation)".to_string()
    } else {
        format!("(provided - {} line(s))", spec.lines().count())
    };

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let block = Block::default().borders(Borders::ALL).title("Create card?");
            let inner = block.inner(area);
            frame.render_widget(block, area);

            let lines = vec![
                Line::from(format!("template: {}", template)),
                Line::from(format!("id:       {}", id)),
                Line::from(format!("provider: {}", provider_display)),
                Line::from(format!("spec:     {}", spec_summary)),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[Y]es", Style::default().fg(Color::Green)),
                    Span::raw("  "),
                    Span::styled("[N]o", Style::default().fg(Color::Red)),
                    Span::raw("  "),
                    Span::styled("[E]dit", Style::default().fg(Color::Yellow)),
                ]),
            ];

            let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
            frame.render_widget(paragraph, inner);
        })?;

        match event::read()? {
            Event::Key(key) if is_key_press(&key) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    anyhow::bail!("card creation cancelled");
                }

                match key.code {
                    KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                        return Ok(ConfirmChoice::Yes);
                    }
                    KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                        return Ok(ConfirmChoice::No);
                    }
                    KeyCode::Char('e') | KeyCode::Char('E') => return Ok(ConfirmChoice::Edit),
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn parse_provider_chain(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect()
}

fn discover_templates(cards_dir: &Path) -> anyhow::Result<Vec<TemplateOption>> {
    let templates_dir = cards_dir.join("templates");
    let mut templates: Vec<String> = fs::read_dir(&templates_dir)
        .with_context(|| format!("failed to read templates from {}", templates_dir.display()))?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".bop"))
                .map(str::to_string)
        })
        .collect();

    templates.sort();
    if let Some(implement_idx) = templates
        .iter()
        .position(|template| template == "implement")
    {
        let implement = templates.remove(implement_idx);
        templates.insert(0, implement);
    }

    if templates.is_empty() {
        anyhow::bail!("no templates found in {}", templates_dir.display());
    }

    Ok(templates
        .into_iter()
        .map(|name| TemplateOption {
            description: template_description(&name),
            name,
        })
        .collect())
}

fn template_description(template: &str) -> &'static str {
    match template {
        "implement" => "Standard implement/QA card",
        "cheap" => "Single-stage, low-cost",
        "ideation" => "Brainstorm + spec only",
        "roadmap" => "Multi-phase planning",
        "qa-only" => "QA pass on existing work",
        "mr-fix" => "Fix a merge-request failure",
        _ => "Card template",
    }
}

fn is_key_press(key: &KeyEvent) -> bool {
    matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

fn is_cancel_key(key: &KeyEvent) -> bool {
    key.code == KeyCode::Esc
        || (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c'))
}

#[cfg(test)]
mod tests {
    use super::{discover_templates, parse_provider_chain};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parse_provider_chain_trims_and_drops_empty_segments() {
        let chain = parse_provider_chain(" codex, , claude ,ollama-local ");
        assert_eq!(chain, vec!["codex", "claude", "ollama-local"]);
    }

    #[test]
    fn discover_templates_loads_only_bop_templates() {
        let td = tempdir().unwrap();
        let templates = td.path().join("templates");
        fs::create_dir_all(templates.join("implement.bop")).unwrap();
        fs::create_dir_all(templates.join("roadmap.bop")).unwrap();
        fs::create_dir_all(templates.join("cheap.jobcard")).unwrap();

        let result = discover_templates(td.path()).unwrap();
        let names: Vec<String> = result.into_iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["implement", "roadmap"]);
    }
}
