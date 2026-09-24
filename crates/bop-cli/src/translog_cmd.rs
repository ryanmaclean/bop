//! `bop translog` — inspect the per-card immutable transition log (bop#9).
//!
//! Output is JSON only (schemas `bop.translog.view.v1`,
//! `bop.translog.verify.v1`, `bop.translog.verify_all.v1`). The directory
//! state stays authoritative; these commands report how the replayed log
//! compares with it.

use anyhow::Context;
use bop_core::translog;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

use crate::paths;

const STATE_DIRS: [&str; 6] = ["drafts", "pending", "running", "done", "merged", "failed"];

fn card_id_of(card: &Path) -> String {
    bop_core::read_meta(card).map(|m| m.id).unwrap_or_else(|_| {
        card.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .trim_end_matches(".bop")
            .to_string()
    })
}

pub fn show_json(root: &Path, id: &str) -> anyhow::Result<serde_json::Value> {
    let card = paths::require_card(root, id)?;
    let rec = translog::recover(&card).context("failed to read transition log")?;
    Ok(json!({
        "schema": translog::VIEW_SCHEMA,
        "card": card_id_of(&card),
        "log": translog::log_path(&card),
        "committed_bytes": rec.committed_len,
        "torn_tail": rec.torn_tail,
        "view": rec.view,
    }))
}

pub fn cmd_show(root: &Path, id: &str) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(&show_json(root, id)?)?);
    Ok(())
}

fn all_cards(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in STATE_DIRS {
        if let Ok(rd) = fs::read_dir(root.join(dir)) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() && p.extension().map(|x| x == "bop").unwrap_or(false) {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

/// Returns `(json, ok)`; `ok` is false when a logged card diverges from its
/// directory or has a torn tail. Cards with no log are counted as `unlogged`.
pub fn verify_json(
    root: &Path,
    id: Option<&str>,
    all: bool,
) -> anyhow::Result<(serde_json::Value, bool)> {
    if let Some(id) = id {
        let card = paths::require_card(root, id)?;
        let rep = translog::verify(&card, &card_id_of(&card))?;
        let ok = rep.consistent;
        return Ok((serde_json::to_value(rep)?, ok));
    }
    if !all {
        anyhow::bail!("pass a card id or --all");
    }
    let mut reports = Vec::new();
    let (mut consistent, mut unlogged, mut divergent, mut torn) = (0, 0, 0, 0);
    for card in all_cards(root) {
        let rep = translog::verify(&card, &card_id_of(&card))?;
        if rep.torn_tail.is_some() {
            torn += 1;
        }
        if rep.consistent {
            consistent += 1;
        } else if rep.log_state.is_none() && rep.torn_tail.is_none() {
            unlogged += 1;
        } else {
            divergent += 1;
        }
        reports.push(rep);
    }
    let ok = divergent == 0;
    Ok((
        json!({
            "schema": "bop.translog.verify_all.v1",
            "summary": {
                "total": reports.len(),
                "consistent": consistent,
                "unlogged": unlogged,
                "divergent": divergent,
                "torn_tail": torn,
            },
            "cards": reports,
        }),
        ok,
    ))
}

pub fn cmd_verify(root: &Path, id: Option<&str>, all: bool) -> anyhow::Result<()> {
    let (v, ok) = verify_json(root, id, all)?;
    println!("{}", serde_json::to_string_pretty(&v)?);
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

/// `--version-cmd` adapter: runs `<cmd> <dir>` and uses its trimmed stdout as
/// the filesystem-native version id. A failing or silent command reports none.
struct CommandProbe<'a> {
    cmd: &'a Path,
    name: String,
}

impl bop_core::bench::VersionProbe for CommandProbe<'_> {
    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self, root: &Path) -> Option<String> {
        let out = std::process::Command::new(self.cmd)
            .arg(root)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!v.is_empty()).then_some(v)
    }
}

pub fn bench_json(
    dir: &Path,
    cfg: bop_core::bench::Config,
    version_cmd: Option<&Path>,
) -> anyhow::Result<(serde_json::Value, bool)> {
    let outcome = match version_cmd {
        Some(cmd) => {
            let probe = CommandProbe {
                cmd,
                name: cmd
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("command")
                    .to_string(),
            };
            bop_core::bench::run(dir, &cfg, &probe)?
        }
        None => bop_core::bench::run(dir, &cfg, &bop_core::bench::NoVersion)?,
    };
    Ok((outcome.record, outcome.ok))
}

