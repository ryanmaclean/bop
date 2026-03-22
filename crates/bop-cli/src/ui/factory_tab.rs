use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, List, ListItem, ListState, Paragraph, StatefulWidget, Widget, Wrap,
};

use crate::factory::{plist_path, systemd_path_path, systemd_service_path};

/// 250ms tick × 8 = 2s refresh cadence for factory status/logs.
pub const FACTORY_REFRESH_TICKS: u64 = 8;
const LOG_READ_BYTES: i64 = 8192;
pub const LOG_TAIL_LINES: usize = 20;

const DISPATCHER_LABEL: &str = "sh.bop.dispatcher";
const MERGE_GATE_LABEL: &str = "sh.bop.merge-gate";
const ICONWATCHER_LABEL: &str = "sh.bop.iconwatcher";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FactoryLogSource {
    #[default]
    Dispatcher,
    MergeGate,
}

impl FactoryLogSource {
    pub fn path(self) -> &'static str {
        match self {
            Self::Dispatcher => "/tmp/bop-dispatcher.log",
            Self::MergeGate => "/tmp/bop-merge-gate.log",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Dispatcher => "DISPATCHER",
            Self::MergeGate => "MERGE-GATE",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Dispatcher => Self::MergeGate,
            Self::MergeGate => Self::Dispatcher,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactoryServiceStatus {
    Running { pid: Option<u32> },
    Stopped,
    NotInstalled,
    Unsupported,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct FactoryServiceRow {
    pub label: &'static str,
    pub status: FactoryServiceStatus,
}

impl FactoryServiceRow {
    fn is_running(&self) -> bool {
        matches!(self.status, FactoryServiceStatus::Running { .. })
    }

    fn action_hint(&self) -> &'static str {
        if self.is_running() {
            "[S]top"
        } else {
            "[R]un"
        }
    }

    fn status_dot(&self) -> (&'static str, Color) {
        match self.status {
            FactoryServiceStatus::Running { .. } => ("●", Color::Green),
            FactoryServiceStatus::Error(_) => ("!", Color::Red),
            FactoryServiceStatus::Unsupported => ("?", Color::DarkGray),
            _ => ("□", Color::DarkGray),
        }
    }

    fn status_text(&self) -> String {
        match self.status {
            FactoryServiceStatus::Running { pid: Some(pid) } => format!("running pid {pid}"),
            FactoryServiceStatus::Running { pid: None } => "running".to_string(),
            FactoryServiceStatus::Stopped => "stopped".to_string(),
            FactoryServiceStatus::NotInstalled => "not installed".to_string(),
            FactoryServiceStatus::Unsupported => "unsupported".to_string(),
            FactoryServiceStatus::Error(ref err) => format!("error: {err}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FactoryTabState {
    pub services: Vec<FactoryServiceRow>,
    pub selected_service: usize,
    pub log_source: FactoryLogSource,
    pub log_lines: Vec<String>,
}

impl Default for FactoryTabState {
    fn default() -> Self {
        Self::new()
    }
}

impl FactoryTabState {
    pub fn new() -> Self {
        Self {
            services: vec![
                FactoryServiceRow {
                    label: DISPATCHER_LABEL,
                    status: FactoryServiceStatus::Stopped,
                },
                FactoryServiceRow {
                    label: MERGE_GATE_LABEL,
                    status: FactoryServiceStatus::Stopped,
                },
                FactoryServiceRow {
                    label: ICONWATCHER_LABEL,
                    status: FactoryServiceStatus::Stopped,
                },
            ],
            selected_service: 0,
            log_source: FactoryLogSource::Dispatcher,
            log_lines: vec!["(no log lines yet)".to_string()],
        }
    }

    pub fn selected_label(&self) -> Option<&'static str> {
        self.services.get(self.selected_service).map(|s| s.label)
    }

    pub fn select_next(&mut self) {
        if self.services.is_empty() {
            return;
        }
        self.selected_service = (self.selected_service + 1) % self.services.len();
    }

    pub fn select_prev(&mut self) {
        if self.services.is_empty() {
            return;
        }
        self.selected_service = if self.selected_service == 0 {
            self.services.len() - 1
        } else {
            self.selected_service - 1
        };
    }

    pub fn toggle_log_source(&mut self) {
        self.log_source = self.log_source.toggle();
        self.refresh_log_tail();
    }

    pub fn refresh(&mut self) {
        for service in &mut self.services {
            service.status = query_service_status(service.label);
        }
        self.refresh_log_tail();
        self.clamp_selection();
    }

    pub fn refresh_log_tail(&mut self) {
        self.log_lines = read_log_tail(Path::new(self.log_source.path()), LOG_TAIL_LINES);
    }

    pub fn start_selected(&mut self) -> Result<&'static str> {
        let label = self
            .selected_label()
            .context("no factory service selected")?;
        start_service(label)?;
        self.refresh();
        Ok(label)
    }

    pub fn stop_selected(&mut self) -> Result<&'static str> {
        let label = self
            .selected_label()
            .context("no factory service selected")?;
        stop_service(label)?;
        self.refresh();
        Ok(label)
    }

    fn clamp_selection(&mut self) {
        if self.services.is_empty() {
            self.selected_service = 0;
        } else if self.selected_service >= self.services.len() {
            self.selected_service = self.services.len() - 1;
        }
    }
}

pub struct FactoryTabWidget<'a> {
    state: &'a FactoryTabState,
}

impl<'a> FactoryTabWidget<'a> {
    pub fn new(state: &'a FactoryTabState) -> Self {
        Self { state }
    }
}

impl Widget for FactoryTabWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let outer = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(vec![
                Span::styled(
                    " FACTORY SERVICES ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" [F2] close ", Style::default().fg(Color::DarkGray)),
            ]));
        let inner = outer.inner(area);
        outer.render(area, buf);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let service_height = (self.state.services.len() as u16 + 2).max(3);
        let chunks =
            Layout::vertical([Constraint::Length(service_height), Constraint::Min(1)]).split(inner);

