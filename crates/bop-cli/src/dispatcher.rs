use anyhow::Context;
use chrono::Utc;
use bop_core::{write_meta, Meta, RunRecord};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::process::Command as TokioCommand;

use super::VcsEngine;
use crate::{cards, inspect, lock, memory, paths, providers, quicklook, reaper, util, workspace};

#[allow(clippy::too_many_arguments)]
pub async fn run_dispatcher(
    cards_dir: &Path,
    vcs_engine: VcsEngine,
    adapter: &str,
    max_workers: usize,
    poll_ms: u64,
    max_retries: u32,
    reap_ms: u64,
    no_reap: bool,
    once: bool,
    validation_fail_threshold: f64,
) -> anyhow::Result<()> {
    paths::ensure_cards_layout(cards_dir)?;
    providers::seed_providers(cards_dir)?;
    providers::ensure_mock_provider_command(cards_dir, adapter)?;
    let _dispatcher_lock = lock::acquire_dispatcher_lock(cards_dir)?;

    let pending_dir = cards_dir.join("pending");
    let running_dir = cards_dir.join("running");
    let done_dir = cards_dir.join("done");
    let failed_dir = cards_dir.join("failed");
    let stale_lease_after = std::cmp::max(
        lock::LEASE_STALE_FLOOR,
        Duration::from_millis(reap_ms.saturating_mul(3)),
    );

    let mut last_reap = std::time::Instant::now()
        .checked_sub(Duration::from_millis(reap_ms))
        .unwrap_or_else(std::time::Instant::now);

    let lineage_enabled = bop_core::lineage::is_enabled(cards_dir);

    loop {
        let mut lineage_events: Vec<bop_core::lineage::RunEvent> = Vec::new();
        let mut record =
            |meta: &bop_core::Meta, from: &str, to: &str, card_dir: Option<&Path>| {
                if lineage_enabled {
                    let et = bop_core::lineage::event_type_for(from, to);
                    lineage_events.push(bop_core::lineage::build_run_event_with_dir(
                        et, meta, from, to, card_dir,
                    ));
                }
                // Write iCalendar VTODO projection into the card bundle
                if let Some(dir) = card_dir {
                    bop_core::lineage::write_ics(dir, meta, to);
                }
            };

        if !no_reap && last_reap.elapsed() >= Duration::from_millis(reap_ms) {
            reaper::reap_orphans(
                &running_dir,
                &pending_dir,
                &failed_dir,
                max_retries,
                stale_lease_after,
            )
            .await?;
            last_reap = std::time::Instant::now();
        }

        let running_count = fs::read_dir(&running_dir)
            .map(|rd| {
                rd.filter(|e| {
                    e.as_ref()
                        .ok()
                        .and_then(|e| e.path().extension().map(|x| x == "bop"))
                        .unwrap_or(false)
                })
                .count()
            })
            .unwrap_or(0);
        let mut available_slots = max_workers.saturating_sub(running_count);

        if available_slots > 0 {
            if let Ok(entries) = fs::read_dir(&pending_dir) {
                for ent in entries.flatten() {
                    if available_slots == 0 {
                        break;
                    }

                    let pending_path = ent.path();
                    if !pending_path.is_dir() {
                        continue;
                    }
                    if pending_path
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        != "bop"
                    {
                        continue;
                    }

                    let name = match pending_path.file_name().and_then(|s| s.to_str()) {
                        Some(n) => n.to_string(),
                        None => continue,
                    };

                    let mut meta = match bop_core::read_meta(&pending_path) {
                        Ok(m) => Some(m),
                        Err(err) => {
                            let reason = format!("invalid_meta: {err}");
                            eprintln!(
                                "[dispatcher] rejecting invalid pending card {}: {}",
                                name, err
                            );
                            let _ = paths::quarantine_invalid_pending_card(
                                &pending_path,
                                &failed_dir,
                                &reason,
                            );
                            continue;
                        }
                    };

                    // ── pre-dispatch gates ──────────────────────────────────
                    // Read meta before moving to running/ so we can skip
                    // cards that aren't ready yet.
                    if let Some(ref pre_meta) = meta {
                        // Gate 1: decision_required — needs human approval first
                        if pre_meta.decision_required {
                            eprintln!("[dispatcher] skipping {} — decision_required", pre_meta.id);
                            continue;
                        }
                        // Gate 2: depends_on — all deps must be in done/ or merged/
                        if !pre_meta.depends_on.is_empty() {
                            let blocked = pre_meta.depends_on.iter().any(|dep_id| {
                                !paths::card_exists_in(cards_dir, "done", dep_id)
                                    && !paths::card_exists_in(cards_dir, "merged", dep_id)
                            });
                            if blocked {
                                eprintln!(
                                    "[dispatcher] skipping {} — unmet depends_on: {:?}",
                                    pre_meta.id, pre_meta.depends_on
                                );
                                continue;
                            }
                        }
                    }

                    let running_path = running_dir.join(&name);
                    // Count before rename so the current card is not included in the tally
                    let active = workspace::count_running_cards(cards_dir);
                    if fs::rename(&pending_path, &running_path).is_err() {
                        continue;
                    }
                    if let Some(ref m) = meta {
                        record(m, "pending", "running", Some(&running_path));
                    }
                    quicklook::render_card_thumbnail(&running_path);

                    let card_id = meta
                        .as_ref()
                        .map(|m| m.id.clone())
                        .unwrap_or_else(|| name.trim_end_matches(".bop").to_string());
                    let ws_info = match workspace::prepare_workspace(
                        vcs_engine,
                        cards_dir,
                        &running_path,
                        &card_id,
                        &mut meta,
                    ) {
                        Ok(info) => info,
                        Err(err) => {
                            eprintln!("[dispatcher] workspace prepare failed: {err}");
                            if let Some(ref mut m) = meta {
                                m.failure_reason = Some(format!("workspace_prepare_failed: {err}"));
                                let _ = write_meta(&running_path, m);
                            }
                            let failed_path = failed_dir.join(&name);
                            let _ = fs::rename(&running_path, &failed_path);
                            if let Some(ref m) = meta {
                                record(m, "running", "failed", Some(&failed_path));
                            }
                            quicklook::render_card_thumbnail(&failed_path);
                            continue;
                        }
                    };
                    workspace::persist_workspace_meta(
                        &mut meta,
                        &running_path,
                        vcs_engine,
                        ws_info.as_ref(),
                    );

                    // Assign deterministic zellij session name
                    if let Some(ref mut m) = meta {
                        if m.zellij_session.is_none() {
                            m.zellij_session = Some(format!("bop-{}", card_id));
                            let _ = write_meta(&running_path, m);
                        }
                    }

                    // Adaptive zellij pane management
                    if workspace::is_zellij_interactive() {
                        match active {
                            0..=5 => workspace::zellij_open_card_pane(&name, &running_path),
                            6..=20 => { /* team pane already open per layout */ }
                            _ => { /* tier 3: status bar only */ }
                        }
                    }

                    available_slots = available_slots.saturating_sub(1);

                    let stage = meta
                        .as_ref()
                        .map(|m| m.stage.clone())
                        .unwrap_or_else(|| "implement".to_string());

                    let (
                        provider_name,
                        provider_cmd,
                        rate_limit_exit,
                        provider_env,
                        provider_model,
                    ) = match providers::select_provider(cards_dir, meta.as_mut(), &stage)? {
                        Some(v) => v,
                        None => {
                            let pending_path = pending_dir.join(&name);
                            let _ = fs::rename(&running_path, &pending_path);
                            if let Some(ref m) = meta {
                                record(m, "running", "pending", Some(&pending_path));
                            }
                            quicklook::render_card_thumbnail(&pending_path);
                            continue;
                        }
                    };

                    if let Some(ref mut meta) = meta {
                        let _ = write_meta(&running_path, meta);
                    }

                    let (exit_code, mut meta) = run_card(
                        cards_dir,
                        &running_path,
                        &provider_cmd,
                        &provider_name,
                        &provider_env,
                        provider_model.as_deref(),
                        rate_limit_exit,
                    )
                    .await
                    .unwrap_or((1, None));

                    let is_rate_limited = exit_code == rate_limit_exit;

                    // Run realtime validation on job output when the job succeeded.
                    let mut validation_triggered_fail = false;
                    if exit_code == 0 {
                        if let Ok(summary) = validate_realtime_output(&running_path) {
                            let error_rate = if summary.total == 0 {
                                0.0
                            } else {
                                summary.invalid as f64 / summary.total as f64
                            };
                            if summary.critical_alerts > 0 && error_rate > validation_fail_threshold
                            {
                                validation_triggered_fail = true;
                                if let Some(ref mut meta) = meta {
                                    meta.failure_reason =
                                        Some("validation_threshold_exceeded".to_string());
                                }
                            }
                            if let Some(ref mut meta) = meta {
                                meta.validation_summary = Some(summary);
                            }
                        }
                    }

                    if let Some(ref mut meta) = meta {
                        if is_rate_limited {
                            let next = meta.retry_count.unwrap_or(0).saturating_add(1);
                            meta.retry_count = Some(next);

                            providers::rotate_provider_chain(meta);
                            let _ =
                                providers::set_provider_cooldown(cards_dir, &provider_name, 300);
                        }

                        let _ = write_meta(&running_path, meta);
                    }
                    let target = if validation_triggered_fail {
                        failed_dir.join(&name)
                    } else if exit_code == 0 {
                        done_dir.join(&name)
                    } else if is_rate_limited {
                        pending_dir.join(&name)
                    } else {
                        failed_dir.join(&name)
                    };

                    let _ = fs::rename(&running_path, &target);
                    if let Some(ref m) = meta {
                        let to_state = if validation_triggered_fail {
                            "failed"
                        } else if exit_code == 0 {
                            "done"
                        } else if is_rate_limited {
                            "pending"
                        } else {
                            "failed"
                        };
                        record(m, "running", to_state, Some(&target));
                    }
                    quicklook::render_card_thumbnail(&target);
                    quicklook::compress_card(&target);
                    if exit_code == 0 && !validation_triggered_fail {
                        maybe_advance_stage(cards_dir, &target);
                        cards::spawn_child_cards(cards_dir, &target);
                    }
                }
            }
        }

        // Flush collected lineage events (O(N) — one write per loop iteration)
        if !lineage_events.is_empty() {
            bop_core::lineage::flush_events(cards_dir, &lineage_events);
            lineage_events.clear();
        }

        if once {
            break;
        }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }

    Ok(())
}

