/// Auto-Claude implementation plan loader.
///
/// Parses `implementation_plan.json` files produced by the Auto-Claude
/// orchestrator and exposes plan/phase/subtask data for CLI and Quick Look
/// rendering.
use anyhow::Context;
use bop_core::Meta;
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::render::CardView;

// ── Data model ──────────────────────────────────────────────────────────

/// Top-level Auto-Claude implementation plan.
#[derive(Debug, Deserialize)]
pub struct AcPlan {
    pub phases: Vec<AcPhase>,
}

/// A phase within an implementation plan.
#[derive(Debug, Deserialize)]
pub struct AcPhase {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub subtasks: Vec<AcSubtask>,
}

/// A single subtask inside a phase.
#[derive(Debug, Deserialize)]
pub struct AcSubtask {
    pub id: String,
    pub description: String,
    /// `"pending"` | `"in_progress"` | `"completed"`
    #[serde(default = "default_status")]
    pub status: String,
}

fn default_status() -> String {
    "pending".to_string()
}

// ── Parsing ─────────────────────────────────────────────────────────────

/// Parse an `implementation_plan.json` file into an [`AcPlan`].
pub fn parse_plan(path: &Path) -> anyhow::Result<AcPlan> {
    let data = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read plan: {}", path.display()))?;
    let plan: AcPlan = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse plan JSON: {}", path.display()))?;
    Ok(plan)
}

// ── Path resolution ─────────────────────────────────────────────────────

/// Maximum number of parent directories to walk when searching for the
/// project root.  Six levels is sufficient for typical mono-repo layouts
/// (e.g. `repo/.cards/running/card.bop/` is 3 levels deep).
const MAX_ANCESTOR_DEPTH: usize = 6;