        let items: Vec<ListItem> = self
            .state
            .services
            .iter()
            .map(|service| {
                let (dot, dot_color) = service.status_dot();
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{dot} "),
                        Style::default().fg(dot_color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{:<20}", service.label),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        format!("{:<18}", service.status_text()),
                        Style::default().fg(Color::Gray),
                    ),
                    Span::styled(service.action_hint(), Style::default().fg(Color::Cyan)),
                ]))
            })
            .collect();

        let mut list_state = ListState::default();
        if !items.is_empty() {
            list_state.select(Some(self.state.selected_service.min(items.len() - 1)));
        }

        let list = List::new(items)
            .block(Block::default().title(" Services "))
            .highlight_style(Style::default().bg(Color::DarkGray))
            .highlight_symbol("▶ ");
        StatefulWidget::render(list, chunks[0], buf, &mut list_state);

        let log_title = format!(
            " {} LOG  {}  (last {} lines) ",
            self.state.log_source.title(),
            self.state.log_source.path(),
            LOG_TAIL_LINES
        );
        let log_lines: Vec<Line> = self
            .state
            .log_lines
            .iter()
            .map(|line| Line::raw(line.clone()))
            .collect();

        let log_widget = Paragraph::new(log_lines)
            .block(Block::default().borders(Borders::TOP).title(log_title))
            .wrap(Wrap { trim: false });
        log_widget.render(chunks[1], buf);
    }
}

fn query_service_status(label: &str) -> FactoryServiceStatus {
    if cfg!(target_os = "macos") {
        query_launchd_status(label)
    } else if cfg!(target_os = "linux") {
        query_systemd_status(label)
    } else {
        FactoryServiceStatus::Unsupported
    }
}

#[cfg(target_os = "macos")]
fn query_launchd_status(label: &str) -> FactoryServiceStatus {
    let installed = plist_path(label).exists();
    if !installed {
        return FactoryServiceStatus::NotInstalled;
    }

    let out = match Command::new("launchctl").args(["list", label]).output() {
        Ok(output) => output,
        Err(err) => return FactoryServiceStatus::Error(err.to_string()),
    };

    if out.status.success() {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let pid = parse_launchctl_pid(&stdout);
        FactoryServiceStatus::Running { pid }
    } else {
        FactoryServiceStatus::Stopped
    }
}