// ── stage auto-progression ────────────────────────────────────────────────────

/// If the completed card has a `stage_chain` with a next stage, create a
/// next-stage card in `pending/` inheriting spec, glyph, pipeline config,
/// and prior stage output.
pub fn maybe_advance_stage(cards_dir: &Path, done_card_dir: &Path) {
    let Ok(meta) = bop_core::read_meta(done_card_dir) else {
        return;
    };
    if meta.stage_chain.is_empty() {
        return;
    }

    let current_idx = match meta.stage_chain.iter().position(|s| s == &meta.stage) {
        Some(i) => i,
        None => return,
    };

    // If this is the final stage, nothing to advance
    if current_idx + 1 >= meta.stage_chain.len() {
        return;
    }

    let next_stage = &meta.stage_chain[current_idx + 1];
    let next_id = format!("{}-{}", meta.id, next_stage);

    let pending_dir = cards_dir.join("pending");
    let _ = fs::create_dir_all(&pending_dir);
    let next_card_dir = pending_dir.join(format!("{}.bop", next_id));
    if next_card_dir.exists() {
        return; // don't overwrite existing card
    }

    let _ = fs::create_dir_all(next_card_dir.join("logs"));
    let _ = fs::create_dir_all(next_card_dir.join("output"));

    // Determine provider for next stage
    let next_provider = meta
        .stage_providers
        .get(next_stage)
        .cloned()
        .unwrap_or_else(|| meta.provider_chain.first().cloned().unwrap_or_default());
    let provider_chain = if next_provider.is_empty() {
        meta.provider_chain.clone()
    } else {
        // Put stage-specific provider first, rest as fallback
        let mut chain = vec![next_provider];
        for p in &meta.provider_chain {
            if !chain.contains(p) {
                chain.push(p.clone());
            }
        }
        chain
    };

    let next_meta = Meta {
        id: next_id.clone(),
        created: Utc::now(),
        stage: next_stage.clone(),
        workflow_mode: meta.workflow_mode.clone(),
        step_index: Some((current_idx + 2) as u32),
        glyph: meta.glyph.clone(),
        token: meta.token.clone(),
        title: meta.title.clone(),
        description: meta.description.clone(),
        labels: meta.labels.clone(),
        provider_chain,
        acceptance_criteria: meta.acceptance_criteria.clone(),
        worktree_branch: Some(format!("job/{}", next_id)),
        template_namespace: meta.template_namespace.clone(),
        stage_chain: meta.stage_chain.clone(),
        stage_models: meta.stage_models.clone(),
        stage_providers: meta.stage_providers.clone(),
        stage_budgets: meta.stage_budgets.clone(),
        timeout_seconds: meta.timeout_seconds,
        ..Default::default()
    };
    let _ = write_meta(&next_card_dir, &next_meta);

    // COW-copy spec.md from parent (APFS clone — zero disk cost until modified)
    let spec_src = done_card_dir.join("spec.md");
    if spec_src.exists() {
        if let Err(err) = paths::cow_copy_file(&spec_src, &next_card_dir.join("spec.md")) {
            eprintln!("[stage-advance] failed COW-copying spec.md: {err}");
        }
    }

    // COW-copy prompt.md template from parent
    let prompt_src = done_card_dir.join("prompt.md");
    if prompt_src.exists() {
        if let Err(err) = paths::cow_copy_file(&prompt_src, &next_card_dir.join("prompt.md")) {
            eprintln!("[stage-advance] failed COW-copying prompt.md: {err}");
        }
    }

    // Carry prior stage output: COW-copy done card's output/result.md → next card's output/prior_result.md
    let result_src = done_card_dir.join("output").join("result.md");
    if result_src.exists() {
        if let Err(err) = paths::cow_copy_file(
            &result_src,
            &next_card_dir.join("output").join("prior_result.md"),
        ) {
            eprintln!("[stage-advance] failed COW-copying output/prior_result.md: {err}");
        }
    }

    quicklook::render_card_thumbnail(&next_card_dir);
    eprintln!(
        "[stage-advance] {} ({}) → {} ({})",
        meta.id, meta.stage, next_id, next_stage
    );
}