/// Walk parent directories from `start_dir` looking for a `.git` directory
/// or a `.auto-claude` directory.  Returns the first ancestor that contains
/// either marker, or `None` if none is found within [`MAX_ANCESTOR_DEPTH`]
/// levels.
///
/// This is a pure filesystem check — no `git` subprocess required — so it
/// works in minimal environments and in test temp-dirs that contain an
/// `.auto-claude` directory without a full git repo.
pub fn find_git_root(start_dir: &Path) -> Option<PathBuf> {
    let canonical = start_dir.canonicalize().ok()?;
    let mut current = canonical.as_path();

    for _ in 0..MAX_ANCESTOR_DEPTH {
        if current.join(".git").exists() || current.join(".auto-claude").exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
    None
}

/// Resolve the `implementation_plan.json` path for a given Auto-Claude spec
/// ID.
///
/// Looks for `<git_root>/.auto-claude/specs/<ac_spec_id>-*/implementation_plan.json`.
/// Returns the first match (there should only be one per ID prefix), or
/// `None` if no matching directory exists.
pub fn resolve_spec_dir(git_root: &Path, ac_spec_id: &str) -> Option<PathBuf> {
    let specs_dir = git_root.join(".auto-claude").join("specs");
    let prefix = format!("{}-", ac_spec_id);

    let entries = std::fs::read_dir(&specs_dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if name_str.starts_with(&prefix) && entry.path().is_dir() {
            let plan_path = entry.path().join("implementation_plan.json");
            if plan_path.exists() {
                return Some(plan_path);
            }
        }
    }
    None
}

// ── Glyphs ──────────────────────────────────────────────────────────────

/// Half-circle glyph encoding phase completion fraction.
///
/// Thresholds (from spec):
/// - `0.0`        → `○` (U+25CB)
/// - `0.01–0.33`  → `◔` (U+25D4)
/// - `0.34–0.66`  → `◑` (U+25D1)
/// - `0.67–0.99`  → `◕` (U+25D5)
/// - `1.0`        → `●` (U+25CF)
pub fn half_circle_glyph(frac: f64) -> char {
    if frac <= 0.0 {
        '○'
    } else if frac <= 0.33 {
        '◔'
    } else if frac <= 0.66 {
        '◑'
    } else if frac < 1.0 {
        '◕'
    } else {
        '●'
    }
}

// ── Plan summary ────────────────────────────────────────────────────────

/// Aggregate statistics from an [`AcPlan`].
#[derive(Debug, Clone)]
pub struct PlanSummary {
    /// Number of completed subtasks across all phases.
    pub completed: usize,
    /// Total number of subtasks across all phases.
    pub total: usize,
    /// Name of the current phase (first phase with incomplete subtasks),
    /// or the last phase name if everything is done.
    pub current_phase_name: Option<String>,
    /// Fraction of subtasks completed in the current phase (0.0–1.0).
    pub current_phase_frac: f64,
}

/// Summarise an [`AcPlan`] into completion counts and current-phase info.
///
/// The "current phase" is the first phase that still has at least one
/// non-completed subtask.  If every subtask in every phase is completed
/// the last phase is returned with `current_phase_frac = 1.0`.
///
/// An empty plan (no phases) returns zeros and `None` for the phase name.
pub fn plan_summary(plan: &AcPlan) -> PlanSummary {
    let mut completed: usize = 0;
    let mut total: usize = 0;
    let mut current_phase_name: Option<String> = None;
    let mut current_phase_frac: f64 = 0.0;

    for phase in &plan.phases {
        let phase_total = phase.subtasks.len();
        let phase_done = phase
            .subtasks
            .iter()
            .filter(|s| s.status == "completed")
            .count();

        total += phase_total;
        completed += phase_done;

        // Current phase = first phase with incomplete subtasks.
        if current_phase_name.is_none() && phase_done < phase_total {
            current_phase_name = Some(phase.name.clone());
            current_phase_frac = if phase_total > 0 {
                phase_done as f64 / phase_total as f64
            } else {
                0.0
            };
        }
    }

    // If all phases are complete, report the last phase at 100 %.
    if current_phase_name.is_none() {
        if let Some(last) = plan.phases.last() {
            current_phase_name = Some(last.name.clone());
            current_phase_frac = 1.0;
        }
    }

    PlanSummary {
        completed,
        total,
        current_phase_name,
        current_phase_frac,
    }
}

// ── CardView enrichment ─────────────────────────────────────────────────

/// Enrich a [`CardView`] with Auto-Claude plan data.
///
/// If `meta.ac_spec_id` is set and the corresponding
/// `implementation_plan.json` can be found under `git_root`, this function
/// populates `phase_name`, `phase_frac`, and `progress` on the view.
///
/// Silently returns without changes when:
/// - `ac_spec_id` is `None`
/// - the spec directory cannot be resolved
/// - the plan file cannot be parsed
pub fn enrich_card_view(view: &mut CardView, meta: &Meta, git_root: &Path) {
    let ac_spec_id = match &meta.ac_spec_id {
        Some(id) => id,
        None => return,
    };

    let plan_path = match resolve_spec_dir(git_root, ac_spec_id) {
        Some(p) => p,
        None => return,
    };

    let plan = match parse_plan(&plan_path) {
        Ok(p) => p,
        Err(_) => return,
    };

    let summary = plan_summary(&plan);

    view.phase_name = summary.current_phase_name;
    view.phase_frac = summary.current_phase_frac as f32;

    if summary.total > 0 {
        view.progress = ((summary.completed as f64 / summary.total as f64) * 100.0) as u8;
        view.ac_subtasks_done = Some(summary.completed);
        view.ac_subtasks_total = Some(summary.total);
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // ── half_circle_glyph thresholds ────────────────────────────────────

    #[test]
    fn acplan_half_circle_zero() {
        assert_eq!(half_circle_glyph(0.0), '○');
    }

    #[test]
    fn acplan_half_circle_one_percent() {
        assert_eq!(half_circle_glyph(0.01), '◔');
    }

    #[test]
    fn acplan_half_circle_thirty_three() {
        assert_eq!(half_circle_glyph(0.33), '◔');
    }

    #[test]
    fn acplan_half_circle_thirty_four() {
        assert_eq!(half_circle_glyph(0.34), '◑');
    }

    #[test]
    fn acplan_half_circle_fifty() {
        assert_eq!(half_circle_glyph(0.50), '◑');
    }

    #[test]
    fn acplan_half_circle_sixty_six() {
        assert_eq!(half_circle_glyph(0.66), '◑');
    }

    #[test]
    fn acplan_half_circle_sixty_seven() {
        assert_eq!(half_circle_glyph(0.67), '◕');
    }

    #[test]
    fn acplan_half_circle_ninety_nine() {
        assert_eq!(half_circle_glyph(0.99), '◕');
    }

    #[test]
    fn acplan_half_circle_hundred() {
        assert_eq!(half_circle_glyph(1.0), '●');
    }

    #[test]
    fn acplan_half_circle_negative() {
        // Negative values clamp to ○.
        assert_eq!(half_circle_glyph(-0.5), '○');
    }

    #[test]
    fn acplan_half_circle_over_one() {
        // Values > 1.0 map to ● (full).
        assert_eq!(half_circle_glyph(1.5), '●');
    }

    // ── parse_plan ──────────────────────────────────────────────────────

    fn sample_plan_json() -> &'static str {
        r#"{
            "phases": [
                {
                    "id": "phase-1",
                    "name": "Core Data Model",
                    "subtasks": [
                        {
                            "id": "subtask-1-1",
                            "description": "Add field to struct",
                            "status": "completed"
                        },
                        {
                            "id": "subtask-1-2",
                            "description": "Write tests",
                            "status": "in_progress"
                        }
                    ]
                },
                {
                    "id": "phase-2",
                    "name": "CLI Rendering",
                    "subtasks": [
                        {
                            "id": "subtask-2-1",
                            "description": "Build progress bar",
                            "status": "pending"
                        }
                    ]
                }
            ]
        }"#
    }

    #[test]
    fn acplan_parse_plan_basic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("implementation_plan.json");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(sample_plan_json().as_bytes()).unwrap();

        let plan = parse_plan(&path).unwrap();
        assert_eq!(plan.phases.len(), 2);
        assert_eq!(plan.phases[0].id, "phase-1");
        assert_eq!(plan.phases[0].name, "Core Data Model");
        assert_eq!(plan.phases[0].subtasks.len(), 2);
        assert_eq!(plan.phases[0].subtasks[0].status, "completed");
        assert_eq!(plan.phases[0].subtasks[1].status, "in_progress");
        assert_eq!(plan.phases[1].subtasks[0].status, "pending");
    }

    #[test]
    fn acplan_parse_plan_extra_fields_ignored() {
        // Real implementation_plan.json has many extra fields (summary,
        // verification_strategy, etc.) — serde should ignore them.
        let json = r#"{
            "feature": "Some feature",
            "workflow_type": "feature",
            "phases": [
                {
                    "id": "phase-1",
                    "name": "Phase One",
                    "type": "implementation",
                    "description": "Desc",
                    "depends_on": [],
                    "parallel_safe": true,
                    "subtasks": [
                        {
                            "id": "s-1",
                            "description": "Do a thing",
                            "service": "bop-core",
                            "files_to_modify": ["foo.rs"],
                            "files_to_create": [],
                            "patterns_from": [],
                            "verification": {"type": "command"},
                            "status": "completed",
                            "notes": "Done",
                            "updated_at": "2026-03-07T00:00:00Z"
                        }
                    ]
                }
            ],
            "summary": {"total_phases": 1},
            "verification_strategy": {},
            "qa_acceptance": {},
            "qa_signoff": null
        }"#;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.json");
        std::fs::write(&path, json).unwrap();

        let plan = parse_plan(&path).unwrap();
        assert_eq!(plan.phases.len(), 1);
        assert_eq!(plan.phases[0].subtasks[0].status, "completed");
    }

    #[test]
    fn acplan_parse_plan_missing_file() {
        let path = std::path::PathBuf::from("/tmp/nonexistent-acplan-test.json");
        let result = parse_plan(&path);
        assert!(result.is_err());
    }

    #[test]
    fn acplan_parse_plan_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json at all").unwrap();

        let result = parse_plan(&path);
        assert!(result.is_err());
    }

    #[test]
    fn acplan_parse_plan_default_status() {
        // If a subtask omits status, it defaults to "pending".
        let json = r#"{
            "phases": [{
                "id": "p1",
                "name": "Phase 1",
                "subtasks": [{
                    "id": "s1",
                    "description": "No status field"
                }]
            }]
        }"#;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.json");
        std::fs::write(&path, json).unwrap();

        let plan = parse_plan(&path).unwrap();
        assert_eq!(plan.phases[0].subtasks[0].status, "pending");
    }

    #[test]
    fn acplan_parse_plan_empty_phases() {
        let json = r#"{"phases": []}"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.json");
        std::fs::write(&path, json).unwrap();

        let plan = parse_plan(&path).unwrap();
        assert!(plan.phases.is_empty());
    }

    // ── Glyph set is BMP-safe ───────────────────────────────────────────

    #[test]
    fn acplan_glyphs_are_bmp() {
        // All half-circle glyphs must be in BMP (< U+FFFF) for safe rendering.
        for frac in [0.0, 0.01, 0.33, 0.34, 0.50, 0.66, 0.67, 0.99, 1.0] {
            let ch = half_circle_glyph(frac);
            assert!(
                (ch as u32) < 0xFFFF,
                "glyph {} (U+{:04X}) is not BMP-safe",
                ch,
                ch as u32
            );
        }
    }

    // ── find_git_root ───────────────────────────────────────────────────

    #[test]
    fn find_git_root_detects_dot_git() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();

        let found = find_git_root(dir.path());
        assert!(found.is_some(), "should detect .git directory");
        assert_eq!(
            found.unwrap().canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn find_git_root_detects_auto_claude() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".auto-claude")).unwrap();

        let found = find_git_root(dir.path());
        assert!(found.is_some(), "should detect .auto-claude directory");
        assert_eq!(
            found.unwrap().canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn find_git_root_walks_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let child = dir.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&child).unwrap();

        let found = find_git_root(&child);
        assert!(found.is_some(), "should find root from nested child");
        assert_eq!(
            found.unwrap().canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn find_git_root_returns_none_for_plain_dir() {
        let dir = tempfile::tempdir().unwrap();
        // No .git or .auto-claude — should return None
        assert!(find_git_root(dir.path()).is_none());
    }

    #[test]
    fn find_git_root_respects_max_depth() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        // Create a child 7 levels deep — exceeds MAX_ANCESTOR_DEPTH (6)
        let mut deep = dir.path().to_path_buf();
        for i in 0..7 {
            deep = deep.join(format!("d{}", i));
        }
        std::fs::create_dir_all(&deep).unwrap();

        assert!(
            find_git_root(&deep).is_none(),
            "should not find root beyond max depth"
        );
    }

    // ── resolve_spec_dir ────────────────────────────────────────────────

    #[test]
    fn resolve_spec_dir_finds_matching_spec() {
        let dir = tempfile::tempdir().unwrap();
        let spec_dir = dir
            .path()
            .join(".auto-claude")
            .join("specs")
            .join("027-ac-progress");
        std::fs::create_dir_all(&spec_dir).unwrap();
        let plan_path = spec_dir.join("implementation_plan.json");
        std::fs::write(&plan_path, r#"{"phases":[]}"#).unwrap();

        let found = resolve_spec_dir(dir.path(), "027");
        assert!(found.is_some(), "should find spec dir with prefix 027-");
        assert_eq!(found.unwrap(), plan_path);
    }

    #[test]
    fn resolve_spec_dir_returns_none_when_no_plan() {
        let dir = tempfile::tempdir().unwrap();
        // Spec dir exists but has no implementation_plan.json
        let spec_dir = dir
            .path()
            .join(".auto-claude")
            .join("specs")
            .join("027-ac-progress");
        std::fs::create_dir_all(&spec_dir).unwrap();

        assert!(
            resolve_spec_dir(dir.path(), "027").is_none(),
            "should return None when plan file is missing"
        );
    }

    #[test]
    fn resolve_spec_dir_returns_none_for_unknown_id() {
        let dir = tempfile::tempdir().unwrap();
        let spec_dir = dir
            .path()
            .join(".auto-claude")
            .join("specs")
            .join("027-ac-progress");
        std::fs::create_dir_all(&spec_dir).unwrap();
        std::fs::write(
            spec_dir.join("implementation_plan.json"),
            r#"{"phases":[]}"#,
        )
        .unwrap();

        assert!(
            resolve_spec_dir(dir.path(), "999").is_none(),
            "should return None for non-matching ID"
        );
    }

    #[test]
    fn resolve_spec_dir_returns_none_when_no_specs_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            resolve_spec_dir(dir.path(), "027").is_none(),
            "should return None when .auto-claude/specs/ does not exist"
        );
    }

    // ── plan_summary ────────────────────────────────────────────────────

    #[test]
    fn plan_summary_basic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.json");
        std::fs::write(&path, sample_plan_json()).unwrap();

        let plan = parse_plan(&path).unwrap();
        let s = plan_summary(&plan);

        // sample: phase-1 has 1 completed + 1 in_progress, phase-2 has 1 pending
        assert_eq!(s.completed, 1);
        assert_eq!(s.total, 3);
        // Current phase = phase-1 (first with incomplete subtasks)
        assert_eq!(s.current_phase_name.as_deref(), Some("Core Data Model"));
        // 1 of 2 done in phase-1 → 0.5
        assert!((s.current_phase_frac - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn plan_summary_all_completed() {
        let json = r#"{
            "phases": [
                {
                    "id": "p1", "name": "Alpha",
                    "subtasks": [
                        {"id": "s1", "description": "A", "status": "completed"},
                        {"id": "s2", "description": "B", "status": "completed"}
                    ]
                },
                {
                    "id": "p2", "name": "Beta",
                    "subtasks": [
                        {"id": "s3", "description": "C", "status": "completed"}
                    ]
                }
            ]
        }"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.json");
        std::fs::write(&path, json).unwrap();

        let plan = parse_plan(&path).unwrap();
        let s = plan_summary(&plan);

        assert_eq!(s.completed, 3);
        assert_eq!(s.total, 3);
        // All done → last phase name, frac = 1.0
        assert_eq!(s.current_phase_name.as_deref(), Some("Beta"));
        assert!((s.current_phase_frac - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn plan_summary_empty_plan() {
        let plan = AcPlan { phases: Vec::new() };
        let s = plan_summary(&plan);

        assert_eq!(s.completed, 0);
        assert_eq!(s.total, 0);
        assert!(s.current_phase_name.is_none());
        assert!((s.current_phase_frac).abs() < f64::EPSILON);
    }

    #[test]
    fn plan_summary_single_phase_none_done() {
        let json = r#"{
            "phases": [{
                "id": "p1", "name": "Setup",
                "subtasks": [
                    {"id": "s1", "description": "A", "status": "pending"},
                    {"id": "s2", "description": "B", "status": "pending"}
                ]
            }]
        }"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.json");
        std::fs::write(&path, json).unwrap();

        let plan = parse_plan(&path).unwrap();
        let s = plan_summary(&plan);

        assert_eq!(s.completed, 0);
        assert_eq!(s.total, 2);
        assert_eq!(s.current_phase_name.as_deref(), Some("Setup"));
        assert!((s.current_phase_frac).abs() < f64::EPSILON);
    }

    #[test]
    fn plan_summary_skips_completed_phases() {
        let json = r#"{
            "phases": [
                {
                    "id": "p1", "name": "Done Phase",
                    "subtasks": [
                        {"id": "s1", "description": "A", "status": "completed"}
                    ]
                },
                {
                    "id": "p2", "name": "Active Phase",
                    "subtasks": [
                        {"id": "s2", "description": "B", "status": "completed"},
                        {"id": "s3", "description": "C", "status": "in_progress"},
                        {"id": "s4", "description": "D", "status": "pending"}
                    ]
                },
                {
                    "id": "p3", "name": "Future Phase",
                    "subtasks": [
                        {"id": "s5", "description": "E", "status": "pending"}
                    ]
                }
            ]
        }"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.json");
        std::fs::write(&path, json).unwrap();

        let plan = parse_plan(&path).unwrap();
        let s = plan_summary(&plan);

        assert_eq!(s.completed, 2); // s1 + s2
        assert_eq!(s.total, 5);
        // Current phase = "Active Phase" (first incomplete)
        assert_eq!(s.current_phase_name.as_deref(), Some("Active Phase"));
        // 1 of 3 done → ~0.333
        assert!((s.current_phase_frac - 1.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn plan_summary_phase_with_empty_subtasks() {
        let json = r#"{
            "phases": [
                {
                    "id": "p1", "name": "Empty",
                    "subtasks": []
                },
                {
                    "id": "p2", "name": "Real",
                    "subtasks": [
                        {"id": "s1", "description": "A", "status": "pending"}
                    ]
                }
            ]
        }"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.json");
        std::fs::write(&path, json).unwrap();

        let plan = parse_plan(&path).unwrap();
        let s = plan_summary(&plan);

        assert_eq!(s.completed, 0);
        assert_eq!(s.total, 1);
        // Empty phase has 0 done < 0 total → false, so skipped.
        // Current phase should be "Real".
        assert_eq!(s.current_phase_name.as_deref(), Some("Real"));
        assert!((s.current_phase_frac).abs() < f64::EPSILON);
    }

    // ── enrich_card_view ────────────────────────────────────────────────

    #[test]
    fn enrich_card_view_populates_fields() {
        // Set up a fake git root with .auto-claude/specs/022-test/implementation_plan.json
        let dir = tempfile::tempdir().unwrap();
        let spec_dir = dir
            .path()
            .join(".auto-claude")
            .join("specs")
            .join("022-test-feature");
        std::fs::create_dir_all(&spec_dir).unwrap();
        std::fs::write(
            spec_dir.join("implementation_plan.json"),
            r#"{
                "phases": [
                    {
                        "id": "p1", "name": "Core",
                        "subtasks": [
                            {"id": "s1", "description": "A", "status": "completed"},
                            {"id": "s2", "description": "B", "status": "completed"}
                        ]
                    },
                    {
                        "id": "p2", "name": "CLI",
                        "subtasks": [
                            {"id": "s3", "description": "C", "status": "in_progress"},
                            {"id": "s4", "description": "D", "status": "pending"}
                        ]
                    }
                ]
            }"#,
        )
        .unwrap();

        let meta = Meta {
            id: "test-card".into(),
            stage: "implement".into(),
            created: chrono::Utc::now(),
            ac_spec_id: Some("022".into()),
            ..Default::default()
        };

        let mut view = crate::render::from_meta(&meta, "running");
        assert!(view.phase_name.is_none());
        assert_eq!(view.progress, 0);

        enrich_card_view(&mut view, &meta, dir.path());

        // 2 of 4 done → progress = 50
        assert_eq!(view.progress, 50);
        // AC subtask counts populated.
        assert_eq!(view.ac_subtasks_done, Some(2));
        assert_eq!(view.ac_subtasks_total, Some(4));
        // Current phase = "CLI" (first with incomplete subtasks)
        assert_eq!(view.phase_name.as_deref(), Some("CLI"));
        // 0 of 2 done in CLI → 0.0
        assert!((view.phase_frac).abs() < f32::EPSILON);
    }

    #[test]
    fn enrich_card_view_noop_without_spec_id() {
        let dir = tempfile::tempdir().unwrap();
        let meta = Meta {
            id: "no-spec".into(),
            stage: "pending".into(),
            created: chrono::Utc::now(),
            ..Default::default()
        };

        let mut view = crate::render::from_meta(&meta, "pending");
        enrich_card_view(&mut view, &meta, dir.path());

        // Nothing changed — ac_spec_id is None.
        assert!(view.phase_name.is_none());
        assert_eq!(view.progress, 0);
    }

    #[test]
    fn enrich_card_view_noop_when_plan_missing() {
        let dir = tempfile::tempdir().unwrap();
        let meta = Meta {
            id: "missing-plan".into(),
            stage: "implement".into(),
            created: chrono::Utc::now(),
            ac_spec_id: Some("999".into()),
            ..Default::default()
        };

        let mut view = crate::render::from_meta(&meta, "running");
        enrich_card_view(&mut view, &meta, dir.path());

        // Spec dir not found → silent no-op.
        assert!(view.phase_name.is_none());
        assert_eq!(view.progress, 0);
    }
}