fn query_systemd_status(label: &str) -> FactoryServiceStatus {
    if label == ICONWATCHER_LABEL {
        return FactoryServiceStatus::Unsupported;
    }

    let installed = systemd_service_path(label).exists() && systemd_path_path(label).exists();
    if !installed {
        return FactoryServiceStatus::NotInstalled;
    }

    let path_unit = format!("{label}.path");
    let active_out = match Command::new("systemctl")
        .args(["--user", "is-active", &path_unit])
        .output()
    {
        Ok(output) => output,
        Err(err) => return FactoryServiceStatus::Error(err.to_string()),
    };

    if !active_out.status.success() {
        return FactoryServiceStatus::Stopped;
    }

    let service_unit = format!("{label}.service");
    let pid = Command::new("systemctl")
        .args(["--user", "show", "-p", "MainPID", &service_unit])
        .output()
        .ok()
        .and_then(|output| {
            let stdout = String::from_utf8_lossy(&output.stdout);
            parse_systemd_pid(&stdout)
        });

    FactoryServiceStatus::Running { pid }
}

fn start_service(label: &str) -> Result<()> {
    if cfg!(target_os = "macos") {
        start_launchd_service(label)
    } else if cfg!(target_os = "linux") {
        start_systemd_service(label)
    } else {
        bail!("factory controls are not supported on this OS")
    }
}

fn stop_service(label: &str) -> Result<()> {
    if cfg!(target_os = "macos") {
        stop_launchd_service(label)
    } else if cfg!(target_os = "linux") {
        stop_systemd_service(label)
    } else {
        bail!("factory controls are not supported on this OS")
    }
}

#[cfg(target_os = "macos")]
fn start_launchd_service(label: &str) -> Result<()> {
    let target = format!("{}/{}", launchd_user_domain()?, label);
    let kickstart = Command::new("launchctl")
        .args(["kickstart", "-k", &target])
        .output()
        .context("failed to execute launchctl kickstart")?;

    if kickstart.status.success() {
        return Ok(());
    }

    // If the service is unloaded, try loading its plist as a fallback.
    let plist = plist_path(label);
    if plist.exists() {
        let load = Command::new("launchctl")
            .args(["load", "-w"])
            .arg(&plist)
            .output()
            .context("failed to execute launchctl load")?;
        if load.status.success() {
            return Ok(());
        }

        let kick_err = String::from_utf8_lossy(&kickstart.stderr);
        let load_err = String::from_utf8_lossy(&load.stderr);
        bail!(
            "launchctl kickstart failed: {}; launchctl load failed: {}",
            kick_err.trim(),
            load_err.trim()
        );
    }

    let err = String::from_utf8_lossy(&kickstart.stderr);
    bail!("launchctl kickstart failed: {}", err.trim())
}

#[cfg(target_os = "macos")]
fn stop_launchd_service(label: &str) -> Result<()> {
    let target = format!("{}/{}", launchd_user_domain()?, label);
    let out = Command::new("launchctl")
        .args(["stop", &target])
        .output()
        .context("failed to execute launchctl stop")?;

    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("launchctl stop failed: {}", err.trim())
    }
}

fn start_systemd_service(label: &str) -> Result<()> {
    if label == ICONWATCHER_LABEL {
        bail!("iconwatcher is macOS-only")
    }

    let unit = format!("{label}.path");
    let out = Command::new("systemctl")
        .args(["--user", "start", &unit])
        .output()
        .context("failed to execute systemctl start")?;

    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("systemctl --user start {} failed: {}", unit, err.trim())
    }
}

fn stop_systemd_service(label: &str) -> Result<()> {
    if label == ICONWATCHER_LABEL {
        bail!("iconwatcher is macOS-only")
    }

    let unit = format!("{label}.path");
    let out = Command::new("systemctl")
        .args(["--user", "stop", &unit])
        .output()
        .context("failed to execute systemctl stop")?;

    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("systemctl --user stop {} failed: {}", unit, err.trim())
    }
}