pub async fn run_card(
    cards_dir: &Path,
    card_dir: &Path,
    adapter: &str,
    provider_name: &str,
    provider_env: &std::collections::BTreeMap<String, String>,
    provider_model: Option<&str>,
    rate_limit_exit: i32,
) -> anyhow::Result<(i32, Option<Meta>)> {
    fs::create_dir_all(card_dir.join("logs"))?;
    fs::create_dir_all(card_dir.join("output"))?;

    let prompt_file = card_dir.join("prompt.md");
    if !prompt_file.exists() {
        fs::write(&prompt_file, "")?;
    }

    let stdout_log = card_dir.join("logs").join("stdout.log");
    let stderr_log = card_dir.join("logs").join("stderr.log");
    let memory_out_file = card_dir.join("memory-out.json");
    let _ = fs::remove_file(&memory_out_file);

    // Render prompt template with actual values
    let mut meta = bop_core::read_meta(card_dir).ok();
    let memory_namespace = meta
        .as_ref()
        .map(memory::memory_namespace_from_meta)
        .unwrap_or_else(|| "default".to_string());
    if let Some(ref m) = meta {
        let mut ctx = bop_core::PromptContext::from_files(card_dir, m)?;
        match memory::read_memory_store(cards_dir, &memory_namespace) {
            Ok(store) => {
                ctx.memory = memory::format_memory_for_prompt(&store);
            }
            Err(err) => {
                let _ = util::append_log_line(
                    &stderr_log,
                    &format!(
                        "memory load failed (namespace={}): {}",
                        memory_namespace, err
                    ),
                );
            }
        }
        let template = fs::read_to_string(&prompt_file)?;
        let rendered = bop_core::render_prompt(&template, &ctx);
        fs::write(&prompt_file, rendered)?;
    }

    let workdir = {
        if let Some(ref m) = meta {
            if let Some(ref p) = m.workspace_path {
                let candidate = PathBuf::from(p);
                if candidate.exists() {
                    candidate
                } else {
                    let ws = card_dir.join("workspace");
                    if ws.exists() {
                        ws
                    } else {
                        card_dir.to_path_buf()
                    }
                }
            } else {
                let ws = card_dir.join("workspace");
                if ws.exists() {
                    ws
                } else {
                    card_dir.to_path_buf()
                }
            }
        } else {
            let ws = card_dir.join("workspace");
            if ws.exists() {
                ws
            } else {
                card_dir.to_path_buf()
            }
        }
    };

    let stage = meta
        .as_ref()
        .map(|m| m.stage.clone())
        .unwrap_or_else(|| "implement".to_string());
    let started_at = Utc::now();
    let started_at_iso = started_at.to_rfc3339();
    let run_id = short_run_id();
    let mut run_idx: Option<usize> = None;
    if let Some(ref mut m) = meta {
        let rec = m
            .stages
            .entry(stage.clone())
            .or_insert(bop_core::StageRecord {
                status: bop_core::StageStatus::Pending,
                agent: None,
                provider: None,
                duration_s: None,
                started: None,
                blocked_by: None,
            });
        rec.status = bop_core::StageStatus::Running;
        rec.started = Some(started_at);
        rec.agent = Some(adapter.to_string());
        rec.provider = Some(provider_name.to_string());

        let initial_model = provider_model
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| model_from_provider_env(provider_env))
            .unwrap_or_else(|| provider_name.to_string());
        m.runs.push(RunRecord {
            run_id: run_id.clone(),
            stage: stage.clone(),
            provider: provider_name.to_string(),
            model: initial_model,
            adapter: adapter.to_string(),
            started_at: started_at_iso,
            ended_at: None,
            outcome: "running".to_string(),
            prompt_tokens: None,
            completion_tokens: None,
            cost_usd: None,
            duration_s: None,
            note: None,
        });
        run_idx = Some(m.runs.len().saturating_sub(1));
        let _ = write_meta(card_dir, m);
    }

    let mut cmd = if adapter.ends_with(".nu") {
        let mut c = TokioCommand::new("nu");
        let adapter_path = if std::path::Path::new(adapter).is_absolute() {
            adapter.to_string()
        } else {
            format!("{}/{}", std::env::current_dir()?.display(), adapter)
        };
        c.arg(adapter_path);
        c
    } else {
        TokioCommand::new(adapter)
    };

    // Per-workspace target dir eliminates cargo lock contention across parallel agents
    let target_dir = workdir.join("target");

    let mut child = cmd
        .arg(&workdir)
        .arg(&prompt_file)
        .arg(&stdout_log)
        .arg(&stderr_log)
        .arg(&memory_out_file)
        .env("JOBCARD_MEMORY_OUT", &memory_out_file)
        .env("JOBCARD_MEMORY_NAMESPACE", &memory_namespace)
        .env("CARGO_TARGET_DIR", &target_dir)
        .envs(provider_env)
        // Card identity — lets any agent orient itself
        .env(
            "BOP_CARD_ID",
            meta.as_ref().map(|m| m.id.as_str()).unwrap_or(""),
        )
        .env("BOP_CARD_DIR", card_dir)
        .env("BOP_CARDS_DIR", cards_dir)
        .env("BOP_STAGE", &stage)
        .env("BOP_PROVIDER", provider_name)
        .env("BOP_TARGET_DIR", &target_dir)
        .spawn()
        .with_context(|| format!("failed to spawn adapter: {}", adapter))?;

    let timeout_seconds = meta
        .as_ref()
        .and_then(|m| m.timeout_seconds)
        .unwrap_or(3600);
    let pid = child
        .id()
        .map(|v| v as i32)
        .with_context(|| "spawned adapter without a child PID")?;
    let pid_str = pid.to_string();
    let _ = fs::write(card_dir.join("logs").join("pid"), &pid_str);
    let _ = TokioCommand::new("xattr")
        .arg("-w")
        .arg("com.yourorg.agent-pid")
        .arg(&pid_str)
        .arg(card_dir)
        .status()
        .await;

    let mut lease = lock::RunLease {
        run_id: util::next_run_id(child.id()),
        pid,
        pid_start_time: started_at,
        started_at,
        heartbeat_at: started_at,
        host: util::host_name(),
    };
    let _ = lock::write_run_lease(card_dir, &lease);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_seconds);
    let mut timed_out = false;
    let status = loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            timed_out = true;
            let _ = child.kill().await;
            break None;
        }
        let remaining = deadline.saturating_duration_since(now);
        let wait_slice = std::cmp::min(lock::LEASE_HEARTBEAT_INTERVAL, remaining);
        match tokio::time::timeout(wait_slice, child.wait()).await {
            Ok(res) => break Some(res?),
            Err(_) => {
                lease.heartbeat_at = Utc::now();
                let _ = lock::write_run_lease(card_dir, &lease);
            }
        }
    };
    let exit_code = if timed_out {
        124
    } else {
        status.and_then(|s| s.code()).unwrap_or(1)
    };

    if let Err(err) = memory::merge_memory_output(cards_dir, &memory_namespace, &memory_out_file) {
        let _ = util::append_log_line(
            &stderr_log,
            &format!(
                "memory merge failed (namespace={}): {}",
                memory_namespace, err
            ),
        );
    }

    let finished_at = Utc::now();
    let duration_s = finished_at
        .signed_duration_since(started_at)
        .num_seconds()
        .try_into()
        .ok();
    let usage = detect_run_usage(card_dir);
    if let Some(ref mut m) = meta {
        let rec = m.stages.entry(stage).or_insert(bop_core::StageRecord {
            status: bop_core::StageStatus::Pending,
            agent: None,
            provider: None,
            duration_s: None,
            started: None,
            blocked_by: None,
        });
        rec.status = if timed_out {
            bop_core::StageStatus::Failed
        } else if exit_code == 0 {
            bop_core::StageStatus::Done
        } else if exit_code == rate_limit_exit {
            bop_core::StageStatus::Pending
        } else {
            bop_core::StageStatus::Failed
        };
        rec.duration_s = duration_s;

        if let Some(idx) = run_idx {
            if let Some(run) = m.runs.get_mut(idx) {
                run.ended_at = Some(finished_at.to_rfc3339());
                run.duration_s = duration_s;
                run.outcome = if timed_out {
                    "timeout".to_string()
                } else if exit_code == 0 {
                    "success".to_string()
                } else if exit_code == rate_limit_exit {
                    "rate_limited".to_string()
                } else {
                    "failed".to_string()
                };
                if run.model.trim().is_empty() {
                    run.model = provider_name.to_string();
                }
                if let Some(run_usage) = usage.as_ref() {
                    if let Some(model) = run_usage.model.as_ref() {
                        run.model = model.clone();
                    }
                    run.prompt_tokens = run_usage.prompt_tokens;
                    run.completion_tokens = run_usage.completion_tokens;
                    run.cost_usd = run_usage.cost_usd;
                } else if let Some(model) = detect_model_from_logs(card_dir) {
                    run.model = model;
                }
                if timed_out {
                    run.note = Some(format!("timeout_seconds={}", timeout_seconds));
                }
            }
        }
    }

    Ok((exit_code, meta))
}

