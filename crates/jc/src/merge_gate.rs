use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::Duration;
use tokio::process::Command as TokioCommand;

use jobcard_core::write_meta;

use super::VcsEngine;
use crate::{paths, policy, quicklook, workspace};

pub async fn run_merge_gate(
    cards_dir: &Path,
    poll_ms: u64,
    once: bool,
    vcs_engine: VcsEngine,
) -> anyhow::Result<()> {
    paths::ensure_cards_layout(cards_dir)?;

    let done_dir = cards_dir.join("done");
    let merged_dir = cards_dir.join("merged");
    let failed_dir = cards_dir.join("failed");
    let mg_lineage_enabled = jobcard_core::lineage::is_enabled(cards_dir);

    loop {
        let mut mg_lineage_events: Vec<jobcard_core::lineage::RunEvent> = Vec::new();
        let mut mg_record =
            |meta: &jobcard_core::Meta, from: &str, to: &str, card_dir: Option<&Path>| {
                if mg_lineage_enabled {
                    let et = jobcard_core::lineage::event_type_for(from, to);
                    mg_lineage_events.push(jobcard_core::lineage::build_run_event_with_dir(
                        et, meta, from, to, card_dir,
                    ));
                }
            };

        if let Ok(entries) = fs::read_dir(&done_dir) {
            for ent in entries.flatten() {
                let card_dir = ent.path();
                if !card_dir.is_dir() {
                    continue;
                }
                if card_dir.extension().and_then(|s| s.to_str()).unwrap_or("") != "jobcard" {
                    continue;
                }

                let name = match card_dir.file_name().and_then(|s| s.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };

                let mut meta = match jobcard_core::read_meta(&card_dir) {
                    Ok(m) => m,
                    Err(_) => {
                        let failed_path = failed_dir.join(&name);
                        let _ = fs::rename(&card_dir, &failed_path);
                        quicklook::render_card_thumbnail(&failed_path);
                        // No meta available for lineage event here
                        continue;
                    }
                };

                fs::create_dir_all(card_dir.join("logs"))?;
                fs::create_dir_all(card_dir.join("output"))?;

                let workdir = {
                    if let Some(ref p) = meta.workspace_path {
                        let candidate = PathBuf::from(p);
                        if candidate.exists() {
                            candidate
                        } else {
                            let ws = card_dir.join("workspace");
                            if ws.exists() {
                                ws
                            } else {
                                card_dir.clone()
                            }
                        }
                    } else {
                        let ws = card_dir.join("workspace");
                        if ws.exists() {
                            ws
                        } else {
                            card_dir.clone()
                        }
                    }
                };

                let qa_log = card_dir.join("logs").join("qa.log");

                // Heal stale working copy before running ACs — concurrent jj ops
                // often leave workspaces stale, causing `jj log` to fail.
                if matches!(vcs_engine, VcsEngine::Jj) {
                    let _ = TokioCommand::new("jj")
                        .args(["workspace", "update-stale"])
                        .current_dir(&workdir)
                        .output()
                        .await;
                }

                let mut acceptance_ok = true;
                let mut failed_criterion: Option<String> = None;
                for criterion in meta.acceptance_criteria.iter() {
                    let output = TokioCommand::new("sh")
                        .arg("-lc")
                        .arg(criterion)
                        .current_dir(&workdir)
                        .env("CARGO_TARGET_DIR", workdir.join("target"))
                        .output()
                        .await;

                    match output {
                        Ok(out) => {
                            let mut f = fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(&qa_log)?;
                            writeln!(f, "--- criterion: {} ---", criterion.replace('\n', "\\n"))?;
                            f.write_all(&out.stdout)?;
                            f.write_all(&out.stderr)?;
                            writeln!(f)?;

                            if !out.status.success() {
                                acceptance_ok = false;
                                failed_criterion = Some(criterion.to_string());
                                break;
                            }
                        }
                        Err(_) => {
                            acceptance_ok = false;
                            failed_criterion = Some(criterion.to_string());
                            break;
                        }
                    }
                }

                if !acceptance_ok {
                    meta.failure_reason = Some("acceptance_criteria_failed".to_string());
                    let report = format!(
                        "criterion failed: {}\n",
                        failed_criterion.unwrap_or_else(|| "<unknown>".to_string())
                    );
                    let _ = fs::write(card_dir.join("output").join("qa_report.md"), report);
                    let _ = write_meta(&card_dir, &meta);
                    let failed_path = failed_dir.join(&name);
                    let _ = fs::rename(&card_dir, &failed_path);
                    mg_record(&meta, "done", "failed", Some(&failed_path));
                    quicklook::render_card_thumbnail(&failed_path);
                    continue;
                }

                if let Err(err) = policy::policy_check_card(cards_dir, &card_dir, &meta.id) {
                    meta.failure_reason = Some("policy_violation".to_string());
                    meta.policy_result = Some(format!("failed: {err}"));
                    let _ = fs::write(
                        card_dir.join("output").join("qa_report.md"),
                        format!("policy violation: {err}\n"),
                    );
                    let _ = write_meta(&card_dir, &meta);
                    let failed_path = failed_dir.join(&name);
                    let _ = fs::rename(&card_dir, &failed_path);
                    mg_record(&meta, "done", "failed", Some(&failed_path));
                    quicklook::render_card_thumbnail(&failed_path);
                    continue;
                }
                meta.policy_result = Some("pass".to_string());

                let ws_path = if let Some(ref p) = meta.workspace_path {
                    PathBuf::from(p)
                } else {
                    card_dir.join("workspace")
                };

                if ws_path.exists() {
                    let mut vcs_err: Option<String> = None;
                    match vcs_engine {
                        VcsEngine::GitGt => {
                            let Some(git_root) = workspace::find_git_root(cards_dir) else {
                                meta.failure_reason = Some("git_root_not_found".to_string());
                                meta.policy_result = Some("failed".to_string());
                                let _ = write_meta(&card_dir, &meta);
                                let failed_path = failed_dir.join(&name);
                                let _ = fs::rename(&card_dir, &failed_path);
                                mg_record(&meta, "done", "failed", Some(&failed_path));
                                quicklook::render_card_thumbnail(&failed_path);
                                continue;
                            };

                            let add_status = StdCommand::new("git")
                                .args(["add", "-A"])
                                .current_dir(&ws_path)
                                .status();
                            if !matches!(add_status, Ok(s) if s.success()) {
                                vcs_err = Some("git add -A failed".to_string());
                            }

                            if vcs_err.is_none() {
                                let diff_cached = StdCommand::new("git")
                                    .args(["diff", "--cached", "--quiet"])
                                    .current_dir(&ws_path)
                                    .status();
                                let has_staged =
                                    matches!(diff_cached, Ok(s) if s.code() == Some(1));
                                if has_staged {
                                    let msg = format!("jobcard: {}", meta.id);
                                    let commit_status = StdCommand::new("git")
                                        .args(["commit", "-m", &msg])
                                        .current_dir(&ws_path)
                                        .status();
                                    if !matches!(commit_status, Ok(s) if s.success()) {
                                        vcs_err = Some("git commit failed".to_string());
                                    }
                                }
                            }

                            if vcs_err.is_none() {
                                let restack = StdCommand::new("gt")
                                    .args(["stack", "restack", "--no-interactive"])
                                    .current_dir(&ws_path)
                                    .status();
                                if !matches!(restack, Ok(s) if s.success()) {
                                    vcs_err = Some("gt stack restack failed".to_string());
                                }
                            }

                            if vcs_err.is_none() {
                                let submit = StdCommand::new("gt")
                                    .args([
                                        "submit",
                                        "--stack",
                                        "--no-interactive",
                                        "--no-edit",
                                        "--draft",
                                    ])
                                    .current_dir(&ws_path)
                                    .status();
                                if !matches!(submit, Ok(s) if s.success()) {
                                    vcs_err = Some("gt submit failed".to_string());
                                }
                            }

                            let _ = workspace::remove_worktree(&ws_path, Some(&git_root));
                        }
                        VcsEngine::Jj => {
                            let repo_root = workspace::find_git_root(cards_dir)
                                .unwrap_or_else(|| cards_dir.to_path_buf());
                            if let Err(e) = jobcard_core::worktree::squash_workspace(&ws_path) {
                                vcs_err = Some(format!("jj squash failed: {e}"));
                            }
                            if vcs_err.is_none() {
                                let ws_name = meta
                                    .workspace_name
                                    .clone()
                                    .unwrap_or_else(|| "workspace".to_string());
                                if let Err(e) =
                                    jobcard_core::worktree::forget_workspace(&repo_root, &ws_name)
                                {
                                    vcs_err = Some(format!("jj workspace forget failed: {e}"));
                                }
                            }
                            // push + PR are best-effort — skip gracefully when no remote
                            if vcs_err.is_none() {
                                let _ = jobcard_core::worktree::push_stack(&repo_root, "origin");
                            }
                            if vcs_err.is_none() {
                                let pr_result = StdCommand::new("gh")
                                    .args(["pr", "create", "--fill", "--draft"])
                                    .current_dir(&repo_root)
                                    .output();
                                // gh pr create is best-effort; no remote = skip silently
                                let _ = pr_result;
                            }
                        }
                    }

                    if let Some(err) = vcs_err {
                        let _ = fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&qa_log)
                            .and_then(|mut f| f.write_all(format!("{err}\n").as_bytes()));
                        meta.failure_reason = Some("vcs_publish_failed".to_string());
                        meta.policy_result = Some("failed".to_string());
                        let _ = write_meta(&card_dir, &meta);
                        let failed_path = failed_dir.join(&name);
                        let _ = fs::rename(&card_dir, &failed_path);
                        mg_record(&meta, "done", "failed", Some(&failed_path));
                        quicklook::render_card_thumbnail(&failed_path);
                        continue;
                    }
                }

                // Best-effort: capture file-change manifest for Quick Look
                let branch = meta.change_ref.clone().unwrap_or_else(|| meta.id.clone());
                let _ = workspace::write_changes_json(&card_dir, &workdir, &branch).await;

                let _ = write_meta(&card_dir, &meta);
                let merged_path = merged_dir.join(&name);
                let _ = fs::rename(&card_dir, &merged_path);
                mg_record(&meta, "done", "merged", Some(&merged_path));
                quicklook::compress_card(&merged_path);
                quicklook::render_card_thumbnail(&merged_path);
            }
        }

        // Flush collected lineage events (O(N) — one write per loop iteration)
        if !mg_lineage_events.is_empty() {
            jobcard_core::lineage::flush_events(cards_dir, &mg_lineage_events);
            mg_lineage_events.clear();
        }

        if once {
            break;
        }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }

    Ok(())
}