pub fn cmd_bench(
    dir: &Path,
    cfg: bop_core::bench::Config,
    version_cmd: Option<&Path>,
    out: Option<&Path>,
) -> anyhow::Result<()> {
    let (v, ok) = bench_json(dir, cfg, version_cmd)?;
    let text = serde_json::to_string_pretty(&v)?;
    if let Some(out) = out {
        fs::write(out, format!("{text}\n"))
            .with_context(|| format!("failed to write {}", out.display()))?;
    }
    println!("{text}");
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn mk_card(root: &Path, state: &str, id: &str) -> PathBuf {
        let dir = root.join(state).join(format!("{id}.bop"));
        fs::create_dir_all(&dir).unwrap();
        let meta = bop_core::Meta {
            id: id.into(),
            stage: "implement".into(),
            ..Default::default()
        };
        bop_core::write_meta(&dir, &meta).unwrap();
        dir
    }

    #[test]
    fn show_and_verify_follow_directory_moves() {
        let root = tempdir().unwrap();
        let r = root.path();
        let pending = mk_card(r, "pending", "c1");
        let meta = bop_core::read_meta(&pending).unwrap();
        // Simulate dispatcher: rename then shadow-write.
        let running = r.join("running").join("c1.bop");
        fs::create_dir_all(r.join("running")).unwrap();
        fs::rename(&pending, &running).unwrap();
        translog::shadow_transition(&running, &meta, "pending", "running").unwrap();
        let done = r.join("done").join("c1.bop");
        fs::create_dir_all(r.join("done")).unwrap();
        fs::rename(&running, &done).unwrap();
        translog::shadow_transition(&done, &meta, "running", "done").unwrap();

        let v = show_json(r, "c1").unwrap();
        assert_eq!(v["schema"], "bop.translog.view.v1");
        assert_eq!(v["view"]["state"], "done");
        assert_eq!(v["view"]["attempt"], 1);
        assert_eq!(v["view"]["lineage"].as_array().unwrap().len(), 3);

        let (rep, ok) = verify_json(r, Some("c1"), false).unwrap();
        assert!(ok, "{rep}");

        // An unlogged card and a divergent card.
        mk_card(r, "pending", "c2");
        let c3 = mk_card(r, "failed", "c3");
        let m3 = bop_core::read_meta(&c3).unwrap();
        translog::shadow_transition(&c3, &m3, "pending", "running").unwrap();
        let (all, ok) = verify_json(r, None, true).unwrap();
        assert!(!ok);
        assert_eq!(all["summary"]["total"], 3);
        assert_eq!(all["summary"]["consistent"], 1);
        assert_eq!(all["summary"]["unlogged"], 1);
        assert_eq!(all["summary"]["divergent"], 1);
    }

    #[test]
    fn bench_emits_bench_v1_and_uses_version_cmd() {
        let d = tempdir().unwrap();
        let dir = d.path().join("bench");
        let cfg = bop_core::bench::Config {
            cards: 3,
            artifact_bytes: 64,
            log_lines: 2,
            filesystem: Some("test".into()),
            ..Default::default()
        };
        // `echo <dir>` stands in for a native version probe.
        let (v, ok) = bench_json(&dir, cfg, Some(Path::new("echo"))).unwrap();
        assert!(ok, "{v}");
        assert_eq!(v["schema"], "ryanlab.bench.v1");
        assert_eq!(v["workload"], "bop.state-machine.v1");
        assert_eq!(v["tags"]["version_probe"], "echo");
        assert_eq!(v["tags"]["fs_version_before"], dir.to_str().unwrap());
        assert_eq!(v["metrics"]["cards"], 3);
        assert_eq!(v["raw"]["cards"].as_array().unwrap().len(), 3);
        // A second run into the same (now non-empty) dir is refused.
        let again = bop_core::bench::Config::default();
        assert!(bench_json(&dir, again, None).is_err());
    }

    #[test]
    fn verify_requires_target() {
        let root = tempdir().unwrap();
        assert!(verify_json(root.path(), None, false).is_err());
    }
}