#[derive(Debug, Clone, Default)]
pub struct RunUsage {
    pub model: Option<String>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
}

pub fn short_run_id() -> String {
    let now = Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| Utc::now().timestamp_micros() * 1000) as u64;
    let pid = std::process::id() as u64;
    let seq = util::RUN_ID_SEQ.fetch_add(1, Ordering::Relaxed);
    let mixed = now ^ (pid << 16) ^ seq;
    format!("{:08x}", (mixed & 0xffff_ffff) as u32)
}

pub fn model_from_provider_env(env: &BTreeMap<String, String>) -> Option<String> {
    for key in ["OLLAMA_MODEL", "ANTHROPIC_MODEL", "OPENAI_MODEL", "MODEL"] {
        if let Some(value) = env.get(key).map(|v| v.trim()).filter(|v| !v.is_empty()) {
            return Some(value.to_string());
        }
    }
    env.iter().find_map(|(k, v)| {
        if k.to_ascii_uppercase().ends_with("_MODEL") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        None
    })
}

// inspect → inspect.rs
pub fn parse_usage_from_stdout(stdout_log: &Path) -> Option<RunUsage> {
    let value = inspect::parse_latest_json_line(stdout_log)?;
    let usage = value.get("usage");

    let mut prompt_tokens = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(|x| x.as_u64());
    let mut completion_tokens = usage
        .and_then(|u| u.get("output_tokens"))
        .and_then(|x| x.as_u64());

    let model = value
        .get("modelUsage")
        .and_then(|m| m.as_object())
        .and_then(|obj| obj.keys().next().cloned())
        .or_else(|| {
            value
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });

    if prompt_tokens.is_none() || completion_tokens.is_none() {
        if let Some(model_usage) = value.get("modelUsage").and_then(|m| m.as_object()) {
            if let Some((_model_name, stats)) = model_usage.iter().next() {
                if prompt_tokens.is_none() {
                    prompt_tokens = stats.get("inputTokens").and_then(|x| x.as_u64());
                }
                if completion_tokens.is_none() {
                    completion_tokens = stats.get("outputTokens").and_then(|x| x.as_u64());
                }
            }
        }
    }

    Some(RunUsage {
        model,
        prompt_tokens,
        completion_tokens,
        cost_usd: value.get("total_cost_usd").and_then(|x| x.as_f64()),
    })
}

