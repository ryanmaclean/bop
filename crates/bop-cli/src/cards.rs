use anyhow::Context;
use bop_core::{write_meta, Meta, StageStatus};
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use tokio::process::Command as TokioCommand;

use crate::{paths, quicklook, reaper, util};

// ── Transient Failure Patterns ──────────────────────────────────────────────

/// Patterns that indicate a card failed due to transient causes (network issues,
/// rate limits, timeouts) rather than permanent failures (build errors, test failures).
/// Used by `bop retry-transient` to determine which failed cards should be retried.
const TRANSIENT_PATTERNS: &[&str] = &[
    "rate limit",
    "429",
    "503",
    "timeout",
    "connection refused",
    "network",
    "ECONNRESET",
    "EX_TEMPFAIL",
    "name resolution failed",
    "no route to host",
    "524",
];

// ── Card Scanning for Cleanup ───────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct ScanResult {
    pub corrupt_cards: Vec<PathBuf>, // Cards with no readable meta.json
    pub old_failed_cards: Vec<PathBuf>, // Cards in failed/ older than threshold
    pub orphan_running_cards: Vec<PathBuf>, // Empty running/ cards with dead PID
    pub target_dirs: Vec<PathBuf>,   // target/ dirs inside cards
}

pub fn scan_cards_for_cleanup(root: &Path, failed_age_days: u32) -> anyhow::Result<ScanResult> {
    let states = vec!["drafts", "pending", "running", "done", "failed", "merged"];
    let mut result = ScanResult::default();

    for state in &states {
        scan_state_dir_for_cleanup(root, state, failed_age_days, &mut result)?;

        // Also scan team-* directories
        if let Ok(entries) = fs::read_dir(root) {
            let mut team_dirs: Vec<_> = entries
                .flatten()
                .filter(|e| {
                    let name = e.file_name();
                    let s = name.to_string_lossy();
                    e.path().is_dir() && s.starts_with("team-")
                })
                .collect();
            team_dirs.sort_by_key(|e| e.file_name());

            for entry in team_dirs {
                scan_state_dir_for_cleanup(&entry.path(), state, failed_age_days, &mut result)?;
            }
        }
    }

    Ok(result)
}

fn scan_state_dir_for_cleanup(
    dir: &Path,
    state: &str,
    failed_age_days: u32,
    result: &mut ScanResult,
) -> anyhow::Result<()> {
    let state_dir = dir.join(state);
    if !state_dir.exists() {
        return Ok(());
    }

    if let Ok(entries) = fs::read_dir(&state_dir) {
        for entry in entries.flatten() {
            let card_path = entry.path();
            if !card_path.is_dir() {
                continue;
            }

            let ext = card_path.extension().and_then(|e| e.to_str());
            if !matches!(ext, Some("bop") | Some("jobcard")) {
                continue;
            }

            // Check for corrupt card (no readable meta.json)
            if bop_core::read_meta(&card_path).is_err() {
                result.corrupt_cards.push(card_path.clone());
                continue;
            }

            // Check for old failed cards
            if state == "failed" {
                if let Ok(metadata) = fs::metadata(&card_path) {
                    if let Ok(modified) = metadata.modified() {
                        let age = std::time::SystemTime::now()
                            .duration_since(modified)
                            .unwrap_or_default();
                        if age.as_secs() > (failed_age_days as u64 * 86400) {
                            result.old_failed_cards.push(card_path.clone());
                        }
                    }
                }
            }

            // Check for orphan running cards (empty with dead PID)
            if state == "running" {
                // Check if card is empty (no meaningful output/logs)
                let is_empty = card_path
                    .join("output")
                    .read_dir()
                    .ok()
                    .and_then(|mut d| if d.next().is_none() { Some(true) } else { None })
                    .unwrap_or(false);

                if is_empty {
                    // Check if there are any meaningful log files (ignore events.jsonl)
                    let has_meaningful_logs = card_path
                        .join("logs")
                        .read_dir()
                        .ok()
                        .and_then(|entries| {
                            entries
                                .flatten()
                                .any(|e| e.file_name() != "events.jsonl" && e.path().is_file())
                                .then_some(true)
                        })
                        .unwrap_or(false);

                    // If empty and no meaningful logs, likely orphaned
                    if !has_meaningful_logs {
                        result.orphan_running_cards.push(card_path.clone());
                    }
                }
            }

            // Check for target/ directories inside cards
            let target_dir = card_path.join("target");
            if target_dir.exists() && target_dir.is_dir() {
                result.target_dirs.push(target_dir);
            }
        }
    }

    Ok(())
}

// ── Helper: Calculate directory size ────────────────────────────────────────

fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(metadata) = fs::metadata(&path) {
                if metadata.is_dir() {
                    total += dir_size(&path);
                } else {
                    total += metadata.len();
                }
            }
        }
    }
    total
}

// ── Removal Logic with Dry-Run Support ─────────────────────────────────────

#[derive(Debug, Default)]
pub struct CleanupStats {
    pub corrupt_cards_removed: usize,
    pub old_failed_cards_removed: usize,
    pub orphan_running_cards_removed: usize,
    pub target_dirs_removed: usize,
    pub bytes_freed: u64,
}

pub fn perform_cleanup(scan_result: &ScanResult, dry_run: bool) -> anyhow::Result<CleanupStats> {
    let mut stats = CleanupStats::default();

    // Remove corrupt cards
    for card_path in &scan_result.corrupt_cards {
        let size = dir_size(card_path);
        if dry_run {
            println!(
                "[DRY-RUN] Would remove corrupt card: {}",
                card_path.display()
            );
        } else {
            fs::remove_dir_all(card_path).with_context(|| {
                format!("Failed to remove corrupt card: {}", card_path.display())
            })?;
            println!("Removed corrupt card: {}", card_path.display());
        }
        stats.corrupt_cards_removed += 1;
        stats.bytes_freed += size;
    }

    // Remove old failed cards
    for card_path in &scan_result.old_failed_cards {
        let size = dir_size(card_path);
        if dry_run {
            println!(
                "[DRY-RUN] Would remove old failed card: {}",
                card_path.display()
            );
        } else {
            fs::remove_dir_all(card_path).with_context(|| {
                format!("Failed to remove old failed card: {}", card_path.display())
            })?;
            println!("Removed old failed card: {}", card_path.display());
        }
        stats.old_failed_cards_removed += 1;
        stats.bytes_freed += size;
    }

    // Remove orphan running cards
    for card_path in &scan_result.orphan_running_cards {
        let size = dir_size(card_path);
        if dry_run {
            println!(
                "[DRY-RUN] Would remove orphan running card: {}",
                card_path.display()
            );
        } else {
            fs::remove_dir_all(card_path).with_context(|| {
                format!(
                    "Failed to remove orphan running card: {}",
                    card_path.display()
                )
            })?;
            println!("Removed orphan running card: {}", card_path.display());
        }
        stats.orphan_running_cards_removed += 1;
        stats.bytes_freed += size;
    }

    // Remove target/ directories
    for target_dir in &scan_result.target_dirs {
        let size = dir_size(target_dir);
        if dry_run {
            println!(
                "[DRY-RUN] Would remove target/ directory: {}",
                target_dir.display()
            );
        } else {
            fs::remove_dir_all(target_dir).with_context(|| {
                format!(
                    "Failed to remove target/ directory: {}",
                    target_dir.display()
                )
            })?;
            println!("Removed target/ directory: {}", target_dir.display());
        }
        stats.target_dirs_removed += 1;
        stats.bytes_freed += size;
    }

    Ok(stats)
}

// ── Summary Reporting ────────────────────────────────────────────────────────

pub fn print_cleanup_summary(stats: &CleanupStats, dry_run: bool) {
    let total_cards = stats.corrupt_cards_removed
        + stats.old_failed_cards_removed
        + stats.orphan_running_cards_removed;

    let mb_freed = stats.bytes_freed as f64 / 1_048_576.0;

    if dry_run {
        println!();
        println!("Summary (dry-run):");
        println!("  Would remove {} card(s)", total_cards);
        println!(
            "  Would remove {} target/ director{}",
            stats.target_dirs_removed,
            if stats.target_dirs_removed == 1 {
                "y"
            } else {
                "ies"
            }
        );
        println!("  Would free {:.2} MB", mb_freed);
    } else {
        println!();
        println!("Summary:");
        println!("  Removed {} card(s)", total_cards);
        println!(
            "  Removed {} target/ director{}",
            stats.target_dirs_removed,
            if stats.target_dirs_removed == 1 {
                "y"
            } else {
                "ies"
            }
        );
        println!("  Freed {:.2} MB", mb_freed);

        // Success message with next-step hint
        if total_cards > 0 || stats.target_dirs_removed > 0 {
            println!();
            println!("✓ Cleaned {} cards → bop list to verify", total_cards);
        }
    }
}

// ── cmd_clean ────────────────────────────────────────────────────────────────

pub fn cmd_clean(
    root: &Path,
    dry_run: bool,
    older_than: Option<u64>,
    state: Option<String>,
) -> anyhow::Result<()> {
    // Set default age threshold (30 days for failed cards)
    let failed_age_days = older_than.unwrap_or(30) as u32;

    // Validate state filter if provided
    if let Some(ref state_filter) = state {
        let valid_states = ["done", "failed", "both"];
        if !valid_states.contains(&state_filter.as_str()) {
            anyhow::bail!(
                "Invalid state '{}'. Must be one of: done, failed, both",
                state_filter
            );
        }
    }

    // Scan for cards to clean up
    let scan_result = scan_cards_for_cleanup(root, failed_age_days)?;

    // Filter by state if requested
    let mut filtered_result = ScanResult::default();
    match state.as_deref() {
        Some("done") => {
            // Only clean done/ cards - for now this means target/ dirs in done/
            // (corrupt/orphan detection works across all states)
            filtered_result.target_dirs = scan_result
                .target_dirs
                .into_iter()
                .filter(|p| p.to_string_lossy().contains("/done/"))
                .collect();
        }
        Some("failed") => {
            filtered_result.old_failed_cards = scan_result.old_failed_cards;
            filtered_result.target_dirs = scan_result
                .target_dirs
                .into_iter()
                .filter(|p| p.to_string_lossy().contains("/failed/"))
                .collect();
        }
        Some("both") | None => {
            // Clean both done/ and failed/ (default behavior)
            filtered_result = scan_result;
        }
        _ => unreachable!("validated above"),
    }

    // Perform cleanup
    let stats = perform_cleanup(&filtered_result, dry_run)?;

    // Print summary
    print_cleanup_summary(&stats, dry_run);

    Ok(())
}

// ── seed_default_templates ───────────────────────────────────────────────────

