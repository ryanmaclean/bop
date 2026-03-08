use anyhow::Context;
use std::ffi::OsString;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use terminal_size::{terminal_size, Height};

use crate::workspace;

const SEARCH_STATES: [&str; 4] = ["done", "merged", "failed", "running"];

#[derive(Debug, Clone)]
struct CardCandidate {
    path: PathBuf,
    state: String,
    canonical_id: String,
    meta_id: Option<String>,
    dir_stem: String,
}

pub fn cmd_diff(
    cards_root: &Path,
    id: &str,
    stat: bool,
    output: bool,
    worktree: bool,
) -> anyhow::Result<()> {
    let card = resolve_card(cards_root, id)?;
    let meta = bop_core::read_meta(&card.path)
        .with_context(|| format!("failed to read meta for {}", card.path.display()))?;

    if output {
        return print_result_output(&card.path);
    }
    if worktree {
        return open_worktree(&card.path, &meta);
    }

    match card.state.as_str() {
        "merged" => show_merged_diff(cards_root, &card.path, &meta, stat),
        _ => show_worktree_diff_or_output(&card.path, &meta, stat),
    }
}

fn show_merged_diff(
    cards_root: &Path,
    card_path: &Path,
    meta: &bop_core::Meta,
    stat: bool,
) -> anyhow::Result<()> {
    let merge_ref = merge_ref(card_path, meta)
        .with_context(|| format!("no merge commit/ref recorded for card {}", meta.id))?;
    let git_root = workspace::find_git_root(cards_root)
        .or_else(|| std::env::current_dir().ok())
        .context("unable to locate git repository root")?;

    let mut args = vec!["diff".to_string()];
    if stat {
        args.push("--shortstat".to_string());
    } else {
        args.push("--color=always".to_string());
    }
    args.push(format!("{merge_ref}^..{merge_ref}"));

    let raw = run_git_capture(&git_root, &args)?;
    render_for_terminal(raw, stat)
}

fn show_worktree_diff_or_output(
    card_path: &Path,
    meta: &bop_core::Meta,
    stat: bool,
) -> anyhow::Result<()> {
    let worktree = meta
        .workspace_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|p| p.exists());

    let Some(worktree) = worktree else {
        println!("no worktree available; showing output/result.md (no code diff).");
        return print_result_output(card_path);
    };

    let mut args = vec!["diff".to_string()];
    if stat {
        args.push("--shortstat".to_string());
    } else {
        args.push("--color=always".to_string());
    }
    args.push("HEAD".to_string());

    let raw = run_git_capture(&worktree, &args)?;
    render_for_terminal(raw, stat)
}

fn run_git_capture(cwd: &Path, args: &[String]) -> anyhow::Result<Vec<u8>> {
    let out = StdCommand::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!(
            "git {} failed in {}: {}",
            args.join(" "),
            cwd.display(),
            stderr.trim()
        );
    }

    Ok(out.stdout)
}

fn render_for_terminal(raw: Vec<u8>, stat: bool) -> anyhow::Result<()> {
    let bytes = if stat { raw } else { maybe_delta(raw) };
    print_with_pager(bytes)
}

fn maybe_delta(raw: Vec<u8>) -> Vec<u8> {
    if !command_exists("delta") {
        return raw;
    }

    let mut child = match StdCommand::new("delta")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return raw,
    };

    if let Some(stdin) = child.stdin.as_mut() {
        if stdin.write_all(&raw).is_err() {
            return raw;
        }
    } else {
        return raw;
    }

    match child.wait_with_output() {
        Ok(out) if out.status.success() => out.stdout,
        _ => raw,
    }
}