#[cfg(target_os = "macos")]
fn launchd_user_domain() -> Result<String> {
    if let Ok(uid) = std::env::var("UID") {
        let uid = uid.trim();
        if !uid.is_empty() {
            return Ok(format!("gui/{uid}"));
        }
    }

    let out = Command::new("id")
        .arg("-u")
        .output()
        .context("failed to execute id -u")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("failed to resolve UID via id -u: {}", err.trim());
    }

    let uid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if uid.is_empty() {
        bail!("failed to resolve UID: empty id -u output");
    }

    Ok(format!("gui/{uid}"))
}

#[cfg(target_os = "macos")]
fn parse_launchctl_pid(stdout: &str) -> Option<u32> {
    for line in stdout.lines() {
        if !line.contains("PID") {
            continue;
        }

        // Supports both `"PID" = 24670;` and `PID = 24670` style output.
        let mut digits = String::new();
        for ch in line.chars() {
            if ch.is_ascii_digit() {
                digits.push(ch);
            } else if !digits.is_empty() {
                break;
            }
        }

        if let Ok(pid) = digits.parse::<u32>() {
            if pid > 0 {
                return Some(pid);
            }
        }
    }

    None
}

fn parse_systemd_pid(stdout: &str) -> Option<u32> {
    stdout
        .trim()
        .strip_prefix("MainPID=")
        .and_then(|pid| pid.parse::<u32>().ok())
        .filter(|&pid| pid != 0)
}

fn read_log_tail(path: &Path, max_lines: usize) -> Vec<String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => {
            return vec![format!("(log not found: {})", path.display())];
        }
    };

    let mut reader = BufReader::new(file);
    let file_len = match reader.get_ref().metadata() {
        Ok(meta) => meta.len(),
        Err(_) => return vec!["(unable to read log metadata)".to_string()],
    };

    let seek_result = if file_len > LOG_READ_BYTES as u64 {
        reader.seek(SeekFrom::End(-LOG_READ_BYTES))
    } else {
        reader.seek(SeekFrom::Start(0))
    };

    if seek_result.is_err() {
        return vec!["(unable to seek log tail)".to_string()];
    }

    let mut chunk = String::new();
    if reader.read_to_string(&mut chunk).is_err() {
        return vec!["(unable to read log file)".to_string()];
    }

    let mut lines: Vec<String> = chunk.lines().map(|line| line.to_string()).collect();
    if lines.len() > max_lines {
        let split_at = lines.len() - max_lines;
        lines = lines.split_off(split_at);
    }

    if lines.is_empty() {
        vec!["(no log lines yet)".to_string()]
    } else {
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    #[cfg(target_os = "macos")]
    fn parse_launchctl_pid_extracts_pid() {
        let sample = r#"
        {
            "Label" = "sh.bop.dispatcher";
            "PID" = 24670;
        }
        "#;
        assert_eq!(parse_launchctl_pid(sample), Some(24670));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn parse_launchctl_pid_ignores_zero() {
        let sample = "\"PID\" = 0;";
        assert_eq!(parse_launchctl_pid(sample), None);
    }

    #[test]
    fn parse_systemd_pid_extracts_pid() {
        assert_eq!(parse_systemd_pid("MainPID=1234\n"), Some(1234));
        assert_eq!(parse_systemd_pid("MainPID=0\n"), None);
    }

    #[test]
    fn read_log_tail_returns_last_n_lines() {
        let td = tempfile::tempdir().expect("tempdir");
        let path = td.path().join("sample.log");

        let mut content = String::new();
        for i in 1..=30 {
            content.push_str(&format!("line {i}\n"));
        }
        fs::write(&path, content).expect("write log");

        let lines = read_log_tail(&path, 20);
        assert_eq!(lines.len(), 20);
        assert_eq!(lines.first().map(String::as_str), Some("line 11"));
        assert_eq!(lines.last().map(String::as_str), Some("line 30"));
    }
}