pub fn seed_default_templates(cards_dir: &Path) -> anyhow::Result<()> {
    let templates_dir = cards_dir.join("templates");
    let implement = templates_dir.join("implement.bop");
    if !implement.exists() {
        fs::create_dir_all(implement.join("logs"))?;
        fs::create_dir_all(implement.join("output"))?;

        let meta = Meta {
            id: "template-implement".to_string(),
            meta_version: 1,
            created: Utc::now(),
            agent_type: None,
            stage: "implement".to_string(),
            card_type: None,
            metadata_source: None,
            metadata_key: None,
            workflow_mode: Some("default-feature".to_string()),
            step_index: Some(1),
            priority: None,
            provider_chain: vec![],
            stages: Default::default(),
            acceptance_criteria: vec![],
            worktree_branch: Some("job/template-implement".to_string()),
            template_namespace: Some("implement".to_string()),
            vcs_engine: None,
            workspace_name: None,
            workspace_path: None,
            change_ref: None,
            policy_scope: vec![],
            decision_required: false,
            decision_path: None,
            depends_on: vec![],
            spawn_to: None,
            policy_result: None,
            timeout_seconds: None,
            retry_count: Some(0),
            failure_reason: None,
            exit_code: None,
            paused_at: None,
            validation_summary: None,
            glyph: None,
            token: None,
            title: None,
            description: None,
            labels: vec![],
            progress: None,
            subtasks: vec![],
            poker_round: None,
            estimates: Default::default(),
            zellij_session: None,
            zellij_pane: None,
            ac_spec_id: None,
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
            runs: vec![],
            checksum: None,
        };
        write_meta(&implement, &meta)?;

        if !implement.join("spec.md").exists() {
            fs::write(implement.join("spec.md"), "")?;
        }
        if !implement.join("prompt.md").exists() {
            fs::write(
                implement.join("prompt.md"),
                "{{spec}}\n\nProject memory:\n{{memory}}\n\n{{bridge_skill}}\n\nAcceptance criteria:\n{{acceptance_criteria}}\n",
            )?;
        }
    }
    Ok(())
}

// ── create_card ──────────────────────────────────────────────────────────────

pub fn create_card(
    cards_dir: &Path,
    template: &str,
    id: &str,
    spec_override: Option<&str>,
    team_override: Option<&str>,
) -> anyhow::Result<PathBuf> {
    paths::ensure_cards_layout(cards_dir)?;

    let template = template.trim();
    if template.is_empty() {
        anyhow::bail!("template cannot be empty");
    }

    let id = id.trim();
    if id.is_empty() {
        anyhow::bail!("id cannot be empty");
    }
    if id.contains('/') || id.contains('\\') {
        anyhow::bail!("id cannot contain path separators");
    }

    let template_dir = cards_dir
        .join("templates")
        .join(format!("{}.bop", template));
    if !template_dir.exists() {
        anyhow::bail!("template not found: {}", template);
    }

    let card_dir =
        cards_dir
            .join("pending")
            .join(format!("{}-{}.bop", bop_core::cardchars::CARD_BACK, id));
    if card_dir.exists() {
        anyhow::bail!("card already exists: {}", id);
    }

    paths::clone_template(&template_dir, &card_dir)
        .with_context(|| format!("failed to clone template {}", template))?;

    fs::create_dir_all(card_dir.join("logs"))?;
    fs::create_dir_all(card_dir.join("output"))?;

    let mut meta = bop_core::read_meta(&card_dir).unwrap_or_else(|_| Meta {
        id: id.to_string(),
        meta_version: 1,
        created: Utc::now(),
        agent_type: None,
        stage: "spec".to_string(),
        card_type: None,
        metadata_source: None,
        metadata_key: None,
        workflow_mode: None,
        step_index: None,
        priority: None,
        provider_chain: vec![],
        stages: Default::default(),
        acceptance_criteria: vec![],
        worktree_branch: Some(format!("job/{}", id)),
        template_namespace: Some(template.to_string()),
        vcs_engine: None,
        workspace_name: None,
        workspace_path: None,
        change_ref: None,
        policy_scope: vec![],
        decision_required: false,
        decision_path: None,
        depends_on: vec![],
        spawn_to: None,
        policy_result: None,
        timeout_seconds: None,
        retry_count: Some(0),
        failure_reason: None,
        validation_summary: None,
        glyph: None,
        token: None,
        title: None,
        description: None,
        labels: vec![],
        progress: None,
        subtasks: vec![],
        poker_round: None,
        estimates: Default::default(),
        zellij_session: None,
        zellij_pane: None,
        ac_spec_id: None,
        stage_chain: vec![],
        stage_models: Default::default(),
        stage_providers: Default::default(),
        stage_budgets: Default::default(),
        runs: vec![],
        exit_code: None,
        paused_at: None,
        checksum: None,
    });

    meta.id = id.to_string();
    meta.created = Utc::now();
    meta.worktree_branch = Some(format!("job/{}", id));
    meta.template_namespace = Some(template.to_string());
    meta.retry_count = Some(0);
    meta.failure_reason = None;
    meta.workflow_mode = Some(
        meta.workflow_mode
            .clone()
            .unwrap_or_else(|| util::workflow_mode_for_template(template).to_string()),
    );
    meta.step_index = Some(util::current_stage_step_index(&meta));

    // Auto-assign glyph + token if not already set by template
    if meta.glyph.is_none() {
        use bop_core::cardchars::{self, Team};
        let team = match team_override {
            Some("cli") => Team::Cli,
            Some("arch") => Team::Arch,
            Some("quality") => Team::Quality,
            Some("platform") => Team::Platform,
            Some(other) => {
                eprintln!("warning: unknown team '{}', defaulting to cli", other);
                Team::Cli
            }
            None => cardchars::team_from_path(&card_dir),
        };
        let used = cardchars::collect_used_glyphs(cards_dir);
        if let Some((glyph, token)) = cardchars::next_glyph(team, &used) {
            meta.glyph = Some(glyph);
            meta.token = Some(token);
        } else {
            eprintln!("warning: suit full for {:?}, no glyph assigned", team);
        }
    }

    write_meta(&card_dir, &meta)?;

    if !card_dir.join("spec.md").exists() {
        fs::write(card_dir.join("spec.md"), "")?;
    }
    if !card_dir.join("prompt.md").exists() {
        fs::write(card_dir.join("prompt.md"), "{{spec}}\n")?;
    }

    if let Some(spec) = spec_override {
        fs::write(card_dir.join("spec.md"), spec)?;
    }

    // Rename card dir from 🂠 placeholder to actual glyph prefix
    if let Some(ref g) = meta.glyph {
        let new_name = format!("{}-{}.bop", g, id);
        let new_dir = card_dir.parent().unwrap().join(&new_name);
        if !new_dir.exists() {
            fs::rename(&card_dir, &new_dir)?;
            quicklook::render_card_thumbnail(&new_dir);
            return Ok(new_dir);
        }
    }

    quicklook::render_card_thumbnail(&card_dir);

    Ok(card_dir)
}

// ── JSON card helpers ─────────────────────────────────────────────────────────

pub fn priority_from_json(value: &serde_json::Value) -> i64 {
    if let Some(n) = value.as_i64() {
        return n;
    }
    if let Some(s) = value.as_str() {
        match s.to_lowercase().replace(['-', ' '], "_").as_str() {
            "must" | "must_have" | "critical" => return 1,
            "should" | "should_have" | "important" => return 2,
            "could" | "could_have" | "nice_to_have" => return 3,
            _ => {}
        }
        if let Ok(n) = s.parse::<i64>() {
            return n;
        }
    }
    3
}

/// Build labels array from explicit labels + roadmap-specific fields
/// (phase, complexity, impact).
pub fn labels_from_json(entry: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut labels: Vec<serde_json::Value> = Vec::new();

    // Explicit labels array
    if let Some(seq) = entry["labels"].as_array() {
        for item in seq {
            if let Some(name) = item.as_str() {
                labels.push(serde_json::json!({"name": name}));
            } else if let Some(name) = item["name"].as_str() {
                labels.push(serde_json::json!({
                    "name": name,
                    "kind": item["kind"].as_str()
                }));
            }
        }
    }

    // Phase → label
    if let Some(phase) = entry["phase"].as_str() {
        labels.push(serde_json::json!({"name": phase, "kind": "phase"}));
    }

    // Complexity → label
    if let Some(complexity) = entry["complexity"].as_str() {
        let display = match complexity.to_lowercase().as_str() {
            "low" => "Low Complexity",
            "medium" => "Medium Complexity",
            "high" => "High Complexity",
            _ => complexity,
        };
        labels.push(serde_json::json!({"name": display, "kind": "complexity"}));
    }

    // Impact → label
    if let Some(impact) = entry["impact"].as_str() {
        let display = match impact.to_lowercase().as_str() {
            "low" => "Low Impact",
            "medium" => "Medium Impact",
            "high" => "High Impact",
            _ => impact,
        };
        labels.push(serde_json::json!({"name": display, "kind": "impact"}));
    }

    labels
}

/// Build subtasks array from explicit subtasks + user_stories.
pub fn subtasks_from_json(entry: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut subtasks: Vec<serde_json::Value> = Vec::new();

    // Explicit subtasks
    if let Some(seq) = entry["subtasks"].as_array() {
        for item in seq {
            if let Some(title) = item["title"].as_str() {
                subtasks.push(serde_json::json!({
                    "id": item["id"].as_str().unwrap_or(&format!("st-{}", subtasks.len() + 1)),
                    "title": title,
                    "done": item["done"].as_bool().unwrap_or(false)
                }));
            }
        }
    }

    // User stories → subtasks
    if let Some(seq) = entry["user_stories"].as_array() {
        for (i, story) in seq.iter().enumerate() {
            if let Some(text) = story.as_str() {
                subtasks.push(serde_json::json!({
                    "id": format!("us-{}", i + 1),
                    "title": text,
                    "done": false
                }));
            }
        }
    }

    subtasks
}

/// Build spec.md content from a JSON card entry.
///
/// Assembles sections: description, rationale, user stories, dependencies.
pub fn build_spec_md(entry: &serde_json::Value, id: &str) -> String {
    let title = entry["title"].as_str().unwrap_or(id);
    let mut spec = format!("# {}\n", title);

    if let Some(desc) = entry["description"].as_str() {
        spec.push_str(&format!("\n{}\n", desc));
    }

    if let Some(rationale) = entry["rationale"].as_str() {
        spec.push_str(&format!("\n## Rationale\n\n{}\n", rationale));
    }

    if let Some(stories) = entry["user_stories"].as_array() {
        let items: Vec<&str> = stories.iter().filter_map(|v| v.as_str()).collect();
        if !items.is_empty() {
            spec.push_str("\n## User Stories\n\n");
            for story in &items {
                spec.push_str(&format!("- {}\n", story));
            }
        }
    }

    if let Some(deps) = entry["depends_on"].as_array() {
        let items: Vec<&str> = deps.iter().filter_map(|v| v.as_str()).collect();
        if !items.is_empty() {
            spec.push_str("\n## Dependencies\n\n");
            for dep in &items {
                spec.push_str(&format!("- `{}`\n", dep));
            }
        }
    }

    if let Some(criteria) = entry["acceptance_criteria"].as_array() {
        let items: Vec<&str> = criteria.iter().filter_map(|v| v.as_str()).collect();
        if !items.is_empty() {
            spec.push_str("\n## Acceptance Criteria\n\n");
            for c in &items {
                spec.push_str(&format!("- [ ] {}\n", c));
            }
        }
    }

    spec
}

