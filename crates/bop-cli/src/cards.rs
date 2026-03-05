use anyhow::Context;
use bop_core::{write_meta, Meta, StageStatus};
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use tokio::process::Command as TokioCommand;

use crate::{paths, quicklook, reaper, util};

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
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
            runs: vec![],
        };
        write_meta(&implement, &meta)?;

        if !implement.join("spec.md").exists() {
            fs::write(implement.join("spec.md"), "")?;
        }
        if !implement.join("prompt.md").exists() {
            fs::write(
                implement.join("prompt.md"),
                "{{spec}}\n\nProject memory:\n{{memory}}\n\nAcceptance criteria:\n{{acceptance_criteria}}\n",
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
        stage_chain: vec![],
        stage_models: Default::default(),
        stage_providers: Default::default(),
        stage_budgets: Default::default(),
        runs: vec![],
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
        Ok(format!("killed pid {} and moved '{}' to failed/", pid, id))
    } else {
        Ok(format!(
            "pid {} was not alive; moved '{}' to failed as stale running card",
            pid, id
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
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
            runs: vec![],
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
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
            runs: vec![],
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
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
            runs: vec![],
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
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
            runs: vec![],
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
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
            runs: vec![],
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
            stage_chain: vec![],
            stage_models: Default::default(),
            stage_providers: Default::default(),
            stage_budgets: Default::default(),
            runs: vec![],
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
}