pub fn parse_usage_from_ollama_stats(stats_log: &Path) -> Option<RunUsage> {
    let content = fs::read_to_string(stats_log).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    Some(RunUsage {
        model: value
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        prompt_tokens: value.get("prompt_tokens").and_then(|x| x.as_u64()),
        completion_tokens: value.get("completion_tokens").and_then(|x| x.as_u64()),
        cost_usd: None,
    })
}

pub fn detect_run_usage(card_dir: &Path) -> Option<RunUsage> {
    let logs_dir = card_dir.join("logs");
    let mut merged = parse_usage_from_stdout(&logs_dir.join("stdout.log"));

    if let Some(ollama) = parse_usage_from_ollama_stats(&logs_dir.join("ollama-stats.json")) {
        if let Some(current) = merged.as_mut() {
            if current.model.is_none() {
                current.model = ollama.model;
            }
            if current.prompt_tokens.is_none() {
                current.prompt_tokens = ollama.prompt_tokens;
            }
            if current.completion_tokens.is_none() {
                current.completion_tokens = ollama.completion_tokens;
            }
        } else {
            merged = Some(ollama);
        }
    }

    merged
}

pub fn detect_model_from_logs(card_dir: &Path) -> Option<String> {
    detect_run_usage(card_dir).and_then(|u| u.model)
}