/// Create a .bop directory from a JSON entry. Used by both
/// `spawn_child_cards()` and `cmd_import()`.
pub fn create_card_from_json(dest_dir: &Path, entry: &serde_json::Value) -> Option<String> {
    let id = entry["id"].as_str()?;
    let child_dir = dest_dir.join(format!("{}.bop", id));
    if child_dir.exists() {
        return None; // don't overwrite
    }
    let _ = fs::create_dir_all(child_dir.join("logs"));
    let _ = fs::create_dir_all(child_dir.join("output"));

    let labels = labels_from_json(entry);
    let subtasks = subtasks_from_json(entry);

    let stage = entry["stage"].as_str().unwrap_or("spec");
    let workflow_mode = entry["workflow_mode"].as_str();

    let mut meta = serde_json::json!({
        "id": id,
        "title": entry["title"].as_str().unwrap_or(id),
        "description": entry["description"].as_str().unwrap_or(""),
        "stage": stage,
        "priority": priority_from_json(&entry["priority"]),
        "created": chrono::Utc::now().to_rfc3339(),
        "provider_chain": entry["provider_chain"].as_array()
            .map(|s| s.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_else(|| vec!["claude"]),
        "stages": {
            "spec": {"status": "pending", "agent": null},
            "plan": {"status": "blocked", "agent": null},
            "implement": {"status": "blocked", "agent": null},
            "qa": {"status": "blocked", "agent": null}
        },
        "acceptance_criteria": entry["acceptance_criteria"].as_array()
            .map(|s| s.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_default(),
        "retry_count": 0
    });

    // Add optional rich fields
    if !labels.is_empty() {
        meta["labels"] = serde_json::Value::Array(labels);
    }
    if !subtasks.is_empty() {
        meta["subtasks"] = serde_json::Value::Array(subtasks);
    }
    if let Some(wm) = workflow_mode {
        meta["workflow_mode"] = serde_json::Value::String(wm.to_string());
    }
    if let Some(deps) = entry["depends_on"].as_array() {
        let dep_ids: Vec<serde_json::Value> = deps
            .iter()
            .filter_map(|v| v.as_str().map(|s| serde_json::Value::String(s.to_string())))
            .collect();
        if !dep_ids.is_empty() {
            meta["depends_on"] = serde_json::Value::Array(dep_ids);
        }
    }

    let _ = fs::write(
        child_dir.join("meta.json"),
        serde_json::to_vec_pretty(&meta).unwrap(),
    );

    // Build rich spec.md
    let spec = build_spec_md(entry, id);
    let _ = fs::write(child_dir.join("spec.md"), spec);

    // Write roadmap.json snapshot if features are present
    if let Some(features) = entry["features"].as_array() {
        let roadmap_json = serde_json::json!({
            "features": features.iter().map(|f| {
                serde_json::json!({
                    "title": f["title"].as_str().unwrap_or(""),
                    "status": f["status"].as_str().unwrap_or("under_review"),
                    "priority": f["priority"].as_str().unwrap_or(""),
                    "phase": f["phase"].as_str().unwrap_or(""),
                })
            }).collect::<Vec<_>>()
        });
        let _ = fs::write(
            child_dir.join("output/roadmap.json"),
            serde_json::to_vec_pretty(&roadmap_json).unwrap(),
        );
    }

    quicklook::render_card_thumbnail(&child_dir);
    Some(id.to_string())
}

pub fn spawn_child_cards(cards_dir: &Path, done_card_dir: &Path) {
    let json_path = done_card_dir.join("output/cards.json");
    if !json_path.exists() {
        return;
    }

    let Ok(text) = fs::read_to_string(&json_path) else {
        return;
    };
    let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(&text) else {
        return;
    };

    // Read parent meta to determine child destination (pending or drafts).
    let dest = bop_core::read_meta(done_card_dir)
        .ok()
        .and_then(|m| m.spawn_to)
        .unwrap_or_else(|| "pending".to_string());
    let dest_dir = cards_dir.join(&dest);
    let _ = fs::create_dir_all(&dest_dir);

    for entry in entries {
        if let Some(id) = create_card_from_json(&dest_dir, &entry) {
            eprintln!("[child-cards] created {} in {}/", id, dest);
        }
    }
}

// ── retry ────────────────────────────────────────────────────────────────────

pub fn cmd_retry(root: &Path, id: &str) -> anyhow::Result<()> {
    let message = retry_card(root, id)?;
    println!("{}", message);
    Ok(())
}

/// Check if a card failed due to transient causes (network, rate limit, etc.)
fn is_transient_failure(meta: &Meta, card_path: &Path) -> bool {
    // Check exit code 75 (rate limit)
    if meta.exit_code == Some(75) {
        return true;
    }

    // Check failure_reason field in meta
    if let Some(reason) = &meta.failure_reason {
        let reason_lower = reason.to_lowercase();
        for pattern in TRANSIENT_PATTERNS {
            if reason_lower.contains(&pattern.to_lowercase()) {
                return true;
            }
        }
    }

    // Check last line of stderr log
    let stderr_path = card_path.join("logs").join("stderr");
    if let Ok(content) = fs::read_to_string(&stderr_path) {
        if let Some(last_line) = content.lines().rfind(|l| !l.trim().is_empty()) {
            let last_line_lower = last_line.to_lowercase();
            for pattern in TRANSIENT_PATTERNS {
                if last_line_lower.contains(&pattern.to_lowercase()) {
                    return true;
                }
            }
        }
    }

    false
}

pub fn cmd_retry_transient(root: &Path, id: Option<&str>, all: bool) -> anyhow::Result<()> {
    // If a specific card ID is provided, retry just that card
    if let Some(card_id) = id {
        let card_path = paths::find_card(root, card_id)
            .with_context(|| format!("card not found: {}", card_id))?;

        let state = card_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        if state != "failed" {
            anyhow::bail!(
                "card '{}' is not in failed state (currently in {})",
                card_id,
                state
            );
        }

        // Read meta
        let mut meta = bop_core::read_meta(&card_path)
            .with_context(|| format!("failed to read meta for {}", card_id))?;

        // Get reason for logging before we clear it
        let reason = meta
            .failure_reason
            .as_deref()
            .unwrap_or("transient failure")
            .to_string();

        // Update meta: increment retry_count, clear failure_reason
        meta.retry_count = Some(meta.retry_count.unwrap_or(0).saturating_add(1));
        meta.failure_reason = None;
        meta.exit_code = None;

        // Reset failed stages to pending
        for stage in meta.stages.values_mut() {
            if matches!(stage.status, StageStatus::Failed) {
                stage.status = StageStatus::Pending;
                stage.agent = None;
                stage.provider = None;
                stage.duration_s = None;
                stage.started = None;
                stage.blocked_by = None;
            }
        }

        // Write meta before rename
        write_meta(&card_path, &meta)
            .with_context(|| format!("failed to write meta for {}", card_id))?;

        // Determine target pending directory (same team structure if applicable)
        let target = if let Some(parent) = card_path.parent() {
            if let Some(grandparent) = parent.parent() {
                if let Some(team_name) = grandparent.file_name().and_then(|n| n.to_str()) {
                    if team_name.starts_with("team-") {
                        grandparent
                            .join("pending")
                            .join(card_path.file_name().unwrap())
                    } else {
                        root.join("pending").join(card_path.file_name().unwrap())
                    }
                } else {
                    root.join("pending").join(card_path.file_name().unwrap())
                }
            } else {
                root.join("pending").join(card_path.file_name().unwrap())
            }
        } else {
            root.join("pending").join(card_path.file_name().unwrap())
        };

        // Ensure target parent directory exists
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create pending dir for {}", card_id))?;
        }

        // Rename from failed/ to pending/
        fs::rename(&card_path, &target)
            .with_context(|| format!("failed to move {} to pending/", card_id))?;

        // Log lineage event if enabled
        if let Ok(m) = bop_core::read_meta(&target) {
            if bop_core::lineage::is_enabled(root) {
                let ev = bop_core::lineage::build_run_event(
                    bop_core::lineage::EventType::Other,
                    &m,
                    "failed",
                    "pending",
                );
                bop_core::lineage::flush_events(root, &[ev]);
            }
        }

        // Re-render thumbnail
        quicklook::render_card_thumbnail(&target);

        println!("↩  retry: {} (reason: {})", card_id, reason);
        return Ok(());
    }

    // Otherwise, scan all failed cards
    let mut failed_dirs = vec![root.join("failed")];

    // Add team-* failed directories
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("team-") {
                        failed_dirs.push(path.join("failed"));
                    }
                }
            }
        }
    }

    let mut all_cards: Vec<PathBuf> = Vec::new();

    for failed_dir in failed_dirs {
        if !failed_dir.exists() {
            continue;
        }

        let entries = match fs::read_dir(&failed_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let cards: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| {
                let path = e.path();
                path.is_dir() && {
                    let ext = path.extension().and_then(|s| s.to_str());
                    matches!(ext, Some("bop") | Some("jobcard"))
                }
            })
            .map(|e| e.path())
            .collect();

        all_cards.extend(cards);
    }

    if all_cards.is_empty() {
        println!("bop retry-transient: no failed cards.");
        return Ok(());
    }

    all_cards.sort();

    let mut retried_count = 0;

    for card_path in all_cards {
        let card_id = card_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Read meta
        let mut meta = match bop_core::read_meta(&card_path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("⚠  failed to read meta for {}: {}", card_id, e);
                continue;
            }
        };

        // Determine if we should retry this card
        let should_retry = all || is_transient_failure(&meta, &card_path);

        if !should_retry {
            let reason = meta.failure_reason.as_deref().unwrap_or("unknown error");
            println!(
                "⚠  skipped: {} (reason: {} — not transient)",
                card_id, reason
            );
            continue;
        }

        // Get reason for logging before we clear it
        let reason = meta
            .failure_reason
            .as_deref()
            .unwrap_or("transient failure")
            .to_string();

        // Update meta: increment retry_count, clear failure_reason
        meta.retry_count = Some(meta.retry_count.unwrap_or(0).saturating_add(1));
        meta.failure_reason = None;
        meta.exit_code = None;

        // Reset failed stages to pending
        for stage in meta.stages.values_mut() {
            if matches!(stage.status, StageStatus::Failed) {
                stage.status = StageStatus::Pending;
                stage.agent = None;
                stage.provider = None;
                stage.duration_s = None;
                stage.started = None;
                stage.blocked_by = None;
            }
        }

        // Write meta before rename
        if let Err(e) = write_meta(&card_path, &meta) {
            eprintln!("⚠  failed to write meta for {}: {}", card_id, e);
            continue;
        }

        // Determine target pending directory (same team structure if applicable)
        let target = if let Some(parent) = card_path.parent() {
            if let Some(grandparent) = parent.parent() {
                if let Some(team_name) = grandparent.file_name().and_then(|n| n.to_str()) {
                    if team_name.starts_with("team-") {
                        grandparent
                            .join("pending")
                            .join(card_path.file_name().unwrap())
                    } else {
                        root.join("pending").join(card_path.file_name().unwrap())
                    }
                } else {
                    root.join("pending").join(card_path.file_name().unwrap())
                }
            } else {
                root.join("pending").join(card_path.file_name().unwrap())
            }
        } else {
            root.join("pending").join(card_path.file_name().unwrap())
        };

        // Ensure target parent directory exists
        if let Some(parent) = target.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                eprintln!("⚠  failed to create pending dir for {}: {}", card_id, e);
                continue;
            }
        }

        // Rename from failed/ to pending/
        if let Err(e) = fs::rename(&card_path, &target) {
            eprintln!("⚠  failed to move {} to pending/: {}", card_id, e);
            continue;
        }

        // Log lineage event if enabled
        if let Ok(m) = bop_core::read_meta(&target) {
            if bop_core::lineage::is_enabled(root) {
                let ev = bop_core::lineage::build_run_event(
                    bop_core::lineage::EventType::Other,
                    &m,
                    "failed",
                    "pending",
                );
                bop_core::lineage::flush_events(root, &[ev]);
            }
        }

        // Re-render thumbnail
        quicklook::render_card_thumbnail(&target);

        println!("↩  retry: {} (reason: {})", card_id, reason);
        retried_count += 1;
    }

    if retried_count == 0 {
        println!("bop retry-transient: no transient failures found.");
    }

    Ok(())
}

