use anyhow::Context;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const SEARCH_STATES: [&str; 6] = ["drafts", "pending", "running", "done", "merged", "failed"];

#[derive(Debug, Clone)]
struct CardCandidate {
    path: PathBuf,
    canonical_id: String,
    meta_id: Option<String>,
    dir_stem: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CommentEntry {
    ts: String,
    msg: String,
}

pub fn cmd_comment(cards_root: &Path, id: &str, message: &str) -> anyhow::Result<()> {
    let msg = message.trim();
    if msg.is_empty() {
        anyhow::bail!("comment message cannot be empty");
    }

    let card = resolve_card(cards_root, id)?;
    append_comment(&card.path, msg)
}

pub fn cmd_comments(cards_root: &Path, id: &str) -> anyhow::Result<()> {
    let card = resolve_card(cards_root, id)?;
    let comments = read_comments(&card.path)?;

    for comment in comments {
        println!("{}", serde_json::to_string(&comment)?);
    }

    Ok(())
}

fn append_comment(card_path: &Path, message: &str) -> anyhow::Result<()> {
    let logs_dir = card_path.join("logs");
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create logs dir at {}", logs_dir.display()))?;

    let comments_path = logs_dir.join("comments.jsonl");
    let entry = CommentEntry {
        ts: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        msg: message.to_string(),
    };

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&comments_path)
        .with_context(|| format!("failed to open {}", comments_path.display()))?;
    writeln!(file, "{}", serde_json::to_string(&entry)?)
        .with_context(|| format!("failed to append {}", comments_path.display()))?;
    Ok(())
}

fn read_comments(card_path: &Path) -> anyhow::Result<Vec<CommentEntry>> {
    let comments_path = card_path.join("logs").join("comments.jsonl");
    if !comments_path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(&comments_path)
        .with_context(|| format!("failed to read {}", comments_path.display()))?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("failed to read {}", comments_path.display()))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let entry: CommentEntry = serde_json::from_str(line).with_context(|| {
            format!(
                "invalid comments.jsonl line {} in {}",
                idx + 1,
                comments_path.display()
            )
        })?;
        out.push(entry);
    }

    Ok(out)
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
            let team_name = team_from_state_dir(cards_root, &state_dir);
            let entries = match fs::read_dir(&state_dir) {
                Ok(it) => it,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !is_card_dir(&path) {
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
        write_card(td.path(), "team-arch/pending", "spec-041", "spec-041");
        let got = resolve_card(td.path(), "spec-041").unwrap();
        assert_eq!(got.canonical_id, "team-arch/spec-041");
    }

    #[test]
    fn resolve_card_finds_draft_cards() {
        let td = tempdir().unwrap();
        write_card(td.path(), "drafts", "spec-062-notes", "spec-062-notes");
        let got = resolve_card(td.path(), "spec-062").unwrap();
        assert_eq!(got.canonical_id, "spec-062-notes");
    }

    #[test]
    fn resolve_card_reports_ambiguous_prefix() {
        let td = tempdir().unwrap();
        write_card(td.path(), "pending", "spec-041-a", "spec-041-a");
        write_card(td.path(), "failed", "spec-041-b", "spec-041-b");
        let err = resolve_card(td.path(), "spec-041").unwrap_err().to_string();
        assert!(err.contains("ambiguous"));
    }

    #[test]
    fn cmd_comment_appends_jsonl_entries() {
        let td = tempdir().unwrap();
        let card = write_card(td.path(), "pending", "spec-062", "spec-062");

        cmd_comment(td.path(), "spec-062", "first").unwrap();
        cmd_comment(td.path(), "spec-062", "second").unwrap();

        let comments_path = card.join("logs").join("comments.jsonl");
        let content = fs::read_to_string(comments_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(first.get("msg").and_then(|v| v.as_str()), Some("first"));
        assert_eq!(second.get("msg").and_then(|v| v.as_str()), Some("second"));
        assert!(first
            .get("ts")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty()));
    }

    #[test]
    fn read_comments_returns_file_order() {
        let td = tempdir().unwrap();
        let card = write_card(td.path(), "done", "spec-062", "spec-062");
        append_comment(&card, "one").unwrap();
        append_comment(&card, "two").unwrap();

        let entries = read_comments(&card).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].msg, "one");
        assert_eq!(entries[1].msg, "two");
    }

    #[test]
    fn cmd_comment_rejects_empty_message() {
        let td = tempdir().unwrap();
        write_card(td.path(), "running", "spec-062", "spec-062");
        let err = cmd_comment(td.path(), "spec-062", "   ")
            .unwrap_err()
            .to_string();
        assert!(err.contains("cannot be empty"));
    }
}