fn command_exists(cmd: &str) -> bool {
    StdCommand::new("sh")
        .arg("-lc")
        .arg(format!("command -v {cmd} >/dev/null 2>&1"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn print_with_pager(content: Vec<u8>) -> anyhow::Result<()> {
    if !io::stdout().is_terminal() {
        io::stdout().write_all(&content)?;
        return Ok(());
    }

    let lines = String::from_utf8_lossy(&content).lines().count();
    let rows = terminal_rows();
    if lines <= rows {
        io::stdout().write_all(&content)?;
        return Ok(());
    }

    let mut pager = StdCommand::new("sh")
        .arg("-lc")
        .arg("exec ${PAGER:-less -R}")
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to launch pager")?;

    if let Some(stdin) = pager.stdin.as_mut() {
        stdin.write_all(&content)?;
    }
    let _ = pager.wait();
    Ok(())
}

fn terminal_rows() -> usize {
    terminal_size()
        .map(|(_, Height(h))| h as usize)
        .unwrap_or(24)
}

fn print_result_output(card_path: &Path) -> anyhow::Result<()> {
    let result_path = card_path.join("output").join("result.md");
    let text = fs::read_to_string(&result_path)
        .with_context(|| format!("result output not found at {}", result_path.display()))?;
    print_with_pager(text.into_bytes())
}

fn open_worktree(card_path: &Path, meta: &bop_core::Meta) -> anyhow::Result<()> {
    let worktree = meta
        .workspace_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|p| p.exists());

    if let Some(path) = worktree {
        println!("cd {}", path.display());
        let status = StdCommand::new("sh")
            .arg("-lc")
            .arg("exec ${EDITOR:-vi} \"$1\"")
            .arg("sh")
            .arg(path.as_os_str())
            .status()
            .context("failed to launch $EDITOR")?;
        if !status.success() {
            anyhow::bail!("$EDITOR exited with status {:?}", status.code());
        }
        return Ok(());
    }

    if let Some(r) = merge_ref(card_path, meta) {
        println!("worktree no longer exists; merged at {}", r);
        return Ok(());
    }

    anyhow::bail!("card has no worktree and no merge commit/ref recorded")
}

fn merge_ref(card_path: &Path, meta: &bop_core::Meta) -> Option<String> {
    let meta_json = fs::read_to_string(card_path.join("meta.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&meta_json).ok()?;
    let merge_commit = value
        .get("merge_commit")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    merge_commit.or_else(|| {
        meta.change_ref
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn resolve_card(cards_root: &Path, id: &str) -> anyhow::Result<CardCandidate> {
    let query = id.trim();
    if query.is_empty() {
        anyhow::bail!("card id cannot be empty");
    }

    let candidates = collect_candidates(cards_root)?;
    let exact: Vec<_> = candidates
        .iter()
        .filter(|c| matches_id_exact(c, query))
        .cloned()
        .collect();

    if exact.len() == 1 {
        return Ok(exact[0].clone());
    }
    if exact.len() > 1 {
        return ambiguous_error(query, &exact);
    }

    let prefix: Vec<_> = candidates
        .iter()
        .filter(|c| matches_id_prefix(c, query))
        .cloned()
        .collect();
    if prefix.len() == 1 {
        return Ok(prefix[0].clone());
    }
    if prefix.is_empty() {
        anyhow::bail!("card id not found: {query}");
    }

    ambiguous_error(query, &prefix)
}

fn ambiguous_error(query: &str, matches: &[CardCandidate]) -> anyhow::Result<CardCandidate> {
    eprintln!("multiple cards match '{query}':");
    for m in matches {
        let rel = m
            .path
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches("./")
            .to_string();
        eprintln!("  - {} ({})", m.canonical_id, rel);
    }
    anyhow::bail!("card id is ambiguous")
}

fn matches_id_exact(c: &CardCandidate, query: &str) -> bool {
    c.canonical_id == query || c.meta_id.as_deref() == Some(query) || c.dir_stem == query
}

fn matches_id_prefix(c: &CardCandidate, query: &str) -> bool {
    c.canonical_id.starts_with(query)
        || c.meta_id
            .as_deref()
            .map(|m| m.starts_with(query))
            .unwrap_or(false)
        || c.dir_stem.starts_with(query)
}

fn collect_candidates(cards_root: &Path) -> anyhow::Result<Vec<CardCandidate>> {
    let mut out = Vec::new();
    for state in SEARCH_STATES {
        for state_dir in state_dirs(cards_root, state)? {
            let state_name = state.to_string();
            let team_name = team_from_state_dir(cards_root, &state_dir);
            let entries = match fs::read_dir(&state_dir) {
                Ok(it) => it,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let ext = path.extension().and_then(|e| e.to_str());
                if !matches!(ext, Some("bop") | Some("jobcard")) {
                    continue;
                }

                let dir_stem = file_stem_string(&path);
                let meta_id = bop_core::read_meta(&path).ok().map(|m| m.id);
                let id_base = meta_id.clone().unwrap_or_else(|| dir_stem.clone());
                let canonical_id = if let Some(team) = &team_name {
                    format!("{team}/{id_base}")
                } else {
                    id_base
                };

                out.push(CardCandidate {
                    path,
                    state: state_name.clone(),
                    canonical_id,
                    meta_id,
                    dir_stem,
                });
            }
        }
    }
    out.sort_by(|a, b| a.canonical_id.cmp(&b.canonical_id));
    Ok(out)
}

fn file_stem_string(path: &Path) -> String {
    path.file_stem()
        .map(OsString::from)
        .and_then(|s| s.into_string().ok())
        .unwrap_or_else(|| String::from("unknown"))
}

fn team_from_state_dir(cards_root: &Path, state_dir: &Path) -> Option<String> {
    let parent = state_dir.parent()?;
    if parent == cards_root {
        return None;
    }
    let name = parent.file_name()?.to_string_lossy().to_string();
    if name.starts_with("team-") {
        Some(name)
    } else {
        None
    }
}

fn state_dirs(cards_root: &Path, state: &str) -> anyhow::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    let root_state = cards_root.join(state);
    if root_state.exists() {
        dirs.push(root_state);
    }

    let entries = fs::read_dir(cards_root)?;
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if p.is_dir() && name.starts_with("team-") {
            let team_state = p.join(state);
            if team_state.exists() {
                dirs.push(team_state);
            }
        }
    }
    dirs.sort();
    Ok(dirs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::tempdir;

    fn write_card(root: &Path, rel_state_dir: &str, dir_name: &str, meta_id: &str) -> PathBuf {
        let card = root.join(rel_state_dir).join(format!("{dir_name}.bop"));
        fs::create_dir_all(&card).unwrap();
        fs::create_dir_all(card.join("logs")).unwrap();
        fs::create_dir_all(card.join("output")).unwrap();
        let meta = bop_core::Meta {
            id: meta_id.to_string(),
            created: Utc::now(),
            stage: "implement".to_string(),
            ..Default::default()
        };
        bop_core::write_meta(&card, &meta).unwrap();
        card
    }

    #[test]
    fn resolve_card_prefix_matches_team_card() {
        let td = tempdir().unwrap();
        write_card(td.path(), "team-arch/done", "spec-041", "spec-041");
        let got = resolve_card(td.path(), "spec-041").unwrap();
        assert_eq!(got.canonical_id, "team-arch/spec-041");
    }

    #[test]
    fn resolve_card_reports_ambiguous_prefix() {
        let td = tempdir().unwrap();
        write_card(td.path(), "done", "spec-041-a", "spec-041-a");
        write_card(td.path(), "failed", "spec-041-b", "spec-041-b");
        let err = resolve_card(td.path(), "spec-041").unwrap_err().to_string();
        assert!(err.contains("ambiguous"));
    }

    #[test]
    fn merge_ref_prefers_merge_commit_field() {
        let td = tempdir().unwrap();
        let card = write_card(td.path(), "merged", "x", "x");
        fs::write(
            card.join("meta.json"),
            r#"{"id":"x","stage":"implement","created":"2026-01-01T00:00:00Z","merge_commit":"abc123"}"#,
        )
        .unwrap();
        let meta = bop_core::Meta {
            id: "x".to_string(),
            created: Utc::now(),
            stage: "implement".to_string(),
            change_ref: Some("fallback".to_string()),
            ..Default::default()
        };
        assert_eq!(merge_ref(&card, &meta).as_deref(), Some("abc123"));
    }
}