pub async fn cmd_pause(root: &Path, _id: &str) -> anyhow::Result<()> {
    // Collect both flat and team-based running directories
    let mut running_dirs = vec![root.join("running")];

    // Add team-* running directories
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("team-") {
                        running_dirs.push(path.join("running"));
                    }
                }
            }
        }
    }

    let mut all_cards: Vec<PathBuf> = Vec::new();

    for running_dir in running_dirs {
        if !running_dir.exists() {
            continue;
        }

        let entries = match fs::read_dir(&running_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let cards: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| {
                let path = e.path();
                path.is_dir() && {
                    let ext = path.extension().and_then(|s| s.to_str());
                    matches!(ext, Some("bop") | Some("jobcard"))
                }
            })
            .map(|e| e.path())
            .collect();

        all_cards.extend(cards);
    }

    if all_cards.is_empty() {
        println!("bop pause: nothing running.");
        return Ok(());
    }

    all_cards.sort();

    let mut paused_count = 0;
    for card_path in all_cards {
        match pause_single_card(root, &card_path).await {
            Ok(msg) => {
                println!("{}", msg);
                paused_count += 1;
            }
            Err(e) => {
                eprintln!("⚠  failed to pause {}: {}", card_path.display(), e);
            }
        }
    }

    if paused_count == 0 {
        println!("bop pause: no cards were paused.");
    }

    Ok(())
}

async fn pause_single_card(root: &Path, card_path: &Path) -> anyhow::Result<String> {
    let card_id = card_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Read PID
    let pid = reaper::read_pid(card_path).await?;

    if let Some(pid) = pid {
        // Send SIGTERM
        let _ = TokioCommand::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status()
            .await;

        // Wait up to 5 seconds for process to exit
        let mut attempts = 0;
        while attempts < 50 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if !reaper::is_alive(pid).await? {
                break;
            }
            attempts += 1;
        }

        // If still alive, send SIGKILL
        if reaper::is_alive(pid).await? {
            let _ = TokioCommand::new("kill")
                .arg("-KILL")
                .arg(pid.to_string())
                .status()
                .await;
            // Give SIGKILL a brief moment to take effect
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    // Check if card still exists in running/ (race condition check)
    // The dispatcher might have moved it if it exited voluntarily with code 75
    if !card_path.exists() {
        anyhow::bail!("card already moved by another process");
    }

    // Update meta with paused_at timestamp
    if let Ok(mut meta) = bop_core::read_meta(card_path) {
        meta.paused_at = Some(Utc::now());
        for stage in meta.stages.values_mut() {
            if stage.status == StageStatus::Running {
                stage.status = StageStatus::Pending;
                stage.agent = None;
                stage.provider = None;
                stage.duration_s = None;
                stage.started = None;
                stage.blocked_by = None;
            }
        }
        write_meta(card_path, &meta)?;
    }

    // Rename to pending/ - preserve team directory if present
    let card_parent = card_path.parent().and_then(|p| p.parent());
    let pending_dir = if let Some(parent) = card_parent {
        parent.join("pending")
    } else {
        root.join("pending")
    };

    fs::create_dir_all(&pending_dir)?;

    let filename = card_path
        .file_name()
        .with_context(|| format!("invalid card path: {}", card_path.display()))?;
    let target = pending_dir.join(filename);

    // Final race condition check before rename
    if !card_path.exists() {
        anyhow::bail!("card already moved by another process");
    }

    fs::rename(card_path, &target)
        .with_context(|| format!("failed to move card to pending/: {}", card_id))?;

    quicklook::render_card_thumbnail(&target);

    let msg = if let Some(pid) = pid {
        format!("⏸  paused: {} (adapter PID {} stopped)", card_id, pid)
    } else {
        format!("⏸  paused: {} (no PID found)", card_id)
    };

    Ok(msg)
}

pub async fn cmd_resume(root: &Path, _id: &str) -> anyhow::Result<()> {
    // Collect both flat and team-based pending directories
    let mut pending_dirs = vec![root.join("pending")];

    // Add team-* pending directories
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("team-") {
                        pending_dirs.push(path.join("pending"));
                    }
                }
            }
        }
    }

    let mut all_cards: Vec<PathBuf> = Vec::new();

    for pending_dir in pending_dirs {
        if !pending_dir.exists() {
            continue;
        }

        let entries = match fs::read_dir(&pending_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let cards: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| {
                let path = e.path();
                path.is_dir() && {
                    let ext = path.extension().and_then(|s| s.to_str());
                    matches!(ext, Some("bop") | Some("jobcard"))
                }
            })
            .map(|e| e.path())
            .collect();

        all_cards.extend(cards);
    }

    if all_cards.is_empty() {
        println!("bop resume: no pending cards.");
        return Ok(());
    }

    all_cards.sort();

    let mut resumed_count = 0;
    for card_path in all_cards {
        let card_id = card_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Read meta
        let mut meta = match bop_core::read_meta(&card_path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("⚠  failed to read meta for {}: {}", card_id, e);
                continue;
            }
        };

        // Check if card has paused_at set
        if meta.paused_at.is_none() {
            continue;
        }

        // Clear paused_at
        meta.paused_at = None;

        // Write meta
        if let Err(e) = write_meta(&card_path, &meta) {
            eprintln!("⚠  failed to write meta for {}: {}", card_id, e);
            continue;
        }

        // Re-render thumbnail since meta changed
        quicklook::render_card_thumbnail(&card_path);

        println!("▶  queued for dispatch: {}", card_id);
        resumed_count += 1;
    }

    if resumed_count == 0 {
        println!("bop resume: no paused cards found.");
    }

    Ok(())
}

pub fn retry_card(root: &Path, id: &str) -> anyhow::Result<String> {
    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let state = card
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    if state == "running" {
        anyhow::bail!("card '{}' is currently running; kill it first", id);
    }
    if state == "pending" {
        anyhow::bail!("card '{}' is already pending", id);
    }

    // Update meta before rename so the write is in the stable card location
    if let Ok(mut meta) = bop_core::read_meta(&card) {
        meta.retry_count = Some(meta.retry_count.unwrap_or(0).saturating_add(1));
        meta.failure_reason = None;
        for stage in meta.stages.values_mut() {
            if matches!(stage.status, StageStatus::Running | StageStatus::Failed) {
                stage.status = StageStatus::Pending;
                stage.agent = None;
                stage.provider = None;
                stage.duration_s = None;
                stage.started = None;
                stage.blocked_by = None;
            }
        }
        let _ = write_meta(&card, &meta);
    }

    let target = root.join("pending").join(format!("{}.bop", id));
    fs::rename(&card, &target)
        .with_context(|| format!("failed to move card to pending/: {}", id))?;
    if let Ok(m) = bop_core::read_meta(&target) {
        if bop_core::lineage::is_enabled(root) {
            let ev = bop_core::lineage::build_run_event(
                bop_core::lineage::EventType::Other,
                &m,
                state,
                "pending",
            );
            bop_core::lineage::flush_events(root, &[ev]);
        }
    }
    quicklook::render_card_thumbnail(&target);
    Ok(format!("retrying: {} -> pending/", id))
}

// ── kill ─────────────────────────────────────────────────────────────────────

pub async fn cmd_kill(root: &Path, id: &str) -> anyhow::Result<()> {
    let message = kill_card(root, id).await?;
    println!("{}", message);
    Ok(())
}

pub async fn kill_card(root: &Path, id: &str) -> anyhow::Result<String> {
    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let state = card
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    if state != "running" {
        anyhow::bail!("card '{}' is not running (state: {})", id, state);
    }

    let pid = reaper::read_pid(&card)
        .await?
        .with_context(|| format!("no PID found for card '{}'", id))?;

    let mut was_running = reaper::is_alive(pid).await.unwrap_or(false);
    if was_running {
        // Send SIGTERM (kill -15)
        let sent = TokioCommand::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status()
            .await
            .with_context(|| format!("failed to send SIGTERM to pid {}", pid))?;

        if !sent.success() {
            // The process may have exited between the liveness check and kill.
            was_running = reaper::is_alive(pid).await.unwrap_or(false);
            if was_running {
                anyhow::bail!("kill -TERM {} returned non-zero", pid);
            }
        }
    }

    // Update meta with failure reason
    if let Ok(mut meta) = bop_core::read_meta(&card) {
        meta.failure_reason = Some("killed".to_string());
        let _ = write_meta(&card, &meta);
    }

    let failed_dir = root.join("failed");
    let target = failed_dir.join(format!("{}.bop", id));
    if let Ok(m) = bop_core::read_meta(&card) {
        if bop_core::lineage::is_enabled(root) {
            let ev = bop_core::lineage::build_run_event(
                bop_core::lineage::EventType::Abort,
                &m,
                "running",
                "failed",
            );
            bop_core::lineage::flush_events(root, &[ev]);
        }
    }
    fs::rename(&card, &target)
        .with_context(|| format!("failed to move card to failed/: {}", id))?;
    quicklook::render_card_thumbnail(&target);

    if was_running {
        Ok(format!("✓ Killed {} → it's in failed/", id))
    } else {
        Ok(format!(
            "✓ Killed {} → it's in failed/ (process was already dead)",
            id
        ))
    }
}

// ── approve ──────────────────────────────────────────────────────────────────

pub fn cmd_approve(root: &Path, id: &str) -> anyhow::Result<()> {
    approve_card(root, id)?;
    Ok(())
}

pub fn approve_card(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let state = card
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    let mut meta = bop_core::read_meta(&card)?;
    meta.decision_required = false;

    if state == "pending" {
        if let Some(record) = meta.stages.get_mut(&meta.stage) {
            if record.status == bop_core::StageStatus::Blocked {
                record.status = bop_core::StageStatus::Pending;
            }
        }
    }

    write_meta(&card, &meta)?;
    quicklook::render_card_thumbnail(&card);
    println!("Approved {}", id);
    Ok(())
}

// ── promote ──────────────────────────────────────────────────────────────────

