use bop_core::VcsEngine as CoreVcsEngine;
use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

pub fn command_available(name: &str) -> bool {
    // Try system PATH first
    if StdCommand::new(name)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return true;
    }
    // Try well-known user-local paths
    let home = std::env::var("HOME").unwrap_or_default();
    for dir in [
        format!("{home}/.local/bin"),
        format!("{home}/.cargo/bin"),
        "/opt/homebrew/bin".to_string(),
    ] {
        let full = format!("{dir}/{name}");
        if StdCommand::new(&full)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

pub fn cmd_doctor(cards_root: &Path) -> anyhow::Result<()> {
    println!("bop doctor");

    // ── core tools ──────────────────────────────────────────────────────────
    println!("\n── tools ──");
    let checks = [
        ("git", command_available("git")),
        ("gt", command_available("gt")),
        ("jj", command_available("jj")),
        ("gh", command_available("gh")),
        ("nu", command_available("nu")),
        ("zellij", command_available("zellij")),
    ];

    let mut failed = 0;
    for (name, ok) in checks {
        if ok {
            println!("ok\t{}", name);
        } else {
            println!("missing\t{}", name);
            failed += 1;
        }
    }

    // ── zellij plugin ──────────────────────────────────────────────────────
    let home = std::env::var("HOME").unwrap_or_default();
    let plugin_path = Path::new(&home).join(".config/zellij/plugins/bop.wasm");
    if plugin_path.exists() {
        println!("ok\tzellij plugin ({})", plugin_path.display());
    } else {
        println!("missing\tzellij plugin (run `bop factory install`)");
    }

    // ── adapters ────────────────────────────────────────────────────────────
    println!("\n── adapters ──");
    let adapters_dir = cards_root.parent().unwrap_or(cards_root).join("adapters");

    // Map adapter name → CLI binary it requires
    let adapter_cli_map: &[(&str, &str)] = &[
        ("claude", "claude"),
        ("codex", "codex"),
        ("ollama-local", "ollama"),
        ("goose", "goose"),
        ("aider", "aider"),
        ("opencode", "opencode"),
        ("mock", "true"), // mock always works
    ];

    if adapters_dir.is_dir() {
        for (adapter, cli) in adapter_cli_map {
            let script = adapters_dir.join(format!("{}.nu", adapter));
            if !script.exists() {
                continue; // adapter not installed, skip
            }
            let cli_ok = command_available(cli);
            if cli_ok {
                println!("ok\t{}\t({})", adapter, cli);
            } else {
                println!("missing\t{}\t({} not found)", adapter, cli);
                // Adapter missing is a warning, not a hard failure —
                // the system works with any subset of adapters
            }
        }
    } else {
        println!("warn\tadapters/ directory not found");
    }

    // ── cards layout ────────────────────────────────────────────────────────
    println!("\n── cards ──");
    let policy = cards_root.join("policy.toml");
    if policy.exists() {
        println!("ok\t{}", policy.display());
    } else {
        println!("missing\t{}", policy.display());
        failed += 1;
    }

    let system_ctx = cards_root.join("system_context.md");
    if system_ctx.exists() {
        println!("ok\tsystem_context.md");
    } else {
        println!("missing\tsystem_context.md");
    }

    let stages_dir = cards_root.join("stages");
    if stages_dir.is_dir() {
        let n_stages = fs::read_dir(&stages_dir)
            .map(|rd| {
                rd.filter(|e| {
                    e.as_ref()
                        .map(|e| e.path().extension().map(|x| x == "md").unwrap_or(false))
                        .unwrap_or(false)
                })
                .count()
            })
            .unwrap_or(0);
        println!("ok\tstages/ ({} files)", n_stages);
    } else {
        println!("missing\tstages/");
    }

    let templates_dir = cards_root.join("templates");
    if templates_dir.is_dir() {
        let n_templates = fs::read_dir(&templates_dir)
            .map(|rd| {
                rd.filter(|e| {
                    e.as_ref()
                        .map(|e| {
                            e.path()
                                .extension()
                                .map(|x| x == "bop")
                                .unwrap_or(false)
                        })
                        .unwrap_or(false)
                })
                .count()
            })
            .unwrap_or(0);
        println!("ok\ttemplates/ ({} templates)", n_templates);
    } else {
        println!("missing\ttemplates/");
    }

    // ── acceptance criteria lint ────────────────────────────────────────────
    println!("\n── acceptance criteria ──");
    let pending_dir = cards_root.join("pending");
    if pending_dir.is_dir() {
        let mut any_cards = false;
        if let Ok(entries) = fs::read_dir(&pending_dir) {
            for entry in entries.flatten() {
                let card_dir = entry.path();
                if card_dir
                    .extension()
                    .map(|e| e == "bop")
                    .unwrap_or(false)
                    && card_dir.is_dir()
                {
                    let meta_path = card_dir.join("meta.json");
                    let Ok(meta) = bop_core::read_meta(&card_dir) else {
                        continue;
                    };
                    if meta.acceptance_criteria.is_empty() {
                        continue;
                    }
                    any_cards = true;
                    let card_id = card_dir.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
                    let _ = meta_path; // read above
                    let is_git_mode = matches!(meta.vcs_engine, Some(CoreVcsEngine::GitGt));
                    let mut issues: Vec<String> = Vec::new();
                    for (idx, cmd) in meta.acceptance_criteria.iter().enumerate() {
                        // 1. Absolute paths
                        if cmd.contains("/Users/") || cmd.contains("/home/") {
                            issues.push(format!(
                                "criteria[{idx}] — absolute path (non-portable): {cmd}"
                            ));
                        }
                        // 2. Missing binary (first token)
                        let binary = cmd.split_whitespace().next().unwrap_or("");
                        if !binary.is_empty() && !command_available(binary) {
                            issues.push(format!("criteria[{idx}] — binary not found: {binary}"));
                        }
                        // 3. jj commands in git mode
                        if is_git_mode {
                            let first = cmd.split_whitespace().next().unwrap_or("");
                            if first == "jj" {
                                issues.push(format!(
                                    "criteria[{idx}] — jj command but vcs_engine is git_gt: {cmd}"
                                ));
                            }
                        }
                    }
                    if issues.is_empty() {
                        println!(
                            "✓ {}: {} criteria OK",
                            card_id,
                            meta.acceptance_criteria.len()
                        );
                    } else {
                        for issue in &issues {
                            println!("✗ {card_id}: {issue}");
                        }
                    }
                }
            }
        }
        if !any_cards {
            println!("ok\tno pending cards with acceptance criteria");
        }
    } else {
        println!("ok\tno pending/ directory");
    }

    if failed > 0 {
        anyhow::bail!("doctor found {} issue(s)", failed);
    }
    Ok(())
}

pub fn print_status_summary(root: &Path) -> anyhow::Result<()> {
    crate::list::list_cards(root, "active")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn command_available_echo() {
        // "echo" should always be available
        assert!(command_available("echo"));
    }

    #[test]
    fn command_available_nonexistent() {
        assert!(!command_available("nonexistent-command-xyz-12345"));
    }

    #[test]
    fn print_status_summary_empty_root() {
        let td = tempdir().unwrap();
        // Create state dirs so list_cards has something to scan
        for state in ["pending", "running", "done"] {
            fs::create_dir_all(td.path().join(state)).unwrap();
        }
        print_status_summary(td.path()).unwrap();
    }

    #[test]
    fn print_status_summary_with_cards() {
        let td = tempdir().unwrap();
        let card_dir = td.path().join("pending").join("test-card.bop");
        fs::create_dir_all(&card_dir).unwrap();
        let meta = bop_core::Meta {
            id: "test-card".into(),
            stage: "implement".into(),
            ..Default::default()
        };
        bop_core::write_meta(&card_dir, &meta).unwrap();
        print_status_summary(td.path()).unwrap();
    }
}