// ── realtime validation ───────────────────────────────────────────────────────

/// Scan `output/*.json` in a job card directory, validate each file as a
/// [`FeedRecord`], write a structured audit log to `logs/validation.log`, and
/// return an aggregated [`ValidationSummary`].
///
/// If `feed_config.json` exists in the card directory it is used as the
/// [`FeedConfig`]; otherwise a permissive default is applied.
pub fn validate_realtime_output(
    card_dir: &Path,
) -> anyhow::Result<bop_core::realtime::ValidationSummary> {
    use bop_core::realtime::{
        check_alerts, validate_record, AlertSeverity, FeedMetrics, FeedRecord, ValidationSummary,
    };

    let output_dir = card_dir.join("output");
    let validation_log = card_dir.join("logs").join("validation.log");
    let config = load_feed_config(card_dir);

    let feed_id = card_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let mut metrics = FeedMetrics::new(feed_id);
    let mut error_entries: Vec<serde_json::Value> = Vec::new();

    if output_dir.exists() {
        for entry in fs::read_dir(&output_dir)?.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let record: FeedRecord = match serde_json::from_slice(&bytes) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let result = validate_record(&record, &config);
            if !result.valid {
                error_entries.push(serde_json::json!({
                    "file": path.file_name().and_then(|s| s.to_str()),
                    "feed_id": record.feed_id,
                    "errors": result.errors,
                }));
            }
            metrics.record_received(result.valid);
        }
    }

    let alerts = check_alerts(&metrics);
    let alert_count = alerts.len() as u64;
    let critical_alerts = alerts
        .iter()
        .filter(|a| a.severity == AlertSeverity::Critical)
        .count() as u64;

    let log_content = serde_json::json!({
        "total": metrics.records_received,
        "valid": metrics.records_valid,
        "invalid": metrics.records_invalid,
        "health": metrics.health,
        "alerts": alerts.iter().map(|a| serde_json::json!({
            "severity": a.severity,
            "message": a.message,
            "timestamp": a.timestamp,
        })).collect::<Vec<_>>(),
        "validation_errors": error_entries,
    });
    let _ = fs::create_dir_all(card_dir.join("logs"));
    let _ = fs::write(&validation_log, serde_json::to_vec_pretty(&log_content)?);

    Ok(ValidationSummary {
        total: metrics.records_received,
        valid: metrics.records_valid,
        invalid: metrics.records_invalid,
        alert_count,
        critical_alerts,
        health: metrics.health,
    })
}