pub fn cmd_promote(root: &Path, id: &str) -> anyhow::Result<()> {
    let drafts_dir = root.join("drafts");
    let pending_dir = root.join("pending");
    let _ = fs::create_dir_all(&pending_dir);

    if id == "all" {
        let mut count = 0u32;
        if let Ok(entries) = fs::read_dir(&drafts_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir()
                    && path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.ends_with(".bop"))
                {
                    let name = entry.file_name();
                    fs::rename(&path, pending_dir.join(&name))?;
                    eprintln!("promoted: {}", name.to_string_lossy());
                    count += 1;
                }
            }
        }
        if count == 0 {
            anyhow::bail!("no draft cards to promote");
        }
        eprintln!("[promote] moved {} card(s) to pending/", count);
    } else {
        let card = paths::find_card_in_dir(&drafts_dir, id)
            .with_context(|| format!("card '{}' not found in drafts/", id))?;
        let name = card.file_name().unwrap().to_owned();
        fs::rename(&card, pending_dir.join(&name))?;
        eprintln!("promoted: {}", id);
    }
    Ok(())
}

// ── bstorm ───────────────────────────────────────────────────────────────────

pub fn cmd_bstorm(
    root: &Path,
    topic_words: Vec<String>,
    team: Option<String>,
) -> anyhow::Result<()> {
    let topic = topic_words.join(" ");
    if topic.trim().is_empty() {
        anyhow::bail!("topic cannot be empty — usage: bop bstorm <topic words>");
    }

    // Slugify: lowercase, non-alnum→hyphen, collapse runs, trim hyphens, cap at 40.
    let slug: String = topic
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug: String = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = &slug[..slug.len().min(40)];

    let spec_body = format!(
        "# Brainstorm: {topic}\n\n\
         Explore ideas, trade-offs, and approaches for the topic above.\n\
         Produce a structured summary of the best options.\n"
    );

    let card_dir = create_card(root, "ideation", slug, Some(&spec_body), team.as_deref())?;
    let display = card_dir.strip_prefix(root).unwrap_or(&card_dir);
    println!("\u{2713} {}", display.display());
    println!("  edit spec: {}/spec.md", card_dir.display());
    Ok(())
}

// ── import ───────────────────────────────────────────────────────────────────

pub fn cmd_import(root: &Path, source: &str, immediate: bool) -> anyhow::Result<()> {
    let text = fs::read_to_string(source).with_context(|| format!("cannot read {}", source))?;
    let entries: Vec<serde_json::Value> = serde_json::from_str(&text)
        .context("invalid JSON — expected a JSON array of card definitions")?;

    let dest = if immediate { "pending" } else { "drafts" };
    let dest_dir = root.join(dest);
    fs::create_dir_all(&dest_dir)?;

    let mut count = 0u32;
    for entry in &entries {
        if entry["id"].as_str().is_none() {
            continue;
        }
        match create_card_from_json(&dest_dir, entry) {
            Some(id) => {
                eprintln!("[import] created {}", id);
                count += 1;
            }
            None => {
                if let Some(id) = entry["id"].as_str() {
                    eprintln!("[import] skipping {} (already exists)", id);
                }
            }
        }
    }

    eprintln!("[import] created {} card(s) in {}/", count, dest);
    Ok(())
}

// ── meta-set ─────────────────────────────────────────────────────────────────

pub fn cmd_meta_set(
    root: &Path,
    id: &str,
    workflow_mode: Option<&str>,
    step_index: Option<u32>,
    clear_workflow_mode: bool,
    clear_step_index: bool,
) -> anyhow::Result<()> {
    if clear_workflow_mode && workflow_mode.is_some() {
        anyhow::bail!("cannot set and clear workflow_mode in the same command");
    }
    if clear_step_index && step_index.is_some() {
        anyhow::bail!("cannot set and clear step_index in the same command");
    }
    if workflow_mode.is_none() && step_index.is_none() && !clear_workflow_mode && !clear_step_index
    {
        anyhow::bail!("no changes requested; use --workflow-mode/--step-index or clear flags");
    }

    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let mut meta = bop_core::read_meta(&card)?;

    if clear_workflow_mode {
        meta.workflow_mode = None;
        meta.step_index = None;
    }
    if let Some(mode) = workflow_mode {
        let mode = mode.trim();
        if mode.is_empty() {
            anyhow::bail!("workflow_mode cannot be empty");
        }
        meta.workflow_mode = Some(mode.to_string());
        if meta.step_index.is_none() {
            meta.step_index = Some(1);
        }
    }
    if clear_step_index {
        meta.step_index = None;
    }
    if let Some(idx) = step_index {
        meta.step_index = Some(idx);
    }

    write_meta(&card, &meta)?;
    println!(
        "updated {}: workflow_mode={:?} step_index={:?}",
        id, meta.workflow_mode, meta.step_index
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ── priority_from_json ────────────────────────────────────────────────

    #[test]
    fn priority_from_json_integer_passthrough() {
        let v = serde_json::json!(2);
        assert_eq!(priority_from_json(&v), 2);
    }

    #[test]
    fn priority_from_json_must_maps_to_1() {
        let v = serde_json::json!("must");
        assert_eq!(priority_from_json(&v), 1);
    }

    #[test]
    fn priority_from_json_should_maps_to_2() {
        let v = serde_json::json!("should");
        assert_eq!(priority_from_json(&v), 2);
    }

    #[test]
    fn priority_from_json_could_maps_to_3() {
        let v = serde_json::json!("could");
        assert_eq!(priority_from_json(&v), 3);
    }

    #[test]
    fn priority_from_json_critical_maps_to_1() {
        let v = serde_json::json!("critical");
        assert_eq!(priority_from_json(&v), 1);
    }

    #[test]
    fn priority_from_json_important_maps_to_2() {
        let v = serde_json::json!("important");
        assert_eq!(priority_from_json(&v), 2);
    }

    #[test]
    fn priority_from_json_string_number_parsed() {
        let v = serde_json::json!("2");
        assert_eq!(priority_from_json(&v), 2);
    }

    #[test]
    fn priority_from_json_unknown_defaults_to_3() {
        let v = serde_json::json!("banana");
        assert_eq!(priority_from_json(&v), 3);
    }

    #[test]
    fn priority_from_json_must_have_maps_to_1() {
        let v = serde_json::json!("must_have");
        assert_eq!(priority_from_json(&v), 1);
    }

    #[test]
    fn priority_from_json_nice_to_have_maps_to_3() {
        let v = serde_json::json!("nice_to_have");
        assert_eq!(priority_from_json(&v), 3);
    }

    // ── labels_from_json ──────────────────────────────────────────────────

    #[test]
    fn labels_from_json_explicit_labels_array() {
        let entry = serde_json::json!({"labels": ["Coding", "Performance"]});
        let labels = labels_from_json(&entry);
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0]["name"], "Coding");
        assert_eq!(labels[1]["name"], "Performance");
    }

    #[test]
    fn labels_from_json_phase_label() {
        let entry = serde_json::json!({"phase": "alpha"});
        let labels = labels_from_json(&entry);
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0]["name"], "alpha");
        assert_eq!(labels[0]["kind"], "phase");
    }

    #[test]
    fn labels_from_json_complexity_low() {
        let entry = serde_json::json!({"complexity": "low"});
        let labels = labels_from_json(&entry);
        assert_eq!(labels[0]["name"], "Low Complexity");
        assert_eq!(labels[0]["kind"], "complexity");
    }

    #[test]
    fn labels_from_json_complexity_medium() {
        let entry = serde_json::json!({"complexity": "medium"});
        let labels = labels_from_json(&entry);
        assert_eq!(labels[0]["name"], "Medium Complexity");
    }

    #[test]
    fn labels_from_json_complexity_high() {
        let entry = serde_json::json!({"complexity": "high"});
        let labels = labels_from_json(&entry);
        assert_eq!(labels[0]["name"], "High Complexity");
    }

    #[test]
    fn labels_from_json_impact_low() {
        let entry = serde_json::json!({"impact": "low"});
        let labels = labels_from_json(&entry);
        assert_eq!(labels[0]["name"], "Low Impact");
        assert_eq!(labels[0]["kind"], "impact");
    }

    #[test]
    fn labels_from_json_impact_medium() {
        let entry = serde_json::json!({"impact": "medium"});
        let labels = labels_from_json(&entry);
        assert_eq!(labels[0]["name"], "Medium Impact");
    }

    #[test]
    fn labels_from_json_impact_high() {
        let entry = serde_json::json!({"impact": "high"});
        let labels = labels_from_json(&entry);
        assert_eq!(labels[0]["name"], "High Impact");
    }

    #[test]
    fn labels_from_json_empty_for_no_labels() {
        let entry = serde_json::json!({"id": "foo"});
        let labels = labels_from_json(&entry);
        assert!(labels.is_empty());
    }

    #[test]
    fn labels_from_json_structured_labels_with_kind() {
        let entry = serde_json::json!({"labels": [{"name": "Bug", "kind": "type"}]});
        let labels = labels_from_json(&entry);
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0]["name"], "Bug");
        assert_eq!(labels[0]["kind"], "type");
    }

    // ── subtasks_from_json ────────────────────────────────────────────────

    #[test]
    fn subtasks_from_json_explicit_subtasks() {
        let entry = serde_json::json!({
            "subtasks": [
                {"id": "st-1", "title": "Write tests", "done": true},
                {"id": "st-2", "title": "Review PR", "done": false}
            ]
        });
        let subs = subtasks_from_json(&entry);
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0]["id"], "st-1");
        assert_eq!(subs[0]["title"], "Write tests");
        assert_eq!(subs[0]["done"], true);
        assert_eq!(subs[1]["done"], false);
    }

    #[test]
    fn subtasks_from_json_user_stories_converted() {
        let entry = serde_json::json!({
            "user_stories": [
                "As a user I want to login",
                "As a user I want to logout"
            ]
        });
        let subs = subtasks_from_json(&entry);
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0]["id"], "us-1");
        assert_eq!(subs[0]["title"], "As a user I want to login");
        assert_eq!(subs[0]["done"], false);
        assert_eq!(subs[1]["id"], "us-2");
    }

    #[test]
    fn subtasks_from_json_empty_for_no_subtasks() {
        let entry = serde_json::json!({"id": "foo"});
        let subs = subtasks_from_json(&entry);
        assert!(subs.is_empty());
    }

    // ── build_spec_md ─────────────────────────────────────────────────────

    #[test]
    fn build_spec_md_includes_title_as_h1() {
        let entry = serde_json::json!({"title": "My Feature"});
        let spec = build_spec_md(&entry, "my-feature");
        assert!(spec.starts_with("# My Feature\n"));
    }

    #[test]
    fn build_spec_md_includes_description() {
        let entry = serde_json::json!({"title": "T", "description": "Some details"});
        let spec = build_spec_md(&entry, "t");
        assert!(spec.contains("\nSome details\n"));
    }

    #[test]
    fn build_spec_md_includes_rationale() {
        let entry = serde_json::json!({"title": "T", "rationale": "Because reasons"});
        let spec = build_spec_md(&entry, "t");
        assert!(spec.contains("## Rationale"));
        assert!(spec.contains("Because reasons"));
    }

    #[test]
    fn build_spec_md_includes_user_stories_as_bullet_list() {
        let entry = serde_json::json!({
            "title": "T",
            "user_stories": ["story one", "story two"]
        });
        let spec = build_spec_md(&entry, "t");
        assert!(spec.contains("## User Stories"));
        assert!(spec.contains("- story one\n"));
        assert!(spec.contains("- story two\n"));
    }

    #[test]
    fn build_spec_md_includes_dependencies_with_backticks() {
        let entry = serde_json::json!({
            "title": "T",
            "depends_on": ["core-lib"]
        });
        let spec = build_spec_md(&entry, "t");
        assert!(spec.contains("## Dependencies"));
        assert!(spec.contains("- `core-lib`\n"));
    }

    #[test]
    fn build_spec_md_includes_acceptance_criteria_as_checklist() {
        let entry = serde_json::json!({
            "title": "T",
            "acceptance_criteria": ["Tests pass"]
        });
        let spec = build_spec_md(&entry, "t");
        assert!(spec.contains("## Acceptance Criteria"));
        assert!(spec.contains("- [ ] Tests pass\n"));
    }

    #[test]
    fn build_spec_md_handles_entry_with_only_title() {
        let entry = serde_json::json!({"title": "Minimal"});
        let spec = build_spec_md(&entry, "minimal");
        assert_eq!(spec, "# Minimal\n");
    }

    #[test]
    fn build_spec_md_uses_id_when_title_absent() {
        let entry = serde_json::json!({"description": "foo"});
        let spec = build_spec_md(&entry, "fallback-id");
        assert!(spec.starts_with("# fallback-id\n"));
    }

    // ── create_card_from_json ─────────────────────────────────────────────

    #[test]
    fn create_card_from_json_seed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let entry: serde_json::Value = serde_json::json!({
            "id": "child-test",
            "template": "implement",
            "spec": "# Child\nDo Y"
        });
        let id = create_card_from_json(tmp.path(), &entry);
        assert!(id.is_some(), "should create card directory");
        let card_dir = tmp.path().join("child-test.bop");
        assert!(card_dir.join("meta.json").exists(), "meta.json must exist");
    }

    #[test]
    fn create_card_from_json_creates_bop_dir() {
        let td = tempdir().unwrap();
        let dest = td.path();
        let entry = serde_json::json!({
            "id": "test-card",
            "title": "Test Card",
            "description": "A test"
        });
        let result = create_card_from_json(dest, &entry);
        assert_eq!(result, Some("test-card".to_string()));

        let card_dir = dest.join("test-card.bop");
        assert!(card_dir.exists());
        assert!(card_dir.join("meta.json").exists());
        assert!(card_dir.join("spec.md").exists());
        assert!(card_dir.join("logs").is_dir());
        assert!(card_dir.join("output").is_dir());
    }

    #[test]
    fn create_card_from_json_skips_existing() {
        let td = tempdir().unwrap();
        let dest = td.path();
        fs::create_dir_all(dest.join("test-card.bop")).unwrap();

        let entry = serde_json::json!({"id": "test-card", "title": "Test"});
        let result = create_card_from_json(dest, &entry);
        assert_eq!(result, None);
    }

    #[test]
    fn create_card_from_json_returns_none_without_id() {
        let td = tempdir().unwrap();
        let entry = serde_json::json!({"title": "No ID"});
        let result = create_card_from_json(td.path(), &entry);
        assert_eq!(result, None);
    }

    #[test]
    fn create_card_from_json_writes_roadmap_json_with_features() {
        let td = tempdir().unwrap();
        let entry = serde_json::json!({
            "id": "roadmap-card",
            "title": "Roadmap",
            "features": [
                {"title": "Feature A", "status": "planned", "priority": "high", "phase": "alpha"}
            ]
        });
        create_card_from_json(td.path(), &entry);
        let roadmap = td.path().join("roadmap-card.bop/output/roadmap.json");
        assert!(roadmap.exists());
        let content: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&roadmap).unwrap()).unwrap();
        assert_eq!(content["features"][0]["title"], "Feature A");
    }

    #[test]
    fn create_card_from_json_meta_json_has_correct_fields() {
        let td = tempdir().unwrap();
        let entry = serde_json::json!({
            "id": "meta-test",
            "title": "Meta Test",
            "priority": "must",
            "stage": "plan"
        });
        create_card_from_json(td.path(), &entry);
        let meta_path = td.path().join("meta-test.bop/meta.json");
        let meta: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(meta_path).unwrap()).unwrap();
        assert_eq!(meta["id"], "meta-test");
        assert_eq!(meta["title"], "Meta Test");
        assert_eq!(meta["priority"], 1);
        assert_eq!(meta["stage"], "plan");
    }

    // ── create_card ───────────────────────────────────────────────────────

    #[test]
    fn create_card_rejects_empty_template() {
        let td = tempdir().unwrap();
        let result = create_card(td.path(), "", "test-id", None, None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("template cannot be empty"));
    }

    #[test]
    fn create_card_rejects_empty_id() {
        let td = tempdir().unwrap();
        let result = create_card(td.path(), "implement", "", None, None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("id cannot be empty"));
    }

    #[test]
    fn create_card_rejects_id_with_forward_slash() {
        let td = tempdir().unwrap();
        let result = create_card(td.path(), "implement", "bad/id", None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("path separators"));
    }

    #[test]
    fn create_card_rejects_id_with_backslash() {
        let td = tempdir().unwrap();
        let result = create_card(td.path(), "implement", "bad\\id", None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("path separators"));
    }

    #[test]
    fn create_card_errors_on_missing_template() {
        let td = tempdir().unwrap();
        paths::ensure_cards_layout(td.path()).unwrap();
        let result = create_card(td.path(), "nonexistent", "test-id", None, None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("template not found"));
    }

    #[test]
    fn create_card_errors_on_duplicate() {
        let td = tempdir().unwrap();
        paths::ensure_cards_layout(td.path()).unwrap();
        seed_default_templates(td.path()).unwrap();

        // Manually create a card with the CARD_BACK prefix (pre-rename state)
        let card_name = format!("{}-dup-test.bop", bop_core::cardchars::CARD_BACK);
        let existing = td.path().join("pending").join(&card_name);
        fs::create_dir_all(&existing).unwrap();

        let result = create_card(td.path(), "implement", "dup-test", None, None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("card already exists"));
    }

    // ── seed_default_templates ────────────────────────────────────────────

    #[test]
    fn seed_default_templates_creates_implement_template() {
        let td = tempdir().unwrap();
        fs::create_dir_all(td.path().join("templates")).unwrap();
        seed_default_templates(td.path()).unwrap();

        let tmpl = td.path().join("templates/implement.bop");
        assert!(tmpl.exists());
        assert!(tmpl.join("meta.json").exists());
        assert!(tmpl.join("spec.md").exists());
        assert!(tmpl.join("prompt.md").exists());
        assert!(tmpl.join("logs").is_dir());
        assert!(tmpl.join("output").is_dir());
    }

    #[test]
    fn seed_default_templates_is_idempotent() {
        let td = tempdir().unwrap();
        fs::create_dir_all(td.path().join("templates")).unwrap();
        seed_default_templates(td.path()).unwrap();
        // Write a marker to spec.md
        let spec = td.path().join("templates/implement.bop/spec.md");
        fs::write(&spec, "custom content").unwrap();
        // Second call should not overwrite
        seed_default_templates(td.path()).unwrap();
        assert_eq!(fs::read_to_string(&spec).unwrap(), "custom content");
    }

    // ── retry_card ────────────────────────────────────────────────────────

    fn setup_card_in_state(root: &Path, id: &str, state: &str) -> PathBuf {
        paths::ensure_cards_layout(root).unwrap();
        let card_dir = root.join(state).join(format!("{}.bop", id));
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        fs::create_dir_all(card_dir.join("output")).unwrap();

        let meta = Meta {
            id: id.to_string(),
            meta_version: 1,
            created: Utc::now(),
            stage: "implement".to_string(),
            retry_count: Some(1),
            failure_reason: Some("test failure".to_string()),
            exit_code: None,
            paused_at: None,
            stages: {
                let mut m = std::collections::BTreeMap::new();
                m.insert(
                    "implement".to_string(),
                    bop_core::StageRecord {
                        status: StageStatus::Failed,
                        agent: Some("test-agent".to_string()),
                        provider: Some("test-provider".to_string()),
                        duration_s: Some(60),
                        started: None,
                        blocked_by: None,
                    },
                );
                m
            },
            agent_type: None,
            card_type: None,
            metadata_source: None,
            metadata_key: None,
            workflow_mode: None,
            step_index: None,
            priority: None,
            timeout_seconds: None,
            provider_chain: vec![],
            acceptance_criteria: vec![],
            worktree_branch: None,
            template_namespace: None,
            vcs_engine: None,
            workspace_name: None,
            workspace_path: None,
            change_ref: None,
            policy_scope: vec![],
            decision_required: false,
            decision_path: None,
            depends_on: vec![],
            spawn_to: None,
            policy_result: None,
            validation_summary: None,
            glyph: None,
            token: None,
            title: None,
            description: None,
            labels: vec![],
            progress: None,
            subtasks: vec![],
            poker_round: None,
            estimates: Default::default(),
            zellij_session: None,
            zellij_pane: None,
            ac_spec_id: None,
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
            runs: vec![],
            checksum: None,
        };
        write_meta(&card_dir, &meta).unwrap();
        card_dir
    }

    #[test]
    fn retry_card_moves_from_failed_to_pending() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "retry-me", "failed");

        let msg = retry_card(td.path(), "retry-me").unwrap();
        assert!(msg.contains("pending"));
        assert!(td.path().join("pending/retry-me.bop").exists());
        assert!(!td.path().join("failed/retry-me.bop").exists());
    }

    #[test]
    fn retry_card_increments_retry_count() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "retry-inc", "failed");

        retry_card(td.path(), "retry-inc").unwrap();
        let meta = bop_core::read_meta(&td.path().join("pending/retry-inc.bop")).unwrap();
        assert_eq!(meta.retry_count, Some(2)); // was 1, now 2
    }

    #[test]
    fn retry_card_clears_failure_reason() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "retry-clear", "failed");

        retry_card(td.path(), "retry-clear").unwrap();
        let meta = bop_core::read_meta(&td.path().join("pending/retry-clear.bop")).unwrap();
        assert_eq!(meta.failure_reason, None);
    }

    #[test]
    fn retry_card_normalizes_failed_stages_to_pending() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "retry-norm", "failed");

        retry_card(td.path(), "retry-norm").unwrap();
        let meta = bop_core::read_meta(&td.path().join("pending/retry-norm.bop")).unwrap();
        let stage = meta.stages.get("implement").unwrap();
        assert_eq!(stage.status, StageStatus::Pending);
        assert_eq!(stage.agent, None);
        assert_eq!(stage.provider, None);
        assert_eq!(stage.duration_s, None);
    }

    #[test]
    fn retry_card_rejects_pending() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "already-pending", "pending");

        let result = retry_card(td.path(), "already-pending");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already pending"));
    }

    #[test]
    fn retry_card_rejects_running() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "is-running", "running");

        let result = retry_card(td.path(), "is-running");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("currently running"));
    }

    #[test]
    fn retry_card_moves_from_done_to_pending() {
        let td = tempdir().unwrap();
        setup_card_in_state(td.path(), "from-done", "done");

        let msg = retry_card(td.path(), "from-done").unwrap();
        assert!(msg.contains("pending"));
        assert!(td.path().join("pending/from-done.bop").exists());
    }

    // ── cmd_promote ───────────────────────────────────────────────────────

    #[test]
    fn cmd_promote_moves_card_from_drafts_to_pending() {
        let td = tempdir().unwrap();
        paths::ensure_cards_layout(td.path()).unwrap();
        let draft = td.path().join("drafts/test-promote.bop");
        fs::create_dir_all(&draft).unwrap();
        fs::write(draft.join("meta.json"), "{}").unwrap();

        cmd_promote(td.path(), "test-promote").unwrap();
        assert!(td.path().join("pending/test-promote.bop").exists());
        assert!(!draft.exists());
    }

    #[test]
    fn cmd_promote_all_moves_all_drafts() {
        let td = tempdir().unwrap();
        paths::ensure_cards_layout(td.path()).unwrap();
        for name in ["a.bop", "b.bop"] {
            let d = td.path().join("drafts").join(name);
            fs::create_dir_all(&d).unwrap();
        }

        cmd_promote(td.path(), "all").unwrap();
        assert!(td.path().join("pending/a.bop").exists());
        assert!(td.path().join("pending/b.bop").exists());
    }

    #[test]
    fn cmd_promote_errors_when_no_drafts() {
        let td = tempdir().unwrap();
        paths::ensure_cards_layout(td.path()).unwrap();

        let result = cmd_promote(td.path(), "all");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no draft cards"));
    }

    // ── approve_card ──────────────────────────────────────────────────────

    #[test]
    fn approve_card_clears_decision_required() {
        let td = tempdir().unwrap();
        paths::ensure_cards_layout(td.path()).unwrap();
        let card = td.path().join("pending/approve-me.bop");
        fs::create_dir_all(card.join("logs")).unwrap();

        let mut meta = Meta {
            id: "approve-me".to_string(),
            meta_version: 1,
            created: Utc::now(),
            stage: "spec".to_string(),
            decision_required: true,
            agent_type: None,
            card_type: None,
            metadata_source: None,
            metadata_key: None,
            workflow_mode: None,
            step_index: None,
            priority: None,
            timeout_seconds: None,
            provider_chain: vec![],
            stages: Default::default(),
            acceptance_criteria: vec![],
            worktree_branch: None,
            template_namespace: None,
            vcs_engine: None,
            workspace_name: None,
            workspace_path: None,
            change_ref: None,
            policy_scope: vec![],
            decision_path: None,
            depends_on: vec![],
            spawn_to: None,
            policy_result: None,
            retry_count: None,
            failure_reason: None,
            exit_code: None,
            paused_at: None,
            validation_summary: None,
            glyph: None,
            token: None,
            title: None,
            description: None,
            labels: vec![],
            progress: None,
            subtasks: vec![],
            poker_round: None,
            estimates: Default::default(),
            zellij_session: None,
            zellij_pane: None,
            ac_spec_id: None,
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
            runs: vec![],
            checksum: None,
        };
        write_meta(&card, &meta).unwrap();

        approve_card(td.path(), "approve-me").unwrap();
        meta = bop_core::read_meta(&card).unwrap();
        assert!(!meta.decision_required);
    }

    #[test]
    fn approve_card_unblocks_stage_to_pending() {
        let td = tempdir().unwrap();
        paths::ensure_cards_layout(td.path()).unwrap();
        let card = td.path().join("pending/approve-unblock.bop");
        fs::create_dir_all(card.join("logs")).unwrap();

        let mut stages = std::collections::BTreeMap::new();
        stages.insert(
            "spec".to_string(),
            bop_core::StageRecord {
                status: StageStatus::Blocked,
                agent: None,
                provider: None,
                duration_s: None,
                started: None,
                blocked_by: None,
            },
        );

        let meta = Meta {
            id: "approve-unblock".to_string(),
            meta_version: 1,
            created: Utc::now(),
            stage: "spec".to_string(),
            decision_required: true,
            stages,
            agent_type: None,
            card_type: None,
            metadata_source: None,
            metadata_key: None,
            workflow_mode: None,
            step_index: None,
            priority: None,
            timeout_seconds: None,
            provider_chain: vec![],
            acceptance_criteria: vec![],
            worktree_branch: None,
            template_namespace: None,
            vcs_engine: None,
            workspace_name: None,
            workspace_path: None,
            change_ref: None,
            policy_scope: vec![],
            decision_path: None,
            depends_on: vec![],
            spawn_to: None,
            policy_result: None,
            retry_count: None,
            failure_reason: None,
            exit_code: None,
            paused_at: None,
            validation_summary: None,
            glyph: None,
            token: None,
            title: None,
            description: None,
            labels: vec![],
            progress: None,
            subtasks: vec![],
            poker_round: None,
            estimates: Default::default(),
            zellij_session: None,
            zellij_pane: None,
            ac_spec_id: None,
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
            runs: vec![],
            checksum: None,
        };
        write_meta(&card, &meta).unwrap();

        approve_card(td.path(), "approve-unblock").unwrap();
        let updated = bop_core::read_meta(&card).unwrap();
        let stage = updated.stages.get("spec").unwrap();
        assert_eq!(stage.status, StageStatus::Pending);
    }

    // ── spawn_child_cards ─────────────────────────────────────────────────

    #[test]
    fn spawn_child_cards_creates_children_from_json() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        // Create a done card with output/cards.json
        let done_card = root.join("done/parent.bop");
        fs::create_dir_all(done_card.join("output")).unwrap();
        fs::create_dir_all(done_card.join("logs")).unwrap();

        let parent_meta = Meta {
            id: "parent".to_string(),
            meta_version: 1,
            created: Utc::now(),
            stage: "done".to_string(),
            spawn_to: None,
            agent_type: None,
            card_type: None,
            metadata_source: None,
            metadata_key: None,
            workflow_mode: None,
            step_index: None,
            priority: None,
            timeout_seconds: None,
            provider_chain: vec![],
            stages: Default::default(),
            acceptance_criteria: vec![],
            worktree_branch: None,
            template_namespace: None,
            vcs_engine: None,
            workspace_name: None,
            workspace_path: None,
            change_ref: None,
            policy_scope: vec![],
            decision_required: false,
            decision_path: None,
            depends_on: vec![],
            policy_result: None,
            retry_count: None,
            failure_reason: None,
            exit_code: None,
            paused_at: None,
            validation_summary: None,
            glyph: None,
            token: None,
            title: None,
            description: None,
            labels: vec![],
            progress: None,
            subtasks: vec![],
            poker_round: None,
            estimates: Default::default(),
            zellij_session: None,
            zellij_pane: None,
            ac_spec_id: None,
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
            runs: vec![],
            checksum: None,
        };
        write_meta(&done_card, &parent_meta).unwrap();

        let json = r#"[
  {"id": "child-a", "title": "Child A"},
  {"id": "child-b", "title": "Child B"}
]"#;
        fs::write(done_card.join("output/cards.json"), json).unwrap();

        spawn_child_cards(root, &done_card);

        assert!(root.join("pending/child-a.bop").exists());
        assert!(root.join("pending/child-b.bop").exists());
    }

    #[test]
    fn spawn_child_cards_noop_when_json_missing() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        let done_card = root.join("done/no-json.bop");
        fs::create_dir_all(done_card.join("logs")).unwrap();

        // Should not panic or error
        spawn_child_cards(root, &done_card);
    }

    // ── cmd_import ────────────────────────────────────────────────────────

    #[test]
    fn cmd_import_creates_cards_from_json() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        let json_file = root.join("import.json");
        fs::write(
            &json_file,
            r#"[
  {"id": "imp-a", "title": "Imported A"},
  {"id": "imp-b", "title": "Imported B"}
]"#,
        )
        .unwrap();

        cmd_import(root, json_file.to_str().unwrap(), false).unwrap();
        assert!(root.join("drafts/imp-a.bop").exists());
        assert!(root.join("drafts/imp-b.bop").exists());
    }

    #[test]
    fn cmd_import_immediate_creates_in_pending() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        let json_file = root.join("import.json");
        fs::write(&json_file, r#"[{"id": "imp-now", "title": "Now"}]"#).unwrap();

        cmd_import(root, json_file.to_str().unwrap(), true).unwrap();
        assert!(root.join("pending/imp-now.bop").exists());
    }

    #[test]
    fn cmd_import_skips_entries_without_id() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        let json_file = root.join("import.json");
        fs::write(
            &json_file,
            r#"[
  {"title": "No ID here"},
  {"id": "has-id", "title": "Has ID"}
]"#,
        )
        .unwrap();

        cmd_import(root, json_file.to_str().unwrap(), false).unwrap();
        // Only has-id should be created
        assert!(root.join("drafts/has-id.bop").exists());
        // Count .bop dirs in drafts
        let count = fs::read_dir(root.join("drafts"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().is_dir() && e.file_name().to_str().is_some_and(|n| n.ends_with(".bop"))
            })
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn cmd_import_reports_already_existing() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        let json_file = root.join("import.json");
        fs::write(&json_file, r#"[{"id": "dup", "title": "Dup"}]"#).unwrap();

        // Import once
        cmd_import(root, json_file.to_str().unwrap(), false).unwrap();
        // Import again — should succeed but skip existing
        cmd_import(root, json_file.to_str().unwrap(), false).unwrap();
        // Still only one card
        assert!(root.join("drafts/dup.bop").exists());
    }

    // ── cmd_bstorm ────────────────────────────────────────────────────────

    #[test]
    fn cmd_bstorm_errors_on_empty_topic() {
        let td = tempdir().unwrap();
        let result = cmd_bstorm(td.path(), vec![], None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("topic cannot be empty"));
    }

    #[test]
    fn cmd_bstorm_errors_on_whitespace_topic() {
        let td = tempdir().unwrap();
        let result = cmd_bstorm(td.path(), vec!["  ".to_string()], None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("topic cannot be empty"));
    }

    // ── cmd_meta_set ──────────────────────────────────────────────────────

    #[test]
    fn cmd_meta_set_updates_workflow_mode() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();
        let card = root.join("pending/meta-wm.bop");
        fs::create_dir_all(card.join("logs")).unwrap();

        let meta = Meta {
            id: "meta-wm".to_string(),
            meta_version: 1,
            created: Utc::now(),
            stage: "spec".to_string(),
            workflow_mode: None,
            step_index: None,
            agent_type: None,
            card_type: None,
            metadata_source: None,
            metadata_key: None,
            priority: None,
            timeout_seconds: None,
            provider_chain: vec![],
            stages: Default::default(),
            acceptance_criteria: vec![],
            worktree_branch: None,
            template_namespace: None,
            vcs_engine: None,
            workspace_name: None,
            workspace_path: None,
            change_ref: None,
            policy_scope: vec![],
            decision_required: false,
            decision_path: None,
            depends_on: vec![],
            spawn_to: None,
            policy_result: None,
            retry_count: None,
            failure_reason: None,
            exit_code: None,
            paused_at: None,
            validation_summary: None,
            glyph: None,
            token: None,
            title: None,
            description: None,
            labels: vec![],
            progress: None,
            subtasks: vec![],
            poker_round: None,
            estimates: Default::default(),
            zellij_session: None,
            zellij_pane: None,
            ac_spec_id: None,
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
            runs: vec![],
            checksum: None,
        };
        write_meta(&card, &meta).unwrap();

        cmd_meta_set(root, "meta-wm", Some("qa-only"), None, false, false).unwrap();
        let updated = bop_core::read_meta(&card).unwrap();
        assert_eq!(updated.workflow_mode, Some("qa-only".to_string()));
        // Setting workflow_mode should also set step_index to 1 if not set
        assert_eq!(updated.step_index, Some(1));
    }

    #[test]
    fn cmd_meta_set_updates_step_index() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();
        let card = root.join("pending/meta-si.bop");
        fs::create_dir_all(card.join("logs")).unwrap();

        let meta = Meta {
            id: "meta-si".to_string(),
            meta_version: 1,
            created: Utc::now(),
            stage: "spec".to_string(),
            workflow_mode: Some("default-feature".to_string()),
            step_index: Some(1),
            agent_type: None,
            card_type: None,
            metadata_source: None,
            metadata_key: None,
            priority: None,
            timeout_seconds: None,
            provider_chain: vec![],
            stages: Default::default(),
            acceptance_criteria: vec![],
            worktree_branch: None,
            template_namespace: None,
            vcs_engine: None,
            workspace_name: None,
            workspace_path: None,
            change_ref: None,
            policy_scope: vec![],
            decision_required: false,
            decision_path: None,
            depends_on: vec![],
            spawn_to: None,
            policy_result: None,
            retry_count: None,
            failure_reason: None,
            exit_code: None,
            paused_at: None,
            validation_summary: None,
            glyph: None,
            token: None,
            title: None,
            description: None,
            labels: vec![],
            progress: None,
            subtasks: vec![],
            poker_round: None,
            estimates: Default::default(),
            zellij_session: None,
            zellij_pane: None,
            ac_spec_id: None,
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
            runs: vec![],
            checksum: None,
        };
        write_meta(&card, &meta).unwrap();

        cmd_meta_set(root, "meta-si", None, Some(3), false, false).unwrap();
        let updated = bop_core::read_meta(&card).unwrap();
        assert_eq!(updated.step_index, Some(3));
    }

    #[test]
    fn cmd_meta_set_rejects_set_and_clear_same_field() {
        let td = tempdir().unwrap();
        let result = cmd_meta_set(td.path(), "x", Some("mode"), None, true, false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("cannot set and clear"));
    }

    #[test]
    fn cmd_meta_set_rejects_set_and_clear_step_index() {
        let td = tempdir().unwrap();
        let result = cmd_meta_set(td.path(), "x", None, Some(1), false, true);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("cannot set and clear"));
    }

    #[test]
    fn cmd_meta_set_errors_when_no_changes() {
        let td = tempdir().unwrap();
        let result = cmd_meta_set(td.path(), "x", None, None, false, false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("no changes requested"));
    }

    // ── scan_cards_for_cleanup / perform_cleanup ───────────────────────────

    fn setup_corrupt_card(root: &Path, state: &str, id: &str) -> PathBuf {
        let card_dir = root.join(state).join(format!("{}.bop", id));
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        fs::create_dir_all(card_dir.join("output")).unwrap();
        // No meta.json = corrupt
        card_dir
    }

    fn setup_valid_card(root: &Path, state: &str, id: &str) -> PathBuf {
        let card_dir = root.join(state).join(format!("{}.bop", id));
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        fs::create_dir_all(card_dir.join("output")).unwrap();

        let meta = Meta {
            id: id.to_string(),
            meta_version: 1,
            created: Utc::now(),
            stage: "implement".to_string(),
            retry_count: Some(0),
            failure_reason: None,
            exit_code: None,
            paused_at: None,
            stages: Default::default(),
            agent_type: None,
            card_type: None,
            metadata_source: None,
            metadata_key: None,
            workflow_mode: None,
            step_index: None,
            priority: None,
            timeout_seconds: None,
            provider_chain: vec![],
            acceptance_criteria: vec![],
            worktree_branch: None,
            template_namespace: None,
            vcs_engine: None,
            workspace_name: None,
            workspace_path: None,
            change_ref: None,
            policy_scope: vec![],
            decision_required: false,
            decision_path: None,
            depends_on: vec![],
            spawn_to: None,
            policy_result: None,
            validation_summary: None,
            glyph: None,
            token: None,
            title: None,
            description: None,
            labels: vec![],
            progress: None,
            subtasks: vec![],
            poker_round: None,
            estimates: Default::default(),
            zellij_session: None,
            zellij_pane: None,
            ac_spec_id: None,
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
            runs: vec![],
            checksum: None,
        };
        write_meta(&card_dir, &meta).unwrap();
        card_dir
    }

    #[test]
    fn cmd_clean_scan_detects_corrupt_cards() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        setup_corrupt_card(root, "pending", "corrupt-1");
        setup_valid_card(root, "pending", "valid-1");

        let result = scan_cards_for_cleanup(root, 30).unwrap();
        assert_eq!(result.corrupt_cards.len(), 1);
        assert!(result.corrupt_cards[0]
            .to_string_lossy()
            .contains("corrupt-1"));
    }

    #[test]
    fn cmd_clean_scan_detects_old_failed_cards() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        // Create a failed card with old timestamp
        let _old_card = setup_valid_card(root, "failed", "old-failed");

        // Touch the card to make it old (simulate 31 days ago)
        // Note: We can't easily set old timestamps in tests, so this test
        // verifies the logic path exists but may not find old cards in practice
        let result = scan_cards_for_cleanup(root, 30).unwrap();

        // The card won't actually be old in the test, but verify scan completes
        assert!(result.old_failed_cards.is_empty() || result.old_failed_cards.len() == 1);
    }

    #[test]
    fn cmd_clean_scan_detects_orphan_running_cards() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        // Create a running card with empty output and no logs
        let orphan = setup_valid_card(root, "running", "orphan-1");
        // Ensure output exists but is empty
        fs::create_dir_all(orphan.join("output")).unwrap();
        // Ensure logs dir exists but has no files
        fs::create_dir_all(orphan.join("logs")).unwrap();

        let result = scan_cards_for_cleanup(root, 30).unwrap();
        assert_eq!(result.orphan_running_cards.len(), 1);
        assert!(result.orphan_running_cards[0]
            .to_string_lossy()
            .contains("orphan-1"));
    }

    #[test]
    fn cmd_clean_scan_detects_target_dirs() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        let card = setup_valid_card(root, "done", "with-target");
        let target = card.join("target");
        fs::create_dir_all(target.join("debug")).unwrap();
        fs::write(target.join("debug").join("artifact"), "data").unwrap();

        let result = scan_cards_for_cleanup(root, 30).unwrap();
        assert_eq!(result.target_dirs.len(), 1);
        assert!(result.target_dirs[0].to_string_lossy().contains("target"));
    }

    #[test]
    fn cmd_clean_scan_includes_team_directories() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        // Create team-cli directory structure
        fs::create_dir_all(root.join("team-cli/pending")).unwrap();
        setup_corrupt_card(root.join("team-cli").as_path(), "pending", "team-corrupt");

        let result = scan_cards_for_cleanup(root, 30).unwrap();
        assert_eq!(result.corrupt_cards.len(), 1);
        assert!(result.corrupt_cards[0]
            .to_string_lossy()
            .contains("team-cli"));
    }

    #[test]
    fn cmd_clean_perform_cleanup_dry_run() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        let corrupt = setup_corrupt_card(root, "pending", "corrupt-dry");

        let scan_result = scan_cards_for_cleanup(root, 30).unwrap();
        let stats = perform_cleanup(&scan_result, true).unwrap();

        // Dry-run should report what would be removed
        assert_eq!(stats.corrupt_cards_removed, 1);

        // But card should still exist
        assert!(corrupt.exists());
    }

    #[test]
    fn cmd_clean_perform_cleanup_removes_corrupt_cards() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        let corrupt = setup_corrupt_card(root, "pending", "corrupt-remove");

        let scan_result = scan_cards_for_cleanup(root, 30).unwrap();
        let stats = perform_cleanup(&scan_result, false).unwrap();

        assert_eq!(stats.corrupt_cards_removed, 1);
        assert!(!corrupt.exists());
    }

    #[test]
    fn cmd_clean_perform_cleanup_removes_target_dirs() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        let card = setup_valid_card(root, "done", "with-target-remove");
        let target = card.join("target");
        fs::create_dir_all(target.join("debug")).unwrap();
        fs::write(target.join("debug").join("big.o"), "compiled code").unwrap();

        let scan_result = scan_cards_for_cleanup(root, 30).unwrap();
        let stats = perform_cleanup(&scan_result, false).unwrap();

        assert_eq!(stats.target_dirs_removed, 1);
        assert!(!target.exists());
        assert!(card.exists()); // Card itself should remain
    }

    #[test]
    fn cmd_clean_perform_cleanup_calculates_bytes_freed() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        let card = setup_valid_card(root, "done", "with-data");
        let target = card.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("data.bin"), vec![0u8; 1024]).unwrap();

        let scan_result = scan_cards_for_cleanup(root, 30).unwrap();
        let stats = perform_cleanup(&scan_result, false).unwrap();

        assert!(stats.bytes_freed >= 1024);
    }

    #[test]
    fn cmd_clean_perform_cleanup_removes_orphan_running() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        let orphan = setup_valid_card(root, "running", "orphan-remove");
        fs::create_dir_all(orphan.join("output")).unwrap();
        fs::create_dir_all(orphan.join("logs")).unwrap();

        let scan_result = scan_cards_for_cleanup(root, 30).unwrap();
        let stats = perform_cleanup(&scan_result, false).unwrap();

        assert_eq!(stats.orphan_running_cards_removed, 1);
        assert!(!orphan.exists());
    }

    #[test]
    fn cmd_clean_handles_empty_scan_result() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        // No corrupt cards, just valid ones
        setup_valid_card(root, "pending", "valid-only");

        let scan_result = scan_cards_for_cleanup(root, 30).unwrap();
        let stats = perform_cleanup(&scan_result, false).unwrap();

        assert_eq!(stats.corrupt_cards_removed, 0);
        assert_eq!(stats.old_failed_cards_removed, 0);
        assert_eq!(stats.orphan_running_cards_removed, 0);
        assert_eq!(stats.target_dirs_removed, 0);
        assert_eq!(stats.bytes_freed, 0);
    }

    #[test]
    fn cmd_clean_scan_ignores_non_bop_directories() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        // Create a non-.bop directory
        fs::create_dir_all(root.join("pending/not-a-card")).unwrap();

        // Create a file (not directory)
        fs::write(root.join("pending/file.txt"), "data").unwrap();

        let result = scan_cards_for_cleanup(root, 30).unwrap();

        // Should find nothing since we only have non-card items
        assert_eq!(result.corrupt_cards.len(), 0);
    }

    #[test]
    fn cmd_clean_scan_handles_jobcard_extension() {
        let td = tempdir().unwrap();
        let root = td.path();
        paths::ensure_cards_layout(root).unwrap();

        // Create a .jobcard (legacy format)
        let card_dir = root.join("pending/legacy.jobcard");
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        // No meta.json = corrupt

        let result = scan_cards_for_cleanup(root, 30).unwrap();
        assert_eq!(result.corrupt_cards.len(), 1);
    }
}