/// Load a [`FeedConfig`] from `card_dir/feed_config.json`, or return a
/// permissive default that accepts any well-formed [`FeedRecord`].
pub fn load_feed_config(card_dir: &Path) -> bop_core::realtime::FeedConfig {
    use bop_core::realtime::{FeedConfig, FeedSourceType, ValidationConfig};

    let path = card_dir.join("feed_config.json");
    if let Ok(bytes) = fs::read(&path) {
        if let Ok(cfg) = serde_json::from_slice::<FeedConfig>(&bytes) {
            return cfg;
        }
    }

    let id = card_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("job")
        .to_string();

    FeedConfig {
        id,
        source_type: FeedSourceType::File,
        endpoint: card_dir.display().to_string(),
        poll_interval_secs: 0,
        validation: ValidationConfig {
            required_fields: vec!["feed_id".to_string()],
            // Large but not u64::MAX to avoid overflow in signed arithmetic.
            max_staleness_secs: 60 * 60 * 24 * 365 * 10,
            value_ranges: std::collections::HashMap::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ── short_run_id ──────────────────────────────────────────────────────────

    #[test]
    fn short_run_id_returns_8_char_hex() {
        let id = short_run_id();
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn short_run_id_returns_different_values() {
        let a = short_run_id();
        let b = short_run_id();
        assert_ne!(a, b);
    }

    // ── model_from_provider_env ───────────────────────────────────────────────

    #[test]
    fn model_from_provider_env_extracts_anthropic_model() {
        let mut env = BTreeMap::new();
        env.insert("ANTHROPIC_MODEL".to_string(), "claude-3".to_string());
        assert_eq!(model_from_provider_env(&env), Some("claude-3".to_string()));
    }

    #[test]
    fn model_from_provider_env_extracts_ollama_model() {
        let mut env = BTreeMap::new();
        env.insert("OLLAMA_MODEL".to_string(), "llama3".to_string());
        assert_eq!(model_from_provider_env(&env), Some("llama3".to_string()));
    }

    #[test]
    fn model_from_provider_env_extracts_openai_model() {
        let mut env = BTreeMap::new();
        env.insert("OPENAI_MODEL".to_string(), "gpt-4".to_string());
        assert_eq!(model_from_provider_env(&env), Some("gpt-4".to_string()));
    }

    #[test]
    fn model_from_provider_env_returns_none_for_empty_map() {
        let env = BTreeMap::new();
        assert_eq!(model_from_provider_env(&env), None);
    }

    #[test]
    fn model_from_provider_env_returns_none_without_model_keys() {
        let mut env = BTreeMap::new();
        env.insert("API_KEY".to_string(), "secret".to_string());
        env.insert("BASE_URL".to_string(), "https://example.com".to_string());
        assert_eq!(model_from_provider_env(&env), None);
    }

    // ── parse_usage_from_stdout ───────────────────────────────────────────────

    #[test]
    fn parse_usage_from_stdout_extracts_cost_and_tokens() {
        let td = tempdir().unwrap();
        let log = td.path().join("stdout.log");
        let json_line = r#"{"usage":{"input_tokens":100,"output_tokens":200},"total_cost_usd":0.05,"model":"claude-3"}"#;
        fs::write(&log, json_line).unwrap();
        let usage = parse_usage_from_stdout(&log).unwrap();
        assert_eq!(usage.prompt_tokens, Some(100));
        assert_eq!(usage.completion_tokens, Some(200));
        assert!((usage.cost_usd.unwrap() - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_usage_from_stdout_handles_missing_usage_fields() {
        let td = tempdir().unwrap();
        let log = td.path().join("stdout.log");
        fs::write(&log, r#"{"model":"test"}"#).unwrap();
        let usage = parse_usage_from_stdout(&log).unwrap();
        assert_eq!(usage.prompt_tokens, None);
        assert_eq!(usage.completion_tokens, None);
        assert_eq!(usage.cost_usd, None);
    }

    #[test]
    fn parse_usage_from_stdout_returns_none_for_empty_file() {
        let td = tempdir().unwrap();
        let log = td.path().join("stdout.log");
        fs::write(&log, "").unwrap();
        assert!(parse_usage_from_stdout(&log).is_none());
    }

    #[test]
    fn parse_usage_from_stdout_returns_none_for_no_json() {
        let td = tempdir().unwrap();
        let log = td.path().join("stdout.log");
        fs::write(&log, "plain text output\nno json here\n").unwrap();
        assert!(parse_usage_from_stdout(&log).is_none());
    }

    #[test]
    fn parse_usage_from_stdout_finds_last_json_line() {
        let td = tempdir().unwrap();
        let log = td.path().join("stdout.log");
        let content = format!(
            "{}\nsome text\n{}",
            r#"{"usage":{"input_tokens":10,"output_tokens":20},"total_cost_usd":0.01}"#,
            r#"{"usage":{"input_tokens":999,"output_tokens":888},"total_cost_usd":0.99}"#,
        );
        fs::write(&log, content).unwrap();
        let usage = parse_usage_from_stdout(&log).unwrap();
        assert_eq!(usage.prompt_tokens, Some(999));
        assert_eq!(usage.completion_tokens, Some(888));
    }

    // ── parse_usage_from_ollama_stats ─────────────────────────────────────────

    #[test]
    fn parse_usage_from_ollama_stats_extracts_from_stats_json() {
        let td = tempdir().unwrap();
        let stats = td.path().join("stats.json");
        fs::write(
            &stats,
            r#"{"model":"llama3","prompt_tokens":50,"completion_tokens":100}"#,
        )
        .unwrap();
        let usage = parse_usage_from_ollama_stats(&stats).unwrap();
        assert_eq!(usage.model, Some("llama3".to_string()));
        assert_eq!(usage.prompt_tokens, Some(50));
        assert_eq!(usage.completion_tokens, Some(100));
        assert_eq!(usage.cost_usd, None);
    }

    #[test]
    fn parse_usage_from_ollama_stats_returns_none_for_missing_file() {
        let td = tempdir().unwrap();
        let stats = td.path().join("nonexistent.json");
        assert!(parse_usage_from_ollama_stats(&stats).is_none());
    }

    // ── detect_run_usage ──────────────────────────────────────────────────────

    #[test]
    fn detect_run_usage_combines_stdout_and_ollama() {
        let td = tempdir().unwrap();
        let card_dir = td.path();
        let logs_dir = card_dir.join("logs");
        fs::create_dir_all(&logs_dir).unwrap();
        fs::write(
            logs_dir.join("stdout.log"),
            r#"{"usage":{"input_tokens":100,"output_tokens":200},"total_cost_usd":0.05}"#,
        )
        .unwrap();
        fs::write(
            logs_dir.join("ollama-stats.json"),
            r#"{"model":"llama3","prompt_tokens":50,"completion_tokens":100}"#,
        )
        .unwrap();
        let usage = detect_run_usage(card_dir).unwrap();
        // stdout takes priority for tokens, ollama fills model
        assert_eq!(usage.prompt_tokens, Some(100));
        assert_eq!(usage.completion_tokens, Some(200));
    }

    #[test]
    fn detect_run_usage_returns_none_for_card_without_logs() {
        let td = tempdir().unwrap();
        assert!(detect_run_usage(td.path()).is_none());
    }

    // ── detect_model_from_logs ────────────────────────────────────────────────

    #[test]
    fn detect_model_from_logs_extracts_model_name() {
        let td = tempdir().unwrap();
        let logs_dir = td.path().join("logs");
        fs::create_dir_all(&logs_dir).unwrap();
        fs::write(
            logs_dir.join("stdout.log"),
            r#"{"model":"claude-opus-4-20250514"}"#,
        )
        .unwrap();
        assert_eq!(
            detect_model_from_logs(td.path()),
            Some("claude-opus-4-20250514".to_string())
        );
    }

    // ── load_feed_config ──────────────────────────────────────────────────────

    #[test]
    fn load_feed_config_returns_defaults_when_no_feed_config() {
        let td = tempdir().unwrap();
        let cfg = load_feed_config(td.path());
        assert!(!cfg.id.is_empty());
        assert_eq!(cfg.validation.required_fields, vec!["feed_id".to_string()]);
    }

    #[test]
    fn load_feed_config_reads_from_existing_config() {
        let td = tempdir().unwrap();
        let cfg_json = serde_json::json!({
            "id": "my-feed",
            "source_type": "http",
            "endpoint": "https://example.com/feed",
            "poll_interval_secs": 30,
            "validation": {
                "required_fields": ["feed_id", "timestamp"],
                "max_staleness_secs": 600,
                "value_ranges": {}
            }
        });
        fs::write(
            td.path().join("feed_config.json"),
            serde_json::to_vec(&cfg_json).unwrap(),
        )
        .unwrap();
        let cfg = load_feed_config(td.path());
        assert_eq!(cfg.id, "my-feed");
        assert_eq!(cfg.poll_interval_secs, 30);
        assert_eq!(
            cfg.validation.required_fields,
            vec!["feed_id".to_string(), "timestamp".to_string()]
        );
    }

    // ── validate_realtime_output ──────────────────────────────────────────────

    #[test]
    fn validate_realtime_output_returns_clean_for_empty_output() {
        let td = tempdir().unwrap();
        let card_dir = td.path();
        fs::create_dir_all(card_dir.join("output")).unwrap();
        fs::create_dir_all(card_dir.join("logs")).unwrap();
        let summary = validate_realtime_output(card_dir).unwrap();
        // No JSON files in output/ → 0 records processed
        assert_eq!(summary.valid, 0);
        assert_eq!(summary.invalid, 0);
        // Empty feed has Down health → 1 critical alert generated
        assert_eq!(summary.critical_alerts, 1);
    }

    #[test]
    fn validate_realtime_output_processes_valid_json_records() {
        let td = tempdir().unwrap();
        let card_dir = td.path();
        fs::create_dir_all(card_dir.join("output")).unwrap();
        fs::create_dir_all(card_dir.join("logs")).unwrap();

        // The default feed config requires "feed_id" in the `fields` map,
        // so include it there as well as at the top level for deserialization.
        let record = serde_json::json!({
            "feed_id": "test-feed",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "fields": {"feed_id": "test-feed", "key": "value"}
        });
        fs::write(
            card_dir.join("output").join("record1.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();

        let summary = validate_realtime_output(card_dir).unwrap();
        assert!(summary.total >= 1, "expected at least 1 record processed");
    }

    // ── RunUsage struct ───────────────────────────────────────────────────────

    #[test]
    fn run_usage_default_values() {
        let usage = RunUsage::default();
        assert_eq!(usage.model, None);
        assert_eq!(usage.prompt_tokens, None);
        assert_eq!(usage.completion_tokens, None);
        assert_eq!(usage.cost_usd, None);
    }
}
